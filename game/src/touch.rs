//! Raw touch input for takeover mode. Wordle only uses the short single-finger
//! tap = Tap(x, y) in screen pixels (for the on-screen controls). The 5-finger
//! quit gesture is deliberately NOT emitted here — it was exiting the game by
//! accident; the on-screen Quit button is the only way out. (Page-swipe gestures
//! are inherited from the shared input module and unused by this game.)

use crate::fb::{SCREEN_H, SCREEN_W};
use std::io;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

const EV_SYN: u16 = 0;
const EV_ABS: u16 = 3;
/// SYN_DROPPED (an EV_SYN code): the kernel dropped events on buffer overflow.
const SYN_DROPPED: u16 = 3;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_POSITION_X: u16 = 53;
const ABS_MT_POSITION_Y: u16 = 54;
const ABS_MT_TRACKING_ID: u16 = 57;
const EVIOCGRAB: libc::c_ulong = 0x40044590;
// EVIOCGABS(code) = _IOR('E', 0x40 + code, sizeof(input_absinfo=24)).
const EVIOCGABS_X: libc::c_ulong = 0x8018_4575; // 0x40 + 0x35 (ABS_MT_POSITION_X)
const EVIOCGABS_Y: libc::c_ulong = 0x8018_4576; // 0x40 + 0x36 (ABS_MT_POSITION_Y)
const MAX_SLOTS: usize = 16;
/// Minimum horizontal travel (raw units) for a lift to count as a page swipe.
const SWIPE_MIN: i32 = 280;
/// A long, deliberate forward drag (≈half the panel) — distinct from a short
/// flick so starting a fresh page can't happen by accident.
const SWIPE_LONG: i32 = 1000;
/// Max travel (raw units) for a lift to still count as a tap (not a drag).
const TAP_MAX: i32 = 60;
/// If contacts still claim to be down but no touch events have arrived for
/// this long, a finger-lift was dropped — clear the stale state (watchdog).
const STUCK_TIMEOUT: Duration = Duration::from_secs(2);

/// What the touch surface reported this drain.
#[derive(Debug, PartialEq, Eq)]
pub enum Gesture {
    None,
    /// Swipe right → show the previous (older) page.
    PrevPage,
    /// Swipe left → show the next (newer) page.
    NextPage,
    /// A long, deliberate forward drag → start a fresh page (same conversation).
    NewPage,
    /// A short single-finger tap at (x, y) in SCREEN pixels.
    Tap(i32, i32),
}

pub struct TouchDevice {
    fd: RawFd,
    slots: [bool; MAX_SLOTS],
    start_x: [i32; MAX_SLOTS],
    last_x: [i32; MAX_SLOTS],
    start_y: [i32; MAX_SLOTS],
    last_y: [i32; MAX_SLOTS],
    x_max: i32,
    y_max: i32,
    cur: usize,
    /// False while `cur` points past our slot table: events for an
    /// out-of-range slot are ignored rather than aliased onto the last slot.
    cur_valid: bool,
    /// Peak number of fingers held at once since the surface was last fully
    /// clear. A fingertip is one contact; a resting palm makes several — so a
    /// swipe/tap only counts as intentional when this stayed at 1 (palm reject).
    peak: usize,
    /// When the last touch event arrived, for the stuck-slot watchdog.
    last_event: Instant,
}

/// Read the maximum value of an ABS axis via EVIOCGABS (input_absinfo.maximum).
fn abs_max(fd: RawFd, code: libc::c_ulong, fallback: i32) -> i32 {
    let mut info = [0i32; 6]; // value, min, max, fuzz, flat, resolution
    let r = unsafe { libc::ioctl(fd, code, info.as_mut_ptr()) };
    if r >= 0 && info[2] > 0 {
        info[2]
    } else {
        fallback
    }
}

