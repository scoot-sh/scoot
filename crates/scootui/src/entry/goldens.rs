//! Snapshot goldens per widget state, read back from pixels, plus the
//! proof that the list draws only its visible window.
//!
//! Each golden is an FNV-1a hash of the whole canvas: any rendering
//! change alters it, so the constant pins the pixels. Beside each hash
//! sit targeted pixel asserts (ring, caret, fills), so a failure says
//! what moved, not just that something did. Canvases are filled with a
//! background wash first, so undrawn corners pin too.
//!
//! The goldens below are placeholders (`0`) until first measured: run,
//! read the actual hashes from the failure output, fill them in, and
//! re-run green. They are stable across runs (the test font is built
//! in code, the theme is `Theme::default()`).

use core::cell::Cell;

use ab_glyph::{FontArc, FontVec};

use super::line::{Entry, InsertOutcome, PasswordEntry};
use super::list::{List, Selector};
use super::paint::{Geometry, paint_entry, paint_list, paint_password, paint_selector};
use crate::color::Color;
use crate::paint::{Canvas, Span};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const EM: f32 = 20.0;
const ROW_H: u32 = 28;
const WIDTH: u32 = 400;
/// Half an em: the gap between a widget's edge and its text.
const INSET: u32 = 10;

const GOLDEN_ENTRY_EMPTY_FOCUSED: u64 = 16614668564813265701;
const GOLDEN_ENTRY_TEXT_CARET_MID: u64 = 6727157936884326765;
const GOLDEN_ENTRY_SELECTED_ALL: u64 = 11087678023190917221;
const GOLDEN_ENTRY_UNFOCUSED: u64 = 5902647548692030597;
const GOLDEN_PASSWORD_MASKED: u64 = 15244525020742787061;
const GOLDEN_LIST_SELECTION_HIGHLIGHTS: u64 = 5463997336095684813;
const GOLDEN_LIST_SCROLLED: u64 = 5428548557355056837;
const GOLDEN_SELECTOR_COLLAPSED: u64 = 3465626071466733637;
const GOLDEN_SELECTOR_EXPANDED: u64 = 17309038250503822317;

fn text() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ))
}

fn theme() -> Theme {
    Theme::default()
}

fn rgb(color: Color) -> [u8; 3] {
    [color.r, color.g, color.b]
}

/// A canvas washed with the background token: undrawn corners pin too.
fn washed_canvas(pixels: &mut [u8], width: u32, height: u32) -> Canvas<'_> {
    let mut canvas = Canvas::new(pixels, width, height).unwrap();
    canvas.fill_rect(Span { x: 0, width }, 0, height, Theme::default().background);
    canvas
}

fn hash(canvas: &Canvas<'_>, width: u32, height: u32) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for y in 0..height {
        for x in 0..width {
            for byte in canvas.at(x, y) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
    }
    hash
}

/// Whether any pixel in the box is `color`: robust ink-presence for
/// glyph-dependent positions (the exact inked pixel inside a glyph
/// varies with the font; its presence in the glyph's cell does not).
fn any_in(canvas: &Canvas<'_>, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 3]) -> bool {
    for y in y0..y1 {
        for x in x0..x1 {
            if canvas.at(x, y) == color {
                return true;
            }
        }
    }
    false
}

fn entry_geo() -> Geometry {
    Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
    }
}

#[test]
fn golden_entry_empty_focused() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let entry = Entry::new(64);
    let geo = entry_geo();
    paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "Search");
    // The placeholder draws dim ink somewhere in the line.
    assert!(any_in(
        &canvas,
        geo.x,
        geo.y,
        geo.x + geo.width,
        geo.y + geo.row_height,
        rgb(theme.dim)
    ));
    // The ring's left edge is accent (an empty line draws no caret).
    assert_eq!(canvas.at(geo.x + 1, geo.y + ROW_H / 2), rgb(theme.accent));
    // The focus ring runs the top edge in accent.
    assert_eq!(canvas.at(geo.x + 4, geo.y), rgb(theme.accent));
    assert_eq!(hash(&canvas, WIDTH, ROW_H + 16), GOLDEN_ENTRY_EMPTY_FOCUSED);
}

