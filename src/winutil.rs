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

// ---------------------------------------------------------------- the window's helpers
/// Virtual-key codes that are held down right now (for the hotkey dialog: any key, any layout).
pub fn keys_down() -> Vec<u32> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        return (0x08u32..=0xFE).filter(|vk| GetAsyncKeyState(*vk as i32) as u16 & 0x8000 != 0).collect();
    }
    #[cfg(not(windows))]
    vec![]
}

/// Put a file on the clipboard the way Explorer does: paste it into Discord, Telegram…
pub fn copy_file_to_clipboard(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        use clipboard_win::{formats::FileList, Clipboard, Setter};
        let p = path.to_string_lossy().into_owned();
        let Ok(_clip) = Clipboard::new_attempts(10) else { return false };
        return FileList.write_clipboard(&[p.as_str()]).is_ok();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Move a file to the Recycle Bin.
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        trash::delete(path).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// Ask for a folder (a native dialog).
pub fn pick_folder(start: &std::path::Path, title: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        return rfd::FileDialog::new().set_title(title).set_directory(start).pick_folder();
    }
    #[cfg(not(windows))]
    {
        let _ = (start, title);
        None
    }
}

/// When started from a terminal, let println! reach it (the program itself has no console window).
pub fn attach_parent_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Press (or release) a key as if typed (the self-test presses the hotkey this way).
pub fn send_key(vk: u32, up: bool) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};
        let input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk as u16), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
    #[cfg(not(windows))]
    let _ = (vk, up);
}
