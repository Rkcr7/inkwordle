//! Offline answer-word definitions shipped as `definitions.tsv` beside the binary
//! (and embedded at compile time as a fallback).

use std::collections::HashMap;
use std::path::PathBuf;

const EMBEDDED: &str = include_str!("../assets/definitions.tsv");

/// Load word → one-line definition. Prefers a file next to the executable so the
/// asset can be updated without a full rebuild; falls back to the embedded table.
pub fn load() -> HashMap<String, String> {
    let mut map = parse_tsv(EMBEDDED);
    if let Some(path) = beside_exe("definitions.tsv") {
        if let Ok(text) = std::fs::read_to_string(path) {
            for (k, v) in parse_tsv(&text) {
                map.insert(k, v);
            }
        }
    }
    map
}

fn beside_exe(name: &str) -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
}

fn parse_tsv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::with_capacity(2400);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((w, d)) = line.split_once('\t') else {
            continue;
        };
        let w = w.trim().to_ascii_lowercase();
        let d = d.trim();
        if w.len() == 5 && !d.is_empty() {
            map.insert(w, d.to_string());
        }
    }
    map
}

/// Look up a 5-letter answer; returns a display string (never empty if you pass a default).
pub fn get<'a>(map: &'a HashMap<String, String>, answer: &[u8; 5]) -> Option<&'a str> {
    let w: String = answer.iter().map(|&b| (b as char).to_ascii_lowercase()).collect();
    map.get(&w).map(|s| s.as_str())
}
