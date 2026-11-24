//! In-memory ring buffer of the MPEG-TS stream ffmpeg writes to its stdout.
//!
//! The stream is cut into chunks that each start at a video keyframe (one per second), so any
//! run of chunks from the end is a playable video. Nothing touches the disk until a save.
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

const TS: usize = 188;
const VIDEO_PID: usize = 0x100; // ffmpeg's mpegts muxer numbers streams from 0x100, video is mapped first
const PAT_PID: usize = 0;
const PMT_PID: usize = 0x1000;
/// a chunk is cut by the clock too once it is this old, so a stream without key frames (a frozen picture,
/// sound only) cannot grow into one chunk that is never trimmed
const MAX_CHUNK: f64 = 2.0;

struct Chunk {
    t: Instant,
    /// starts at a video key frame (cut by the clock otherwise)
    key: bool,
    data: Vec<u8>,
}

struct Inner {
    seconds: f64,
    max_bytes: usize,
    chunks: VecDeque<Chunk>,
    size: usize,
    pending: Vec<u8>,
    pat: Vec<u8>,
    pmt: Vec<u8>,
    tap: Option<File>, // a file the stream is also written to (full recording)
    tap_header: bool,
    last_video: Instant, // when the last video packet arrived
}

pub struct Ring {
    inner: Mutex<Inner>,
}

impl Inner {
    /// seconds since this chunk began arriving: by the clock, not by the video's own timestamps,
    /// which jump when the screen is still or frames are dropped
    fn age(c: &Chunk, now: Instant) -> f64 {
        now.duration_since(c.t).as_secs_f64()
    }
    fn trim(&mut self, now: Instant) {
        while self.chunks.len() > 1 && (Self::age(&self.chunks[1], now) >= self.seconds || self.size > self.max_bytes) {
            let c = self.chunks.pop_front().unwrap();
            self.size -= c.data.len();
        }
    }
    fn reset(&mut self) {
        self.chunks.clear();
        self.size = 0;
        self.pending.clear();
        self.pat.clear();
        self.pmt.clear();
        self.tap = None;
        self.tap_header = false;
        self.last_video = Instant::now();
    }
}

fn resync(buf: &[u8], mut i: usize) -> usize {
    loop {
        match buf[i + 1..].iter().position(|b| *b == 0x47) {
            None => return buf.len(),
            Some(p) => {
                let j = i + 1 + p;
                if j + TS >= buf.len() || buf[j + TS] == 0x47 {
                    return j;
                }
                i = j;
            }
        }
    }
}

impl Ring {
    pub fn new(seconds: f64, max_bytes: usize) -> Ring {
        Ring {
            inner: Mutex::new(Inner {
                seconds,
                max_bytes,
                chunks: VecDeque::new(),
                size: 0,
                pending: vec![],
                pat: vec![],
                pmt: vec![],
                tap: None,
                tap_header: false,
                last_video: Instant::now(),
            }),
        }
    }

    pub fn reset(&self) {
        self.inner.lock().unwrap().reset();
    }
    pub fn set_seconds(&self, s: f64) {
        self.inner.lock().unwrap().seconds = s;
    }
    pub fn size(&self) -> usize {
        self.inner.lock().unwrap().size
    }
    /// seconds since the last video packet (a frozen picture keeps this growing while sound goes on)
    pub fn video_idle(&self) -> f64 {
        Instant::now().duration_since(self.inner.lock().unwrap().last_video).as_secs_f64()
    }
    pub fn has_chunks(&self) -> bool {
        !self.inner.lock().unwrap().chunks.is_empty()
    }

