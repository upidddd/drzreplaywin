"""Settings, paths and constants of DrzReplayWIN."""
import json
import os
import sys

APP_NAME = "DrzReplayWIN"
VERSION = "1.1.1"
REPO = "upidddd/drzreplaywin"
INSTALL_CMD = f"irm https://github.com/{REPO}/releases/latest/download/install.ps1 | iex"
SETUP_URL = f"https://github.com/{REPO}/releases/latest/download/DrzReplayWIN-Setup.exe"

WINDOWS = sys.platform == "win32"
FROZEN = getattr(sys, "frozen", False)     # running from the PyInstaller build

PKG_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(sys._MEIPASS, "assets") if FROZEN else os.path.join(PKG_DIR, "assets")
ICON_PATH = os.path.join(ASSETS, "icons", "drzreplaywin.ico" if WINDOWS else "drzreplaywin.png")
ICON_SVG = os.path.join(ASSETS, "icons", "drzreplaywin.svg")
CLICK_SOUND = os.path.join(ASSETS, "sounds", "click.wav")

if os.environ.get("DRZREPLAYWIN_HOME"):           # tests and the self-test keep everything in one folder
    CONF_DIR = DATA_DIR = os.environ["DRZREPLAYWIN_HOME"]
elif WINDOWS:
    CONF_DIR = os.path.join(os.environ.get("APPDATA") or os.path.expanduser("~"), APP_NAME)
    DATA_DIR = os.path.join(os.environ.get("LOCALAPPDATA") or CONF_DIR, APP_NAME)
else:
    CONF_DIR = os.path.join(os.path.expanduser("~/.config"), "drzreplaywin")
    DATA_DIR = os.path.join(os.path.expanduser("~/.cache"), "drzreplaywin")
CONF_PATH = os.path.join(CONF_DIR, "config.json")
RECORDER_LOG = os.path.join(DATA_DIR, "recorder.log")
APP_LOG = os.path.join(DATA_DIR, "app.log")
THUMBS = os.path.join(DATA_DIR, "thumbs")
PROBE_CACHE = os.path.join(DATA_DIR, "probe.json")
UPDATE_CACHE = os.path.join(DATA_DIR, "update.json")

# CQ / CRF / QP per preset — lower is better quality and more memory
REC_QUALITY = {"low": 28, "balanced": 23, "high": 19, "ultra": 16}
MAX_BUFFER_MB = 2048          # the ring never grows past this, whatever the length


def default_video_dir():
    base = None
    if WINDOWS:
        base = known_folder("{18989B1D-99B5-455B-841C-AB7C74E4DDFC}")     # FOLDERID_Videos
    return os.path.join(base or os.path.join(os.path.expanduser("~"), "Videos"), "DrzReplay")


def known_folder(guid):
    """Path of a Windows known folder (follows OneDrive / moved folders)."""
    import ctypes
    from ctypes import wintypes
    import uuid

    class GUID(ctypes.Structure):
        _fields_ = [("data", ctypes.c_ubyte * 16)]

    g = GUID()
    ctypes.memmove(g.data, uuid.UUID(guid).bytes_le, 16)
    out = ctypes.c_wchar_p()
    try:
        if ctypes.windll.shell32.SHGetKnownFolderPath(ctypes.byref(g), 0, None, ctypes.byref(out)) != 0:
            return None
        return out.value
    except Exception:
        return None
    finally:
        if out:
            ctypes.windll.ole32.CoTaskMemFree(ctypes.cast(out, wintypes.LPVOID))


DEFAULTS = {
    # capture
    "seconds": 60,
    "fps": 60,
    "monitor": 0,                 # ddagrab output index
    "draw_mouse": True,
    "encoder": "auto",            # auto | nvenc | amf | qsv | x264
    "rec_quality": "balanced",
    # sound
    "system_audio": True,
    "system_device": "",          # "" = the Windows default, follows it when it changes
    "system_volume": 100,
    "mic": False,
    "mic_device": "",
    "mic_volume": 100,
    # saving
    "output_dir": "",
    "file_prefix": "DrzReplay",
    "container": "mp4",
    "keep_max_clips": 0,          # 0 = unlimited
    "keep_max_gb": 0,
    # controls
    "hotkey": "Alt+Z",
    "click_sound": True,
    "notifications": True,
    "record_hotkey": "Alt+R",     # start / stop recording a full video
    "intro": False,               # replay number and date in the corner for the first seconds
    "replay_since_last": True,    # a replay empties the buffer: the next one has only what came after it
    "tray_hint_shown": False,
}

# changing any of these restarts the recorder
RECORDER_KEYS = {"seconds", "fps", "monitor", "draw_mouse", "encoder", "rec_quality", "system_audio",
                 "system_device", "mic", "mic_device"}


def exists():
    return os.path.exists(CONF_PATH)


def load():
    cfg = dict(DEFAULTS)
    try:
        with open(CONF_PATH, encoding="utf-8") as f:
            data = json.load(f)
        cfg.update({k: v for k, v in data.items() if k in DEFAULTS})
    except FileNotFoundError:
        pass
    except Exception as e:
        print(f"{APP_NAME}: config is broken ({e}), using defaults", file=sys.stderr)
    if not cfg["output_dir"]:
        cfg["output_dir"] = default_video_dir()
    return cfg


def save(cfg):
    write_json(CONF_PATH, {k: cfg[k] for k in DEFAULTS}, indent=2)


def output_dir(cfg):
    return os.path.expanduser(cfg["output_dir"])


def read_json(path, default=None):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return default


def write_json(path, data, indent=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=indent, ensure_ascii=False)
    os.replace(tmp, path)
