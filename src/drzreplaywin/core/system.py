"""Probing the machine (screens, encoders), running ffmpeg, update check, opening files."""
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request

from . import config as C

CREATE_NO_WINDOW = 0x08000000
ABOVE_NORMAL_PRIORITY_CLASS = 0x00008000
BELOW_NORMAL_PRIORITY_CLASS = 0x00004000

ENCODER_LABELS = {"nvenc": "NVIDIA NVENC", "amf": "AMD AMF", "qsv": "Intel Quick Sync", "x264": "CPU (x264)"}
CODECS = {"nvenc": "h264_nvenc", "amf": "h264_amf", "qsv": "h264_qsv", "x264": "libx264"}

# filters between the capture and the encoder, best first. ddagrab gives GPU (D3D11) frames,
# gdigrab / x11grab give BGRA frames in memory.
FILTERS = {
    "dda": {
        "nvenc": [None, "hwdownload,format=bgra"],
        "amf": [None, "hwdownload,format=bgra,format=nv12"],
        "qsv": ["hwmap=derive_device=qsv,format=qsv", "hwdownload,format=bgra,format=nv12"],
        "x264": ["hwdownload,format=bgra,format=yuv420p"],
    },
    "gdi": {"nvenc": [None], "amf": ["format=nv12"], "qsv": ["format=nv12"], "x264": ["format=yuv420p"]},
    "x11": {"nvenc": [None], "x264": ["format=yuv420p"]},
}


def ffmpeg():
    if C.FROZEN:
        exe = os.path.join(sys._MEIPASS, "ffmpeg", "ffmpeg.exe")
        if os.path.exists(exe):
            return exe
    return shutil.which("ffmpeg") or "ffmpeg"


def _flags(priority=0):
    return (CREATE_NO_WINDOW | priority) if C.WINDOWS else 0


def popen(cmd, priority=0, **kw):
    """subprocess.Popen without a console window flashing up on Windows."""
    return subprocess.Popen(cmd, creationflags=_flags(priority), **kw)


def run(cmd, timeout=15, priority=0):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=timeout,
                              creationflags=_flags(priority), stdin=subprocess.DEVNULL)
    except Exception:
        return None


# ---------------------------------------------------------------------- probing
def ffmpeg_version():
    r = run([ffmpeg(), "-hide_banner", "-version"])
    return r.stdout.split("\n", 1)[0] if r and r.returncode == 0 else None


def _dda_outputs():
    """[(w, h)] of every screen ddagrab can capture (outputs of the main graphics card)."""
    out = []
    for i in range(8):
        r = run([ffmpeg(), "-hide_banner", "-v", "verbose", "-f", "lavfi", "-i", f"ddagrab=output_idx={i}:framerate=5",
                 "-frames:v", "1", "-f", "null", "-"], timeout=20)
        m = r and re.search(r"Opened dxgi output \d+ with dimensions (\d+)x(\d+)", r.stderr)
        if not m or r.returncode != 0:
            break
        out.append((int(m.group(1)), int(m.group(2))))
    return out


def _dxgi_outputs():
    """[(x, y, w, h)] of the screens of the main graphics card in desktop pixels, numbered like ddagrab
    numbers them. ddagrab only reports sizes; the position tells which one is the main screen (it's at 0, 0)."""
    import ctypes
    from ctypes import wintypes

    class GUID(ctypes.Structure):
        _fields_ = [("a", ctypes.c_uint32), ("b", ctypes.c_uint16), ("c", ctypes.c_uint16), ("d", ctypes.c_ubyte * 8)]

    class OUTPUT_DESC(ctypes.Structure):
        _fields_ = [("name", ctypes.c_wchar * 32), ("rect", wintypes.RECT), ("attached", wintypes.BOOL),
                    ("rotation", ctypes.c_uint), ("monitor", wintypes.HMONITOR)]

    def call(obj, index, *args):
        """COM method `index` of the object's vtable; args are ints (UINT) or ctypes objects passed by pointer."""
        vtbl = ctypes.cast(obj, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
        types = [ctypes.c_uint if isinstance(a, int) else ctypes.c_void_p for a in args]
        values = [a if isinstance(a, int) else ctypes.addressof(a) for a in args]
        proto = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, *types)
        return proto(vtbl[index])(obj, *values)

    out = []
    iid = GUID(0x770AAE78, 0xF26F, 0x4DBA, (ctypes.c_ubyte * 8)(0xA8, 0x29, 0x25, 0x3C, 0x83, 0xD1, 0xB3, 0x87))
    factory, adapter = ctypes.c_void_p(), ctypes.c_void_p()
    if ctypes.WinDLL("dxgi").CreateDXGIFactory1(ctypes.byref(iid), ctypes.byref(factory)) < 0:
        return out
    try:
        if call(factory, 7, 0, adapter) < 0:    # EnumAdapters
            return out
        for i in range(8):
            output, desc = ctypes.c_void_p(), OUTPUT_DESC()
            if call(adapter, 7, i, output) < 0:    # EnumOutputs
                break
            if call(output, 7, desc) >= 0:    # GetDesc
                r = desc.rect
                out.append((r.left, r.top, r.right - r.left, r.bottom - r.top))
            call(output, 2)    # Release
        call(adapter, 2)
    finally:
        call(factory, 2)
    return out


