//! DrzReplayWIN command line.
//!
//!     DrzReplayWIN.exe                 open the window (starts recording if it isn't running)
//!     DrzReplayWIN.exe --background    start in the tray only (used by "Start with Windows")
//!     DrzReplayWIN.exe --updated       the same, plus an "Updated" notification (run by the installer)
//!     DrzReplayWIN.exe save            save a replay now
//!     DrzReplayWIN.exe record          start / stop recording a full video
//!     DrzReplayWIN.exe quit            stop recording and close
//!     DrzReplayWIN.exe restart         close and start again in the tray
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod config;
mod fmt;
mod fonts;
mod gfx;
mod hotkey;
mod intro;
mod ipc;
mod overlay;
mod recorder;
mod ring;
mod selftest;
mod service;
mod system;
mod tray;
mod trim;
mod ui;
mod winutil;

use config as C;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

/// A windowed program has no console: keep panics in a log file instead of losing them.
fn install_panic_log() {
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all(C::data_dir());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(C::app_log()) {
            let _ = writeln!(f, "{} {info}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    }));
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| argv.iter().any(|a| a == f);
    let value = |f: &str| argv.iter().position(|a| a == f).and_then(|i| argv.get(i + 1)).cloned();
    if has("--version") {
        winutil::attach_parent_console();
        println!("{}", C::VERSION);
        return;
    }
    if let Some(dir) = value("--selftest") {
        install_panic_log();
        winutil::attach_parent_console();
        std::process::exit(selftest::run(&PathBuf::from(dir)));
    }
    install_panic_log();
    winutil::set_dpi_aware();
    if has("--window") {
        std::process::exit(ui::run_window(None));
    }
    if let Some(pid) = value("--after").and_then(|p| p.parse().ok()) {
        winutil::wait_for_exit(pid, 20);
    }
    let cmd = argv.iter().find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| "show".into());
    let quiet = has("--background") || has("--updated");
    // already running: it does the job
    if let Some(reply) = ipc::request(if quiet { "ping" } else { &cmd }, json!({})) {
        winutil::attach_parent_console();
        std::process::exit(if reply.get("ok").and_then(|o| o.as_bool()).unwrap_or(true) { 0 } else { 1 });
    }
    if matches!(cmd.as_str(), "save" | "record" | "quit" | "restart") {
        winutil::attach_parent_console();
        eprintln!("{} is not running", C::APP_NAME);
        std::process::exit(1);
    }
    let (svc, rx) = service::Service::start(!quiet, has("--updated"), false);
    std::process::exit(svc.run(rx));
}
