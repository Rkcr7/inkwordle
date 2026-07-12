//! All drawing + layout geometry for Wordle. Fills carry the colour (they dither
//! and pop on Gallery 3); glyphs are always black for guaranteed contrast.

use crate::game::{Game, Mark, State, COLS, ROWS};
use crate::surface::{Surface, BLACK, BLUE, FADED, WHITE};
use crate::ui;
use ab_glyph::FontRef;

pub const W: i32 = 1620;
pub const H: i32 = 2160;

// grid geometry
pub const CELL: i32 = 200;
pub const GAP: i32 = 20;
pub const STEP: i32 = CELL + GAP; // 220
pub const GRID_W: i32 = COLS as i32 * CELL + (COLS as i32 - 1) * GAP; // 1080
pub const GRID_X0: i32 = (W - GRID_W) / 2; // 270
pub const GRID_Y0: i32 = 268;
pub const GRID_BOTTOM: i32 = GRID_Y0 + ROWS as i32 * CELL + (ROWS as i32 - 1) * GAP; // 1568

pub const TOAST_Y: i32 = 1584;
pub const TOAST_H: i32 = 52;

// letter tracker (A..Z as two rows of 13)
pub const TRACK_Y: i32 = 1656;
pub const TRACK_COLS: i32 = 13;
pub const TRACK_TILE_W: i32 = 108;
pub const TRACK_TILE_H: i32 = 80;
pub const TRACK_GAP: i32 = 10;
pub const TRACK_W: i32 = TRACK_COLS * TRACK_TILE_W + (TRACK_COLS - 1) * TRACK_GAP;
pub const TRACK_X0: i32 = (W - TRACK_W) / 2;
pub const TRACK_STEP_Y: i32 = TRACK_TILE_H + 12;

// control bar
pub const BTN_Y: i32 = 1902;
pub const BTN_H: i32 = 150;

// --- colours (fills pop; glyphs stay black) ---
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3)
}
pub const C_CORRECT: u16 = rgb565(0x3a, 0xa8, 0x56); // vivid green fill
pub const C_PRESENT: u16 = rgb565(0xe2, 0xa4, 0x22); // amber/gold fill
pub const C_ABSENT: u16 = rgb565(0x86, 0x8b, 0x92); // mid gray fill
pub const C_KEYBG: u16 = rgb565(0xe8, 0xe8, 0xe8); // unused tracker tile
pub const C_BORDER: u16 = rgb565(0x9a, 0x9e, 0xa4); // empty-cell hairline
pub const C_ACTIVE: u16 = BLUE; // active-cell accent (top pop stroke)
pub const C_DIM: u16 = rgb565(0x6c, 0x72, 0x7a); // dim text

pub fn cell_xy(r: usize, c: usize) -> (i32, i32) {
    (GRID_X0 + c as i32 * STEP, GRID_Y0 + r as i32 * STEP)
}

pub fn fill(m: Mark) -> u16 {
    match m {
        Mark::Correct => C_CORRECT,
        Mark::Present => C_PRESENT,
        Mark::Absent => C_ABSENT,
        Mark::Empty => WHITE,
    }
}

/// One tile: rounded look via a filled rect + border, big black centred glyph.
pub fn draw_cell(surf: &mut Surface, font: &FontRef, x: i32, y: i32, ch: Option<char>, m: Mark, active: bool) {
    let bg = fill(m);
    surf.fill_rect(x as usize, y as usize, CELL as usize, CELL as usize, bg);
    if m == Mark::Empty {
        // empty: hairline border; active: bold blue border
        if active {
            ui::border(surf, x, y, CELL, CELL, 6, C_ACTIVE);
        } else {
            ui::border(surf, x, y, CELL, CELL, 3, C_BORDER);
        }
    } else {
        ui::border(surf, x, y, CELL, CELL, 2, rgb565(0x50, 0x54, 0x58));
    }
    if let Some(c) = ch {
        let s = c.to_ascii_uppercase().to_string();
        ui::text_center(surf, font, x + CELL / 2, y + CELL / 2 - 78, 150.0, &s, BLACK);
    }
}

pub const QUIT_X: i32 = W - 200;
pub const QUIT_Y: i32 = 44;
pub const QUIT_W: i32 = 152;
pub const QUIT_H: i32 = 76;
pub fn quit_hit(x: i32, y: i32) -> bool {
    x >= QUIT_X && x < QUIT_X + QUIT_W && y >= QUIT_Y && y < QUIT_Y + QUIT_H
}

