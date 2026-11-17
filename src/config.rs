//! Settings, paths and constants of DrzReplayWIN.
use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "DrzReplayWIN";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "upidddd/drzreplaywin";
pub const MAX_BUFFER_MB: u64 = 2048; // the ring never grows past this, whatever the length

pub fn install_cmd() -> String {
    format!("irm https://github.com/{REPO}/releases/latest/download/install.ps1 | iex")
}
pub fn setup_url() -> String {
    format!("https://github.com/{REPO}/releases/latest/download/DrzReplayWIN-Setup.exe")
}

pub const WINDOWS: bool = cfg!(windows);

pub fn home() -> PathBuf {
    env::var_os(if WINDOWS { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn dirs() -> (PathBuf, PathBuf) {
    // (settings, data): tests and the self-test keep everything in one folder
    if let Some(h) = env::var_os("DRZREPLAYWIN_HOME") {
        let h = PathBuf::from(h);
        return (h.clone(), h);
    }
    if WINDOWS {
        let conf = env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join(APP_NAME);
        let data = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| conf.clone()).join(APP_NAME);
        (conf, data)
    } else {
        (home().join(".config/drzreplaywin"), home().join(".cache/drzreplaywin"))
    }
}

pub fn conf_dir() -> PathBuf {
    dirs().0
}
pub fn data_dir() -> PathBuf {
    dirs().1
}
pub fn conf_path() -> PathBuf {
    conf_dir().join("config.json")
}
pub fn recorder_log() -> PathBuf {
    data_dir().join("recorder.log")
}
pub fn app_log() -> PathBuf {
    data_dir().join("app.log")
}
pub fn thumbs_dir() -> PathBuf {
    data_dir().join("thumbs")
}
pub fn probe_cache() -> PathBuf {
    data_dir().join("probe.json")
}
pub fn update_cache() -> PathBuf {
    data_dir().join("update.json")
}
pub fn ipc_port_file() -> PathBuf {
    data_dir().join("ipc.port")
}

// ---------------------------------------------------------------- assets (inside the program)
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icons/drzreplaywin.png");
pub const ICON_ICO: &[u8] = include_bytes!("../assets/icons/drzreplaywin.ico");
pub const ICON_SVG: &str = include_str!("../assets/icons/drzreplaywin.svg");
pub const CLICK_WAV: &[u8] = include_bytes!("../assets/sounds/click.wav");
pub const OSWALD: &[u8] = include_bytes!("../assets/fonts/Oswald.ttf");
pub const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");

/// CQ / CRF / QP per preset: lower is better quality and more memory
pub fn rec_quality(name: &str) -> i64 {
    match name {
        "low" => 28,
        "balanced" => 23,
        "high" => 19,
        "ultra" => 16,
        _ => 23,
    }
}

pub fn default_video_dir() -> PathBuf {
    let base = known_videos().unwrap_or_else(|| home().join("Videos"));
    base.join("DrzReplay")
}

#[cfg(windows)]
fn known_videos() -> Option<PathBuf> {
    // FOLDERID_Videos: follows OneDrive / moved folders
    use windows::core::GUID;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    let id = GUID::from_u128(0x18989B1D_99B5_455B_841C_AB7C74E4DDFC);
    unsafe {
        let p = SHGetKnownFolderPath(&id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}
#[cfg(not(windows))]
fn known_videos() -> Option<PathBuf> {
    None
}

// ---------------------------------------------------------------- config
#[derive(Clone, Debug)]
pub struct Config(pub Map<String, Value>);

fn defaults() -> Vec<(&'static str, Value)> {
    vec![
        // capture
        ("seconds", json!(60)),
        ("fps", json!(60)),
        ("monitor", json!(-1)), // ddagrab output index, -1 = the main screen
        ("draw_mouse", json!(true)),
        ("encoder", json!("auto")), // auto | nvenc | amf | qsv | x264
        ("rec_quality", json!("balanced")),
        // sound
        ("system_audio", json!(true)),
        ("system_device", json!("")), // "" = the Windows default, follows it when it changes
        ("system_volume", json!(100)),
        ("mic", json!(false)),
        ("mic_device", json!("")),
        ("mic_volume", json!(100)),
        // saving
        ("output_dir", json!("")),
        ("file_prefix", json!("DrzReplay")),
        ("container", json!("mp4")),
        ("keep_max_clips", json!(0)), // 0 = unlimited
        ("keep_max_gb", json!(0)),
        // controls
        ("hotkey", json!("Alt+Z")),
        ("click_sound", json!(true)),
        ("notifications", json!(true)),
        ("record_hotkey", json!("Alt+R")), // start / stop recording a full video
        ("intro", json!(false)),           // replay number and date in the corner for the first seconds
        ("replay_since_last", json!(true)), // a replay empties the buffer: the next one has only what came after it
        ("tray_hint_shown", json!(false)),
        ("screen_picked", json!(false)), // the user chose the screen (before that "monitor" 0 was only the default)
    ]
}

/// changing any of these restarts the recorder
pub const RECORDER_KEYS: &[&str] =
    &["seconds", "fps", "monitor", "draw_mouse", "encoder", "rec_quality", "system_audio", "system_device", "mic", "mic_device"];

impl Config {
    pub fn defaults() -> Config {
        Config(defaults().into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn exists() -> bool {
        conf_path().exists()
    }
    pub fn load() -> Config {
        let mut cfg = Config::defaults();
        match fs::read_to_string(conf_path()) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(data)) => {
                    for (k, v) in data {
                        if cfg.0.get(&k).is_some_and(|d| same_kind(d, &v)) {
                            cfg.0.insert(k, v);
                        }
                    }
                }
                _ => eprintln!("{APP_NAME}: config is broken, using defaults"),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("{APP_NAME}: config is broken ({e}), using defaults"),
        }
        if !cfg.b("screen_picked") {
            cfg.set("monitor", json!(-1)); // until 1.1.2 screen 1 was the default even when it wasn't the main one
        }
        if cfg.s("output_dir").is_empty() {
            cfg.set("output_dir", json!(default_video_dir().to_string_lossy()));
        }
        cfg
    }
    pub fn save(&self) {
        let mut out = Map::new();
        for (k, _) in defaults() {
            out.insert(k.to_string(), self.0[k].clone());
        }
        write_json_pretty(&conf_path(), &Value::Object(out));
    }
    pub fn get(&self, key: &str) -> &Value {
        self.0.get(key).unwrap_or(&Value::Null)
    }
    pub fn set(&mut self, key: &str, v: Value) {
        self.0.insert(key.to_string(), v);
    }
    pub fn b(&self, key: &str) -> bool {
        match self.get(key) {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            _ => false,
        }
    }
    pub fn f(&self, key: &str) -> f64 {
        self.get(key).as_f64().unwrap_or(0.0)
    }
    pub fn i(&self, key: &str) -> i64 {
        let v = self.get(key);
        v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
    }
    pub fn s(&self, key: &str) -> String {
        self.get(key).as_str().unwrap_or("").to_string()
    }
    pub fn output_dir(&self) -> PathBuf {
        expand_user(&self.s("output_dir"))
    }
    pub fn prefix(&self) -> String {
        let p = self.s("file_prefix");
        if p.is_empty() {
            "DrzReplay".into()
        } else {
            p
        }
    }
    pub fn ext(&self) -> &'static str {
        if self.s("container") == "mkv" {
            "mkv"
        } else {
            "mp4"
        }
    }
}

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Bool(_), Value::Bool(_)) | (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)))
}

pub fn expand_user(p: &str) -> PathBuf {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        home().join(rest)
    } else {
        PathBuf::from(p)
    }
}

// ---------------------------------------------------------------- json helpers
pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}
pub fn read_obj(path: &Path) -> Map<String, Value> {
    match read_json(path) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}
pub fn write_json(path: &Path, data: &Value) {
    write_text(path, &serde_json::to_string(data).unwrap_or_default());
}
pub fn write_json_pretty(path: &Path, data: &Value) {
    write_text(path, &serde_json::to_string_pretty(data).unwrap_or_default());
}
fn write_text(path: &Path, text: &str) {
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}
pub fn update_json(path: &Path, extra: Value) {
    let mut m = read_obj(path);
    if let Value::Object(e) = extra {
        m.extend(e);
    }
    write_json(path, &Value::Object(m));
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}
