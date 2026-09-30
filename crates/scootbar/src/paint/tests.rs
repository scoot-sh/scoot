use super::{Canvas, Corners, Span};
use crate::color::Color;

const COLOR: Color = Color {
    r: 0x12,
    g: 0x34,
    b: 0x56,
};
const WHITE: Color = Color {
    r: 0xff,
    g: 0xff,
    b: 0xff,
};

fn canvas(pixels: &mut [u8], width: u32, height: u32) -> Canvas<'_> {
    Canvas::new(pixels, width, height).unwrap()
}

#[test]
fn a_span_fill_is_full_height_in_memory_order() {
    let mut pixels = vec![0u8; 4 * 5 * 2];
    let mut c = canvas(&mut pixels, 5, 2);
    c.fill_span(Span { x: 1, width: 2 }, COLOR);
    for y in 0..2 {
        assert_eq!(c.at(0, y), [0, 0, 0]);
        assert_eq!(c.at(1, y), [0x12, 0x34, 0x56]);
        assert_eq!(c.at(2, y), [0x12, 0x34, 0x56]);
        assert_eq!(c.at(3, y), [0, 0, 0]);
    }
    // Bytes are blue, green, red, then the unused byte set opaque.
    assert_eq!(&pixels[4..8], &[0x56, 0x34, 0x12, 0xff]);
}

#[test]
fn spans_past_the_edge_are_clipped() {
    let mut pixels = vec![0u8; 4 * 4];
    let mut c = canvas(&mut pixels, 4, 1);
    c.fill_span(
        Span {
            x: 2,
            width: u32::MAX,
        },
        COLOR,
    );
    c.fill_span(Span { x: 9, width: 3 }, WHITE);
    c.fill_span(Span { x: 0, width: 0 }, WHITE);
    assert_eq!(c.at(1, 0), [0, 0, 0]);
    assert_eq!(c.at(3, 0), [0x12, 0x34, 0x56]);
}

#[test]
fn blending_mixes_by_coverage_and_rounds() {
    let mut pixels = vec![0u8; 4 * 3];
    let mut c = canvas(&mut pixels, 3, 1);
    let all = Span { x: 0, width: 3 };
    c.blend(0, 0, 255, WHITE, all);
    c.blend(1, 0, 128, WHITE, all);
    c.blend(2, 0, 0, WHITE, all);
    assert_eq!(c.at(0, 0), [255, 255, 255]);
    assert_eq!(c.at(1, 0), [128, 128, 128]);
    assert_eq!(c.at(2, 0), [0, 0, 0]);
}

#[test]
fn blending_outside_the_clip_or_the_canvas_does_nothing() {
    let mut pixels = vec![0u8; 4 * 4];
    let mut c = canvas(&mut pixels, 2, 2);
    let clip = Span { x: 1, width: 1 };
    for (x, y) in [
        (0, 0),
        (-1, 0),
        (0, -1),
        (2, 0),
        (1, 2),
        (i64::MAX, 0),
        (1, i64::MIN),
    ] {
        c.blend(x, y, 255, WHITE, clip);
    }
    assert!(pixels.iter().all(|&b| b == 0));
    let mut c = canvas(&mut pixels, 2, 2);
    c.blend(1, 1, 255, WHITE, clip);
    assert_eq!(c.at(1, 1), [255, 255, 255]);
}

#[test]
fn a_short_buffer_is_no_canvas() {
    let mut pixels = vec![0u8; 4 * 3];
    assert!(Canvas::new(&mut pixels, 2, 2).is_none());
    assert!(Canvas::new(&mut pixels, u32::MAX, u32::MAX).is_none());
    assert!(Canvas::new(&mut pixels, 0, 0).is_some());
}

/// A canvas filled through `fill_shaped` at `radius`, opaque: its alpha
/// plane as rows of bytes.
fn shaped(width: u32, height: u32, radius: u32, alpha: u8, span: Span) -> Vec<Vec<u8>> {
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut c = canvas(&mut pixels, width, height);
    c.fill_shaped(span, COLOR, alpha, &Corners::new(radius));
    (0..height)
        .map(|y| (0..width).map(|x| c.alpha_at(x, y)).collect())
        .collect()
}

#[test]
fn the_corner_table_is_a_quarter_circle_of_analytic_coverage() {
    assert_eq!(Corners::new(0), Corners::NONE);
    assert_eq!(Corners::NONE.radius(), 0);
    let corners = Corners::new(8);
    // The far corner pixel is outside, the one at the circle's center is
    // inside, and coverage never drops moving inward along a row or down a
    // column.
    assert_eq!(corners.at(0, 0), 0);
    assert_eq!(corners.at(7, 7), 255);
    for y in 0..8 {
        for x in 0..7 {
            assert!(corners.at(x, y) <= corners.at(x + 1, y), "row {y} at {x}");
            assert!(
                corners.at(y, x) <= corners.at(y, x + 1),
                "column {y} at {x}"
            );
        }
    }
    // Symmetric about the diagonal.
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(corners.at(x, y), corners.at(y, x));
        }
    }
    // Some pixels are partly covered: an antialiased edge, not a stair.
    assert!((0..8).any(|x| (1..255).contains(&corners.at(x, 1))));
    // Past the table is inside.
    assert_eq!(corners.at(8, 0), 255);
    assert_eq!(corners.at(0, 99), 255);
}

