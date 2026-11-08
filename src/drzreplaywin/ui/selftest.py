"""--selftest DIR: record, save, press the hotkey, take screenshots of every tab, write DIR/report.json.

Run by the build on a real Windows machine; exits with 0 when a clip of the right length was saved.
"""
import json
import os
import platform
import time
import traceback

from PySide6.QtCore import QTimer

from ..core import config as C
from ..core import system
from . import widgets as W


class SelfTest:
    def __init__(self, app, out):
        self.app, self.out = app, out
        self.report = {"version": C.VERSION, "platform": platform.platform(), "steps": []}
        self.saved = []
        self.t0 = time.monotonic()
        app.cfg["output_dir"] = os.path.join(out, "clips")
        app.cfg["seconds"] = 6
        app.recorder.ring.seconds = 6
        self.recorded = []
        app.event.connect(self.on_event)
        self.steps = iter([self.wait_recording, self.wait_buffer, self.save, self.wait_saved, self.hotkey,
                           self.wait_hotkey, self.record, self.wait_record_stop, self.wait_recorded,
                           self.screenshots, self.finish])
        self.step = next(self.steps)
        self.deadline = time.monotonic() + 120
        self.timer = QTimer(interval=250, timeout=self.tick)
        self.timer.start()

    def log(self, text):
        self.report["steps"].append(f"{time.monotonic() - self.t0:6.1f}s {text}")

    def on_event(self, kind, data):
        if kind == "saved":
            self.saved.append(data)
        elif kind == "recorded":
            self.recorded.append(data)
        elif kind == "save_failed":
            self.log(f"save failed: {data}")

    def tick(self):
        try:
            if self.step():
                self.step = next(self.steps)
                self.deadline = time.monotonic() + 90
            elif time.monotonic() > self.deadline:
                self.log(f"timeout in {self.step.__name__}")
                self.finish(False)
        except StopIteration:
            pass
        except Exception:
            self.log(traceback.format_exc())
            self.finish(False)

    # ------------------------------------------------------------------ steps
    def wait_recording(self):
        st = self.app.recorder.state
        if st == "error":
            self.log(f"recorder error: {self.app.recorder.error}")
            self.finish(False)
        if st == "recording":
            self.log(f"recording: {self.app.recorder.status()}")
            self.report["probe"] = self.app.recorder.probe
            return True

    def wait_buffer(self):
        return self.app.recorder.ring.buffered() >= 5.9

    def save(self):
        self.log(f"buffer {self.app.recorder.ring.buffered():.1f} s, "
                 f"{self.app.recorder.ring.size / 1e6:.1f} MB; saving the whole buffer (6 s)")
        return self.app.save_replay()

    def wait_saved(self):
        if self.saved:
            clip = self.saved[0]
            clip["duration"] = system.media_duration(clip["path"])
            r = system.run([system.ffmpeg(), "-hide_banner", "-i", clip["path"]])
            clip["streams"] = [ln.strip() for ln in (r.stderr if r else "").splitlines() if "Stream #" in ln]
            self.log(f"saved {clip}")
            return True

    def hotkey(self):
        hk = self.app.hotkey
        self.log(f"hotkey ok={hk.ok} error={hk.error!r}")
        if not C.WINDOWS or not hk.ok:
            self.report["hotkey"] = "not registered"
            return True
        import ctypes
        user32 = ctypes.windll.user32
        for vk, up in ((0x12, 0), (0x5A, 0), (0x5A, 2), (0x12, 2)):      # Alt down, Z down, Z up, Alt up
            user32.keybd_event(vk, 0, up, 0)
            time.sleep(0.03)
        self.hotkey_at = time.monotonic()
        return True

    def wait_hotkey(self):
        if len(self.saved) >= 2:
            self.report["hotkey"] = "saved a clip"
            # pressed ~1 s after the first replay: with the buffer cleared it holds about that much
            d = system.media_duration(self.saved[1]["path"])
            self.report["since_last"] = f"{d:.2f} s ({'ok' if d < 3.5 else 'TOO LONG'})"
            self.log(f"hotkey clip {self.saved[1]}, {d:.2f} s")
            return True
        if self.report.get("hotkey") or time.monotonic() - getattr(self, "hotkey_at", 0) > 8:
            self.report.setdefault("hotkey", "no clip after the key press")
            return True

    def record(self):
        ok = self.app.toggle_recording()
        self.log(f"recording started: {ok}")
        self.rec_at = time.monotonic()
        return True

    def wait_record_stop(self):
        if time.monotonic() - self.rec_at >= 4:
            self.app.toggle_recording()
            return True

    def wait_recorded(self):
        if self.recorded:
            d = system.media_duration(self.recorded[0]["path"])
            self.report["recording"] = f"{d:.2f} s for 4 s ({'ok' if 3.5 <= d <= 6 else 'WRONG'})"
            self.log(f"recorded {self.recorded[0]}, {d:.2f} s")
            return True

    def screenshots(self):
        app = self.app
        app.show_window()
        win = app.window
        win.load_clips(True)
        QTimer.singleShot(0, lambda: None)
        for theme in ("light", "dark"):
            self.force_theme(theme)
            for i, name in enumerate(("record", "clips", "settings")):
                win.tabs.button(i).setChecked(True)
                win.poll()
                for _ in range(40 if name == "clips" and theme == "light" else 5):      # thumbnails load
                    app.qapp.processEvents()
                    time.sleep(0.05)
                win.grab().save(os.path.join(self.out, f"{name}-{theme}.png"))
        self.force_theme("light")
        app.overlay.show_text("Replay saved", "0:20 · 31.4 MB")
        for _ in range(5):
            app.qapp.processEvents()
            time.sleep(0.05)
        app.overlay.grab().save(os.path.join(self.out, "overlay.png"))
        self.log("screenshots taken")
        return True

    def force_theme(self, theme):
        W.apply_theme(self.app.qapp, dark=theme == "dark")

    def finish(self, ok=None):
        self.timer.stop()
        rec = self.app.recorder
        if ok is None:
            clip = self.saved[0] if self.saved else {}
            ok = 5.0 <= clip.get("duration", 0) <= 8.0 and "ok" in self.report.get("recording", "") and \
                "TOO" not in self.report.get("since_last", "")
        self.report["ok"] = bool(ok)
        self.report["status"] = rec.status()
        self.report["recorder_log"] = rec.log_lines[-120:]
        self.report["audio_devices"] = getattr(self.app, "devices", None)
        with open(os.path.join(self.out, "report.json"), "w", encoding="utf-8") as f:
            json.dump(self.report, f, indent=2, default=str)
        rec.write_log()
        self.app.quit(0 if ok else 1)
        return True