impl TouchDevice {
    pub fn open() -> io::Result<Self> {
        for i in 0..8 {
            let name_path = format!("/sys/class/input/event{i}/device/name");
            if let Ok(name) = std::fs::read_to_string(&name_path) {
                if name.to_lowercase().contains("touch") {
                    let path = std::ffi::CString::new(format!("/dev/input/event{i}")).unwrap();
                    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
                    if fd < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let grab = unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) };
                    if grab != 0 {
                        eprintln!("muse: warning: touch EVIOCGRAB failed ({}) — xochitl will also see touches", io::Error::last_os_error());
                    }
                    return Ok(Self {
                        fd,
                        slots: [false; MAX_SLOTS],
                        start_x: [i32::MIN; MAX_SLOTS],
                        last_x: [i32::MIN; MAX_SLOTS],
                        start_y: [i32::MIN; MAX_SLOTS],
                        last_y: [i32::MIN; MAX_SLOTS],
                        x_max: abs_max(fd, EVIOCGABS_X, 2064),
                        y_max: abs_max(fd, EVIOCGABS_Y, 2832),
                        cur: 0,
                        cur_valid: true,
                        peak: 0,
                        last_event: Instant::now(),
                    });
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "no touch device"))
    }

    /// Drop all multitouch state. Used on SYN_DROPPED (kernel buffer overflow,
    /// where a finger-lift may have been lost) and by the stuck-slot watchdog:
    /// the next contacts re-establish cleanly from a clear surface.
    fn reset_mt(&mut self) {
        self.slots = [false; MAX_SLOTS];
        self.start_x = [i32::MIN; MAX_SLOTS];
        self.last_x = [i32::MIN; MAX_SLOTS];
        self.start_y = [i32::MIN; MAX_SLOTS];
        self.last_y = [i32::MIN; MAX_SLOTS];
        self.peak = 0;
        self.cur = 0;
        self.cur_valid = true;
    }

    /// Drain touch events, returning a gesture. Only a clean single-finger tap
    /// (palm-rejected) is used by Wordle; horizontal lifts map to page gestures
    /// for other apps. No quit gesture is produced.
    pub fn drain(&mut self) -> Gesture {
        let mut gesture = Gesture::None;
        // Watchdog: if fingers still claim to be down but nothing has arrived
        // for a while, a lift was dropped — reset so the palm-reject peak check
        // can't wedge shut and silently swallow every future gesture.
        if self.slots.iter().any(|&s| s) && self.last_event.elapsed() > STUCK_TIMEOUT {
            self.reset_mt();
        }
        let mut buf = [0u8; 24 * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                    break; // no more data for now
                }
                eprintln!("muse: warning: touch read error: {e}");
                break;
            }
            if n == 0 {
                break;
            }
            self.last_event = Instant::now();
            for chunk in buf[..n as usize].chunks_exact(24) {
                let etype = u16::from_le_bytes(chunk[16..18].try_into().unwrap());
                let code = u16::from_le_bytes(chunk[18..20].try_into().unwrap());
                let value = i32::from_le_bytes(chunk[20..24].try_into().unwrap());
                if etype == EV_SYN {
                    if code == SYN_DROPPED {
                        // State may be inconsistent (a lift could have been
                        // dropped). Full MT resync is the simplest correct fix.
                        self.reset_mt();
                    }
                    continue;
                }
                if etype != EV_ABS {
                    continue;
                }
                match code {
                    ABS_MT_SLOT => {
                        if value >= 0 && (value as usize) < MAX_SLOTS {
                            self.cur = value as usize;
                            self.cur_valid = true;
                        } else {
                            // Ignore this slot's events rather than aliasing
                            // them onto slot 15 and corrupting the count.
                            self.cur_valid = false;
                        }
                    }
                    ABS_MT_POSITION_X if self.cur_valid => {
                        if self.start_x[self.cur] == i32::MIN {
                            self.start_x[self.cur] = value;
                        }
                        self.last_x[self.cur] = value;
                    }
                    ABS_MT_POSITION_Y if self.cur_valid => {
                        if self.start_y[self.cur] == i32::MIN {
                            self.start_y[self.cur] = value;
                        }
                        self.last_y[self.cur] = value;
                    }
                    ABS_MT_TRACKING_ID if self.cur_valid => {
                        let was = self.slots[self.cur];
                        let now = value != -1;
                        self.slots[self.cur] = now;
                        let fingers = self.slots.iter().filter(|&&s| s).count();
                        self.peak = self.peak.max(fingers);
                        if now && !was {
                            self.start_x[self.cur] = i32::MIN; // set on first X
                            self.last_x[self.cur] = i32::MIN;
                            self.start_y[self.cur] = i32::MIN;
                            self.last_y[self.cur] = i32::MIN;
                        }
                        if !now && was {
                            let (sx, lx) = (self.start_x[self.cur], self.last_x[self.cur]);
                            let (sy, ly) = (self.start_y[self.cur], self.last_y[self.cur]);
                            // Only a CLEAN single-finger contact (peak == 1) is an
                            // intentional swipe/tap; a palm brings several contacts
                            // down at once (peak > 1) and is rejected here.
                            if fingers == 0 && self.peak <= 1 && sx != i32::MIN && lx != i32::MIN {
                                let dx = lx - sx;
                                let dy = if sy != i32::MIN { ly - sy } else { 0 };
                                if dx <= -SWIPE_LONG {
                                    // A long, deliberate forward drag — used to
                                    // start a fresh page (a short flick won't).
                                    gesture = Gesture::NewPage;
                                } else if dx <= -SWIPE_MIN {
                                    gesture = Gesture::NextPage;
                                } else if dx >= SWIPE_MIN {
                                    gesture = Gesture::PrevPage;
                                } else if dx.abs() <= TAP_MAX && dy.abs() <= TAP_MAX && sy != i32::MIN {
                                    // Short single-finger tap → map to screen px.
                                    let px = (lx as i64 * SCREEN_W as i64 / self.x_max.max(1) as i64) as i32;
                                    let py = (ly as i64 * SCREEN_H as i64 / self.y_max.max(1) as i64) as i32;
                                    gesture = Gesture::Tap(px, py);
                                }
                            }
                        }
                        // All contacts lifted → reset the palm counter for the
                        // next gesture. (5-finger quit is intentionally not handled;
                        // multi-finger touches are still palm-rejected above.)
                        if fingers == 0 {
                            self.peak = 0;
                        }
                    }
                    _ => {}
                }
            }
        }
        gesture
    }
}

impl Drop for TouchDevice {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, EVIOCGRAB, 0i32);
            libc::close(self.fd);
        }
    }
}