pub const HELP_X: i32 = 48;
pub const HELP_Y: i32 = 44;
pub const HELP_W: i32 = 170;
pub const HELP_H: i32 = 76;
pub fn help_hit(x: i32, y: i32) -> bool {
    x >= HELP_X && x < HELP_X + HELP_W && y >= HELP_Y && y < HELP_Y + HELP_H
}

// Hint — smaller, below Rules, so a palm/stray tap near the title won't open it.
pub const HINT_X: i32 = 48;
pub const HINT_Y: i32 = 132;
pub const HINT_W: i32 = 150;
pub const HINT_H: i32 = 64;
pub fn hint_hit(x: i32, y: i32) -> bool {
    x >= HINT_X && x < HINT_X + HINT_W && y >= HINT_Y && y < HINT_Y + HINT_H
}

// Hint popup — short card (not a full-board wall of UI).
pub const HP_W: i32 = 1000;
pub const HP_H: i32 = 420;
pub const HP_X: i32 = (W - HP_W) / 2;
pub const HP_Y: i32 = 720;
const HP_BTN_W: i32 = 360;
const HP_BTN_H: i32 = 110;
const HP_BTN_X: i32 = (W - HP_BTN_W) / 2;
const HP_BTN_Y: i32 = HP_Y + HP_H - 130;

/// True if the tap is on the hint popup's "Got it" button.
pub fn hint_close_hit(x: i32, y: i32) -> bool {
    x >= HP_BTN_X && x < HP_BTN_X + HP_BTN_W && y >= HP_BTN_Y && y < HP_BTN_Y + HP_BTN_H
}

/// True if the tap is inside the hint card (not the dimmed outside).
pub fn hint_card_hit(x: i32, y: i32) -> bool {
    x >= HP_X && x < HP_X + HP_W && y >= HP_Y && y < HP_Y + HP_H
}

/// Draw the small Hint control (only while a game is in progress).
pub fn draw_hint_button(surf: &mut Surface, font: &FontRef, enabled: bool) {
    // Clear the button slot so we don't ghost old ink under it.
    surf.fill_rect(
        HINT_X as usize,
        HINT_Y as usize,
        HINT_W as usize,
        HINT_H as usize,
        WHITE,
    );
    if !enabled {
        return;
    }
    // Solid dark control (same language as primary actions) so it reads clearly
    // on Gallery 3 — not a pale outline that washes out.
    surf.fill_rect(
        HINT_X as usize,
        HINT_Y as usize,
        HINT_W as usize,
        HINT_H as usize,
        BLACK,
    );
    ui::text_center(
        surf,
        font,
        HINT_X + HINT_W / 2,
        HINT_Y + 14,
        36.0,
        "Hint",
        WHITE,
    );
}

/// Compact modal: definition clue for the hidden answer (never shows the letters).
pub fn draw_hint_popup(surf: &mut Surface, font: &FontRef, definition: &str) {
    surf.fill_rect(HP_X as usize, HP_Y as usize, HP_W as usize, HP_H as usize, WHITE);
    ui::border(surf, HP_X, HP_Y, HP_W, HP_H, 4, BLACK);
    surf.fill_rect(HP_X as usize, HP_Y as usize, HP_W as usize, 14, C_ACTIVE);

    ui::text_center(surf, font, W / 2, HP_Y + 28, 56.0, "Hint", BLACK);
    ui::text_center(
        surf,
        font,
        W / 2,
        HP_Y + 90,
        30.0,
        "Clue only — not the letters",
        C_DIM,
    );

    let body = {
        let t = definition.trim();
        if t.is_empty() {
            "No hint is available for this word."
        } else {
            t
        }
    };
    // Compact body: 2–3 short lines, no giant grey panel.
    let max_w = (HP_W - 80) as f32;
    let lines = ui::wrap_lines(font, 38.0, body, max_w);
    let mut y = HP_Y + 140;
    for line in lines.iter().take(3) {
        ui::text_center(surf, font, W / 2, y, 38.0, line, BLACK);
        y += 44;
    }

    ui::Button::new(HP_BTN_X, HP_BTN_Y, HP_BTN_W, HP_BTN_H, "Got it").draw(surf, font, true);
    ui::text_center(
        surf,
        font,
        W / 2,
        HP_Y + HP_H + 22,
        28.0,
        "(tap outside to close)",
        C_DIM,
    );
}

