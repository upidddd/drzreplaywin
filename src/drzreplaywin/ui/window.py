"""The DrzReplayWIN window: Record, Clips and Settings tabs."""
import hashlib
import os
import queue
import threading
from datetime import datetime, timedelta

from PySide6.QtCore import QMimeData, QPoint, QSize, Qt, QTimer, QUrl, Signal
from PySide6.QtGui import QDrag, QIcon, QKeySequence, QPixmap
from PySide6.QtWidgets import (QApplication, QButtonGroup, QComboBox, QDialog, QFileDialog, QHBoxLayout, QLabel,
                               QLineEdit, QMainWindow, QProgressBar, QPushButton, QScrollArea, QSlider, QSpinBox,
                               QStackedWidget, QVBoxLayout, QWidget)

from ..core import autostart, system
from ..core import config as C
from ..recorder import hotkey as HK
from . import widgets as W
from .widgets import T

FPS = [(30, "30"), (60, "60"), (120, "120")]
QUALITY = [("low", "Low"), ("balanced", "Medium"), ("high", "High"), ("ultra", "Maximum")]
FORMAT = [("mp4", "MP4"), ("mkv", "MKV")]
MAX_CLIPS_SHOWN = 60


def human_size(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit in ("B", "KB") else f"{n:.1f} {unit}"
        n /= 1024


def human_time(sec):
    sec = int(round(sec))
    return f"{sec // 60}:{sec % 60:02d}"


def human_date(ts):
    d = datetime.fromtimestamp(ts)
    today = datetime.now().date()
    if d.date() == today:
        return f"Today, {d:%H:%M}"
    if d.date() == today - timedelta(days=1):
        return f"Yesterday, {d:%H:%M}"
    return f"{d:%b} {d.day}, {d:%H:%M}"


def short_path(path):
    home = os.path.expanduser("~")
    return "~" + path[len(home):] if path.startswith(home + os.sep) else path


def file_mime(path):
    """Drag / clipboard data for a file: drop or paste it into Discord, Telegram, Explorer."""
    m = QMimeData()
    m.setUrls([QUrl.fromLocalFile(path)])
    return m


class ThumbLoader(QWidget):
    """Makes clip thumbnails and reads durations in a background thread, cached on disk."""
    done = Signal(str, str, float)

    def __init__(self):
        super().__init__()
        self.q = queue.Queue()
        os.makedirs(C.THUMBS, exist_ok=True)
        threading.Thread(target=self.work, daemon=True, name="thumbs").start()

    def request(self, path):
        self.q.put(path)

    def work(self):
        while True:
            path = self.q.get()
            try:
                st = os.stat(path)
            except OSError:
                continue
            key = hashlib.sha1(f"{path}|{st.st_mtime}|{st.st_size}".encode()).hexdigest()
            jpg, meta = os.path.join(C.THUMBS, key + ".jpg"), os.path.join(C.THUMBS, key + ".json")
            dur = (C.read_json(meta) or {}).get("duration")
            if dur is None:
                dur = system.media_duration(path)
                C.write_json(meta, {"duration": dur})
            if not os.path.exists(jpg):
                system.run([system.ffmpeg(), "-v", "error", "-y", "-ss", f"{min(1.0, dur / 2):.2f}", "-i", path,
                            "-frames:v", "1", "-q:v", "3", "-vf",
                            "scale=192:108:force_original_aspect_ratio=increase,crop=192:108", jpg],
                           timeout=20, priority=system.BELOW_NORMAL_PRIORITY_CLASS)
            self.done.emit(path, jpg if os.path.exists(jpg) else "", float(dur or 0))


class ClipRow(W.Row):
    def __init__(self, win, path, mtime, size):
        super().__init__(human_date(mtime), human_size(size), activatable=True)
        self.path, self.size = path, size
        self.setToolTip(os.path.basename(path))
        self.thumb = QLabel()
        self.thumb.setObjectName("thumb")
        self.thumb.setFixedSize(96, 54)
        self.add_prefix(self.thumb)
        self.add_suffix(W.IconButton("copy", "Copy (paste into Discord, Telegram…)", lambda: win.copy_clip(path)))
        self.add_suffix(W.IconButton("folder", "Show in folder", lambda: system.show_in_folder(path)))
        self.add_suffix(W.IconButton("trash", "Move to Recycle Bin", lambda: win.trash_clip(path)))
        self.clicked.connect(lambda: system.open_path(path))
        self._press = None

    def loaded(self, jpg, dur):
        if jpg:
            pm = QPixmap(jpg)
            if not pm.isNull():
                self.thumb.setPixmap(W.rounded(pm, 96, 54))
        if dur:
            self.set_subtitle(f"{human_time(dur)} · {human_size(self.size)}")

    def mousePressEvent(self, e):
        if e.button() == Qt.LeftButton:
            self._press = e.position().toPoint()
        super().mousePressEvent(e)

    def mouseMoveEvent(self, e):
        if self._press is not None and (e.position().toPoint() - self._press).manhattanLength() > 10:
            self._press = None
            drag = QDrag(self)
            drag.setMimeData(file_mime(self.path))
            if self.thumb.pixmap() and not self.thumb.pixmap().isNull():
                drag.setPixmap(self.thumb.pixmap())
                drag.setHotSpot(QPoint(48, 27))
            drag.exec(Qt.CopyAction)
            return
        super().mouseMoveEvent(e)

    def mouseReleaseEvent(self, e):
        if self._press is None:          # it was a drag
            return
        self._press = None
        super().mouseReleaseEvent(e)


class SecondsDialog(QDialog):
    def __init__(self, parent, top, value):
        super().__init__(parent)
        self.setWindowTitle("Save the last…")
        self.setModal(True)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(24, 20, 24, 20)
        lay.setSpacing(16)
        lay.addWidget(W.label("Save the last…", "big"))
        g = W.Group()
        row = W.Row("Seconds", f"Up to {top}")
        self.spin = QSpinBox(minimum=1, maximum=top, value=min(value, top))
        row.add_suffix(self.spin)
        g.add(row)
        lay.addWidget(g)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        cancel = QPushButton("Cancel")
        cancel.clicked.connect(self.reject)
        ok = QPushButton("Save")
        ok.setObjectName("accent")
        ok.setDefault(True)
        ok.clicked.connect(self.accept)
        buttons.addWidget(cancel)
        buttons.addWidget(ok)
        lay.addLayout(buttons)
        self.setMinimumWidth(340)
        self.spin.setFocus()
        self.spin.selectAll()


class HotkeyDialog(QDialog):
    MODIFIERS = {Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt, Qt.Key_Meta, Qt.Key_AltGr, Qt.Key_CapsLock,
                 Qt.Key_NumLock, Qt.Key_Super_L, Qt.Key_Super_R}
    QT_NAMES = {Qt.Key_Print: "PrintScreen", Qt.Key_ScrollLock: "ScrollLock", Qt.Key_PageUp: "PageUp",
                Qt.Key_PageDown: "PageDown", Qt.Key_Insert: "Insert", Qt.Key_Delete: "Delete",
                Qt.Key_Home: "Home", Qt.Key_End: "End", Qt.Key_Pause: "Pause", Qt.Key_Space: "Space",
                Qt.Key_Left: "Left", Qt.Key_Right: "Right", Qt.Key_Up: "Up", Qt.Key_Down: "Down"}

    def __init__(self, parent):
        super().__init__(parent)
        self.setWindowTitle("Hotkey")
        self.result_key = None
        lay = QVBoxLayout(self)
        lay.setContentsMargins(28, 24, 28, 20)
        lay.setSpacing(8)
        lay.addWidget(W.label("Press a new hotkey", "big"))
        self.hint = W.label("For example Alt+Z or F9. Esc to cancel.", "subtitle")
        lay.addWidget(self.hint)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        cancel = QPushButton("Cancel")
        cancel.setFocusPolicy(Qt.NoFocus)
        cancel.clicked.connect(self.reject)
        buttons.addWidget(cancel)
        lay.addSpacing(12)
        lay.addLayout(buttons)
        self.setMinimumWidth(360)

    def key_name(self, e):
        vk = e.nativeVirtualKey() if C.WINDOWS else 0
        if vk in HK.NAMES:
            return HK.NAMES[vk]
        k = e.key()
        if k in self.QT_NAMES:
            return self.QT_NAMES[k]
        if Qt.Key_F1 <= k <= Qt.Key_F24:
            return f"F{k - Qt.Key_F1 + 1}"
        text = QKeySequence(k).toString().upper()
        return text if text in HK.KEYS else None

    def keyPressEvent(self, e):
        k = e.key()
        if k in self.MODIFIERS:
            return
        mods = e.modifiers()
        if k == Qt.Key_Escape and not mods:
            self.reject()
            return
        name = self.key_name(e)
        if not name:
            self.hint.setText("This key can't be used. Try another one.")
            return
        mod_names = {n for flag, n in ((Qt.ControlModifier, "Ctrl"), (Qt.AltModifier, "Alt"),
                                       (Qt.ShiftModifier, "Shift"), (Qt.MetaModifier, "Win")) if mods & flag}
        if not mod_names and name not in HK.SOLO_OK:
            self.hint.setText("Use it with Ctrl, Alt, Shift or Win.")
            return
        self.result_key = HK.compose(mod_names, name)
        self.accept()


class Window(QMainWindow):
    def __init__(self, app):
        super().__init__()
        self.app = app
        self.cfg = app.cfg
        self.rows = {}
        self._busy = False
        self._clips_sig = None
        self.clip_rows = {}
        self.setWindowTitle(C.APP_NAME)
        self.setWindowIcon(QIcon(C.ICON_PATH))
        self.resize(660, 820)
        self.setMinimumSize(520, 480)

        central = QWidget()
        central.setObjectName("page")
        outer = QVBoxLayout(central)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        header = QHBoxLayout()
        header.setContentsMargins(12, 10, 12, 6)
        self.stack = QStackedWidget()
        self.tabs = QButtonGroup(self)
        menu = W.menu(self)
        menu.addAction("Recorder log", self.open_log)
        menu.addAction("Detect graphics card again", self.app.redetect)
        menu.addSeparator()
        menu.addAction(f"About {C.APP_NAME}", self.about)
        more = W.IconButton("more", "Menu")
        more.setMenu(menu)
        more.setPopupMode(more.ToolButtonPopupMode.InstantPopup)
        spacer = QWidget()
        spacer.setFixedWidth(more.sizeHint().width())
        header.addWidget(spacer)
        header.addStretch(1)
        for i, (text, icon_name) in enumerate((("Record", "record"), ("Clips", "clips"), ("Settings", "settings"))):
            tab = W.Tab(text, icon_name)
            self.tabs.addButton(tab, i)
            header.addWidget(tab)
        header.addStretch(1)
        header.addWidget(more)
        self.tabs.idToggled.connect(lambda i, on: on and self.stack.setCurrentIndex(i))
        outer.addLayout(header)

        self.banner = W.Banner("Update", self.app.run_update)
        outer.addWidget(self.banner)

        self.stack.addWidget(self.scroll(self.page_record()))
        self.stack.addWidget(self.page_clips())
        self.stack.addWidget(self.scroll(self.page_settings()))
        outer.addWidget(self.stack, 1)
        self.setCentralWidget(central)
        self.toast = W.Toast(central)
        self.tabs.button(0).setChecked(True)

        self.thumbs = ThumbLoader()
        self.thumbs.done.connect(self.thumb_ready)
        self.update_sensitivity()
        self.timer = QTimer(self, interval=1000, timeout=self.poll)
        self.timer.start()
        self.poll()

    # ------------------------------------------------------------------ building blocks
    def scroll(self, content):
        area = QScrollArea()
        area.setWidgetResizable(True)
        area.setFrameShape(QScrollArea.NoFrame)
        holder = QWidget()
        holder.setObjectName("page")
        lay = QHBoxLayout(holder)
        lay.setContentsMargins(24, 8, 24, 28)
        lay.addStretch(1)
        content.setMaximumWidth(620)
        content.setMinimumWidth(420)
        lay.addWidget(content, 100)
        lay.addStretch(1)
        area.setWidget(holder)
        return area

    def column(self):
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(26)
        return w, lay

    def set(self, key, value):
        if self._busy or self.cfg.get(key) == value:
            return
        self.app.set(key, value)
        self.update_sensitivity()

    def switch(self, title, key, subtitle=None):
        row = W.Row(title, subtitle)
        sw = row.add_suffix(W.Switch(bool(self.cfg[key])))
        sw.toggled.connect(lambda on: self.set(key, on))
        self.rows[key] = ("switch", sw, None)
        row.control = sw
        return row

    def combo(self, title, key, options, subtitle=None):
        row = W.Row(title, subtitle)
        box = row.add_suffix(QComboBox())
        box.setFocusPolicy(Qt.StrongFocus)
        self.fill_combo(key, box, options)
        box.currentIndexChanged.connect(lambda i: i >= 0 and self.set(key, box.itemData(i)))
        self.rows[key] = ("combo", box, None)
        row.control = box
        return row

    def fill_combo(self, key, box, options):
        box.blockSignals(True)
        box.clear()
        values = [v for v, _ in options]
        for v, text in options:
            box.addItem(text, v)
        if self.cfg[key] not in values:
            box.addItem(str(self.cfg[key]), self.cfg[key])
        box.setCurrentIndex(box.findData(self.cfg[key]))
        box.blockSignals(False)

    def spin(self, title, subtitle, key, lo, hi, step=1):
        row = W.Row(title, subtitle)
        sp = row.add_suffix(QSpinBox(minimum=lo, maximum=hi, singleStep=step, value=int(self.cfg[key])))
        sp.setFocusPolicy(Qt.StrongFocus)
        sp.setKeyboardTracking(False)
        sp.valueChanged.connect(lambda v: self.set(key, int(v)))
        self.rows[key] = ("spin", sp, None)
        row.control = sp
        return row

    def slider(self, title, key, lo=0, hi=200):
        row = W.Row(title)
        value = W.label(f"{self.cfg[key]}%", "dim")
        value.setFixedWidth(44)
        value.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        s = QSlider(Qt.Horizontal, minimum=lo, maximum=hi, singleStep=5, pageStep=10, value=int(self.cfg[key]))
        s.setFixedWidth(200)
        s.setFocusPolicy(Qt.StrongFocus)
        s.valueChanged.connect(lambda v: (value.setText(f"{v}%"), self.set(key, int(v))))
        row.add_suffix(value)
        row.add_suffix(s)
        self.rows[key] = ("slider", s, None)
        row.control = s
        return row

    # ------------------------------------------------------------------ Record tab
    def page_record(self):
        page, lay = self.column()

        g = W.Group()
        self.rec_row = W.Row("Recording", "Starting…")
        self.rec_dot = self.rec_row.add_prefix(W.Dot())
        self.rec_switch = self.rec_row.add_suffix(W.Switch(True))
        self.rec_switch.toggled.connect(self.on_rec_switch)
        g.add(self.rec_row)
        self.buf_row = W.Row("Buffer", "Empty")
        self.buf_bar = QProgressBar(textVisible=False, maximum=1000)
        self.buf_bar.setFixedWidth(170)
        self.buf_row.add_suffix(self.buf_bar)
        g.add(self.buf_row)
        lay.addWidget(g)

        box = QVBoxLayout()
        box.setSpacing(8)
        m = W.menu(self)
        m.addAction("Save last…", self.ask_seconds)
        self.save_btn = W.SplitButton("Save replay", m)
        self.save_btn.clicked.connect(lambda: self.app.save_replay())
        box.addWidget(self.save_btn, 0, Qt.AlignHCenter)
        self.save_hint = W.label("", "subtitle")
        box.addWidget(self.save_hint, 0, Qt.AlignHCenter)
        holder = QWidget()
        holder.setLayout(box)
        lay.addWidget(holder)

        g = W.Group("Video")
        g.add(self.spin("Length", "Seconds kept in memory", "seconds", 10, 600, 5))
        self.screen_row = g.add(self.combo("Screen", "monitor", [(0, "Main screen")]))
        g.add(self.combo("Frame rate", "fps", FPS))
        g.add(self.combo("Quality", "rec_quality", QUALITY))
        self.enc_row = g.add(self.combo("Encoder", "encoder", [("auto", "Auto")]))
        g.add(self.switch("Show cursor", "draw_mouse"))
        self.video_group = g
        lay.addWidget(g)

        g = W.Group("Sound")
        g.add(self.switch("System sound", "system_audio", "What you hear"))
        self.out_row = g.add(self.combo("Output", "system_device", [("", "Default")]))
        self.out_vol = g.add(self.slider("Volume", "system_volume"))
        g.add(self.switch("Microphone", "mic"))
        self.mic_row = g.add(self.combo("Input", "mic_device", [("", "Default")]))
        self.mic_vol = g.add(self.slider("Volume", "mic_volume"))
        lay.addWidget(g)
        lay.addStretch(1)
        self._devices_loaded = False
        self._probe_shown = None
        return page

    def set_devices(self, outs, ins):
        self.fill_combo("system_device", self.out_row.control, [("", "Default")] + [(n, n) for n in outs])
        self.fill_combo("mic_device", self.mic_row.control, [("", "Default")] + [(n, n) for n in ins])

    def show_probe(self, st):
        sig = (tuple((m["w"], m["h"]) for m in st["monitors"]), tuple(st["encoders"]))
        if sig == self._probe_shown or not st["monitors"]:
            return
        self._probe_shown = sig
        screens = [(i, f"Screen {i + 1} ({m['w']}×{m['h']})") for i, m in enumerate(st["monitors"])]
        self.fill_combo("monitor", self.screen_row.control, screens)
        self.video_group.set_row_visible(self.screen_row, len(screens) > 1)
        encs = [("auto", "Auto")] + [(e, system.ENCODER_LABELS[e]) for e in st["encoders"]]
        self.fill_combo("encoder", self.enc_row.control, encs)

    # ------------------------------------------------------------------ Clips tab
    def page_clips(self):
        self.clips_stack = QStackedWidget()
        page, lay = self.column()
        self.clips_group = W.Group("Saved clips")
        self.clips_group.add_header(W.IconButton("refresh", "Refresh", lambda: self.load_clips(True)))
        self.clips_group.add_header(W.IconButton("folder", "Open folder", self.app.open_folder))
        lay.addWidget(self.clips_group)
        self.clips_more = W.label("", "subtitle")
        self.clips_more.setAlignment(Qt.AlignCenter)
        lay.addWidget(self.clips_more)
        lay.addStretch(1)
        self.clips_stack.addWidget(self.scroll(page))

        empty = QWidget()
        el = QVBoxLayout(empty)
        el.addStretch(2)
        pic = QLabel()
        pic.setPixmap(W.icon("clips", T["dim"], 64).pixmap(64, 64))
        pic.setAlignment(Qt.AlignCenter)
        el.addWidget(pic)
        el.addSpacing(10)
        title = W.label("No clips yet", "big")
        title.setAlignment(Qt.AlignCenter)
        el.addWidget(title)
        self.empty_hint = W.label("", "dim")
        self.empty_hint.setAlignment(Qt.AlignCenter)
        el.addWidget(self.empty_hint)
        el.addSpacing(14)
        open_btn = QPushButton("Open folder")
        open_btn.clicked.connect(self.app.open_folder)
        el.addWidget(open_btn, 0, Qt.AlignHCenter)
        el.addStretch(3)
        self.clips_stack.addWidget(empty)
        return self.clips_stack

    def clip_files(self):
        out = []
        try:
            for e in os.scandir(C.output_dir(self.cfg)):
                if e.is_file() and e.name.lower().endswith((".mp4", ".mkv")):
                    st = e.stat()
                    out.append((st.st_mtime, st.st_size, e.path))
        except OSError:
            pass
        return sorted(out, reverse=True)

    def load_clips(self, force=False):
        clips = self.clip_files()
        sig = (C.output_dir(self.cfg), tuple(clips[:MAX_CLIPS_SHOWN]))
        if sig == self._clips_sig and not force:
            return
        self._clips_sig = sig
        total = sum(s for _, s, _ in clips)
        self.clips_group.set_description(f"{len(clips)} clip{'s' * (len(clips) != 1)} · {human_size(total)}"
                                         if clips else None)
        self.empty_hint.setText(f"Press {self.cfg['hotkey']} to save the last {self.cfg['seconds']} seconds")
        self.clips_stack.setCurrentIndex(0 if clips else 1)
        self.clips_group.clear()
        self.clip_rows = {}
        for mtime, size, path in clips[:MAX_CLIPS_SHOWN]:
            row = ClipRow(self, path, mtime, size)
            self.clips_group.add(row)
            self.clip_rows[path] = row
            self.thumbs.request(path)
        hidden = len(clips) - MAX_CLIPS_SHOWN
        self.clips_more.setVisible(hidden > 0)
        self.clips_more.setText(f"{hidden} older clip{'s' * (hidden != 1)} in the folder" if hidden > 0 else "")

    def thumb_ready(self, path, jpg, dur):
        row = self.clip_rows.get(path)
        if row:
            row.loaded(jpg, dur)

    def copy_clip(self, path):
        QApplication.clipboard().setMimeData(file_mime(path))
        self.toast.show_text("Copied — paste it into a chat")

    def trash_clip(self, path):
        from PySide6.QtCore import QFile
        if QFile.moveToTrash(path):
            self.toast.show_text("Moved to Recycle Bin")
        else:
            try:
                os.remove(path)
                self.toast.show_text("Deleted")
            except OSError as e:
                self.toast.show_text(f"Couldn't delete: {e.strerror}")
        self.load_clips(True)

    # ------------------------------------------------------------------ Settings tab
    def page_settings(self):
        page, lay = self.column()

        g = W.Group("Saving")
        self.dir_row = W.Row("Folder", short_path(C.output_dir(self.cfg)))
        self.dir_row.add_suffix(W.IconButton("folder", "Open", self.app.open_folder))
        change = QPushButton("Change")
        change.clicked.connect(self.pick_folder)
        self.dir_row.add_suffix(change)
        g.add(self.dir_row)
        row = W.Row("File name", "Followed by the date and time")
        name = QLineEdit(self.cfg["file_prefix"])
        name.setFixedWidth(180)
        bad = '\\/:*?"<>|'
        name.editingFinished.connect(
            lambda: self.set("file_prefix", "".join(c for c in name.text().strip() if c not in bad) or "DrzReplay"))
        row.add_suffix(name)
        g.add(row)
        g.add(self.combo("Format", "container", FORMAT))
        lay.addWidget(g)

        g = W.Group("Cleanup", "Oldest clips are deleted when a limit is reached")
        g.add(self.spin("Keep clips", "0 = keep all", "keep_max_clips", 0, 9999))
        g.add(self.spin("Max space, GB", "0 = no limit", "keep_max_gb", 0, 2000))
        lay.addWidget(g)

        g = W.Group("Controls")
        self.hk_row = W.Row("Hotkey")
        self.hk_label = self.hk_row.add_suffix(W.label(self.cfg["hotkey"], "dim"))
        hk_btn = QPushButton("Change")
        hk_btn.clicked.connect(self.change_hotkey)
        self.hk_row.add_suffix(hk_btn)
        g.add(self.hk_row)
        g.add(self.switch("Click sound", "click_sound", "When a replay is being saved"))
        g.add(self.switch("Notifications", "notifications"))
        lay.addWidget(g)

        g = W.Group("Startup")
        row = W.Row("Start with Windows", "Recording starts in the tray when you log in")
        self.auto_switch = row.add_suffix(W.Switch(autostart.enabled()))
        self.auto_switch.toggled.connect(lambda on: autostart.safe(autostart.set_enabled, on))
        self.auto_switch.setEnabled(C.FROZEN)
        g.add(row)
        lay.addWidget(g)

        g = W.Group("Updates")
        self.ver_row = W.Row(f"{C.APP_NAME} {C.VERSION}")
        self.ver_btn = QPushButton("Check")
        self.ver_btn.clicked.connect(self.on_ver_btn)
        self.ver_row.add_suffix(self.ver_btn)
        g.add(self.ver_row)
        lay.addWidget(g)
        lay.addStretch(1)
        return page

    def update_sensitivity(self):
        for w in (self.out_row, self.out_vol):
            w.setEnabled(bool(self.cfg["system_audio"]))
        for w in (self.mic_row, self.mic_vol):
            w.setEnabled(bool(self.cfg["mic"]))
        self.save_hint.setText(f"or press {self.cfg['hotkey']}")

    # ------------------------------------------------------------------ status
    def poll(self):
        st = self.app.recorder.status()
        state = st["state"]
        recording = state == "recording"
        self.rec_switch.set_quiet(state in ("recording", "starting", "probing"))
        self.rec_dot.set_on(recording)
        if recording:
            w, h = st["size"] or (0, 0)
            enc = system.ENCODER_LABELS.get(st["encoder"], st["encoder"] or "")
            extra = " · no sound" if st["audio_off"] and (self.cfg["system_audio"] or self.cfg["mic"]) else ""
            self.rec_row.set_subtitle(f"{w}×{h} · {st['fps']} fps · {enc}{extra}")
        elif state == "probing":
            self.rec_row.set_subtitle("Checking the graphics card…")
        elif state == "starting":
            self.rec_row.set_subtitle("Starting…")
        elif state == "paused":
            self.rec_row.set_subtitle("Paused")
        elif state == "error":
            self.rec_row.set_subtitle(st["error"] or "Error")
        else:
            self.rec_row.set_subtitle("Off")
        secs = float(st["seconds"])
        buffered = float(st["buffered"])
        self.buf_bar.setValue(int(1000 * min(1.0, buffered / secs)) if secs else 0)
        self.buf_row.set_subtitle(f"{buffered:.0f} of {secs:.0f} s · {st['buffer_mb']:.0f} MB in memory"
                                  if recording else "Empty")
        self.save_btn.setEnabled(recording and buffered >= 1)
        self.show_probe(st)
        hk = self.app.hotkey
        self.hk_row.set_subtitle("" if hk.ok or hk.error == "starting" else f"Not working: {hk.error}")
        self.load_clips()

    def on_rec_switch(self, on):
        if on:
            self.app.resume()
        else:
            self.app.pause()

    def ask_seconds(self):
        top = int(self.cfg["seconds"])
        dlg = SecondsDialog(self, top, int(self.cfg["custom_seconds"]))
        if dlg.exec():
            sec = dlg.spin.value()
            self.set("custom_seconds", sec)
            self.app.save_replay(sec)

    def pick_folder(self):
        d = QFileDialog.getExistingDirectory(self, "Folder for clips", C.output_dir(self.cfg))
        if d:
            d = os.path.normpath(d)
            self.set("output_dir", d)
            self.dir_row.set_subtitle(short_path(d))
            self.load_clips(True)

    def change_hotkey(self):
        self.app.hotkey.set("")            # free the old key so it can be pressed in the dialog
        dlg = HotkeyDialog(self)
        if dlg.exec() and dlg.result_key:
            self.set("hotkey", dlg.result_key)
            self.hk_label.setText(dlg.result_key)
            self.update_sensitivity()
        self.app.hotkey.set(self.cfg["hotkey"])

    # ------------------------------------------------------------------ updates
    def show_update(self, latest, checked=True):
        newer = latest and system.version_tuple(latest) > system.version_tuple(C.VERSION)
        self.ver_btn.setEnabled(True)
        if newer:
            self.ver_row.set_subtitle(f"Version {latest} is available")
            self.ver_btn.setText("Update")
            self.banner.text.setText(f"{C.APP_NAME} {latest} is available")
            self.banner.show()
        elif latest:
            self.ver_row.set_subtitle("Up to date")
        else:
            self.ver_row.set_subtitle("Couldn't check (offline?)" if checked else "")

    def on_ver_btn(self):
        if self.ver_btn.text() == "Update":
            self.app.run_update()
        else:
            self.ver_btn.setEnabled(False)
            self.ver_row.set_subtitle("Checking…")
            self.app.check_updates(manual=True)

    def update_progress(self, text):
        self.banner.text.setText(text)
        self.banner.button.setEnabled(False)
        self.banner.show()
        self.ver_row.set_subtitle(text)
        self.ver_btn.setEnabled(False)

    # ------------------------------------------------------------------ menu
    def open_log(self):
        self.app.recorder.write_log()
        if os.path.exists(C.RECORDER_LOG):
            system.open_path(C.RECORDER_LOG)

    def about(self):
        from PySide6.QtWidgets import QMessageBox
        box = QMessageBox(self)
        box.setWindowTitle(f"About {C.APP_NAME}")
        box.setIconPixmap(QIcon(C.ICON_PATH).pixmap(64, 64))
        box.setTextFormat(Qt.RichText)
        box.setText(f"<b style='font-size:13pt'>{C.APP_NAME}</b><br>Version {C.VERSION}")
        box.setInformativeText(
            f"Instant replay for Windows by DRZ.<br><a href='https://github.com/{C.REPO}'>github.com/{C.REPO}</a>"
            "<br><br>Uses FFmpeg (GPL), Qt (LGPL), PortAudio. Click sound by Brian MacIntosh (CC0).")
        box.exec()

    # ------------------------------------------------------------------ window
    def resizeEvent(self, e):
        super().resizeEvent(e)
        if self.toast.isVisible():
            self.toast.move((self.centralWidget().width() - self.toast.width()) // 2,
                            self.centralWidget().height() - self.toast.height() - 24)

    def closeEvent(self, e):
        # closing the window keeps recording in the tray
        e.ignore()
        self.hide()
        self.app.window_hidden()

    def changeEvent(self, e):
        super().changeEvent(e)
        if e.type() == e.Type.ActivationChange and self.isActiveWindow():
            self.load_clips()

    def sizeHint(self):
        return QSize(660, 820)
