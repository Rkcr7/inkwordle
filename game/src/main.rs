//! InkWordle for the reMarkable Paper Pro — write each letter by hand into the grid,
//! recognized on-device (no LLM). Optimized hardware path: adaptive 1–2 ms poll,
//! coalesced ink flush, u8 ink, warmed model, region redraws, soft new-game refresh.
//! Word definitions power a mid-game hint clue and the game-over reveal.

mod definitions;
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
use engine::{Engine, Kind, PredictScratch};
use game::{auto_correct, Game, State, Words, COLS};
use render as R;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use surface::{Surface, BLACK};

const PEN_R: i32 = 6;
const INK_R: i32 = 4;

fn model_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}
fn env_u32(k: &str, d: u32) -> u32 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(d)
}
fn env_flag_on(k: &str, default_on: bool) -> bool {
    match std::env::var(k) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => default_on,
    }
}
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e3779b9)
}

struct Cell {
    /// Dense ink: 0 = paper, 255 = stroke (4× less RAM/traffic than f32).
    ink: Vec<u8>,
    letter: Option<char>,
    top: Vec<(char, f32)>,
    dirty: bool,
    last_ink: Instant,
}
impl Cell {
    fn new() -> Self {
        Cell {
            ink: vec![0u8; (R::CELL * R::CELL) as usize],
            letter: None,
            top: Vec::new(),
            dirty: false,
            last_ink: Instant::now(),
        }
    }
    fn clear(&mut self) {
        self.ink.fill(0);
        self.letter = None;
        self.top.clear();
        self.dirty = false;
    }
}

struct App {
    game: Game,
    cells: Vec<Cell>,
    active: Option<usize>,
    focus: usize,
    last_pt: Option<(i32, i32)>,
    last_stroke: Instant,
    /// Last time the pen was actively writing (for adaptive poll).
    pen_hot_until: Instant,
    toast_until: Option<Instant>,
    locked: bool,
    quit: bool,
    help: bool,
    gover: bool,
    confirm_new: bool,
    /// Mid-game meaning clue popup (does not reveal the letters).
    hint_popup: bool,
    recent: Vec<[u8; 5]>,
    /// Offline answer → one-line meaning (hint + game-over card).
    defs: HashMap<String, String>,
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
}

/// Model-plane disk stamp with horizontal spans (matches screen brush density).
fn ink_segment(ink: &mut [u8], cw: i32, ch: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    // Subsample along the segment like brush_line — disks overlap heavily.
    let stride_step = (r / 2).max(1);
    let n = (steps / stride_step).max(1);
    for s in 0..=n {
        let t = s as f32 / n as f32;
        let cx = (x0 as f32 + (x1 - x0) as f32 * t).round() as i32;
        let cy = (y0 as f32 + (y1 - y0) as f32 * t).round() as i32;
        ink_disk(ink, cw, ch, cx, cy, r);
    }
    ink_disk(ink, cw, ch, x1, y1, r);
}

#[inline]
fn isqrt_u32(n: u32) -> i32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x as i32
}

