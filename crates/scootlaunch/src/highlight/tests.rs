//! The rounded fill: square when the radius is zero, rounded corners
//! cut, a radius larger than the rectangle clamped, and a rectangle off
//! the canvas clipped without a panic.

use super::*;

const W: u32 = 40;
const H: u32 = 30;

fn paint(rect: Rect, radius: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, W, H).expect("canvas");
    let color = Color {
        r: 200,
        g: 100,
        b: 50,
    };
    fill_round_rect(&mut canvas, rect, radius, color);
    pixels
}

fn alpha(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * W + x) * 4 + 3) as usize]
}

#[test]
fn zero_radius_fills_every_pixel_of_the_rect_and_none_outside() {
    let rect = Rect {
        x: 5,
        y: 4,
        width: 20,
        height: 10,
    };
    let pixels = paint(rect, 0);
    for y in 0..H {
        for x in 0..W {
            let inside = (5..25).contains(&x) && (4..14).contains(&y);
            assert_eq!(alpha(&pixels, x, y) == 255, inside, "({x},{y})");
            assert!(inside || alpha(&pixels, x, y) == 0);
        }
    }
}

#[test]
fn a_radius_cuts_the_corners_and_keeps_the_middle() {
    let rect = Rect {
        x: 2,
        y: 2,
        width: 30,
        height: 20,
    };
    let pixels = paint(rect, 8);
    assert_eq!(alpha(&pixels, 2, 2), 0, "the corner pixel is cut");
    assert_eq!(alpha(&pixels, 31, 21), 0);
    assert_eq!(alpha(&pixels, 2, 12), 255, "the straight edge is whole");
    assert_eq!(alpha(&pixels, 16, 2), 255);
    assert_eq!(alpha(&pixels, 16, 12), 255);
    // Mirror symmetry left to right.
    for y in 2..22 {
        for lx in 0..8 {
            assert_eq!(alpha(&pixels, 2 + lx, y), alpha(&pixels, 31 - lx, y));
        }
    }
}

#[test]
fn a_radius_past_half_the_rect_is_a_pill_and_does_not_panic() {
    let rect = Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 8,
    };
    let pixels = paint(rect, 500);
    assert_eq!(alpha(&pixels, 10, 4), 255);
    assert_eq!(alpha(&pixels, 0, 0), 0);
}

#[test]
fn empty_and_off_canvas_rects_draw_nothing_and_do_not_panic() {
    for rect in [
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 5,
        },
        Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 0,
        },
        Rect {
            x: W + 10,
            y: 0,
            width: 10,
            height: 10,
        },
        Rect {
            x: 0,
            y: H + 10,
            width: 10,
            height: 10,
        },
        Rect {
            x: u32::MAX - 3,
            y: u32::MAX - 3,
            width: 10,
            height: 10,
        },
    ] {
        let pixels = paint(rect, 4);
        assert!(pixels.iter().all(|&byte| byte == 0), "{rect:?}");
    }
    // Partly off the canvas: the visible part draws.
    let pixels = paint(
        Rect {
            x: W - 5,
            y: H - 5,
            width: 20,
            height: 20,
        },
        0,
    );
    assert_eq!(alpha(&pixels, W - 1, H - 1), 255);
}
