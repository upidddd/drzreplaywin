"""System sound (WASAPI loopback) and microphone, mixed into one steady 48 kHz stereo stream for ffmpeg.

The stream is paced by the clock, not by the devices: Windows sends nothing from a loopback device
while nothing plays, so gaps are filled with silence and the sound stays in sync with the video.
"""
import threading
import time
from collections import deque

import numpy as np

from ..core import config as C

RATE = 48000
LATENCY = 0.05            # jitter buffer per device, seconds
MAX_BACKLOG = 0.15        # drop audio older than this behind the clock

try:
    import pyaudiowpatch as pa
except ImportError:       # not on Windows (development): silence only
    pa = None


def to_stereo(x):
    """(n, ch) float32 → (n, 2); 5.1 / 7.1 are folded down so dialogue in the centre isn't lost."""
    ch = x.shape[1]
    if ch == 2:
        return x
    if ch == 1:
        return np.repeat(x, 2, axis=1)
    # WAVE order: FL FR [FC LFE] [BL BR] [SL SR]; quad is FL FR BL BR
    k = 0.7071
    left, right = x[:, 0].copy(), x[:, 1].copy()
    if ch == 3 or ch >= 5:
        left += k * x[:, 2]
        right += k * x[:, 2]
    for a, b in ((2, 3),) if ch == 4 else ((4, 5), (6, 7)):
        if b < ch:
            left += k * x[:, a]
            right += k * x[:, b]
    return np.stack([left, right], axis=1) * (1.0 / (1.0 + k))


class Resampler:
    """Streaming linear resampler, keeps its phase across blocks."""

    def __init__(self, src, dst):
        self.step = src / dst
        self.pos = 0.0
        self.last = np.zeros(2, np.float32)

    def process(self, x):
        n = len(x)
        if not n:
            return x
        buf = np.vstack([self.last[None, :], x])        # buf[0] is the previous block's last sample
        idx = np.arange(self.pos, n, self.step)
        i = idx.astype(np.int64)
        f = (idx - i).astype(np.float32)[:, None]
        out = buf[i] * (1 - f) + buf[np.minimum(i + 1, n)] * f
        self.pos = (idx[-1] + self.step - n) if len(idx) else self.pos - n
        self.last = x[-1]
        return out.astype(np.float32)


class Source:
    """One capture device feeding a jitter buffer from PortAudio's callback thread."""

    def __init__(self, p, info):
        self.name = info["name"]
        self.ch = max(1, int(info["maxInputChannels"]))
        self.rate = int(info["defaultSampleRate"])
        self.res = Resampler(self.rate, RATE) if self.rate != RATE else None
        self.lock = threading.Lock()
        self.q = deque()
        self.count = 0
        self.primed = False
        self.stream = p.open(format=pa.paFloat32, channels=self.ch, rate=self.rate, input=True,
                             input_device_index=info["index"], stream_callback=self.callback)

    def callback(self, data, frames, _info, _status):
        x = np.frombuffer(data, np.float32)[:frames * self.ch].reshape(-1, self.ch)
        x = to_stereo(x).astype(np.float32)
        if self.res:
            x = self.res.process(x)
        with self.lock:
            self.q.append(x)
            self.count += len(x)
        return None, pa.paContinue

    def clear(self):
        with self.lock:
            self.q.clear()
            self.count = 0
            self.primed = False

    def pull(self, n):
        """n frames for the mix, or None (silence) while the buffer fills up."""
        with self.lock:
            if not self.primed:
                if self.count < n + LATENCY * RATE:
                    return None
                self.primed = True
            extra = self.count - n - int(MAX_BACKLOG * RATE)
            if extra > 0:
                self._take(self.count - n - int(LATENCY * RATE))
            if self.count < n:          # ran dry (nothing playing): silence until it refills
                self.primed = False
                got = self._take(self.count)
                return np.vstack([got, np.zeros((n - len(got), 2), np.float32)])
            return self._take(n)

    def _take(self, n):
        out = []
        while n > 0 and self.q:
            a = self.q[0]
            if len(a) <= n:
                out.append(self.q.popleft())
                n -= len(a)
                self.count -= len(a)
            else:
                out.append(a[:n])
                self.q[0] = a[n:]
                self.count -= n
                n = 0
        return np.vstack(out) if out else np.zeros((0, 2), np.float32)

    def close(self):
        try:
            self.stream.stop_stream()
            self.stream.close()
        except Exception:
            pass


# ---------------------------------------------------------------------- devices
def _wasapi(p):
    return p.get_host_api_info_by_type(pa.paWASAPI)


def devices():
    """(outputs, inputs): names of the WASAPI playback and recording devices."""
    if pa is None:
        return [], []
    p = pa.PyAudio()
    try:
        api = _wasapi(p)["index"]
        outs, ins = [], []
        for i in range(p.get_device_count()):
            d = p.get_device_info_by_index(i)
            if d["hostApi"] != api or d.get("isLoopbackDevice"):
                continue
            if d["maxOutputChannels"] > 0:
                outs.append(d["name"])
            elif d["maxInputChannels"] > 0:
                ins.append(d["name"])
        return outs, ins
    except Exception:
        return [], []
    finally:
        p.terminate()


def _loopback_for(p, name):
    """The loopback twin of the playback device `name` ("" = default)."""
    if not name:
        name = p.get_device_info_by_index(_wasapi(p)["defaultOutputDevice"])["name"]
    first = None
    for d in p.get_loopback_device_info_generator():
        first = first or d
        if d["name"].startswith(name):
            return d
    return first


