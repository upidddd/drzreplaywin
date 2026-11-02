"""The running app: owns the settings, the recorder, the hotkey, the tray and the window."""
import os
import subprocess
import tempfile
import threading
import time

from PySide6.QtCore import QObject, QTimer, Signal
from PySide6.QtGui import QIcon
from PySide6.QtNetwork import QLocalServer
from PySide6.QtWidgets import QSystemTrayIcon

from ..core import autostart, system
from ..core import config as C
from ..recorder.audio import devices
from ..recorder.hotkey import Hotkey
from ..recorder.recorder import Recorder
from . import widgets as W
from .tray import Tray


def server_name():
    return f"{C.APP_NAME}-{os.environ.get('USERNAME') or os.environ.get('USER') or 'user'}"


class App(QObject):
    event = Signal(str, object)

    def __init__(self, qapp, show=True, selftest=None):
        super().__init__()
        self.qapp = qapp
        self.first_run = not C.exists()
        self.cfg = C.load()
        self.window = None
        self.update_ready = None
        self.last_clip = None
        self.selftest = selftest
        self._quitting = False
        self.event.connect(self.on_event)

        self.save_timer = QTimer(self, singleShot=True, interval=400, timeout=lambda: C.save(self.cfg))
        self.restart_timer = QTimer(self, singleShot=True, interval=800, timeout=self.restart_recorder)

        self.recorder = Recorder(self.cfg, self.event.emit)
        self.hotkey = Hotkey(self.cfg["hotkey"], lambda: self.event.emit("hotkey", None))
        self.tray = Tray(self)
        self.tray.messageClicked.connect(self.on_message_clicked)
        self.tray.show()

        self.server = QLocalServer(self)
        QLocalServer.removeServer(server_name())
        self.server.listen(server_name())
        self.server.newConnection.connect(self.on_connection)

        if self.first_run:
            C.save(self.cfg)
            if not selftest:
                autostart.safe(autostart.set_enabled, True)
        elif not selftest:
            autostart.safe(autostart.refresh)
        self.recorder.start()
        threading.Thread(target=lambda: self.event.emit("devices", devices()), daemon=True).start()

        self.update_timer = QTimer(self, interval=6 * 3600 * 1000, timeout=lambda: self.check_updates(False))
        self.update_timer.start()
        QTimer.singleShot(60 * 1000, lambda: self.check_updates(False))
        self.tray_timer = QTimer(self, interval=2000, timeout=self.tray.update)
        self.tray_timer.start()

        if show or self.first_run:
            self.show_window()
            if self.first_run:
                QTimer.singleShot(400, lambda: self.window.toast.show_text(
                    f"Recording. Press {self.cfg['hotkey']} to save a replay.", 6000))
        if selftest:
            from .selftest import SelfTest
            self._selftest = SelfTest(self, selftest)

    # ------------------------------------------------------------------ settings
    def set(self, key, value):
        if self.cfg.get(key) == value:
            return
        self.cfg[key] = value
        self.save_timer.start()
        if key in C.RECORDER_KEYS and not self.paused():
            self.restart_timer.start()
        if key == "seconds":
            self.recorder.ring.seconds = value
        if key == "hotkey":
            self.hotkey.set(value)
        self.tray.update()

    def restart_recorder(self):
        if not self.paused():
            self.recorder.start()

    # ------------------------------------------------------------------ actions
    def paused(self):
        return self.recorder.state == "paused"

    def pause(self):
        self.recorder.pause()
        self.tray.update()

    def resume(self):
        if self.recorder.state in ("paused", "error", "off"):
            self.recorder.start()
        self.tray.update()

    def redetect(self):
        self.recorder.redetect()
        if self.window:
            self.window.toast.show_text("Checking the graphics card…")

    def save_replay(self, seconds=None):
        if self.recorder.save(seconds):
            return True
        st = self.recorder.state
        msg = "Recording is paused" if st == "paused" else "Nothing recorded yet" if st != "error" else \
            "Not recording: " + (self.recorder.error or "error")
        self.notify(msg)
        return False

    def open_folder(self):
        d = C.output_dir(self.cfg)
        os.makedirs(d, exist_ok=True)
        system.open_path(d)

    def show_window(self):
        if not self.window:
            from .window import Window
            self.window = Window(self)
            if self.update_ready:
                self.window.show_update(self.update_ready)
            if getattr(self, "devices", None):
                self.window.set_devices(*self.devices)
        self.window.showNormal()
        self.window.raise_()
        self.window.activateWindow()

    def window_hidden(self):
        if not self.cfg["tray_hint_shown"]:
            self.set("tray_hint_shown", True)
            self.tray.showMessage(C.APP_NAME, "Still recording here in the tray. Right click it to quit.",
                                  QIcon(C.ICON_PATH), 5000)

    def quit(self, code=0):
        if self._quitting:
            return
        self._quitting = True
        if self.save_timer.isActive():
            self.save_timer.stop()
            C.save(self.cfg)
        self.hotkey.stop()
        self.recorder.stop()
        self.recorder.write_log()
        self.tray.hide()
        self.server.close()
        self.qapp.exit(code)

    def notify(self, text):
        if self.window and self.window.isVisible() and self.window.isActiveWindow():
            self.window.toast.show_text(text)
        elif self.cfg["notifications"]:
            self.tray.showMessage(C.APP_NAME, text, QIcon(C.ICON_PATH), 4000)

    def on_message_clicked(self):
        if self.update_ready and self._last_message == "update":
            self.run_update()
        elif self.last_clip and os.path.exists(self.last_clip):
            system.show_in_folder(self.last_clip)

    _last_message = None

    # ------------------------------------------------------------------ events from threads
    def on_event(self, kind, data):
        if kind == "hotkey":
            self.save_replay()
        elif kind == "saving":
            if self.cfg["click_sound"]:
                play_click()
        elif kind == "saved":
            self.last_clip = data["path"]
            self._last_message = "clip"
            from .window import human_size, human_time
            self.notify(f"Replay saved · {human_time(data['length'])} · {human_size(data['size'])}")
            if self.window:
                self.window.load_clips(True)
        elif kind == "save_failed":
            self.notify(f"Couldn't save: {data}")
        elif kind == "state":
            self.tray.update()
            if self.window:
                self.window.poll()
        elif kind == "devices":
            self.devices = data
            if self.window:
                self.window.set_devices(*data)
        elif kind == "update":
            self.on_update(*data)
        elif kind == "update_file":
            self.install_update(data)
        elif kind == "update_failed":
            self.notify(f"Update failed: {data}")
            if self.window:
                self.window.banner.button.setEnabled(True)
                self.window.show_update(self.update_ready)

    # ------------------------------------------------------------------ updates
    def check_updates(self, manual):
        def work():
            latest = system.latest_version()
            if latest:
                C.write_json(C.UPDATE_CACHE, {**(C.read_json(C.UPDATE_CACHE) or {}), "time": time.time(),
                                              "latest": latest})
            self.event.emit("update", (latest, manual))
        threading.Thread(target=work, daemon=True).start()

    def on_update(self, latest, manual):
        newer = latest and system.version_tuple(latest) > system.version_tuple(C.VERSION)
        if newer:
            self.update_ready = latest
            self.tray.update()
        if self.window:
            self.window.show_update(latest)
        if newer and not manual:
            cache = C.read_json(C.UPDATE_CACHE) or {}
            if cache.get("notified") != latest or time.time() - cache.get("notified_at", 0) > 24 * 3600:
                C.write_json(C.UPDATE_CACHE, {**cache, "notified": latest, "notified_at": time.time()})
                self._last_message = "update"
                self.tray.showMessage(f"{C.APP_NAME} {latest} is available",
                                      "Click here or use Update in the tray menu.", QIcon(C.ICON_PATH), 8000)

    def run_update(self):
        if not C.FROZEN:
            system.open_path(f"https://github.com/{C.REPO}/releases/latest")
            return
        if self.window:
            self.window.update_progress("Downloading the update…")
        dest = os.path.join(tempfile.gettempdir(), f"{C.APP_NAME}-Setup.exe")

        def work():
            try:
                system.download(C.SETUP_URL, dest)
                self.event.emit("update_file", dest)
            except Exception as e:
                self.event.emit("update_failed", str(e))
        threading.Thread(target=work, daemon=True).start()

    def install_update(self, path):
        # the installer closes nothing of ours: we quit first, it starts the new version when done
        subprocess.Popen([path, "/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"],
                         creationflags=0x00000008 if C.WINDOWS else 0)     # DETACHED_PROCESS
        self.quit()

    # ------------------------------------------------------------------ second launches
    def on_connection(self):
        sock = self.server.nextPendingConnection()
        sock.waitForReadyRead(1000)
        cmd = bytes(sock.readAll()).decode("utf-8", "replace").strip().split()
        ok = True
        if not cmd or cmd[0] == "show":
            self.show_window()
        elif cmd[0] == "save":
            ok = self.save_replay(int(cmd[1]) if len(cmd) > 1 and cmd[1].isdigit() else None)
        elif cmd[0] == "ping":
            pass
        elif cmd[0] == "quit":
            QTimer.singleShot(0, self.quit)
        sock.write(b"ok\n" if ok else b"fail\n")
        sock.flush()
        sock.waitForBytesWritten(500)
        sock.disconnectFromServer()


