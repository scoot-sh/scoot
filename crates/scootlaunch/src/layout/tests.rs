//! The panel geometry's invariants over fonts, scales and radii: the
//! highlight, the icon column, the field and the inner box never
//! overlap where they should not, and text never leaves its row.

use super::*;

const EMS: [f32; 4] = [11.0, 13.0, 16.0, 20.0];
const FACTORS: [f64; 3] = [1.0, 1.5, 2.0];
const RADII: [u32; 5] = [0, 4, 14, 28, 64];

fn each(mut check: impl FnMut(Layout, f64, u32, u32)) {
    for em in EMS {
        for factor in FACTORS {
            for radius in RADII {
                let em_d = em * factor as f32;
                let radius_d = (f64::from(radius) * factor).round() as u32;
                let layout = Layout::new(em_d, radius_d, factor);
                // The default 30-em panel, in device pixels.
                let width = (30.0 * em_d) as u32;
                check(layout, factor, radius_d, width);
            }
        }
    }
}

#[test]
fn the_field_and_every_row_share_one_inner_box() {
    each(|layout, _, _, width| {
        let inner = layout.inner_box(width, 16);
        let field = layout.field(width);
        assert_eq!((field.x, field.width), (inner.x, inner.width));
        for slot in 0..16 {
            let row = layout.row(width, slot);
            assert_eq!((row.x, row.width), (inner.x, inner.width), "{layout:?}");
            assert!(inner.contains(row), "{layout:?} slot {slot}");
            assert!(!row.overlaps(field), "{layout:?} slot {slot}");
        }
        assert!(inner.contains(field));
        // The panel's right padding equals its left.
        assert_eq!(width - inner.right(), inner.x);
    });
}

#[test]
fn rows_tile_without_overlap_and_the_panel_height_is_exact() {
    each(|layout, _, _, width| {
        for rows in [0u32, 1, 2, 16, 64] {
            if rows > 0 {
                let last = layout.row(width, rows - 1);
                assert_eq!(
                    last.bottom() + layout.inner(),
                    layout.height(rows),
                    "{layout:?} rows {rows}"
                );
            }
            for slot in 1..rows {
                let a = layout.row(width, slot - 1);
                let b = layout.row(width, slot);
                assert!(!a.overlaps(b));
                assert_eq!(a.bottom(), b.y);
            }
        }
        assert_eq!(
            layout.field(width).bottom() + layout.gap,
            layout.row(width, 0).y
        );
        assert_eq!(
            layout.inner_box(width, 3).bottom() + layout.inner(),
            layout.height(3)
        );
    });
}

#[test]
fn the_icon_sits_inside_the_highlight_and_clear_of_the_text() {
    each(|layout, _, _, width| {
        let row = layout.row(width, 0);
        let icon = layout.icon(row);
        assert!(row.contains(icon), "{layout:?}");
        // Inset from the highlight's left edge, never against it.
        assert_eq!(icon.x - row.x, layout.lead);
        assert!(layout.lead >= 1);
        assert!(layout.text_x(row) >= icon.right() + layout.icon_gap);
        assert!(layout.text_x(row) < layout.text_end(row), "{layout:?}");
        // Centered to within a pixel.
        let top = icon.y - row.y;
        let bottom = row.bottom() - icon.bottom();
        assert!(top.abs_diff(bottom) <= 1, "{layout:?}");
    });
}

#[test]
fn the_icon_is_sized_to_the_text() {
    each(|layout, _, _, _| {
        let em = layout.em;
        assert!(layout.side as f32 >= em.round().min(layout.row_h as f32 - 4.0));
        assert!(layout.side <= layout.row_h);
        assert!(layout.side <= MAX_RENDER_SIDE);
        // A touch more row than the old em + 8.
        assert!(layout.row_h as f32 >= em.ceil() + 8.0, "{layout:?}");
        assert!(layout.row_h > layout.side);
    });
}

#[test]
fn the_highlight_radius_matches_the_panel_and_never_exceeds_the_row() {
    each(|layout, _, radius_d, width| {
        assert!(layout.radius <= radius_d);
        assert!(layout.radius <= layout.row_h / 2);
        assert!(layout.radius <= layout.row(width, 0).width / 2);
        // Where the panel's radius fits in a row, the highlight takes it.
        if radius_d <= layout.row_h / 2 {
            assert_eq!(layout.radius, radius_d);
        }
        // The icon stays clear of the highlight's curved corners: its
        // top-left corner is inside the corner circle.
        let row = layout.row(width, 0);
        let icon = layout.icon(row);
        let r = layout.radius as f32;
        let dx = r - (icon.x - row.x) as f32;
        let dy = r - (icon.y - row.y) as f32;
        if dx > 0.0 && dy > 0.0 {
            assert!(dx.hypot(dy) <= r, "{layout:?}");
        }
    });
}