def _gdi_monitors():
    """[(w, h, x, y)] of every monitor, in desktop pixels."""
    import ctypes
    from ctypes import wintypes
    mons = []
    proc_t = ctypes.WINFUNCTYPE(ctypes.c_int, wintypes.HMONITOR, wintypes.HDC, ctypes.POINTER(wintypes.RECT),
                                wintypes.LPARAM)

    def cb(_h, _dc, rect, _lp):
        r = rect.contents
        mons.append((r.right - r.left, r.bottom - r.top, r.left, r.top))
        return 1

    ctypes.windll.user32.EnumDisplayMonitors(None, None, proc_t(cb), 0)
    return mons or [(1920, 1080, 0, 0)]


def _x11_screen():
    r = run(["xdpyinfo"]) if shutil.which("xdpyinfo") else None
    m = r and re.search(r"dimensions:\s+(\d+)x(\d+)", r.stdout)
    return [(int(m.group(1)), int(m.group(2)), 0, 0)] if m else [(1920, 1080, 0, 0)]


def screen_index(probe, cfg):
    """The screen to record: the one picked in Settings, else the main one."""
    mons = probe["monitors"]
    if 0 <= cfg["monitor"] < len(mons):
        return cfg["monitor"]
    return next((i for i, m in enumerate(mons) if m.get("main")), 0)


def capture_args(probe, cfg, fps=None):
    fps = fps or cfg["fps"]
    mons = probe["monitors"]
    idx = screen_index(probe, cfg)
    mouse = int(bool(cfg["draw_mouse"]))
    if probe["capture"] == "dda":
        return ["-f", "lavfi", "-i", f"ddagrab=output_idx={idx}:framerate={fps}:draw_mouse={mouse}"]
    m = mons[idx]
    w, h = m["w"] - m["w"] % 2, m["h"] - m["h"] % 2
    if probe["capture"] == "gdi":
        return ["-f", "gdigrab", "-thread_queue_size", "64", "-framerate", str(fps), "-draw_mouse", str(mouse),
                "-offset_x", str(m["x"]), "-offset_y", str(m["y"]), "-video_size", f"{w}x{h}", "-i", "desktop"]
    return ["-f", "x11grab", "-thread_queue_size", "64", "-framerate", str(fps), "-draw_mouse", str(mouse),
            "-video_size", f"{w}x{h}", "-i", f"{os.environ.get('DISPLAY', ':0')}+{m['x']},{m['y']}"]


def encoder_args(enc, quality, fps):
    q = str(C.REC_QUALITY.get(quality, 23))
    gop = ["-g", str(fps)]           # a keyframe every second: clips start at most 1 s early
    if enc == "nvenc":
        return ["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", q, "-b:v", "0", "-bf", "0"] + gop
    if enc == "amf":
        return ["-c:v", "h264_amf", "-quality", "speed", "-rc", "cqp", "-qp_i", q, "-qp_p", q, "-bf", "0"] + gop
    if enc == "qsv":
        return ["-c:v", "h264_qsv", "-preset", "veryfast", "-global_quality", q, "-bf", "0"] + gop
    return ["-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency", "-crf", q] + gop


