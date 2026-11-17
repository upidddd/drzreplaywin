//! The small widgets the window is built from, drawn to the metrics of the Qt style sheet the
//! window used before.
use super::theme::{icon, pt, semibold, Palette};
use eframe::egui::{self, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder};

pub const BODY: f32 = 13.33; // 10 pt
pub const SMALL: f32 = 12.0; // 9 pt

pub fn popup_is_open(ui: &Ui, id: egui::Id) -> bool {
    ui.memory(|m| m.data.get_temp::<bool>(id)).unwrap_or(false)
}

pub fn popup_set(ui: &Ui, id: egui::Id, on: bool) {
    ui.memory_mut(|m| m.data.insert_temp(id, on));
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Proportional)
}
pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// text size in a font
pub fn text_size(ui: &Ui, s: &str, f: FontId) -> egui::Vec2 {
    ui.painter().layout_no_wrap(s.to_string(), f, Color32::WHITE).size()
}

// ---------------------------------------------------------------- labels
pub fn label(ui: &mut Ui, p: &Palette, text: &str, size: f32, color: Color32) {
    ui.label(egui::RichText::new(text).font(font(size)).color(color));
    let _ = p;
}

pub fn dim(ui: &mut Ui, p: &Palette, text: &str) {
    label(ui, p, text, BODY, p.dim);
}

pub fn subtitle(ui: &mut Ui, p: &Palette, text: &str) {
    label(ui, p, text, SMALL, p.dim);
}

pub fn big(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).font(font_semibold(pt(14.0))).color(p.text));
}

// ---------------------------------------------------------------- switch
pub fn switch(ui: &mut Ui, p: &Palette, on: &mut bool, enabled: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(42.0, 22.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let hovered = resp.hovered() && enabled;
    let r = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(40.0, 20.0));
    let pa = ui.painter();
    if *on {
        pa.rect_filled(r, radius(10.0), if enabled { p.accent } else { p.track });
    } else {
        pa.rect_filled(r, radius(10.0), p.control);
        pa.rect_stroke(r, radius(10.0), Stroke::new(1.0, if enabled { p.dim } else { p.track }), StrokeKind::Inside);
    }
    let d = if hovered { 14.0 } else { 12.0 };
    let x = rect.min.x + 11.0 + t * 20.0;
    let knob = if *on && enabled { p.accent_text } else if enabled { p.dim } else { p.track };
    pa.circle_filled(egui::pos2(x, rect.min.y + 11.0), d / 2.0, knob);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

// ---------------------------------------------------------------- buttons
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Normal,
    Accent,
    /// the big "Save replay"
    Big,
    /// "Start recording": red while recording
    Rec(bool),
}