#[test]
fn the_field_and_rows_clear_the_panels_rounded_corners() {
    each(|layout, _, radius_d, width| {
        let inner = layout.inner();
        let r = radius_d as f32;
        if r > inner as f32 {
            // The inner box's top-left corner lies inside the panel's
            // inner contour (radius less the border).
            let d = (r - inner as f32).hypot(r - inner as f32);
            assert!(d <= r - layout.border as f32, "{layout:?} r {radius_d}");
        }
        let bottom = layout.height(16);
        assert!(layout.inner_box(width, 16).bottom() < bottom);
    });
}

#[test]
fn text_never_leaves_its_row() {
    each(|layout, _, _, width| {
        let row = layout.row(width, 0);
        let geo = layout.list_geometry(width);
        assert_eq!(geo.text_x(), layout.text_x(row), "{layout:?}");
        let clip = geo.clip();
        assert_eq!(clip.x, layout.text_x(row));
        assert_eq!(clip.end(), layout.text_end(row));
        assert!(clip.end() <= row.right() - layout.lead);
        assert_eq!(geo.y, row.y);
        assert_eq!(geo.row_height, layout.row_h);
        // The search text starts inside the field and clips inside it.
        let field = layout.field(width);
        let entry = layout.field_geometry(width);
        assert!(entry.text_x() >= field.x && entry.clip().end() <= field.right());
    });
}

#[test]
fn a_panel_too_narrow_for_its_padding_draws_nothing_and_does_not_panic() {
    for width in [0u32, 1, 10, 37] {
        let layout = Layout::new(13.0, 14, 1.0);
        let row = layout.row(width, 0);
        assert!(row.width == 0 || row.right() <= width);
        let geo = layout.list_geometry(width);
        assert_eq!(geo.clip().width, 0);
        assert_eq!(layout.field(width).width, row.width);
    }
}

#[test]
fn hostile_inputs_clamp() {
    let layout = Layout::new(f32::NAN, u32::MAX, f64::NAN);
    assert!(layout.row_h >= 1 && layout.side >= 1);
    let layout = Layout::new(f32::INFINITY, u32::MAX, f64::INFINITY);
    assert!(layout.row_h <= 4096);
    let layout = Layout::new(1.0, 0, 0.0);
    assert!(layout.side >= 1 && layout.side < layout.row_h);
    assert_eq!(layout.height(u32::MAX), u32::MAX);
}

#[test]
fn the_default_panel_at_13_is_roomier_than_before() {
    // Before: row = ceil(13) + 8 = 21, pad 8, icon 13 against the field.
    let layout = Layout::new(13.0, 14, 1.0);
    assert_eq!(layout.row_h, 23);
    assert_eq!(layout.side, 16);
    assert_eq!(layout.inner(), layout.border + layout.pad);
    assert!(layout.pad >= 10);
}

#[test]
fn the_dmenu_layout_keeps_the_old_numbers() {
    let layout = Layout::dmenu(13.0);
    assert_eq!((layout.border, layout.pad, layout.gap), (2, 8, 4));
    assert_eq!(layout.row_h, 21);
    assert_eq!(layout.height(11), 2 * 10 + 21 * 12 + 4);
}

#[test]
fn the_dmenu_layout_scales_its_fixed_spacing_with_the_output() {
    // 13 px text at 2x is 26 device pixels: the border, padding, gap and
    // row slack double with it.
    let layout = Layout::dmenu_scaled(26.0, 2.0);
    assert_eq!((layout.border, layout.pad, layout.gap), (4, 16, 8));
    assert_eq!(layout.row_h, 26 + 16);
    // At 1.5 the rounding is the apps panel's: round, never zero border.
    let layout = Layout::dmenu_scaled(19.5, 1.5);
    assert_eq!((layout.border, layout.pad, layout.gap), (3, 12, 6));
    assert_eq!(layout.row_h, 20 + 12);
    // Hostile values stay finite and nonzero.
    for (em, factor) in [(f32::NAN, 1.0), (1.0, f64::NAN), (0.0, 0.0), (1e9, 1e9)] {
        let layout = Layout::dmenu_scaled(em, factor);
        assert!(layout.border >= 1 && layout.row_h >= 1, "{em} {factor}");
        assert!(layout.height(64) < u32::MAX, "{em} {factor}");
    }
}
