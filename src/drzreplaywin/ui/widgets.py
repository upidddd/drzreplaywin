"""Theme (Windows 11 light / dark) and the small widgets the window is built from."""
import os

from PySide6.QtCore import (QByteArray, QEasingCurve, QPoint, QRectF, QSize, Qt, QTimer, QVariantAnimation,
                            Signal)
from PySide6.QtGui import QColor, QIcon, QPainter, QPainterPath, QPalette, QPixmap
from PySide6.QtSvg import QSvgRenderer
from PySide6.QtWidgets import (QAbstractButton, QApplication, QFrame, QHBoxLayout, QLabel, QMenu, QPushButton,
                               QSizePolicy, QToolButton, QVBoxLayout, QWidget)

from ..core import config as C

LIGHT = dict(bg="#f3f3f3", card="#fbfbfb", border="#e5e5e5", sep="#ebebeb", text="#1a1a1a", dim="#5d5d5d",
             hover="rgba(0,0,0,0.045)", press="rgba(0,0,0,0.075)", control="#ffffff", control_hover="#f9f9f9",
             control_border="#d5d5d5", accent="#0078d4", accent_hover="#1a86d9", accent_press="#006cbe",
             accent_text="#ffffff", track="#d4d4d4", banner="#dcebf8", toast="#2b2b2b", toast_text="#ffffff",
             rec="#c42b1c", thumb="#e6e6e6")
DARK = dict(bg="#202020", card="#2b2b2b", border="#1c1c1c", sep="#232323", text="#ffffff", dim="#c5c5c5",
            hover="rgba(255,255,255,0.06)", press="rgba(255,255,255,0.09)", control="#373737",
            control_hover="#3c3c3c", control_border="#434343", accent="#4cc2ff", accent_hover="#47b1e8",
            accent_press="#42a1d2", accent_text="#000000", track="#5a5a5a", banner="#1f3447", toast="#f2f2f2",
            toast_text="#1a1a1a", rec="#ff99a4", thumb="#3a3a3a")
T = dict(LIGHT)

# stroke icons in the style of Lucide (ISC); "C" is replaced by the colour
ICONS = {
    "record": '<circle cx="12" cy="12" r="9" fill="none" stroke="C" stroke-width="2"/><circle cx="12" cy="12" r="4.5" fill="C"/>',
    "clips": '<path d="m16 13 5.2 3.5a.5.5 0 0 0 .8-.4V7.9a.5.5 0 0 0-.8-.4L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>',
    "settings": '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
    "folder": '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
    "copy": '<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
    "trash": '<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
    "refresh": '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>',
    "more": '<circle cx="5" cy="12" r="1.3" fill="C"/><circle cx="12" cy="12" r="1.3" fill="C"/><circle cx="19" cy="12" r="1.3" fill="C"/>',
    "chevron": '<path d="m6 9 6 6 6-6"/>',
    "chevron-up": '<path d="m18 15-6-6-6 6"/>',
    "play": '<path d="M7 4.5v15l12-7.5z" fill="C"/>',
}


def svg(name, color, width=1.75):
    body = ICONS[name].replace('"C"', f'"{color}"')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{color}" '
            f'stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round">{body}</svg>')


def icon(name, color=None, size=16):
    color = color or T["text"]
    pm = QPixmap(size * 3, size * 3)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    QSvgRenderer(QByteArray(svg(name, color).encode())).render(p)
    p.end()
    pm.setDevicePixelRatio(3)
    return QIcon(pm)


def is_dark():
    return T is not LIGHT and T["bg"] == DARK["bg"]


def apply_theme(app, dark=None):
    if dark is None:
        dark = app.styleHints().colorScheme() == Qt.ColorScheme.Dark
    T.clear()
    T.update(DARK if dark else LIGHT)
    # arrows for combo / spin boxes are files referenced from the style sheet
    d = os.path.join(C.DATA_DIR, "ui")
    os.makedirs(d, exist_ok=True)
    arrows = {}
    for name in ("chevron", "chevron-up"):
        path = os.path.join(d, f"{name}-{'dark' if dark else 'light'}.svg")
        with open(path, "w", encoding="utf-8") as f:
            f.write(svg(name, T["dim"], 2))
        arrows[name] = path.replace("\\", "/")

    pal = QPalette()
    for role, key in ((QPalette.Window, "bg"), (QPalette.Base, "control"), (QPalette.AlternateBase, "card"),
                      (QPalette.Button, "control"), (QPalette.WindowText, "text"), (QPalette.Text, "text"),
                      (QPalette.ButtonText, "text"), (QPalette.ToolTipBase, "card"), (QPalette.ToolTipText, "text"),
                      (QPalette.Highlight, "accent"), (QPalette.HighlightedText, "accent_text"),
                      (QPalette.PlaceholderText, "dim")):
        pal.setColor(role, QColor(T[key]))
    app.setPalette(pal)
    app.setStyleSheet(STYLE.format(**T, down=arrows["chevron"], up=arrows["chevron-up"]))
    for w in app.allWidgets():
        if hasattr(w, "retheme"):
            w.retheme()


