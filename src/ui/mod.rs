//! The DrzReplayWIN window: Record, Clips and Settings tabs. It runs as its own process and talks to the
//! running app over IPC; settings are written to the config file, then the app is asked to reload.
mod dialogs;
mod pages;
pub mod theme;
pub mod widgets;

use crate::config::{self as C, Config};
use crate::{ipc, system as sys, winutil};
use eframe::egui;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use theme::Palette;

pub const MAX_CLIPS_SHOWN: usize = 60;

/// what worker threads tell the window
pub enum UiEv {
    Thumb { path: PathBuf, jpg: Option<PathBuf>, dur: f64 },
    Frame { jpg: Vec<u8> },
    TrimDone(Result<PathBuf, String>),
    Folder(PathBuf),
    Toast(String),
}

#[derive(Clone)]
pub struct Clip {
    pub mtime: SystemTime,
    pub size: u64,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Thumb {
    pub tex: Option<egui::TextureHandle>,
    pub dur: f64,
}

/// a way to make the window take screenshots of itself (the self-test)
pub struct Script {
    pub out: PathBuf,
    pub steps: Vec<Step>,
    pub at: usize,
    pub wait: u32,
    pub waiting_shot: bool,
}

#[derive(Clone)]
pub enum Step {
    Theme(bool),
    Tab(usize),
    Trim(PathBuf),
    Shot(String),
    CloseDialog,
}

pub struct App {
    pub cfg: Config,
    pub dirty: HashSet<String>,
    pub flush_at: Option<Instant>,
    pub tab: usize,
    pub status: Arc<Mutex<Option<Value>>>,
    pub active: Arc<AtomicBool>,
    pub pal: Palette,
    pub force_dark: Option<bool>,
    pub toast: Option<(String, Instant)>,
    pub rx: Receiver<UiEv>,
    pub tx: Sender<UiEv>,
    pub clips: Vec<Clip>,
    pub clips_sig: Option<String>,
    pub dir_sig: Option<(PathBuf, Option<SystemTime>)>,
    pub thumbs: HashMap<PathBuf, Thumb>,
    pub thumb_tx: Sender<PathBuf>,
    pub thumb_asked: HashSet<PathBuf>,
    pub dialog: Option<dialogs::Dialog>,
    pub script: Option<Script>,
    pub misses: u32,
    pub update_note: Option<String>,
    pub ver_busy: bool,
    pub ver_started: f64,
    pub name_buf: Option<String>,
}

impl App {
    pub fn new(ctx: &egui::Context, script: Option<Script>) -> App {
        theme::install_fonts(ctx);
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(None));
        let active = Arc::new(AtomicBool::new(true));
        // the status of the running app, once a second
        {
            let (status, active, ctx, tx) = (status.clone(), active.clone(), ctx.clone(), tx.clone());
            std::thread::Builder::new()
                .name("status".into())
                .spawn(move || loop {
                    let r = ipc::request("status", json!({"active": active.load(Ordering::Relaxed)}));
                    let gone = r.is_none();
                    if let Some(Value::Object(m)) = &r {
                        if let Some(Value::Array(t)) = m.get("toasts") {
                            for x in t {
                                if let Some(s) = x.as_str() {
                                    let _ = tx.send(UiEv::Toast(s.to_string()));
                                }
                            }
                        }
                    }
                    *status.lock().unwrap() = Some(match r {
                        Some(v) => v,
                        None => json!({"gone": true}),
                    });
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(if gone { 400 } else { 1000 }));
                })
                .ok();
        }
        let thumb_tx = pages::spawn_thumb_worker(tx.clone(), ctx.clone());
        App {
            cfg: Config::load(),
            dirty: HashSet::new(),
            flush_at: None,
            tab: 0,
            status,
            active,
            pal: Palette::light(),
            force_dark: None,
            toast: None,
            rx,
            tx,
            clips: vec![],
            clips_sig: None,
            dir_sig: None,
            thumbs: HashMap::new(),
            thumb_tx,
            thumb_asked: HashSet::new(),
            dialog: None,
            script,
            misses: 0,
            update_note: None,
            ver_busy: false,
            ver_started: 0.0,
            name_buf: None,
        }
    }

    pub fn show_toast(&mut self, text: &str) {
        self.toast = Some((text.to_string(), Instant::now() + Duration::from_millis(3000)));
    }

    pub fn status(&self) -> Value {
        self.status.lock().unwrap().clone().unwrap_or_else(|| json!({}))
    }

    // ------------------------------------------------------------------ settings plumbing
    pub fn set(&mut self, key: &str, value: Value) {
        if self.cfg.get(key) == &value {
            return;
        }
        if key == "monitor" {
            self.set("screen_picked", json!(true));
        }
        self.cfg.set(key, value);
        self.dirty.insert(key.to_string());
        self.flush_at = Some(Instant::now() + Duration::from_millis(400));
    }

    /// write what was changed here (the app may have changed other keys meanwhile), then ask it to reload
    pub fn flush(&mut self) {
        self.flush_at = None;
        if self.dirty.is_empty() {
            return;
        }
        let mut cfg = Config::load();
        for k in self.dirty.drain() {
            cfg.set(&k, self.cfg.get(&k).clone());
        }
        cfg.save();
        self.cfg = cfg;
        std::thread::spawn(|| {
            let _ = ipc::request("reload", json!({}));
        });
    }

    pub fn command(&self, cmd: &'static str) {
        std::thread::spawn(move || {
            let _ = ipc::request(cmd, json!({}));
        });
    }

    // ------------------------------------------------------------------ clips
    pub fn clip_files(&self) -> Vec<Clip> {
        let mut out: Vec<Clip> = std::fs::read_dir(self.cfg.output_dir())
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        if !(name.ends_with(".mp4") || name.ends_with(".mkv")) {
                            return None;
                        }
                        let m = e.metadata().ok()?;
                        m.is_file().then(|| Clip { mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: m.len(), path: e.path() })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then(b.path.cmp(&a.path)));
        out
    }

    pub fn load_clips(&mut self, force: bool) {
        // polled every second: a look at the folder tells whether anything changed
        let d = self.cfg.output_dir();
        let dir_sig = (d.clone(), std::fs::metadata(&d).and_then(|m| m.modified()).ok());
        if self.dir_sig.as_ref() == Some(&dir_sig) && !force && self.clips_sig.is_some() {
            return;
        }
        self.dir_sig = Some(dir_sig);
        let clips = self.clip_files();
        let sig = format!("{}|{}", d.display(), clips.iter().take(MAX_CLIPS_SHOWN).map(|c| format!("{}:{:?}:{}", c.path.display(), c.mtime, c.size)).collect::<Vec<_>>().join(";"));
        if self.clips_sig.as_deref() == Some(sig.as_str()) && !force {
            return;
        }
        self.clips_sig = Some(sig);
        self.clips = clips;
        if force {
            self.thumb_asked.clear();
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                UiEv::Thumb { path, jpg, dur } => {
                    let t = self.thumbs.entry(path).or_default();
                    t.dur = dur;
                    if let Some(j) = jpg {
                        if let Ok(img) = image::open(&j) {
                            let rgba = img.to_rgba8();
                            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                            t.tex = Some(ctx.load_texture("thumb", ci, egui::TextureOptions::LINEAR));
                        }
                    }
                }
                UiEv::Frame { jpg } => {
                    if let (Some(dialogs::Dialog::Trim(t)), Ok(img)) = (self.dialog.as_mut(), image::load_from_memory(&jpg)) {
                        let rgba = img.to_rgba8();
                        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                        t.preview = Some(ctx.load_texture("frame", ci, egui::TextureOptions::LINEAR));
                    }
                }
                UiEv::TrimDone(r) => {
                    match r {
                        Ok(out) => self.show_toast(&format!("Saved {}", out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                        Err(e) => self.show_toast(&format!("Couldn't trim: {e}")),
                    }
                    self.load_clips(true);
                }
                UiEv::Folder(d) => {
                    self.set("output_dir", json!(d.to_string_lossy()));
                    self.load_clips(true);
                }
                UiEv::Toast(t) => self.show_toast(&t),
            }
        }
    }

    fn follow_theme(&mut self, ctx: &egui::Context) {
        let dark = self.force_dark.unwrap_or_else(|| ctx.system_theme().map(|t| t == egui::Theme::Dark).unwrap_or(false));
        if dark != self.pal.dark || self.force_dark.is_some() && !self.pal.dark == dark {
            self.pal = Palette::from_dark(dark);
            theme::apply(ctx, &self.pal);
        }
    }

    fn run_script(&mut self, ctx: &egui::Context) {
        let Some(mut sc) = self.script.take() else { return };
        if sc.wait > 0 {
            sc.wait -= 1;
            ctx.request_repaint();
        } else {
            match sc.steps.get(sc.at).cloned() {
                None => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(Step::Theme(dark)) => {
                    self.force_dark = Some(dark);
                    sc.at += 1;
                    sc.wait = 4;
                }
                Some(Step::Tab(t)) => {
                    self.tab = t;
                    sc.at += 1;
                    sc.wait = 6;
                }
                Some(Step::Trim(p)) => {
                    let dur = sys::media_duration(&p);
                    self.dialog = Some(dialogs::Dialog::Trim(Box::new(dialogs::TrimState::new(&p, dur.max(1.0), &self.tx, ctx))));
                    sc.at += 1;
                    sc.wait = 40; // the first frame has to arrive
                }
                Some(Step::CloseDialog) => {
                    self.dialog = None;
                    sc.at += 1;
                    sc.wait = 3;
                }
                Some(Step::Shot(name)) => {
                    let path = sc.out.join(name);
                    ctx.request_screenshot(move |img| save_png(&path, &img));
                    sc.at += 1;
                    sc.wait = 8; // the picture arrives a frame or two later
                }
            }
            ctx.request_repaint();
        }
        self.script = Some(sc);
    }
}

