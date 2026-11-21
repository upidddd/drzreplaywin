//! The running app: owns the settings, the recorder, the hotkeys, the tray and the badge.
//! The window is a separate process (`DrzReplayWIN.exe --window`) that talks to this one.
use crate::config::{self as C, Config};
use crate::fmt::{human_size, human_time};
use crate::hotkey::Hotkey;
use crate::overlay::Overlay;
use crate::recorder::{Ev, Recorder};
use crate::tray::{Act, Look, Tray, View};
use crate::{audio, ipc, system as sys, winutil};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum Msg {
    Rec(Ev),
    Hotkey,
    RecHotkey,
    Tray(Act),
    Devices(Vec<String>, Vec<String>),
    Update(Option<String>, bool),
    UpdateFile(PathBuf),
    UpdateFailed(String),
    Reload,
    Quit(i32),
    Restart,
}

#[derive(PartialEq, Clone, Copy)]
enum Message {
    None,
    Clip,
    Update,
}

struct Svc {
    update_ready: Option<String>,
    update_progress: Option<String>,
    last_clip: Option<PathBuf>,
    last_message: Message,
    devices: Option<(Vec<String>, Vec<String>)>,
    ui: Option<Child>,
    show_requested: bool,
    ui_active_at: Option<Instant>,
    toasts: Vec<String>,
    restart_recorder_at: Option<Instant>,
    quitting: bool,
    ui_was_open: bool,
}

pub struct Service {
    pub cfg: Arc<Mutex<Config>>,
    pub recorder: Arc<Recorder>,
    hotkey: Hotkey,
    rec_hotkey: Hotkey,
    tray: Tray,
    overlay: Overlay,
    tx: Sender<Msg>,
    st: Mutex<Svc>,
    selftest: bool,
    /// what the recorder reported (kept for the self-test only)
    pub seen: Mutex<Vec<Ev>>,
}

impl Service {
    /// Start everything. `show`: open the window; `updated`: say that an update has just been installed.
    pub fn start(show: bool, updated: bool, selftest: bool) -> (Arc<Service>, Receiver<Msg>) {
        let first_run = !Config::exists();
        let cfg = Config::load();
        let cfg = Arc::new(Mutex::new(cfg));
        let (tx, rx) = channel::<Msg>();
        let t = tx.clone();
        let recorder = Recorder::new(cfg.clone(), move |e| {
            let _ = t.send(Msg::Rec(e));
        });
        let c = cfg.lock().unwrap().clone();
        let (t1, t2, t3) = (tx.clone(), tx.clone(), tx.clone());
        let hotkey = Hotkey::new(&c.s("hotkey"), move || {
            let _ = t1.send(Msg::Hotkey);
        });
        let rec_hotkey = Hotkey::new(&c.s("record_hotkey"), move || {
            let _ = t2.send(Msg::RecHotkey);
        });
        let tray = Tray::new(View::default(), move |a| {
            let _ = t3.send(Msg::Tray(a));
        });
        let svc = Arc::new(Service {
            cfg,
            recorder,
            hotkey,
            rec_hotkey,
            tray,
            overlay: Overlay::new(),
            tx,
            st: Mutex::new(Svc { update_ready: None, update_progress: None, last_clip: None, last_message: Message::None, devices: None, ui: None, show_requested: false, ui_active_at: None, toasts: vec![], restart_recorder_at: None, quitting: false, ui_was_open: false }),
            selftest,
            seen: Mutex::new(vec![]),
        });
        {
            let s = svc.clone();
            let _ = ipc::serve(Arc::new(move |cmd, req| s.handle_ipc(cmd, req)));
        }
        if first_run {
            c.save();
            if !selftest {
                winutil::set_autostart(true);
            }
        } else if !selftest {
            winutil::refresh_autostart();
        }
        svc.recorder.start();
        let t = svc.tx.clone();
        std::thread::spawn(move || {
            let (o, i) = audio::devices();
            let _ = t.send(Msg::Devices(o, i));
        });
        svc.update_tray();
        if show || first_run {
            svc.show_window();
            if first_run {
                svc.toast(&format!("Replays are on. Press {} to save one.", c.s("hotkey")));
            }
        }
        if updated && !first_run {
            svc.tray.balloon(C::APP_NAME, &format!("Updated to {}. Still recording.", C::VERSION));
        }
        (svc, rx)
    }