/// A compact example colour tile with a letter + explanation, for the help overlay.
fn legend_row(surf: &mut Surface, font: &FontRef, x: i32, y: i32, m: Mark, letter: char, title: &str, desc: &str) {
    let sz = 92;
    surf.fill_rect(x as usize, y as usize, sz as usize, sz as usize, fill(m));
    ui::border(surf, x, y, sz, sz, 2, rgb565(0x50, 0x54, 0x58));
    ui::text_center(surf, font, x + sz / 2, y + sz / 2 - 34, 66.0, &letter.to_ascii_uppercase().to_string(), BLACK);
    ui::text(surf, font, x + sz + 32, y + 6, 42.0, title, BLACK);
    ui::text(surf, font, x + sz + 32, y + 52, 34.0, desc, C_DIM);
}

/// Full-screen "How to play" overlay. Closed by tapping anywhere / the Close button.
pub fn draw_help(surf: &mut Surface, font: &FontRef) {
    let lx = 140; // left margin for section content
    surf.fill_rect(0, 0, W as usize, H as usize, WHITE);
    ui::text_center(surf, font, W / 2, 44, 72.0, "How to Play", BLACK);
    surf.fill_rect((W / 2 - 300) as usize, 146, 600, 4, C_ACTIVE);

    ui::text_center(surf, font, W / 2, 196, 42.0, "Guess the hidden 5-letter word in 6 tries.", BLACK);
    ui::text_center(surf, font, W / 2, 252, 38.0, "Write each letter by hand — read on the tablet, no internet.", C_DIM);

    // --- tile colours ---
    ui::text(surf, font, lx, 336, 44.0, "What the tiles mean", BLACK);
    legend_row(surf, font, lx, 408, Mark::Correct, 'w', "Correct", "right letter, right spot");
    legend_row(surf, font, lx, 512, Mark::Present, 'o', "Present", "in the word, wrong spot");
    legend_row(surf, font, lx, 616, Mark::Absent, 'r', "Absent", "not in the word");

    // --- controls ---
    ui::text(surf, font, lx, 740, 44.0, "Playing", BLACK);
    ui::text(surf, font, lx, 806, 38.0, "Write freely across the boxes — even quickly.", BLACK);
    ui::text(surf, font, lx, 858, 38.0, "Erase a box (back of the pen) or tap it to redo.", BLACK);
    ui::text(surf, font, lx, 910, 38.0, "Enter submits. New = fresh word. Delete = clear a box.", BLACK);
    ui::text(surf, font, lx, 962, 38.0, "The A-Z tracker shows what you've ruled out.", BLACK);

    // --- which words count ---
    ui::text(surf, font, lx, 1074, 44.0, "Which words count?", BLACK);
    ui::text(surf, font, lx, 1140, 38.0, "Any real 5-letter English word is allowed:", BLACK);
    ui::text(surf, font, lx + 20, 1188, 34.0, "nouns, verbs, adjectives, plurals — table, climb, dusty, cakes", C_DIM);
    ui::text(surf, font, lx, 1248, 38.0, "Names of people, places & brands are NOT allowed:", BLACK);
    ui::text(surf, font, lx + 20, 1296, 34.0, "not valid — james, paris, delhi, tesla", C_DIM);
    ui::text(surf, font, lx, 1356, 38.0, "No made-up/misspelt words or abbreviations:", BLACK);
    ui::text(surf, font, lx + 20, 1404, 34.0, "not valid — snale, zzzzz, kerla", C_DIM);

    let close = ui::Button::new(W / 2 - 230, 1520, 460, 150, "Got it");
    close.draw(surf, font, true);
    ui::text_center(surf, font, W / 2, 1780, 34.0, "(tap anywhere to close)", C_DIM);
}

/// The display face for the "InkWordle" wordmark — an elegant flowing script,
/// distinct from the plain UI font, so the title reads like a logo.
fn title_font() -> FontRef<'static> {
    FontRef::try_from_slice(include_bytes!("../assets/title-font.ttf")).expect("title font")
}