pub fn button(ui: &mut Ui, p: &Palette, text: &str, kind: Kind, enabled: bool) -> Response {
    let (size_pt, semi, pad_x, min_h): (f32, bool, f32, f32) = match kind {
        Kind::Normal => (10.0, false, 14.0, 30.0),
        Kind::Accent => (10.0, true, 14.0, 30.0),
        Kind::Big => (10.5, true, 40.0, 38.0),
        Kind::Rec(_) => (10.0, matches!(kind, Kind::Rec(true)), 22.0, 34.0),
    };
    let f = if semi { font_semibold(pt(size_pt)) } else { font(pt(size_pt)) };
    let ts = text_size(ui, text, f.clone());
    let size = vec2(ts.x + 2.0 * pad_x + 2.0, min_h.max(ts.y + 12.0));
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let down = resp.is_pointer_button_down_on() && enabled;
    let hov = resp.hovered() && enabled;
    let (fill, stroke, fg) = match kind {
        Kind::Accent | Kind::Big => {
            if !enabled {
                (p.track, p.track, p.card)
            } else if down {
                (p.accent_press, p.accent_press, p.accent_text)
            } else if hov {
                (p.accent_hover, p.accent_hover, p.accent_text)
            } else {
                (p.accent, p.accent, p.accent_text)
            }
        }
        Kind::Rec(true) => (p.rec, p.rec, p.card),
        _ => {
            let fill = if down { p.bg } else if hov { p.control_hover } else { p.control };
            (fill, p.control_border, if !enabled || down { p.dim } else { p.text })
        }
    };
    let pa = ui.painter();
    pa.rect_filled(rect, radius(5.0), fill);
    pa.rect_stroke(rect, radius(5.0), Stroke::new(1.0, stroke), StrokeKind::Inside);
    pa.text(rect.center(), Align2::CENTER_CENTER, text, f, fg);
    if hov {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// the flat icon buttons (copy, folder, trash…)
pub fn icon_button(ui: &mut Ui, p: &Palette, name: &str, tip: &str) -> Response {
    let size = 16.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 12.0, size + 12.0), Sense::click());
    let pa = ui.painter();
    if resp.is_pointer_button_down_on() {
        pa.rect_filled(rect, radius(5.0), p.press);
    } else if resp.hovered() {
        pa.rect_filled(rect, radius(5.0), p.hover);
    }
    let tex = icon(ui.ctx(), name, p.text, size, 1.75);
    pa.image(tex.id(), Rect::from_center_size(rect.center(), vec2(size, size)), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(tip)
}

pub fn draw_icon(ui: &Ui, rect: Rect, name: &str, color: Color32, stroke: f32) {
    let tex = icon(ui.ctx(), name, color, rect.width(), stroke);
    ui.painter().image(tex.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// header tab: icon + text, an accent pill under the selected one
pub fn tab(ui: &mut Ui, p: &Palette, text: &str, name: &str, selected: bool) -> Response {
    let f = if selected { font_semibold(pt(10.0)) } else { font(pt(10.0)) };
    let ts = text_size(ui, text, f.clone());
    let size = vec2(14.0 + 16.0 + 6.0 + ts.x + 14.0, 7.0 + ts.y.max(16.0) + 9.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let pa = ui.painter();
    if resp.hovered() {
        pa.rect_filled(rect, radius(6.0), p.hover);
    }
    let col = if selected || resp.hovered() { p.text } else { p.dim };
    let icon_rect = Rect::from_min_size(egui::pos2(rect.min.x + 14.0, rect.center().y - 8.5), vec2(16.0, 16.0));
    draw_icon(ui, icon_rect, name, if selected { p.text } else { p.dim }, 1.75);
    ui.painter().text(egui::pos2(icon_rect.max.x + 6.0, rect.center().y), Align2::LEFT_CENTER, text, f, col);
    if selected {
        let w = 18.0;
        ui.painter().rect_filled(Rect::from_min_size(egui::pos2(rect.center().x - w / 2.0, rect.max.y - 3.0), vec2(w, 3.0)), radius(1.5), p.accent);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

// ---------------------------------------------------------------- combo box
fn chevron(ui: &Ui, rect: Rect, p: &Palette, up: bool) {
    draw_icon(ui, rect, if up { "chevron-up" } else { "chevron" }, p.dim, 2.0);
}

/// A drop-down list; returns true when another entry was picked.
pub fn combo(ui: &mut Ui, p: &Palette, id: &str, options: &[String], selected: &mut usize, enabled: bool) -> bool {
    let cur = options.get(*selected).cloned().unwrap_or_default();
    let width = {
        let widest = options.iter().map(|o| text_size(ui, o, font(BODY)).x).fold(0.0, f32::max);
        (widest + 10.0 + 28.0 + 8.0).max(150.0)
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let hov = resp.hovered() && enabled;
    let pa = ui.painter();
    pa.rect_filled(rect, radius(5.0), if hov { p.control_hover } else { p.control });
    pa.rect_stroke(rect, radius(5.0), Stroke::new(1.0, p.control_border), StrokeKind::Inside);
    pa.text(egui::pos2(rect.min.x + 11.0, rect.center().y), Align2::LEFT_CENTER, &cur, font(BODY), if enabled { p.text } else { p.dim });
    chevron(ui, Rect::from_center_size(egui::pos2(rect.max.x - 16.0, rect.center().y), vec2(12.0, 12.0)), p, false);
    if hov {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut changed = false;
    let popup_id = ui.make_persistent_id(("combo", id));
    if resp.clicked() {
        popup_set(ui, popup_id, !popup_is_open(ui, popup_id));
    }
    if popup_is_open(ui, popup_id) {
        let area = egui::Area::new(popup_id).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, 2.0)).constrain(true);
        let inner = area.show(ui.ctx(), |ui| {
            egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.control_border)).corner_radius(radius(6.0)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_width(width - 10.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (i, o) in options.iter().enumerate() {
                    let (r, rr) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    if rr.hovered() || i == *selected {
                        ui.painter().rect_filled(r, radius(4.0), p.hover);
                    }
                    ui.painter().text(egui::pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, o, font(BODY), p.text);
                    if rr.clicked() {
                        if i != *selected {
                            *selected = i;
                            changed = true;
                        }
                        popup_set(ui, popup_id, false);
                    }
                }
            });
        });
        // a click outside closes it
        if ui.input(|i| i.pointer.any_click()) && !inner.response.contains_pointer() && !resp.contains_pointer() {
            popup_set(ui, popup_id, false);
        }
    }
    changed
}

// ---------------------------------------------------------------- spin box
/// A number field with up / down buttons. Returns true when the value changed.
pub fn spin(ui: &mut Ui, p: &Palette, id: &str, value: &mut f64, lo: f64, hi: f64, step: f64, decimals: usize, suffix: &str, width: f32, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let edit_id = ui.make_persistent_id(("spin", id));
    let mut changed = false;
    let fmt = |v: f64| format!("{v:.decimals$}{suffix}");
    let hov = resp.hovered() && enabled;
    let editing = ui.memory(|m| m.has_focus(edit_id));
    let pa = ui.painter();
    pa.rect_filled(rect, radius(5.0), if hov { p.control_hover } else { p.control });
    pa.rect_stroke(rect, radius(5.0), Stroke::new(1.0, p.control_border), StrokeKind::Inside);
    if editing {
        pa.rect_filled(Rect::from_min_max(egui::pos2(rect.min.x + 3.0, rect.max.y - 2.0), egui::pos2(rect.max.x - 3.0, rect.max.y)), radius(1.0), p.accent);
    }
    // the up / down buttons on the right
    let up = Rect::from_min_size(egui::pos2(rect.max.x - 24.0, rect.min.y + 3.0), vec2(22.0, 13.0));
    let down = Rect::from_min_size(egui::pos2(rect.max.x - 24.0, rect.max.y - 16.0), vec2(22.0, 13.0));
    let (up_r, down_r) = (ui.interact(up, edit_id.with("up"), if enabled { Sense::click() } else { Sense::hover() }), ui.interact(down, edit_id.with("down"), if enabled { Sense::click() } else { Sense::hover() }));
    for (r, rr, is_up) in [(up, &up_r, true), (down, &down_r, false)] {
        if rr.hovered() && enabled {
            ui.painter().rect_filled(r, radius(3.0), p.hover);
        }
        chevron(ui, Rect::from_center_size(r.center(), vec2(10.0, 10.0)), p, is_up);
    }
    if enabled {
        if up_r.clicked() {
            *value = (*value + step).min(hi);
            changed = true;
        }
        if down_r.clicked() {
            *value = (*value - step).max(lo);
            changed = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 && editing {
                *value = (*value + step * scroll.signum() as f64).clamp(lo, hi);
                changed = true;
            }
        }
    }
    // the number: typed into a text field while it has focus
    let text_rect = Rect::from_min_max(egui::pos2(rect.min.x + 8.0, rect.min.y + 2.0), egui::pos2(rect.max.x - 26.0, rect.max.y - 3.0));
    let buf_id = edit_id.with("buf");
    if enabled && (resp.clicked() || editing) {
        let mut buf: String = ui.memory_mut(|m| m.data.get_temp::<String>(buf_id)).unwrap_or_else(|| format!("{value:.decimals$}"));
        let mut child = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(Layout::left_to_right(Align::Center)));
        let te = egui::TextEdit::singleline(&mut buf).id(edit_id).frame(egui::Frame::NONE).font(font(BODY)).text_color(p.text).desired_width(text_rect.width()).margin(egui::Margin::ZERO);
        let r = child.add(te);
        if resp.clicked() && !editing {
            r.request_focus();
        }
        if r.has_focus() {
            ui.memory_mut(|m| m.data.insert_temp(buf_id, buf.clone()));
        }
        if r.lost_focus() {
            if let Ok(v) = buf.trim().trim_end_matches(suffix.trim()).trim().replace(',', ".").parse::<f64>() {
                let v = v.clamp(lo, hi);
                if (v - *value).abs() > 1e-9 {
                    *value = v;
                    changed = true;
                }
            }
            ui.memory_mut(|m| m.data.remove::<String>(buf_id));
        }
    } else {
        ui.painter().text(egui::pos2(text_rect.min.x, rect.center().y), Align2::LEFT_CENTER, fmt(*value), font(BODY), if enabled { p.text } else { p.dim });
    }
    changed
}

// ---------------------------------------------------------------- slider and progress
/// QSlider: a 4 px track, an accent part up to the handle, a round handle with a ring.
pub fn slider(ui: &mut Ui, p: &Palette, value: &mut f64, lo: f64, hi: f64, step: f64, width: f32, enabled: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 22.0), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let inner = Rect::from_min_max(egui::pos2(rect.min.x + 9.0, rect.min.y), egui::pos2(rect.max.x - 9.0, rect.max.y));
    if enabled && (resp.dragged() || resp.clicked()) {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((pos.x - inner.min.x) / inner.width()).clamp(0.0, 1.0) as f64;
            let mut v = lo + t * (hi - lo);
            if step > 0.0 {
                v = (v / step).round() * step;
            }
            let v = v.clamp(lo, hi);
            if (v - *value).abs() > 1e-9 {
                *value = v;
                resp.mark_changed();
            }
        }
    }
    let t = if hi > lo { ((*value - lo) / (hi - lo)) as f32 } else { 0.0 };
    let x = inner.min.x + t * inner.width();
    let cy = rect.center().y;
    let pa = ui.painter();
    pa.rect_filled(Rect::from_min_max(egui::pos2(inner.min.x - 2.0, cy - 2.0), egui::pos2(inner.max.x + 2.0, cy + 2.0)), radius(2.0), p.track);
    pa.rect_filled(Rect::from_min_max(egui::pos2(inner.min.x - 2.0, cy - 2.0), egui::pos2(x, cy + 2.0)), radius(2.0), if enabled { p.accent } else { p.track });
    let hov = (resp.hovered() || resp.dragged()) && enabled;
    pa.circle_filled(egui::pos2(x, cy), 9.0, if enabled { p.accent } else { p.track });
    pa.circle_stroke(egui::pos2(x, cy), 9.0 - if hov { 1.5 } else { 2.0 }, Stroke::new(if hov { 3.0 } else { 4.0 }, p.control));
    pa.circle_filled(egui::pos2(x, cy), if hov { 6.0 } else { 5.0 }, if enabled { p.accent } else { p.track });
    resp
}

pub fn progress(ui: &mut Ui, p: &Palette, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, radius(2.0), p.track);
    let w = (rect.width() * fraction.clamp(0.0, 1.0)).max(0.0);
    if w > 0.5 {
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(w.max(4.0), 4.0)), radius(2.0), p.accent);
    }
}

