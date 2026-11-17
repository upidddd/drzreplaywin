//! The recorder: one ffmpeg process capturing the screen into the in-memory ring, kept alive by a watchdog,
//! and a save queue that turns the newest part of the ring into a clip without re-encoding.
use crate::audio::{Log, Mixer};
use crate::config::{self as C, Config};
use crate::intro;
use crate::ring::Ring;
use crate::system::{self as sys, args, Probe, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS};
use shared_child::SharedChild;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// let the encoder flush the moment the key was pressed
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// What happened, for whoever shows it (called from worker threads).
#[derive(Debug, Clone)]
pub enum Ev {
    State(String),
    Saving,
    Saved { path: PathBuf, length: f64, size: u64 },
    SaveFailed(String),
    RecStarted,
    RecStopped,
    Recorded { path: PathBuf, length: f64, size: u64 },
}

struct FileRec {
    tmp: PathBuf,
    path: PathBuf,
    t0: f64,
    length: f64,
}

enum Job {
    Replay(Instant),
    Recording(FileRec),
}

#[derive(Clone, Default, Debug)]
pub struct Info {
    pub size: Option<(i64, i64)>,
    pub fps: i64,
    pub encoder: String,
}

struct State {
    state: String, // off | probing | starting | recording | paused | error
    error: String,
    proc: Option<Arc<SharedChild>>,
    mixer: Option<Mixer>,
    gen: u64,
    fails: u32,
    audio_off: bool, // sound kept failing: record without it
    probe: Option<Probe>,
    info: Info,
    started_at: Instant,
    log_lines: Vec<String>,
    session: Instant, // when the current ring started (a restart empties it)
    last_save: Option<(Instant, Instant)>, // (session, press time) of the last replay
    filerec: Option<(PathBuf, PathBuf, f64)>, // tmp, path, start (epoch seconds)
    rec_only: bool,                           // replays are off: the screen is captured only for a full recording
    file_pending: bool,                       // that recording starts as soon as the capture is up
}

pub struct Recorder {
    pub cfg: Arc<Mutex<Config>>,
    emit: Arc<dyn Fn(Ev) + Send + Sync>,
    pub ring: Ring,
    st: Mutex<State>,
    jobs: Mutex<Sender<Job>>,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: String,
    pub error: String,
    pub buffered: f64,
    pub file_recording: Option<f64>,
    pub replays_off: bool,
    pub seconds: f64,
    pub buffer_mb: f64,
    pub size: Option<(i64, i64)>,
    pub fps: i64,
    pub encoder: String,
    pub audio_off: bool,
    pub monitors: Vec<sys::Mon>,
    pub encoders: Vec<String>,
}

fn new_state() -> State {
    State {
        state: "off".into(),
        error: String::new(),
        proc: None,
        mixer: None,
        gen: 0,
        fails: 0,
        audio_off: false,
        probe: None,
        info: Info::default(),
        started_at: Instant::now(),
        log_lines: vec![],
        session: Instant::now(),
        last_save: None,
        filerec: None,
        rec_only: false,
        file_pending: false,
    }
}