def _try_encoder(cap, enc, vf):
    cmd = [ffmpeg(), "-hide_banner", "-v", "error"] + cap + ["-frames:v", "8"]
    if vf:
        cmd += ["-vf", vf]
    r = run(cmd + encoder_args(enc, "balanced", 30) + ["-f", "null", "-"], timeout=30)
    return bool(r) and r.returncode == 0


def probe(force=False):
    """{"capture", "monitors": [{w, h, x, y}], "encoders": {enc: filter}, "available": [...]}, cached."""
    version = ffmpeg_version()
    cached = C.read_json(C.PROBE_CACHE)
    if (not force and cached and cached.get("ffmpeg") == version and cached.get("app") == C.VERSION
            and time.time() - cached.get("time", 0) < 7 * 24 * 3600):
        return cached
    if C.WINDOWS:
        dda = _dda_outputs()
        if dda:
            try:
                pos = _dxgi_outputs()
            except Exception:
                pos = []
            if [(w, h) for _x, _y, w, h in pos] != dda:          # not the same screens: sizes only
                pos = [(None, None, w, h) for w, h in dda]
            capture, mons = "dda", [dict(w=w, h=h, x=x, y=y) for x, y, w, h in pos]
        else:
            capture, mons = "gdi", [dict(w=w, h=h, x=x, y=y) for w, h, x, y in _gdi_monitors()]
    else:
        capture, mons = "x11", [dict(w=w, h=h, x=x, y=y) for w, h, x, y in _x11_screen()]
    for m in mons:
        m["main"] = (m["x"], m["y"]) == (0, 0)          # the main screen's corner is the desktop's origin
    if mons and not any(m["main"] for m in mons):
        mons[0]["main"] = True
    cap = capture_args({"capture": capture, "monitors": mons}, {"monitor": 0, "draw_mouse": True, "fps": 30})
    encoders = {}
    for enc, variants in FILTERS[capture].items():
        for vf in variants:
            if _try_encoder(cap, enc, vf):
                encoders[enc] = vf
                break
    res = {"capture": capture, "monitors": mons, "encoders": encoders,
           "available": [e for e in ENCODER_LABELS if e in encoders],
           "ffmpeg": version, "app": C.VERSION, "time": time.time()}
    C.write_json(C.PROBE_CACHE, res)
    return res


def pick_encoder(probe, wanted):
    avail = probe["available"]
    if wanted in avail:
        return wanted
    return avail[0] if avail else "x264"


def media_duration(path):
    r = run([ffmpeg(), "-hide_banner", "-i", path], timeout=15)
    m = r and re.search(r"Duration: (\d+):(\d+):(\d+(?:\.\d+)?)", r.stderr)
    return int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3)) if m else 0.0


# ---------------------------------------------------------------------- files
def open_path(path):
    if C.WINDOWS:
        os.startfile(path)
    else:
        popen(["xdg-open", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def show_in_folder(path):
    if C.WINDOWS:
        popen(["explorer", "/select,", os.path.normpath(path)])
    else:
        open_path(os.path.dirname(path))


# ---------------------------------------------------------------------- updates
def latest_version(timeout=6):
    """Tag of the newest GitHub release, read from the /releases/latest redirect (no API limits)."""
    req = urllib.request.Request(f"https://github.com/{C.REPO}/releases/latest", method="HEAD",
                                 headers={"User-Agent": f"{C.APP_NAME}/{C.VERSION}"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            m = re.search(r"/tag/v?([0-9][0-9.]*)$", r.geturl())
            return m.group(1) if m else None
    except Exception:
        return None


def version_tuple(v):
    try:
        return tuple(int(x) for x in str(v).split("."))
    except ValueError:
        return (0,)


def download(url, dest, progress=None):
    req = urllib.request.Request(url, headers={"User-Agent": f"{C.APP_NAME}/{C.VERSION}"})
    with urllib.request.urlopen(req, timeout=30) as r, open(dest + ".part", "wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        done = 0
        while chunk := r.read(1 << 16):
            f.write(chunk)
            done += len(chunk)
            if progress and total:
                progress(done / total)
    os.replace(dest + ".part", dest)