def play_click():
    if C.WINDOWS:
        import winsound
        try:
            winsound.PlaySound(C.CLICK_SOUND, winsound.SND_FILENAME | winsound.SND_ASYNC | winsound.SND_NODEFAULT)
        except RuntimeError:
            pass


def run(qapp, show=True, selftest=None, updated=False):
    if C.WINDOWS:
        import ctypes
        try:
            ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("DRZ.DrzReplayWIN")
        except Exception:
            pass
    qapp.setApplicationName(C.APP_NAME)
    qapp.setApplicationDisplayName(C.APP_NAME)
    qapp.setStyle("Fusion")
    qapp.setQuitOnLastWindowClosed(False)
    qapp.setWindowIcon(QIcon(C.ICON_PATH))
    W.apply_theme(qapp)
    qapp.styleHints().colorSchemeChanged.connect(lambda _: W.apply_theme(qapp))
    if not QSystemTrayIcon.isSystemTrayAvailable() and not selftest:
        show = True
    app = App(qapp, show=show, selftest=selftest)
    if updated and not app.first_run:
        app.tray.showMessage(C.APP_NAME, f"Updated to {C.VERSION}. Still recording.", QIcon(C.ICON_PATH), 5000)
    qapp.aboutToQuit.connect(lambda: app.quit())
    return qapp.exec()