impl Recorder {
    pub fn new(cfg: Arc<Mutex<Config>>, emit: impl Fn(Ev) + Send + Sync + 'static) -> Arc<Recorder> {
        let (tx, rx) = channel::<Job>();
        let seconds = cfg.lock().unwrap().f("seconds");
        let r = Arc::new(Recorder { cfg, emit: Arc::new(emit), ring: Ring::new(seconds, (C::MAX_BUFFER_MB as usize) << 20), st: Mutex::new(new_state()), jobs: Mutex::new(tx) });
        let w = r.clone();
        std::thread::Builder::new().name("saver".into()).spawn(move || w.save_worker(rx)).ok();
        r
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap()
    }
    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }
    fn emit(&self, e: Ev) {
        (self.emit)(e)
    }

    // ------------------------------------------------------------------ control
    pub fn start(self: &Arc<Self>) {
        let mut st = self.lock();
        st.gen += 1;
        st.fails = 0;
        st.audio_off = false;
        self.kill(&mut st);
        self.set(&mut st, "starting", "");
        let (me, gen) = (self.clone(), st.gen);
        std::thread::Builder::new().name("recorder".into()).spawn(move || me.launch(gen)).ok();
    }

    pub fn stop(&self, state: &str) {
        let rec = {
            let mut st = self.lock();
            self.stop_inner(&mut st, state)
        };
        if let Some(r) = rec {
            self.jobs.lock().unwrap().send(Job::Recording(r)).ok();
        }
    }

    /// stops everything; a running full recording is handed back to be finished
    fn stop_inner(&self, st: &mut State, state: &str) -> Option<FileRec> {
        let rec = self.stop_file_inner(st);
        st.gen += 1;
        self.kill(st);
        self.ring.reset();
        self.set(st, state, "");
        rec
    }

    pub fn pause(&self) {
        self.stop("paused");
    }

    pub fn redetect(self: &Arc<Self>) {
        self.lock().probe = None;
        self.start();
    }

    fn set(&self, st: &mut State, state: &str, error: &str) {
        st.state = state.into();
        st.error = error.into();
        self.emit(Ev::State(state.into()));
    }

    fn kill(&self, st: &mut State) {
        if let Some(mut m) = st.mixer.take() {
            m.stop();
        }
        if let Some(p) = st.proc.take() {
            let _ = p.kill();
        }
    }

    pub fn state(&self) -> String {
        self.lock().state.clone()
    }
    pub fn rec_only(&self) -> bool {
        self.lock().rec_only
    }
    pub fn clear_rec_only(&self) {
        self.lock().rec_only = false;
    }
    pub fn error(&self) -> String {
        self.lock().error.clone()
    }
    pub fn info(&self) -> Info {
        self.lock().info.clone()
    }
    pub fn filerec_start(&self) -> Option<f64> {
        self.lock().filerec.as_ref().map(|f| f.2)
    }
    pub fn probe_cached(&self) -> Option<Probe> {
        self.lock().probe.clone()
    }

    pub fn status(&self) -> Status {
        let st = self.lock();
        let p = st.probe.clone();
        let buffered = if st.state == "recording" { self.buffered_locked(&st) } else { 0.0 };
        let cfg = self.cfg();
        let want_audio = cfg.b("system_audio") || cfg.b("mic");
        Status {
            state: st.state.clone(),
            error: st.error.clone(),
            buffered,
            file_recording: st.filerec.as_ref().map(|f| f.2),
            replays_off: st.state == "paused" || st.rec_only,
            seconds: cfg.f("seconds"),
            buffer_mb: self.ring.size() as f64 / (1 << 20) as f64,
            size: st.info.size,
            fps: st.info.fps,
            encoder: st.info.encoder.clone(),
            audio_off: st.audio_off || (want_audio && st.mixer.as_ref().is_some_and(|m| !m.working())),
            monitors: p.as_ref().map(|p| p.monitors.clone()).unwrap_or_default(),
            encoders: p.map(|p| p.available).unwrap_or_default(),
        }
    }

    // ------------------------------------------------------------------ ffmpeg
    pub fn log(&self, line: &str) {
        let mut st = self.lock();
        Self::log_to(&mut st, line);
    }
    fn log_to(st: &mut State, line: &str) {
        st.log_lines.push(format!("{} {line}", chrono::Local::now().format("%H:%M:%S")));
        if st.log_lines.len() > 2000 {
            st.log_lines.drain(..500);
        }
    }
    pub fn logger(self: &Arc<Self>) -> Log {
        let me = self.clone();
        Arc::new(move |l| me.log(&l))
    }

    pub fn write_log(&self) {
        let text = self.lock().log_lines.join("\n") + "\n";
        let _ = fs::create_dir_all(C::data_dir());
        let _ = fs::write(C::recorder_log(), text);
    }

    fn build_cmd(&self, st: &State, cfg: &Config, enc: &str, with_audio: bool) -> Vec<String> {
        let probe = st.probe.as_ref().unwrap();
        let vf = probe.filter_for(enc);
        let mut cmd = args(&["-hide_banner", "-nostats", "-v", "verbose"]);
        cmd.extend(sys::capture_args(probe, cfg, None));
        if with_audio {
            // raw PCM needs no probing; by default ffmpeg would wait 5 s of audio before encoding anything
            cmd.extend(args(&["-thread_queue_size", "1024", "-probesize", "32", "-analyzeduration", "0", "-f", "s16le", "-ar", "48000", "-ac", "2", "-i", "-"]));
        }
        cmd.extend(args(&["-map", "0:v"]));
        if with_audio {
            cmd.extend(args(&["-map", "1:a"]));
        }
        if let Some(f) = vf {
            cmd.extend(args(&["-vf", &f]));
        }
        cmd.extend(args(&["-fps_mode", "passthrough"]));
        cmd.extend(sys::encoder_args(enc, &cfg.s("rec_quality"), cfg.i("fps")));
        if with_audio {
            cmd.extend(args(&["-c:a", "aac", "-b:a", "160k"]));
        }
        cmd.extend(args(&["-f", "mpegts", "-flush_packets", "1", "-muxdelay", "0", "-muxpreload", "0", "pipe:1"]));
        cmd
    }

    fn launch(self: Arc<Self>, gen: u64) {
        // the probe starts programs, which can take a while: not under the lock
        let need_probe = self.lock().probe.is_none();
        if need_probe {
            self.emit(Ev::State("probing".into()));
            let force = self.lock().fails >= 4;
            let p = sys::probe(force);
            let mut st = self.lock();
            if st.gen == gen {
                st.probe = Some(p);
            }
        }
        let (child, with_audio, cfg) = {
            let mut st = self.lock();
            if st.gen != gen {
                return;
            }
            let cfg = self.cfg();
            let enc = sys::pick_encoder(st.probe.as_ref().unwrap(), &cfg.s("encoder"));
            let with_audio = (cfg.b("system_audio") || cfg.b("mic")) && !st.audio_off;
            let cmd = self.build_cmd(&st, &cfg, &enc, with_audio);
            Self::log_to(&mut st, &format!("$ {}", cmd.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")));
            self.ring.set_seconds(cfg.f("seconds"));
            let mut pending_rec = None;
            if st.filerec.is_some() {
                // a new ffmpeg means a new stream: finish the running recording
                let keep = st.rec_only;
                st.rec_only = false;
                pending_rec = self.stop_file_inner(&mut st);
                // replays off: that capture only exists for the video, go on in a new file
                st.rec_only = keep;
                st.file_pending = keep;
            }
            if let Some(r) = pending_rec {
                self.jobs.lock().unwrap().send(Job::Recording(r)).ok();
            }
            self.ring.reset();
            st.session = Instant::now();
            let mut c = sys::command(sys::ffmpeg(), ABOVE_NORMAL_PRIORITY_CLASS);
            c.args(&cmd).stdin(if with_audio { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
            let child = match SharedChild::spawn(&mut c) {
                Ok(ch) => Arc::new(ch),
                Err(e) => {
                    Self::log_to(&mut st, &format!("cannot start ffmpeg: {e}"));
                    self.set(&mut st, "error", "ffmpeg is missing");
                    return;
                }
            };
            st.proc = Some(child.clone());
            if with_audio {
                if let Some(stdin) = child.take_stdin() {
                    st.mixer = Some(Mixer::start(&cfg, stdin, self.logger()));
                }
            }
            let (idx, count, w, h) = {
                let probe = st.probe.as_ref().unwrap();
                let mons = &probe.monitors;
                let idx = sys::screen_index(probe, &cfg);
                let m = &mons[idx.min(mons.len().saturating_sub(1))];
                (idx, mons.len(), m.w, m.h)
            };
            if count > 1 {
                Self::log_to(&mut st, &format!("recording screen {} of {count}: {w}x{h}", idx + 1));
            }
            st.info = Info { size: Some((w, h)), fps: cfg.i("fps"), encoder: enc };
            st.started_at = Instant::now();
            (child, with_audio, cfg)
        };
        let _ = cfg;
        // stderr: the log, and the moment the first frame is grabbed (audio sample 0 is pinned to it)
        if let Some(err) = child.take_stderr() {
            let me = self.clone();
            std::thread::spawn(move || me.read_err(err, gen));
        }
        if let Some(out) = child.take_stdout() {
            self.read_out(out, gen);
        }
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let retry = {
            let mut st = self.lock();
            if st.gen != gen {
                return;
            }
            st.fails += 1;
            let fails = st.fails;
            Self::log_to(&mut st, &format!("ffmpeg exited with code {code} (failure {fails})"));
            drop(st);
            self.write_log();
            let mut st = self.lock();
            if st.started_at.elapsed() > Duration::from_secs(60) {
                st.fails = 1; // it ran fine for a while: a hiccup (UAC prompt, mode switch), not a broken setup
            }
            if with_audio && st.fails >= 3 {
                st.audio_off = true;
            }
            if st.fails >= 4 && st.fails % 4 == 0 {
                st.probe = None; // maybe the graphics card / driver changed
            }
            if st.fails >= 8 {
                self.kill(&mut st);
                let e = Self::last_error(&st);
                self.set(&mut st, "error", &if e.is_empty() { "Recording keeps failing".to_string() } else { e });
                return;
            }
            let wait = (1 + st.fails as u64).min(10);
            self.set(&mut st, "starting", "");
            wait
        };
        std::thread::sleep(Duration::from_secs(retry));
        let st = self.lock();
        if st.gen == gen {
            drop(st);
            let me = self.clone();
            std::thread::Builder::new().name("recorder".into()).spawn(move || me.launch(gen)).ok();
        }
    }

    fn read_out(&self, mut out: impl Read, gen: u64) {
        let mut buf = vec![0u8; 1 << 16];
        let mut first = true;
        loop {
            let n = match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            self.ring.feed(&buf[..n]);
            if first && self.ring.has_chunks() {
                first = false;
                let mut st = self.lock();
                if st.gen == gen {
                    self.set(&mut st, "recording", "");
                    if st.file_pending {
                        st.file_pending = false;
                        let _ = self.start_file_inner(&mut st);
                    }
                }
            }
        }
    }

    fn read_err(&self, err: impl Read, gen: u64) {
        use std::io::BufRead;
        let mut r = std::io::BufReader::new(err);
        let mut raw = vec![];
        while r.read_until(b'\n', &mut raw).unwrap_or(0) > 0 {
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            raw.clear();
            let mut st = self.lock();
            if st.gen != gen {
                continue;
            }
            // a log line printed right before the first frame is grabbed
            let first = ["Opened dxgi output", "Capturing whole desktop", "Input #0, x11grab", "Input #0, gdigrab", "Input #0, lavfi"].iter().any(|k| line.contains(k));
            if first {
                if let Some(m) = &st.mixer {
                    m.set_anchor(Instant::now());
                }
            }
            if !line.is_empty() && !line.starts_with("[out#") && !line.starts_with("frame=") {
                Self::log_to(&mut st, &line);
            }
        }
    }

    fn last_error(st: &State) -> String {
        let n = st.log_lines.len();
        for line in st.log_lines[n.saturating_sub(40)..].iter().rev() {
            let low = line.to_lowercase();
            if low.contains("error") || low.contains("failed") || low.contains("cannot") {
                return line.split_once(' ').map(|(_, r)| r.chars().take(160).collect()).unwrap_or_default();
            }
        }
        String::new()
    }

    // ------------------------------------------------------------------ saving
    fn buffered_locked(&self, st: &State) -> f64 {
        // seconds a replay would save now: a replay empties the buffer
        let mut b = self.ring.buffered();
        if self.cfg().b("replay_since_last") {
            if let Some((session, press)) = st.last_save {
                if session == st.session {
                    b = b.min(press.elapsed().as_secs_f64().max(0.0));
                }
            }
        }
        b
    }

    pub fn buffered(&self) -> f64 {
        let st = self.lock();
        self.buffered_locked(&st)
    }

    /// Queue a save; two presses give two clips.
    pub fn save(&self) -> bool {
        {
            let st = self.lock();
            if st.state != "recording" || st.rec_only || !self.ring.has_chunks() {
                return false;
            }
        }
        self.jobs.lock().unwrap().send(Job::Replay(Instant::now())).is_ok()
    }

    fn save_worker(self: Arc<Self>, rx: Receiver<Job>) {
        for job in rx {
            match job {
                Job::Recording(rec) => self.finish_file(rec),
                Job::Replay(t) => {
                    self.emit(Ev::Saving);
                    let due = t + SAVE_DELAY;
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));
                    match self.save_now(t) {
                        Ok((path, length)) => {
                            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            self.emit(Ev::Saved { path, length, size });
                        }
                        Err(e) => {
                            self.log(&format!("save failed: {e}"));
                            self.write_log();
                            self.emit(Ev::SaveFailed(e));
                        }
                    }
                }
            }
        }
    }

    fn new_path(&self, cfg: &Config, ext: &str) -> PathBuf {
        let folder = cfg.output_dir();
        let _ = fs::create_dir_all(&folder);
        let base = format!("{} {}", cfg.prefix(), chrono::Local::now().format("%Y-%m-%d %H-%M-%S"));
        let mut path = folder.join(format!("{base}.{ext}"));
        let mut n = 2;
        while path.exists() {
            path = folder.join(format!("{base} ({n}).{ext}"));
            n += 1;
        }
        path
    }

    /// MPEG-TS (a file, or bytes) → MP4 / MKV, no re-encoding.
    fn remux(&self, src: Option<&Path>, path: &Path, data: Option<&[u8]>) -> Result<(), String> {
        let tmp = PathBuf::from(format!("{}.part", path.display()));
        let mut a = args(&["-hide_banner", "-v", "error", "-y", "-f", "mpegts", "-i"]);
        a.push(src.map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "-".into()));
        a.extend(args(&["-map", "0", "-c", "copy", "-dn", "-avoid_negative_ts", "make_zero"]));
        let mp4 = path.extension().is_some_and(|e| e == "mp4");
        a.extend(if mp4 { args(&["-movflags", "+faststart", "-f", "mp4"]) } else { args(&["-f", "matroska"]) });
        a.push(tmp.to_string_lossy().into_owned());
        let r = run_with_input(&a, data, Duration::from_secs(if data.is_none() { 600 } else { 120 }));
        match r {
            Some(o) if o.ok() && tmp.exists() => {
                fs::rename(&tmp, path).map_err(|e| e.to_string())
            }
            other => {
                let _ = fs::remove_file(&tmp);
                Err(other.map(|o| o.last_err("ffmpeg failed")).unwrap_or_else(|| "ffmpeg failed".into()))
            }
        }
    }

    /// MPEG-TS bytes → MP4 / MKV with the intro laid over the first seconds. The picture has to be encoded
    /// again for that: the encoder the recorder uses, x264 if it can't.
    fn with_intro(&self, cfg: &Config, data: &[u8], path: &Path, number: i64, when: chrono::DateTime<chrono::Local>) -> Result<(), String> {
        let clip = C::data_dir().join("intro.mov");
        let _ = fs::create_dir_all(C::data_dir());
        let info = self.info();
        let h = info.size.map(|s| s.1).unwrap_or(1080) as u32;
        intro::render(&clip, number, when, h, cfg.i("fps") as u32)?;
        let (x, y) = intro::position(h);
        let tmp = PathBuf::from(format!("{}.part", path.display()));
        let enc = if info.encoder.is_empty() { "x264".to_string() } else { info.encoder.clone() };
        let mut encs = vec![enc.clone()];
        if enc != "x264" {
            encs.push("x264".into());
        }
        let mut err = "ffmpeg failed".to_string();
        let mp4 = path.extension().is_some_and(|e| e == "mp4");
        let mut result = Err(err.clone());
        for e in encs {
            let pix = if e == "qsv" || e == "amf" { "nv12" } else { "yuv420p" };
            // ffmpeg starts the replay's timestamps at 0, so the intro's START is seconds into the clip
            let fc = format!("[1:v]setpts=PTS-STARTPTS+{}/TB[ov];[0:v][ov]overlay={x}:{y}:eof_action=pass,format={pix}[v]", intro::START);
            let mut a = args(&["-hide_banner", "-v", "error", "-y", "-f", "mpegts", "-i", "-", "-i", &clip.to_string_lossy(), "-filter_complex", &fc, "-map", "[v]", "-map", "0:a?", "-c:a", "copy", "-dn", "-fps_mode", "passthrough"]);
            a.extend(sys::encoder_args(&e, &cfg.s("rec_quality"), cfg.i("fps")));
            a.extend(if mp4 { args(&["-movflags", "+faststart", "-f", "mp4"]) } else { args(&["-f", "matroska"]) });
            a.push(tmp.to_string_lossy().into_owned());
            match run_with_input(&a, Some(data), Duration::from_secs(600)) {
                Some(o) if o.ok() && fs::metadata(&tmp).map(|m| m.len() > 1000).unwrap_or(false) => {
                    let _ = fs::rename(&tmp, path);
                    result = Ok(());
                    break;
                }
                Some(o) => {
                    err = o.last_err(&err);
                    self.log(&format!("intro export with {e} failed: {err}"));
                    result = Err(err.clone());
                }
                None => result = Err(err.clone()),
            }
        }
        let _ = fs::remove_file(&tmp);
        let _ = fs::remove_file(&clip);
        result
    }

    fn save_now(&self, pressed: Instant) -> Result<(PathBuf, f64), String> {
        let cfg = self.cfg();
        let mut want = cfg.f("seconds");
        let (session, last_save) = {
            let st = self.lock();
            (st.session, st.last_save)
        };
        if cfg.b("replay_since_last") {
            if let Some((s, press)) = last_save {
                if s == session {
                    want = want.min(pressed.saturating_duration_since(press).as_secs_f64().max(1.0)); // only what came after the last one
                }
            }
        }
        let (data, length) = self.ring.snapshot(want).ok_or_else(|| "Nothing recorded yet".to_string())?;
        let prev = {
            let mut st = self.lock();
            let prev = st.last_save;
            st.last_save = Some((session, pressed));
            prev
        };
        let path = self.new_path(&cfg, cfg.ext());
        let number = intro::next_number(&cfg); // every replay counts, intro or not
        let res = if cfg.b("intro") {
            let when = chrono::Local::now() - chrono::Duration::milliseconds(pressed.elapsed().as_millis() as i64);
            self.with_intro(&cfg, &data, &path, number, when).or_else(|e| {
                // a broken intro must not cost the replay
                self.log(&format!("intro failed, saving without it: {e}"));
                self.remux(None, &path, Some(&data))
            })
        } else {
            self.remux(None, &path, Some(&data))
        };
        if let Err(e) = res {
            self.lock().last_save = prev;
            return Err(e);
        }
        intro::commit(number);
        self.cleanup(&cfg, &path);
        Ok((path, length.min(want + 1.0)))
    }

    // ------------------------------------------------------------------ full recordings
    pub fn toggle_file(self: &Arc<Self>) -> bool {
        if self.filerec_start().is_some() {
            self.stop_file(false)
        } else {
            self.start_file()
        }
    }

    /// Write the stream to disk from now until stopped (same encoder, nothing extra to encode).
    pub fn start_file(self: &Arc<Self>) -> bool {
        let mut st = self.lock();
        if st.filerec.is_some() || st.file_pending {
            return false;
        }
        if st.state == "paused" || st.state == "off" {
            // replays are off: capture just for this video, and stop again when it ends
            st.rec_only = true;
            st.file_pending = true;
            st.gen += 1;
            st.fails = 0;
            st.audio_off = false;
            self.kill(&mut st);
            self.set(&mut st, "starting", "");
            let (me, gen) = (self.clone(), st.gen);
            std::thread::Builder::new().name("recorder".into()).spawn(move || me.launch(gen)).ok();
            return true;
        }
        self.start_file_inner(&mut st)
    }

    fn start_file_inner(&self, st: &mut State) -> bool {
        if st.state != "recording" || !self.ring.has_chunks() {
            return false;
        }
        let cfg = self.cfg();
        let ext = cfg.ext();
        let path = self.new_path(&cfg, ext);
        let tmp = PathBuf::from(format!("{}.recording.ts", path.with_extension("").display()));
        let f = match fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => {
                self.emit(Ev::SaveFailed(format!("can't record: {e}")));
                return false;
            }
        };
        st.filerec = Some((tmp, path, C::now()));
        self.ring.start_tap(f);
        self.emit(Ev::RecStarted);
        true
    }

    pub fn stop_file(&self, wait: bool) -> bool {
        let (rec, pending) = {
            let mut st = self.lock();
            let pending = st.file_pending;
            if pending {
                // stopped before the capture was even up
                st.file_pending = false;
                st.rec_only = false;
                let r = self.stop_inner(&mut st, "paused");
                (r, true)
            } else {
                (self.stop_file_inner(&mut st), false)
            }
        };
        if pending {
            return true;
        }
        let Some(rec) = rec else { return false };
        if wait {
            self.finish_file(rec);
        } else {
            self.jobs.lock().unwrap().send(Job::Recording(rec)).ok();
        }
        true
    }

    /// closes the file the stream is tapped into; the recording still has to be finished
    fn stop_file_inner(&self, st: &mut State) -> Option<FileRec> {
        let back_off = std::mem::replace(&mut st.rec_only, false);
        let (tmp, path, t0) = st.filerec.take()?;
        drop(self.ring.stop_tap());
        let length = C::now() - t0;
        self.emit(Ev::RecStopped);
        if back_off {
            // replays stay off, as they were
            st.gen += 1;
            self.kill(st);
            self.ring.reset();
            self.set(st, "paused", "");
        }
        Some(FileRec { tmp, path, t0, length })
    }

    fn finish_file(&self, rec: FileRec) {
        match self.remux(Some(&rec.tmp), &rec.path, None) {
            Ok(()) => {
                let _ = fs::remove_file(&rec.tmp);
                let size = fs::metadata(&rec.path).map(|m| m.len()).unwrap_or(0);
                self.emit(Ev::Recorded { path: rec.path, length: rec.length, size });
            }
            Err(e) => {
                self.log(&format!("recording failed: {e}"));
                self.write_log();
                self.emit(Ev::SaveFailed(format!("recording: {e} (the raw file is {})", rec.tmp.display())));
            }
        }
        let _ = rec.t0;
    }

    /// Delete the oldest clips past the count / size limits.
    fn cleanup(&self, cfg: &Config, keep: &Path) {
        let (max_n, max_gb) = (cfg.i("keep_max_clips"), cfg.f("keep_max_gb"));
        if max_n == 0 && max_gb == 0.0 {
            return;
        }
        let mut clips: Vec<(std::time::SystemTime, u64, PathBuf)> = fs::read_dir(cfg.output_dir())
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        if !(name.ends_with(".mp4") || name.ends_with(".mkv")) {
                            return None;
                        }
                        let m = e.metadata().ok()?;
                        m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        clips.sort_by(|a, b| b.0.cmp(&a.0));
        let mut total = 0u64;
        for (i, (_, size, p)) in clips.iter().enumerate() {
            total += size;
            if p != keep && ((max_n > 0 && i as i64 >= max_n) || (max_gb > 0.0 && total as f64 > max_gb * (1u64 << 30) as f64)) {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// Run ffmpeg feeding `data` to its stdin; the output is collected.
fn run_with_input(a: &[String], data: Option<&[u8]>, timeout: Duration) -> Option<sys::Out> {
    let mut c = sys::command(sys::ffmpeg(), BELOW_NORMAL_PRIORITY_CLASS);
    c.args(a).stdin(if data.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::null()).stderr(Stdio::piped());
    let child = Arc::new(SharedChild::spawn(&mut c).ok()?);
    let feeder = data.map(|d| {
        let mut stdin = child.take_stdin().unwrap();
        let d = d.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&d);
        })
    });
    let mut err = child.take_stderr()?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let waiter = {
        let c = child.clone();
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            let _ = c.kill();
        })
    };
    let status = child.wait().ok()?;
    drop(waiter);
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let stderr = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    Some(sys::Out { code: status.code().unwrap_or(-1), stdout: vec![], stderr })
}
