"""Clips tab: cut a piece out of a saved clip (frame-exact, encoded again) and rename clips."""
import os
import re
import subprocess

from ..core import system

# names the app gives clips itself: "<prefix> 2026-11-09 14-03-27", maybe " (trimmed)" / " (trimmed 2)"
AUTO_NAME = re.compile(r"\d{4}-\d{2}-\d{2} \d{2}-\d{2}-\d{2}(?: \(trimmed(?: \d+)?\))?$")
BAD_CHARS = set('<>:"/\\|?*')


def is_auto_name(path):
    return bool(AUTO_NAME.search(os.path.splitext(os.path.basename(path))[0]))


def rename(path, name):
    """Give the clip a new name (the extension stays). Returns the new path; ValueError says what's wrong."""
    name = name.strip()
    ext = os.path.splitext(path)[1]
    if name.lower().endswith(ext.lower()):
        name = name[:-len(ext)].rstrip()
    name = name.rstrip(". ")
    if not name:
        raise ValueError("The name is empty")
    if any(c in BAD_CHARS or ord(c) < 32 for c in name):
        raise ValueError('A name can\'t contain < > : " / \\ | ? *')
    new = os.path.join(os.path.dirname(path), name + ext)
    if os.path.normcase(new) == os.path.normcase(path):
        if new != path:                     # only the letter case changed
            os.rename(path, new)
        return new
    if os.path.exists(new):
        raise ValueError("A clip with this name already exists")
    try:
        os.rename(path, new)
    except OSError as e:
        raise ValueError(e.strerror or "Couldn't rename") from None
    return new


def output_path(src):
    base, ext = os.path.splitext(src)
    path = f"{base} (trimmed){ext}"
    n = 2
    while os.path.exists(path):
        path = f"{base} (trimmed {n}){ext}"
        n += 1
    return path


def frame(path, t, width=640):
    """One JPEG frame at t seconds (for the trim preview), or None."""
    cmd = [system.ffmpeg(), "-hide_banner", "-v", "error", "-ss", f"{max(0.0, t):.3f}", "-i", path,
           "-frames:v", "1", "-vf", f"scale={width}:-2", "-q:v", "4", "-f", "image2pipe", "-c:v", "mjpeg", "-"]
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=15, stdin=subprocess.DEVNULL,
                           creationflags=system._flags())
    except Exception:
        return None
    return r.stdout if r.returncode == 0 and r.stdout else None


def trim(cfg, enc, src, start, end, delete_original=False):
    """Save start…end of src as "<name> (trimmed)". With the recorder's encoder, x264 if it can't.
    Returns the new path; RuntimeError when nothing worked."""
    out = output_path(src)
    tmp = out + ".part"
    err = "ffmpeg failed"
    try:
        for e in dict.fromkeys([enc or "x264", "x264"]):
            pix = "nv12" if e in ("qsv", "amf") else "yuv420p"
            # -ss before -i with encoding is exact: ffmpeg decodes from the keyframe and drops up to `start`
            cmd = [system.ffmpeg(), "-hide_banner", "-v", "error", "-y", "-ss", f"{start:.3f}", "-i", src,
                   "-t", f"{end - start:.3f}", "-map", "0:v:0", "-map", "0:a?", "-vf", f"format={pix}",
                   "-c:a", "aac", "-b:a", "192k"]
            cmd += system.encoder_args(e, cfg["rec_quality"], int(cfg["fps"]))
            cmd += ["-movflags", "+faststart", "-f", "mp4"] if src.lower().endswith(".mp4") else ["-f", "matroska"]
            r = system.run(cmd + [tmp], timeout=1800, priority=system.BELOW_NORMAL_PRIORITY_CLASS)
            if r and r.returncode == 0 and os.path.exists(tmp) and os.path.getsize(tmp) > 1000:
                os.replace(tmp, out)
                if delete_original:
                    try:
                        os.remove(src)
                    except OSError:
                        pass
                return out
            if r:
                err = (r.stderr.strip().splitlines() or [err])[-1]
        raise RuntimeError(err)
    finally:
        try:
            os.remove(tmp)
        except OSError:
            pass