#[test]
fn the_first_letter_keeps_clear_of_the_ring() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let mut entry = Entry::new(64);
    assert_eq!(entry.insert_text("Hello"), InsertOutcome::Inserted);
    let geo = entry_geo();
    paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "");
    // The ring is 2 px wide; between it and the text edge stays clear of
    // ink, on every row inside the ring.
    for y in geo.y + 2..geo.y + geo.row_height - 2 {
        for x in geo.x + 2..geo.x + INSET {
            assert_ne!(canvas.at(x, y), rgb(theme.foreground), "ink at ({x},{y})");
        }
    }
    assert!(any_in(
        &canvas,
        geo.x + INSET,
        geo.y + 2,
        geo.x + INSET + 40,
        geo.y + geo.row_height - 2,
        rgb(theme.foreground)
    ));
}

#[test]
fn golden_entry_text_caret_mid() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let mut entry = Entry::new(64);
    assert_eq!(entry.insert_text("a1:mp 09"), InsertOutcome::Inserted);
    entry.move_left();
    entry.move_left();
    entry.move_left();
    let geo = entry_geo();
    paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "");
    // Text ink in foreground, caret bar in accent right after "a1:m".
    assert!(any_in(
        &canvas,
        geo.x,
        geo.y,
        geo.x + geo.width,
        geo.y + geo.row_height,
        rgb(theme.foreground)
    ));
    let caret = geo.x + INSET + text.measure(None, "a1:mp", EM);
    assert_eq!(canvas.at(caret + 1, geo.y + ROW_H / 2), rgb(theme.accent));
    assert_eq!(
        hash(&canvas, WIDTH, ROW_H + 16),
        GOLDEN_ENTRY_TEXT_CARET_MID
    );
}

#[test]
fn golden_entry_selected_all() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let mut entry = Entry::new(64);
    assert_eq!(entry.insert_text("a1mp"), InsertOutcome::Inserted);
    entry.select_all();
    let geo = entry_geo();
    paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "");
    // The selection fill sits under the text in accent.
    assert_eq!(canvas.at(geo.x + INSET + 1, geo.y + 1), rgb(theme.accent));
    assert_eq!(hash(&canvas, WIDTH, ROW_H + 16), GOLDEN_ENTRY_SELECTED_ALL);
}

#[test]
fn golden_entry_unfocused() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let mut entry = Entry::new(64);
    assert_eq!(entry.insert_text("09"), InsertOutcome::Inserted);
    let geo = entry_geo();
    paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, false, "");
    // No ring: the top edge stays background.
    assert_eq!(canvas.at(geo.x + 4, geo.y), rgb(theme.background));
    // No caret: past the text end sits background, not accent.
    let end = geo.x + INSET + text.measure(None, "09", EM);
    assert_eq!(canvas.at(end + 4, geo.y + ROW_H / 2), rgb(theme.background));
    assert_eq!(hash(&canvas, WIDTH, ROW_H + 16), GOLDEN_ENTRY_UNFOCUSED);
}

#[test]
fn golden_password_masked() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, ROW_H + 16);
    let mut text = text();
    let mut backing = [0u8; 64];
    let mut entry = PasswordEntry::new(&mut backing);
    assert_eq!(entry.insert_text("hunter2"), InsertOutcome::Inserted);
    let geo = entry_geo();
    paint_password(&mut canvas, &mut text, &theme, &entry, &geo, true);
    // Seven mask blocks draw foreground ink; no secret letterform:
    // the mask cells hold the font's `.notdef`, and the seventh cell
    // onward is empty background.
    assert!(any_in(
        &canvas,
        geo.x,
        geo.y,
        geo.x + 200,
        geo.y + geo.row_height,
        rgb(theme.foreground)
    ));
    let mask_end = geo.x + INSET + (7.0 * text.advance(super::line::MASK_CHAR, EM)).ceil() as u32;
    assert_eq!(
        canvas.at(mask_end + 12, geo.y + ROW_H / 2),
        rgb(theme.background)
    );
    assert_eq!(hash(&canvas, WIDTH, ROW_H + 16), GOLDEN_PASSWORD_MASKED);
    entry.clear();
}

