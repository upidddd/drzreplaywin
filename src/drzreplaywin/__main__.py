"""DrzReplayWIN command line.

    DrzReplayWIN.exe                 open the window (starts recording if it isn't running)
    DrzReplayWIN.exe --background    start in the tray only (used by "Start with Windows")
    DrzReplayWIN.exe --updated       the same, plus an "Updated" notification (run by the installer)
    DrzReplayWIN.exe save            save a replay now
    DrzReplayWIN.exe record          start / stop recording a full video
    DrzReplayWIN.exe quit            stop recording and close
"""
import os
import sys


def send(cmd):
    """Hand the command to the running instance; None when there is none."""
    from PySide6.QtNetwork import QLocalSocket
    from .ui.app import server_name
    sock = QLocalSocket()
    sock.connectToServer(server_name())
    if not sock.waitForConnected(500):
        return None
    sock.write(cmd.encode())
    sock.flush()
    sock.waitForReadyRead(3000)
    return bytes(sock.readAll()).decode().strip() or "ok"


def setup_logging():
    """A windowed exe has no console: keep Python errors in a log file instead of losing them."""
    from .core import config as C
    if sys.stderr is None or C.FROZEN:
        os.makedirs(C.DATA_DIR, exist_ok=True)
        f = open(C.APP_LOG, "a", encoding="utf-8", buffering=1)
        sys.stdout = sys.stderr = f


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    from .core import config as C
    if "--version" in argv:
        print(C.VERSION)
        return 0
    if "--selftest" in argv:
        out = os.path.abspath(argv[argv.index("--selftest") + 1])
        os.makedirs(out, exist_ok=True)
        os.environ["DRZREPLAYWIN_HOME"] = os.path.join(out, "home")
        import importlib
        importlib.reload(C)                 # pick up the test home
        setup_logging()
        from PySide6.QtWidgets import QApplication
        from .ui.app import run
        return run(QApplication(sys.argv[:1]), show=True, selftest=out)

    setup_logging()
    cmd = argv[0] if argv and not argv[0].startswith("-") else "show"
    from PySide6.QtWidgets import QApplication
    qapp = QApplication(sys.argv[:1])
    reply = send("ping" if "--background" in argv or "--updated" in argv else cmd)
    if reply is not None:                   # already running: it did the job
        return 0 if reply == "ok" else 1
    if cmd in ("save", "record", "quit"):
        print(f"{C.APP_NAME} is not running", file=sys.stderr)
        return 1
    from .ui.app import run
    quiet = "--background" in argv or "--updated" in argv
    return run(qapp, show=not quiet, updated="--updated" in argv)


if __name__ == "__main__":
    sys.exit(main())
