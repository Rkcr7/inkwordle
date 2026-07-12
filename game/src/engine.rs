//! On-device handwritten-character recognition (tract + EMNIST-62).
//! Speed fork: reusable scratch buffers, u8 ink entry, no ensemble.

use std::path::Path;
use tract_onnx::prelude::*;

use crate::preprocess::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Digit,
    Balanced,
}

impl Kind {
    fn labels(self) -> Vec<char> {
        match self {
            Kind::Primary => "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
                .chars()
                .collect(),
            Kind::Digit => "0123456789".chars().collect(),
            Kind::Balanced => [
                48u32, 49, 50, 51, 52, 53, 54, 55, 56, 57, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
                75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 97, 98, 100, 101,
                102, 103, 104, 110, 113, 114, 116,
            ]
            .iter()
            .map(|c| char::from_u32(*c).unwrap())
            .collect(),
        }
    }
    fn glyph_size(self) -> usize {
        match self {
            Kind::Primary => GLYPH_PRIMARY,
            Kind::Digit => GLYPH_DIGIT,
            Kind::Balanced => GLYPH_BALANCED,
        }
    }
    fn input_shape(self) -> [usize; 4] {
        match self {
            Kind::Balanced => [1, CANVAS, CANVAS, 1],
            _ => [1, 1, CANVAS, CANVAS],
        }
    }
    fn tensor(self, mask: &[f32; CANVAS * CANVAS]) -> Vec<f32> {
        match self {
            Kind::Primary => tensor_primary(mask),
            Kind::Digit => tensor_digit(mask),
            Kind::Balanced => tensor_balanced(mask),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::Primary => "emnist-62",
            Kind::Digit => "mnist-12",
            Kind::Balanced => "balanced-47",
        }
    }
}

fn to_probs(out: &[f32]) -> Vec<f32> {
    let min = out.iter().cloned().fold(f32::MAX, f32::min);
    let sum: f32 = out.iter().sum();
    if min >= -1e-4 && (sum - 1.0).abs() < 0.05 {
        out.to_vec()
    } else {
        let max = out.iter().cloned().fold(f32::MIN, f32::max);
        let exp: Vec<f32> = out.iter().map(|v| (v - max).exp()).collect();
        let s: f32 = exp.iter().sum();
        exp.iter().map(|e| e / s).collect()
    }
}

/// Softmax into `out` (len == logits len) without allocating when possible.
fn to_probs_into(logits: &[f32], out: &mut [f32]) {
    debug_assert_eq!(logits.len(), out.len());
    let min = logits.iter().cloned().fold(f32::MAX, f32::min);
    let sum: f32 = logits.iter().sum();
    if min >= -1e-4 && (sum - 1.0).abs() < 0.05 {
        out.copy_from_slice(logits);
        return;
    }
    let max = logits.iter().cloned().fold(f32::MIN, f32::max);
    let mut s = 0.0f32;
    for (i, &v) in logits.iter().enumerate() {
        let e = (v - max).exp();
        out[i] = e;
        s += e;
    }
    let inv = if s > 0.0 { 1.0 / s } else { 0.0 };
    for v in out.iter_mut() {
        *v *= inv;
    }
}

type Plan = RunnableModel<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct Model {
    pub kind: Kind,
    plan: Plan,
    labels: Vec<char>,
}

#[derive(Clone)]
pub struct Prediction {
    pub ch: char,
    pub confidence: f32,
    pub top: Vec<(char, f32)>,
    pub latency_ms: f32,
}

/// Reused across cells / frames to avoid heap churn on the HWR hot path.
pub struct PredictScratch {
    tensor: Vec<f32>,
    logits: Vec<f32>,
    probs: Vec<f32>,
}

impl PredictScratch {
    pub fn new() -> Self {
        Self {
            tensor: vec![0.0; CANVAS * CANVAS],
            logits: Vec::with_capacity(62),
            probs: vec![0.0; 62],
        }
    }
}

