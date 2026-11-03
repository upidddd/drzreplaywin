"""The "Replay saved" badge in the corner of the screen.

Windows hides notifications while a game is running (Do not disturb when playing games), so a save
is confirmed by a small always-on-top window instead. It never takes focus and lets clicks through.
"""
from PySide6.QtCore import QRectF, Qt, QTimer
from PySide6.QtGui import QColor, QFont, QFontMetrics, QGuiApplication, QPainter
from PySide6.QtWidgets import QWidget

from . import widgets as W

MARGIN = 24


class Overlay(QWidget):
    def __init__(self):
        super().__init__(None, Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
                         | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus | Qt.NoDropShadowWindowHint)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        self.title, self.detail, self.ok = "", "", True
        self.title_font = QFont(self.font())
        self.title_font.setPointSizeF(10.5)
        self.title_font.setWeight(QFont.DemiBold)
        self.detail_font = QFont(self.font())
        self.detail_font.setPointSizeF(9)
        self.timer = QTimer(self, singleShot=True, timeout=self.hide)

    def show_text(self, title, detail="", ok=True, ms=2800):
        self.title, self.detail, self.ok = title, detail, ok
        tw = QFontMetrics(self.title_font).horizontalAdvance(title)
        dw = QFontMetrics(self.detail_font).horizontalAdvance(detail) if detail else 0
        self.resize(max(tw, dw) + 64, 58 if detail else 44)
        screen = QGuiApplication.primaryScreen().availableGeometry()
        self.move(screen.right() - self.width() - MARGIN, screen.top() + MARGIN)
        self.show()
        self.raise_()
        self.update()
        self.timer.start(ms)

    def paintEvent(self, _):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setPen(QColor(255, 255, 255, 30))
        p.setBrush(QColor(32, 32, 32, 235))
        p.drawRoundedRect(QRectF(0.5, 0.5, self.width() - 1, self.height() - 1), 8, 8)
        # status dot: blue = saved, red = failed
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(W.DARK["accent"] if self.ok else "#ff6b6b"))
        p.drawEllipse(QRectF(16, self.height() / 2 - 5, 10, 10))
        p.setPen(QColor("#ffffff"))
        p.setFont(self.title_font)
        if self.detail:
            p.drawText(QRectF(38, 9, self.width() - 46, 22), Qt.AlignLeft | Qt.AlignVCenter, self.title)
            p.setPen(QColor("#c5c5c5"))
            p.setFont(self.detail_font)
            p.drawText(QRectF(38, 29, self.width() - 46, 20), Qt.AlignLeft | Qt.AlignVCenter, self.detail)
        else:
            p.drawText(QRectF(38, 0, self.width() - 46, self.height()), Qt.AlignLeft | Qt.AlignVCenter, self.title)
