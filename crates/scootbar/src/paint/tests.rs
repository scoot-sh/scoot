use super::fill;
use crate::color::Color;

const COLOR: Color = Color {
    r: 0x12,
    g: 0x34,
    b: 0x56,
};

#[test]
fn every_pixel_is_the_color_in_memory_order() {
    let mut pixels = vec![0u8; 4 * 7];
    fill(&mut pixels, COLOR);
    for pixel in pixels.chunks_exact(4) {
        assert_eq!(pixel, [0x56, 0x34, 0x12, 0xff]);
    }
}

#[test]
fn a_repaint_overwrites_what_was_there() {
    let mut pixels = vec![0xaa; 4 * 3];
    fill(&mut pixels, COLOR);
    fill(&mut pixels, Color { r: 1, g: 2, b: 3 });
    assert_eq!(pixels, [3, 2, 1, 0xff].repeat(3));
}

#[test]
fn empty_and_partial_buffers_are_safe() {
    let mut empty: [u8; 0] = [];
    fill(&mut empty, COLOR);
    let mut partial = [0u8; 6];
    fill(&mut partial, COLOR);
    assert_eq!(partial, [0x56, 0x34, 0x12, 0xff, 0, 0]);
}
