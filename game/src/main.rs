//! Wordle for the reMarkable Paper Pro — write each letter by hand into the grid,
//! recognized on-device (no LLM). Premium e-ink UI: live ink, per-cell recognition,
//! colored reveals, a letter tracker.

mod display;
mod engine;
mod fb;
mod game;
mod history;
mod pen;
mod power;
mod preprocess;
mod qtfb;
mod render;
mod surface;
mod touch;
mod ui;

use ab_glyph::FontRef;
use engine::{Engine, Kind};
use game::{auto_correct, Game, State, Words, COLS};
use render as R;
use std::path::PathBuf;
use std::time::Instant;
use surface::{Surface, BLACK};

const PEN_R: i32 = 6; // on-screen ink
const INK_R: i32 = 4; // model ink (thin; preprocessing normalizes thickness/size)

fn model_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}
fn env_u32(k: &str, d: u32) -> u32 {
    std::env::var(k).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(d)
}
/// A time-derived seed (no calendar dependency); good enough to vary the answer.
fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9e3779b9)
}

struct Cell {
    ink: Vec<f32>,
    letter: Option<char>,
    top: Vec<(char, f32)>, // ranked recognizer candidates (letter, confidence) — feeds auto-correct
    dirty: bool,
    last_ink: Instant,
}
impl Cell {
    fn new() -> Self {
        Cell { ink: vec![0.0; (R::CELL * R::CELL) as usize], letter: None, top: Vec::new(), dirty: false, last_ink: Instant::now() }
    }
    fn clear(&mut self) {
        self.ink.fill(0.0);
        self.letter = None;
        self.top.clear();
        self.dirty = false;
    }
}

struct App {
    game: Game,
    cells: Vec<Cell>,       // the active row's 5 cells
    active: Option<usize>,  // cell currently receiving strokes
    focus: usize,           // highlighted cell
    last_pt: Option<(i32, i32)>,
    last_stroke: Instant,   // any-cell ink time — recognition waits for a global pause
    toast_until: Option<Instant>,
    locked: bool,           // grid input off (game over)
    quit: bool,
    help: bool,             // rules overlay open
    gover: bool,            // game-over card showing
    confirm_new: bool,      // "start a new game?" confirmation showing
    recent: Vec<[u8; 5]>,   // recently-seen answers (persisted) — don't repeat these
}
impl App {
    fn reset_row(&mut self) {
        self.cells = (0..COLS).map(|_| Cell::new()).collect();
        self.active = None;
        self.focus = 0;
        self.last_pt = None;
    }
    fn letters(&self) -> [Option<char>; COLS] {
        let mut a = [None; COLS];
        for c in 0..COLS {
            a[c] = self.cells[c].letter;
        }
        a
    }
    fn all_filled(&self) -> bool {
        self.cells.iter().all(|c| c.letter.is_some() || c.dirty)
    }
}

fn ink_segment(ink: &mut [f32], cw: i32, ch: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let cx = (x0 as f32 + (x1 - x0) as f32 * t).round() as i32;
        let cy = (y0 as f32 + (y1 - y0) as f32 * t).round() as i32;
        for yy in (cy - r).max(0)..(cy + r + 1).min(ch) {
            for xx in (cx - r).max(0)..(cx + r + 1).min(cw) {
                let (dx, dy) = (xx - cx, yy - cy);
                if dx * dx + dy * dy <= r * r {
                    ink[(yy * cw + xx) as usize] = 1.0;
                }
            }
        }
    }
}

/// Which active-row cell does screen point (x,y) belong to? None if outside the row.
fn cell_at(game: &Game, x: i32, y: i32) -> Option<usize> {
    if game.state != State::Playing {
        return None;
    }
    let r = game.filled_rows;
    let ry = R::GRID_Y0 + r as i32 * R::STEP;
    if y < ry - 12 || y >= ry + R::CELL + 12 {
        return None;
    }
    if x < R::GRID_X0 - 12 || x >= R::GRID_X0 + R::GRID_W + 12 {
        return None;
    }
    let c = ((x - R::GRID_X0) / R::STEP).clamp(0, COLS as i32 - 1);
    Some(c as usize)
}