    /// The event loop: returns the exit code once the app is told to quit.
    pub fn run(self: &Arc<Self>, rx: Receiver<Msg>) -> i32 {
        let started = Instant::now();
        let mut checked_first = false;
        let mut next_update = Instant::now() + Duration::from_secs(6 * 3600);
        let mut next_tray = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(m) => {
                    if let Some(code) = self.on_msg(m) {
                        return code;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return 0,
            }
            let now = Instant::now();
            if now >= next_tray {
                next_tray = now + Duration::from_secs(2);
                self.update_tray();
            }
            if !checked_first && !self.selftest && started.elapsed() > Duration::from_secs(60) {
                checked_first = true;
                self.check_updates(false);
            }
            if now >= next_update {
                next_update += Duration::from_secs(6 * 3600);
                self.check_updates(false);
            }
            self.housekeeping();
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, Svc> {
        self.st.lock().unwrap()
    }
    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    fn housekeeping(&self) {
        let mut hint = false;
        let mut restart = false;
        {
            let mut st = self.st();
            if let Some(t) = st.restart_recorder_at {
                if Instant::now() >= t {
                    st.restart_recorder_at = None;
                    restart = true;
                }
            }
            // the window closed
            let open = match st.ui.as_mut() {
                Some(c) => matches!(c.try_wait(), Ok(None)),
                None => false,
            };
            if !open {
                st.ui = None;
            }
            if st.ui_was_open && !open {
                hint = true;
            }
            st.ui_was_open = open;
        }
        if restart && !self.paused() {
            self.recorder.start();
        }
        if hint {
            let mut c = self.cfg();
            if !c.b("tray_hint_shown") {
                c.set("tray_hint_shown", json!(true));
                self.set_cfg(c);
                self.tray.balloon(C::APP_NAME, "Still recording here in the tray. Right click it to quit.");
            }
        }
    }

    // ------------------------------------------------------------------ settings
    /// Take new settings (the window wrote the file and asked us to reload, or we changed one ourselves).
    fn set_cfg(&self, new: Config) {
        let old = {
            let mut g = self.cfg.lock().unwrap();
            let old = g.clone();
            *g = new.clone();
            old
        };
        new.save();
        self.apply_cfg(&old, &new);
    }

    fn reload_config(&self) {
        let new = Config::load();
        let old = {
            let mut g = self.cfg.lock().unwrap();
            let old = g.clone();
            *g = new.clone();
            old
        };
        self.apply_cfg(&old, &new);
    }

    fn apply_cfg(&self, old: &Config, new: &Config) {
        if C::RECORDER_KEYS.iter().any(|k| old.get(k) != new.get(k)) && !self.paused() {
            self.st().restart_recorder_at = Some(Instant::now() + Duration::from_millis(800));
        }
        if old.get("seconds") != new.get("seconds") {
            self.recorder.ring.set_seconds(new.f("seconds"));
        }
        if old.s("hotkey") != new.s("hotkey") {
            self.hotkey.set(&new.s("hotkey"));
        }
        if old.s("record_hotkey") != new.s("record_hotkey") {
            self.rec_hotkey.set(&new.s("record_hotkey"));
        }
        self.update_tray();
    }

    // ------------------------------------------------------------------ actions
    pub fn paused(&self) -> bool {
        self.recorder.state() == "paused"
    }

    pub fn pause(&self) {
        self.recorder.pause();
        self.update_tray();
    }

    pub fn resume(&self) {
        if self.recorder.rec_only() {
            // turned on during a recording: the capture is already up
            self.recorder.clear_rec_only();
        } else if matches!(self.recorder.state().as_str(), "paused" | "error" | "off") {
            self.recorder.start();
        }
        self.update_tray();
    }

    pub fn toggle_recording(&self) -> bool {
        if self.recorder.filerec_start().is_some() {
            self.recorder.stop_file(false);
            return true;
        }
        if self.recorder.start_file() {
            return true;
        }
        self.badge("The recorder isn't working", &self.recorder.error(), false);
        false
    }

    pub fn save_replay(&self) -> bool {
        if self.recorder.save() {
            return true;
        }
        let state = self.recorder.state();
        let msg = if state == "paused" || self.recorder.rec_only() {
            "Replays are off".to_string()
        } else if state != "error" {
            "Nothing recorded yet".to_string()
        } else {
            format!("Not recording: {}", if self.recorder.error().is_empty() { "error".to_string() } else { self.recorder.error() })
        };
        self.badge(&msg, "", false);
        false
    }

    pub fn open_folder(&self) {
        let d = self.cfg().output_dir();
        let _ = std::fs::create_dir_all(&d);
        sys::open_path(&d.to_string_lossy());
    }

    pub fn show_window(&self) {
        if self.selftest {
            return;
        }
        let mut st = self.st();
        let open = st.ui.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if open {
            st.show_requested = true; // the window raises itself the next time it asks
            return;
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut c = std::process::Command::new(exe);
            c.arg("--window").stdin(std::process::Stdio::null());
            st.ui = c.spawn().ok();
            st.ui_was_open = st.ui.is_some();
        }
    }

    pub fn toast(&self, text: &str) {
        self.st().toasts.push(text.to_string());
    }

    /// Restart the whole app over (when something got stuck): a new copy waits for this one to close.
    fn restart(&self) -> i32 {
        winutil::spawn_self(&["--background", "--after", &std::process::id().to_string()]);
        self.quit(0)
    }

    fn quit(&self, code: i32) -> i32 {
        self.st().quitting = true;
        self.hotkey.stop();
        self.rec_hotkey.stop();
        self.recorder.stop_file(true); // don't lose a running recording
        self.recorder.stop("off");
        self.recorder.write_log();
        self.tray.hide();
        ipc::remove_port_file();
        if let Some(mut c) = self.st().ui.take() {
            let _ = c.kill();
        }
        code
    }

    /// "Recording saved" and the like, in the corner over everything (games included); the window shows
    /// a toast instead when it is the active window.
    fn badge(&self, title: &str, detail: &str, ok: bool) {
        let ui_active = self.st().ui_active_at.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
        if ui_active {
            let text = if detail.is_empty() { title.to_string() } else { format!("{title} · {detail}") };
            self.toast(&text);
        } else if self.cfg().b("notifications") {
            self.overlay.show_text(title, detail, ok);
        }
    }

    // ------------------------------------------------------------------ events
    fn on_msg(self: &Arc<Self>, m: Msg) -> Option<i32> {
        match m {
            Msg::Hotkey => {
                self.save_replay();
            }
            Msg::RecHotkey => {
                self.toggle_recording();
            }
            Msg::Tray(a) => return self.on_tray(a),
            Msg::Rec(e) => self.on_rec(e),
            Msg::Devices(o, i) => self.st().devices = Some((o, i)),
            Msg::Update(latest, manual) => self.on_update(latest, manual),
            Msg::UpdateFile(p) => {
                // the installer closes nothing of ours: we quit first, it starts the new version when done
                if let Ok(mut c) = std::process::Command::new(&p).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]).spawn() {
                    std::thread::spawn(move || {
                        let _ = c.wait();
                    });
                }
                return Some(self.quit(0));
            }
            Msg::UpdateFailed(e) => {
                self.st().update_progress = None;
                self.notify(&format!("Update failed: {e}"));
            }
            Msg::Reload => self.reload_config(),
            Msg::Quit(code) => return Some(self.quit(code)),
            Msg::Restart => return Some(self.restart()),
        }
        None
    }