def _input_for(p, name):
    api = _wasapi(p)
    if name:
        for i in range(p.get_device_count()):
            d = p.get_device_info_by_index(i)
            if d["hostApi"] == api["index"] and d["name"] == name and d["maxInputChannels"] > 0 \
                    and not d.get("isLoopbackDevice"):
                return d
    return p.get_device_info_by_index(api["defaultInputDevice"])


def default_endpoint(flow):
    """Id of the Windows default playback (flow 0) / recording (flow 1) device, to notice switches."""
    if not C.WINDOWS:
        return None
    import ctypes
    import uuid

    class GUID(ctypes.Structure):
        _fields_ = [("data", ctypes.c_ubyte * 16)]

    def guid(s):
        g = GUID()
        ctypes.memmove(g.data, uuid.UUID(s).bytes_le, 16)
        return g

    def method(obj, index, *argtypes):
        vtbl = ctypes.cast(obj, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p)))[0]
        return ctypes.WINFUNCTYPE(ctypes.HRESULT, ctypes.c_void_p, *argtypes)(vtbl[index])

    ole32 = ctypes.windll.ole32
    ole32.CoInitializeEx(None, 0)
    enum, dev, sid = ctypes.c_void_p(), ctypes.c_void_p(), ctypes.c_wchar_p()
    try:
        ole32.CoCreateInstance(ctypes.byref(guid("BCDE0395-E52F-467C-8E3D-C4579291692E")), None, 23,
                               ctypes.byref(guid("A95664D2-9614-4F35-A746-DE8DB63617E6")), ctypes.byref(enum))
        if not enum:
            return None
        method(enum, 4, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_void_p))(
            enum, flow, 0, ctypes.byref(dev))                     # GetDefaultAudioEndpoint(flow, eConsole)
        method(dev, 5, ctypes.POINTER(ctypes.c_wchar_p))(dev, ctypes.byref(sid))      # GetId
        return sid.value
    except OSError:
        return None
    finally:
        if sid:
            ole32.CoTaskMemFree(sid)
        for o in (dev, enum):
            if o:
                ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(
                    ctypes.cast(o, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p)))[0][2])(o)


class Mixer:
    """Writes the mix to `out` (ffmpeg's stdin) from the moment the first video frame was taken."""

    def __init__(self, cfg, out, log=print):
        self.cfg = cfg
        self.out = out
        self.log = log
        self.p = None
        self.sources = {}
        self.errors = {}
        self.anchor = None
        self.anchored = threading.Event()
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True, name="audio")

    def start(self):
        self.thread.start()

    def stop(self):
        self.stopped.set()

    def set_anchor(self, t):
        """Monotonic time of the first video frame: audio sample 0 belongs to it."""
        if not self.anchored.is_set():
            self.anchor = t + LATENCY       # every device is LATENCY behind the clock
            self.anchored.set()

    @property
    def working(self):
        """False when sound was asked for but no device could be opened."""
        want = [k for k in ("system", "mic") if self.cfg["system_audio" if k == "system" else "mic"]]
        return not want or any(k in self.sources for k in want)

    def open(self):
        self.close_devices()
        if pa is None:
            return
        try:
            self.p = pa.PyAudio()
        except Exception as e:
            self.errors["system"] = str(e)
            return
        for key, enabled, find, name in (("system", self.cfg["system_audio"], _loopback_for, self.cfg["system_device"]),
                                         ("mic", self.cfg["mic"], _input_for, self.cfg["mic_device"])):
            if not enabled:
                continue
            try:
                info = find(self.p, name)
                if info is None:
                    raise RuntimeError("no device")
                self.sources[key] = Source(self.p, info)
                self.errors.pop(key, None)
                self.log(f"audio: {key} = {info['name']} ({int(info['defaultSampleRate'])} Hz, "
                         f"{info['maxInputChannels']} ch)")
            except Exception as e:
                self.errors[key] = str(e)
                self.log(f"audio: {key} failed: {e}")

    def close_devices(self):
        for s in self.sources.values():
            s.close()
        self.sources = {}
        if self.p:
            try:
                self.p.terminate()
            except Exception:
                pass
            self.p = None

    def write(self, data):
        mv = memoryview(data)
        while mv:
            n = self.out.write(mv)
            if not n:
                raise BrokenPipeError
            mv = mv[n:]

    def volumes(self):
        return {"system": self.cfg["system_volume"] / 100.0, "mic": self.cfg["mic_volume"] / 100.0}

    def run(self):
        follow = (not self.cfg["system_device"] and self.cfg["system_audio"],
                  not self.cfg["mic_device"] and self.cfg["mic"])
        ids = (default_endpoint(0) if follow[0] else None, default_endpoint(1) if follow[1] else None)
        self.open()
        if not self.anchored.wait(5):
            self.set_anchor(time.monotonic())
        for s in self.sources.values():
            s.clear()
        written = 0
        next_check = time.monotonic() + 3
        try:
            while not self.stopped.is_set():
                now = time.monotonic()
                need = int((now - self.anchor) * RATE) - written
                if need > 0:
                    mix = np.zeros((need, 2), np.float32)
                    vol = self.volumes()
                    for key, src in list(self.sources.items()):
                        part = src.pull(need)
                        if part is not None:
                            mix += part * vol[key]
                    np.clip(mix, -1.0, 1.0, out=mix)
                    self.write((mix * 32767).astype("<i2").tobytes())
                    written += need
                if now >= next_check and any(follow):
                    next_check = now + 3
                    new = (default_endpoint(0) if follow[0] else None, default_endpoint(1) if follow[1] else None)
                    if new != ids:            # headphones plugged in, default device switched…
                        ids = new
                        self.log("audio: default device changed, reopening")
                        self.open()
                self.stopped.wait(0.01)
        except (BrokenPipeError, OSError, ValueError):
            pass                              # ffmpeg is gone
        finally:
            self.close_devices()