const ROWS: [&str; 12] = [
    "a1pine", "mpv", "09er", "am:pm", "1a2", "p0", "m3", "a9", "01", "pa", "mm", "a0",
];

fn list_geo() -> Geometry {
    Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
    }
}

fn paint_rows_case(
    pixels: &mut [u8],
    height: u32,
    selected: usize,
    first: usize,
    highlights: &[(usize, &[(u32, u32)])],
    focused: bool,
) -> (u64, Theme) {
    let theme = theme();
    let mut canvas = washed_canvas(pixels, WIDTH, height);
    let mut text = text();
    let mut list = List::new(4);
    list.set_count(ROWS.len() * 1000);
    list.select(selected);
    list.scroll_to(first);
    list.set_highlights(highlights);
    let geo = list_geo();
    paint_list(
        &mut canvas,
        &mut text,
        &theme,
        &list,
        &geo,
        &|row| ROWS[row % ROWS.len()],
        focused,
    );
    (hash(&canvas, WIDTH, height), theme)
}

#[test]
fn golden_list_selection_highlights() {
    let height = 8 + ROW_H * 4 + 8;
    let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
    let spans: [(u32, u32); 2] = [(0, 2), (3, 5)];
    let hi: [(usize, &[(u32, u32)]); 2] = [(1, &spans[..]), (2, &spans[..1])];
    let (digest, theme) = paint_rows_case(&mut pixels, height, 1, 0, &hi, true);
    let canvas = Canvas::new(&mut pixels, WIDTH, height).unwrap();
    // The selected row (slot 1) fills accent; a match span inside it
    // inks urgent; the plain row's span inks hover.
    assert_eq!(canvas.at(10, 8 + ROW_H + 2), rgb(theme.accent));
    assert!(any_in(
        &canvas,
        8,
        8 + ROW_H,
        WIDTH - 8,
        8 + ROW_H * 2,
        rgb(theme.urgent)
    ));
    assert!(any_in(
        &canvas,
        8,
        8 + ROW_H * 2,
        WIDTH - 8,
        8 + ROW_H * 3,
        rgb(theme.hover)
    ));
    assert_eq!(digest, GOLDEN_LIST_SELECTION_HIGHLIGHTS);
}

#[test]
fn golden_list_scrolled() {
    let height = 8 + ROW_H * 4 + 8;
    let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
    let (digest, theme) = paint_rows_case(&mut pixels, height, 6000, 5999, &[], false);
    let canvas = Canvas::new(&mut pixels, WIDTH, height).unwrap();
    // Scrolled deep into 12,000 rows: the window shows rows
    // 5999..6003 whatever the count. Unfocused, so the left strip over
    // the unselected first row is background, not the accent ring.
    assert!(!any_in(&canvas, 8, 8, 10, 8 + ROW_H, rgb(theme.accent)));
    assert_eq!(digest, GOLDEN_LIST_SCROLLED);
}

