//! The scale wiring and the geometry that rides on it: which scale wins,
//! when a redraw is asked for, where a click lands on rows drawn at a
//! fractional scale, and that the device layout fits the buffer.

use scootui::density::Scale;

use crate::dmenu::DmenuUi;
use crate::layout::{Layout, scale_len};
use crate::look::Look;
use crate::panel::{App, LaunchCtx, Model, device_layout, dmenu_layout, row_at};
use crate::stage::Effects;

/// The scales the launcher meets: integer, the fractional ones laptops
/// ship with, and 3.
fn scales() -> [Scale; 6] {
    [
        Scale::Integer(1),
        Scale::Fractional(150),
        Scale::Fractional(180),
        Scale::Fractional(210),
        Scale::Integer(2),
        Scale::Integer(3),
    ]
}

fn app() -> App {
    let launch = LaunchCtx {
        state_home: None,
        home: None,
        display_set: false,
    };
    let look = Look::default();
    let layout = Layout::dmenu(look.font_size_px);
    App::new(
        None,
        Model::Dmenu(DmenuUi::open(vec![String::from("a")], 6, false, false)),
        300,
        layout.height(6),
        layout,
        6,
        launch,
        None,
        "",
        String::new(),
        Vec::new(),
        Effects::off(),
    )
}

#[test]
fn a_change_of_the_scale_to_draw_at_asks_for_a_redraw_and_nothing_else_does() {
    let mut app = app();
    app.dirty = false;
    app.rescale(|demand| demand.prefer_buffer_scale(2));
    assert!(app.dirty, "1 to 2 redraws");
    assert_eq!(app.demand.scale(), Scale::Integer(2));

    app.dirty = false;
    app.rescale(|demand| demand.prefer_buffer_scale(2));
    assert!(!app.dirty, "the same preference again is not a change");

    app.rescale(|demand| demand.prefer_fractional(180));
    assert!(app.dirty, "a fraction under the integer scale redraws");
    // 1.5 rounds up to 2, which is the surface's own integer scale: the
    // fraction wins, as the best the compositor said.
    assert_eq!(app.demand.scale(), Scale::Fractional(180));

    app.dirty = false;
    app.rescale(|demand| demand.prefer_fractional(0));
    app.rescale(|demand| demand.prefer_buffer_scale(0));
    app.rescale(|demand| demand.prefer_buffer_scale(-1));
    assert!(!app.dirty, "a broken scale is ignored");
    assert_eq!(app.demand.scale(), Scale::Fractional(180));

    // Back to 1x while open: the fractional wish is replaced.
    app.rescale(|demand| demand.prefer_fractional(120));
    app.rescale(|demand| demand.prefer_buffer_scale(1));
    assert_eq!(app.demand.scale().factor(), 1.0);
}

#[test]
fn an_entered_output_backs_up_a_compositor_that_sends_no_preference() {
    let mut app = app();
    app.dirty = false;
    app.rescale_to(None);
    assert!(!app.dirty, "on no output yet: nothing to apply");
    app.rescale_to(Some(2));
    assert!(app.dirty);
    assert_eq!(app.demand.scale(), Scale::Integer(2));
    // A surface preference above the output's wins; below it, the output's.
    app.rescale(|demand| demand.prefer_buffer_scale(3));
    assert_eq!(app.demand.scale(), Scale::Integer(3));
    app.rescale(|demand| demand.prefer_buffer_scale(1));
    assert_eq!(app.demand.scale(), Scale::Integer(2));
    // Leaving every output keeps the last: the surface did not change.
    app.rescale_to(None);
    assert_eq!(app.demand.scale(), Scale::Integer(2));
}

