"""Writes build/version.txt: the version resource of DrzReplayWIN.exe (name in Task Manager, toasts…)."""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "src"))
from drzreplaywin.core import config as C  # noqa: E402

v = tuple(int(x) for x in C.VERSION.split(".")) + (0,) * (4 - len(C.VERSION.split(".")))
TEMPLATE = f"""VSVersionInfo(
  ffi=FixedFileInfo(filevers={v}, prodvers={v}, mask=0x3f, flags=0x0, OS=0x40004, fileType=0x1, subtype=0x0),
  kids=[
    StringFileInfo([StringTable('040904B0', [
      StringStruct('CompanyName', 'DRZ'),
      StringStruct('FileDescription', '{C.APP_NAME}'),
      StringStruct('FileVersion', '{C.VERSION}'),
      StringStruct('InternalName', '{C.APP_NAME}'),
      StringStruct('LegalCopyright', 'Copyright (c) 2026 DRZ, MIT License'),
      StringStruct('OriginalFilename', '{C.APP_NAME}.exe'),
      StringStruct('ProductName', '{C.APP_NAME}'),
      StringStruct('ProductVersion', '{C.VERSION}')])]),
    VarFileInfo([VarStruct('Translation', [1033, 1200])])
  ]
)
"""
os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
with open(os.path.join(ROOT, "build", "version.txt"), "w", encoding="utf-8") as f:
    f.write(TEMPLATE)
if os.environ.get("GITHUB_ENV"):
    with open(os.environ["GITHUB_ENV"], "a") as f:
        f.write(f"APP_VERSION={C.VERSION}\n")
print(C.VERSION)
