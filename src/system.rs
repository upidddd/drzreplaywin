//! Probing the machine (screens, encoders), running ffmpeg, update check, opening files.
use crate::config::{self as C, Config};
use serde_json::{json, Map, Value};
use std::env;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

pub const ENCODER_ORDER: &[&str] = &["nvenc", "amf", "qsv", "x264"];

pub fn encoder_label(name: &str) -> String {
    match name {
        "nvenc" => "NVIDIA NVENC",
        "amf" => "AMD AMF",
        "qsv" => "Intel Quick Sync",
        "x264" => "CPU (x264)",
        other => other,
    }
    .to_string()
}

pub fn ffmpeg() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(d) = exe.parent() {
            for rel in ["ffmpeg/ffmpeg.exe", "ffmpeg.exe", "ffmpeg/ffmpeg", "ffmpeg"] {
                let p = d.join(rel);
                if p.is_file() {
                    return p;
                }
            }
        }
    }
    which("ffmpeg").unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if C::WINDOWS { &["", ".exe"] } else { &[""] };
    env::var_os("PATH").and_then(|p| env::split_paths(&p).flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}")))).find(|f| f.is_file()))
}

/// A command that doesn't flash a console window up on Windows and runs at the given priority.
pub fn command(prog: impl AsRef<std::ffi::OsStr>, priority: u32) -> Command {
    let c = Command::new(prog);
    set_flags(c, priority)
}

#[cfg(windows)]
fn set_flags(mut c: Command, priority: u32) -> Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(CREATE_NO_WINDOW | priority);
    c
}
#[cfg(not(windows))]
fn set_flags(mut c: Command, priority: u32) -> Command {
    use std::os::unix::process::CommandExt;
    if priority == BELOW_NORMAL_PRIORITY_CLASS {
        unsafe {
            c.pre_exec(|| {
                libc::nice(5);
                Ok(())
            });
        }
    }
    c
}

pub struct Out {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    pub fn last_err(&self, fallback: &str) -> String {
        self.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(fallback).trim().to_string()
    }
}

/// Run a command to the end (or the timeout) and collect its output.
pub fn run(mut cmd: Command, timeout: Duration) -> Option<Out> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let (mut so, mut se) = (child.stdout.take()?, child.stderr.take()?);
    let t1 = std::thread::spawn(move || {
        let mut b = vec![];
        let _ = so.read_to_end(&mut b);
        b
    });
    let t2 = std::thread::spawn(move || {
        let mut b = vec![];
        let _ = se.read_to_end(&mut b);
        b
    });
    let end = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    let (o, e) = (t1.join().unwrap_or_default(), t2.join().unwrap_or_default());
    let s = status?;
    Some(Out { code: s.code().unwrap_or(-1), stdout: o, stderr: String::from_utf8_lossy(&e).into_owned() })
}

/// Run ffmpeg with these arguments.
pub fn run_ffmpeg(args: &[String], timeout: Duration, priority: u32) -> Option<Out> {
    let mut c = command(ffmpeg(), priority);
    c.args(args);
    run(c, timeout)
}

pub fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------- probing
pub fn ffmpeg_version() -> Option<String> {
    let o = run_ffmpeg(&args(&["-hide_banner", "-version"]), Duration::from_secs(15), 0)?;
    o.ok().then(|| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string())
}

/// "Opened dxgi output 0 with dimensions 1920x1080" → (1920, 1080)
fn parse_dda_size(log: &str) -> Option<(i64, i64)> {
    let i = log.find("Opened dxgi output")?;
    let rest = &log[i..];
    let j = rest.find("with dimensions ")? + "with dimensions ".len();
    let dims: String = rest[j..].chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
    let (w, h) = dims.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// sizes of every screen ddagrab can capture (outputs of the main graphics card)
fn dda_outputs() -> Vec<(i64, i64)> {
    let mut out = vec![];
    for i in 0..8 {
        let a = args(&["-hide_banner", "-v", "verbose", "-f", "lavfi", "-i", &format!("ddagrab=output_idx={i}:framerate=5"), "-frames:v", "1", "-f", "null", "-"]);
        let Some(r) = run_ffmpeg(&a, Duration::from_secs(20), 0) else { break };
        match parse_dda_size(&r.stderr) {
            Some(s) if r.ok() => out.push(s),
            _ => break,
        }
    }
    out
}

/// (x, y, w, h) of the screens of the main graphics card in desktop pixels, numbered like ddagrab numbers them.
/// ddagrab only reports sizes; the position tells which one is the main screen (it's at 0, 0).
#[cfg(windows)]
fn dxgi_outputs() -> Vec<(i64, i64, i64, i64)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    let mut out = vec![];
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let Ok(adapter) = factory.EnumAdapters1(0) else { return out };
        for i in 0..8 {
            let Ok(o) = adapter.EnumOutputs(i) else { break };
            if let Ok(d) = o.GetDesc() {
                let r = d.DesktopCoordinates;
                out.push((r.left as i64, r.top as i64, (r.right - r.left) as i64, (r.bottom - r.top) as i64));
            }
        }
    }
    out
}
#[cfg(not(windows))]
fn dxgi_outputs() -> Vec<(i64, i64, i64, i64)> {
    vec![]
}