#[test]
fn a_shaped_fill_cuts_all_four_corners_alike() {
    let whole = Span { x: 0, width: 20 };
    let plane = shaped(20, 12, 5, 255, whole);
    for (y, row) in plane.iter().enumerate() {
        let mirrored: Vec<u8> = row.iter().rev().copied().collect();
        assert_eq!(row, &mirrored, "row {y} is left-right symmetric");
        assert_eq!(row, &plane[11 - y], "row {y} matches its mirror");
    }
    assert_eq!(plane[0][0], 0, "the corner pixel is cut");
    assert_eq!(plane[0][10], 255, "the middle of the top edge is not");
    assert_eq!(plane[6][0], 255, "the middle of the left edge is not");
    // A full-radius bar (height 12, radius 6) is a pill, with no panic at
    // the limit.
    let pill = shaped(20, 12, 6, 255, whole);
    assert_eq!(pill[0][0], 0);
    assert!(pill[5][0] > 240, "the pill's side is nearly full");
}

#[test]
fn a_shaped_fill_of_a_span_only_cuts_where_it_meets_a_corner() {
    // A span inside: no cut. A span at the left: cut at its left end only.
    let middle = shaped(20, 12, 5, 255, Span { x: 6, width: 8 });
    assert!(middle[0][6..14].iter().all(|&a| a == 255));
    assert!(middle[0][..6].iter().all(|&a| a == 0), "untouched stays 0");
    let left = shaped(20, 12, 5, 255, Span { x: 0, width: 8 });
    assert_eq!(left[0][0], 0);
    assert_eq!(left[0][7], 255);
    // Repainting a span over an already shaped canvas leaves the cut
    // corners cut: what a module's repaint at the bar's edge relies on.
    let mut pixels = vec![0u8; 20 * 12 * 4];
    let mut c = canvas(&mut pixels, 20, 12);
    let corners = Corners::new(5);
    c.fill_shaped(Span { x: 0, width: 20 }, COLOR, 255, &corners);
    let before: Vec<u8> = (0..12).map(|y| c.alpha_at(0, y)).collect();
    c.fill_shaped(Span { x: 0, width: 8 }, COLOR, 255, &corners);
    let after: Vec<u8> = (0..12).map(|y| c.alpha_at(0, y)).collect();
    assert_eq!(before, after);
}

#[test]
fn a_shaped_fill_is_premultiplied_and_scales_the_corners_by_it() {
    let mut pixels = vec![0u8; 20 * 12 * 4];
    let mut c = canvas(&mut pixels, 20, 12);
    c.fill_shaped(Span { x: 0, width: 20 }, WHITE, 128, &Corners::new(5));
    let (mid, edge) = (10, 0);
    assert_eq!(c.alpha_at(mid, 6), 128);
    assert_eq!(c.at(mid, 6), [128, 128, 128], "premultiplied, not 255");
    assert_eq!(c.alpha_at(edge, 0), 0);
    assert_eq!(c.at(edge, 0), [0, 0, 0]);
    // A partly covered pixel is dimmer than the bar's own alpha.
    let rim = (0..5)
        .map(|x| c.alpha_at(x, 1))
        .find(|a| (1..128).contains(a));
    assert!(rim.is_some(), "an antialiased rim pixel below alpha 128");
}

#[test]
fn corners_that_do_not_fit_leave_a_plain_fill_and_odd_spans_do_not_panic() {
    // Twice the radius is past the height: square, whatever the table.
    let plane = shaped(20, 8, 5, 255, Span { x: 0, width: 20 });
    assert!(plane.iter().flatten().all(|&a| a == 255));
    // Past the edge, empty, and a zero-sized canvas.
    let _ = shaped(20, 12, 5, 255, Span { x: 30, width: 4 });
    let _ = shaped(20, 12, 5, 255, Span { x: 0, width: 0 });
    let _ = shaped(
        20,
        12,
        5,
        255,
        Span {
            x: u32::MAX,
            width: u32::MAX,
        },
    );
    let _ = shaped(0, 0, 5, 255, Span { x: 0, width: 4 });
    let _ = shaped(1, 1, 500, 255, Span { x: 0, width: 4 });
}

#[test]
fn a_blend_over_a_translucent_pixel_is_a_premultiplied_over() {
    let mut pixels = vec![0u8; 4];
    let mut c = canvas(&mut pixels, 1, 1);
    let whole = Span { x: 0, width: 1 };
    // Half-alpha black, then white at 50% coverage: alpha 128 -> 192.
    c.fill_shaped(whole, Color { r: 0, g: 0, b: 0 }, 128, &Corners::NONE);
    c.blend(0, 0, 128, WHITE, whole);
    assert_eq!(c.alpha_at(0, 0), 192);
    assert_eq!(c.at(0, 0), [128, 128, 128]);
    // Over an opaque pixel it stays opaque, and full coverage is opaque
    // whatever was under it.
    c.fill_span(whole, COLOR);
    c.blend(0, 0, 77, WHITE, whole);
    assert_eq!(c.alpha_at(0, 0), 255);
    c.fill_shaped(whole, COLOR, 0, &Corners::NONE);
    c.blend(0, 0, 255, WHITE, whole);
    assert_eq!(c.alpha_at(0, 0), 255);
}