STYLE = """
QWidget {{ color: {text}; font-family: "Segoe UI Variable Text", "Segoe UI", "Noto Sans", sans-serif; font-size: 10pt; }}
QMainWindow, #page, QDialog {{ background: {bg}; }}
QScrollArea {{ background: transparent; border: none; }}
QScrollBar:vertical {{ background: transparent; width: 10px; margin: 2px; }}
QScrollBar::handle:vertical {{ background: {track}; border-radius: 3px; min-height: 30px; margin: 0 2px; }}
QScrollBar::handle:vertical:hover {{ margin: 0; border-radius: 4px; }}
QScrollBar::add-line, QScrollBar::sub-line, QScrollBar::add-page, QScrollBar::sub-page {{ height: 0; background: none; }}
#card {{ background: {card}; border: 1px solid {border}; border-radius: 8px; }}
#sep {{ background: {sep}; border: none; min-height: 1px; max-height: 1px; }}
#row {{ background: transparent; border-radius: 7px; }}
#row[activatable="true"]:hover {{ background: {hover}; }}
#groupTitle {{ font-weight: 600; font-size: 10.5pt; }}
#dim {{ color: {dim}; }}
#subtitle {{ color: {dim}; font-size: 9pt; }}
#big {{ font-size: 14pt; font-weight: 600; }}
QLabel {{ background: transparent; }}

QPushButton {{ background: {control}; border: 1px solid {control_border}; border-radius: 5px; padding: 5px 14px; min-height: 20px; }}
QPushButton:hover {{ background: {control_hover}; }}
QPushButton:pressed {{ background: {bg}; color: {dim}; }}
QPushButton:disabled {{ color: {dim}; }}
QPushButton#accent, QPushButton#saveMain, QToolButton#accentDrop {{ background: {accent}; color: {accent_text}; border: 1px solid {accent}; font-weight: 600; }}
QPushButton#accent:hover, QPushButton#saveMain:hover, QToolButton#accentDrop:hover {{ background: {accent_hover}; border-color: {accent_hover}; }}
QPushButton#accent:pressed, QPushButton#saveMain:pressed, QToolButton#accentDrop:pressed {{ background: {accent_press}; }}
QPushButton#accent:disabled, QPushButton#saveMain:disabled, QToolButton#accentDrop:disabled {{ background: {track}; border-color: {track}; color: {card}; }}
QPushButton#saveMain {{ border-top-right-radius: 0; border-bottom-right-radius: 0; padding: 0 28px; min-height: 38px; font-size: 10.5pt; }}
QToolButton#accentDrop {{ border-top-left-radius: 0; border-bottom-left-radius: 0; border-left: 1px solid {accent_press}; min-height: 38px; min-width: 34px; border-radius: 5px; }}
QToolButton#accentDrop::menu-indicator {{ image: none; width: 0; }}
QToolButton#flat {{ background: transparent; border: none; border-radius: 5px; padding: 6px; }}
QToolButton#flat:hover {{ background: {hover}; }}
QToolButton#flat:pressed {{ background: {press}; }}
QToolButton#flat::menu-indicator {{ image: none; width: 0; }}
QToolButton#tab {{ background: transparent; border: none; border-radius: 6px; padding: 7px 14px 9px 14px; color: {dim}; font-size: 10pt; }}
QToolButton#tab:hover {{ background: {hover}; color: {text}; }}
QToolButton#tab:checked {{ color: {text}; font-weight: 600; }}

QComboBox, QSpinBox, QLineEdit {{ background: {control}; border: 1px solid {control_border}; border-radius: 5px; padding: 4px 10px; min-height: 22px; selection-background-color: {accent}; selection-color: {accent_text}; }}
QComboBox:hover, QSpinBox:hover, QLineEdit:hover {{ background: {control_hover}; }}
QLineEdit:focus, QSpinBox:focus {{ border-bottom: 2px solid {accent}; }}
QComboBox {{ min-width: 150px; padding-right: 28px; }}
QComboBox::drop-down {{ border: none; width: 28px; }}
QComboBox::down-arrow {{ image: url("{down}"); width: 12px; height: 12px; }}
QComboBox QAbstractItemView {{ background: {card}; border: 1px solid {control_border}; border-radius: 6px; padding: 4px; outline: 0; selection-background-color: {hover}; selection-color: {text}; }}
QComboBox QAbstractItemView::item {{ min-height: 30px; padding: 0 8px; border-radius: 4px; }}
QSpinBox {{ padding-left: 8px; padding-right: 24px; min-width: 70px; }}
QSpinBox::up-button, QSpinBox::down-button {{ border: none; background: transparent; width: 22px; subcontrol-origin: border; }}
QSpinBox::up-button {{ subcontrol-position: top right; margin: 3px 2px 0 0; }}
QSpinBox::down-button {{ subcontrol-position: bottom right; margin: 0 2px 3px 0; }}
QSpinBox::up-button:hover, QSpinBox::down-button:hover {{ background: {hover}; border-radius: 3px; }}
QSpinBox::up-arrow {{ image: url("{up}"); width: 10px; height: 10px; }}
QSpinBox::down-arrow {{ image: url("{down}"); width: 10px; height: 10px; }}

QSlider {{ min-height: 22px; }}
QSlider::groove:horizontal {{ height: 4px; background: {track}; border-radius: 2px; }}
QSlider::sub-page:horizontal {{ height: 4px; background: {accent}; border-radius: 2px; }}
QSlider::handle:horizontal {{ background: {accent}; border: 4px solid {control}; width: 10px; height: 10px; margin: -8px 0; border-radius: 9px; }}
QSlider::handle:horizontal:hover {{ border-width: 3px; }}
QProgressBar {{ background: {track}; border: none; border-radius: 2px; max-height: 4px; min-height: 4px; }}
QProgressBar::chunk {{ background: {accent}; border-radius: 2px; }}

QMenu {{ background: {card}; border: 1px solid {control_border}; border-radius: 8px; padding: 4px; }}
QMenu::item {{ padding: 7px 28px 7px 12px; border-radius: 4px; }}
QMenu::item:selected {{ background: {hover}; }}
QMenu::item:disabled {{ color: {dim}; }}
QMenu::separator {{ height: 1px; background: {control_border}; margin: 4px 6px; }}
QToolTip {{ background: {card}; color: {text}; border: 1px solid {control_border}; padding: 4px 8px; }}
#banner {{ background: {banner}; }}
#toast {{ background: {toast}; color: {toast_text}; border-radius: 6px; padding: 10px 18px; }}
#thumb {{ background: {thumb}; border-radius: 4px; }}
"""


