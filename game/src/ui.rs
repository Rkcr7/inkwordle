//! Minimal on-surface UI: text (via ab_glyph), bordered boxes, and tappable
//! buttons with hit-testing. No dependency on Muse's typeset stack.

use crate::surface::{Surface, BLACK, FADED, WHITE};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

pub const FONT_BYTES: &[u8] = include_bytes!("../assets/ui-font.ttf");

pub fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_BYTES).expect("ui font")
}

/// Draw a left-aligned string; `y` is the top of the text. Returns the end x.
pub fn text(surf: &mut Surface, font: &FontRef, x: i32, y: i32, px: f32, s: &str, color: u16) -> i32 {
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let ascent = scaled.ascent();
    let mut cx = x as f32;
    for ch in s.chars() {
        let gid = font.glyph_id(ch);
        let g = gid.with_scale_and_position(scale, ab_glyph::point(cx, y as f32 + ascent));
        if let Some(og) = font.outline_glyph(g) {
            let bb = og.px_bounds();
            og.draw(|gx, gy, c| {
                if c > 0.45 {
                    surf.put_px(bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32, color);
                }
            });
        }
        cx += scaled.h_advance(gid);
    }
    cx as i32
}

pub fn text_width(font: &FontRef, px: f32, s: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    s.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()
}

/// Centre a string horizontally on `cx`.
pub fn text_center(surf: &mut Surface, font: &FontRef, cx: i32, y: i32, px: f32, s: &str, color: u16) {
    let w = text_width(font, px, s);
    text(surf, font, cx - (w / 2.0) as i32, y, px, s, color);
}

/// A `t`-px border around a rectangle.
pub fn border(surf: &mut Surface, x: i32, y: i32, w: i32, h: i32, t: i32, color: u16) {
    let (x, y, w, h, t) = (x.max(0) as usize, y.max(0) as usize, w.max(0) as usize, h.max(0) as usize, t.max(1) as usize);
    surf.fill_rect(x, y, w, t, color);
    surf.fill_rect(x, y + h - t, w, t, color);
    surf.fill_rect(x, y, t, h, color);
    surf.fill_rect(x + w - t, y, t, h, color);
}

pub struct Button {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub label: String,
}

impl Button {
    pub fn new(x: i32, y: i32, w: i32, h: i32, label: &str) -> Self {
        Button { x, y, w, h, label: label.to_string() }
    }

    pub fn hit(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Filled = a solid dark button with light text (the primary action).
    pub fn draw(&self, surf: &mut Surface, font: &FontRef, filled: bool) {
        if filled {
            surf.fill_rect(self.x as usize, self.y as usize, self.w as usize, self.h as usize, BLACK);
        } else {
            surf.fill_rect(self.x as usize, self.y as usize, self.w as usize, self.h as usize, WHITE);
            border(surf, self.x, self.y, self.w, self.h, 3, FADED);
        }
        let px = (self.h as f32 * 0.42).round();
        let ty = self.y + (self.h - px as i32) / 2 - (px * 0.12) as i32;
        text_center(surf, font, self.x + self.w / 2, ty, px, &self.label, if filled { WHITE } else { BLACK });
    }
}
