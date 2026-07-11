//! Persisted recent-answer memory, so the same word doesn't recur for a long
//! stretch — even across app restarts, reinstalls, and reboots.
//!
//! Design notes:
//! - Stored on disk (NOT in RAM only), because the whole point is surviving a
//!   restart. In-memory alone would forget everything the moment the app closes.
//! - Stored in $HOME, NOT the app folder: `remagic install` replaces the app
//!   folder wholesale, so history there would reset on every update. HOME persists.
//! - Stored as the actual 5-letter words (one per line, newest last), NOT list
//!   indices: the answer list has been edited before, and an index would then
//!   point at a different word. A stale word simply never matches — robust.
//! - Capped at CAP entries. See CAP for why 200.

use std::path::PathBuf;

/// How many recent answers we refuse to repeat.
///
/// A heavy replay sitting is ~20-40 games; 200 is 5-10x that, so a fresh word is
/// guaranteed for far longer than any realistic session. Yet 200 is only ~8.6% of
/// the 2,315-word pool, so the remaining ~2,100 keep every pick feeling genuinely
/// random rather than a predictable cycle.
const CAP: usize = 200;

fn path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/root".to_string());
    PathBuf::from(home).join(".wordle-history")
}

/// Read the recent-answer list (newest last). Missing/garbage lines are ignored.
pub fn load() -> Vec<[u8; 5]> {
    let mut v = Vec::new();
    if let Ok(s) = std::fs::read_to_string(path()) {
        for line in s.lines() {
            let b = line.trim().as_bytes();
            if b.len() == 5 && b.iter().all(u8::is_ascii_lowercase) {
                let mut w = [0u8; 5];
                w.copy_from_slice(b);
                v.push(w);
            }
        }
    }
    v
}

/// Record a freshly-chosen answer: append it, cap the list to the most recent
/// CAP, and rewrite the file. A write failure is non-fatal (we just lose memory
/// of this one game rather than crashing the app).
pub fn record(recent: &mut Vec<[u8; 5]>, w: [u8; 5]) {
    recent.push(w);
    let len = recent.len();
    if len > CAP {
        recent.drain(0..len - CAP);
    }
    let body: String = recent
        .iter()
        .filter_map(|w| std::str::from_utf8(w).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(path(), body);
}
