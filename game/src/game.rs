//! Wordle game logic — word lists, scoring, state, and dictionary auto-correct.
//! Words are `[u8; 5]` of lowercase ascii for speed.

use std::collections::HashSet;

const ANSWERS_TXT: &str = include_str!("../assets/answers.txt");
const VALID_TXT: &str = include_str!("../assets/valid.txt");

pub const ROWS: usize = 6;
pub const COLS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Empty,
    Absent,
    Present,
    Correct,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Playing,
    Won,
    Lost,
}

fn parse(s: &str) -> impl Iterator<Item = [u8; 5]> + '_ {
    s.lines().filter_map(|l| {
        let b = l.trim().as_bytes();
        if b.len() == 5 && b.iter().all(u8::is_ascii_lowercase) {
            let mut w = [0u8; 5];
            w.copy_from_slice(b);
            Some(w)
        } else {
            None
        }
    })
}

/// The dictionaries: the curated answer pool and the full accepted-guess set.
pub struct Words {
    pub answers: Vec<[u8; 5]>,
    pub valid: HashSet<[u8; 5]>,
}

impl Words {
    pub fn load() -> Self {
        let answers: Vec<[u8; 5]> = parse(ANSWERS_TXT).collect();
        let valid: HashSet<[u8; 5]> = parse(VALID_TXT).chain(answers.iter().copied()).collect();
        Words { answers, valid }
    }
    pub fn is_valid(&self, w: &[u8; 5]) -> bool {
        self.valid.contains(w)
    }
    pub fn pick(&self, seed: u64) -> [u8; 5] {
        self.answers[(seed as usize) % self.answers.len().max(1)]
    }
    /// Pick an answer avoiding any word in `recent`. Draws from a fast LCG so the
    /// choice stays effectively uniform; with `recent` small (~200 of 2,315) a
    /// fresh word is found almost immediately. Falls back to an unfiltered pick if
    /// everything is somehow excluded, so this can never loop forever.
    pub fn pick_avoiding(&self, mut seed: u64, recent: &[[u8; 5]]) -> [u8; 5] {
        let n = self.answers.len().max(1);
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let w = self.answers[(seed >> 33) as usize % n];
            if !recent.contains(&w) {
                return w;
            }
        }
        self.answers[(seed as usize) % n]
    }
}

pub struct Game {
    pub answer: [u8; 5],
    pub rows: [[u8; 5]; ROWS],
    pub marks: [[Mark; COLS]; ROWS],
    pub filled_rows: usize, // number of submitted guesses
    pub state: State,
    pub letter: [Mark; 26], // best-known state per a..z, for the tracker
}

impl Game {
    pub fn new(answer: [u8; 5]) -> Self {
        Game {
            answer,
            rows: [[0; 5]; ROWS],
            marks: [[Mark::Empty; COLS]; ROWS],
            filled_rows: 0,
            state: State::Playing,
            letter: [Mark::Empty; 26],
        }
    }

    /// The classic two-pass green/yellow/gray scoring, with CORRECT duplicate-letter
    /// handling: greens first (consuming answer letters), then presents from what's
    /// left, else absent.
    pub fn score(answer: &[u8; 5], guess: &[u8; 5]) -> [Mark; COLS] {
        let mut marks = [Mark::Absent; COLS];
        let mut counts = [0i32; 26];
        for &c in answer {
            counts[(c - b'a') as usize] += 1;
        }
        for i in 0..COLS {
            if guess[i] == answer[i] {
                marks[i] = Mark::Correct;
                counts[(guess[i] - b'a') as usize] -= 1;
            }
        }
        for i in 0..COLS {
            if marks[i] == Mark::Correct {
                continue;
            }
            let idx = (guess[i] - b'a') as usize;
            if counts[idx] > 0 {
                marks[i] = Mark::Present;
                counts[idx] -= 1;
            }
        }
        marks
    }

    /// Record a (pre-validated) guess: score it, update the keyboard/letter states,
    /// advance, and set win/lose.
    pub fn submit(&mut self, guess: [u8; 5]) {
        let r = self.filled_rows;
        let m = Game::score(&self.answer, &guess);
        self.rows[r] = guess;
        self.marks[r] = m;
        for i in 0..COLS {
            let li = (guess[i] - b'a') as usize;
            self.letter[li] = best_mark(self.letter[li], m[i]);
        }
        self.filled_rows += 1;
        if m.iter().all(|&x| x == Mark::Correct) {
            self.state = State::Won;
        } else if self.filled_rows >= ROWS {
            self.state = State::Lost;
        }
    }
}