// ---------------------------------------------------------------- rows and groups
/// One row of a card: [prefix] title / subtitle … [suffix]. A hovered activatable row gets a highlight.
#[allow(clippy::too_many_arguments)]
pub fn row(ui: &mut Ui, p: &Palette, title: &str, sub: &str, activatable: bool, enabled: bool, prefix: impl FnOnce(&mut Ui), suffix: impl FnOnce(&mut Ui)) -> Response {
    let h = if sub.is_empty() { 52.0 } else { 54.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), if activatable { Sense::click() } else { Sense::hover() });
    if activatable && resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 0.0)), radius(7.0), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let inner = Rect::from_min_max(egui::pos2(rect.min.x + 16.0, rect.min.y), egui::pos2(rect.max.x - 12.0, rect.max.y));
    // the controls on the right first: they say how much room is left for the text
    let mut right = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing = vec2(6.0, 0.0);
    if !enabled {
        right.disable();
    }
    suffix(&mut right);
    let used_r = right.min_rect();
    let suffix_left = if used_r.width() > 0.5 { used_r.min.x } else { inner.max.x };
    // the picture on the left
    let mut left = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    left.spacing_mut().item_spacing = vec2(12.0, 0.0);
    if !enabled {
        left.disable();
    }
    prefix(&mut left);
    let used_l = left.min_rect();
    let text_x = if used_l.width() > 0.5 { used_l.max.x + 12.0 } else { inner.min.x };
    // title and subtitle, together centred in the row
    let tc = if enabled { p.text } else { p.dim };
    let tg = ui.painter().layout_no_wrap(title.to_string(), font(BODY), tc);
    let sg = (!sub.is_empty()).then(|| ui.painter().layout_no_wrap(sub.to_string(), font(SMALL), p.dim));
    let total = tg.size().y + sg.as_ref().map(|g| g.size().y + 1.0).unwrap_or(0.0);
    let top = rect.center().y - total / 2.0;
    let clip = Rect::from_min_max(egui::pos2(text_x, rect.min.y), egui::pos2((suffix_left - 12.0).max(text_x), rect.max.y));
    let pa = ui.painter().with_clip_rect(clip);
    let th = tg.size().y;
    pa.galley(egui::pos2(text_x, top), tg, tc);
    if let Some(g) = sg {
        pa.galley(egui::pos2(text_x, top + th + 1.0), g, p.dim);
    }
    resp
}