/// (w, h, x, y) of every monitor, in desktop pixels
#[cfg(windows)]
fn gdi_monitors() -> Vec<(i64, i64, i64, i64)> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    unsafe extern "system" fn cb(_m: HMONITOR, _dc: HDC, r: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<(i64, i64, i64, i64)>);
        let r = &*r;
        v.push(((r.right - r.left) as i64, (r.bottom - r.top) as i64, r.left as i64, r.top as i64));
        BOOL(1)
    }
    let mut v: Vec<(i64, i64, i64, i64)> = vec![];
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    if v.is_empty() {
        v.push((1920, 1080, 0, 0));
    }
    v
}
#[cfg(not(windows))]
fn gdi_monitors() -> Vec<(i64, i64, i64, i64)> {
    // development on Linux: the X screen
    let size = which("xdpyinfo").and_then(|_| {
        let mut c = Command::new("xdpyinfo");
        c.stdin(Stdio::null());
        let o = run(c, Duration::from_secs(5))?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let i = text.find("dimensions:")? + "dimensions:".len();
        let dims: String = text[i..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
        let (w, h) = dims.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    });
    let (w, h) = size.unwrap_or((1920, 1080));
    vec![(w, h, 0, 0)]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mon {
    pub w: i64,
    pub h: i64,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub main: bool,
}

#[derive(Clone, Debug)]
pub struct Probe {
    /// "dda" (Desktop Duplication, GPU frames), "gdi", or "x11" (development on Linux)
    pub capture: String,
    pub monitors: Vec<Mon>,
    /// encoder → the filter that gets frames into it (None = no filter)
    pub encoders: Vec<(String, Option<String>)>,
    pub available: Vec<String>,
}

impl Probe {
    fn to_json(&self, version: &Option<String>) -> Value {
        let mons: Vec<Value> = self.monitors.iter().map(|m| json!({"w": m.w, "h": m.h, "x": m.x, "y": m.y, "main": m.main})).collect();
        let mut enc = Map::new();
        for (e, f) in &self.encoders {
            enc.insert(e.clone(), f.clone().map(Value::String).unwrap_or(Value::Null));
        }
        json!({"capture": self.capture, "monitors": mons, "encoders": enc, "available": self.available,
               "ffmpeg": version, "app": C::VERSION, "time": C::now()})
    }
    fn from_json(v: &Value) -> Option<Probe> {
        let monitors = v.get("monitors")?.as_array()?.iter().map(|m| Some(Mon { w: m.get("w")?.as_i64()?, h: m.get("h")?.as_i64()?, x: m.get("x").and_then(|x| x.as_i64()), y: m.get("y").and_then(|x| x.as_i64()), main: m.get("main").and_then(|x| x.as_bool()).unwrap_or(false) })).collect::<Option<Vec<_>>>()?;
        let encoders = v.get("encoders")?.as_object()?.iter().map(|(k, f)| (k.clone(), f.as_str().map(String::from))).collect();
        let available = v.get("available")?.as_array()?.iter().filter_map(|e| e.as_str().map(String::from)).collect();
        Some(Probe { capture: v.get("capture")?.as_str()?.to_string(), monitors, encoders, available })
    }
    pub fn filter_for(&self, enc: &str) -> Option<String> {
        self.encoders.iter().find(|(e, _)| e == enc).and_then(|(_, f)| f.clone())
    }
}

/// filters between the capture and the encoder, best first. ddagrab gives GPU (D3D11) frames,
/// gdigrab / x11grab give BGRA frames in memory.
fn filter_variants(capture: &str, enc: &str) -> Vec<Option<&'static str>> {
    match (capture, enc) {
        ("dda", "nvenc") => vec![None, Some("hwdownload,format=bgra")],
        ("dda", "amf") => vec![None, Some("hwdownload,format=bgra,format=nv12")],
        ("dda", "qsv") => vec![Some("hwmap=derive_device=qsv,format=qsv"), Some("hwdownload,format=bgra,format=nv12")],
        ("dda", "x264") => vec![Some("hwdownload,format=bgra,format=yuv420p")],
        ("gdi", "nvenc") => vec![None],
        ("gdi", "amf") | ("gdi", "qsv") => vec![Some("format=nv12")],
        ("gdi", "x264") => vec![Some("format=yuv420p")],
        ("x11", "nvenc") => vec![None],
        ("x11", "x264") => vec![Some("format=yuv420p")],
        _ => vec![],
    }
}

pub fn screen_index(p: &Probe, cfg: &Config) -> usize {
    let m = cfg.i("monitor");
    if m >= 0 && (m as usize) < p.monitors.len() {
        return m as usize;
    }
    p.monitors.iter().position(|m| m.main).unwrap_or(0)
}

pub fn capture_args(p: &Probe, cfg: &Config, fps: Option<i64>) -> Vec<String> {
    let fps = fps.unwrap_or_else(|| cfg.i("fps"));
    let idx = screen_index(p, cfg);
    let mouse = i32::from(cfg.b("draw_mouse"));
    if p.capture == "dda" {
        return args(&["-f", "lavfi", "-i", &format!("ddagrab=output_idx={idx}:framerate={fps}:draw_mouse={mouse}")]);
    }
    let m = &p.monitors[idx.min(p.monitors.len().saturating_sub(1))];
    let (w, h) = (m.w - m.w % 2, m.h - m.h % 2);
    let (x, y) = (m.x.unwrap_or(0), m.y.unwrap_or(0));
    if p.capture == "gdi" {
        return args(&["-f", "gdigrab", "-thread_queue_size", "64", "-framerate", &fps.to_string(), "-draw_mouse", &mouse.to_string(), "-offset_x", &x.to_string(), "-offset_y", &y.to_string(), "-video_size", &format!("{w}x{h}"), "-i", "desktop"]);
    }
    let display = env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    args(&["-f", "x11grab", "-thread_queue_size", "64", "-framerate", &fps.to_string(), "-draw_mouse", &mouse.to_string(), "-video_size", &format!("{w}x{h}"), "-i", &format!("{display}+{x},{y}")])
}

pub fn encoder_args(enc: &str, quality: &str, fps: i64) -> Vec<String> {
    let q = C::rec_quality(quality).to_string();
    let gop = ["-g".to_string(), fps.to_string()]; // a keyframe every second: clips start at most 1 s early
    let mut v = match enc {
        "nvenc" => args(&["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", &q, "-b:v", "0", "-bf", "0"]),
        "amf" => args(&["-c:v", "h264_amf", "-quality", "speed", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-bf", "0"]),
        "qsv" => args(&["-c:v", "h264_qsv", "-preset", "veryfast", "-global_quality", &q, "-bf", "0"]),
        _ => args(&["-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency", "-crf", &q]),
    };
    v.extend(gop);
    v
}

fn try_encoder(cap: &[String], enc: &str, vf: Option<&str>) -> bool {
    let mut a = args(&["-hide_banner", "-v", "error"]);
    a.extend_from_slice(cap);
    a.extend(args(&["-frames:v", "8"]));
    if let Some(f) = vf {
        a.extend(args(&["-vf", f]));
    }
    a.extend(encoder_args(enc, "balanced", 30));
    a.extend(args(&["-f", "null", "-"]));
    run_ffmpeg(&a, Duration::from_secs(30), 0).is_some_and(|o| o.ok())
}

/// {"capture", "monitors", "encoders": {enc: filter}, "available": [...]}, cached
pub fn probe(force: bool) -> Probe {
    let version = ffmpeg_version();
    if !force {
        let cached = C::read_json(&C::probe_cache());
        if let Some(c) = cached {
            let fresh = c.get("ffmpeg").and_then(|v| v.as_str()) == version.as_deref()
                && c.get("app").and_then(|v| v.as_str()) == Some(C::VERSION)
                && C::now() - c.get("time").and_then(|t| t.as_f64()).unwrap_or(0.0) < 7.0 * 24.0 * 3600.0;
            if fresh {
                if let Some(p) = Probe::from_json(&c) {
                    return p;
                }
            }
        }
    }
    let (capture, mut mons): (String, Vec<Mon>) = if C::WINDOWS {
        let dda = dda_outputs();
        if !dda.is_empty() {
            let mut pos: Vec<(Option<i64>, Option<i64>, i64, i64)> = dxgi_outputs().into_iter().map(|(x, y, w, h)| (Some(x), Some(y), w, h)).collect();
            if pos.iter().map(|p| (p.2, p.3)).collect::<Vec<_>>() != dda {
                pos = dda.iter().map(|(w, h)| (None, None, *w, *h)).collect(); // not the same screens: sizes only
            }
            ("dda".into(), pos.into_iter().map(|(x, y, w, h)| Mon { w, h, x, y, main: false }).collect())
        } else {
            ("gdi".into(), gdi_monitors().into_iter().map(|(w, h, x, y)| Mon { w, h, x: Some(x), y: Some(y), main: false }).collect())
        }
    } else {
        ("x11".into(), gdi_monitors().into_iter().map(|(w, h, x, y)| Mon { w, h, x: Some(x), y: Some(y), main: false }).collect())
    };
    for m in mons.iter_mut() {
        m.main = (m.x, m.y) == (Some(0), Some(0)); // the main screen's corner is the desktop's origin
    }
    if !mons.is_empty() && !mons.iter().any(|m| m.main) {
        mons[0].main = true;
    }
    let mut p = Probe { capture: capture.clone(), monitors: mons, encoders: vec![], available: vec![] };
    let mut cfg = Config::defaults();
    cfg.set("monitor", json!(0));
    cfg.set("draw_mouse", json!(true));
    let cap = capture_args(&p, &cfg, Some(30));
    for enc in ENCODER_ORDER {
        for vf in filter_variants(&capture, enc) {
            if try_encoder(&cap, enc, vf) {
                p.encoders.push((enc.to_string(), vf.map(String::from)));
                break;
            }
        }
    }
    p.available = ENCODER_ORDER.iter().filter(|e| p.encoders.iter().any(|(k, _)| k == *e)).map(|e| e.to_string()).collect();
    C::write_json(&C::probe_cache(), &p.to_json(&version));
    p
}

pub fn pick_encoder(p: &Probe, wanted: &str) -> String {
    if p.available.iter().any(|a| a == wanted) {
        return wanted.to_string();
    }
    p.available.first().cloned().unwrap_or_else(|| "x264".into())
}

/// seconds of video in a file
pub fn media_duration(path: &Path) -> f64 {
    let a = vec!["-hide_banner".to_string(), "-i".to_string(), path.to_string_lossy().into_owned()];
    let Some(r) = run_ffmpeg(&a, Duration::from_secs(15), 0) else { return 0.0 };
    let Some(i) = r.stderr.find("Duration: ") else { return 0.0 };
    let t: String = r.stderr[i + 10..].chars().take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.').collect();
    let p: Vec<f64> = t.split(':').filter_map(|x| x.parse().ok()).collect();
    if p.len() == 3 {
        p[0] * 3600.0 + p[1] * 60.0 + p[2]
    } else {
        0.0
    }
}

// ---------------------------------------------------------------- files
pub fn open_path(path: &str) {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        unsafe {
            ShellExecuteW(None, w!("open"), &HSTRING::from(path), None, None, SW_SHOWNORMAL);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = command("xdg-open", 0).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

pub fn show_in_folder(path: &Path) {
    if C::WINDOWS {
        let _ = command("explorer", 0).arg("/select,").arg(path).spawn();
    } else if let Some(d) = path.parent() {
        open_path(&d.to_string_lossy());
    }
}

// ---------------------------------------------------------------- updates
/// an HTTP client: Windows' own TLS (SChannel), rustls elsewhere
fn builder() -> ureq::AgentBuilder {
    let b = ureq::AgentBuilder::new().user_agent(&format!("{}/{}", C::APP_NAME, C::VERSION));
    #[cfg(windows)]
    let b = match native_tls::TlsConnector::new() {
        Ok(c) => b.tls_connector(std::sync::Arc::new(c)),
        Err(_) => b,
    };
    b
}

fn agent() -> ureq::Agent {
    builder().timeout(Duration::from_secs(30)).build()
}

/// Tag of the newest GitHub release, read from the /releases/latest redirect (no API limits).
pub fn latest_version() -> Option<String> {
    let a = builder().redirects(0).timeout(Duration::from_secs(6)).build();
    let resp = match a.head(&format!("https://github.com/{}/releases/latest", C::REPO)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(_) => return None,
    };
    let loc = resp.header("location")?.to_string();
    let tag = loc.trim_end_matches('/').rsplit('/').next()?.trim_start_matches('v').to_string();
    (!tag.is_empty() && tag.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))).then_some(tag)
}

pub fn version_tuple(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(latest: &str) -> bool {
    version_tuple(latest) > version_tuple(C::VERSION)
}

pub fn download(url: &str, dest: &Path, progress: impl Fn(f32)) -> Result<(), String> {
    let resp = agent().get(url).call().map_err(|e| e.to_string())?;
    let total: u64 = resp.header("Content-Length").and_then(|l| l.parse().ok()).unwrap_or(0);
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut r = resp.into_reader();
    let (mut buf, mut done) = (vec![0u8; 1 << 16], 0u64);
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if total > 0 {
            progress(done as f32 / total as f32);
        }
    }
    drop(f);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}
