//! The three tabs of the window.
use super::dialogs::Dialog;
use super::theme::Palette;
use super::widgets::{self as w, Kind};
use super::{App, UiEv, MAX_CLIPS_SHOWN};
use crate::config::{self as C};
use crate::fmt::{human_date, human_size, human_time, short_path};
use crate::{hotkey, system as sys, trim, winutil};
use eframe::egui::{self, vec2, Align, Layout, Rect, Sense, Ui};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};

const FPS: &[(i64, &str)] = &[(30, "30"), (60, "60"), (120, "120")];
const QUALITY: &[(&str, &str)] = &[("low", "Low"), ("balanced", "Medium"), ("high", "High"), ("ultra", "Maximum")];
const FORMAT: &[(&str, &str)] = &[("mp4", "MP4"), ("mkv", "MKV")];
/// the gap between blocks (the column's spacing is 26 px, `ui` adds 6 of its own)
const GAP: f32 = 22.0;

fn sget<'a>(st: &'a Value, k: &str) -> &'a str {
    st.get(k).and_then(|v| v.as_str()).unwrap_or("")
}
fn fget(st: &Value, k: &str) -> f64 {
    st.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// A page: scrolls, content in a centred column (at most 620 px wide, 24 px from the edges).
pub fn scroll(ui: &mut Ui, reserve_bar: bool, content: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // the Qt window keeps 10 px for its scroll bar when there is one; the column then takes
        // 100/102 of what is left between the 24 px margins (stretch 1 : 100 : 1) up to 620 px
        let full = ui.available_width();
        let room = full - if reserve_bar { 10.0 } else { 0.0 };
        let col = ((room - 48.0) * 100.0 / 102.0).clamp(420.0, 620.0).min(full);
        let left = ((room - col) / 2.0).max(0.0);
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.add_space(left);
            ui.allocate_ui_with_layout(vec2(col, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(col);
                content(ui);
            });
        });
        ui.add_space(28.0);
    });
}

fn sep_if(ui: &mut Ui, p: &Palette, first: &mut bool) {
    if !*first {
        w::separator(ui, p);
    }
    *first = false;
}

