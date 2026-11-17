//! How sizes, times and dates are shown.
use chrono::{DateTime, Local};
use std::time::SystemTime;

pub fn human_size(mut n: f64) -> String {
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || unit == "GB" {
            return if unit == "B" || unit == "KB" { format!("{n:.0} {unit}") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    String::new()
}

pub fn human_time(sec: f64) -> String {
    let s = sec.round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "1:05.3": minutes and seconds with a tenth
pub fn human_time_exact(sec: f64) -> String {
    format!("{}:{:04.1}", sec as i64 / 60, sec % 60.0)
}

pub fn human_date(ts: SystemTime) -> String {
    let d: DateTime<Local> = ts.into();
    let today = Local::now().date_naive();
    if d.date_naive() == today {
        format!("Today, {}", d.format("%H:%M"))
    } else if d.date_naive() == today.pred_opt().unwrap_or(today) {
        format!("Yesterday, {}", d.format("%H:%M"))
    } else {
        format!("{} {}, {}", d.format("%b"), d.format("%-d"), d.format("%H:%M"))
    }
}

/// "~\Videos\DrzReplay" for paths in the user's folder
pub fn short_path(path: &std::path::Path) -> String {
    let home = crate::config::home();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats() {
        assert_eq!(human_size(552.0 * 1024.0), "552 KB");
        assert_eq!(human_size(31.4 * 1024.0 * 1024.0), "31.4 MB");
        assert_eq!(human_time(20.0), "0:20");
        assert_eq!(human_time_exact(62.5), "1:02.5");
    }
}