fn main() {
    let (disp, mut surf) = match display::Display::open() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("wordle: display open failed: {e}");
            std::process::exit(1);
        }
    };
    let takeover = matches!(disp, display::Display::Quill);
    let font = ui::font();

    surf.fill_rect(0, 0, R::W as usize, R::H as usize, surface::WHITE);
    ui::text_center(&mut surf, &font, R::W / 2, 980, 60.0, "Loading…", R::C_DIM);
    disp.update_all(R::W as usize, R::H as usize);

    let words = Words::load();
    let engine = Engine::load_dir(&model_dir());
    if engine.get(Kind::Primary).is_none() {
        surf.fill_rect(0, 0, R::W as usize, R::H as usize, surface::WHITE);
        ui::text_center(&mut surf, &font, R::W / 2, 980, 52.0, "model missing", surface::ROSE);
        disp.update_all(R::W as usize, R::H as usize);
        std::thread::sleep(std::time::Duration::from_secs(5));
        disp.terminate();
        return;
    }
    let idle_ms: u128 = env_u32("WORDLE_IDLE_MS", 900) as u128;

    let mut pen = if takeover { pen::PenDevice::open().ok() } else { None };
    let mut touch = if takeover { touch::TouchDevice::open().ok() } else { None };
    let mut power = if takeover { power::PowerButton::open().ok() } else { None };

    // Load the persisted recent-answer memory and pick a first word that avoids it,
    // so reopening the app doesn't hand you a word you just played.
    let mut recent = history::load();
    let first = words.pick_avoiding(seed(), &recent);
    history::record(&mut recent, first);

    let mut app = App {
        game: Game::new(first),
        cells: (0..COLS).map(|_| Cell::new()).collect(),
        active: None,
        focus: 0,
        last_pt: None,
        last_stroke: Instant::now(),
        toast_until: None,
        locked: false,
        quit: false,
        help: false,
        gover: false,
        confirm_new: false,
        recent,
    };

    R::draw_all(&mut surf, &font, &app.game, &app.letters(), app.focus, false);
    disp.update_all(R::W as usize, R::H as usize);
    disp.full_refresh(R::W as usize, R::H as usize);

    let grace = Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        // --- pen: write ink into the active-row cells ---
        if let Some(ref mut p) = pen {
            let mut dirty: Option<(i32, i32, i32, i32)> = None;
            for s in p.drain() {
                // Eraser tip (back of the pen): touch a cell to wipe it clean so you
                // can rewrite that one box before pressing Enter. Dragging the eraser
                // across the row clears each box it passes over.
                if s.tool == pen::Tool::Eraser {
                    if s.touching && !app.locked && !app.help && !app.confirm_new {
                        if let Some(col) = cell_at(&app.game, s.x, s.y) {
                            if app.cells[col].letter.is_some() || app.cells[col].dirty {
                                app.cells[col].clear();
                                app.focus = col;
                                redraw_cell(&mut app, &mut surf, &disp, &font, col);
                                redraw_buttons(&mut app, &mut surf, &disp, &font);
                            }
                        }
                    }
                    app.last_pt = None;
                    continue;
                }
                if !(s.touching && s.pressure > 40) || app.locked || app.help || app.confirm_new {
                    app.last_pt = None;
                    continue;
                }
                let Some(col) = cell_at(&app.game, s.x, s.y) else {
                    app.last_pt = None;
                    continue;
                };
                // Rewriting a committed cell reopens it (clear the old letter once).
                if app.cells[col].letter.is_some() && !app.cells[col].dirty {
                    app.cells[col].clear();
                    redraw_cell(&mut app, &mut surf, &disp, &font, col);
                }
                app.active = Some(col);
                app.focus = col;
                // WRITING IS PURE INK — no recognition, no focus/button repaints here,
                // so writing quickly across cells stays perfectly smooth. Recognition
                // happens once, in a batch, after a global pause (below).
                let (cx, cy) = R::cell_xy(app.game.filled_rows, col);
                let prev = app.last_pt;
                let (ax, ay) = prev.unwrap_or((s.x, s.y));
                match prev {
                    Some((lx, ly)) => surf.brush_line(lx, ly, s.x, s.y, PEN_R, BLACK),
                    None => surf.stamp(s.x, s.y, PEN_R, BLACK),
                }
                ink_segment(&mut app.cells[col].ink, R::CELL, R::CELL, ax - cx, ay - cy, s.x - cx, s.y - cy, INK_R);
                app.cells[col].dirty = true;
                app.cells[col].last_ink = Instant::now();
                app.last_stroke = Instant::now();
                app.last_pt = Some((s.x, s.y));
                let (x0, y0, x1, y1) = (ax.min(s.x), ay.min(s.y), ax.max(s.x), ay.max(s.y));
                dirty = Some(match dirty {
                    Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                    None => (x0, y0, x1, y1),
                });
            }
            if let Some((x0, y0, x1, y1)) = dirty {
                let pad = PEN_R + 2;
                let (ux, uy) = ((x0 - pad).max(0), (y0 - pad).max(0));
                disp.update(ux, uy, (x1 + pad - ux).min(R::W - ux), (y1 + pad - uy).min(R::H - uy), true);
            }
        }

        // --- once the pen has been globally idle, recognize EVERY written cell in
        //     one batch and reveal the letters together (flowless — no mid-writing
        //     inference). Focus lands on the next empty cell. ---
        if !app.locked && !app.help && !app.confirm_new
            && app.cells.iter().any(|c| c.dirty)
            && app.last_stroke.elapsed().as_millis() >= idle_ms
        {
            for c in 0..COLS {
                if app.cells[c].dirty {
                    recognize_cell(&mut app, &engine, c);
                }
            }
            app.focus = (0..COLS).find(|&i| app.cells[i].letter.is_none()).unwrap_or(COLS - 1);
            redraw_active_row(&mut app, &mut surf, &disp, &font);
            redraw_buttons(&mut app, &mut surf, &disp, &font);
        }

        // --- clear a stale toast ---
        if let Some(t) = app.toast_until {
            if Instant::now() >= t {
                app.toast_until = None;
                R::draw_toast(&mut surf, &font, "");
                disp.update(0, R::TOAST_Y, R::W, R::TOAST_H, false);
            }
        }

        // --- touch ---
        if let Some(ref mut t) = touch {
            if let touch::Gesture::Tap(x, y) = t.drain() {
                on_tap(&mut app, &mut surf, &disp, &font, &engine, &words, x, y);
            }
        }
        if app.quit {
            break;
        }
        if let Some(ref mut pw) = power {
            if pw.drain_pressed() && Instant::now() >= grace {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(6));
    }
    disp.terminate();
}

