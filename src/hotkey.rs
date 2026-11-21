//! Global hotkey: RegisterHotKey in a thread with its own message loop.
//!
//! Hotkeys are stored as text like "Alt+Z" or "Ctrl+Shift+F9"; the key is a virtual-key code,
//! so it works the same with any keyboard layout (Alt+Я is Alt+Z).
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// keys that may be used without Ctrl / Alt / Shift / Win
pub fn solo_ok(name: &str) -> bool {
    matches!(name, "Pause" | "ScrollLock" | "PrintScreen" | "Insert") || (name.starts_with('F') && name[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)))
}

/// virtual-key code of a key name ("Z", "F9", "PageUp"…)
pub fn vk_of(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let u = c.to_ascii_uppercase();
        if u.is_ascii_uppercase() || u.is_ascii_digit() {
            return Some(u as u32);
        }
        return Some(match c {
            ';' => 0xBA,
            '=' => 0xBB,
            ',' => 0xBC,
            '-' => 0xBD,
            '.' => 0xBE,
            '/' => 0xBF,
            '`' => 0xC0,
            '[' => 0xDB,
            '\\' => 0xDC,
            ']' => 0xDD,
            '\'' => 0xDE,
            _ => return None,
        });
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x6F + n);
        }
    }
    if let Some(n) = name.strip_prefix("Num").and_then(|n| n.parse::<u32>().ok()) {
        if n <= 9 {
            return Some(0x60 + n);
        }
    }
    Some(match name {
        "Space" => 0x20,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "PrintScreen" => 0x2C,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Pause" => 0x13,
        "ScrollLock" => 0x91,
        "Num*" => 0x6A,
        "NumPlus" => 0x6B,
        "NumMinus" => 0x6D,
        "Num." => 0x6E,
        "Num/" => 0x6F,
        _ => return None,
    })
}

/// name of a virtual-key code, the other way round
pub fn name_of(vk: u32) -> Option<String> {
    if (0x41..=0x5A).contains(&vk) || (0x30..=0x39).contains(&vk) {
        return Some((vk as u8 as char).to_string());
    }
    if (0x70..=0x87).contains(&vk) {
        return Some(format!("F{}", vk - 0x6F));
    }
    if (0x60..=0x69).contains(&vk) {
        return Some(format!("Num{}", vk - 0x60));
    }
    for n in ["Space", "PageUp", "PageDown", "End", "Home", "Left", "Up", "Right", "Down", "PrintScreen", "Insert", "Delete", "Pause", "ScrollLock", "Num*", "NumPlus", "NumMinus", "Num.", "Num/", ";", "=", ",", "-", ".", "/", "`", "[", "\\", "]", "'"] {
        if vk_of(n) == Some(vk) {
            return Some(n.to_string());
        }
    }
    None
}

/// "Ctrl+Alt+Z" → (modifier bits, vk) or None
pub fn parse(text: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = text.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, mods) = parts.split_last()?;
    let mut bits = 0;
    for m in mods {
        bits |= match *m {
            "Ctrl" => MOD_CONTROL,
            "Alt" => MOD_ALT,
            "Shift" => MOD_SHIFT,
            "Win" => MOD_WIN,
            _ => return None,
        };
    }
    Some((bits, vk_of(key)?))
}

pub fn compose(ctrl: bool, alt: bool, shift: bool, win: bool, key: &str) -> String {
    let mut v = vec![];
    for (on, n) in [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift"), (win, "Win")] {
        if on {
            v.push(n.to_string());
        }
    }
    v.push(key.to_string());
    v.join("+")
}

/// Calls on_press() (from its own thread) whenever the hotkey is pressed anywhere.
pub struct Hotkey {
    text: Arc<Mutex<String>>,
    ok: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
    tid: Arc<AtomicU32>,
}

impl Hotkey {
    pub fn new(text: &str, on_press: impl Fn() + Send + Sync + 'static) -> Hotkey {
        let h = Hotkey { text: Arc::new(Mutex::new(text.to_string())), ok: Arc::new(AtomicBool::new(false)), error: Arc::new(Mutex::new("starting".into())), tid: Arc::new(AtomicU32::new(0)) };
        platform::spawn(&h, Arc::new(on_press));
        h
    }

    pub fn ok(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }
    pub fn error(&self) -> String {
        self.error.lock().unwrap().clone()
    }

    pub fn set(&self, text: &str) {
        *self.text.lock().unwrap() = text.to_string();
        platform::poke(&self.tid, 1);
    }

    pub fn stop(&self) {
        platform::poke(&self.tid, 2);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY};

    const MOD_NOREPEAT: u32 = 0x4000;

    pub fn poke(tid: &AtomicU32, what: u32) {
        for _ in 0..100 {
            let t = tid.load(Ordering::Relaxed);
            if t != 0 {
                unsafe {
                    let _ = PostThreadMessageW(t, WM_APP + what, WPARAM(0), LPARAM(0));
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    pub fn spawn(h: &Hotkey, on_press: Arc<dyn Fn() + Send + Sync>) {
        let (text, ok, error, tid) = (h.text.clone(), h.ok.clone(), h.error.clone(), h.tid.clone());
        std::thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || unsafe {
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0x400, 0x400, PM_NOREMOVE); // create the message queue
                tid.store(GetCurrentThreadId(), Ordering::Relaxed);
                let mut registered = false;
                let register = |registered: &mut bool| {
                    if *registered {
                        let _ = UnregisterHotKey(None, 1);
                        *registered = false;
                    }
                    let t = text.lock().unwrap().clone();
                    let Some((mods, vk)) = parse(&t) else {
                        ok.store(false, Ordering::Relaxed);
                        *error.lock().unwrap() = if t.is_empty() { "no key".into() } else { "not a valid key".into() };
                        return;
                    };
                    match RegisterHotKey(None, 1, HOT_KEY_MODIFIERS(mods | MOD_NOREPEAT), vk) {
                        Ok(()) => {
                            *registered = true;
                            ok.store(true, Ordering::Relaxed);
                            error.lock().unwrap().clear();
                        }
                        Err(e) => {
                            ok.store(false, Ordering::Relaxed);
                            let code = e.code().0 as u32 & 0xFFFF;
                            *error.lock().unwrap() = if code == 1409 { "another program uses this key".into() } else { format!("error {code}") };
                        }
                    }
                };
                register(&mut registered);
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_HOTKEY {
                        on_press();
                    } else if msg.message == WM_APP + 1 {
                        register(&mut registered);
                    } else if msg.message == WM_APP + 2 {
                        break;
                    }
                }
                if registered {
                    let _ = UnregisterHotKey(None, 1);
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn poke(_tid: &AtomicU32, _what: u32) {}

    pub fn spawn(h: &Hotkey, _on_press: Arc<dyn Fn() + Send + Sync>) {
        *h.error.lock().unwrap() = "only works on Windows".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse("Alt+Z"), Some((MOD_ALT, 0x5A)));
        assert_eq!(parse("Ctrl+Shift+F9"), Some((MOD_CONTROL | MOD_SHIFT, 0x78)));
        assert_eq!(parse("Pause"), Some((0, 0x13)));
        assert_eq!(parse("Foo+Z"), None);
        assert_eq!(parse(""), None);
        assert_eq!(compose(true, true, false, false, "Z"), "Ctrl+Alt+Z");
        assert_eq!(name_of(0x78), Some("F9".into()));
        assert!(solo_ok("F9") && !solo_ok("Z"));
    }
}