#[test]
fn golden_selector_collapsed_and_expanded() {
    let theme = theme();
    let geo = Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
    };
    // Collapsed: one line with the choice.
    let height = 8 + ROW_H + 8;
    let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, height);
    let mut text = text();
    let mut selector = Selector::new(4);
    selector.list_mut().set_count(ROWS.len());
    selector.list_mut().select(2);
    paint_selector(
        &mut canvas,
        &mut text,
        &theme,
        &selector,
        &geo,
        &|row| ROWS[row % ROWS.len()],
        "no user",
        true,
    );
    assert!(any_in(
        &canvas,
        geo.x,
        geo.y,
        geo.x + geo.width,
        geo.y + geo.row_height,
        rgb(theme.foreground)
    ));
    assert_eq!(hash(&canvas, WIDTH, height), GOLDEN_SELECTOR_COLLAPSED);
    // Expanded: the line plus the rows below it.
    let height = 8 + ROW_H * (1 + 4) + 8;
    let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, height);
    selector.expand();
    paint_selector(
        &mut canvas,
        &mut text,
        &theme,
        &selector,
        &geo,
        &|row| ROWS[row % ROWS.len()],
        "no user",
        true,
    );
    // The selected row below the line fills accent.
    assert_eq!(canvas.at(10, 8 + ROW_H * 3 + 2), rgb(theme.accent));
    assert_eq!(hash(&canvas, WIDTH, height), GOLDEN_SELECTOR_EXPANDED);
}

#[test]
fn the_list_draw_reads_only_its_visible_window() {
    let theme = theme();
    let height = 8 + ROW_H * 10 + 8;
    let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, height);
    let mut text = text();
    let mut list = List::new(10);
    list.set_count(65_536);
    list.scroll_to(60_000);
    // Fixed-size, stack-side record: observing the draw allocates
    // nothing itself.
    let seen: Cell<[usize; 10]> = Cell::new([usize::MAX; 10]);
    let count: Cell<usize> = Cell::new(0);
    let geo = Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
    };
    paint_list(
        &mut canvas,
        &mut text,
        &theme,
        &list,
        &geo,
        &|row| {
            let i = count.get();
            if i < 10 {
                let mut record = seen.get();
                record[i] = row;
                seen.set(record);
            }
            count.set(i + 1);
            ROWS[row % ROWS.len()]
        },
        true,
    );
    // Exactly the ten visible rows, in order, out of 65,536.
    assert_eq!(count.get(), 10);
    assert_eq!(
        seen.get(),
        [
            60000, 60001, 60002, 60003, 60004, 60005, 60006, 60007, 60008, 60009
        ]
    );
}

#[test]
fn hostile_spans_and_tiny_canvases_never_panic_the_draw() {
    let theme = theme();
    let mut pixels = vec![0u8; (WIDTH * 64) as usize * 4];
    let mut canvas = washed_canvas(&mut pixels, WIDTH, 64);
    let mut text = text();
    let mut list = List::new(4);
    list.set_count(3);
    // Spans past the label, inverted, and at u32::MAX: clipped or
    // skipped, never a panic.
    let spans: [(u32, u32); 4] = [(0, 99), (5, 2), (u32::MAX - 1, u32::MAX), (1, 1)];
    let hi: [(usize, &[(u32, u32)]); 3] = [(0, &spans[..]), (1, &spans[..]), (2, &spans[..])];
    list.set_highlights(&hi);
    let geo = Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
    };
    paint_list(
        &mut canvas,
        &mut text,
        &theme,
        &list,
        &geo,
        &|row| ROWS[row % ROWS.len()],
        true,
    );
    // A zero-width canvas and a zero row height draw nothing, not a panic.
    let mut empty = [0u8; 4];
    if let Some(mut canvas) = Canvas::new(&mut empty[..], 1, 1) {
        let flat = Geometry {
            x: 0,
            y: 0,
            width: 0,
            row_height: 0,
            em: EM,
        };
        let mut entry = Entry::new(8);
        paint_entry(&mut canvas, &mut text, &theme, &entry, &flat, true, "");
        entry.insert_text("a1");
        paint_list(
            &mut canvas,
            &mut text,
            &theme,
            &list,
            &flat,
            &|row| ROWS[row % ROWS.len()],
            true,
        );
    }
}