/// Pure recognition — inference only, no drawing (so a batch can run then repaint
/// once). Digits are masked and case merged in the engine, so the result is a
/// lowercase letter.
fn recognize_cell(app: &mut App, engine: &Engine, c: usize) {
    let model = engine.get(Kind::Primary).unwrap();
    if let Ok(letters) = model.predict_letters(&app.cells[c].ink, R::CELL as usize, R::CELL as usize) {
        app.cells[c].letter = letters.first().map(|&(ch, _)| ch);
        app.cells[c].top = letters.iter().take(3).cloned().collect();
    }
    app.cells[c].dirty = false;
}

/// Repaint the active row's 5 cells (letters + focus highlight) in one update.
fn redraw_active_row(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    let r = app.game.filled_rows;
    for c in 0..COLS {
        let (x, y) = R::cell_xy(r, c);
        R::draw_cell(surf, font, x, y, app.cells[c].letter, game::Mark::Empty, c == app.focus);
    }
    let (_, ry) = R::cell_xy(r, 0);
    disp.update(R::GRID_X0, ry, R::GRID_W, R::CELL, false);
}

fn redraw_cell(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef, c: usize) {
    let (x, y) = R::cell_xy(app.game.filled_rows, c);
    R::draw_cell(surf, font, x, y, app.cells[c].letter, game::Mark::Empty, c == app.focus);
    disp.update(x, y, R::CELL, R::CELL, false);
}