fn ink_disk(ink: &mut [u8], cw: i32, ch: i32, cx: i32, cy: i32, r: i32) {
    let r2 = (r * r) as u32;
    let ink_len = ink.len();
    for dy in -r..=r {
        let y = cy + dy;
        if y < 0 || y >= ch {
            continue;
        }
        let max_dx = isqrt_u32(r2.saturating_sub((dy * dy) as u32));
        let x0 = (cx - max_dx).max(0);
        let x1 = (cx + max_dx).min(cw - 1);
        if x1 < x0 {
            continue;
        }
        let start = (y * cw + x0) as usize;
        let n = (x1 - x0 + 1) as usize;
        // SAFETY: x0..=x1 clamped to [0,cw), y in [0,ch), ink is cw*ch.
        debug_assert!(start + n <= ink_len);
        unsafe {
            let p = ink.as_mut_ptr().add(start);
            core::ptr::write_bytes(p, 255, n);
        }
    }
}

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
            eprintln!("inkwordle: display open failed: {e}");
            std::process::exit(1);
        }
    };
    let takeover = matches!(disp, display::Display::Quill);
    let font = ui::font();

    surf.fill_rect(0, 0, R::W as usize, R::H as usize, surface::WHITE);
    ui::text_center(&mut surf, &font, R::W / 2, 980, 60.0, "Loading…", R::C_DIM);
    disp.update_all(R::W as usize, R::H as usize);

    let words = Words::load();
    let defs = definitions::load();
    eprintln!("inkwordle: loaded {} word definitions", defs.len());
    let engine = Engine::load_dir(&model_dir());
    if engine.get(Kind::Primary).is_none() {
        surf.fill_rect(0, 0, R::W as usize, R::H as usize, surface::WHITE);
        ui::text_center(
            &mut surf,
            &font,
            R::W / 2,
            980,
            52.0,
            "model missing",
            surface::ROSE,
        );
        disp.update_all(R::W as usize, R::H as usize);
        std::thread::sleep(Duration::from_secs(5));
        disp.terminate();
        return;
    }

    // Idle before recognition: stock-compatible 900 ms default (writers need
    // time to lift between letters / think mid-word). Still overridable via env.
    let idle_ms: u128 = env_u32("INKWORDLE_IDLE_MS", 900) as u128;
    let flush_ms: u128 = env_u32("INKWORDLE_FLUSH_MS", 2).clamp(1, 16) as u128;
    let soft_new = env_flag_on("INKWORDLE_SOFT_NEW", false);

    let mut scratch = PredictScratch::new();
    // Warm tract so the first real letter isn't a cold-graph hitch.
    if let Some(m) = engine.get(Kind::Primary) {
        let t0 = Instant::now();
        m.warmup(&mut scratch);
        eprintln!(
            "inkwordle: model warmup {}ms (idle={}ms flush={}ms soft_new={})",
            t0.elapsed().as_millis(),
            idle_ms,
            flush_ms,
            soft_new
        );
    }

    let mut pen = if takeover {
        pen::PenDevice::open().ok()
    } else {
        None
    };
    let mut touch = if takeover {
        touch::TouchDevice::open().ok()
    } else {
        None
    };
    let mut power = if takeover {
        power::PowerButton::open().ok()
    } else {
        None
    };

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
        pen_hot_until: Instant::now(),
        toast_until: None,
        locked: false,
        quit: false,
        help: false,
        gover: false,
        confirm_new: false,
        hint_popup: false,
        recent,
        defs,
    };

    R::draw_all(&mut surf, &font, &app.game, &app.letters(), app.focus, false, def_for(&app));
    R::draw_hint_button(&mut surf, &font, true);
    disp.update_all(R::W as usize, R::H as usize);
    // First paint: one full refresh for a clean board; later New Games can soft-refresh.
    disp.full_refresh(R::W as usize, R::H as usize);

    let grace = Instant::now() + Duration::from_millis(1500);
    let mut ink_dirty: Option<(i32, i32, i32, i32)> = None;
    let mut last_flush = Instant::now();

    loop {
        let mut pen_active = false;

        // --- pen ---
        if let Some(ref mut p) = pen {
            for s in p.drain() {
                if s.tool == pen::Tool::Eraser {
                    if s.touching
                        && !app.locked
                        && !app.help
                        && !app.confirm_new
                        && !app.hint_popup
                        && !app.gover
                    {
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
                if !(s.touching && s.pressure > 40)
                    || app.locked
                    || app.help
                    || app.confirm_new
                    || app.hint_popup
                    || app.gover
                {
                    app.last_pt = None;
                    continue;
                }
                let Some(col) = cell_at(&app.game, s.x, s.y) else {
                    app.last_pt = None;
                    continue;
                };
                pen_active = true;
                app.pen_hot_until = Instant::now() + Duration::from_millis(80);

                if app.cells[col].letter.is_some() && !app.cells[col].dirty {
                    app.cells[col].clear();
                    redraw_cell(&mut app, &mut surf, &disp, &font, col);
                }
                app.active = Some(col);
                app.focus = col;
                let (cx, cy) = R::cell_xy(app.game.filled_rows, col);
                let prev = app.last_pt;
                let (ax, ay) = prev.unwrap_or((s.x, s.y));
                match prev {
                    Some((lx, ly)) => surf.brush_line(lx, ly, s.x, s.y, PEN_R, BLACK),
                    None => surf.stamp(s.x, s.y, PEN_R, BLACK),
                }
                ink_segment(
                    &mut app.cells[col].ink,
                    R::CELL,
                    R::CELL,
                    ax - cx,
                    ay - cy,
                    s.x - cx,
                    s.y - cy,
                    INK_R,
                );
                app.cells[col].dirty = true;
                app.cells[col].last_ink = Instant::now();
                app.last_stroke = Instant::now();
                app.last_pt = Some((s.x, s.y));
                let (x0, y0, x1, y1) = (ax.min(s.x), ay.min(s.y), ax.max(s.x), ay.max(s.y));
                ink_dirty = Some(match ink_dirty {
                    Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                    None => (x0, y0, x1, y1),
                });
            }
            // Coalesce panel pushes (default 2 ms) — snappier than 8 ms, safer than every sample.
            if let Some((x0, y0, x1, y1)) = ink_dirty {
                if last_flush.elapsed().as_millis() >= flush_ms {
                    let pad = PEN_R + 2;
                    let (ux, uy) = ((x0 - pad).max(0), (y0 - pad).max(0));
                    disp.update(
                        ux,
                        uy,
                        (x1 + pad - ux).min(R::W - ux),
                        (y1 + pad - uy).min(R::H - uy),
                        true,
                    );
                    ink_dirty = None;
                    last_flush = Instant::now();
                }
            }
        }

        // Flush remaining ink when pen went quiet (don't leave the last stroke stuck).
        if !pen_active {
            if let Some((x0, y0, x1, y1)) = ink_dirty.take() {
                let pad = PEN_R + 2;
                let (ux, uy) = ((x0 - pad).max(0), (y0 - pad).max(0));
                disp.update(
                    ux,
                    uy,
                    (x1 + pad - ux).min(R::W - ux),
                    (y1 + pad - uy).min(R::H - uy),
                    true,
                );
                last_flush = Instant::now();
            }
        }

        // --- batch recognition after global idle ---
        if !app.locked
            && !app.help
            && !app.confirm_new
            && !app.hint_popup
            && !app.gover
            && app.cells.iter().any(|c| c.dirty)
            && app.last_stroke.elapsed().as_millis() >= idle_ms
        {
            // Flush leftover pen ink first so we don't stack waveform work.
            if let Some((x0, y0, x1, y1)) = ink_dirty.take() {
                let pad = PEN_R + 2;
                let (ux, uy) = ((x0 - pad).max(0), (y0 - pad).max(0));
                disp.update(
                    ux,
                    uy,
                    (x1 + pad - ux).min(R::W - ux),
                    (y1 + pad - uy).min(R::H - uy),
                    true,
                );
            }
            for c in 0..COLS {
                if app.cells[c].dirty {
                    recognize_cell(&mut app, &engine, &mut scratch, c);
                }
            }
            app.focus = (0..COLS)
                .find(|&i| app.cells[i].letter.is_none())
                .unwrap_or(COLS - 1);
            // Enter first (fast waveform), then letters — so submit is tappable
            // without waiting for the whole row's slower paint to finish.
            redraw_buttons_fast(&mut app, &mut surf, &disp, &font);
            redraw_active_row_fast(&mut app, &mut surf, &disp, &font);
        }

        if let Some(t) = app.toast_until {
            if Instant::now() >= t {
                app.toast_until = None;
                R::draw_toast(&mut surf, &font, "");
                disp.update(0, R::TOAST_Y, R::W, R::TOAST_H, false);
            }
        }

        if let Some(ref mut t) = touch {
            if let touch::Gesture::Tap(x, y) = t.drain() {
                on_tap(
                    &mut app,
                    &mut surf,
                    &disp,
                    &font,
                    &engine,
                    &mut scratch,
                    &words,
                    soft_new,
                    x,
                    y,
                );
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

        // Adaptive poll: 1 ms while pen hot, 2 ms if dirty cells pending, 8 ms idle.
        let sleep_ms = if Instant::now() < app.pen_hot_until || pen_active {
            1
        } else if app.cells.iter().any(|c| c.dirty) || ink_dirty.is_some() {
            2
        } else if app.toast_until.is_some() {
            4
        } else {
            8
        };
        std::thread::sleep(Duration::from_millis(sleep_ms));
    }
    disp.terminate();
}


fn def_for<'a>(app: &'a App) -> Option<&'a str> {
    definitions::get(&app.defs, &app.game.answer)
}

/// Full board restore after closing a modal (hint / help / confirm / game-over).
fn restore_board(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    let enter_on = app.game.state == State::Playing
        && !app.locked
        && app.cells.iter().all(|c| c.letter.is_some());
    R::draw_all(
        surf,
        font,
        &app.game,
        &app.letters(),
        app.focus,
        enter_on,
        def_for(app),
    );
    R::draw_hint_button(
        surf,
        font,
        app.game.state == State::Playing && !app.locked,
    );
    disp.update_all(R::W as usize, R::H as usize);
}

fn recognize_cell(app: &mut App, engine: &Engine, scratch: &mut PredictScratch, c: usize) {
    let model = engine.get(Kind::Primary).unwrap();
    if let Ok(letters) =
        model.predict_letters_u8(&app.cells[c].ink, R::CELL as usize, R::CELL as usize, scratch)
    {
        app.cells[c].letter = letters.first().map(|&(ch, _)| ch);
        app.cells[c].top = letters.iter().take(3).cloned().collect();
    }
    app.cells[c].dirty = false;
}

fn redraw_active_row(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    redraw_active_row_mode(app, surf, disp, font, false);
}

/// Fast waveform — used right after recognition so the UI unlocks quickly.
fn redraw_active_row_fast(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    redraw_active_row_mode(app, surf, disp, font, true);
}

fn redraw_active_row_mode(
    app: &App,
    surf: &mut Surface,
    disp: &display::Display,
    font: &FontRef,
    fast: bool,
) {
    let r = app.game.filled_rows;
    for c in 0..COLS {
        let (x, y) = R::cell_xy(r, c);
        R::draw_cell(
            surf,
            font,
            x,
            y,
            app.cells[c].letter,
            game::Mark::Empty,
            c == app.focus,
        );
    }
    let (_, ry) = R::cell_xy(r, 0);
    disp.update(R::GRID_X0, ry, R::GRID_W, R::CELL, fast);
}

fn redraw_cell(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef, c: usize) {
    let (x, y) = R::cell_xy(app.game.filled_rows, c);
    R::draw_cell(
        surf,
        font,
        x,
        y,
        app.cells[c].letter,
        game::Mark::Empty,
        c == app.focus,
    );
    // Fast: cell clears/rewrites should feel instant.
    disp.update(x, y, R::CELL, R::CELL, true);
}

fn redraw_buttons(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    redraw_buttons_mode(app, surf, disp, font, false);
}

fn redraw_buttons_fast(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    redraw_buttons_mode(app, surf, disp, font, true);
}

fn redraw_buttons_mode(
    app: &App,
    surf: &mut Surface,
    disp: &display::Display,
    font: &FontRef,
    fast: bool,
) {
    let on = app.game.state == State::Playing
        && !app.locked
        && app.cells.iter().all(|c| c.letter.is_some());
    R::draw_buttons(surf, font, on);
    // Prefer a tight rect around the control bar (not the whole width if avoidable).
    // Full-width is still needed for layout, but fast mode keeps it snappy.
    disp.update(0, R::BTN_Y - 10, R::W, R::BTN_H + 40, fast);
}

fn toast(app: &mut App, surf: &mut Surface, disp: &display::Display, font: &FontRef, msg: &str) {
    R::draw_toast(surf, font, msg);
    disp.update(0, R::TOAST_Y, R::W, R::TOAST_H, false);
    app.toast_until = Some(Instant::now() + Duration::from_millis(1400));
}

/// After scoring: repaint only the bands that changed (not one huge upper-half rect).
fn redraw_after_submit(app: &App, surf: &mut Surface, disp: &display::Display, font: &FontRef) {
    R::draw_header(surf, font, &app.game, def_for(app));
    R::draw_hint_button(
        surf,
        font,
        app.game.state == State::Playing && !app.locked,
    );
    R::draw_grid(surf, font, &app.game, &app.letters(), app.focus);
    R::draw_tracker(surf, font, &app.game);
    R::draw_buttons(surf, font, false);
    // Header strip (includes Hint slot under Rules)
    disp.update(0, 0, R::W, R::GRID_Y0 - 4, false);
    // Full grid (scored row + next empty) — still cheaper than header→buttons mega-rect when
    // combined with separate tracker/button updates? One grid rect is clear and correct.
    let grid_h = R::GRID_BOTTOM - R::GRID_Y0 + 8;
    disp.update(R::GRID_X0 - 4, R::GRID_Y0 - 4, R::GRID_W + 8, grid_h, false);
    // Tracker
    let track_h = R::TRACK_STEP_Y * 2 + 8;
    disp.update(0, R::TRACK_Y - 4, R::W, track_h, false);
    // Buttons
    disp.update(0, R::BTN_Y - 10, R::W, R::BTN_H + 40, false);
}

fn on_tap(
    app: &mut App,
    surf: &mut Surface,
    disp: &display::Display,
    font: &FontRef,
    engine: &Engine,
    scratch: &mut PredictScratch,
    words: &Words,
    soft_new: bool,
    x: i32,
    y: i32,
) {
    // --- modal: hint popup (close only; board is frozen) ---
    if app.hint_popup {
        // Got it button OR tap outside the card closes; taps on the card body stay open.
        if R::hint_close_hit(x, y) || !R::hint_card_hit(x, y) {
            app.hint_popup = false;
            restore_board(app, surf, disp, font);
        }
        return;
    }
    if app.gover {
        if R::gameover_new_hit(x, y) {
            new_game(app, surf, disp, font, words, soft_new);
        } else {
            app.gover = false;
            restore_board(app, surf, disp, font);
        }
        return;
    }
    if app.confirm_new {
        let start = R::confirm_yes_hit(x, y);
        app.confirm_new = false;
        if start {
            new_game(app, surf, disp, font, words, soft_new);
        } else {
            restore_board(app, surf, disp, font);
        }
        return;
    }
    if app.help {
        app.help = false;
        restore_board(app, surf, disp, font);
        return;
    }
    if R::help_hit(x, y) {
        app.help = true;
        R::draw_help(surf, font);
        disp.update_all(R::W as usize, R::H as usize);
        return;
    }
    // Hint — only while actively playing (not after win/lose).
    if R::hint_hit(x, y) && app.game.state == State::Playing && !app.locked {
        app.hint_popup = true;
        let def = def_for(app).unwrap_or("");
        R::draw_hint_popup(surf, font, def);
        // Fast waveform: small card should appear quickly.
        disp.update(R::HP_X - 4, R::HP_Y - 4, R::HP_W + 8, R::HP_H + 60, true);
        return;
    }
    if R::quit_hit(x, y) {
        app.quit = true;
        return;
    }
    let btns = R::buttons(true);
    if btns[0].hit(x, y) {
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
        let in_progress = app.game.state == State::Playing
            && (app.game.filled_rows > 0
                || app.cells.iter().any(|c| c.letter.is_some() || c.dirty));
        if in_progress {
            app.confirm_new = true;
            R::draw_confirm_new(surf, font);
            disp.update(R::CF_X - 4, R::CF_Y - 4, R::CF_W + 8, R::CF_H + 8, false);
        } else {
            new_game(app, surf, disp, font, words, soft_new);
        }
        return;
    }
    if btns[1].hit(x, y) {
        submit(app, surf, disp, font, engine, scratch, words);
        return;
    }
    if let Some(c) = cell_at(&app.game, x, y) {
        app.cells[c].clear();
        app.focus = c;
        redraw_cell(app, surf, disp, font, c);
        redraw_buttons(app, surf, disp, font);
    }
}

fn submit(
    app: &mut App,
    surf: &mut Surface,
    disp: &display::Display,
    font: &FontRef,
    engine: &Engine,
    scratch: &mut PredictScratch,
    words: &Words,
) {
    if app.game.state != State::Playing || app.locked {
        return;
    }
    let had_dirty = app.cells.iter().any(|c| c.dirty);
    for c in 0..COLS {
        if app.cells[c].dirty {
            recognize_cell(app, engine, scratch, c);
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
    if corrected {
        for c in 0..COLS {
            if word[c] != chosen[c] {
                app.cells[c].letter = Some(word[c] as char);
                redraw_cell(app, surf, disp, font, c);
            }
        }
    }
    app.game.submit(word);
    app.reset_row();
    let over = app.game.state != State::Playing;
    if over {
        app.locked = true;
    }
    redraw_after_submit(app, surf, disp, font);
    if corrected && !over {
        let from: String = chosen
            .iter()
            .map(|&b| (b as char).to_ascii_uppercase())
            .collect();
        let to: String = word
            .iter()
            .map(|&b| (b as char).to_ascii_uppercase())
            .collect();
        toast(app, surf, disp, font, &format!("read {from} as {to}"));
    }
    if over {
        app.gover = true;
        let def = definitions::get(&app.defs, &app.game.answer).unwrap_or("");
        R::draw_gameover(surf, font, &app.game, def);
        disp.update(R::GO_X - 4, R::GO_Y - 4, R::GO_W + 8, R::GO_H + 90, false);
    }
}

fn new_game(
    app: &mut App,
    surf: &mut Surface,
    disp: &display::Display,
    font: &FontRef,
    words: &Words,
    soft_new: bool,
) {
    let mut s = seed();
    for &b in &app.game.answer {
        s = s.wrapping_mul(31).wrapping_add(b as u64);
    }
    let next = words.pick_avoiding(s, &app.recent);
    history::record(&mut app.recent, next);
    app.game = Game::new(next);
    app.reset_row();
    app.locked = false;
    app.gover = false;
    app.confirm_new = false;
    app.hint_popup = false;
    app.toast_until = None;
    restore_board(app, surf, disp, font);
    if !soft_new {
        disp.full_refresh(R::W as usize, R::H as usize);
    }
}