    fn on_tray(self: &Arc<Self>, a: Act) -> Option<i32> {
        match a {
            Act::Show => self.show_window(),
            Act::Save => {
                self.save_replay();
            }
            Act::ToggleRecord => {
                self.toggle_recording();
            }
            Act::TogglePause => {
                if self.paused() || self.recorder.rec_only() {
                    self.resume()
                } else {
                    self.pause()
                }
            }
            Act::Update => self.run_update(),
            Act::OpenFolder => self.open_folder(),
            Act::Restart => return Some(self.restart()),
            Act::Quit => return Some(self.quit(0)),
            Act::BalloonClicked => {
                let (msg, clip) = {
                    let st = self.st();
                    (st.last_message, st.last_clip.clone())
                };
                if msg == Message::Update && self.st().update_ready.is_some() {
                    self.run_update();
                } else if let Some(c) = clip.filter(|c| c.exists()) {
                    sys::show_in_folder(&c);
                }
            }
        }
        None
    }

    fn on_rec(&self, e: Ev) {
        if self.selftest {
            self.seen.lock().unwrap().push(e.clone());
        }
        let cfg = self.cfg();
        match e {
            Ev::RecStarted => {
                if cfg.b("click_sound") {
                    winutil::play_click();
                }
                let rk = cfg.s("record_hotkey");
                self.badge("Recording", &if rk.is_empty() { String::new() } else { format!("{rk} to stop") }, true);
                self.update_tray();
            }
            Ev::RecStopped => {
                if cfg.b("click_sound") {
                    winutil::play_click();
                }
                self.update_tray();
            }
            Ev::Recorded { path, length, size } => {
                {
                    let mut st = self.st();
                    st.last_clip = Some(path);
                    st.last_message = Message::Clip;
                }
                self.badge("Recording saved", &format!("{} · {}", human_time(length), human_size(size as f64)), true);
            }
            Ev::Saving => {
                if cfg.b("click_sound") {
                    winutil::play_click();
                }
            }
            Ev::Saved { path, length, size } => {
                {
                    let mut st = self.st();
                    st.last_clip = Some(path);
                    st.last_message = Message::Clip;
                }
                self.badge("Replay saved", &format!("{} · {}", human_time(length), human_size(size as f64)), true);
            }
            Ev::SaveFailed(e) => self.badge("Couldn't save the replay", &e.chars().take(80).collect::<String>(), false),
            Ev::State(_) => self.update_tray(),
        }
    }