fn redraw_buttons(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    let on = app.game.state == State::Playing && !app.locked && app.cells.iter().all(|c| c.letter.is_some());
    R::draw_buttons(surf, font, on);
    disp.update(0, R::BTN_Y - 10, R::W, R::BTN_H + 40, false);
}

fn toast(app: &mut App, surf: &mut Surface, disp: &display::Display, font: &FontRef, msg: &str) {
    R::draw_toast(surf, font, msg);
    disp.update(0, R::TOAST_Y, R::W, R::TOAST_H, false);
    app.toast_until = Some(Instant::now() + std::time::Duration::from_millis(1600));
}

fn on_tap(app: &mut App, surf: &mut Surface, disp: &display::Display, font: &FontRef, engine: &Engine, words: &Words, x: i32, y: i32) {
    // Game-over card: "New Game" starts fresh; tapping elsewhere dismisses to the board.
    if app.gover {
        if R::gameover_new_hit(x, y) {
            new_game(app, surf, disp, font, words); // clears gover
        } else {
            app.gover = false;
            R::draw_all(surf, font, &app.game, &app.letters(), app.focus, false);
            disp.update_all(R::W as usize, R::H as usize);
        }
        return;
    }
    // "Start a new game?" confirmation: only the two buttons act; a tap anywhere
    // else cancels. Either way we restore the board.
    if app.confirm_new {
        let start = R::confirm_yes_hit(x, y);
        app.confirm_new = false;
        if start {
            new_game(app, surf, disp, font, words);
        } else {
            R::draw_all(surf, font, &app.game, &app.letters(), app.focus, false);
            disp.update_all(R::W as usize, R::H as usize);
        }
        return;
    }
    // While the rules overlay is up, any tap dismisses it and restores the board.
    if app.help {
        app.help = false;
        R::draw_all(surf, font, &app.game, &app.letters(), app.focus, false);
        disp.update_all(R::W as usize, R::H as usize);
        return;
    }
    if R::help_hit(x, y) {
        app.help = true;
        R::draw_help(surf, font);
        disp.update_all(R::W as usize, R::H as usize);
        return;
    }
    if R::quit_hit(x, y) {
        app.quit = true;
        return;
    }
    let btns = R::buttons(true);
    if btns[0].hit(x, y) {
        // Delete: clear focused cell, else the last filled
        let target = if app.cells[app.focus].letter.is_some() || app.cells[app.focus].dirty {
            Some(app.focus)
        } else {
            (0..COLS).rev().find(|&c| app.cells[c].letter.is_some())
        };
        if let Some(c) = target {
            app.cells[c].clear();
            app.focus = c;
            redraw_cell(app, surf, disp, font, c);
            redraw_buttons(app, surf, disp, font);
        }
        return;
    }
    if btns[2].hit(x, y) {
        // New: confirm first if a game is in progress with something to lose, so an
        // accidental tap can't discard it. A finished or untouched board starts fresh.
        let in_progress = app.game.state == State::Playing
            && (app.game.filled_rows > 0 || app.cells.iter().any(|c| c.letter.is_some() || c.dirty));
        if in_progress {
            app.confirm_new = true;
            R::draw_confirm_new(surf, font);
            disp.update(R::CF_X - 4, R::CF_Y - 4, R::CF_W + 8, R::CF_H + 8, false);
        } else {
            new_game(app, surf, disp, font, words);
        }
        return;
    }
    if btns[1].hit(x, y) {
        submit(app, surf, disp, font, engine, words);
        return;
    }
    // tap a cell in the active row -> clear it for rewrite
    if let Some(c) = cell_at(&app.game, x, y) {
        app.cells[c].clear();
        app.focus = c;
        redraw_cell(app, surf, disp, font, c);
        redraw_buttons(app, surf, disp, font);
    }
}

