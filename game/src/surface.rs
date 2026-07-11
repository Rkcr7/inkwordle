//! Drawing surface abstraction: same drawing code renders into either the
//! qtfb RGB565 shared memory (in-xochitl backend) or the vendor engine's
//! RGB32 aux framebuffer (takeover backend). Colors are RGB565 u16 at the
//! API; the surface converts on write.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PixFmt {
    /// 2 bytes/px, little-endian RGB565 (qtfb FBFMT_RMPP_RGB565).
    Rgb565,
    /// 4 bytes/px, QImage Format_RGB32: bytes B,G,R,0xFF.
    Rgb32,
}

pub struct Surface {
    ptr: *mut u8,
    len: usize,
    pub w: usize,
    pub h: usize,
    pub stride: usize,
    pub fmt: PixFmt,
}

// Single-threaded writer over a long-lived mapping.
unsafe impl Send for Surface {}

pub const WHITE: u16 = 0xFFFF;
pub const BLACK: u16 = 0x0000;
/// Old ink: how the diary writes its memories (a readable e-ink gray).
pub const FADED: u16 = 0x7BCF;

// ---------------------------------------------------------------------------
// The Gallery 3 palette, derived (not guessed) from reMarkable's own ICC profile.
//
// The panel is SUBTRACTIVE cyan/magenta/yellow/white particle ink. It reaches
// ~20,000 colours, but only by DITHERING over an AREA. A thin stroke or a small
// glyph has no room to halftone, so ink lands on a native colour — which is why
// the stock app offers just nine pens.
//
// Two numbers decide whether a colour works, and both come from soft-proofing
// the input RGB through the profile:
//   * chroma  — how colourful it actually appears (the panel is very muted)
//   * contrast against white paper = 100 - L
// "Pop" is the product. Measured:
//        chroma  contrast   pop
//   blue   45.9      53.6  24.6   <- by far the strongest colour on this panel
//   violet 33.7      37.5  12.7
//   cyan   28.6      35.9  10.3
//   magenta 25.9     31.4   8.2
//   rose   21.9      32.1   7.0
//   green  17.9      27.3   4.9
//   orange 18.6      16.9   3.1   <- FILL ONLY
//   yellow 26.5       5.0   1.3   <- vivid, but L=95: invisible as a thin line
//
// So YELLOW and ORANGE are genuinely colourful yet have almost no contrast on
// white: superb as fills, useless as text, lines or arrows. Everything else is
// pairwise >= dE 12 apart, so the reader can always tell two roles apart.
// ---------------------------------------------------------------------------
pub const BLUE: u16 = 0x001F; // #0000ff -> #4d6bb8  pop 24.6 (strongest)
pub const VIOLET: u16 = 0x797F; // #782dff -> #9491cd  pop 12.7
pub const CYAN: u16 = 0x3C3F; // #3c87ff -> #7d9dce  pop 10.3
pub const MAGENTA: u16 = 0xD35F; // #d269ff -> #ba9dc8  pop 8.2
pub const ROSE: u16 = 0xF810; // #ff0087 -> #c898ae  pop 7.0 (the warm accent)
pub const GREEN: u16 = 0x07E0; // #00ff00 -> #abb797  pop 4.9
pub const YELLOW: u16 = 0xF660; // FILL ONLY — vivid (C 26.5) but L 95
pub const ORANGE: u16 = 0xFAC0; // FILL ONLY — L 83; web orange (#FFA500) reads as yellow

// Legacy names kept as aliases so callers keep compiling; each points at the
// nearest colour the panel can actually resolve.
pub const RED: u16 = ROSE; // true red soft-proofs to a pale mauve (L 77) — use rose
pub const PURPLE: u16 = MAGENTA;
pub const INDIGO: u16 = VIOLET;
pub const TEAL: u16 = CYAN; // old 0x0567 "teal" was indistinguishable from gray
pub const CRIMSON: u16 = ROSE;
pub const DARKGREEN: u16 = GREEN; // dark greens collapse to gray (chroma 10.5)
pub const BULLET: u16 = BLUE; // list markers — the panel's strongest hue
pub const SEPIA: u16 = ROSE; // your own words: the only warm the panel renders

