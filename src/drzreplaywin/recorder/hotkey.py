"""Global hotkey: RegisterHotKey in a thread with its own message loop.

Hotkeys are stored as text like "Alt+Z" or "Ctrl+Shift+F9"; the key is a virtual-key code,
so it works the same with any keyboard layout (Alt+Я is Alt+Z).
"""
import threading

from ..core import config as C

MODS = {"Ctrl": 0x2, "Alt": 0x1, "Shift": 0x4, "Win": 0x8}
MOD_NOREPEAT = 0x4000
WM_HOTKEY, WM_APP = 0x0312, 0x8000
SOLO_OK = {"Pause", "ScrollLock", "PrintScreen", "Insert"} | {f"F{i}" for i in range(1, 25)}

KEYS = {chr(c): c for c in range(ord("A"), ord("Z") + 1)}
KEYS.update({chr(c): c for c in range(ord("0"), ord("9") + 1)})
KEYS.update({f"F{i}": 0x6F + i for i in range(1, 25)})
KEYS.update({f"Num{i}": 0x60 + i for i in range(10)})
KEYS.update({"Space": 0x20, "PageUp": 0x21, "PageDown": 0x22, "End": 0x23, "Home": 0x24, "Left": 0x25,
             "Up": 0x26, "Right": 0x27, "Down": 0x28, "PrintScreen": 0x2C, "Insert": 0x2D, "Delete": 0x2E,
             "Pause": 0x13, "ScrollLock": 0x91, "Num*": 0x6A, "NumPlus": 0x6B, "NumMinus": 0x6D, "Num.": 0x6E,
             "Num/": 0x6F, ";": 0xBA, "=": 0xBB, ",": 0xBC, "-": 0xBD, ".": 0xBE, "/": 0xBF, "`": 0xC0,
             "[": 0xDB, "\\": 0xDC, "]": 0xDD, "'": 0xDE})
NAMES = {v: k for k, v in KEYS.items()}


def parse(text):
    """"Ctrl+Alt+Z" → (modifier bits, vk) or None."""
    parts = [p for p in text.replace(" ", "").split("+") if p] if text else []
    if not parts:
        return None
    mods = 0
    for p in parts[:-1]:
        if p not in MODS:
            return None
        mods |= MODS[p]
    vk = KEYS.get(parts[-1].upper() if len(parts[-1]) == 1 else parts[-1])
    return (mods, vk) if vk else None


def compose(mod_names, key):
    order = [m for m in ("Ctrl", "Alt", "Shift", "Win") if m in mod_names]
    return "+".join(order + [key])


class Hotkey:
    """Calls on_press() (from its own thread) whenever the hotkey is pressed anywhere."""

    def __init__(self, text, on_press):
        self.text = text
        self.on_press = on_press
        self.ok = False
        self.error = "starting"
        self.tid = None
        self.ready = threading.Event()
        if C.WINDOWS:
            threading.Thread(target=self.loop, daemon=True, name="hotkey").start()
        else:
            self.error = "only works on Windows"

    def set(self, text):
        self.text = text
        if self.tid and self.ready.wait(2):
            import ctypes
            ctypes.windll.user32.PostThreadMessageW(self.tid, WM_APP + 1, 0, 0)

    def stop(self):
        if self.tid and self.ready.wait(2):
            import ctypes
            ctypes.windll.user32.PostThreadMessageW(self.tid, WM_APP + 2, 0, 0)

    def loop(self):
        import ctypes
        from ctypes import wintypes
        user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
        user32.RegisterHotKey.argtypes = [wintypes.HWND, ctypes.c_int, wintypes.UINT, wintypes.UINT]
        msg = wintypes.MSG()
        user32.PeekMessageW(ctypes.byref(msg), None, 0x400, 0x400, 0)   # create the message queue
        self.tid = kernel32.GetCurrentThreadId()
        registered = False

        def register():
            nonlocal registered
            if registered:
                user32.UnregisterHotKey(None, 1)
                registered = False
            key = parse(self.text)
            if not key:
                self.ok, self.error = False, "not a valid key"
                return
            if user32.RegisterHotKey(None, 1, key[0] | MOD_NOREPEAT, key[1]):
                registered = True
                self.ok, self.error = True, ""
            else:
                err = ctypes.GetLastError()
                self.ok = False
                self.error = "another program uses this key" if err == 1409 else f"error {err}"

        register()
        self.ready.set()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            if msg.message == WM_HOTKEY:
                try:
                    self.on_press()
                except Exception:
                    pass
            elif msg.message == WM_APP + 1:
                register()
            elif msg.message == WM_APP + 2:
                break
        if registered:
            user32.UnregisterHotKey(None, 1)
