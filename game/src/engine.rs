//! A lightweight, on-device handwritten-character recognition engine.
//!
//! Loads the three pretrained ONNX models (EMNIST-62 case-sensitive, EMNIST
//! balanced-47 case-merged, MNIST-12 digits) with `tract` — pure Rust, no
//! onnxruntime, no Python, no network — and predicts a single glyph. This is the
//! detection core the tablet games/apps call instead of an LLM.

use std::path::Path;
use tract_onnx::prelude::*;

use crate::preprocess::*;

/// Which model, and therefore which glyph size / tensor layout / labels.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// EMNIST-62: case-sensitive `0-9A-Za-z`. Big (18 MB) but complete.
    Primary,
    /// MNIST-12: digits only. Tiny (11 KB), int8.
    Digit,
    /// EMNIST balanced-47: case-merged alphanumeric. Small (258 KB) — the sweet spot.
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
            // NHWC for the LeNet/Keras export; NCHW for the torch/onnx-zoo ones.
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

/// Model output → probability distribution. Some models end in a softmax (output
/// already sums to 1); others emit raw logits. Detect: if non-negative and sums to
/// ~1, use as-is; otherwise softmax. (Running softmax on already-normalized output
/// flattens confidence toward uniform.)
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

impl Model {
    pub fn load(kind: Kind, path: &Path) -> TractResult<Self> {
        let s = kind.input_shape();
        let plan = tract_onnx::onnx()
            .model_for_path(path)?
            .with_input_fact(0, f32::fact([s[0], s[1], s[2], s[3]]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Model { kind, plan, labels: kind.labels() })
    }

    /// Predict from an ink plane (row-major, 1.0 = stroke).
    pub fn predict(&self, ink: &[f32], w: usize, h: usize) -> Result<Prediction, String> {
        Ok(self.predict_debug(ink, w, h)?.0)
    }

    /// Wordle-specific recognition: return top LETTERS (lowercase, best-first) with
    /// confidence. Digit classes are masked out and upper/lower case are merged, so
    /// the classic O/0, I/1/l, 2/Z, 5/S confusions collapse to the letter for free,
    /// and the result is already lowercase. Requires the 62-class model.
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
        // labels are "0-9" (0..10), "A-Z" (10..36), "a-z" (36..62). Merge case.
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

    /// Like `predict`, but also returns the 28x28 the model actually saw (for a
    /// preview so you can see what was fed in).
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

/// The full engine — as many of the three models as load successfully.
pub struct Engine {
    pub models: Vec<Model>,
}

impl Engine {
    /// Load from a directory holding the three `.onnx` files. A model that fails to
    /// load (e.g. an unsupported op) is skipped with a warning rather than fatal.
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