class Switch(QAbstractButton):
    """Windows 11 toggle switch."""

    def __init__(self, checked=False, parent=None):
        super().__init__(parent)
        self.setCheckable(True)
        self.setChecked(checked)
        self.setCursor(Qt.PointingHandCursor)
        self.setFixedSize(42, 22)
        self._pos = 1.0 if checked else 0.0
        self._anim = QVariantAnimation(self, duration=120, easingCurve=QEasingCurve.OutCubic)
        self._anim.valueChanged.connect(self._moved)
        self.toggled.connect(self._animate)

    def _moved(self, v):
        self._pos = v
        self.update()

    def _animate(self, on):
        self._anim.stop()
        self._anim.setStartValue(self._pos)
        self._anim.setEndValue(1.0 if on else 0.0)
        self._anim.start()

    def set_quiet(self, on):
        """Change the state without animating or emitting."""
        self.blockSignals(True)
        self.setChecked(on)
        self.blockSignals(False)
        self._pos = 1.0 if on else 0.0
        self.update()

    def sizeHint(self):
        return QSize(42, 22)

    def paintEvent(self, _):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        r = QRectF(1, 1, 40, 20)
        on = self.isChecked()
        enabled = self.isEnabled()
        if on:
            p.setPen(Qt.NoPen)
            p.setBrush(QColor(T["accent"] if enabled else T["track"]))
        else:
            p.setPen(QColor(T["dim"] if enabled else T["track"]))
            p.setBrush(QColor(T["control"]))
        p.drawRoundedRect(r, 10, 10)
        d = 14 if self.underMouse() and enabled else 12
        x = 11 + self._pos * 20
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(T["accent_text"] if on and enabled else (T["dim"] if enabled else T["track"])))
        p.drawEllipse(QRectF(x - d / 2, 11 - d / 2, d, d))

    def enterEvent(self, e):
        self.update()
        super().enterEvent(e)

    def leaveEvent(self, e):
        self.update()
        super().leaveEvent(e)


def label(text="", name=None, wrap=False):
    lb = QLabel(text)
    if name:
        lb.setObjectName(name)
    lb.setWordWrap(wrap)
    return lb


