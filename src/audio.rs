//! System sound (WASAPI loopback) and microphone, mixed into one steady 48 kHz stereo stream for ffmpeg.
//!
//! The stream is paced by the clock, not by the devices: Windows sends nothing from a loopback device
//! while nothing plays, so gaps are filled with silence and the sound stays in sync with the video.
use crate::config::Config;
use std::collections::VecDeque;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const RATE: usize = 48000;
/// jitter buffer per device, seconds
const LATENCY: f64 = 0.05;
/// drop audio older than this behind the clock
const MAX_BACKLOG: f64 = 0.15;

pub type Log = Arc<dyn Fn(String) + Send + Sync>;

// ---------------------------------------------------------------- one device
/// Interleaved stereo f32 samples from the capture thread, waiting for the mixer.
#[derive(Default)]
struct Buf {
    data: VecDeque<f32>,
    primed: bool,
}

pub struct Source {
    buf: Arc<Mutex<Buf>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// what you hear (loopback of a playback device)
    System,
    /// a microphone
    Mic,
}

impl Source {
    pub fn open(kind: Kind, name: &str, log: Log) -> Result<(Source, String), String> {
        let buf = Arc::new(Mutex::new(Buf::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let (b, s, n) = (buf.clone(), stop.clone(), name.to_string());
        let thread = std::thread::spawn(move || {
            if let Err(e) = platform::capture(kind, &n, &b, &s, &tx) {
                let _ = tx.send(Err(e.clone()));
                log(format!("audio: capture stopped: {e}"));
            }
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(info)) => Ok((Source { buf, stop, thread: Some(thread) }, info)),
            Ok(Err(e)) => {
                stop.store(true, Ordering::Relaxed);
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err("the device didn't start".into())
            }
        }
    }

    fn clear(&self) {
        let mut g = self.buf.lock().unwrap();
        g.data.clear();
        g.primed = false;
    }

    /// n frames (interleaved) for the mix, or None (silence) while the buffer fills up
    fn pull(&self, n: usize) -> Option<Vec<f32>> {
        let mut g = self.buf.lock().unwrap();
        let count = g.data.len() / 2;
        if !g.primed {
            if (count as f64) < n as f64 + LATENCY * RATE as f64 {
                return None;
            }
            g.primed = true;
        }
        let extra = count as i64 - n as i64 - (MAX_BACKLOG * RATE as f64) as i64;
        if extra > 0 {
            let drop = count - n - (LATENCY * RATE as f64) as usize;
            g.data.drain(..drop * 2);
        }
        let count = g.data.len() / 2;
        if count < n {
            // ran dry (nothing playing): silence until it refills
            g.primed = false;
            let mut got: Vec<f32> = g.data.drain(..).collect();
            got.resize(n * 2, 0.0);
            return Some(got);
        }
        Some(g.data.drain(..n * 2).collect())
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------- devices
/// (outputs, inputs): names of the playback and recording devices
pub fn devices() -> (Vec<String>, Vec<String>) {
    platform::devices()
}

/// Id of the default playback (system) / recording (mic) device, to notice switches.
fn default_endpoint(kind: Kind) -> Option<String> {
    platform::default_endpoint(kind)
}

// ---------------------------------------------------------------- the mixer
struct Shared {
    anchor: Mutex<Option<Instant>>,
    anchored: Condvar,
    stop: AtomicBool,
    working: AtomicBool,
}

/// Writes the mix to `out` (ffmpeg's stdin) from the moment the first video frame was taken.
pub struct Mixer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Mixer {
    pub fn start(cfg: &Config, out: ChildStdin, log: Log) -> Mixer {
        let shared = Arc::new(Shared { anchor: Mutex::new(None), anchored: Condvar::new(), stop: AtomicBool::new(false), working: AtomicBool::new(true) });
        let (s, cfg) = (shared.clone(), cfg.clone());
        let thread = std::thread::Builder::new().name("audio".into()).spawn(move || run(&cfg, out, &s, log)).ok();
        Mixer { shared, thread }
    }

    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.anchored.notify_all();
        // the thread ends by itself once ffmpeg is gone; don't wait for it here
        self.thread.take();
    }

    /// Time of the first video frame: audio sample 0 belongs to it.
    pub fn set_anchor(&self, t: Instant) {
        let mut a = self.shared.anchor.lock().unwrap();
        if a.is_none() {
            *a = Some(t + Duration::from_secs_f64(LATENCY)); // every device is LATENCY behind the clock
            self.shared.anchored.notify_all();
        }
    }

    /// False when sound was asked for but no device could be opened.
    pub fn working(&self) -> bool {
        self.shared.working.load(Ordering::Relaxed)
    }
}

fn open_sources(cfg: &Config, log: &Log) -> Vec<(Kind, Source)> {
    let mut v = vec![];
    for (kind, enabled, name) in [(Kind::System, cfg.b("system_audio"), cfg.s("system_device")), (Kind::Mic, cfg.b("mic"), cfg.s("mic_device"))] {
        if !enabled {
            continue;
        }
        let label = if kind == Kind::System { "system" } else { "mic" };
        match Source::open(kind, &name, log.clone()) {
            Ok((s, info)) => {
                log(format!("audio: {label} = {info}"));
                v.push((kind, s));
            }
            Err(e) => log(format!("audio: {label} failed: {e}")),
        }
    }
    v
}

fn run(cfg: &Config, mut out: ChildStdin, sh: &Shared, log: Log) {
    let want = cfg.b("system_audio") as usize + cfg.b("mic") as usize;
    let follow = (cfg.s("system_device").is_empty() && cfg.b("system_audio"), cfg.s("mic_device").is_empty() && cfg.b("mic"));
    let ids = |f: (bool, bool)| (f.0.then(|| default_endpoint(Kind::System)).flatten(), f.1.then(|| default_endpoint(Kind::Mic)).flatten());
    let mut cur_ids = ids(follow);
    let mut sources = open_sources(cfg, &log);
    sh.working.store(want == 0 || !sources.is_empty(), Ordering::Relaxed);
    // wait for the first video frame
    let anchor = {
        let mut a = sh.anchor.lock().unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while a.is_none() && !sh.stop.load(Ordering::Relaxed) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            a = sh.anchored.wait_timeout(a, left).unwrap().0;
        }
        *a.get_or_insert_with(|| Instant::now() + Duration::from_secs_f64(LATENCY))
    };
    for (_, s) in &sources {
        s.clear();
    }
    let vol = |k: Kind| if k == Kind::System { cfg.f("system_volume") / 100.0 } else { cfg.f("mic_volume") / 100.0 } as f32;
    let mut written: i64 = 0;
    let mut next_check = Instant::now() + Duration::from_secs(3);
    while !sh.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let need = (now.saturating_duration_since(anchor).as_secs_f64() * RATE as f64) as i64 - written;
        if need > 0 {
            let n = need as usize;
            let mut mix = vec![0.0f32; n * 2];
            for (kind, src) in &sources {
                if let Some(part) = src.pull(n) {
                    let v = vol(*kind);
                    for (m, p) in mix.iter_mut().zip(part) {
                        *m += p * v;
                    }
                }
            }
            let mut bytes = Vec::with_capacity(n * 4);
            for m in &mix {
                bytes.extend_from_slice(&((m.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
            if out.write_all(&bytes).is_err() {
                break; // ffmpeg is gone
            }
            written += need;
        }
        if now >= next_check && (follow.0 || follow.1) {
            next_check = now + Duration::from_secs(3);
            let new = ids(follow);
            if new != cur_ids {
                // headphones plugged in, default device switched…
                cur_ids = new;
                log("audio: default device changed, reopening".into());
                for (_, s) in sources.iter_mut() {
                    s.close();
                }
                sources = open_sources(cfg, &log);
                sh.working.store(want == 0 || !sources.is_empty(), Ordering::Relaxed);
                for (_, s) in &sources {
                    s.clear();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for (_, s) in sources.iter_mut() {
        s.close();
    }
}

// ---------------------------------------------------------------- platform
#[cfg(windows)]
mod platform {
    use super::*;
    use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    pub fn devices() -> (Vec<String>, Vec<String>) {
        let _ = initialize_mta();
        let list = |d: Direction| -> Vec<String> {
            let Ok(en) = DeviceEnumerator::new() else { return vec![] };
            let Ok(col) = en.get_device_collection(d) else { return vec![] };
            (&col).into_iter().filter_map(|dev| dev.ok()?.get_friendlyname().ok()).collect()
        };
        (list(Direction::Render), list(Direction::Capture))
    }

    pub fn default_endpoint(kind: Kind) -> Option<String> {
        let _ = initialize_mta();
        let en = DeviceEnumerator::new().ok()?;
        let d = en.get_default_device(if kind == Kind::System { Direction::Render } else { Direction::Capture }).ok()?;
        d.get_id().ok()
    }

    pub fn capture(kind: Kind, name: &str, buf: &Arc<Mutex<Buf>>, stop: &AtomicBool, tx: &std::sync::mpsc::Sender<Result<String, String>>) -> Result<(), String> {
        let e = |e: wasapi::WasapiError| e.to_string();
        let _ = initialize_mta();
        let en = DeviceEnumerator::new().map_err(e)?;
        let dir = if kind == Kind::System { Direction::Render } else { Direction::Capture };
        let device = if name.is_empty() {
            en.get_default_device(dir).map_err(e)?
        } else {
            let col = en.get_device_collection(dir).map_err(e)?;
            (&col).into_iter().filter_map(|d| d.ok()).find(|d| d.get_friendlyname().is_ok_and(|n| n.starts_with(name))).ok_or_else(|| "no device".to_string())?
        };
        let label = device.get_friendlyname().unwrap_or_default();
        let mut client = device.get_iaudioclient().map_err(e)?;
        // Windows converts whatever the device speaks to this
        let fmt = WaveFormat::new(32, 32, SampleType::Float, RATE, 2, None);
        let (_, min_time) = client.get_device_period().map_err(e)?;
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min_time };
        // capturing from a playback device is the loopback
        client.initialize_client(&fmt, Direction::Capture, mode, None).map_err(e)?;
        let ev = client.set_get_eventhandle().map_err(e)?;
        let cap = client.get_audiocaptureclient().map_err(e)?;
        client.start_stream().map_err(e)?;
        let _ = tx.send(Ok(format!("{label} ({RATE} Hz, stereo)")));
        let mut bytes: VecDeque<u8> = VecDeque::new();
        while !stop.load(Ordering::Relaxed) {
            if cap.read_from_device_to_deque(&mut bytes).is_ok() && !bytes.is_empty() {
                let n = bytes.len() / 4 * 4;
                let mut samples = Vec::with_capacity(n / 4);
                let raw: Vec<u8> = bytes.drain(..n).collect();
                for c in raw.chunks_exact(4) {
                    samples.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
                buf.lock().unwrap().data.extend(samples);
            }
            // a loopback device sends nothing while nothing plays: the timeout is normal
            let _ = ev.wait_for_event(200);
        }
        let _ = client.stop_stream();
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn devices() -> (Vec<String>, Vec<String>) {
        (vec![], vec![])
    }
    pub fn default_endpoint(_kind: Kind) -> Option<String> {
        None
    }
    pub fn capture(_kind: Kind, _name: &str, _buf: &Arc<Mutex<Buf>>, _stop: &AtomicBool, _tx: &std::sync::mpsc::Sender<Result<String, String>>) -> Result<(), String> {
        Err("sound capture only works on Windows".into())
    }
}