pub fn draw_header(surf: &mut Surface, font: &FontRef, game: &Game, definition: Option<&str>) {
    surf.fill_rect(0, 0, W as usize, (GRID_Y0 - 8) as usize, WHITE);
    ui::text_center(surf, &title_font(), W / 2, 16, 108.0, "InkWordle", BLACK);
    // rules (top-left) + quit (top-right)
    ui::Button::new(HELP_X, HELP_Y, HELP_W, HELP_H, "Rules").draw(surf, font, false);
    ui::Button::new(QUIT_X, QUIT_Y, QUIT_W, QUIT_H, "Quit").draw(surf, font, false);
    // Hint sits under Rules (see draw_hint_button) — drawn by the caller when Playing.
    let sub = match game.state {
        State::Playing => format!("Guess {} of {}", (game.filled_rows + 1).min(ROWS), ROWS),
        State::Won => "Solved!".to_string(),
        State::Lost => {
            let a: String = game
                .answer
                .iter()
                .map(|&b| (b as char).to_ascii_uppercase())
                .collect();
            format!("The word was {a}")
        }
    };
    ui::text_center(surf, font, W / 2, 148, 40.0, &sub, C_DIM);
    // After the round ends, show a short meaning under the status (board review).
    if matches!(game.state, State::Won | State::Lost) {
        if let Some(def) = definition.map(str::trim).filter(|s| !s.is_empty()) {
            let max_w = (W - 80) as f32;
            // One or two compact lines so the grid still fits.
            let lines = ui::wrap_lines(font, 32.0, def, max_w);
            let show: Vec<&str> = lines.iter().map(|s| s.as_str()).take(2).collect();
            let mut y = 188;
            for line in show {
                ui::text_center(surf, font, W / 2, y, 32.0, line, C_DIM);
                y += 36;
            }
        }
    }
    // thin blue accent rule
    surf.fill_rect((GRID_X0) as usize, 236, GRID_W as usize, 4, C_ACTIVE);
}

/// Draw every grid row. `active` letters/marks come from the live cells (Empty
/// marks, letters shown in the active row); submitted rows use game.marks.
pub fn draw_grid(surf: &mut Surface, font: &FontRef, game: &Game, active_letters: &[Option<char>; COLS], focus: usize) {
    for r in 0..ROWS {
        let submitted = r < game.filled_rows;
        let is_active = r == game.filled_rows && game.state == State::Playing;
        for c in 0..COLS {
            let (x, y) = cell_xy(r, c);
            if submitted {
                let ch = Some(game.rows[r][c] as char);
                draw_cell(surf, font, x, y, ch, game.marks[r][c], false);
            } else if is_active {
                draw_cell(surf, font, x, y, active_letters[c], Mark::Empty, c == focus);
            } else {
                draw_cell(surf, font, x, y, None, Mark::Empty, false);
            }
        }
    }
}

pub fn draw_tracker(surf: &mut Surface, font: &FontRef, game: &Game) {
    surf.fill_rect(0, TRACK_Y as usize, W as usize, (2 * TRACK_STEP_Y) as usize, WHITE);
    for i in 0..26 {
        let row = (i / TRACK_COLS as usize) as i32;
        let col = (i % TRACK_COLS as usize) as i32;
        let x = TRACK_X0 + col * (TRACK_TILE_W + TRACK_GAP);
        let y = TRACK_Y + row * TRACK_STEP_Y;
        let m = game.letter[i];
        let bg = if m == Mark::Empty { C_KEYBG } else { fill(m) };
        surf.fill_rect(x as usize, y as usize, TRACK_TILE_W as usize, TRACK_TILE_H as usize, bg);
        let s = ((b'A' + i as u8) as char).to_string();
        ui::text_center(surf, font, x + TRACK_TILE_W / 2, y + 14, 52.0, &s, BLACK);
    }
}

/// The three control buttons. Returns their rects for hit-testing.
pub fn buttons(enter_on: bool) -> [ui::Button; 3] {
    let (m, g) = (150, 40);
    let total = W - 2 * m;
    let wide = 460; // Enter
    let side = (total - wide - 2 * g) / 2;
    let del = ui::Button::new(m, BTN_Y, side, BTN_H, "Delete");
    let _ = enter_on; // geometry is the same; label/lock is chosen in draw_buttons
    let enter = ui::Button::new(m + side + g, BTN_Y, wide, BTN_H, "Enter");
    let new = ui::Button::new(m + side + g + wide + g, BTN_Y, side, BTN_H, "New");
    [del, enter, new]
}