/// The panel's logical width and height and its device layout for `rows`,
/// the way `open` and `App::draw_frame` make them: apps (`dmenu` false) or
/// dmenu.
fn geometry(look: &Look, scale: Scale, rows: u32, dmenu: bool) -> (u32, u32, Layout) {
    let logical = if dmenu {
        Layout::dmenu(look.font_size_px)
    } else {
        Layout::new(look.font_size_px, look.radius, Scale::Integer(1))
    };
    let width = (30.0 * look.font_size_px) as u32;
    let height = logical.height(rows);
    let device = if dmenu {
        dmenu_layout(look, scale, height, rows)
    } else {
        device_layout(look, scale, height, rows)
    };
    (width, height, device)
}

/// The device layout rounds each measurement on its own, so unfitted it
/// stands a few pixels taller than the scaled logical height at a
/// fractional scale (11 px at the worst here); held to that height, the
/// rows shrink to fit and the leftover joins the top and bottom padding,
/// so the buffer, exactly the scaled surface, holds all of it.
#[test]
fn the_device_layout_fits_the_scaled_panel_at_every_scale() {
    let (mut worst, mut shrunk) = (0, 0);
    for font in [11.0f32, 13.0, 16.0, 20.0] {
        let look = Look {
            font_size_px: font,
            ..Look::default()
        };
        for scale in scales() {
            for dmenu in [false, true] {
                for rows in 0..=12 {
                    let (_, height, layout) = geometry(&look, scale, rows, dmenu);
                    let factor = scale.factor();
                    let em = (f64::from(font) * factor) as f32;
                    let natural = if dmenu {
                        Layout::dmenu_scaled(em, scale)
                    } else {
                        let radius = scale_len(look.radius, scale);
                        Layout::new(em, radius, scale)
                    };
                    let what = format!("font {font} scale {factor} rows {rows} dmenu {dmenu}");
                    let scaled = scale_len(height, scale);
                    worst = worst.max(natural.height(rows).saturating_sub(scaled));
                    assert_eq!(
                        layout.height(rows),
                        natural.height(rows).min(scaled),
                        "{what}: the fit lands on the scaled height"
                    );
                    // The rows only ever give pixels up, never below the text.
                    assert!(layout.row_h <= natural.row_h, "{what}");
                    assert!(layout.row_h >= em.ceil() as u32, "{what}");
                    shrunk = shrunk.max(natural.row_h - layout.row_h);
                    // Everything else is the layout it always was.
                    assert_eq!(
                        (layout.pad, layout.gap, layout.border),
                        (natural.pad, natural.gap, natural.border),
                        "{what}"
                    );
                    // The remainder joins the padding, top matching bottom.
                    assert!(
                        layout.top_extra().abs_diff(layout.bottom_extra()) <= 1,
                        "{what}: {layout:?}"
                    );
                    // At 1x nothing is fitted: the panel is its logical self.
                    if scale == Scale::Integer(1) {
                        assert_eq!(layout, natural, "{what}: 1x is unchanged");
                    }
                }
            }
        }
    }
    eprintln!(
        "unfitted, up to {worst} px over the scaled height; a row gives up a few px ({shrunk} px here; 3 px measured at font 8 with no rows)"
    );
    assert!(shrunk <= 3, "{shrunk}");
}

/// A click lands on the row the pixels show: sweeping the pointer down the
/// panel one logical pixel at a time, the row index never goes back, every
/// shown row is reached, none past them is, and the middle of each drawn
/// row (in device pixels) maps to that row.
#[test]
fn a_click_lands_on_the_row_the_pixels_show_at_every_scale() {
    for font in [11.0f32, 13.0, 16.0, 20.0] {
        let look = Look {
            font_size_px: font,
            ..Look::default()
        };
        for scale in scales() {
            for dmenu in [false, true] {
                for rows in 1..=10u32 {
                    let (_, height, layout) = geometry(&look, scale, rows, dmenu);
                    let factor = scale.factor();
                    let at = |y| row_at(&layout, factor, y, rows as usize);
                    let what = format!("font {font} scale {factor} rows {rows} dmenu {dmenu}");
                    let mut last = None;
                    let mut reached = vec![false; rows as usize];
                    for y in 0..height {
                        if let Some(row) = at(y) {
                            assert!(Some(row) >= last, "{what}: row went back at {y}");
                            last = Some(row);
                            reached[row] = true;
                        }
                    }
                    assert!(
                        reached.iter().all(|r| *r),
                        "{what}: a row no pointer reaches"
                    );
                    for row in 0..rows {
                        let mid = layout.list_y() + row * layout.row_h + layout.row_h / 2;
                        let y = (f64::from(mid) / factor) as u32;
                        assert_eq!(at(y), Some(row as usize), "{what}: row {row}'s middle");
                    }
                    assert_eq!(at(0), None, "{what}: the top border");
                }
            }
        }
    }
}

