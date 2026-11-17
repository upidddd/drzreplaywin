//! A tiny software canvas: rounded rectangles, circles and text, for the corner badge.
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    /// straight-alpha RGBA, f32 (colour 0..255, alpha 0..1)
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, px: vec![[0.0; 4]; w * h] }
    }

    fn over(&mut self, x: i64, y: i64, rgb: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h || a <= 0.0 {
            return;
        }
        let d = &mut self.px[y as usize * self.w + x as usize];
        let out = a + d[3] * (1.0 - a);
        if out <= 0.0 {
            return;
        }
        for i in 0..3 {
            d[i] = (rgb[i] * a + d[i] * d[3] * (1.0 - a)) / out;
        }
        d[3] = out;
    }

    /// signed distance to a rounded rectangle → coverage
    fn rr_cover(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let (qx, qy) = ((x - cx).abs() - (hx - r), (y - cy).abs() - (hy - r));
        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
        (0.5 - d).clamp(0.0, 1.0)
    }

    pub fn round_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, rgb: [f32; 3], alpha: f32) {
        for y in (y0.floor() as i64).max(0)..(y1.ceil() as i64).min(self.h as i64) {
            for x in (x0.floor() as i64).max(0)..(x1.ceil() as i64).min(self.w as i64) {
                let c = Self::rr_cover(x as f32 + 0.5, y as f32 + 0.5, x0, y0, x1, y1, r);
                self.over(x, y, rgb, c * alpha);
            }
        }
    }

    /// the 1 px outline of a rounded rectangle (inside the given bounds)
    pub fn round_rect_outline(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, width: f32, rgb: [f32; 3], alpha: f32) {
        for y in (y0.floor() as i64).max(0)..(y1.ceil() as i64).min(self.h as i64) {
            for x in (x0.floor() as i64).max(0)..(x1.ceil() as i64).min(self.w as i64) {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let outer = Self::rr_cover(fx, fy, x0, y0, x1, y1, r);
                let inner = Self::rr_cover(fx, fy, x0 + width, y0 + width, x1 - width, y1 - width, (r - width).max(0.0));
                self.over(x, y, rgb, (outer - inner).max(0.0) * alpha);
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, rgb: [f32; 3]) {
        self.round_rect(cx - r, cy - r, cx + r, cy + r, r, rgb, 1.0);
    }

    pub fn text_width(font: &FontRef, size_px: f32, s: &str) -> f32 {
        let scale = PxScale::from(size_px * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0));
        let sf = font.as_scaled(scale);
        let mut x = 0.0;
        let mut prev = None;
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            x += sf.h_advance(id);
            prev = Some(id);
        }
        x
    }

    /// text with its vertical centre at `y_mid`
    pub fn text(&mut self, font: &FontRef, x: f32, y_mid: f32, size_px: f32, s: &str, rgb: [f32; 3]) {
        let scale = PxScale::from(size_px * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0));
        let sf = font.as_scaled(scale);
        let top = y_mid - (sf.ascent() + sf.descent()) / 2.0;
        let (mut cx, mut prev) = (x, None);
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                cx += sf.kern(p, id);
            }
            if let Some(o) = font.outline_glyph(id.with_scale_and_position(scale, ab_glyph::point(cx, top + sf.ascent()))) {
                let b = o.px_bounds();
                o.draw(|gx, gy, c| self.over(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, rgb, c));
            }
            cx += sf.h_advance(id);
            prev = Some(id);
        }
    }

    /// premultiplied BGRA, what a layered window wants
    pub fn to_bgra_premultiplied(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for p in &self.px {
            let a = p[3];
            out.extend_from_slice(&[(p[2] * a).round() as u8, (p[1] * a).round() as u8, (p[0] * a).round() as u8, (a * 255.0).round() as u8]);
        }
        out
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for p in &self.px {
            out.extend_from_slice(&[p[0].round() as u8, p[1].round() as u8, p[2].round() as u8, (p[3] * 255.0).round() as u8]);
        }
        out
    }
}
