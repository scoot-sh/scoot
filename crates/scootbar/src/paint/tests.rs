use super::{Canvas, Span};
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