    fn notify(&self, text: &str) {
        let ui_active = self.st().ui_active_at.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
        if ui_active {
            self.toast(text);
        } else if self.cfg().b("notifications") {
            self.tray.balloon(C::APP_NAME, text);
        }
    }

    // ------------------------------------------------------------------ tray
    fn tray_view(&self) -> View {
        let cfg = self.cfg();
        let state = self.recorder.state();
        let paused = state == "paused";
        let rec_only = self.recorder.rec_only();
        let off = paused || rec_only;
        let filerec = self.recorder.filerec_start();
        let on = state == "recording" && !rec_only;
        let record_label = match filerec {
            Some(t0) => {
                let s = (C::now() - t0) as i64;
                format!("Stop recording · {}:{:02}", s / 60, s % 60)
            }
            None => {
                let rk = cfg.s("record_hotkey");
                format!("Start recording{}", if rk.is_empty() { String::new() } else { format!(" ({rk})") })
            }
        };
        let tip = if filerec.is_some() {
            "recording a video"
        } else {
            match state.as_str() {
                "recording" => "recording",
                "paused" => "paused",
                "error" => "not recording",
                _ => "starting…",
            }
        };
        View {
            update_label: self.st().update_ready.as_ref().map(|u| format!("Update to {u}…")),
            save_label: format!("Save replay ({})", cfg.s("hotkey")),
            save_enabled: on,
            record_label,
            record_enabled: on || paused || filerec.is_some(), // with replays off it records just the video
            pause_label: if off { "Turn replays on" } else { "Turn replays off" }.into(),
            open_label: format!("Open {}", C::APP_NAME),
            look: if state == "recording" || filerec.is_some() {
                Look::Rec
            } else if paused || state == "error" {
                Look::Off
            } else {
                Look::On
            },
            tooltip: format!("{}: {tip}", C::APP_NAME),
        }
    }

    fn update_tray(&self) {
        self.tray.update(self.tray_view());
    }