impl App {
    // ------------------------------------------------------------------ row builders
    fn switch_row(&mut self, ui: &mut Ui, p: &Palette, first: &mut bool, title: &str, key: &str, sub: &str) {
        sep_if(ui, p, first);
        let mut on = self.cfg.b(key);
        w::row(ui, p, title, sub, false, true, |_| {}, |ui| {
            if w::switch(ui, p, &mut on, true).changed() {
                self.set(key, json!(on));
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn combo_row(&mut self, ui: &mut Ui, p: &Palette, first: &mut bool, title: &str, key: &str, options: Vec<(Value, String)>, sub: &str, enabled: bool) {
        sep_if(ui, p, first);
        let (mut values, mut labels): (Vec<Value>, Vec<String>) = options.into_iter().unzip();
        let cur = self.cfg.get(key).clone();
        if !values.contains(&cur) {
            labels.push(cur.as_str().map(String::from).unwrap_or_else(|| cur.to_string()));
            values.push(cur.clone());
        }
        let mut sel = values.iter().position(|v| *v == cur).unwrap_or(0);
        w::row(ui, p, title, sub, false, enabled, |_| {}, |ui| {
            if w::combo(ui, p, key, &labels, &mut sel, enabled) {
                self.set(key, values[sel].clone());
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn spin_row(&mut self, ui: &mut Ui, p: &Palette, first: &mut bool, title: &str, sub: &str, key: &str, lo: f64, hi: f64, step: f64) {
        sep_if(ui, p, first);
        let mut v = self.cfg.f(key);
        w::row(ui, p, title, sub, false, true, |_| {}, |ui| {
            if w::spin(ui, p, key, &mut v, lo, hi, step, 0, "", 126.0, true) {
                self.set(key, json!(v as i64));
            }
        });
    }

    fn slider_row(&mut self, ui: &mut Ui, p: &Palette, first: &mut bool, key: &str, enabled: bool) {
        sep_if(ui, p, first);
        let mut v = self.cfg.f(key);
        w::row(ui, p, "Volume", "", false, enabled, |_| {}, |ui| {
            // right to left: the slider, then the number
            if w::slider(ui, p, &mut v, 0.0, 200.0, 5.0, 200.0, enabled).changed() {
                self.set(key, json!(v as i64));
            }
            ui.allocate_ui_with_layout(vec2(44.0, 20.0), Layout::right_to_left(Align::Center), |ui| {
                w::dim(ui, p, &format!("{}%", v as i64));
            });
        });
    }

    // ------------------------------------------------------------------ Record tab
    pub fn page_record(&mut self, ui: &mut Ui, p: &Palette, st: &Value) {
        let state = sget(st, "state").to_string();
        let replays_off = st.get("replays_off").and_then(|v| v.as_bool()).unwrap_or(false);
        let recording = state == "recording" && !replays_off;
        let buffered = if recording { fget(st, "buffered") } else { 0.0 };
        let secs = if fget(st, "seconds") > 0.0 { fget(st, "seconds") } else { self.cfg.f("seconds") };

        let subtitle = if recording {
            let size = st.get("size").and_then(|s| s.as_array());
            let (gw, gh) = size.map(|a| (a.first().and_then(|v| v.as_i64()).unwrap_or(0), a.get(1).and_then(|v| v.as_i64()).unwrap_or(0))).unwrap_or((0, 0));
            let enc = sys::encoder_label(sget(st, "encoder"));
            let sound = (self.cfg.b("system_audio") || self.cfg.b("mic")) && st.get("audio_off").and_then(|v| v.as_bool()).unwrap_or(false);
            format!("{gw}×{gh} · {} fps · {enc}{}", fget(st, "fps") as i64, if sound { " · no sound" } else { "" })
        } else {
            match state.as_str() {
                "probing" => "Checking the graphics card…".into(),
                "starting" => "Starting…".into(),
                "paused" => "Off · turn on to save replays".into(),
                "error" => if sget(st, "error").is_empty() { "Error".into() } else { sget(st, "error").to_string() },
                _ if replays_off => "Off · turn on to save replays".into(),
                _ => "Off".into(),
            }
        };
        let switch_on = matches!(state.as_str(), "recording" | "starting" | "probing") && !replays_off;

        let mut first = true;
        w::group(ui, p, "", "", |_| {}, |ui| {
            sep_if(ui, p, &mut first);
            let mut on = switch_on;
            w::row(
                ui,
                p,
                "Replays",
                &subtitle,
                false,
                true,
                |ui| {
                    // the recording dot
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    let c = if recording { p.rec } else { p.dim };
                    ui.painter().circle_stroke(r.center(), 7.0, egui::Stroke::new(1.0, c));
                    ui.painter().circle_filled(r.center(), 4.0, c);
                },
                |ui| {
                    if w::switch(ui, p, &mut on, true).changed() {
                        self.command(if on { "resume" } else { "pause" });
                    }
                },
            );
            sep_if(ui, p, &mut first);
            let buf_sub = if recording { format!("{buffered:.0} of {secs:.0} s · {:.0} MB in memory", fget(st, "buffer_mb")) } else { "Empty".into() };
            w::row(ui, p, "Buffer", &buf_sub, false, true, |_| {}, |ui| {
                w::progress(ui, p, if secs > 0.0 { (buffered / secs) as f32 } else { 0.0 }, 170.0);
            });
        });
        ui.add_space(27.0);

        // the two big buttons
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if w::button(ui, p, "Save replay", Kind::Big, recording && buffered >= 1.0).clicked() {
                self.command("save");
            }
            w::subtitle(ui, p, &format!("or press {}", self.cfg.s("hotkey")));
            ui.add_space(14.0);
            let rec = st.get("file_recording").and_then(|v| v.as_f64());
            let label = match rec {
                Some(t0) => {
                    let s = (C::now() - t0) as i64;
                    format!("■  Stop recording · {}:{:02}", s / 60, s % 60)
                }
                None => "●  Start recording".into(),
            };
            let can = matches!(state.as_str(), "recording" | "paused" | "off") || rec.is_some();
            if w::button(ui, p, &label, Kind::Rec(rec.is_some()), can).clicked() {
                self.command("record");
            }
            let rk = self.cfg.s("record_hotkey");
            if !rk.is_empty() {
                w::subtitle(ui, p, &format!("or press {rk}"));
            }
        });
        ui.add_space(31.0);

        // Video
        let monitors: Vec<(i64, i64, bool)> = st.get("monitors").and_then(|m| m.as_array()).map(|a| a.iter().map(|m| (m["w"].as_i64().unwrap_or(0), m["h"].as_i64().unwrap_or(0), m["main"].as_bool().unwrap_or(false))).collect()).unwrap_or_default();
        let encoders: Vec<String> = st.get("encoders").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect()).unwrap_or_default();
        let mut first = true;
        w::group(ui, p, "Video", "", |_| {}, |ui| {
            self.spin_row(ui, p, &mut first, "Length", "Seconds kept in memory", "seconds", 10.0, 600.0, 5.0);
            if monitors.len() > 1 {
                let mut o: Vec<(Value, String)> = vec![(json!(-1), "Main screen".into())];
                o.extend(monitors.iter().enumerate().map(|(i, (mw, mh, main))| (json!(i), format!("Screen {} ({mw}×{mh}){}", i + 1, if *main { " · main" } else { "" }))));
                self.combo_row(ui, p, &mut first, "Screen", "monitor", o, "", true);
            }
            self.combo_row(ui, p, &mut first, "Frame rate", "fps", FPS.iter().map(|(v, l)| (json!(v), l.to_string())).collect(), "", true);
            self.combo_row(ui, p, &mut first, "Quality", "rec_quality", QUALITY.iter().map(|(v, l)| (json!(v), l.to_string())).collect(), "", true);
            let mut e: Vec<(Value, String)> = vec![(json!("auto"), "Auto".into())];
            e.extend(encoders.iter().map(|x| (json!(x), sys::encoder_label(x))));
            self.combo_row(ui, p, &mut first, "Encoder", "encoder", e, "", true);
            self.switch_row(ui, p, &mut first, "Show cursor", "draw_mouse", "");
        });
        ui.add_space(GAP);

        // Sound
        let devices = st.get("devices").filter(|d| !d.is_null());
        let names = |k: &str| -> Vec<String> { devices.and_then(|d| d.get(k)).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
        let (outs, ins) = (names("outs"), names("ins"));
        let mut first = true;
        w::group(ui, p, "Sound", "", |_| {}, |ui| {
            self.switch_row(ui, p, &mut first, "System sound", "system_audio", "What you hear");
            let sys_on = self.cfg.b("system_audio");
            let mut o: Vec<(Value, String)> = vec![(json!(""), "Default".into())];
            o.extend(outs.iter().map(|n| (json!(n), n.clone())));
            self.combo_row(ui, p, &mut first, "Output", "system_device", o, "", sys_on);
            self.slider_row(ui, p, &mut first, "system_volume", sys_on);
            self.switch_row(ui, p, &mut first, "Microphone", "mic", "");
            let mic_on = self.cfg.b("mic");
            let mut i: Vec<(Value, String)> = vec![(json!(""), "Default".into())];
            i.extend(ins.iter().map(|n| (json!(n), n.clone())));
            self.combo_row(ui, p, &mut first, "Input", "mic_device", i, "", mic_on);
            self.slider_row(ui, p, &mut first, "mic_volume", mic_on);
        });
    }

    // ------------------------------------------------------------------ Clips tab
    pub fn page_clips(&mut self, ui: &mut Ui, p: &Palette) {
        if self.clips.is_empty() && self.clips_sig.is_some() {
            // nothing saved yet
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.28).max(40.0));
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                w::draw_icon(ui, r, "clips", p.dim, 1.75);
                ui.add_space(10.0);
                w::big(ui, p, "No clips yet");
                w::dim(ui, p, &format!("Press {} to save the last {} seconds", self.cfg.s("hotkey"), self.cfg.i("seconds")));
                ui.add_space(14.0);
                if w::button(ui, p, "Open folder", Kind::Normal, true).clicked() {
                    self.open_folder();
                }
            });
            return;
        }
        let clips = self.clips.clone();
        scroll(ui, clips.len() > 8, |ui| {
            let total: u64 = clips.iter().map(|c| c.size).sum();
            let desc = format!("{} clip{} · {}", clips.len(), if clips.len() == 1 { "" } else { "s" }, human_size(total as f64));
            let mut refresh = false;
            let mut folder = false;
            w::group(
                ui,
                p,
                "Saved clips",
                &desc,
                |ui| {
                    // right to left: the folder first
                    folder = w::icon_button(ui, p, "folder", "Open folder").clicked();
                    refresh = w::icon_button(ui, p, "refresh", "Refresh").clicked();
                },
                |ui| {
                    let mut first = true;
                    for c in clips.iter().take(MAX_CLIPS_SHOWN) {
                        sep_if(ui, p, &mut first);
                        self.clip_row(ui, p, c);
                    }
                },
            );
            if refresh {
                self.load_clips(true);
            }
            if folder {
                self.open_folder();
            }
            let hidden = clips.len().saturating_sub(MAX_CLIPS_SHOWN);
            if hidden > 0 {
                ui.add_space(6.0);
                ui.vertical_centered(|ui| w::subtitle(ui, p, &format!("{hidden} older clip{} in the folder", if hidden == 1 { "" } else { "s" })));
            }
        });
    }

    fn clip_row(&mut self, ui: &mut Ui, p: &Palette, c: &super::Clip) {
        let path = c.path.clone();
        // clips the user renamed show their name, the others the date they were saved
        let named = !trim::is_auto_name(&path);
        let date = if named { Some(human_date(c.mtime)) } else { None };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = if named { stem } else { human_date(c.mtime) };
        if self.thumb_asked.insert(path.clone()) {
            let _ = self.thumb_tx.send(path.clone());
        }
        let (tex, dur) = self.thumbs.get(&path).map(|t| (t.tex.clone(), t.dur)).unwrap_or((None, 0.0));
        let mut parts: Vec<String> = vec![];
        if let Some(d) = &date {
            parts.push(d.clone());
        }
        if dur > 0.0 {
            parts.push(human_time(dur));
        }
        parts.push(human_size(c.size as f64));
        let sub = parts.join(" · ");
        let mut act: Option<&'static str> = None;
        let resp = w::row_h(
            ui,
            p,
            74.0,
            &title,
            &sub,
            true,
            true,
            |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(96.0, 54.0), Sense::hover());
                ui.painter().rect_filled(r, w::radius(4.0), p.thumb);
                if let Some(t) = &tex {
                    ui.put(r, egui::Image::new(t).corner_radius(4).fit_to_exact_size(vec2(96.0, 54.0)));
                }
            },
            |ui| {
                for (icon, tip, a) in [("trash", "Move to Recycle Bin", "trash"), ("folder", "Show in folder", "folder"), ("copy", "Copy (paste into Discord, Telegram…)", "copy"), ("pencil", "Rename", "rename"), ("scissors", "Trim", "trim")] {
                    if w::icon_button(ui, p, icon, tip).clicked() {
                        act = Some(a);
                    }
                }
            },
        );
        // dragging the row out of the window: drop the clip into Discord, Telegram, Explorer
        if resp.drag_started() && act.is_none() {
            winutil::drag_file(&path);
            ui.ctx().request_repaint();
            return;
        }
        let resp = resp.on_hover_text(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        match act {
            Some("trash") => self.trash_clip(&path),
            Some("folder") => sys::show_in_folder(&path),
            Some("copy") => {
                if winutil::copy_file_to_clipboard(&path) {
                    self.show_toast("Copied — paste it into a chat");
                }
            }
            Some("rename") => self.dialog = Some(Dialog::rename(&path)),
            Some("trim") => {
                let d = if dur > 0.0 { dur } else { sys::media_duration(&path) };
                if d < 1.0 {
                    self.show_toast("Can't read this clip");
                } else {
                    let st = super::dialogs::TrimState::new(&path, d, &self.tx, ui.ctx());
                    self.dialog = Some(Dialog::Trim(Box::new(st)));
                }
            }
            _ => {
                if resp.clicked() {
                    sys::open_path(&path.to_string_lossy());
                }
            }
        }
    }

    fn trash_clip(&mut self, path: &Path) {
        match winutil::trash(path) {
            Ok(()) => self.show_toast("Moved to Recycle Bin"),
            Err(_) => match std::fs::remove_file(path) {
                Ok(()) => self.show_toast("Deleted"),
                Err(e) => self.show_toast(&format!("Couldn't delete: {e}")),
            },
        }
        self.load_clips(true);
    }

    pub fn open_folder(&self) {
        let d = self.cfg.output_dir();
        let _ = std::fs::create_dir_all(&d);
        sys::open_path(&d.to_string_lossy());
    }

    // ------------------------------------------------------------------ Settings tab
    pub fn page_settings(&mut self, ui: &mut Ui, p: &Palette, st: &Value) {
        let mut first = true;
        w::group(ui, p, "Saving", "", |_| {}, |ui| {
            sep_if(ui, p, &mut first);
            let dir = short_path(&self.cfg.output_dir());
            w::row(ui, p, "Folder", &dir, false, true, |_| {}, |ui| {
                // right to left: Change, then the folder icon
                if w::button(ui, p, "Change", Kind::Normal, true).clicked() {
                    let (tx, start) = (self.tx.clone(), self.cfg.output_dir());
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        if let Some(d) = winutil::pick_folder(&start, "Folder for clips") {
                            let _ = tx.send(UiEv::Folder(d));
                            ctx.request_repaint();
                        }
                    });
                }
                if w::icon_button(ui, p, "folder", "Open").clicked() {
                    self.open_folder();
                }
            });
            sep_if(ui, p, &mut first);
            let mut name = self.name_buf.clone().unwrap_or_else(|| self.cfg.s("file_prefix"));
            w::row(ui, p, "File name", "Followed by the date and time", false, true, |_| {}, |ui| {
                let r = w::text_field(ui, p, "file_prefix", &mut name, 180.0);
                if r.lost_focus() {
                    let bad = "\\/:*?\"<>|";
                    let clean: String = name.trim().chars().filter(|c| !bad.contains(*c)).collect();
                    let clean = if clean.is_empty() { "DrzReplay".to_string() } else { clean };
                    self.set("file_prefix", json!(clean.clone()));
                    name = clean;
                    self.name_buf = None;
                } else if r.has_focus() || r.changed() {
                    self.name_buf = Some(name.clone());
                }
            });
            self.combo_row(ui, p, &mut first, "Format", "container", FORMAT.iter().map(|(v, l)| (json!(v), l.to_string())).collect(), "", true);
            // intro, with a preview button
            sep_if(ui, p, &mut first);
            let mut on = self.cfg.b("intro");
            w::row(ui, p, "Intro", "Replay number and date in the corner for the first seconds", false, true, |_| {}, |ui| {
                // right to left: the preview button, then the switch
                if w::icon_button(ui, p, "play", "Preview").clicked() {
                    self.flush();
                    self.show_toast("Rendering the preview…");
                    preview_intro(self.cfg.clone());
                }
                if w::switch(ui, p, &mut on, true).changed() {
                    self.set("intro", json!(on));
                }
            });
        });
        ui.add_space(GAP);