#[inline]
fn expand565(c: u16) -> (u8, u8, u8) {
    let r = ((c >> 11) & 0x1f) as u32;
    let g = ((c >> 5) & 0x3f) as u32;
    let b = (c & 0x1f) as u32;
    (
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    )
}

impl Surface {
    pub fn new(ptr: *mut u8, len: usize, w: usize, h: usize, stride: usize, fmt: PixFmt) -> Self {
        Self { ptr, len, w, h, stride, fmt }
    }

    #[inline]
    fn buf(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    #[inline]
    fn buf_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    #[inline]
    pub fn put_px(&mut self, x: i32, y: i32, c: u16) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let (stride, fmt) = (self.stride, self.fmt);
        match fmt {
            PixFmt::Rgb565 => {
                let i = y as usize * stride + x as usize * 2;
                let b = self.buf();
                b[i] = (c & 0xff) as u8;
                b[i + 1] = (c >> 8) as u8;
            }
            PixFmt::Rgb32 => {
                let (r, g, bl) = expand565(c);
                let i = y as usize * stride + x as usize * 4;
                let b = self.buf();
                b[i] = bl;
                b[i + 1] = g;
                b[i + 2] = r;
                b[i + 3] = 0xFF;
            }
        }
    }

    /// Luminance 0..255 — used by the PNG rasterizer and dissolve inkness test.
    #[inline]
    pub fn luma(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return 255;
        }
        let b = self.buf_ref();
        match self.fmt {
            PixFmt::Rgb565 => {
                let i = y as usize * self.stride + x as usize * 2;
                let px = (b[i] as u16) | ((b[i + 1] as u16) << 8);
                (((px >> 5) & 0x3f) as u32 * 255 / 63) as u8
            }
            PixFmt::Rgb32 => {
                let i = y as usize * self.stride + x as usize * 4;
                // Green approximates luma well enough for mono ink.
                b[i + 1]
            }
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: u16) {
        let x1 = (x + w).min(self.w);
        let y1 = (y + h).min(self.h);
        if x >= x1 || y >= y1 {
            return;
        }
        // Hoist the format branch + color conversion out of the pixel loops and
        // write directly into each row (no per-pixel bounds check — clamped).
        let (stride, fmt) = (self.stride, self.fmt);
        let buf = self.buf();
        match fmt {
            PixFmt::Rgb565 => {
                let (lo, hi) = ((c & 0xff) as u8, (c >> 8) as u8);
                for row in y..y1 {
                    let base = row * stride;
                    for col in x..x1 {
                        let i = base + col * 2;
                        buf[i] = lo;
                        buf[i + 1] = hi;
                    }
                }
            }
            PixFmt::Rgb32 => {
                let (r, g, bl) = expand565(c);
                for row in y..y1 {
                    let base = row * stride;
                    for col in x..x1 {
                        let i = base + col * 4;
                        buf[i] = bl;
                        buf[i + 1] = g;
                        buf[i + 2] = r;
                        buf[i + 3] = 0xFF;
                    }
                }
            }
        }
    }