fn save_png(path: &std::path::Path, img: &egui::ColorImage) {
    let mut buf = image::RgbaImage::new(img.width() as u32, img.height() as u32);
    for (i, p) in img.pixels.iter().enumerate() {
        buf.put_pixel((i % img.width()) as u32, (i / img.width()) as u32, image::Rgba([p.r(), p.g(), p.b(), p.a()]));
    }
    let _ = buf.save(path);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.follow_theme(&ctx);
        self.drain_events(&ctx);
        self.active.store(ctx.input(|i| i.viewport().focused.unwrap_or(true)), Ordering::Relaxed);
        if self.script.is_some() {
            self.run_script(&ctx);
        }
        if self.flush_at.is_some_and(|t| Instant::now() >= t) {
            self.flush();
        }
        if self.toast.as_ref().is_some_and(|(_, t)| Instant::now() >= *t) {
            self.toast = None;
        }
        // the app
        let st = self.status();
        if st.get("gone").is_some() && self.script.is_none() {
            self.misses += 1;
            if self.misses > 6 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close); // the app quit
            }
        } else {
            self.misses = 0;
        }
        if st.get("show").and_then(|s| s.as_bool()).unwrap_or(false) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.poll_update_check();
        self.load_clips(false);

        let p = self.pal;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, p.bg);
        self.header(ui, &p);
        self.banner(ui, &p, &st);
        let rest = ui.available_rect_before_wrap();
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::top_down(egui::Align::Min)));
        match self.tab {
            0 => pages::scroll(&mut body, true, |ui| self.page_record(ui, &p, &st)),
            1 => self.page_clips(&mut body, &p),
            _ => pages::scroll(&mut body, true, |ui| self.page_settings(ui, &p, &st)),
        }
        self.dialogs(&ctx, &p);
        if let Some((text, _)) = &self.toast {
            widgets::toast(&ctx, &p, text, 24.0);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl App {
    fn header(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.add_space(10.0);
        let more_w = 28.0;
        let mut more = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // centred: the tabs sit between two equal margins
            let tabs_w: f32 = ["Record", "Clips", "Settings"].iter().map(|t| 14.0 + 16.0 + 6.0 + widgets::text_size(ui, t, widgets::font_semibold(theme::pt(10.0))).x + 14.0).sum();
            let side = ((ui.available_width() - tabs_w) / 2.0).max(0.0);
            ui.add_space(side.max(12.0 + more_w) - more_w);
            ui.add_space(more_w);
            for (i, (text, icon)) in [("Record", "record"), ("Clips", "clips"), ("Settings", "settings")].into_iter().enumerate() {
                if widgets::tab(ui, p, text, icon, self.tab == i).clicked() {
                    self.tab = i;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                more = widgets::icon_button(ui, p, "more", "Menu").clicked();
            });
        });
        ui.add_space(6.0);
        self.menu_popup(ui, p, more);
    }

    fn banner(&mut self, ui: &mut egui::Ui, p: &Palette, st: &Value) {
        let progress = st.get("update_progress").and_then(|v| v.as_str()).map(String::from);
        let ready = st.get("update_ready").and_then(|v| v.as_str()).map(String::from);
        let text = progress.clone().or_else(|| ready.as_ref().map(|v| format!("{} {v} is available", C::APP_NAME)));
        let Some(text) = text else { return };
        let h = 46.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.banner);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(egui::pos2(rect.min.x + 20.0, rect.min.y), egui::pos2(rect.max.x - 12.0, rect.max.y))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        widgets::label(&mut child, p, &text, widgets::BODY, p.text);
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, p, "Update", widgets::Kind::Accent, progress.is_none()).clicked() {
                self.command("run_update");
            }
        });
    }

    pub fn check_updates(&mut self, manual: bool) {
        if manual {
            self.ver_busy = true;
            self.ver_started = C::now();
            self.update_note = Some("Checking…".into());
        }
        self.command("check_update");
    }

    /// the answer to "Check": the app writes what it found next to its other caches
    fn poll_update_check(&mut self) {
        if !self.ver_busy {
            return;
        }
        let c = C::read_obj(&C::update_cache());
        let done = c.get("time").and_then(|t| t.as_f64()).is_some_and(|t| t >= self.ver_started - 0.5);
        if done {
            self.ver_busy = false;
            let latest = c.get("latest").and_then(|l| l.as_str()).map(String::from);
            self.update_note = Some(match latest {
                Some(l) if sys::is_newer(&l) => format!("Version {l} is available"),
                Some(_) => "Up to date".into(),
                None => "Couldn't check (offline?)".into(),
            });
        } else if C::now() - self.ver_started > 10.0 {
            self.ver_busy = false;
            self.update_note = Some("Couldn't check (offline?)".into());
        }
    }
}

// ---------------------------------------------------------------- entry
pub fn viewport() -> egui::ViewportBuilder {
    let icon = image::load_from_memory(C::ICON_PNG).ok().map(|i| {
        let r = i.to_rgba8();
        egui::IconData { width: r.width(), height: r.height(), rgba: r.into_raw() }
    });
    let mut v = egui::ViewportBuilder::default().with_title(C::APP_NAME).with_inner_size([660.0, 820.0]).with_min_inner_size([520.0, 480.0]);
    if let Some(i) = icon {
        v = v.with_icon(Arc::new(i));
    }
    v
}

/// Open the window (the app is running already). Returns when it is closed.
pub fn run_window(script: Option<Script>) -> i32 {
    winutil::set_dpi_aware();
    let options = eframe::NativeOptions { viewport: viewport(), ..Default::default() };
    let r = eframe::run_native(C::APP_NAME, options, Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, script)))));
    if let Err(e) = &r {
        eprintln!("window: {e}");
    }
    i32::from(r.is_err())
}
