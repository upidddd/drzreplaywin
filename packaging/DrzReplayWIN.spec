# PyInstaller build of DrzReplayWIN.exe (a folder build: starts fast, fewer antivirus false alarms)
import os

ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))
SRC = os.path.join(ROOT, "src")

a = Analysis(
    [os.path.join(SPECPATH, "entry.py")],
    pathex=[SRC],
    datas=[(os.path.join(SRC, "drzreplaywin", "assets"), "assets"),
           (os.path.join(ROOT, "build", "ffmpeg", "ffmpeg.exe"), "ffmpeg")],
    hiddenimports=["pyaudiowpatch"],
    excludes=["tkinter", "unittest", "pydoc", "PySide6.QtQml", "PySide6.QtQuick", "PySide6.QtOpenGL",
              "PySide6.QtPdf", "PySide6.QtSql", "PySide6.QtTest", "PySide6.QtXml", "PySide6.QtDBus"],
)
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name="DrzReplayWIN", console=False, upx=False,
          icon=os.path.join(SRC, "drzreplaywin", "assets", "icons", "drzreplaywin.ico"),
          version=os.path.join(ROOT, "build", "version.txt"))
coll = COLLECT(exe, a.binaries, a.datas, name="DrzReplayWIN", upx=False)