fn rank(m: Mark) -> u8 {
    match m {
        Mark::Correct => 3,
        Mark::Present => 2,
        Mark::Absent => 1,
        Mark::Empty => 0,
    }
}
fn best_mark(a: Mark, b: Mark) -> Mark {
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// Conservative dictionary auto-correct. If `chosen` isn't a valid word, try
/// swapping ONE cell at a time for an alternate letter that cell's recognizer also
/// considered (`alts[pos]`). If exactly one such single-substitution is a valid
/// word, return it (the misread was almost certainly that one letter); otherwise
/// return None (genuine reject — let the player fix it).
/// Confidence-gated single-substitution correction. `alts[pos]` are the model's
/// ranked candidates for cell `pos` as (letter, confidence), best first.
///
/// A cell's letter may be swapped for a runner-up ONLY if the model was almost
/// evenly torn there — the runner-up's confidence is at least `RATIO` of the top
/// pick's (0.8 = a near coin-flip). This is deliberately strict: normally-written
/// letters are never swapped. It fixes "snale -> swale" (a clearly-written 'n' is
/// never rewritten to 'w') while still rescuing the rare true toss-up (e.g. an
/// 'a'/'o' the model genuinely couldn't separate). If exactly one valid word
/// results from all eligible swaps, we accept it.
pub fn auto_correct(words: &Words, chosen: &[u8; 5], alts: &[Vec<(char, f32)>; COLS]) -> Option<[u8; 5]> {
    if words.is_valid(chosen) {
        return Some(*chosen);
    }
    const RATIO: f32 = 0.8;
    let mut found: Vec<[u8; 5]> = Vec::new();
    for pos in 0..COLS {
        let top_conf = alts[pos].first().map(|&(_, p)| p).unwrap_or(0.0);
        for &(alt, conf) in &alts[pos] {
            let a = alt as u8;
            if a == chosen[pos] || !a.is_ascii_lowercase() {
                continue;
            }
            if conf < RATIO * top_conf {
                continue; // the model was confident in the written letter — don't swap it
            }
            let mut w = *chosen;
            w[pos] = a;
            if words.is_valid(&w) && !found.contains(&w) {
                found.push(w);
            }
        }
    }
    if found.len() == 1 {
        Some(found[0])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> [u8; 5] {
        let mut a = [0u8; 5];
        a.copy_from_slice(s.as_bytes());
        a
    }
    #[test]
    fn scoring_duplicates() {
        use Mark::*;
        // trivial: an exact guess is all green
        assert_eq!(Game::score(&w("crane"), &w("crane")), [Correct; 5]);
        // answer SPEED (two E's), guess ERASE (two E's, none aligned):
        // both E's become Present (answer has 2), S also Present.
        assert_eq!(Game::score(&w("speed"), &w("erase")), [Present, Absent, Absent, Present, Present]);
        // answer ABBEY (one E, two B), guess BABES:
        // pos2 B and pos3 E are green; the remaining B and A are present; trailing S absent.
        assert_eq!(Game::score(&w("abbey"), &w("babes")), [Present, Present, Correct, Correct, Absent]);
        // over-guessing a single letter: answer ABBEY (one E), guess EERIE:
        // only the FIRST E is Present; the rest (incl. other E's) are Absent.
        assert_eq!(Game::score(&w("abbey"), &w("eerie")), [Present, Absent, Absent, Absent, Absent]);
    }
    #[test]
    fn win_and_letter_states() {
        let words = Words::load();
        assert!(words.answers.len() > 2000);
        let mut g = Game::new(w("crane"));
        g.submit(w("crane"));
        assert_eq!(g.state, State::Won);
        assert_eq!(g.letter[(b'c' - b'a') as usize], Mark::Correct);
    }
    #[test]
    fn autocorrect_single_sub() {
        let words = Words::load();
        // "crano" invalid; the last cell was a genuine toss-up o/e -> corrects to crane.
        let alts = [
            vec![('c', 0.9)],
            vec![('r', 0.9)],
            vec![('a', 0.9)],
            vec![('n', 0.9)],
            vec![('o', 0.5), ('e', 0.46)], // near-tie: 0.46 >= 0.8 * 0.5
        ];
        assert_eq!(auto_correct(&words, &w("crano"), &alts), Some(w("crane")));
    }
    #[test]
    fn autocorrect_respects_confident_letter() {
        let words = Words::load();
        // "snale" invalid; 'w' is only a weak runner-up on a confident 'n' -> NO swap,
        // so it is rejected (None) instead of silently becoming "swale".
        let alts = [
            vec![('s', 0.95)],
            vec![('n', 0.93), ('w', 0.02)], // 'n' dominant, 'w' negligible
            vec![('a', 0.95)],
            vec![('l', 0.95)],
            vec![('e', 0.90)],
        ];
        assert_eq!(auto_correct(&words, &w("snale"), &alts), None);
    }
    #[test]
    fn pick_avoiding_skips_recent() {
        let words = Words::load();
        // Exclude the first 200 answers; every draw must land outside that set.
        let recent: Vec<[u8; 5]> = words.answers.iter().take(200).copied().collect();
        for seed in 0..5000u64 {
            let got = words.pick_avoiding(seed, &recent);
            assert!(!recent.contains(&got), "returned an excluded word for seed {seed}");
        }
    }
    #[test]
    fn pick_avoiding_falls_back_when_all_excluded() {
        let words = Words::load();
        // Everything excluded -> must still return a valid answer (no infinite loop).
        let recent = words.answers.clone();
        let got = words.pick_avoiding(123, &recent);
        assert!(words.answers.contains(&got));
    }
}
