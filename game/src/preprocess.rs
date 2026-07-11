//! Deterministic glyph preprocessing — a faithful Rust port of the Python
//! `preprocessing.py`, so the same ink produces the same 28x28 input the models
//! were validated against.
//!
//! Input is an "ink" plane: row-major `f32`, `1.0` = full stroke, `0.0` = paper.
//! (A rendered glyph's coverage is already ink, so no invert is needed.)

pub const CANVAS: usize = 28;
const INK_THRESHOLD: f32 = 0.035;
// MNIST channel mean/std, used by the case-sensitive EMNIST-62 model.
const MEAN: f32 = 0.1307;
const STD: f32 = 0.3081;

/// Per-model glyph target sizes (the drawn glyph is scaled so its long side is
/// this many px inside the 28x28 canvas) — matches the Python constants.
pub const GLYPH_PRIMARY: usize = 25;
pub const GLYPH_DIGIT: usize = 20;
pub const GLYPH_BALANCED: usize = 26;

#[derive(Debug)]
pub struct BlankDrawing;

/// Crop to the ink, scale so the long side == `glyph_size`, paste centered into a
/// 28x28 canvas, then shift by centre-of-mass so the glyph is centred like EMNIST.
pub fn crop_scale_center(
    ink: &[f32],
    w: usize,
    h: usize,
    glyph_size: usize,
) -> Result<[f32; CANVAS * CANVAS], BlankDrawing> {
    // Bounding box of meaningful ink.
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
    // 1px pad, clamped — matches Python's `min-1 .. max+2`.
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

    // Scale so the long side is `glyph_size`.
    let scale = glyph_size as f32 / cw.max(ch) as f32;
    let tw = ((cw as f32 * scale).round() as usize).max(1);
    let th = ((ch as f32 * scale).round() as usize).max(1);
    let resized = resize_area(&crop, cw, ch, tw, th);

    // Paste centered into the 28x28 canvas.
    let mut canvas = [0.0f32; CANVAS * CANVAS];
    let oy = (CANVAS - th) / 2;
    let ox = (CANVAS - tw) / 2;
    for y in 0..th {
        for x in 0..tw {
            canvas[(oy + y) * CANVAS + (ox + x)] = resized[y * tw + x];
        }
    }

    // (Deslanting was evaluated and removed: moment-based slant estimation
    // conflates shape asymmetry with slant, so it distorted upright asymmetric
    // glyphs like F/L/P — a net loss on the common upright case.)

    // Intensity normalize: a thin stroke anti-aliases to faint grey when scaled
    // down from a large drawing, so boost the darkest ink back to full strength
    // before measuring/thickening (capped so we don't amplify stray noise).
    let mx = canvas.iter().cloned().fold(0.0f32, f32::max);
    if mx > 1e-3 {
        let g = (1.0 / mx).min(4.0);
        for v in canvas.iter_mut() {
            *v = (*v * g).min(1.0);
        }
    }

    // Adaptive stroke thickening: the ink is captured thin so it preserves the
    // character shape at any size (a fat fixed stroke blobs a small glyph). Here
    // we normalize the stroke WIDTH in the 28x28 domain — dilate (3x3 grayscale
    // max) until coverage looks in-distribution (EMNIST inks ~60-150 of 784
    // cells). Capped so it never over-grows.
    let inked = |c: &[f32; CANVAS * CANVAS]| c.iter().filter(|v| **v > 0.4).count();
    let mut passes = 0;
    while inked(&canvas) < 60 && passes < 3 {
        canvas = dilate3(&canvas);
        passes += 1;
    }

    // Centre-of-mass shift (no wrap), as in Python.
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

/// Area-average (box) resampling — correct for both up- and down-scaling, and far
/// better than point/bilinear when shrinking a big render down to ~25px.
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

/// 3x3 grayscale max (morphological dilation) — thickens strokes by ~1px.
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

/// EMNIST-62 tensor: standardized, **transposed** (the EMNIST orientation fix),
/// NCHW `[1,1,28,28]`.
pub fn tensor_primary(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    let mut t = vec![0.0f32; CANVAS * CANVAS];
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            // transpose: t[x][y] = norm(mask[y][x])
            t[x * CANVAS + y] = (mask[y * CANVAS + x] - MEAN) / STD;
        }
    }
    t
}

/// MNIST-12 tensor: 0..255 scale, no transpose, NCHW `[1,1,28,28]`.
pub fn tensor_digit(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    mask.iter().map(|v| v * 255.0).collect()
}

/// EMNIST-balanced tensor: 0..1, no transpose, NHWC `[1,28,28,1]` (with C=1 the
/// flat row-major order is identical to the mask).
pub fn tensor_balanced(mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
    mask.to_vec()
}