    // ------------------------------------------------------------------ updates
    pub fn check_updates(&self, manual: bool) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let latest = sys::latest_version();
            if let Some(l) = &latest {
                C::update_json(&C::update_cache(), json!({"time": C::now(), "latest": l}));
            }
            let _ = tx.send(Msg::Update(latest, manual));
        });
    }

    fn on_update(&self, latest: Option<String>, manual: bool) {
        let newer = latest.as_deref().filter(|l| sys::is_newer(l)).map(String::from);
        if let Some(n) = &newer {
            self.st().update_ready = Some(n.clone());
            self.update_tray();
        }
        if let (Some(n), false) = (newer, manual) {
            let cache = C::read_obj(&C::update_cache());
            let same = cache.get("notified").and_then(|v| v.as_str()) == Some(n.as_str());
            let at = cache.get("notified_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !same || C::now() - at > 24.0 * 3600.0 {
                C::update_json(&C::update_cache(), json!({"notified": n, "notified_at": C::now()}));
                self.st().last_message = Message::Update;
                self.tray.balloon(&format!("{} {n} is available", C::APP_NAME), "Click here or use Update in the tray menu.");
            }
        }
    }

    pub fn run_update(&self) {
        self.st().update_progress = Some("Downloading the update…".into());
        let dest = std::env::temp_dir().join(format!("{}-Setup.exe", C::APP_NAME));
        let tx = self.tx.clone();
        std::thread::spawn(move || match sys::download(&C::setup_url(), &dest, |_| {}) {
            Ok(()) => {
                let _ = tx.send(Msg::UpdateFile(dest));
            }
            Err(e) => {
                let _ = tx.send(Msg::UpdateFailed(e));
            }
        });
    }

    // ------------------------------------------------------------------ the window and the command line
    fn status_json(&self, active: bool) -> Value {
        let (update_ready, progress, devices, show, toasts) = {
            let mut st = self.st();
            if active {
                st.ui_active_at = Some(Instant::now());
            }
            let show = std::mem::take(&mut st.show_requested);
            (st.update_ready.clone(), st.update_progress.clone(), st.devices.clone(), show, std::mem::take(&mut st.toasts))
        };
        let s = self.recorder.status();
        let hk = |h: &Hotkey| json!({"ok": h.ok(), "error": h.error()});
        json!({
            "version": C::VERSION,
            "state": s.state, "error": s.error, "buffered": s.buffered, "file_recording": s.file_recording,
            "replays_off": s.replays_off, "seconds": s.seconds, "buffer_mb": s.buffer_mb,
            "size": s.size.map(|(w, h)| json!([w, h])), "fps": s.fps, "encoder": s.encoder, "audio_off": s.audio_off,
            "monitors": s.monitors.iter().map(|m| json!({"w": m.w, "h": m.h, "main": m.main})).collect::<Vec<_>>(),
            "encoders": s.encoders,
            "hotkey": hk(&self.hotkey), "rec_hotkey": hk(&self.rec_hotkey),
            "update_ready": update_ready, "update_progress": progress,
            "devices": devices.map(|(o, i)| json!({"outs": o, "ins": i})),
            "show": show, "toasts": toasts,
        })
    }

    fn handle_ipc(&self, cmd: &str, req: &Value) -> Value {
        match cmd {
            "ping" => json!({}),
            "show" => {
                self.show_window();
                json!({})
            }
            "status" => self.status_json(req.get("active").and_then(|a| a.as_bool()).unwrap_or(false)),
            "save" => json!({"ok": self.save_replay()}),
            "record" => json!({"ok": self.toggle_recording()}),
            "resume" => {
                self.resume();
                json!({})
            }
            "pause" => {
                self.pause();
                json!({})
            }
            "redetect" => {
                self.recorder.redetect();
                json!({})
            }
            "reload" => {
                let _ = self.tx.send(Msg::Reload);
                json!({})
            }
            "run_update" => {
                self.run_update();
                json!({})
            }
            "check_update" => {
                self.check_updates(true);
                json!({})
            }
            "hotkey_hold" => {
                // the window is asking for a new key: let go of the old one so it can be pressed
                let which = req.get("which").and_then(|w| w.as_str()).unwrap_or("hotkey");
                let on = req.get("on").and_then(|o| o.as_bool()).unwrap_or(true);
                let hk = if which == "hotkey" { &self.hotkey } else { &self.rec_hotkey };
                hk.set(&if on { String::new() } else { self.cfg().s(which) });
                json!({})
            }
            "log" => {
                self.recorder.write_log();
                json!({"path": C::recorder_log().to_string_lossy()})
            }
            "quit" => {
                let _ = self.tx.send(Msg::Quit(0));
                json!({})
            }
            "restart" => {
                let _ = self.tx.send(Msg::Restart);
                json!({})
            }
            _ => json!({"ok": false, "error": "unknown command"}),
        }
    }

    /// For the self-test: the events a real user would cause.
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    pub fn hotkey_state(&self) -> (bool, String) {
        (self.hotkey.ok(), self.hotkey.error())
    }
}