#[test]
fn a_pointer_below_the_shown_rows_hits_none() {
    let look = Look::default();
    let (_, _, layout) = geometry(&look, Scale::Fractional(180), 6, false);
    // Three rows shown of six: the fourth row's place is empty.
    let fourth = ((layout.list_y() + 3 * layout.row_h + 2) as f64 / 1.5) as u32;
    assert_eq!(row_at(&layout, 1.5, fourth, 3), None);
    assert_eq!(row_at(&layout, 1.5, fourth, 4), Some(3));
    // A zero-height row (a broken layout) hits nothing and does not divide.
    let flat = Layout { row_h: 0, ..layout };
    assert_eq!(row_at(&flat, 1.5, fourth, 4), None);
}

/// The panel's device size rounds the way the buffer and the compositor's
/// mapping do: [`scale_len`] is [`scaled_length`] (integer, half up), not
/// the old `f64` round, so the panel inside the buffer is the buffer.
#[test]
fn scale_len_matches_scaled_length_at_every_v120_and_length() {
    use scootui::density::scaled_length;
    // Zero stays zero; nonzero is at least one.
    assert_eq!(scale_len(0, Scale::Integer(1)), 0);
    assert_eq!(scale_len(0, Scale::Fractional(150)), 0);
    for factor in [1u32, 2, 3] {
        let scale = Scale::Integer(factor);
        for length in 1..1500u32 {
            assert_eq!(
                scale_len(length, scale),
                length.saturating_mul(factor).max(1),
                "integer {factor}x length {length}"
            );
        }
    }
    for v120 in 120..=1920u32 {
        let scale = Scale::Fractional(v120);
        for length in 1..1500u32 {
            assert_eq!(
                scale_len(length, scale),
                scaled_length(length, v120).unwrap_or(u32::MAX),
                "v120 {v120} length {length}"
            );
        }
    }
}

/// The old `f64` rounding disagrees with the integer rule: the sweep above
/// would fail against it. Pinned here so the change of rule shows.
#[test]
fn the_old_f64_scale_len_disagrees_with_scaled_length() {
    use scootui::density::scaled_length;
    fn old_scale_len(logical: u32, v120: u32) -> u32 {
        if logical == 0 {
            0
        } else {
            let factor = f64::from(v120) / 120.0;
            ((f64::from(logical) * factor)
                .round()
                .max(1.0)
                .min(f64::from(u32::MAX))) as u32
        }
    }
    // The 25 v120 values up to 300 the ticket names: each has a length
    // where the old rule is off by one.
    let known = [
        122, 123, 130, 131, 138, 145, 146, 153, 161, 183, 191, 206, 244, 245, 246, 247, 260, 261,
        262, 275, 276, 277, 290, 291, 292,
    ];
    for v120 in known {
        let mut witness = None;
        for length in 1..1500u32 {
            if old_scale_len(length, v120) != scaled_length(length, v120).unwrap() {
                witness = Some(length);
                break;
            }
        }
        assert!(
            witness.is_some(),
            "v120 {v120}: the old rule agrees everywhere, the sweep would not fail"
        );
    }
}