    // ------------------------------------------------------------------ writing
    pub fn feed(&self, data: &[u8]) {
        let mut g = self.inner.lock().unwrap();
        let mut buf = std::mem::take(&mut g.pending);
        buf.extend_from_slice(data);
        let mut start = 0;
        if buf.first() != Some(&0x47) {
            // lost sync: skip to the next packet start
            start = resync(&buf, 0).min(buf.len());
        }
        let mut n = start + (buf.len() - start) / TS * TS;
        let mut keys: Vec<usize> = vec![];
        let (mut pat, mut pmt) = (None, None);
        let mut i = start;
        let mut bad = false;
        let mut video = false;
        while i < n {
            let p = &buf[i..i + TS];
            if p[0] != 0x47 {
                // keep the good part, resync after it
                n = i;
                g.pending = buf[resync(&buf, n).min(buf.len())..].to_vec();
                bad = true;
                break;
            }
            let pid = ((p[1] & 0x1f) as usize) << 8 | p[2] as usize;
            video |= pid == VIDEO_PID;
            if pid == VIDEO_PID && p[1] & 0x40 != 0 && p[3] & 0x20 != 0 && p[4] > 0 && p[5] & 0x40 != 0 {
                keys.push(i);
            } else if pid == PAT_PID {
                pat = Some(i);
            } else if pid == PMT_PID {
                pmt = Some(i);
            }
            i += TS;
        }
        if !bad {
            g.pending = buf[n..].to_vec();
        }
        if n <= start {
            return;
        }
        if let Some(i) = pat {
            g.pat = buf[i..i + TS].to_vec();
        }
        if let Some(i) = pmt {
            g.pmt = buf[i..i + TS].to_vec();
        }
        let now = Instant::now();
        if video {
            g.last_video = now;
        }
        let had_chunk = !g.chunks.is_empty();
        let mut pos = start;
        for &i in &keys {
            if let Some(c) = g.chunks.back_mut() {
                c.data.extend_from_slice(&buf[pos..i]);
                g.size += i - pos;
            }
            g.chunks.push_back(Chunk { t: now, key: true, data: vec![] });
            pos = i;
        }
        if let Some(c) = g.chunks.back_mut() {
            c.data.extend_from_slice(&buf[pos..n]);
            g.size += n - pos;
        }
        if g.chunks.back().is_some_and(|c| Inner::age(c, now) >= MAX_CHUNK) {
            g.chunks.push_back(Chunk { t: now, key: false, data: vec![] });
        }
        if g.tap.is_some() && (had_chunk || !keys.is_empty()) {
            let first = if had_chunk { start } else { keys[0] };
            if !g.tap_header {
                let header = [g.pat.clone(), g.pmt.clone()].concat();
                if let Some(f) = g.tap.as_mut() {
                    let _ = f.write_all(&header);
                }
                g.tap_header = true;
            }
            if let Some(f) = g.tap.as_mut() {
                let _ = f.write_all(&buf[first..n]);
            }
        }
        g.trim(now);
    }

    /// Also write the stream to f from the current keyframe on (at most a second before now).
    pub fn start_tap(&self, mut f: File) {
        let mut g = self.inner.lock().unwrap();
        g.tap_header = false;
        if let Some(cur) = g.chunks.back() {
            if !g.pat.is_empty() && !g.pmt.is_empty() {
                let _ = f.write_all(&g.pat);
                let _ = f.write_all(&g.pmt);
                let _ = f.write_all(&cur.data);
                g.tap_header = true;
            }
        }
        g.tap = Some(f);
    }

    pub fn stop_tap(&self) -> Option<File> {
        self.inner.lock().unwrap().tap.take()
    }

    // ------------------------------------------------------------------ reading
    /// how many seconds are in the buffer now
    pub fn buffered(&self) -> f64 {
        let g = self.inner.lock().unwrap();
        match g.chunks.front() {
            None => 0.0,
            Some(c) => Inner::age(c, Instant::now()).min(g.seconds),
        }
    }

