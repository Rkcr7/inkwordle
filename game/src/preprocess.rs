//! Deterministic glyph preprocessing — faithful to the validated EMNIST pipeline.
//! Hot path accepts dense u8 ink (0=paper, 255=stroke) to avoid f32 capture cost.

pub const CANVAS: usize = 28;
/// Ink threshold on 0..1 scale (legacy / float path).
const INK_THRESHOLD: f32 = 0.035;
/// Same threshold on u8 capture: ceil(0.035 * 255) ≈ 9.
const INK_THRESHOLD_U8: u8 = 9;

const MEAN: f32 = 0.1307;
const STD: f32 = 0.3081;

pub const GLYPH_PRIMARY: usize = 25;
pub const GLYPH_DIGIT: usize = 20;
pub const GLYPH_BALANCED: usize = 26;

#[derive(Debug)]
pub struct BlankDrawing;

/// Preprocess from u8 ink plane (row-major, 255 = full stroke). Preferred hot path.
pub fn crop_scale_center_u8(
    ink: &[u8],
    w: usize,
    h: usize,
    glyph_size: usize,
) -> Result<[f32; CANVAS * CANVAS], BlankDrawing> {
    debug_assert_eq!(ink.len(), w * h);
    let (mut top, mut left) = (usize::MAX, usize::MAX);
    let (mut bottom, mut right) = (0usize, 0usize);
    let mut total: u32 = 0;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let v = ink[row + x];
            total += v as u32;
            if v > INK_THRESHOLD_U8 {
                top = top.min(y);
                bottom = bottom.max(y);
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    // ~0.5 on 0..1 scale over many pixels ≈ total < 128 as a blank guard
    if top == usize::MAX || total < 128 {
        return Err(BlankDrawing);
    }
    let top = top.saturating_sub(1);
    let left = left.saturating_sub(1);
    let bottom = (bottom + 2).min(h);
    let right = (right + 2).min(w);
    let (cw, ch) = (right - left, bottom - top);

    // Convert only the crop to f32 once.
    let mut crop = vec![0.0f32; cw * ch];
    for y in 0..ch {
        let src = (top + y) * w + left;
        let dst = y * cw;
        for x in 0..cw {
            crop[dst + x] = ink[src + x] as f32 * (1.0 / 255.0);
        }
    }
    finish_mask(&crop, cw, ch, glyph_size)
}

/// Legacy f32 path (tests / debug).
pub fn crop_scale_center(
    ink: &[f32],
    w: usize,
    h: usize,
    glyph_size: usize,
) -> Result<[f32; CANVAS * CANVAS], BlankDrawing> {
    let (mut top, mut left) = (usize::MAX, usize::MAX);
    let (mut bottom, mut right) = (0usize, 0usize);
    let mut total = 0.0f32;
    for y in 0..h {
        for x in 0..w {
            let v = ink[y * w + x];
            total += v;
            if v > INK_THRESHOLD {
                top = top.min(y);
                bottom = bottom.max(y);
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    if top == usize::MAX || total < 0.5 {
        return Err(BlankDrawing);
    }
    let top = top.saturating_sub(1);
    let left = left.saturating_sub(1);
    let bottom = (bottom + 2).min(h);
    let right = (right + 2).min(w);
    let (cw, ch) = (right - left, bottom - top);
    let mut crop = vec![0.0f32; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            crop[y * cw + x] = ink[(top + y) * w + (left + x)];
        }
    }
    finish_mask(&crop, cw, ch, glyph_size)
}

fn finish_mask(
    crop: &[f32],
    cw: usize,
    ch: usize,
    glyph_size: usize,
) -> Result<[f32; CANVAS * CANVAS], BlankDrawing> {
    let scale = glyph_size as f32 / cw.max(ch) as f32;
    let tw = ((cw as f32 * scale).round() as usize).max(1);
    let th = ((ch as f32 * scale).round() as usize).max(1);
    let resized = resize_area(crop, cw, ch, tw, th);

    let mut canvas = [0.0f32; CANVAS * CANVAS];
    let oy = (CANVAS - th) / 2;
    let ox = (CANVAS - tw) / 2;
    for y in 0..th {
        for x in 0..tw {
            canvas[(oy + y) * CANVAS + (ox + x)] = resized[y * tw + x];
        }
    }

    let mx = canvas.iter().cloned().fold(0.0f32, f32::max);
    if mx > 1e-3 {
        let g = (1.0 / mx).min(4.0);
        for v in canvas.iter_mut() {
            *v = (*v * g).min(1.0);
        }
    }

    let inked = |c: &[f32; CANVAS * CANVAS]| c.iter().filter(|v| **v > 0.4).count();
    let mut passes = 0;
    while inked(&canvas) < 60 && passes < 3 {
        canvas = dilate3(&canvas);
        passes += 1;
    }

    let mut sum = 0.0f32;
    let mut cy = 0.0f32;
    let mut cx = 0.0f32;
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let v = canvas[y * CANVAS + x];
            sum += v;
            cy += v * y as f32;
            cx += v * x as f32;
        }
    }
    if sum <= 0.0 {
        return Err(BlankDrawing);
    }
    let mid = (CANVAS as f32 - 1.0) / 2.0;
    let shift_y = (mid - cy / sum).round() as isize;
    let shift_x = (mid - cx / sum).round() as isize;
    Ok(translate_no_wrap(&canvas, shift_y, shift_x))
}

fn resize_area(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut dst = vec![0.0f32; dw * dh];
    let rx = sw as f32 / dw as f32;
    let ry = sh as f32 / dh as f32;
    for dy in 0..dh {
        let (y0, y1) = (dy as f32 * ry, (dy as f32 + 1.0) * ry);
        for dx in 0..dw {
            let (x0, x1) = (dx as f32 * rx, (dx as f32 + 1.0) * rx);
            let mut acc = 0.0f32;
            let mut wsum = 0.0f32;
            for yy in (y0.floor() as usize)..(y1.ceil() as usize).min(sh) {
                let wy = ((yy as f32 + 1.0).min(y1) - (yy as f32).max(y0)).max(0.0);
                for xx in (x0.floor() as usize)..(x1.ceil() as usize).min(sw) {
                    let wx = ((xx as f32 + 1.0).min(x1) - (xx as f32).max(x0)).max(0.0);
                    let wgt = wx * wy;
                    acc += src[yy * sw + xx] * wgt;
                    wsum += wgt;
                }
            }
            dst[dy * dw + dx] = if wsum > 0.0 { acc / wsum } else { 0.0 };
        }
    }
    dst
}

fn dilate3(a: &[f32; CANVAS * CANVAS]) -> [f32; CANVAS * CANVAS] {
    let mut o = [0.0f32; CANVAS * CANVAS];
    for y in 0..CANVAS as i32 {
        for x in 0..CANVAS as i32 {
            let mut m = 0.0f32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && nx < CANVAS as i32 && ny >= 0 && ny < CANVAS as i32 {
                        m = m.max(a[(ny * CANVAS as i32 + nx) as usize]);
                    }
                }
            }
            o[(y * CANVAS as i32 + x) as usize] = m;
        }
    }
    o
}

