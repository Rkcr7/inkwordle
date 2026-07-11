//! Raw evdev pen input: the full digitizer, bypassing Qt's filtered view.
//! Gives us 0-4096 pressure, tilt, hover, and the eraser tip (BTN_TOOL_RUBBER),
//! at the hardware event rate.
//!
//! The device is grabbed (EVIOCGRAB) while the diary is open so xochitl
//! doesn't also react to the pen; released automatically on close/exit.

use std::io;
use std::os::fd::RawFd;

use crate::fb::{SCREEN_H, SCREEN_W};

// Digitizer axis ranges on the Paper Pro ("Elan marker input").
const DIGI_MAX_X: i32 = 11180;
const DIGI_MAX_Y: i32 = 15340;
pub const MAX_PRESSURE: i32 = 4096;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
/// SYN_DROPPED (an EV_SYN code): the kernel dropped events on buffer overflow.
const SYN_DROPPED: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_PRESSURE: u16 = 24;
const BTN_TOOL_PEN: u16 = 320;
const BTN_TOOL_RUBBER: u16 = 321;
const BTN_TOUCH: u16 = 330;

const EVIOCGRAB: libc::c_ulong = 0x40044590;
// EVIOCGABS(code) = _IOR('E', 0x40 + code, sizeof(input_absinfo)=24).
const EVIOCGABS_X: libc::c_ulong = 0x8018_4540; // 0x40 + 0x00 (ABS_X)
const EVIOCGABS_Y: libc::c_ulong = 0x8018_4541; // 0x40 + 0x01 (ABS_Y)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Eraser,
}

#[derive(Debug, Clone, Copy)]
pub struct PenSample {
    /// Screen coordinates.
    pub x: i32,
    pub y: i32,
    /// 0..4096
    pub pressure: i32,
    pub tool: Tool,
    pub touching: bool,
}

pub struct PenDevice {
    fd: RawFd,
    // Digitizer axis maxima, queried at open() (fall back to DIGI_MAX_*).
    x_max: i32,
    y_max: i32,
    // Accumulated state between SYN_REPORTs.
    raw_x: i32,
    raw_y: i32,
    pressure: i32,
    tool: Tool,
    touching: bool,
    dirty: bool,
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

impl PenDevice {
    /// Find and grab the marker input device.
    pub fn open() -> io::Result<Self> {
        let path = find_marker_device()?;
        let cpath = std::ffi::CString::new(path.clone()).unwrap();
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let grab = unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) };
        if grab != 0 {
            eprintln!("muse: warning: EVIOCGRAB failed ({}) — xochitl will also see the pen", io::Error::last_os_error());
        }
        eprintln!("muse: pen device {path} opened (grabbed: {})", grab == 0);
        Ok(Self {
            fd,
            x_max: abs_max(fd, EVIOCGABS_X, DIGI_MAX_X),
            y_max: abs_max(fd, EVIOCGABS_Y, DIGI_MAX_Y),
            raw_x: 0,
            raw_y: 0,
            pressure: 0,
            tool: Tool::Pen,
            touching: false,
            dirty: false,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    /// Drain all pending events; returns one sample per SYN_REPORT frame
    /// that changed state.
    pub fn drain(&mut self) -> Vec<PenSample> {
        let mut out = Vec::new();
        // input_event on 64-bit: struct timeval (16) + type u16 + code u16 + value i32.
        let mut buf = [0u8; 24 * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                    break; // no more data for now
                }
                eprintln!("muse: warning: pen read error: {e}");
                break;
            }
            if n == 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(24) {
                let etype = u16::from_le_bytes(chunk[16..18].try_into().unwrap());
                let code = u16::from_le_bytes(chunk[18..20].try_into().unwrap());
                let value = i32::from_le_bytes(chunk[20..24].try_into().unwrap());
                match (etype, code) {
                    (EV_ABS, ABS_X) => {
                        self.raw_x = value;
                        self.dirty = true;
                    }
                    (EV_ABS, ABS_Y) => {
                        self.raw_y = value;
                        self.dirty = true;
                    }
                    (EV_ABS, ABS_PRESSURE) => {
                        self.pressure = value;
                        self.dirty = true;
                    }
                    (EV_KEY, BTN_TOOL_PEN) if value == 1 => {
                        self.tool = Tool::Pen;
                    }
                    (EV_KEY, BTN_TOOL_RUBBER) => {
                        self.tool = if value == 1 { Tool::Eraser } else { Tool::Pen };
                    }
                    (EV_KEY, BTN_TOUCH) => {
                        self.touching = value == 1;
                        self.dirty = true;
                    }
                    (EV_SYN, SYN_DROPPED) => {
                        // Buffer overflow: a BTN_TOUCH-up may have been dropped,
                        // which would leave `touching` stuck and ink phantom
                        // strokes on later hover samples. Drop the accumulation.
                        self.touching = false;
                        self.dirty = false;
                        self.pressure = 0;
                    }
                    (EV_SYN, SYN_REPORT) => {
                        if self.dirty {
                            self.dirty = false;
                            // Clamp: an out-of-range digitizer value must not map
                            // to coordinates off the drawing surface.
                            out.push(PenSample {
                                x: (self.raw_x * (SCREEN_W as i32 - 1) / self.x_max.max(1))
                                    .clamp(0, SCREEN_W as i32 - 1),
                                y: (self.raw_y * (SCREEN_H as i32 - 1) / self.y_max.max(1))
                                    .clamp(0, SCREEN_H as i32 - 1),
                                pressure: self.pressure,
                                tool: self.tool,
                                touching: self.touching,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

impl Drop for PenDevice {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, EVIOCGRAB, 0i32);
            libc::close(self.fd);
        }
    }
}

fn find_marker_device() -> io::Result<String> {
    for i in 0..8 {
        let name_path = format!("/sys/class/input/event{i}/device/name");
        if let Ok(name) = std::fs::read_to_string(&name_path) {
            if name.to_lowercase().contains("marker") {
                return Ok(format!("/dev/input/event{i}"));
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no marker input device found"))
}
