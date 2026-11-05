//! The apps panel is as tall as the rows it shows: drawn at the top of a
//! surface that keeps the height of all of them, with nothing below. These
//! tests draw it at chosen match counts and read the pixels back, and pin
//! that a count that changes from key to key costs the draw no allocation
//! a steady one did not.

use std::time::Instant;

use scootui::density::Scale;
use scootui::effects::ShadowStyle;
use scootui::motion::Curve;
use scootui::output::Size;
use scootui::text::Text;

use super::*;
use crate::layout::{MIN_ROWS, scale_len, shown_rows};
use crate::rank::frecency::Frecency;

const ROWS: u32 = 10;
const ENTRIES: usize = 40;

/// A panel over `ENTRIES` entries ("Application number N"), its look and
/// its effects, drawn into a buffer the size of the surface.
struct Rig {
    _dir: tempfile::TempDir,
    text: Text,
    icons: crate::icons::Cache,
    ui: Ui,
    look: Look,
    fx: Effects,
    scale: Scale,
    width: u32,
    height: u32,
    out: Vec<u8>,
}

impl Rig {
    fn new(look: Look, scale: Scale) -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let text = super::bytes_tests::test_text(dir.path());
        let ui = Ui::open(
            super::bytes_tests::entries(ENTRIES),
            Frecency::empty(),
            0,
            ROWS as usize,
        );
        let layout = Layout::new(look.font_size_px, look.radius, Scale::Integer(1));
        let width = (30.0 * look.font_size_px) as u32;
        let height = layout.height(ROWS);
        let fx = Effects::from_look(&look);
        let mut rig = Self {
            _dir: dir,
            text,
            icons: crate::icons::Cache::default(),
            ui,
            look,
            fx,
            scale,
            width,
            height,
            out: Vec::new(),
        };
        let (_, dims) = rig.surface();
        rig.out = vec![0xAA; dims.0 as usize * dims.1 as usize * 4];
        rig
    }

    fn surface(&self) -> (Size, (u32, u32)) {
        let size = Size {
            width: self.width + 2 * self.fx.room.0,
            height: self.height + 2 * self.fx.room.1,
        };
        let dims = (
            scale_len(size.width, self.scale),
            scale_len(size.height, self.scale),
        );
        (size, dims)
    }

    fn type_text(&mut self, text: &str) {
        for char in text.chars() {
            let mut piece = [0u8; 4];
            self.ui.key(0, false, char.encode_utf8(&mut piece));
        }
    }

    fn backspace(&mut self, times: usize) {
        for _ in 0..times {
            self.ui.key(scootui::entry::KEY_BACKSPACE, false, "");
        }
    }

    fn shown(&self) -> u32 {
        shown_rows(self.ui.list().count(), ROWS)
    }

    /// One key's draw into the rig's own buffer: `shrink` as the app
    /// shrinks, else every row (the panel as it was before it followed its
    /// matches). Returns the panel's origin in the surface.
    fn frame(&mut self, shrink: bool) -> (u32, u32) {
        let (size, dims) = self.surface();
        let shown = if shrink { self.shown() } else { ROWS };
        let view = PanelView {
            ui: &self.ui,
            look: &self.look,
            width: self.width,
            height: self.height,
            rows: ROWS,
            shown,
            error: None,
            empty_text: "empty",
            scale: self.scale,
            icon_theme: "Fixture",
            icon_bases: &[],
        };
        if self.fx.stage.is_some() {
            staged::paint_staged(
                &mut self.fx,
                &view,
                &mut self.text,
                &mut self.icons,
                &mut self.out,
                dims,
                size,
                true,
                Instant::now(),
            )
        } else {
            paint_panel(
                &mut self.out,
                dims,
                &view,
                &mut self.text,
                &mut self.icons,
                None,
            );
            (0, 0)
        }
    }

    /// The device height of the panel at `shown` rows.
    fn drawn_height(&self, shown: u32) -> u32 {
        if shown >= ROWS {
            scale_len(self.height, self.scale)
        } else {
            device_layout(&self.look, self.scale, self.height, ROWS).height(shown)
        }
    }

    /// The alpha down the surface's middle column, top to bottom.
    fn column(&self) -> Vec<u8> {
        let (_, dims) = self.surface();
        let x = dims.0 / 2;
        (0..dims.1)
            .map(|y| self.out[(y * dims.0 + x) as usize * 4 + 3])
            .collect()
    }
}

fn soft_moving() -> Look {
    Look {
        shadow: ShadowStyle::Soft,
        motion: Curve::Spring,
        motion_ms: 160,
        ..Look::default()
    }
}

#[test]
fn the_shown_rows_follow_the_matches_between_the_minimum_and_all() {
    assert_eq!(shown_rows(0, 10), MIN_ROWS);
    assert_eq!(shown_rows(1, 10), MIN_ROWS);
    assert_eq!(shown_rows(MIN_ROWS as usize, 10), MIN_ROWS);
    assert_eq!(shown_rows(4, 10), 4);
    assert_eq!(shown_rows(10, 10), 10);
    assert_eq!(shown_rows(11, 10), 10);
    assert_eq!(shown_rows(usize::MAX, 10), 10);
    // A panel configured with fewer rows than the minimum is never taller
    // than that.
    for rows in 1..MIN_ROWS {
        assert_eq!(shown_rows(0, rows), rows);
        assert_eq!(shown_rows(50, rows), rows);
    }
}

