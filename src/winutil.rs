//! Small Windows things: start with Windows, the click sound, waiting for another process, DPI.
use crate::config as C;

pub const DETACHED_PROCESS: u32 = 0x0000_0008;

// ---------------------------------------------------------------- start with Windows
// a value under HKCU\...\Run (what Task Manager → Startup apps shows)
#[cfg(windows)]
mod reg {
    use crate::config as C;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_READ};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn command() -> String {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        format!("\"{exe}\" --background")
    }

    pub fn enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ).and_then(|k| k.get_value::<String, _>(C::APP_NAME)).is_ok_and(|v| !v.is_empty())
    }

    pub fn set_enabled(on: bool) -> std::io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
        if on {
            key.set_value(C::APP_NAME, &command())
        } else {
            match key.delete_value(C::APP_NAME) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }
}

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    reg::enabled()
}
#[cfg(windows)]
pub fn set_autostart(on: bool) {
    if let Err(e) = reg::set_enabled(on) {
        eprintln!("autostart: {e}");
    }
}
#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}
#[cfg(not(windows))]
pub fn set_autostart(_on: bool) {}

/// After an update or a move of the install folder, point the entry at this exe again.
pub fn refresh_autostart() {
    if autostart_enabled() {
        set_autostart(true);
    }
}

// ---------------------------------------------------------------- the click sound
pub fn play_click() {
    #[cfg(windows)]
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        let _ = PlaySoundW(PCWSTR(C::CLICK_WAV.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

// ---------------------------------------------------------------- processes
/// A restart: the old copy is still saving and closing, take over only once it is gone.
pub fn wait_for_exit(pid: u32, timeout_secs: u64) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, (timeout_secs * 1000) as u32);
            let _ = CloseHandle(h);
        }
    }
    #[cfg(not(windows))]
    {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < end && std::path::Path::new(&format!("/proc/{pid}")).exists() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// Start this program again with other arguments, on its own (not tied to us).
pub fn spawn_self(args: &[&str]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut c = std::process::Command::new(exe);
    c.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(DETACHED_PROCESS);
    }
    c.spawn().is_ok()
}

/// The process and its windows scale with the screen like Windows' own (per-monitor DPI).
pub fn set_dpi_aware() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        use windows::core::w;
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        let _ = SetCurrentProcessExplicitAppUserModelID(w!("DRZ.DrzReplayWIN"));
    }
}

/// UI scale of the primary screen (1.0 = 96 dpi)
pub fn dpi_scale() -> f32 {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::GetDpiForSystem;
        return GetDpiForSystem() as f32 / 96.0;
    }
    #[cfg(not(windows))]
    1.0
}