    /// (ts bytes, length) of the newest chunks covering at least `seconds`, or None
    pub fn snapshot(&self, seconds: f64) -> Option<(Vec<u8>, f64)> {
        let g = self.inner.lock().unwrap();
        if g.chunks.is_empty() || g.pat.is_empty() || g.pmt.is_empty() {
            return None;
        }
        let now = Instant::now();
        // the newest chunk that is long enough and starts at a key frame, else the newest that is long enough
        let old: Vec<usize> = (0..g.chunks.len()).rev().filter(|&i| Inner::age(&g.chunks[i], now) >= seconds).collect();
        let first = old.iter().copied().find(|&i| g.chunks[i].key).or(old.first().copied()).unwrap_or(0);
        let mut out = Vec::with_capacity(g.size + 2 * TS);
        out.extend_from_slice(&g.pat);
        out.extend_from_slice(&g.pmt);
        for c in g.chunks.iter().skip(first) {
            out.extend_from_slice(&c.data);
        }
        Some((out, Inner::age(&g.chunks[first], now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(pid: usize, pusi: bool, rai: bool, pts: i64) -> Vec<u8> {
        let mut p = vec![0xffu8; TS];
        p[0] = 0x47;
        p[1] = (if pusi { 0x40 } else { 0 }) | (pid >> 8) as u8;
        p[2] = (pid & 255) as u8;
        let off;
        if rai {
            p[3] = 0x30;
            p[4] = 7;
            p[5] = 0x40;
            off = 12;
        } else {
            p[3] = 0x10;
            off = 4;
        }
        if pusi {
            p[off..off + 3].copy_from_slice(&[0, 0, 1]);
            p[off + 7] = 0x80;
            p[off + 9..off + 14].copy_from_slice(&[0x21 | ((pts >> 29) & 0xe) as u8, (pts >> 22) as u8, (((pts >> 14) & 0xfe) | 1) as u8, (pts >> 7) as u8, (((pts << 1) & 0xfe) | 1) as u8]);
        }
        p
    }

    #[test]
    fn cuts_chunks_at_keyframes_and_keeps_the_newest() {
        let ring = Ring::new(3.0, 1 << 30);
        let mut stream = pkt(PAT_PID, false, false, 0);
        stream.extend(pkt(PMT_PID, false, false, 0));
        for s in 0..10 {
            stream.extend(pkt(VIDEO_PID, true, true, s * 90000));
            for _ in 0..50 {
                stream.extend(pkt(VIDEO_PID, false, false, 0));
            }
        }
        // odd read sizes, like a pipe gives them
        for c in stream.chunks(7001) {
            ring.feed(c);
        }
        let g = ring.inner.lock().unwrap();
        assert!(g.chunks.len() >= 2 && g.chunks.len() <= 11);
        assert!(g.chunks.iter().all(|c| c.data.len() % TS == 0 && c.data[0] == 0x47));
        drop(g);
        let snap = ring.snapshot(1.0).expect("a snapshot");
        assert_eq!(snap.0[0], 0x47);
        assert_eq!(snap.0.len() % TS, 0);
    }

    #[test]
    fn the_buffer_is_bounded_by_the_clock_not_by_video_timestamps() {
        // video timestamps that do not move (a still screen) must not keep old chunks alive
        let ring = Ring::new(3.0, 1 << 30);
        let mut stream = pkt(PAT_PID, false, false, 0);
        stream.extend(pkt(PMT_PID, false, false, 0));
        for _ in 0..30 {
            stream.extend(pkt(VIDEO_PID, true, true, 0)); // every keyframe has the same timestamp
            stream.extend(pkt(VIDEO_PID, false, false, 0));
        }
        ring.feed(&stream);
        {
            let mut g = ring.inner.lock().unwrap();
            let n = g.chunks.len();
            let now = Instant::now();
            for (i, c) in g.chunks.iter_mut().enumerate() {
                // one chunk a second, the newest just now
                c.t = now - std::time::Duration::from_secs((n - 1 - i) as u64);
            }
            g.trim(now);
            assert!(g.chunks.len() <= 4, "kept {} chunks", g.chunks.len());
        }
        let (_, len) = ring.snapshot(3.0).unwrap();
        assert!(len < 4.5, "a replay of 3 s must not be {len} s");
    }

    #[test]
    fn sound_without_picture_stays_bounded() {
        // the picture freezes after one key frame, the sound goes on: the buffer must still hold only a few seconds
        let ring = Ring::new(3.0, 1 << 30);
        let mut stream = pkt(PAT_PID, false, false, 0);
        stream.extend(pkt(PMT_PID, false, false, 0));
        stream.extend(pkt(VIDEO_PID, true, true, 0));
        ring.feed(&stream);
        for _ in 0..60 {
            {
                // a second passes
                let mut g = ring.inner.lock().unwrap();
                for c in g.chunks.iter_mut() {
                    c.t -= std::time::Duration::from_secs(1);
                }
                g.last_video -= std::time::Duration::from_secs(1);
            }
            let mut audio = vec![];
            for _ in 0..20 {
                audio.extend(pkt(VIDEO_PID + 1, false, false, 0));
            }
            ring.feed(&audio);
        }
        let g = ring.inner.lock().unwrap();
        assert!(g.chunks.len() <= 6, "kept {} chunks", g.chunks.len());
        assert!(g.size <= 6 * 20 * TS * 3, "kept {} bytes", g.size);
        drop(g);
        assert!(ring.video_idle() >= 59.0, "idle {}", ring.video_idle());
        let (_, len) = ring.snapshot(3.0).unwrap();
        assert!(len < 6.0, "a 3 s replay came out as {len} s");
    }

    #[test]
    fn resyncs_after_garbage() {
        let ring = Ring::new(60.0, 1 << 30);
        let mut stream = vec![0u8, 1, 2, 3, 4];
        stream.extend(pkt(PAT_PID, false, false, 0));
        stream.extend(pkt(PMT_PID, false, false, 0));
        stream.extend(pkt(VIDEO_PID, true, true, 0));
        stream.extend(pkt(VIDEO_PID, false, false, 0));
        ring.feed(&stream);
        assert!(ring.has_chunks());
    }
}
