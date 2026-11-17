//! Theme (Windows 11 light / dark) and the icons the window is built from.
use crate::fonts;
use eframe::egui::{self, Color32, ColorImage, FontData, FontDefinitions, FontFamily, TextureHandle, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub sep: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub hover: Color32,
    pub press: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub control_border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_press: Color32,
    pub accent_text: Color32,
    pub track: Color32,
    pub banner: Color32,
    pub toast: Color32,
    pub toast_text: Color32,
    pub rec: Color32,
    pub thumb: Color32,
}

fn hex(s: &str) -> Color32 {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Palette {
    pub fn light() -> Palette {
        Palette {
            dark: false,
            bg: hex("#f3f3f3"),
            card: hex("#fbfbfb"),
            border: hex("#e5e5e5"),
            sep: hex("#ebebeb"),
            text: hex("#1a1a1a"),
            dim: hex("#5d5d5d"),
            hover: Color32::from_black_alpha(12),
            press: Color32::from_black_alpha(19),
            control: hex("#ffffff"),
            control_hover: hex("#f9f9f9"),
            control_border: hex("#d5d5d5"),
            accent: hex("#0078d4"),
            accent_hover: hex("#1a86d9"),
            accent_press: hex("#006cbe"),
            accent_text: hex("#ffffff"),
            track: hex("#d4d4d4"),
            banner: hex("#dcebf8"),
            toast: hex("#2b2b2b"),
            toast_text: hex("#ffffff"),
            rec: hex("#c42b1c"),
            thumb: hex("#e6e6e6"),
        }
    }

    pub fn dark() -> Palette {
        Palette {
            dark: true,
            bg: hex("#202020"),
            card: hex("#2b2b2b"),
            border: hex("#1c1c1c"),
            sep: hex("#232323"),
            text: hex("#ffffff"),
            dim: hex("#c5c5c5"),
            hover: Color32::from_white_alpha(15),
            press: Color32::from_white_alpha(23),
            control: hex("#373737"),
            control_hover: hex("#3c3c3c"),
            control_border: hex("#434343"),
            accent: hex("#4cc2ff"),
            accent_hover: hex("#47b1e8"),
            accent_press: hex("#42a1d2"),
            accent_text: hex("#000000"),
            track: hex("#5a5a5a"),
            banner: hex("#1f3447"),
            toast: hex("#f2f2f2"),
            toast_text: hex("#1a1a1a"),
            rec: hex("#ff99a4"),
            thumb: hex("#3a3a3a"),
        }
    }

    pub fn from_dark(dark: bool) -> Palette {
        if dark {
            Palette::dark()
        } else {
            Palette::light()
        }
    }
}

pub const SEMIBOLD: &str = "semibold";

pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// Segoe UI like the Qt version asked for it, with the bundled font as a fallback
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts_def = FontDefinitions::default();
    fonts_def.font_data.insert("regular".into(), Arc::new(FontData::from_owned(fonts::regular())));
    fonts_def.font_data.insert(SEMIBOLD.into(), Arc::new(FontData::from_owned(fonts::semibold())));
    let prop = fonts_def.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "regular".into());
    let semi = fonts_def.families.entry(FontFamily::Name(SEMIBOLD.into())).or_default();
    semi.insert(0, SEMIBOLD.into());
    semi.push("regular".into());
    ctx.set_fonts(fonts_def);
}

/// text size in points → egui logical pixels at 96 dpi
pub fn pt(p: f32) -> f32 {
    p * 4.0 / 3.0
}

/// Base visuals so that stock widgets (text fields, scroll bars) fit in.
pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.control;
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent_text);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
    v.window_stroke = egui::Stroke::new(1.0, p.control_border);
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.interaction.tooltip_delay = 0.4;
    });
}

// ---------------------------------------------------------------- icons
// stroke icons in the style of Lucide (ISC); "C" is replaced by the colour
pub fn icon_body(name: &str) -> &'static str {
    match name {
        "record" => r#"<circle cx="12" cy="12" r="9" fill="none" stroke="C" stroke-width="2"/><circle cx="12" cy="12" r="4.5" fill="C"/>"#,
        "clips" => r#"<path d="m16 13 5.2 3.5a.5.5 0 0 0 .8-.4V7.9a.5.5 0 0 0-.8-.4L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#,
        "settings" => r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#,
        "folder" => r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#,
        "copy" => r#"<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#,
        "trash" => r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#,
        "refresh" => r#"<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>"#,
        "more" => r#"<circle cx="5" cy="12" r="1.3" fill="C"/><circle cx="12" cy="12" r="1.3" fill="C"/><circle cx="19" cy="12" r="1.3" fill="C"/>"#,
        "chevron" => r#"<path d="m6 9 6 6 6-6"/>"#,
        "chevron-up" => r#"<path d="m18 15-6-6-6 6"/>"#,
        "play" => r#"<path d="M7 4.5v15l12-7.5z" fill="C"/>"#,
        "scissors" => r#"<circle cx="6" cy="6" r="3"/><path d="M8.12 8.12 12 12"/><path d="M20 4 8.12 15.88"/><circle cx="6" cy="18" r="3"/><path d="M14.8 14.8 20 20"/>"#,
        "pencil" => r#"<path d="M21.17 6.81a1 1 0 0 0-3.99-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5z"/><path d="m15 5 4 4"/>"#,
        "flag" => r#"<path d="M4 22V4a1 1 0 0 1 .4-.8A6 6 0 0 1 8 2c3 0 5 2 7.33 2q2 0 3.67-.8a1 1 0 0 1 1 .8v11a1 1 0 0 1-.4.8A6 6 0 0 1 16 16c-3 0-5-2-8-2a6 6 0 0 0-4 1.53"/>"#,
        _ => "",
    }
}

fn css(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub fn svg(name: &str, color: Color32, width: f32) -> String {
    let body = icon_body(name).replace("\"C\"", &format!("\"{}\"", css(color)));
    let c = css(color);
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{c}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#)
}

/// Draw SVG text into a square of `px` pixels (premultiplied RGBA).
pub fn rasterize(svg: &str, px: u32) -> Option<ColorImage> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pm = resvg::tiny_skia::Pixmap::new(px, px)?;
    let k = px as f32 / tree.size().width().max(1.0);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data()))
}

thread_local! {
    static ICONS: RefCell<HashMap<(String, [u8; 4], u32), TextureHandle>> = RefCell::new(HashMap::new());
}

/// An icon as a texture, made once per name / colour / size (drawn at 3× so it stays sharp).
pub fn icon(ctx: &egui::Context, name: &str, color: Color32, size: f32, stroke: f32) -> TextureHandle {
    let px = (size * 3.0).round() as u32;
    let key = (name.to_string(), color.to_array(), px * 100 + (stroke * 10.0) as u32);
    ICONS.with(|c| {
        if let Some(t) = c.borrow().get(&key) {
            return t.clone();
        }
        let img = rasterize(&svg(name, color, stroke), px).unwrap_or_else(|| ColorImage::filled([1, 1], Color32::TRANSPARENT));
        let tex = ctx.load_texture(format!("icon-{name}"), img, TextureOptions::LINEAR);
        c.borrow_mut().insert(key, tex.clone());
        tex
    })
}
