//! --selftest DIR: record, save, press the hotkey, take screenshots of every tab, write DIR/report.json.
//!
//! Run by the build on a real Windows machine; exits with 0 when a clip of the right length was saved.
use crate::config::{self as C, Config};
use crate::recorder::Ev;
use crate::service::{Msg, Service};
use crate::ui::{self, Script, Step};
use crate::{hotkey, intro, overlay, system as sys, trim, winutil};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Test {
    svc: Arc<Service>,
    out: PathBuf,
    t0: Instant,
    steps: Vec<String>,
    report: serde_json::Map<String, Value>,
}

#[derive(Clone)]
struct Clip {
    path: PathBuf,
    length: f64,
}

impl Test {
    fn log(&mut self, text: &str) {
        let line = format!("{:6.1}s {text}", self.t0.elapsed().as_secs_f64());
        eprintln!("{line}");
        self.steps.push(line);
    }

    fn wait(&self, secs: u64, mut cond: impl FnMut(&Test) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if cond(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn saved(&self) -> Vec<Clip> {
        self.svc.seen.lock().unwrap().iter().filter_map(|e| if let Ev::Saved { path, length, .. } = e { Some(Clip { path: path.clone(), length: *length }) } else { None }).collect()
    }
    fn recorded(&self) -> Vec<Clip> {
        self.svc.seen.lock().unwrap().iter().filter_map(|e| if let Ev::Recorded { path, length, .. } = e { Some(Clip { path: path.clone(), length: *length }) } else { None }).collect()
    }
    fn failures(&self) -> Vec<String> {
        self.svc.seen.lock().unwrap().iter().filter_map(|e| if let Ev::SaveFailed(m) = e { Some(m.clone()) } else { None }).collect()
    }
    fn set_cfg(&self, key: &str, v: Value) {
        self.svc.cfg.lock().unwrap().set(key, v);
    }
    fn state(&self) -> String {
        self.svc.recorder.state()
    }
}

fn stream_lines(path: &Path) -> Vec<String> {
    let a = vec!["-hide_banner".to_string(), "-i".to_string(), path.to_string_lossy().into_owned()];
    sys::run_ffmpeg(&a, Duration::from_secs(30), 0).map(|r| r.stderr.lines().filter(|l| l.contains("Stream #")).map(|l| l.trim().to_string()).collect()).unwrap_or_default()
}

pub fn run(out: &Path) -> i32 {
    let _ = std::fs::create_dir_all(out);
    let out = out.canonicalize().unwrap_or(out.to_path_buf());
    std::env::set_var("DRZREPLAYWIN_HOME", out.join("home"));
    winutil::set_dpi_aware();
    // the test's own settings: a short buffer, clips in the report folder
    let mut cfg = Config::load();
    cfg.set("output_dir", json!(out.join("clips").to_string_lossy()));
    cfg.set("seconds", json!(6));
    cfg.save();
    let (svc, rx) = Service::start(false, false, true);
    let s2 = svc.clone();
    let loop_thread = std::thread::spawn(move || s2.run(rx));
    let mut t = Test { svc: svc.clone(), out: out.clone(), t0: Instant::now(), steps: vec![], report: serde_json::Map::new() };
    t.report.insert("version".into(), json!(C::VERSION));
    t.report.insert("platform".into(), json!(std::env::consts::OS));
    // DRZ_SHOTS_ONLY: just the pictures of the window, with the clips of an earlier run
    let shots_only = std::env::var_os("DRZ_SHOTS_ONLY").is_some();
    let ok = if shots_only { std::thread::sleep(Duration::from_secs(3)); true } else { steps(&mut t) };
    // the window's screenshots (they need the app's status, so the app is still running)
    let shots = screenshots(&mut t);
    t.report.insert("screenshots".into(), json!(shots));
    finish(&mut t, ok);
    svc.send(Msg::Quit(0));
    let _ = loop_thread.join();
    if ok {
        0
    } else {
        1
    }
}

fn steps(t: &mut Test) -> bool {
    // recording
    let up = t.wait(120, |t| matches!(t.state().as_str(), "recording" | "error"));
    if !up || t.state() == "error" {
        let e = t.svc.recorder.error();
        t.log(&format!("recorder error: {e}"));
        return false;
    }
    let st = t.svc.recorder.status();
    t.log(&format!("recording: {st:?}"));
    t.report.insert("probe".into(), json!({"monitors": st.monitors.iter().map(|m| json!({"w": m.w, "h": m.h, "main": m.main})).collect::<Vec<_>>(), "encoders": st.encoders, "encoder": st.encoder}));

    if !t.wait(60, |t| t.svc.recorder.ring.buffered() >= 5.9) {
        t.log("the buffer never filled up");
        return false;
    }
    t.log(&format!("buffer {:.1} s, {:.1} MB; saving the whole buffer (6 s)", t.svc.recorder.ring.buffered(), t.svc.recorder.ring.size() as f64 / 1e6));
    if !t.svc.save_replay() {
        t.log("save was refused");
        return false;
    }
    if !t.wait(60, |t| !t.saved().is_empty()) {
        t.log(&format!("no clip was saved: {:?}", t.failures()));
        return false;
    }
    let first = t.saved()[0].clone();
    let dur = sys::media_duration(&first.path);
    t.report.insert("first_clip".into(), json!({"path": first.path.to_string_lossy(), "length": first.length, "duration": dur, "streams": stream_lines(&first.path)}));
    t.log(&format!("saved {} ({dur:.2} s)", first.path.display()));

    // the hotkey, pressed like a user does
    let (ok, err) = t.svc.hotkey_state();
    t.log(&format!("hotkey ok={ok} error={err:?}"));
    if !cfg!(windows) || !ok {
        t.report.insert("hotkey".into(), json!("not registered"));
    } else {
        for (vk, up) in [(0x12, false), (0x5A, false), (0x5A, true), (0x12, true)] {
            winutil::send_key(vk, up); // Alt down, Z down, Z up, Alt up
            std::thread::sleep(Duration::from_millis(30));
        }
        if t.wait(8, |t| t.saved().len() >= 2) {
            t.report.insert("hotkey".into(), json!("saved a clip"));
            // pressed ~1 s after the first replay: with the buffer cleared it holds about that much
            let d = sys::media_duration(&t.saved()[1].path);
            t.report.insert("since_last".into(), json!(format!("{d:.2} s ({})", if d < 3.5 { "ok" } else { "TOO LONG" })));
            t.log(&format!("hotkey clip, {d:.2} s"));
        } else {
            t.report.insert("hotkey".into(), json!("no clip after the key press"));
        }
    }

    // a full recording of 4 seconds
    let started = t.svc.toggle_recording();
    t.log(&format!("recording started: {started}"));
    std::thread::sleep(Duration::from_secs(4));
    t.svc.toggle_recording();
    if t.wait(60, |t| !t.recorded().is_empty()) {
        let r = t.recorded()[0].clone();
        let d = sys::media_duration(&r.path);
        t.report.insert("recording".into(), json!(format!("{d:.2} s for 4 s ({})", if (3.5..=6.0).contains(&d) { "ok" } else { "WRONG" })));
        t.log(&format!("recorded {d:.2} s"));
    } else {
        t.report.insert("recording".into(), json!("never finished (WRONG)"));
    }

    // the intro
    t.set_cfg("intro", json!(true));
    let before = t.saved().len();
    t.log("saving a replay with the intro");
    // the buffer has to have something after the last replay
    std::thread::sleep(Duration::from_secs(3));
    t.svc.save_replay();
    if t.wait(90, |t| t.saved().len() > before) {
        let clip = t.saved().last().cloned().unwrap();
        let d = sys::media_duration(&clip.path);
        // a frame while the intro is on screen, for a look at the artifact
        let shot = t.out.join("intro-frame.png");
        let a = sys::args(&["-v", "error", "-y", "-ss", "1.2", "-i", &clip.path.to_string_lossy(), "-frames:v", "1", "-vf", "crop=iw/2:ih/3:0:0", &shot.to_string_lossy()]);
        let _ = sys::run_ffmpeg(&a, Duration::from_secs(30), 0);
        let good = d > 0.5 && shot.exists();
        t.report.insert("intro".into(), json!(format!("{d:.2} s, frame {} ({})", if shot.exists() { "saved" } else { "MISSING" }, if good { "ok" } else { "FAILED" })));
        t.log(&format!("intro clip, {d:.2} s"));
    } else {
        t.report.insert("intro".into(), json!("no clip (FAILED)"));
    }
    t.set_cfg("intro", json!(false));

    // replays off, then a full recording: the capture starts just for it and stops after
    t.svc.pause();
    let started = t.svc.toggle_recording();
    t.log(&format!("recording with replays off: {started}, state {}", t.state()));
    let n = t.recorded().len();
    if t.wait(40, |t| t.svc.recorder.filerec_start().is_some()) {
        std::thread::sleep(Duration::from_secs(5));
        t.svc.toggle_recording();
        if t.wait(60, |t| t.recorded().len() > n) {
            let d = sys::media_duration(&t.recorded()[n].path);
            let state = t.state();
            let ok = d >= 2.0 && state == "paused";
            t.report.insert("paused_recording".into(), json!(format!("{d:.2} s, recorder {state} after ({})", if ok { "ok" } else { "WRONG" })));
        } else {
            t.report.insert("paused_recording".into(), json!("never finished (WRONG)"));
        }
    } else {
        t.report.insert("paused_recording".into(), json!("never started (WRONG)"));
    }
    t.svc.resume();

    // trim 1…4 s out of the first replay with the recorder's encoder, then rename the result
    let enc = t.svc.recorder.info().encoder;
    let cfg = t.svc.cfg.lock().unwrap().clone();
    let r = trim::trim(&cfg, &enc, &first.path, 1.0, 4.0, false).and_then(|out| {
        let d = sys::media_duration(&out);
        let new = trim::rename(&out, "my clip")?;
        let ok = (2.8..=3.3).contains(&d) && new.exists() && !trim::is_auto_name(&new);
        Ok(format!("{d:.2} s for 3 s with {enc}, renamed to {} ({})", new.file_name().unwrap().to_string_lossy(), if ok { "ok" } else { "WRONG" }))
    });
    let msg = match r {
        Ok(m) => m,
        Err(e) => format!("failed: {e} (WRONG)"),
    };
    t.log(&format!("trim + rename: {msg}"));
    t.report.insert("trim_rename".into(), json!(msg));

    // the badge and the intro, as pictures
    let (w, h, canvas) = overlay::render("Replay saved", "0:20 · 31.4 MB", true, 1.0);
    if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, canvas.to_rgba()) {
        let _ = img.save(t.out.join("overlay.png"));
    }
    let p = intro::Painter::new(12, chrono::Local::now(), 1080);
    let (st, _) = (p.state(1.2), 0);
    if let Some(img) = image::RgbaImage::from_raw(p.w as u32, p.h as u32, p.draw(st.0, st.1, st.2)) {
        let _ = img.save(t.out.join("intro-painter.png"));
    }
    true
}

fn screenshots(t: &mut Test) -> String {
    let existing = std::fs::read_dir(t.out.join("clips")).ok().and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "mp4")));
    let Some(path) = t.saved().first().map(|c| c.path.clone()).or(existing) else { return "no clip to show".into() };
    let clip = Clip { path, length: 0.0 };
    let mut steps = vec![];
    for (theme, dark) in [("light", false), ("dark", true)] {
        steps.push(Step::Theme(dark));
        for (i, name) in ["record", "clips", "settings"].iter().enumerate() {
            steps.push(Step::Tab(i));
            steps.push(Step::Shot(format!("{name}-{theme}.png")));
        }
    }
    steps.push(Step::Theme(false));
    steps.push(Step::Trim(clip.path));
    steps.push(Step::Shot("trim.png".into()));
    steps.push(Step::CloseDialog);
    let script = Script { out: t.out.clone(), steps, at: 0, wait: 30 };
    t.log("taking screenshots");
    // the window runs in this thread; the app keeps going in the others
    let code = ui::run_window(Some(script));
    t.log(&format!("screenshots done ({code})"));
    if code == 0 {
        "taken".into()
    } else {
        "the window failed to open".into()
    }
}

fn finish(t: &mut Test, ok: bool) {
    let r = &t.report;
    let has = |k: &str, w: &str| r.get(k).and_then(|v| v.as_str()).is_some_and(|s| s.contains(w));
    let first_ok = r.get("first_clip").and_then(|c| c.get("duration")).and_then(|d| d.as_f64()).is_some_and(|d| (5.0..=8.0).contains(&d));
    let good = ok && first_ok && has("recording", "ok") && !has("since_last", "TOO") && has("intro", "ok") && has("paused_recording", "ok") && has("trim_rename", "ok");
    t.report.insert("ok".into(), json!(good));
    let steps = t.steps.clone();
    t.report.insert("steps".into(), json!(steps));
    t.svc.recorder.write_log();
    let log: Vec<String> = std::fs::read_to_string(C::recorder_log()).map(|l| l.lines().rev().take(120).map(String::from).collect::<Vec<_>>().into_iter().rev().collect()).unwrap_or_default();
    t.report.insert("recorder_log".into(), json!(log));
    let _ = std::fs::write(t.out.join("report.json"), serde_json::to_string_pretty(&Value::Object(t.report.clone())).unwrap_or_default());
    let _ = hotkey::solo_ok("F9");
}
