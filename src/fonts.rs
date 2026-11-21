//! The fonts the program draws with itself: Segoe UI from Windows, the bundled Inter where it isn't there.
use crate::config as C;
use std::path::PathBuf;

fn windows_fonts() -> PathBuf {
    std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts")
}

fn first_file(names: &[&str]) -> Option<Vec<u8>> {
    let dirs = [windows_fonts(), PathBuf::from("/usr/share/fonts/truetype/noto"), PathBuf::from("/usr/share/fonts/truetype/dejavu")];
    for d in dirs {
        for n in names {
            if let Ok(b) = std::fs::read(d.join(n)) {
                return Some(b);
            }
        }
    }
    None
}

/// Segoe UI (regular), or the bundled font
pub fn regular() -> Vec<u8> {
    first_file(&["segoeui.ttf", "NotoSans-Regular.ttf"]).unwrap_or_else(|| C::INTER.to_vec())
}

/// Segoe UI Semibold, or the regular one
pub fn semibold() -> Vec<u8> {
    first_file(&["seguisb.ttf", "NotoSans-SemiBold.ttf", "NotoSans-Bold.ttf"]).unwrap_or_else(regular)
}

