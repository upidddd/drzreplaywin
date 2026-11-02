"""Renders assets/icons/drzreplaywin.svg into the .ico (exe, installer, tray) and .png files."""
import io
import os
import sys

from PIL import Image
from PySide6.QtCore import QBuffer, QByteArray, Qt
from PySide6.QtGui import QGuiApplication, QImage, QPainter
from PySide6.QtSvg import QSvgRenderer

ICONS = os.path.join(os.path.dirname(__file__), "..", "src", "drzreplaywin", "assets", "icons")
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def render(svg, size):
    img = QImage(size, size, QImage.Format_ARGB32)
    img.fill(Qt.transparent)
    p = QPainter(img)
    p.setRenderHint(QPainter.Antialiasing)
    svg.render(p)
    p.end()
    buf = QBuffer()
    buf.open(QBuffer.WriteOnly)
    img.save(buf, "PNG")
    return Image.open(io.BytesIO(bytes(buf.data()))).convert("RGBA")


def main():
    QGuiApplication(sys.argv[:1] + ["-platform", "offscreen"])
    svg = QSvgRenderer(os.path.join(ICONS, "drzreplaywin.svg"))
    imgs = [render(svg, s) for s in SIZES]
    imgs[-1].save(os.path.join(ICONS, "drzreplaywin.ico"), sizes=[(s, s) for s in SIZES], append_images=imgs[:-1])
    imgs[-1].save(os.path.join(ICONS, "drzreplaywin.png"))
    print("written", os.path.normpath(ICONS))


if __name__ == "__main__":
    main()