class Row(QFrame):
    """A settings row: title and subtitle on the left, controls on the right."""
    clicked = Signal()

    def __init__(self, title, subtitle=None, activatable=False):
        super().__init__()
        self.setObjectName("row")
        self.setProperty("activatable", activatable)
        self.activatable = activatable
        if activatable:
            self.setCursor(Qt.PointingHandCursor)
        lay = QHBoxLayout(self)
        lay.setContentsMargins(16, 10, 12, 10)
        lay.setSpacing(12)
        self.prefix = QHBoxLayout()
        self.prefix.setSpacing(12)
        lay.addLayout(self.prefix)
        text = QVBoxLayout()
        text.setSpacing(1)
        self.title = label(title)
        self.subtitle = label(subtitle or "", "subtitle")
        self.subtitle.setVisible(bool(subtitle))
        text.addStretch(1)
        text.addWidget(self.title)
        text.addWidget(self.subtitle)
        text.addStretch(1)
        lay.addLayout(text, 1)
        self.suffix = QHBoxLayout()
        self.suffix.setSpacing(6)
        lay.addLayout(self.suffix)
        self.setMinimumHeight(52)

    def set_subtitle(self, text):
        self.subtitle.setText(text or "")
        self.subtitle.setVisible(bool(text))

    def add_prefix(self, w):
        self.prefix.addWidget(w)
        return w

    def add_suffix(self, w):
        self.suffix.addWidget(w, 0, Qt.AlignVCenter)
        return w

    def mouseReleaseEvent(self, e):
        if self.activatable and e.button() == Qt.LeftButton and self.rect().contains(e.position().toPoint()):
            self.clicked.emit()
        super().mouseReleaseEvent(e)


class Group(QWidget):
    """A titled card of rows, like a libadwaita preferences group."""

    def __init__(self, title=None, description=None):
        super().__init__()
        lay = QVBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(6)
        head = QHBoxLayout()
        head.setContentsMargins(2, 0, 0, 0)
        titles = QVBoxLayout()
        titles.setSpacing(0)
        self.title = label(title or "", "groupTitle")
        self.description = label(description or "", "subtitle", wrap=True)
        titles.addWidget(self.title)
        titles.addWidget(self.description)
        self.title.setVisible(bool(title))
        self.description.setVisible(bool(description))
        head.addLayout(titles, 1)
        self.head_suffix = QHBoxLayout()
        self.head_suffix.setSpacing(2)
        head.addLayout(self.head_suffix)
        self.head = head
        lay.addLayout(head)
        self.card = QFrame()
        self.card.setObjectName("card")
        self.rows = QVBoxLayout(self.card)
        self.rows.setContentsMargins(2, 2, 2, 2)
        self.rows.setSpacing(0)
        lay.addWidget(self.card)
        self.items = []

    def set_description(self, text):
        self.description.setText(text or "")
        self.description.setVisible(bool(text))

    def add_header(self, w):
        self.head_suffix.addWidget(w, 0, Qt.AlignBottom)
        return w

    def add(self, row):
        if self.items:
            sep = QFrame()
            sep.setObjectName("sep")
            self.rows.addWidget(sep)
            row._sep = sep
        else:
            row._sep = None
        self.rows.addWidget(row)
        self.items.append(row)
        return row

    def clear(self):
        while self.rows.count():
            w = self.rows.takeAt(0).widget()
            if w:
                w.deleteLater()
        self.items = []

    def set_row_visible(self, row, on):
        row.setVisible(on)
        if row._sep:
            row._sep.setVisible(on)


class IconButton(QToolButton):
    def __init__(self, name, tip=None, cb=None, size=16):
        super().__init__()
        self.setObjectName("flat")
        self.name, self.size_ = name, size
        self.setIconSize(QSize(size, size))
        self.setCursor(Qt.PointingHandCursor)
        if tip:
            self.setToolTip(tip)
        if cb:
            self.clicked.connect(cb)
        self.retheme()

    def retheme(self):
        self.setIcon(icon(self.name, T["text"], self.size_))


class Tab(QToolButton):
    """Header tab: icon + text, an accent pill under the selected one."""

    def __init__(self, text, name):
        super().__init__()
        self.setObjectName("tab")
        self.setText(text)
        self.name = name
        self.setCheckable(True)
        self.setAutoExclusive(True)
        self.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
        self.setIconSize(QSize(16, 16))
        self.setCursor(Qt.PointingHandCursor)
        self.toggled.connect(lambda _: self.retheme())
        self.retheme()

    def retheme(self):
        self.setIcon(icon(self.name, T["text"] if self.isChecked() else T["dim"]))

    def paintEvent(self, e):
        super().paintEvent(e)
        if self.isChecked():
            p = QPainter(self)
            p.setRenderHint(QPainter.Antialiasing)
            p.setPen(Qt.NoPen)
            p.setBrush(QColor(T["accent"]))
            w = 18
            p.drawRoundedRect(QRectF((self.width() - w) / 2, self.height() - 3, w, 3), 1.5, 1.5)