/// A small padlock, drawn from primitives — shown on the Enter button until all
/// 5 letters are in, so it reads clearly as "locked / not submittable yet".
fn draw_lock(surf: &mut Surface, cx: i32, cy: i32, color: u16) {
    let cy = cy - 8; // nudge up so the whole icon sits centred in the button
    // shackle: an annulus above the body; the body (drawn after) hides its lower half.
    let sy = cy - 18;
    surf.stamp(cx, sy, 30, color);
    surf.stamp(cx, sy, 21, WHITE);
    // body
    let (bw, bh) = (90, 66);
    let (bx, by) = (cx - bw / 2, cy - 4);
    surf.fill_rect(bx as usize, by as usize, bw as usize, bh as usize, color);
    // keyhole
    surf.stamp(cx, by + 26, 8, WHITE);
    surf.fill_rect((cx - 3) as usize, (by + 26) as usize, 6, 22, WHITE);
}

pub fn draw_buttons(surf: &mut Surface, font: &FontRef, enter_on: bool) {
    surf.fill_rect(0, (BTN_Y - 10) as usize, W as usize, (BTN_H + 40) as usize, WHITE);
    let b = buttons(enter_on);
    b[0].draw(surf, font, false);
    if enter_on {
        b[1].draw(surf, font, true); // filled "Enter" — the primary action
    } else {
        // locked: bordered box + padlock, no text
        let e = &b[1];
        surf.fill_rect(e.x as usize, e.y as usize, e.w as usize, e.h as usize, WHITE);
        ui::border(surf, e.x, e.y, e.w, e.h, 3, FADED);
        draw_lock(surf, e.x + e.w / 2, e.y + e.h / 2, C_DIM);
    }
    b[2].draw(surf, font, false);
}

pub fn draw_toast(surf: &mut Surface, font: &FontRef, msg: &str) {
    surf.fill_rect(0, TOAST_Y as usize, W as usize, TOAST_H as usize, WHITE);
    if !msg.is_empty() {
        ui::text_center(surf, font, W / 2, TOAST_Y + 4, 40.0, msg, crate::surface::ROSE);
    }
}

// --- game-over "pop" card (taller to fit a one-line / wrapped definition) ---
pub const GO_W: i32 = 1240;
pub const GO_H: i32 = 920;
pub const GO_X: i32 = (W - GO_W) / 2;
pub const GO_Y: i32 = 420;
const GO_BTN_W: i32 = 520;
const GO_BTN_H: i32 = 140;
const GO_BTN_X: i32 = (W - GO_BTN_W) / 2;
const GO_BTN_Y: i32 = GO_Y + GO_H - 170;

/// True if a tap hit the card's "New Game" button.
pub fn gameover_new_hit(x: i32, y: i32) -> bool {
    x >= GO_BTN_X && x < GO_BTN_X + GO_BTN_W && y >= GO_BTN_Y && y < GO_BTN_Y + GO_BTN_H
}

/// A prominent centred card shown when the game ends (win or loss). Shows the
/// answer word and a short plain-English definition underneath — always, for
/// both win and loss.
pub fn draw_gameover(surf: &mut Surface, font: &FontRef, game: &Game, definition: &str) {
    let won = game.state == State::Won;
    surf.fill_rect(GO_X as usize, GO_Y as usize, GO_W as usize, GO_H as usize, WHITE);
    ui::border(surf, GO_X, GO_Y, GO_W, GO_H, 5, BLACK);
    // coloured accent bar across the top: green win, gray loss
    let accent = if won { C_CORRECT } else { C_ABSENT };
    surf.fill_rect(GO_X as usize, GO_Y as usize, GO_W as usize, 22, accent);

    let title = if won { "You solved it!" } else { "Out of tries" };
    ui::text_center(surf, font, W / 2, GO_Y + 48, 84.0, title, BLACK);

    ui::text_center(surf, font, W / 2, GO_Y + 150, 38.0, "The word was", C_DIM);
    let word: String = game
        .answer
        .iter()
        .map(|&b| (b as char).to_ascii_uppercase())
        .collect();
    ui::text_center(surf, font, W / 2, GO_Y + 196, 112.0, &word, BLACK);

    // Meaning panel: always draw the band so the layout is stable.
    let panel_x = GO_X + 48;
    let panel_y = GO_Y + 360;
    let panel_w = GO_W - 96;
    let panel_h = 280;
    surf.fill_rect(
        panel_x as usize,
        panel_y as usize,
        panel_w as usize,
        panel_h as usize,
        rgb565(0xF4, 0xF6, 0xF8),
    );
    ui::border(surf, panel_x, panel_y, panel_w, panel_h, 2, C_BORDER);
    ui::text_center(surf, font, W / 2, panel_y + 24, 34.0, "Meaning", C_DIM);

    let def = definition.trim();
    let body = if def.is_empty() {
        "No definition available for this word."
    } else {
        def
    };
    let max_w = (panel_w - 48) as f32;
    // Up to ~4 lines of meaning, comfortably readable on e-ink.
    let lines = ui::wrap_lines(font, 40.0, body, max_w);
    let mut y = panel_y + 78;
    for line in lines.iter().take(4) {
        ui::text_center(surf, font, W / 2, y, 40.0, line, BLACK);
        y += 48;
    }

    let sub = if won {
        let n = game.filled_rows;
        format!("Solved in {} {}", n, if n == 1 { "guess" } else { "guesses" })
    } else {
        "Better luck next round".to_string()
    };
    ui::text_center(surf, font, W / 2, GO_BTN_Y - 64, 38.0, &sub, C_DIM);

    ui::Button::new(GO_BTN_X, GO_BTN_Y, GO_BTN_W, GO_BTN_H, "New Game").draw(surf, font, true);
    ui::text_center(
        surf,
        font,
        W / 2,
        GO_Y + GO_H + 28,
        32.0,
        "(tap outside to review your board)",
        C_DIM,
    );
}