        let mut first = true;
        w::group(ui, p, "Cleanup", "Oldest clips are deleted when a limit is reached", |_| {}, |ui| {
            self.spin_row(ui, p, &mut first, "Keep clips", "0 = keep all", "keep_max_clips", 0.0, 9999.0, 1.0);
            self.spin_row(ui, p, &mut first, "Max space, GB", "0 = no limit", "keep_max_gb", 0.0, 2000.0, 1.0);
        });
        ui.add_space(GAP);

        let mut first = true;
        w::group(ui, p, "Controls", "", |_| {}, |ui| {
            for (title, key, sub) in [("Replay hotkey", "hotkey", ""), ("Recording hotkey", "record_hotkey", "Starts and stops recording a full video")] {
                sep_if(ui, p, &mut first);
                let hk = st.get(if key == "hotkey" { "hotkey" } else { "rec_hotkey" });
                let ok = hk.and_then(|h| h.get("ok")).and_then(|v| v.as_bool()).unwrap_or(true);
                let err = hk.and_then(|h| h.get("error")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let sub = if ok || err == "starting" || err.is_empty() { sub.to_string() } else { format!("Not working: {err}") };
                let cur = self.cfg.s(key);
                w::row(ui, p, title, &sub, false, true, |_| {}, |ui| {
                    if w::button(ui, p, "Change", Kind::Normal, true).clicked() {
                        self.dialog = Some(Dialog::hotkey(key));
                    }
                    w::dim(ui, p, if cur.is_empty() { "None" } else { &cur });
                });
            }
            self.switch_row(ui, p, &mut first, "Clear the buffer after a replay", "replay_since_last", "The next replay only has what happened after the last one");
            self.switch_row(ui, p, &mut first, "Click sound", "click_sound", "When a replay is saved or a recording starts and stops");
            self.switch_row(ui, p, &mut first, "Notifications", "notifications", "A badge in the corner of the screen, over games too");
        });
        ui.add_space(GAP);

        let mut first = true;
        w::group(ui, p, "Startup", "", |_| {}, |ui| {
            sep_if(ui, p, &mut first);
            let mut on = winutil::autostart_enabled();
            w::row(ui, p, "Start with Windows", "Recording starts in the tray when you log in", false, cfg!(windows), |_| {}, |ui| {
                if w::switch(ui, p, &mut on, cfg!(windows)).changed() {
                    winutil::set_autostart(on);
                }
            });
        });
        ui.add_space(GAP);

        let mut first = true;
        w::group(ui, p, "Updates", "", |_| {}, |ui| {
            sep_if(ui, p, &mut first);
            let ready = st.get("update_ready").and_then(|v| v.as_str()).map(String::from);
            let busy = st.get("update_progress").and_then(|v| v.as_str()).map(String::from);
            let sub = busy.clone().or_else(|| ready.as_ref().map(|v| format!("Version {v} is available"))).or_else(|| self.update_note.clone()).unwrap_or_default();
            w::row(ui, p, &format!("{} {}", C::APP_NAME, C::VERSION), &sub, false, true, |_| {}, |ui| {
                let label = if ready.is_some() { "Update" } else { "Check" };
                if w::button(ui, p, label, Kind::Normal, busy.is_none() && !self.ver_busy).clicked() {
                    if ready.is_some() {
                        self.command("run_update");
                    } else {
                        self.check_updates(true);
                    }
                }
            });
        });
        ui.add_space(GAP);
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("privet mmpd").font(w::font(11.0)).color(p.dim));
        });
    }

    // ------------------------------------------------------------------ the "…" menu
    pub fn menu_popup(&mut self, ui: &mut Ui, p: &Palette, clicked: bool) {
        let id = egui::Id::new("more-menu");
        if clicked {
            w::popup_set(ui, id, !w::popup_is_open(ui, id));
        }
        if !w::popup_is_open(ui, id) {
            return;
        }
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let mut chosen: Option<usize> = None;
        let inner = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(egui::pos2(screen.right() - 12.0 - 230.0, 46.0)).show(&ctx, |ui| {
            egui::Frame::new().fill(p.card).stroke(egui::Stroke::new(1.0, p.control_border)).corner_radius(w::radius(8.0)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_width(230.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (i, label) in ["Recorder log", "Detect graphics card again", "", &format!("About {}", C::APP_NAME)].iter().enumerate() {
                    if label.is_empty() {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                        ui.painter().rect_filled(Rect::from_min_size(egui::pos2(r.min.x + 6.0, r.center().y), vec2(r.width() - 12.0, 1.0)), 0.0, p.control_border);
                        continue;
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, w::radius(4.0), p.hover);
                    }
                    ui.painter().text(egui::pos2(r.min.x + 12.0, r.center().y), egui::Align2::LEFT_CENTER, label, w::font(w::BODY), p.text);
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
        let outside = ctx.input(|i| i.pointer.any_click()) && !inner.response.contains_pointer() && !clicked;
        if chosen.is_some() || outside {
            w::popup_set(ui, id, false);
        }
        match chosen {
            Some(0) => {
                std::thread::spawn(|| {
                    if let Some(r) = crate::ipc::request("log", json!({})) {
                        if let Some(path) = r.get("path").and_then(|p| p.as_str()) {
                            sys::open_path(path);
                        }
                    }
                });
            }
            Some(1) => {
                self.command("redetect");
                self.show_toast("Checking the graphics card…");
            }
            Some(3) => self.dialog = Some(Dialog::About),
            _ => {}
        }
    }
}

/// Render the intro over a plain background and open it in the video player.
fn preview_intro(cfg: crate::config::Config) {
    std::thread::spawn(move || {
        let (clip, out) = (C::data_dir().join("intro-preview.mov"), C::data_dir().join("intro-preview.mp4"));
        let _ = std::fs::create_dir_all(C::data_dir());
        if crate::intro::render(&clip, crate::intro::next_number(&cfg), chrono::Local::now(), 1080, 30).is_err() {
            return;
        }
        let (x, y) = crate::intro::position(1080);
        let fc = format!("gradients=s=1920x1080:c0=0x3a4552:c1=0x1d2229:x0=0:y0=0:x1=1920:y1=1080:speed=0.01:r=30:d=4[bg];[0:v]setpts=PTS-STARTPTS+{}/TB[ov];[bg][ov]overlay={x}:{y}:eof_action=pass,format=yuv420p[v]", crate::intro::START);
        let a = sys::args(&["-v", "error", "-y", "-i", &clip.to_string_lossy(), "-filter_complex", &fc, "-map", "[v]", "-t", "4", "-c:v", "libx264", "-preset", "veryfast", &out.to_string_lossy()]);
        if sys::run_ffmpeg(&a, std::time::Duration::from_secs(60), 0).is_some_and(|o| o.ok()) {
            sys::open_path(&out.to_string_lossy());
        }
        let _ = std::fs::remove_file(&clip);
    });
}

// ---------------------------------------------------------------- thumbnails
/// Makes clip thumbnails and reads durations in one background thread, cached on disk.
pub fn spawn_thumb_worker(tx: Sender<UiEv>, ctx: egui::Context) -> Sender<PathBuf> {
    let (req_tx, req_rx) = channel::<PathBuf>();
    let _ = std::fs::create_dir_all(C::thumbs_dir());
    std::thread::Builder::new()
        .name("thumbs".into())
        .spawn(move || {
            for path in req_rx {
                let (jpg, dur) = make_thumb(&path);
                let _ = tx.send(UiEv::Thumb { path, jpg, dur });
                ctx.request_repaint();
            }
        })
        .ok();
    req_tx
}

fn fnv(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn make_thumb(path: &Path) -> (Option<PathBuf>, f64) {
    let Ok(m) = std::fs::metadata(path) else { return (None, 0.0) };
    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    let key = fnv(&format!("{}|{mtime}|{}", path.display(), m.len()));
    let (jpg, meta) = (C::thumbs_dir().join(format!("{key}.jpg")), C::thumbs_dir().join(format!("{key}.json")));
    let dur = match C::read_obj(&meta).get("duration").and_then(|d| d.as_f64()) {
        Some(d) => d,
        None => {
            let d = sys::media_duration(path);
            C::write_json(&meta, &json!({"duration": d}));
            d
        }
    };
    if !jpg.exists() {
        let a = sys::args(&["-v", "error", "-y", "-ss", &format!("{:.2}", (dur / 2.0).min(1.0)), "-i", &path.to_string_lossy(), "-frames:v", "1", "-q:v", "3", "-vf", "scale=192:108:force_original_aspect_ratio=increase,crop=192:108", &jpg.to_string_lossy()]);
        let _ = sys::run_ffmpeg(&a, std::time::Duration::from_secs(20), sys::BELOW_NORMAL_PRIORITY_CLASS);
    }
    (jpg.exists().then_some(jpg), dur)
}

#[allow(dead_code)]
fn _hk(_: &str) -> bool {
    hotkey::solo_ok("F9")
}