class Dot(QWidget):
    """The red recording dot."""

    def __init__(self):
        super().__init__()
        self.setFixedSize(20, 20)
        self.on = False

    def set_on(self, on):
        if on != self.on:
            self.on = on
            self.update()

    def paintEvent(self, _):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        c = QColor(T["rec"] if self.on else T["dim"])
        p.setPen(c)
        p.setBrush(Qt.NoBrush)
        p.drawEllipse(QRectF(2, 2, 16, 16))
        p.setPen(Qt.NoPen)
        p.setBrush(c)
        p.drawEllipse(QRectF(6, 6, 8, 8))

    def retheme(self):
        self.update()


class SplitButton(QWidget):
    """Accent button with a dropdown arrow on its right."""
    clicked = Signal()

    def __init__(self, text, menu):
        super().__init__()
        lay = QHBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(0)
        self.main = QPushButton(text)
        self.main.setObjectName("saveMain")
        self.main.setCursor(Qt.PointingHandCursor)
        self.main.clicked.connect(self.clicked)
        self.drop = QToolButton()
        self.drop.setObjectName("accentDrop")
        self.drop.setMenu(menu)
        self.drop.setPopupMode(QToolButton.InstantPopup)
        self.drop.setCursor(Qt.PointingHandCursor)
        self.drop.setToolTip("Save only the last few seconds")
        lay.addWidget(self.main)
        lay.addWidget(self.drop)
        self.retheme()

    def retheme(self):
        self.drop.setIcon(icon("chevron", T["accent_text"] if self.isEnabled() else T["card"]))

    def changeEvent(self, e):
        super().changeEvent(e)
        if e.type() == e.Type.EnabledChange:
            self.retheme()


class Banner(QFrame):
    def __init__(self, button_text, cb):
        super().__init__()
        self.setObjectName("banner")
        lay = QHBoxLayout(self)
        lay.setContentsMargins(20, 8, 12, 8)
        self.text = label("")
        lay.addWidget(self.text, 1)
        self.button = QPushButton(button_text)
        self.button.setObjectName("accent")
        self.button.setCursor(Qt.PointingHandCursor)
        self.button.clicked.connect(cb)
        lay.addWidget(self.button)
        self.hide()


class Toast(QLabel):
    """Short message floating at the bottom of the window."""

    def __init__(self, parent):
        super().__init__(parent)
        self.setObjectName("toast")
        self.setAlignment(Qt.AlignCenter)
        self.timer = QTimer(self, singleShot=True, timeout=self.hide)
        self.hide()

    def show_text(self, text, ms=3000):
        self.setText(text)
        self.adjustSize()
        par = self.parentWidget()
        self.move((par.width() - self.width()) // 2, par.height() - self.height() - 24)
        self.raise_()
        self.show()
        self.timer.start(ms)


def rounded(pm, w, h, radius=4):
    """Crop a pixmap to w×h (cover) with rounded corners, at the screen's pixel ratio."""
    ratio = QApplication.primaryScreen().devicePixelRatio() if QApplication.primaryScreen() else 1
    out = QPixmap(int(w * ratio), int(h * ratio))
    out.fill(Qt.transparent)
    out.setDevicePixelRatio(ratio)
    scaled = pm.scaled(int(w * ratio), int(h * ratio), Qt.KeepAspectRatioByExpanding, Qt.SmoothTransformation)
    scaled.setDevicePixelRatio(ratio)
    p = QPainter(out)
    p.setRenderHint(QPainter.Antialiasing)
    path = QPainterPath()
    path.addRoundedRect(QRectF(0, 0, w, h), radius, radius)
    p.setClipPath(path)
    sw, sh = scaled.width() / ratio, scaled.height() / ratio
    p.drawPixmap(QPoint(int((w - sw) / 2), int((h - sh) / 2)), scaled)
    p.end()
    return out


def menu(parent=None):
    m = QMenu(parent)
    m.setWindowFlags(m.windowFlags() | Qt.FramelessWindowHint | Qt.NoDropShadowWindowHint)
    m.setAttribute(Qt.WA_TranslucentBackground)
    return m


def expanding():
    w = QWidget()
    w.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Preferred)
    return w