// --- "start a new game?" confirmation card (guards the New button mid-game) ---
pub const CF_W: i32 = 1180;
pub const CF_H: i32 = 560;
pub const CF_X: i32 = (W - CF_W) / 2;
pub const CF_Y: i32 = 660;
const CF_BTN_W: i32 = 500;
const CF_BTN_H: i32 = 150;
const CF_BTN_Y: i32 = CF_Y + 360;
const CF_GAP: i32 = 40;
const CF_NO_X: i32 = CF_X + 60; // "Keep Playing" (cancel)
const CF_YES_X: i32 = CF_NO_X + CF_BTN_W + CF_GAP; // "New Game" (confirm)

pub fn confirm_no_hit(x: i32, y: i32) -> bool {
    x >= CF_NO_X && x < CF_NO_X + CF_BTN_W && y >= CF_BTN_Y && y < CF_BTN_Y + CF_BTN_H
}
pub fn confirm_yes_hit(x: i32, y: i32) -> bool {
    x >= CF_YES_X && x < CF_YES_X + CF_BTN_W && y >= CF_BTN_Y && y < CF_BTN_Y + CF_BTN_H
}

/// Centred confirmation shown when New is tapped during a game in progress, so an
/// accidental tap can't discard it. Overlays the board.
pub fn draw_confirm_new(surf: &mut Surface, font: &FontRef) {
    surf.fill_rect(CF_X as usize, CF_Y as usize, CF_W as usize, CF_H as usize, WHITE);
    ui::border(surf, CF_X, CF_Y, CF_W, CF_H, 5, BLACK);
    surf.fill_rect(CF_X as usize, CF_Y as usize, CF_W as usize, 22, C_ACTIVE);

    ui::text_center(surf, font, W / 2, CF_Y + 96, 84.0, "Start a new game?", BLACK);
    ui::text_center(surf, font, W / 2, CF_Y + 236, 46.0, "This ends your current game.", C_DIM);

    ui::Button::new(CF_NO_X, CF_BTN_Y, CF_BTN_W, CF_BTN_H, "Keep Playing").draw(surf, font, false);
    ui::Button::new(CF_YES_X, CF_BTN_Y, CF_BTN_W, CF_BTN_H, "New Game").draw(surf, font, true);
}

/// Full repaint (new game / start). `definition` is shown under the header after
/// the round ends (win or loss) so the meaning stays visible when reviewing.
pub fn draw_all(
    surf: &mut Surface,
    font: &FontRef,
    game: &Game,
    active_letters: &[Option<char>; COLS],
    focus: usize,
    enter_on: bool,
    definition: Option<&str>,
) {
    surf.fill_rect(0, 0, W as usize, H as usize, WHITE);
    draw_header(surf, font, game, definition);
    draw_grid(surf, font, game, active_letters, focus);
    draw_toast(surf, font, "");
    draw_tracker(surf, font, game);
    draw_buttons(surf, font, enter_on);
}
