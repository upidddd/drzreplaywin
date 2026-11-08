"""In-memory ring buffer of the MPEG-TS stream ffmpeg writes to its stdout.

The stream is cut into chunks that each start at a video keyframe (one per second), so any
run of chunks from the end is a playable video. Nothing touches the disk until a save.
"""
import threading
import time
from collections import deque

import numpy as np

TS = 188
VIDEO_PID = 0x100         # ffmpeg's mpegts muxer numbers streams from 0x100, video is mapped first
PAT_PID, PMT_PID = 0, 0x1000
PTS_WRAP = 1 << 33
PTS_HZ = 90000


def pes_pts(pkt):
    """PTS of the PES packet that starts in this TS packet, or None."""
    off = 4 + (1 + pkt[4] if pkt[3] & 0x20 else 0)
    if pkt[off:off + 3] != b"\0\0\1" or not pkt[off + 7] & 0x80:
        return None
    p = pkt[off + 9:off + 14]
    if len(p) < 5:
        return None
    return ((p[0] >> 1) & 7) << 30 | p[1] << 22 | (p[2] >> 1) << 15 | p[3] << 7 | p[4] >> 1


class Chunk:
    __slots__ = ("pts", "t", "data")

    def __init__(self, pts, t):
        self.pts, self.t, self.data = pts, t, bytearray()


class Ring:
    def __init__(self, seconds=60, max_bytes=2048 << 20):
        self.lock = threading.Lock()
        self.seconds = seconds
        self.max_bytes = max_bytes
        self.reset()

    def reset(self):
        with self.lock:
            self.chunks = deque()
            self.cur = None
            self.size = 0
            self.pending = b""
            self.pat = self.pmt = b""
            self.tap = None             # a file the stream is also written to (full recording)
            self.tap_header = False

    # ------------------------------------------------------------------ writing
    def feed(self, data):
        buf = self.pending + data
        start = 0
        if buf[:1] != b"\x47":                      # lost sync: skip to the next packet start
            start = self._resync(buf, 0)
        n = start + (len(buf) - start) // TS * TS
        arr = np.frombuffer(buf, np.uint8, count=n - start, offset=start).reshape(-1, TS)
        bad = np.flatnonzero(arr[:, 0] != 0x47)
        if bad.size:                                # keep the good part, resync after it
            n = start + int(bad[0]) * TS
            arr = arr[:bad[0]]
            self.pending = buf[self._resync(buf, n):]
        else:
            self.pending = buf[n:]
        if not len(arr):
            return
        pid = ((arr[:, 1].astype(np.uint16) & 0x1F) << 8) | arr[:, 2]
        pusi = (arr[:, 1] & 0x40) != 0
        rai = ((arr[:, 3] & 0x20) != 0) & (arr[:, 4] > 0) & ((arr[:, 5] & 0x40) != 0)
        keys = np.flatnonzero((pid == VIDEO_PID) & pusi & rai)
        for p, attr in ((PAT_PID, "pat"), (PMT_PID, "pmt")):
            idx = np.flatnonzero(pid == p)
            if idx.size:
                i = start + int(idx[-1]) * TS
                setattr(self, attr, bytes(buf[i:i + TS]))
        now = time.monotonic()
        with self.lock:
            had_chunk = self.cur is not None
            pos = start
            for k in keys:
                i = start + int(k) * TS
                if self.cur is not None:
                    self.cur.data += buf[pos:i]
                pts = pes_pts(buf[i:i + TS])
                if pts is None:
                    pts = (self.cur.pts + PTS_HZ) % PTS_WRAP if self.cur else 0
                self.cur = Chunk(pts, now)
                self.chunks.append(self.cur)
                pos = i
            if self.cur is not None:
                self.cur.data += buf[pos:n]
            if self.tap is not None and (had_chunk or keys.size):
                first = start if had_chunk else start + int(keys[0]) * TS
                if not self.tap_header:
                    self.tap.write(self.pat + self.pmt)
                    self.tap_header = True
                self.tap.write(buf[first:n])
            self.size = sum(len(c.data) for c in self.chunks)
            self._trim(now)

    @staticmethod
    def _resync(buf, i):
        while True:
            j = buf.find(b"\x47", i + 1)
            if j < 0 or j + TS >= len(buf) or buf[j + TS] == 0x47:
                return j if j >= 0 else len(buf)
            i = j

    def _trim(self, now):
        keep = self.seconds
        while len(self.chunks) > 1 and (self._age(self.chunks[1], now) >= keep or self.size > self.max_bytes):
            self.size -= len(self.chunks.popleft().data)

    def start_tap(self, f):
        """Also write the stream to f from the current keyframe on (at most a second before now)."""
        with self.lock:
            self.tap, self.tap_header = f, False
            if self.cur is not None and self.pat and self.pmt:
                f.write(self.pat + self.pmt + bytes(self.cur.data))
                self.tap_header = True

    def stop_tap(self):
        with self.lock:
            f, self.tap = self.tap, None
            return f

    # ------------------------------------------------------------------ reading
    def _now_pts(self, now):
        last = self.chunks[-1]
        return last.pts + (now - last.t) * PTS_HZ

    def _age(self, chunk, now):
        """Seconds from the start of this chunk to now (by video time, wrap-safe)."""
        return ((self._now_pts(now) - chunk.pts) % PTS_WRAP) / PTS_HZ

    def buffered(self):
        with self.lock:
            if not self.chunks:
                return 0.0
            return min(self.seconds, self._age(self.chunks[0], time.monotonic()))

    def snapshot(self, seconds):
        """(ts_bytes, length) of the newest chunks covering at least `seconds`, or (None, 0)."""
        with self.lock:
            if not self.chunks or not self.pat or not self.pmt:
                return None, 0.0
            now = time.monotonic()
            first = 0
            for i in range(len(self.chunks) - 1, -1, -1):
                if self._age(self.chunks[i], now) >= seconds:
                    first = i
                    break
            parts = [self.pat, self.pmt] + [bytes(c.data) for c in list(self.chunks)[first:]]
            return b"".join(parts), self._age(self.chunks[first], now)
