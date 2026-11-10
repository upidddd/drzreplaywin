"""The recorder: one ffmpeg process capturing the screen into the in-memory ring, kept alive by a watchdog,
and a save queue that turns the newest part of the ring into a clip without re-encoding."""
import os
import queue
import re
import subprocess
import threading
import time
from datetime import datetime, timedelta

from ..core import config as C
from ..core import system
from . import intro
from .audio import RATE, Mixer
from .ring import Ring

# a log line printed right before the first frame is grabbed: audio sample 0 is pinned to it
FIRST_FRAME = re.compile(r"Opened dxgi output|Capturing whole desktop|Input #0, (x11grab|gdigrab|lavfi)")
SAVE_DELAY = 0.3          # let the encoder flush the moment the key was pressed


class Recorder:
    def __init__(self, cfg, emit):
        """emit(kind, data) is called from worker threads: "state", "saving", "saved", "save_failed"."""
        self.cfg = cfg
        self.emit = emit
        self.ring = Ring(cfg["seconds"], C.MAX_BUFFER_MB << 20)
        self.lock = threading.RLock()
        self.state = "off"            # off | starting | recording | paused | error
        self.error = ""
        self.proc = None
        self.mixer = None
        self.gen = 0
        self.fails = 0
        self.audio_off = False        # sound kept failing: record without it
        self.probe = None
        self.info = {}
        self.started_at = 0.0
        self.log_lines = []
        self.session = 0.0            # when the current ring started (a restart empties it)
        self.last_save = None         # (session, press time) of the last replay
        self.filerec = None           # full recording: {"tmp", "path", "t0", "file"}
        self.rec_only = False         # replays are off: the screen is captured only for a full recording
        self.file_pending = False     # that recording starts as soon as the capture is up
        self.saves = queue.Queue()
        threading.Thread(target=self.save_worker, daemon=True, name="saver").start()

    # ------------------------------------------------------------------ control
    def start(self):
        with self.lock:
            self.gen += 1
            self.fails = 0
            self.audio_off = False
            self._kill()
            self._set("starting")
            threading.Thread(target=self._launch, args=(self.gen,), daemon=True).start()

    def stop(self, state="off"):
        with self.lock:
            self.stop_file()
            self.gen += 1
            self._kill()
            self.ring.reset()
            self._set(state)

    def pause(self):
        self.stop("paused")

    def redetect(self):
        self.probe = None
        self.start()

    def _set(self, state, error=""):
        self.state, self.error = state, error
        self.emit("state", state)

    def _kill(self):
        if self.mixer:
            self.mixer.stop()
            self.mixer = None
        if self.proc:
            try:
                self.proc.kill()
            except OSError:
                pass
            self.proc = None

    def status(self):
        p = self.probe or {}
        return {
            "state": self.state, "error": self.error,
            "buffered": self.buffered() if self.state == "recording" else 0.0,
            "file_recording": self.filerec["t0"] if self.filerec else None,
            "replays_off": self.state == "paused" or self.rec_only,
            "seconds": self.cfg["seconds"], "buffer_mb": self.ring.size / (1 << 20),
            "size": self.info.get("size"), "fps": self.info.get("fps"), "encoder": self.info.get("encoder"),
            "audio_off": self.audio_off or (self.mixer is not None and not self.mixer.working),
            "monitors": p.get("monitors", []), "encoders": p.get("available", []),
        }

    # ------------------------------------------------------------------ ffmpeg
    def log(self, line):
        self.log_lines.append(f"{datetime.now():%H:%M:%S} {line}")
        if len(self.log_lines) > 2000:
            del self.log_lines[:500]

    def write_log(self):
        try:
            os.makedirs(C.DATA_DIR, exist_ok=True)
            with open(C.RECORDER_LOG, "w", encoding="utf-8") as f:
                f.write("\n".join(self.log_lines) + "\n")
        except OSError:
            pass

    def build_cmd(self, enc, with_audio):
        cfg = self.cfg
        vf = self.probe["encoders"].get(enc)
        cmd = [system.ffmpeg(), "-hide_banner", "-nostats", "-v", "verbose"]
        cmd += system.capture_args(self.probe, cfg)
        if with_audio:
            # raw PCM needs no probing; by default ffmpeg would wait 5 s of audio before encoding anything
            cmd += ["-thread_queue_size", "1024", "-probesize", "32", "-analyzeduration", "0",
                    "-f", "s16le", "-ar", str(RATE), "-ac", "2", "-i", "-"]
        cmd += ["-map", "0:v"] + (["-map", "1:a"] if with_audio else [])
        if vf:
            cmd += ["-vf", vf]
        cmd += ["-fps_mode", "passthrough"] + system.encoder_args(enc, cfg["rec_quality"], cfg["fps"])
        if with_audio:
            cmd += ["-c:a", "aac", "-b:a", "160k"]
        cmd += ["-f", "mpegts", "-flush_packets", "1", "-muxdelay", "0", "-muxpreload", "0", "pipe:1"]
        return cmd

    def _launch(self, gen):
        if self.probe is None:
            self.emit("state", "probing")
            self.probe = system.probe(force=self.fails >= 4)
        with self.lock:
            if gen != self.gen:
                return
            enc = system.pick_encoder(self.probe, self.cfg["encoder"])
            with_audio = (self.cfg["system_audio"] or self.cfg["mic"]) and not self.audio_off
            cmd = self.build_cmd(enc, with_audio)
            self.log("$ " + " ".join(f'"{a}"' if " " in a else a for a in cmd))
            self.ring.seconds = self.cfg["seconds"]
            if self.filerec:          # a new ffmpeg means a new stream: finish the running recording
                keep = self.rec_only
                self.rec_only = False
                self.stop_file()
                # replays off: that capture only exists for the video, go on in a new file
                self.rec_only = self.file_pending = keep
            self.ring.reset()
            self.session = time.monotonic()
            try:
                self.proc = proc = system.popen(cmd, priority=system.ABOVE_NORMAL_PRIORITY_CLASS, bufsize=0,
                                                stdin=subprocess.PIPE if with_audio else subprocess.DEVNULL,
                                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            except OSError as e:
                self.log(f"cannot start ffmpeg: {e}")
                self._set("error", "ffmpeg is missing")
                return
            self.mixer = Mixer(self.cfg, proc.stdin, self.log) if with_audio else None
            if self.mixer:
                self.mixer.start()
            mons = self.probe["monitors"]
            idx = system.screen_index(self.probe, self.cfg)
            m = mons[idx]
            if len(mons) > 1:
                self.log(f"recording screen {idx + 1} of {len(mons)}: {m}")
            self.info = {"size": (m["w"], m["h"]), "fps": self.cfg["fps"], "encoder": enc}
            self.started_at = time.monotonic()
        threading.Thread(target=self._read_err, args=(proc, self.mixer), daemon=True).start()
        self._read_out(proc, gen)
        code = proc.wait()
        with self.lock:
            if gen != self.gen:
                return
            self.fails += 1
            self.log(f"ffmpeg exited with code {code} (failure {self.fails})")
            self.write_log()
            if time.monotonic() - self.started_at > 60:
                self.fails = 1        # it ran fine for a while: a hiccup (UAC prompt, mode switch), not a broken setup
            if with_audio and self.fails >= 3:
                self.audio_off = True
            if self.fails >= 4 and self.fails % 4 == 0:
                self.probe = None     # maybe the graphics card / driver changed
            if self.fails >= 8:
                self._kill()
                self._set("error", self.last_error() or "Recording keeps failing")
                return
            self._set("starting")
        time.sleep(min(10, 1 + self.fails))
        with self.lock:
            if gen == self.gen:
                threading.Thread(target=self._launch, args=(gen,), daemon=True).start()

    def _read_out(self, proc, gen):
        first = True
        while True:
            try:
                data = proc.stdout.read(1 << 16)
            except (OSError, ValueError):
                break
            if not data:
                break
            self.ring.feed(data)
            if first and self.ring.chunks:
                first = False
                with self.lock:
                    if gen == self.gen:
                        self._set("recording")
                        if self.file_pending:
                            self.file_pending = False
                            self.start_file()

    def _read_err(self, proc, mixer):
        for raw in iter(proc.stderr.readline, b""):
            line = raw.decode("utf-8", "replace").rstrip()
            if mixer and FIRST_FRAME.search(line):
                mixer.set_anchor(time.monotonic())
            if line and not line.startswith(("[out#", "frame=")):
                self.log(line)

    def last_error(self):
        for line in reversed(self.log_lines[-40:]):
            low = line.lower()
            if "error" in low or "failed" in low or "cannot" in low:
                return line.split(" ", 1)[1][:160]
        return ""

    # ------------------------------------------------------------------ saving
    def buffered(self):
        """Seconds a replay would save now: a replay empties the buffer."""
        b = self.ring.buffered()
        if self.cfg["replay_since_last"] and self.last_save and self.last_save[0] == self.session:
            b = min(b, max(0.0, time.monotonic() - self.last_save[1]))
        return b

    def save(self):
        """Queue a save; two presses give two clips."""
        if self.state != "recording" or self.rec_only or not self.ring.chunks:
            return False
        self.saves.put(("replay", time.monotonic()))
        return True

    def save_worker(self):
        while True:
            kind, t = self.saves.get()
            if kind == "recording":
                self.finish_file(t)
                continue
            self.emit("saving", None)
            time.sleep(max(0.0, t + SAVE_DELAY - time.monotonic()))
            try:
                path, length = self.save_now(t)
                self.emit("saved", {"path": path, "length": length, "size": os.path.getsize(path)})
            except Exception as e:
                self.log(f"save failed: {e}")
                self.write_log()
                self.emit("save_failed", str(e))

    def new_path(self, ext):
        folder = C.output_dir(self.cfg)
        os.makedirs(folder, exist_ok=True)
        base = f"{self.cfg['file_prefix'] or 'DrzReplay'} {datetime.now():%Y-%m-%d %H-%M-%S}"
        path = os.path.join(folder, f"{base}.{ext}")
        n = 2
        while os.path.exists(path):
            path = os.path.join(folder, f"{base} ({n}).{ext}")
            n += 1
        return path

    def remux(self, src, path, data=None):
        """MPEG-TS (a file, or bytes when src is None) → MP4 / MKV, no re-encoding."""
        tmp = path + ".part"
        cmd = [system.ffmpeg(), "-hide_banner", "-v", "error", "-y", "-f", "mpegts", "-i", src or "-",
               "-map", "0", "-c", "copy", "-dn", "-avoid_negative_ts", "make_zero"]
        cmd += ["-movflags", "+faststart", "-f", "mp4"] if path.endswith(".mp4") else ["-f", "matroska"]
        proc = system.popen(cmd + [tmp], priority=system.BELOW_NORMAL_PRIORITY_CLASS,
                            stdin=subprocess.PIPE if data is not None else subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        _, err = proc.communicate(data, timeout=600 if data is None else 120)
        if proc.returncode != 0 or not os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass
            raise RuntimeError(err.decode("utf-8", "replace").strip().splitlines()[-1] if err else "ffmpeg failed")
        os.replace(tmp, path)

    def with_intro(self, data, path, number, when):
        """MPEG-TS bytes → MP4 / MKV with the intro laid over the first seconds. The picture has to be encoded
        again for that: the encoder the recorder uses, x264 if it can't."""
        clip = os.path.join(C.DATA_DIR, "intro.mov")
        os.makedirs(C.DATA_DIR, exist_ok=True)
        h = (self.info.get("size") or (1920, 1080))[1]
        intro.render(clip, number, when, h, int(self.cfg["fps"]))
        x, y = intro.position(h)
        tmp = path + ".part"
        enc = self.info.get("encoder") or "x264"
        err = "ffmpeg failed"
        try:
            for e in dict.fromkeys([enc, "x264"]):
                pix = "nv12" if e in ("qsv", "amf") else "yuv420p"
                # ffmpeg starts the replay's timestamps at 0, so the intro's START is seconds into the clip
                fc = (f"[1:v]setpts=PTS-STARTPTS+{intro.START}/TB[ov];"
                      f"[0:v][ov]overlay={x}:{y}:eof_action=pass,format={pix}[v]")
                cmd = [system.ffmpeg(), "-hide_banner", "-v", "error", "-y", "-f", "mpegts", "-i", "-", "-i", clip,
                       "-filter_complex", fc, "-map", "[v]", "-map", "0:a?", "-c:a", "copy", "-dn",
                       "-fps_mode", "passthrough"]
                cmd += system.encoder_args(e, self.cfg["rec_quality"], self.cfg["fps"])
                cmd += ["-movflags", "+faststart", "-f", "mp4"] if path.endswith(".mp4") else ["-f", "matroska"]
                proc = system.popen(cmd + [tmp], priority=system.BELOW_NORMAL_PRIORITY_CLASS, stdin=subprocess.PIPE,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                _, out = proc.communicate(data, timeout=600)
                if proc.returncode == 0 and os.path.exists(tmp) and os.path.getsize(tmp) > 1000:
                    os.replace(tmp, path)
                    return
                err = out.decode("utf-8", "replace").strip().splitlines()[-1] if out else err
                self.log(f"intro export with {e} failed: {err}")
            raise RuntimeError(err)
        finally:
            for f in (tmp, clip):
                try:
                    os.remove(f)
                except OSError:
                    pass

    def save_now(self, pressed=None):
        pressed = pressed or time.monotonic()
        want = float(self.cfg["seconds"])
        if self.cfg["replay_since_last"] and self.last_save and self.last_save[0] == self.session:
            want = min(want, max(1.0, pressed - self.last_save[1]))      # only what came after the last one
        data, length = self.ring.snapshot(want)
        if not data:
            raise RuntimeError("Nothing recorded yet")
        prev, self.last_save = self.last_save, (self.session, pressed)
        ext = "mkv" if self.cfg["container"] == "mkv" else "mp4"
        path = self.new_path(ext)
        number = intro.next_number(self.cfg)            # every replay counts, intro or not
        try:
            if self.cfg["intro"] and intro.available():
                when = datetime.now() - timedelta(seconds=max(0.0, time.monotonic() - pressed))
                try:
                    self.with_intro(data, path, number, when)
                except Exception as e:                  # a broken intro must not cost the replay
                    self.log(f"intro failed, saving without it: {e}")
                    self.remux(None, path, data)
            else:
                self.remux(None, path, data)
        except Exception:
            self.last_save = prev
            raise
        intro.commit(number)
        self.cleanup(path)
        return path, min(length, want + 1.0)

    # ------------------------------------------------------------------ full recordings
    def toggle_file(self):
        return self.stop_file() if self.filerec else self.start_file()

    def start_file(self):
        """Write the stream to disk from now until stopped (same encoder, nothing extra to encode)."""
        if self.filerec or self.file_pending:
            return False
        if self.state in ("paused", "off"):
            # replays are off: capture just for this video, and stop again when it ends
            self.rec_only = self.file_pending = True
            self.start()
            return True
        if self.state != "recording" or not self.ring.chunks:
            return False
        ext = "mkv" if self.cfg["container"] == "mkv" else "mp4"
        path = self.new_path(ext)
        tmp = path[:-len(ext) - 1] + ".recording.ts"
        try:
            f = open(tmp, "wb", buffering=1 << 20)
        except OSError as e:
            self.emit("save_failed", f"can't record: {e.strerror}")
            return False
        self.filerec = {"tmp": tmp, "path": path, "t0": time.time(), "file": f}
        self.ring.start_tap(f)
        self.emit("rec_started", None)
        return True

    def stop_file(self, wait=False):
        rec = self.filerec
        back_off, self.rec_only = self.rec_only, False
        if self.file_pending:                # stopped before the capture was even up
            self.file_pending = False
            self.stop("paused")
            return True
        if not rec:
            return False
        self.filerec = None
        self.ring.stop_tap()
        rec["file"].close()
        rec["length"] = time.time() - rec["t0"]
        self.emit("rec_stopped", None)
        if back_off:
            self.stop("paused")              # replays stay off, as they were
        if wait:
            self.finish_file(rec)
        else:
            self.saves.put(("recording", rec))
        return True

    def finish_file(self, rec):
        try:
            self.remux(rec["tmp"], rec["path"])
            os.remove(rec["tmp"])
            self.emit("recorded", {"path": rec["path"], "length": rec["length"],
                                   "size": os.path.getsize(rec["path"])})
        except Exception as e:
            self.log(f"recording failed: {e}")
            self.write_log()
            self.emit("save_failed", f"recording: {e} (the raw file is {rec['tmp']})")

    def cleanup(self, keep):
        """Delete the oldest clips past the count / size limits."""
        max_n, max_gb = int(self.cfg["keep_max_clips"]), float(self.cfg["keep_max_gb"])
        if not max_n and not max_gb:
            return
        clips = []
        for e in os.scandir(C.output_dir(self.cfg)):
            if e.is_file() and e.name.lower().endswith((".mp4", ".mkv")):
                st = e.stat()
                clips.append((st.st_mtime, st.st_size, e.path))
        clips.sort(reverse=True)
        total = 0
        for i, (_, size, p) in enumerate(clips):
            total += size
            if p != keep and ((max_n and i >= max_n) or (max_gb and total > max_gb * (1 << 30))):
                try:
                    os.remove(p)
                except OSError:
                    pass