    /// Blit a 1-byte-per-pixel coverage mask (`true` = inked) at (x, y) in one
    /// color; `embolden` double-strikes one pixel right (synthetic bold). Clips
    /// to the surface, borrows the buffer once, and hoists the pixel-format
    /// branch and color conversion out of the loop — the hot path for all text.
    pub fn blit_mask(
        &mut self,
        mask: &[bool],
        mw: usize,
        mh: usize,
        x: i32,
        y: i32,
        color: u16,
        embolden: bool,
    ) {
        if mw == 0 || mh == 0 {
            return;
        }
        let (sw, sh) = (self.w as i32, self.h as i32);
        let stride = self.stride;
        match self.fmt {
            PixFmt::Rgb565 => {
                let (lo, hi) = ((color & 0xff) as u8, (color >> 8) as u8);
                let buf = self.buf();
                for row in 0..mh {
                    let py = y + row as i32;
                    if py < 0 || py >= sh {
                        continue;
                    }
                    let base = py as usize * stride;
                    let mrow = row * mw;
                    for col in 0..mw {
                        if !mask[mrow + col] {
                            continue;
                        }
                        let px = x + col as i32;
                        if px >= 0 && px < sw {
                            let i = base + px as usize * 2;
                            buf[i] = lo;
                            buf[i + 1] = hi;
                        }
                        if embolden {
                            let e = px + 1;
                            if e >= 0 && e < sw {
                                let i = base + e as usize * 2;
                                buf[i] = lo;
                                buf[i + 1] = hi;
                            }
                        }
                    }
                }
            }
            PixFmt::Rgb32 => {
                let (r, g, bl) = expand565(color);
                let buf = self.buf();
                for row in 0..mh {
                    let py = y + row as i32;
                    if py < 0 || py >= sh {
                        continue;
                    }
                    let base = py as usize * stride;
                    let mrow = row * mw;
                    for col in 0..mw {
                        if !mask[mrow + col] {
                            continue;
                        }
                        let px = x + col as i32;
                        if px >= 0 && px < sw {
                            let i = base + px as usize * 4;
                            buf[i] = bl;
                            buf[i + 1] = g;
                            buf[i + 2] = r;
                            buf[i + 3] = 0xFF;
                        }
                        if embolden {
                            let e = px + 1;
                            if e >= 0 && e < sw {
                                let i = base + e as usize * 4;
                                buf[i] = bl;
                                buf[i + 1] = g;
                                buf[i + 2] = r;
                                buf[i + 3] = 0xFF;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Invert the RGB of a rect (cursor/pressed-key feedback).
    pub fn invert_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let x1 = (x + w).min(self.w);
        let y1 = (y + h).min(self.h);
        let (stride, fmt) = (self.stride, self.fmt);
        let buf = self.buf();
        for row in y..y1 {
            match fmt {
                PixFmt::Rgb565 => {
                    let s = row * stride + x * 2;
                    let e = row * stride + x1 * 2;
                    for b in &mut buf[s..e] {
                        *b = !*b;
                    }
                }
                PixFmt::Rgb32 => {
                    for col in x..x1 {
                        let i = row * stride + col * 4;
                        buf[i] = !buf[i];
                        buf[i + 1] = !buf[i + 1];
                        buf[i + 2] = !buf[i + 2];
                    }
                }
            }
        }
    }

    #[inline]
    pub fn bpp(&self) -> usize {
        match self.fmt {
            PixFmt::Rgb565 => 2,
            PixFmt::Rgb32 => 4,
        }
    }

    /// Snapshot a rect's raw bytes (for save-under panels).
    pub fn copy_rect(&self, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
        let bpp = self.bpp();
        let b = self.buf_ref();
        let mut out = Vec::with_capacity((x1 - x) * (y1 - y) * bpp);
        for row in y..y1 {
            let s = row * self.stride + x * bpp;
            out.extend_from_slice(&b[s..s + (x1 - x) * bpp]);
        }
        out
    }

    /// Put back bytes captured by `copy_rect` with the same geometry.
    pub fn paste_rect(&mut self, x: usize, y: usize, w: usize, h: usize, data: &[u8]) {
        let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
        if x >= x1 || y >= y1 {
            return;
        }
        let (bpp, stride) = (self.bpp(), self.stride);
        let row_len = (x1 - x) * bpp;
        // Refuse a geometry mismatch rather than panic/corrupt: `data` must hold
        // at least one full row per destination row.
        if data.len() < (y1 - y) * row_len {
            return;
        }
        let b = self.buf();
        for (i, row) in (y..y1).enumerate() {
            let s = row * stride + x * bpp;
            b[s..s + row_len].copy_from_slice(&data[i * row_len..(i + 1) * row_len]);
        }
    }

    pub fn stamp(&mut self, cx: i32, cy: i32, r: i32, c: u16) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    self.put_px(cx + dx, cy + dy, c);
                }
            }
        }
    }

    pub fn brush_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, c: u16) {
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let steps = dx.max(dy).max(1);
        for i in 0..=steps {
            let x = x0 + (x1 - x0) * i / steps;
            let y = y0 + (y1 - y0) * i / steps;
            self.stamp(x, y, r, c);
        }
    }
}
