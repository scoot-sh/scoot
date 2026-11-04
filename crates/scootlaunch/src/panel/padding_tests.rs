//! The panel's top and bottom padding at the scales an output runs.

use scootui::density::Scale;

use super::*;

/// (top, bottom) padding in device pixels, border to content: from the
/// panel's border to the search field, and from the last list row to the
/// border, for the buffer `paint_panel` fills (the logical height scaled).
fn paddings(look: &Look, scale: Scale, rows: u32) -> (u32, u32) {
    let em = look.font_size_px.clamp(1.0, 256.0);
    let logical = Layout::new(em, look.radius, Scale::Integer(1));
    let height = logical.height(rows);
    let layout = device_layout(look, scale, height, rows);
    let height_d = scale_len(height, scale);
    let width = 40 * layout.row_h.max(1);
    let top = layout.field(width).y - layout.border;
    let last = layout.row(width, rows - 1);
    let bottom = height_d - layout.border - (last.y + last.height);
    (top, bottom)
}

/// The scales the panel runs at: 1x, the fractional ones in between, 2x.
const SCALES: [Scale; 8] = [
    Scale::Integer(1),
    Scale::Fractional(120),
    Scale::Fractional(150),
    Scale::Fractional(180),
    Scale::Integer(2),
    Scale::Fractional(225),
    Scale::Fractional(270),
    Scale::Integer(3),
];

/// The bottom padding matches the top one to within the single pixel an
/// odd remainder cannot split, for every scale and list length. Before the
/// spare under a row joined the padding, 2.25x over ten rows stood 2 px
/// apart (18 above, 20 below).
#[test]
fn the_bottom_padding_matches_the_top_at_every_scale() {
    let look = Look::default();
    for scale in SCALES {
        for rows in 1..=12 {
            let (top, bottom) = paddings(&look, scale, rows);
            assert!(
                top.abs_diff(bottom) <= 1,
                "{scale:?} over {rows} rows: {top} above, {bottom} below"
            );
        }
    }
}

/// The same for other font sizes, where the em rounds differently.
#[test]
fn the_padding_holds_for_other_font_sizes() {
    for size in [8.0_f32, 11.0, 13.0, 15.0, 18.0, 24.0] {
        let look = Look {
            font_size_px: size,
            ..Look::default()
        };
        for scale in SCALES {
            for rows in [1, 4, 9] {
                let (top, bottom) = paddings(&look, scale, rows);
                assert!(
                    top.abs_diff(bottom) <= 1,
                    "{size}px {scale:?} over {rows} rows: {top} above, {bottom} below"
                );
            }
        }
    }
}
