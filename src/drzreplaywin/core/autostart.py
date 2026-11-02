"""Start with Windows: a value under HKCU\\...\\Run (what Task Manager → Startup apps shows)."""
import sys

from . import config as C

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def command():
    return f'"{sys.executable}" --background'


def enabled():
    if not (C.WINDOWS and C.FROZEN):
        return False
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as k:
            return bool(winreg.QueryValueEx(k, C.APP_NAME)[0])
    except OSError:
        return False


def set_enabled(on):
    # only the installed exe can start itself; running from source would register python.exe
    if not (C.WINDOWS and C.FROZEN):
        return
    import winreg
    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
        if on:
            winreg.SetValueEx(k, C.APP_NAME, 0, winreg.REG_SZ, command())
        else:
            try:
                winreg.DeleteValue(k, C.APP_NAME)
            except FileNotFoundError:
                pass


def refresh():
    """After an update or a move of the install folder, point the entry at this exe again."""
    if enabled():
        set_enabled(True)


def safe(fn, *args):
    try:
        fn(*args)
    except OSError as e:
        print(f"autostart: {e}")