impl Model {
    pub fn load(kind: Kind, path: &Path) -> TractResult<Self> {
        let s = kind.input_shape();
        let plan = tract_onnx::onnx()
            .model_for_path(path)?
            .with_input_fact(0, f32::fact([s[0], s[1], s[2], s[3]]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Model {
            kind,
            plan,
            labels: kind.labels(),
        })
    }

    pub fn predict(&self, ink: &[f32], w: usize, h: usize) -> Result<Prediction, String> {
        Ok(self.predict_debug(ink, w, h)?.0)
    }

    /// Wordle path: u8 ink + scratch. Reuses tensor buffer via take/restore
    /// (avoids a second 784-float alloc per call).
    pub fn predict_letters_u8(
        &self,
        ink: &[u8],
        w: usize,
        h: usize,
        scratch: &mut PredictScratch,
    ) -> Result<Vec<(char, f32)>, String> {
        let mask = crop_scale_center_u8(ink, w, h, self.kind.glyph_size())
            .map_err(|_| "blank".to_string())?;
        if scratch.tensor.len() < CANVAS * CANVAS {
            scratch.tensor.resize(CANVAS * CANVAS, 0.0);
        }
        tensor_primary_into(&mask, &mut scratch.tensor[..CANVAS * CANVAS]);
        let s = self.kind.input_shape();
        // Move tensor into tract; restore an empty capacity-ready buffer after.
        let mut data = std::mem::take(&mut scratch.tensor);
        data.truncate(CANVAS * CANVAS);
        if data.len() < CANVAS * CANVAS {
            data.resize(CANVAS * CANVAS, 0.0);
        }
        let input: Tensor = match tract_ndarray::Array::from_shape_vec(s.to_vec(), data) {
            Ok(a) => a.into(),
            Err(e) => {
                scratch.tensor = vec![0.0; CANVAS * CANVAS];
                return Err(e.to_string());
            }
        };
        let out = match self.plan.run(tvec!(input.into())) {
            Ok(o) => o,
            Err(e) => {
                scratch.tensor = vec![0.0; CANVAS * CANVAS];
                return Err(e.to_string());
            }
        };
        // Reclaim a reusable tensor buffer for next cell.
        scratch.tensor = vec![0.0; CANVAS * CANVAS];

        let view = out[0].to_array_view::<f32>().map_err(|e| e.to_string())?;
        let slice = view
            .as_slice()
            .ok_or_else(|| "model output not contiguous".to_string())?;
        if slice.len() != 62 {
            return Err("letter recognition needs the 62-class model".into());
        }
        scratch.logits.resize(62, 0.0);
        // SAFETY: both sides are 62 f32; slice is valid for read.
        unsafe {
            core::ptr::copy_nonoverlapping(slice.as_ptr(), scratch.logits.as_mut_ptr(), 62);
        }
        if scratch.probs.len() < 62 {
            scratch.probs.resize(62, 0.0);
        }
        to_probs_into(&scratch.logits, &mut scratch.probs[..62]);
        let probs = &scratch.probs[..62];
        let mut letter = [0.0f32; 26];
        for i in 0..26 {
            // SAFETY: probs len 62; indices 10..36 and 36..62 are in range.
            unsafe {
                letter[i] = *probs.get_unchecked(10 + i) + *probs.get_unchecked(36 + i);
            }
        }
        let tot: f32 = letter.iter().sum();
        let inv = if tot > 0.0 { 1.0 / tot } else { 0.0 };
        let mut scored: [(char, f32); 26] = [('a', 0.0); 26];
        for i in 0..26 {
            scored[i] = ((b'a' + i as u8) as char, letter[i] * inv);
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok(scored.to_vec())
    }

    /// Warm the graph so the first real letter doesn't pay cold costs.
    pub fn warmup(&self, scratch: &mut PredictScratch) {
        let ink = vec![0u8; 200 * 200];
        // A thick blob in the middle so preprocess doesn't blank-out.
        let mut ink = ink;
        for y in 60..140 {
            for x in 60..140 {
                ink[y * 200 + x] = 255;
            }
        }
        let _ = self.predict_letters_u8(&ink, 200, 200, scratch);
    }

    pub fn predict_letters(
        &self,
        ink: &[f32],
        w: usize,
        h: usize,
    ) -> Result<Vec<(char, f32)>, String> {
        let mask = crop_scale_center(ink, w, h, self.kind.glyph_size())
            .map_err(|_| "blank".to_string())?;
        let data = self.kind.tensor(&mask);
        let s = self.kind.input_shape();
        let input: Tensor = tract_ndarray::Array::from_shape_vec(s.to_vec(), data)
            .map_err(|e| e.to_string())?
            .into();
        let out = self.plan.run(tvec!(input.into())).map_err(|e| e.to_string())?;
        let logits: Vec<f32> = out[0]
            .to_array_view::<f32>()
            .map_err(|e| e.to_string())?
            .iter()
            .copied()
            .collect();
        if logits.len() != 62 {
            return Err("letter recognition needs the 62-class model".into());
        }
        let probs = to_probs(&logits);
        let mut letter = [0.0f32; 26];
        for i in 0..26 {
            letter[i] = probs[10 + i] + probs[36 + i];
        }
        let tot: f32 = letter.iter().sum();
        let mut scored: Vec<(char, f32)> = (0..26)
            .map(|i| {
                (
                    (b'a' + i as u8) as char,
                    if tot > 0.0 { letter[i] / tot } else { 0.0 },
                )
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok(scored)
    }

    pub fn predict_debug(
        &self,
        ink: &[f32],
        w: usize,
        h: usize,
    ) -> Result<(Prediction, [f32; CANVAS * CANVAS]), String> {
        let mask = crop_scale_center(ink, w, h, self.kind.glyph_size())
            .map_err(|_| "blank drawing".to_string())?;
        let data = self.kind.tensor(&mask);
        let s = self.kind.input_shape();
        let input: Tensor = tract_ndarray::Array::from_shape_vec(s.to_vec(), data)
            .map_err(|e| e.to_string())?
            .into();
        let t0 = std::time::Instant::now();
        let out = self.plan.run(tvec!(input.into())).map_err(|e| e.to_string())?;
        let latency_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let logits: Vec<f32> = out[0]
            .to_array_view::<f32>()
            .map_err(|e| e.to_string())?
            .iter()
            .copied()
            .collect();
        Ok((self.decode(&logits, latency_ms), mask))
    }

    fn decode(&self, out: &[f32], latency_ms: f32) -> Prediction {
        let probs = to_probs(out);
        let mut scored: Vec<(char, f32)> = self
            .labels
            .iter()
            .zip(probs.iter())
            .map(|(&c, &p)| (c, p))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let (ch, confidence) = scored[0];
        Prediction {
            ch,
            confidence,
            top: scored.into_iter().take(3).collect(),
            latency_ms,
        }
    }
}

pub struct Engine {
    pub models: Vec<Model>,
}

impl Engine {
    pub fn load_dir(dir: &Path) -> Self {
        let specs = [
            (Kind::Balanced, "emnist-balanced-47.onnx"),
            (Kind::Digit, "mnist-12-int8.onnx"),
            (Kind::Primary, "emnist-62.onnx"),
        ];
        let mut models = Vec::new();
        for (kind, file) in specs {
            match Model::load(kind, &dir.join(file)) {
                Ok(m) => models.push(m),
                Err(e) => eprintln!("  [skip] {} could not load: {e}", kind.name()),
            }
        }
        Engine { models }
    }

    pub fn get(&self, kind: Kind) -> Option<&Model> {
        self.models.iter().find(|m| m.kind == kind)
    }
}