fn submit(app: &mut App, surf: &mut Surface, disp: &display::Display, font: &FontRef, engine: &Engine, words: &Words) {
    if app.game.state != State::Playing || app.locked {
        return;
    }
    // force-recognize any still-dirty cells (pressing Enter before the idle pause),
    // then reveal them so what gets scored is what the player sees.
    let had_dirty = app.cells.iter().any(|c| c.dirty);
    for c in 0..COLS {
        if app.cells[c].dirty {
            recognize_cell(app, engine, c);
        }
    }
    if had_dirty {
        redraw_active_row(app, surf, disp, font);
    }
    if app.cells.iter().any(|c| c.letter.is_none()) {
        toast(app, surf, disp, font, "Write all 5 letters");
        return;
    }
    let mut chosen = [0u8; COLS];
    let mut alts: [Vec<(char, f32)>; COLS] = Default::default();
    for c in 0..COLS {
        chosen[c] = app.cells[c].letter.unwrap() as u8;
        alts[c] = app.cells[c].top.clone();
    }
    let word = match auto_correct(words, &chosen, &alts) {
        Some(w) => w,
        None => {
            toast(app, surf, disp, font, "Not in word list");
            return;
        }
    };
    let corrected = word != chosen;
    // apply any auto-correction to the display
    if corrected {
        for c in 0..COLS {
            if word[c] != chosen[c] {
                app.cells[c].letter = Some(word[c] as char);
                redraw_cell(app, surf, disp, font, c);
            }
        }
    }
    // accept: score, reveal, advance
    app.game.submit(word);
    app.reset_row();
    let over = app.game.state != State::Playing;
    if over {
        app.locked = true;
    }
    // one combined repaint: header (attempt/result), grid (colors), tracker, buttons
    R::draw_header(surf, font, &app.game);
    R::draw_grid(surf, font, &app.game, &app.letters(), app.focus);
    R::draw_tracker(surf, font, &app.game);
    R::draw_buttons(surf, font, false);
    disp.update(0, 0, R::W, R::BTN_Y + R::BTN_H + 20, false);
    // transparency: tell the player when a genuine toss-up was auto-corrected
    if corrected && !over {
        let from: String = chosen.iter().map(|&b| (b as char).to_ascii_uppercase()).collect();
        let to: String = word.iter().map(|&b| (b as char).to_ascii_uppercase()).collect();
        toast(app, surf, disp, font, &format!("read {from} as {to}"));
    }
    // game over: pop the result card over the board
    if over {
        app.gover = true;
        R::draw_gameover(surf, font, &app.game);
        disp.update(R::GO_X - 4, R::GO_Y - 4, R::GO_W + 8, R::GO_H + 90, false);
    }
}

fn new_game(app: &mut App, surf: &mut Surface, disp: &display::Display, font: &FontRef, words: &Words) {
    // fresh time seed, mixed with the previous answer so consecutive games differ
    let mut s = seed();
    for &b in &app.game.answer {
        s = s.wrapping_mul(31).wrapping_add(b as u64);
    }
    // Pick avoiding the recent-answer memory, then record the new word (persisted).
    let next = words.pick_avoiding(s, &app.recent);
    history::record(&mut app.recent, next);
    app.game = Game::new(next);
    app.reset_row();
    app.locked = false;
    app.gover = false;
    app.confirm_new = false;
    app.toast_until = None;
    R::draw_all(surf, font, &app.game, &app.letters(), app.focus, false);
    disp.update_all(R::W as usize, R::H as usize);
    disp.full_refresh(R::W as usize, R::H as usize);
}
