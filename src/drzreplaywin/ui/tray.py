"""Tray icon: left click opens the window, middle click saves, right click shows the menu."""
import time

from PySide6.QtCore import QPointF
from PySide6.QtGui import QColor, QIcon, QPainter, QPen
from PySide6.QtWidgets import QSystemTrayIcon

from ..core import config as C
from . import widgets as W


class Tray(QSystemTrayIcon):
    def __init__(self, app):
        self.icon_on = QIcon(C.ICON_PATH)
        super().__init__(self.icon_on)
        self.app = app
        self.icon_off = QIcon(self.icon_on.pixmap(64, 64, QIcon.Disabled))
        self.icon_rec = with_red_dot(self.icon_on)
        self.current = None
        m = W.menu()
        self.item_update = m.addAction("Update", app.run_update)
        self.item_save = m.addAction("Save replay", lambda: app.save_replay())
        self.item_record = m.addAction("Start recording", app.toggle_recording)
        m.addSeparator()
        self.item_pause = m.addAction("Turn replays off", lambda: app.resume() if self.replays_off() else app.pause())
        m.addSeparator()
        m.addAction("Open folder", app.open_folder)
        m.addAction(f"Open {C.APP_NAME}", app.show_window)
        m.addSeparator()
        m.addAction("Restart", app.restart)      # when something got stuck
        m.addAction("Quit", app.quit)
        self.menu = m
        self.setContextMenu(m)
        self.activated.connect(self.on_activated)
        m.aboutToShow.connect(self.update)
        self.update()

    def on_activated(self, reason):
        if reason == QSystemTrayIcon.Trigger:
            self.app.show_window()
        elif reason == QSystemTrayIcon.MiddleClick:
            self.app.save_replay()

    def replays_off(self):
        return self.app.paused() or self.app.recorder.rec_only

    def update(self):
        a = self.app
        paused = a.paused()
        self.item_pause.setText("Turn replays on" if self.replays_off() else "Turn replays off")
        self.item_update.setVisible(bool(a.update_ready))
        if a.update_ready:
            self.item_update.setText(f"Update to {a.update_ready}…")
        self.item_save.setText(f"Save replay ({a.cfg['hotkey']})")
        on = a.recorder.state == "recording" and not a.recorder.rec_only
        self.item_save.setEnabled(on)
        rec = a.recorder.filerec
        if rec:
            mins, secs = divmod(int(time.time() - rec["t0"]), 60)
            self.item_record.setText(f"Stop recording · {mins}:{secs:02d}")
        else:
            key = f" ({a.cfg['record_hotkey']})" if a.cfg["record_hotkey"] else ""
            self.item_record.setText(f"Start recording{key}")
        self.item_record.setEnabled(on or paused or bool(rec))     # with replays off it records just the video
        # the red dot: the screen is being captured (replay buffer or a full recording)
        icon = self.icon_rec if a.recorder.state == "recording" or rec else \
            self.icon_off if paused or a.recorder.state == "error" else self.icon_on
        if icon is not self.current:
            self.current = icon
            self.setIcon(icon)
        state = {"recording": "recording", "paused": "paused", "error": "not recording"}.get(a.recorder.state,
                                                                                            "starting…")
        if rec:
            state = "recording a video"
        self.setToolTip(f"{C.APP_NAME}: {state}")


def with_red_dot(icon):
    """The app icon with a red dot in the bottom-right corner, at the sizes Windows uses in the tray."""
    out = QIcon()
    for size in (16, 20, 24, 32, 40, 48, 64):
        pm = icon.pixmap(size, size)
        p = QPainter(pm)
        p.setRenderHint(QPainter.Antialiasing)
        r = size * 0.24
        c = size - r - size * 0.04
        p.setPen(QPen(QColor("#202020"), max(1.0, size / 16)))
        p.setBrush(QColor("#e81123"))
        p.drawEllipse(QPointF(c, c), r, r)
        p.end()
        out.addPixmap(pm)
    return out