fn translate_no_wrap(a: &[f32], sy: isize, sx: isize) -> [f32; CANVAS * CANVAS] {
    let mut out = [0.0f32; CANVAS * CANVAS];
    for y in 0..CANVAS {
        let syy = y as isize + sy;
        if syy < 0 || syy >= CANVAS as isize {
            continue;
        }
        for x in 0..CANVAS {
            let sxx = x as isize + sx;
            if sxx < 0 || sxx >= CANVAS as isize {
                continue;
            }
            out[syy as usize * CANVAS + sxx as usize] = a[y * CANVAS + x];
        }
    }
    out
}

/// Write EMNIST-62 tensor into `out` (must be len >= 784). NCHW, transposed, standardized.
pub fn tensor_primary_into(mask: &[f32; CANVAS * CANVAS], out: &mut [f32]) {
    debug_assert!(out.len() >= CANVAS * CANVAS);
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            out[x * CANVAS + y] = (mask[y * CANVAS + x] - MEAN) / STD;
        }
    }
}

pub fn tensor_primary(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    let mut t = vec![0.0f32; CANVAS * CANVAS];
    tensor_primary_into(mask, &mut t);
    t
}

pub fn tensor_digit(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    mask.iter().map(|v| v * 255.0).collect()
}

pub fn tensor_balanced(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    mask.to_vec()
}