#[test]
fn a_short_list_draws_a_short_panel_with_nothing_below_it() {
    for scale in [Scale::Integer(1), Scale::Fractional(180), Scale::Integer(2)] {
        let mut rig = Rig::new(Look::default(), scale);
        let full = rig.frame(true);
        assert_eq!(rig.shown(), ROWS);
        let full_column = rig.column();
        rig.type_text("number 39");
        assert!(rig.shown() < ROWS, "the query narrows the list");
        let origin = rig.frame(true);
        // The field stays where it was: the panel is pinned to the top.
        assert_eq!(origin, full);
        let column = rig.column();
        let drawn = rig.drawn_height(rig.shown()) as usize;
        assert!(drawn < full_column.iter().filter(|a| **a != 0).count());
        assert!(
            column[..drawn].iter().all(|a| *a != 0),
            "{scale:?}: the panel is opaque down its drawn height"
        );
        assert!(
            column[drawn..].iter().all(|a| *a == 0),
            "{scale:?}: nothing, not the last frame's rows, below it"
        );
    }
}

#[test]
fn growing_back_repaints_the_rows_a_shrink_cleared() {
    let mut rig = Rig::new(Look::default(), Scale::Integer(1));
    rig.frame(true);
    let full = rig.column();
    rig.type_text("number 39");
    rig.frame(true);
    let query = rig.ui.query().chars().count();
    rig.backspace(query);
    rig.frame(true);
    assert_eq!(rig.column(), full);
}

#[test]
fn a_staged_panel_keeps_its_shadow_room_and_origin_as_it_shrinks() {
    let mut rig = Rig::new(soft_moving(), Scale::Integer(1));
    let full = rig.frame(true);
    assert_eq!(full, (12, 16));
    rig.type_text("number 39");
    let shrunk = rig.frame(true);
    assert_eq!(shrunk, full);
    let drawn = rig.drawn_height(rig.shown()) as usize;
    let column = rig.column();
    let room = 16usize;
    // The panel, then its shadow's room, then nothing to the surface's end.
    assert!(column[room..room + drawn].iter().all(|a| *a != 0));
    assert!(
        column[room + drawn + room..].iter().all(|a| *a == 0),
        "clear below the panel's shadow"
    );
}

#[test]
fn the_input_region_follows_the_drawn_panel() {
    let surface = (400, 300);
    // Whole panel, no room: the default region.
    assert_eq!(
        staged_region(false, (0, 0), surface, surface),
        Region::Surface
    );
    // A short panel at the top of the surface.
    assert_eq!(
        staged_region(false, (0, 0), (400, 120), surface),
        Region::Panel(Rect {
            x: 0,
            y: 0,
            width: 400,
            height: 120
        })
    );
    // Inside a shadow's room.
    assert_eq!(
        staged_region(false, (12, 16), (400, 120), surface),
        Region::Panel(Rect {
            x: 12,
            y: 16,
            width: 400,
            height: 120
        })
    );
    assert_eq!(
        staged_region(true, (0, 0), surface, surface),
        Region::Nothing
    );
}

/// The shrunk panel's draw allocates what the whole panel's draw does at
/// the same keys: a shrink or a growth costs nothing extra. Counted over
/// the second pass of one key sequence (the first warms the glyph caches
/// and the stage's layers), on the plain path and on a staged one.
#[cfg(target_os = "linux")]
#[test]
fn a_changing_row_count_allocates_no_more_than_a_steady_one() {
    for (name, look) in [("plain", Look::default()), ("staged", soft_moving())] {
        let mut counts = [0usize; 2];
        let mut micros = [0u128; 2];
        for (slot, shrink) in [false, true].into_iter().enumerate() {
            let mut rig = Rig::new(look.clone(), Scale::Integer(1));
            let mut keys = Vec::new();
            // 40 matches down to a handful and back: 10 -> 3 -> 10 rows.
            let sequence = |rig: &mut Rig, tally: &mut Vec<u32>| {
                for char in "number 39".chars() {
                    rig.type_text(&char.to_string());
                    rig.frame(shrink);
                    tally.push(rig.shown());
                }
                let query = rig.ui.query().chars().count();
                for _ in 0..query {
                    rig.backspace(1);
                    rig.frame(shrink);
                    tally.push(rig.shown());
                }
            };
            rig.frame(shrink);
            sequence(&mut rig, &mut keys);
            keys.clear();
            let started = Instant::now();
            let (_, allocs) = scootbg_mem::count_allocations(|| sequence(&mut rig, &mut keys));
            micros[slot] = started.elapsed().as_micros();
            counts[slot] = allocs;
            if shrink {
                assert!(
                    keys.contains(&ROWS) && keys.iter().any(|shown| *shown < ROWS),
                    "{name}: the sequence crosses the shown rows: {keys:?}"
                );
            }
        }
        eprintln!(
            "{name}: allocations over 18 key frames: whole panel {} shrinking {}; \
             {} us vs {} us",
            counts[0], counts[1], micros[0], micros[1]
        );
        assert!(
            counts[1] <= counts[0],
            "{name}: shrinking allocates {} over the whole panel's {}",
            counts[1],
            counts[0]
        );
    }
}
