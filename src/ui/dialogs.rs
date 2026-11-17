//! The dialogs: trim, rename, a new hotkey, about.
use super::theme::Palette;
use super::widgets::{self as w, Kind};
use super::{App, UiEv};
use crate::config as C;
use crate::fmt::human_time_exact;
use crate::{hotkey, trim, winutil};
use eframe::egui::{self, vec2, Align, Layout, Rect, Sense};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

pub enum Dialog {
    Trim(Box<TrimState>),
    Rename { path: PathBuf, buf: String, error: String, focus: bool },
    Hotkey { key: &'static str, hint: String, held: Vec<u32>, hold_sent: bool },
    About,
}

impl Dialog {
    pub fn rename(path: &Path) -> Dialog {
        Dialog::Rename { path: path.to_path_buf(), buf: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), error: String::new(), focus: true }
    }
    pub fn hotkey(key: &str) -> Dialog {
        Dialog::Hotkey { key: if key == "hotkey" { "hotkey" } else { "record_hotkey" }, hint: "For example Alt+Z or F9. Esc to cancel.".into(), held: vec![], hold_sent: false }
    }
}

/// Pick a start and an end on the clip's timeline, watching the frames.
pub struct TrimState {
    path: PathBuf,
    name: String,
    dur: f64,
    start: f64,
    end: f64,
    pos: f64, // timeline steps
    delete: bool,
    pub preview: Option<egui::TextureHandle>,
    req: Sender<f64>,
    last_req: i64,
}

const STEP: f64 = 10.0; // timeline steps per second

impl TrimState {
    pub fn new(path: &Path, dur: f64, tx: &Sender<UiEv>, ctx: &egui::Context) -> TrimState {
        let (req, rx): (Sender<f64>, Receiver<f64>) = channel();
        let (p, tx, ctx) = (path.to_path_buf(), tx.clone(), ctx.clone());
        // grabs preview frames with ffmpeg; when asked faster than it can, only the latest counts
        std::thread::Builder::new()
            .name("trim-frames".into())
            .spawn(move || {
                while let Ok(mut t) = rx.recv() {
                    while let Ok(n) = rx.try_recv() {
                        t = n;
                    }
                    if t.is_nan() {
                        return;
                    }
                    if let Some(jpg) = trim::frame(&p, t, 640) {
                        let _ = tx.send(UiEv::Frame { jpg });
                        ctx.request_repaint();
                    }
                }
            })
            .ok();
        let _ = req.send(0.0);
        TrimState { path: path.to_path_buf(), name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), dur, start: 0.0, end: (dur * 10.0).round() / 10.0, pos: 0.0, delete: false, preview: None, req, last_req: 0 }
    }

    fn ask(&mut self) {
        let t = (self.pos / STEP).min((self.dur - 0.05).max(0.0));
        let key = (t * 1000.0) as i64;
        if key != self.last_req {
            self.last_req = key;
            let _ = self.req.send(t);
        }
    }
}

impl Drop for TrimState {
    fn drop(&mut self) {
        let _ = self.req.send(f64::NAN);
    }
}