pub fn separator(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.sep);
}

/// A titled card of rows, like a libadwaita preferences group.
pub fn group<R>(ui: &mut Ui, p: &Palette, title: &str, description: &str, header_suffix: impl FnOnce(&mut Ui), rows: impl FnOnce(&mut Ui) -> R) -> R {
    ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
    let has_head = !title.is_empty() || !description.is_empty();
    if has_head {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if !title.is_empty() {
                    ui.label(egui::RichText::new(title).font(font_semibold(pt(10.5))).color(p.text));
                }
                if !description.is_empty() {
                    subtitle(ui, p, description);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                header_suffix(ui);
            });
        });
    }
    let out = egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(radius(8.0)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        rows(ui)
    });
    out.inner
}

/// toast: short message at the bottom of the window
pub fn toast(ctx: &egui::Context, p: &Palette, text: &str, bottom_margin: f32) {
    egui::Area::new(egui::Id::new("toast")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -bottom_margin)).interactable(false).show(ctx, |ui| {
        egui::Frame::new().fill(p.toast).corner_radius(radius(6.0)).inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
            ui.label(egui::RichText::new(text).font(font(BODY)).color(p.toast_text));
        });
    });
}

/// A one-line text field (QLineEdit): returns the text edit's response.
pub fn text_field(ui: &mut Ui, p: &Palette, id: &str, text: &mut String, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    let edit_id = ui.make_persistent_id(("textfield", id));
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let hover = ui.rect_contains_pointer(rect);
    let pa = ui.painter();
    pa.rect_filled(rect, radius(5.0), if hover { p.control_hover } else { p.control });
    pa.rect_stroke(rect, radius(5.0), Stroke::new(1.0, p.control_border), StrokeKind::Inside);
    if focused {
        pa.rect_filled(Rect::from_min_max(egui::pos2(rect.min.x + 3.0, rect.max.y - 2.0), egui::pos2(rect.max.x - 3.0, rect.max.y)), radius(1.0), p.accent);
    }
    let inner = Rect::from_min_max(egui::pos2(rect.min.x + 10.0, rect.min.y + 2.0), egui::pos2(rect.max.x - 10.0, rect.max.y - 3.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    child.add(egui::TextEdit::singleline(text).id(edit_id).frame(egui::Frame::NONE).font(font(BODY)).text_color(p.text).desired_width(inner.width()).margin(egui::Margin::ZERO))
}
