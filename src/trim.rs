//! Clips tab: cut a piece out of a saved clip (frame-exact, encoded again) and rename clips.
use crate::config::Config;
use crate::system::{self as sys, args, BELOW_NORMAL_PRIORITY_CLASS};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BAD_CHARS: &str = "<>:\"/\\|?*";

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// names the app gives clips itself: "<prefix> 2026-11-09 14-03-27", maybe " (trimmed)" / " (trimmed 2)"
pub fn is_auto_name(path: &Path) -> bool {
    let name = stem(path);
    let mut n = name.as_str();
    if let Some(i) = n.rfind(" (trimmed") {
        let tail = &n[i + 9..];
        let ok = tail == ")" || (tail.starts_with(' ') && tail.ends_with(')') && tail.len() > 2 && tail[1..tail.len() - 1].chars().all(|c| c.is_ascii_digit()));
        if ok {
            n = &n[..i];
        }
    }
    let b = n.as_bytes();
    if b.len() < 19 {
        return false;
    }
    b[b.len() - 19..].iter().enumerate().all(|(i, c)| match i {
        4 | 7 | 13 | 16 => *c == b'-',
        10 => *c == b' ',
        _ => c.is_ascii_digit(),
    })
}

/// Give the clip a new name (the extension stays). Returns the new path; Err says what's wrong.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    let mut name = name.trim().to_string();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    if !ext.is_empty() && name.to_lowercase().ends_with(&ext.to_lowercase()) {
        name.truncate(name.len() - ext.len());
        name = name.trim_end().to_string();
    }
    let name = name.trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() {
        return Err("The name is empty".into());
    }
    if name.chars().any(|c| BAD_CHARS.contains(c) || (c as u32) < 32) {
        return Err("A name can't contain < > : \" / \\ | ? *".into());
    }
    let new = path.with_file_name(format!("{name}{ext}"));
    let same = if cfg!(windows) { new.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase() } else { new == path };
    if same {
        if new != path {
            // only the letter case changed
            let _ = fs::rename(path, &new);
        }
        return Ok(new);
    }
    if new.exists() {
        return Err("A clip with this name already exists".into());
    }
    fs::rename(path, &new).map_err(|e| {
        let m = e.to_string();
        m.split(" (os error").next().unwrap_or("Couldn't rename").to_string()
    })?;
    Ok(new)
}

pub fn output_path(src: &Path) -> PathBuf {
    let base = stem(src);
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut path = src.with_file_name(format!("{base} (trimmed){ext}"));
    let mut n = 2;
    while path.exists() {
        path = src.with_file_name(format!("{base} (trimmed {n}){ext}"));
        n += 1;
    }
    path
}

/// One JPEG frame at t seconds (for the trim preview), or None.
pub fn frame(path: &Path, t: f64, width: u32) -> Option<Vec<u8>> {
    let a = args(&["-hide_banner", "-v", "error", "-ss", &format!("{:.3}", t.max(0.0)), "-i", &path.to_string_lossy(), "-frames:v", "1", "-vf", &format!("scale={width}:-2"), "-q:v", "4", "-f", "image2pipe", "-c:v", "mjpeg", "-"]);
    let o = sys::run_ffmpeg(&a, Duration::from_secs(15), 0)?;
    (o.ok() && !o.stdout.is_empty()).then_some(o.stdout)
}

/// Save start…end of src as "<name> (trimmed)". With the recorder's encoder, x264 if it can't.
/// Returns the new path; Err when nothing worked.
pub fn trim(cfg: &Config, enc: &str, src: &Path, start: f64, end: f64, delete_original: bool) -> Result<PathBuf, String> {
    let out = output_path(src);
    let tmp = PathBuf::from(format!("{}.part", out.display()));
    let mut err = "ffmpeg failed".to_string();
    let mut encs = vec![if enc.is_empty() { "x264".to_string() } else { enc.to_string() }];
    if encs[0] != "x264" {
        encs.push("x264".into());
    }
    let mp4 = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    for e in encs {
        let pix = if e == "qsv" || e == "amf" { "nv12" } else { "yuv420p" };
        // -ss before -i with encoding is exact: ffmpeg decodes from the keyframe and drops up to `start`
        let mut a = args(&["-hide_banner", "-v", "error", "-y", "-ss", &format!("{start:.3}"), "-i", &src.to_string_lossy(), "-t", &format!("{:.3}", end - start), "-map", "0:v:0", "-map", "0:a?", "-vf", &format!("format={pix}"), "-c:a", "aac", "-b:a", "192k"]);
        a.extend(sys::encoder_args(&e, &cfg.s("rec_quality"), cfg.i("fps")));
        a.extend(if mp4 { args(&["-movflags", "+faststart", "-f", "mp4"]) } else { args(&["-f", "matroska"]) });
        a.push(tmp.to_string_lossy().into_owned());
        if let Some(r) = sys::run_ffmpeg(&a, Duration::from_secs(1800), BELOW_NORMAL_PRIORITY_CLASS) {
            if r.ok() && fs::metadata(&tmp).map(|m| m.len() > 1000).unwrap_or(false) {
                let _ = fs::rename(&tmp, &out);
                if delete_original {
                    let _ = fs::remove_file(src);
                }
                return Ok(out);
            }
            err = r.last_err(&err);
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_names() {
        for (n, ok) in [
            ("DrzReplay 2026-11-09 14-03-27.mp4", true),
            ("DrzReplay 2026-11-09 14-03-27 (trimmed 2).mp4", true),
            ("My boss fight.mp4", false),
        ] {
            assert_eq!(is_auto_name(Path::new(n)), ok, "{n}");
        }
    }
    #[test]
    fn bad_names() {
        assert!(rename(Path::new("a.mp4"), "x?y").is_err());
        assert!(rename(Path::new("a.mp4"), "  ").is_err());
    }
}