fn modal_frame(p: &Palette, mx: i8, top: i8, bottom: i8) -> egui::Frame {
    egui::Frame::new().fill(p.bg).stroke(egui::Stroke::new(1.0, p.control_border)).corner_radius(w::radius(8.0)).inner_margin(egui::Margin { left: mx, right: mx, top, bottom })
}

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(d) = self.dialog.take() else { return };
        let keep = match d {
            Dialog::Trim(mut t) => {
                let keep = self.trim_dialog(ctx, p, &mut t);
                keep.then(|| Dialog::Trim(t))
            }
            Dialog::Rename { path, mut buf, mut error, mut focus } => {
                let keep = self.rename_dialog(ctx, p, &path, &mut buf, &mut error, &mut focus);
                keep.then(|| Dialog::Rename { path, buf, error, focus })
            }
            Dialog::Hotkey { key, mut hint, mut held, mut hold_sent } => {
                let keep = self.hotkey_dialog(ctx, p, key, &mut hint, &mut held, &mut hold_sent);
                keep.then(|| Dialog::Hotkey { key, hint, held, hold_sent })
            }
            Dialog::About => self.about_dialog(ctx, p).then_some(Dialog::About),
        };
        // a dialog that opened another (e.g. the trim saved) may have set a new one
        if self.dialog.is_none() {
            self.dialog = keep;
        }
    }

    fn trim_dialog(&mut self, ctx: &egui::Context, p: &Palette, t: &mut TrimState) -> bool {
        let mut close = false;
        let mut save = false;
        let m = egui::Modal::new(egui::Id::new("trim")).frame(modal_frame(p, 24, 20, 18)).show(ctx, |ui| {
            ui.set_width(528.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 12.0);
            w::big(ui, p, &t.name);
            // the frame being picked
            let (r, _) = ui.allocate_exact_size(vec2(528.0, 297.0), Sense::hover());
            ui.painter().rect_filled(r, w::radius(4.0), p.thumb);
            if let Some(tex) = &t.preview {
                let s = tex.size_vec2();
                let k = (r.width() / s.x).min(r.height() / s.y);
                let fit = Rect::from_center_size(r.center(), s * k);
                ui.put(fit, egui::Image::new(tex).corner_radius(6).fit_to_exact_size(fit.size()));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if w::slider(ui, p, &mut t.pos, 0.0, t.dur * STEP, 1.0, 528.0 - 90.0 - 8.0, true).changed() {
                    t.ask();
                }
                ui.allocate_ui_with_layout(vec2(90.0, 20.0), Layout::right_to_left(Align::Center), |ui| {
                    w::subtitle(ui, p, &format!("{} / {}", human_time_exact(t.pos / STEP), human_time_exact(t.dur)));
                });
            });
            let mut first = true;
            w::group(ui, p, "", "", |_| {}, |ui| {
                for (title, is_start) in [("Start", true), ("End", false)] {
                    if !first {
                        w::separator(ui, p);
                    }
                    first = false;
                    let mut v = if is_start { t.start } else { t.end };
                    let mut here = false;
                    let mut changed = false;
                    w::row(ui, p, title, "", false, true, |_| {}, |ui| {
                        // right to left: the flag button, then the box
                        here = w::icon_button(ui, p, "flag", "Use the current position").clicked();
                        changed = w::spin(ui, p, if is_start { "trim_start" } else { "trim_end" }, &mut v, 0.0, (t.dur * 10.0).round() / 10.0, 0.1, 1, " s", 110.0, true);
                    });
                    if here {
                        v = (t.pos / STEP * 10.0).round() / 10.0;
                        changed = true;
                    }
                    if changed {
                        if is_start {
                            t.start = v;
                        } else {
                            t.end = v;
                        }
                        t.pos = (v * STEP).round();
                        t.ask();
                    }
                }
                w::separator(ui, p);
                w::row(ui, p, "Delete the original", "", false, true, |_| {}, |ui| {
                    w::switch(ui, p, &mut t.delete, true);
                });
            });
            ui.add_space(4.0);
            let ok = t.end - t.start >= 0.5;
            ui.horizontal(|ui| {
                w::subtitle(ui, p, &if ok { format!("New length {}", human_time_exact(t.end - t.start)) } else { "The end must be after the start".to_string() });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if w::button(ui, p, "Save trimmed copy", Kind::Accent, ok).clicked() {
                        save = true;
                    }
                    if w::button(ui, p, "Cancel", Kind::Normal, true).clicked() {
                        close = true;
                    }
                });
            });
        });
        if m.should_close() {
            close = true;
        }
        if save {
            self.show_toast("Trimming…");
            let (tx, ctx2) = (self.tx.clone(), ctx.clone());
            let (cfg, path, a, b, rm) = (self.cfg.clone(), t.path.clone(), t.start, t.end, t.delete);
            let st = self.status();
            let enc = st.get("encoder").and_then(|e| e.as_str()).filter(|e| !e.is_empty()).map(String::from).unwrap_or_else(|| crate::system::pick_encoder(&crate::system::probe(false), &cfg.s("encoder")));
            std::thread::spawn(move || {
                let _ = tx.send(UiEv::TrimDone(trim::trim(&cfg, &enc, &path, a, b, rm)));
                ctx2.request_repaint();
            });
            close = true;
        }
        !close
    }

    fn rename_dialog(&mut self, ctx: &egui::Context, p: &Palette, path: &Path, buf: &mut String, error: &mut String, focus: &mut bool) -> bool {
        let mut close = false;
        let mut go = false;
        let m = egui::Modal::new(egui::Id::new("rename")).frame(modal_frame(p, 28, 24, 20)).show(ctx, |ui| {
            ui.set_width(364.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            w::big(ui, p, "Rename clip");
            let r = w::text_field(ui, p, "rename", buf, 364.0);
            if *focus {
                r.request_focus();
                *focus = false;
            }
            if r.changed() {
                error.clear();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            w::subtitle(ui, p, error);
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if w::button(ui, p, "Rename", Kind::Accent, true).clicked() {
                    go = true;
                }
                if w::button(ui, p, "Cancel", Kind::Normal, true).clicked() {
                    close = true;
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if go {
            match trim::rename(path, buf) {
                Ok(new) => {
                    if new != path {
                        self.show_toast("Renamed");
                        self.load_clips(true);
                    }
                    close = true;
                }
                Err(e) => *error = e,
            }
        }
        !close
    }

    fn hotkey_dialog(&mut self, ctx: &egui::Context, p: &Palette, key: &'static str, hint: &mut String, held: &mut Vec<u32>, hold_sent: &mut bool) -> bool {
        if !*hold_sent {
            *hold_sent = true;
            // let go of the old key so it can be pressed here
            std::thread::spawn(move || {
                let _ = crate::ipc::request("hotkey_hold", json!({"which": key, "on": true}));
            });
        }
        let mut close = false;
        let mut result: Option<String> = None;
        // any key, whatever the layout: the keys held down right now
        let now = winutil::keys_down();
        let is_mod = |vk: u32| matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C | 0x14 | 0x90);
        let has = |a: &[u32]| now.iter().any(|v| a.contains(v));
        let (ctrl, alt, shift, win) = (has(&[0x11, 0xA2, 0xA3]), has(&[0x12, 0xA4, 0xA5]), has(&[0x10, 0xA0, 0xA1]), has(&[0x5B, 0x5C]));
        for vk in now.iter().copied().filter(|vk| !held.contains(vk) && !is_mod(*vk)) {
            if vk == 0x1B && !(ctrl || alt || shift || win) {
                close = true;
                break;
            }
            let Some(name) = hotkey::name_of(vk) else {
                *hint = "This key can't be used. Try another one.".into();
                continue;
            };
            if !(ctrl || alt || shift || win) && !hotkey::solo_ok(&name) {
                *hint = "Use it with Ctrl, Alt, Shift or Win.".into();
                continue;
            }
            result = Some(hotkey::compose(ctrl, alt, shift, win, &name));
            break;
        }
        *held = now;
        let m = egui::Modal::new(egui::Id::new("hotkey")).frame(modal_frame(p, 28, 24, 20)).show(ctx, |ui| {
            ui.set_width(304.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            w::big(ui, p, "Press a new hotkey");
            w::subtitle(ui, p, hint);
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, p, "Cancel", Kind::Normal, true).clicked() {
                    close = true;
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(r) = result {
            let other = if key == "hotkey" { "record_hotkey" } else { "hotkey" };
            if r == self.cfg.s(other) {
                self.show_toast("That key is already used for the other action");
            } else {
                self.set(key, json!(r));
            }
            close = true;
        }
        if close {
            std::thread::spawn(move || {
                // give the key back (the new one, once the settings are saved)
                std::thread::sleep(std::time::Duration::from_millis(600));
                let _ = crate::ipc::request("hotkey_hold", json!({"which": key, "on": false}));
            });
        }
        let _ = ctx;
        !close
    }

    fn about_dialog(&mut self, ctx: &egui::Context, p: &Palette) -> bool {
        let mut close = false;
        let m = egui::Modal::new(egui::Id::new("about")).frame(modal_frame(p, 28, 24, 20)).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                if let Ok(img) = image::load_from_memory(C::ICON_PNG) {
                    let r = img.to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([r.width() as usize, r.height() as usize], r.as_raw());
                    let tex = ui.ctx().load_texture("about-icon", ci, egui::TextureOptions::LINEAR);
                    ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(64.0, 64.0)));
                }
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(C::APP_NAME).font(w::font_semibold(theme_pt(13.0))).color(p.text));
                    w::dim(ui, p, &format!("Version {}", C::VERSION));
                });
            });
            ui.add_space(6.0);
            w::label(ui, p, "Instant replay for Windows by DRZ.", w::BODY, p.text);
            ui.hyperlink_to(format!("github.com/{}", C::REPO), format!("https://github.com/{}", C::REPO));
            ui.add_space(6.0);
            w::subtitle(ui, p, "Uses FFmpeg (GPL). Click sound by Brian MacIntosh (CC0).");
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, p, "OK", Kind::Accent, true).clicked() {
                    close = true;
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        !close
    }
}

fn theme_pt(p: f32) -> f32 {
    super::theme::pt(p)
}
