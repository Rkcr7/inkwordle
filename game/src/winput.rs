//! Windowed-mode input: translate qtfb protocol events into the same
//! `PenSample` / tap that the takeover (evdev) path produces, so the game loop
//! handles both input sources with identical logic.
//!
//! qtfb hands us framebuffer-pixel coordinates — the same 1620x2160 space the
//! game renders into — so there is NO transform, rotation, or scaling to do
//! (verified against rm-appload's FBController::convertPointToQTFBPixels). Pen
//! pressure arrives as `d` in 0..100 (the digitizer's pressure * 100), 0 on
//! release. qtfb cannot distinguish pen from eraser (dev_id is always 0 for the
//! pen), so windowed mode has no eraser tip — you clear a cell by writing over it.

use crate::pen::{PenSample, Tool};
use crate::qtfb::{
    InputEvent, INPUT_PEN_PRESS, INPUT_PEN_RELEASE, INPUT_PEN_UPDATE, INPUT_TOUCH_PRESS,
    INPUT_TOUCH_RELEASE, INPUT_TOUCH_UPDATE,
};

/// Max travel (screen px) from touch-down for the lift to still count as a tap.
const TAP_MAX: i32 = 60;

pub struct WinInput {
    /// The single-finger contact being tracked for a tap: (dev_id, start_x, start_y).
    touch: Option<(i32, i32, i32)>,
    /// Set once the tracked finger travels past TAP_MAX — then the lift isn't a tap.
    moved: bool,
}

impl WinInput {
    pub fn new() -> Self {
        Self { touch: None, moved: false }
    }

    /// Translate a batch of qtfb events into pen samples (for ink) and an
    /// optional tap `(x, y)` in screen pixels (for the on-screen controls).
    pub fn translate(&mut self, events: &[InputEvent]) -> (Vec<PenSample>, Option<(i32, i32)>) {
        let mut samples = Vec::new();
        let mut tap = None;
        for ev in events {
            match ev.input_type {
                INPUT_PEN_PRESS | INPUT_PEN_UPDATE => samples.push(PenSample {
                    x: ev.x,
                    y: ev.y,
                    // A qtfb pen event always means real contact; map the coarse
                    // 0..100 pressure into the game's 0..4096 `> 40` gate, with a
                    // floor so even a light reading still draws.
                    pressure: (ev.d.clamp(0, 100) * 40 + 80).min(4096),
                    tool: Tool::Pen,
                    touching: true,
                }),
                INPUT_PEN_RELEASE => samples.push(PenSample {
                    x: ev.x,
                    y: ev.y,
                    pressure: 0,
                    tool: Tool::Pen,
                    touching: false,
                }),
                INPUT_TOUCH_PRESS => {
                    self.touch = Some((ev.dev_id, ev.x, ev.y));
                    self.moved = false;
                }
                INPUT_TOUCH_UPDATE => {
                    if let Some((id, sx, sy)) = self.touch {
                        if ev.dev_id == id
                            && ((ev.x - sx).abs() > TAP_MAX || (ev.y - sy).abs() > TAP_MAX)
                        {
                            self.moved = true;
                        }
                    }
                }
                INPUT_TOUCH_RELEASE => {
                    if let Some((id, _, _)) = self.touch {
                        if ev.dev_id == id && !self.moved {
                            tap = Some((ev.x, ev.y));
                        }
                    }
                    self.touch = None;
                    self.moved = false;
                }
                _ => {}
            }
        }
        (samples, tap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: i32, dev: i32, x: i32, y: i32, d: i32) -> InputEvent {
        InputEvent { input_type: t, dev_id: dev, x, y, d }
    }

    #[test]
    fn pen_press_draws_and_passes_pressure_gate() {
        let mut w = WinInput::new();
        let (s, tap) = w.translate(&[ev(INPUT_PEN_PRESS, 0, 100, 200, 50)]);
        assert_eq!(s.len(), 1);
        assert!(s[0].touching);
        assert!(s[0].pressure > 40, "pressure must clear the game's > 40 gate");
        assert_eq!((s[0].x, s[0].y), (100, 200));
        assert!(tap.is_none());
    }

    #[test]
    fn light_pen_press_still_draws() {
        // d == 0 (pressure sensor floor) must still produce a drawing sample.
        let mut w = WinInput::new();
        let (s, _) = w.translate(&[ev(INPUT_PEN_UPDATE, 0, 5, 5, 0)]);
        assert!(s[0].touching && s[0].pressure > 40);
    }

    #[test]
    fn pen_release_lifts() {
        let mut w = WinInput::new();
        let (s, _) = w.translate(&[ev(INPUT_PEN_RELEASE, 0, 1, 2, 0)]);
        assert_eq!(s.len(), 1);
        assert!(!s[0].touching);
    }

    #[test]
    fn short_touch_is_a_tap() {
        let mut w = WinInput::new();
        w.translate(&[ev(INPUT_TOUCH_PRESS, 1, 50, 60, 0)]);
        let (_, tap) = w.translate(&[ev(INPUT_TOUCH_RELEASE, 1, 52, 61, 0)]);
        assert_eq!(tap, Some((52, 61)));
    }

    #[test]
    fn dragged_touch_is_not_a_tap() {
        let mut w = WinInput::new();
        w.translate(&[ev(INPUT_TOUCH_PRESS, 1, 50, 60, 0)]);
        w.translate(&[ev(INPUT_TOUCH_UPDATE, 1, 300, 60, 0)]);
        let (_, tap) = w.translate(&[ev(INPUT_TOUCH_RELEASE, 1, 300, 60, 0)]);
        assert!(tap.is_none(), "a drag must not fire a control tap");
    }

    #[test]
    fn press_release_in_one_batch_taps() {
        let mut w = WinInput::new();
        let (_, tap) = w.translate(&[
            ev(INPUT_TOUCH_PRESS, 2, 800, 900, 0),
            ev(INPUT_TOUCH_RELEASE, 2, 801, 900, 0),
        ]);
        assert_eq!(tap, Some((801, 900)));
    }
}
