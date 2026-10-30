use super::*;

const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
};
const RED: Color = Color { r: 200, g: 0, b: 0 };

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
fn mixing_moves_a_color_toward_another_and_rounds() {
    assert_eq!(RED.mix(WHITE, 0), RED);
    assert_eq!(RED.mix(WHITE, 255), WHITE);
    assert_eq!(
        Color {
            r: 0,
            g: 100,
            b: 255
        }
        .mix(
            Color {
                r: 255,
                g: 200,
                b: 0
            },
            128
        ),
        Color {
            r: 128,
            g: 150,
            b: 127
        }
    );
}

#[test]
fn a_solid_fill_is_the_gradient_with_equal_stops() {
    let mut pixels = vec![0u8; 20 * 12 * 4];
    let mut canvas = Canvas::new(&mut pixels, 20, 12).unwrap();
    canvas.fill_rounded_gradient(rect(2, 1, 16, 10), 4, RED, RED, 255);
    // Straight run: the color, opaque; the cut corner: untouched.
    assert_eq!(pixel(&pixels, 20, 10, 5), [0, 0, 200, 255]);
    assert_eq!(pixel(&pixels, 20, 2, 1), [0, 0, 0, 0]);
    // The corner's anti-aliased edge blends in between.
    let edge = pixel(&pixels, 20, 3, 1);
    assert!(edge[3] > 0 && edge[3] < 255, "{edge:?}");
}

#[test]
fn a_gradient_runs_top_to_bottom_through_the_row_centers() {
    let top = Color { r: 0, g: 0, b: 0 };
    let bottom = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    let mut pixels = vec![0u8; 8 * 10 * 4];
    let mut canvas = Canvas::new(&mut pixels, 8, 10).unwrap();
    canvas.fill_rounded_gradient(rect(0, 0, 8, 10), 0, top, bottom, 255);
    let column: Vec<u8> = (0..10).map(|y| pixel(&pixels, 8, 4, y)[0]).collect();
    assert!(
        column.windows(2).all(|pair| pair[0] < pair[1]),
        "{column:?}"
    );
    assert!(column[0] < 20 && column[9] > 235, "{column:?}");
    // A one-row fill is the midpoint, not a stop.
    let mut single = vec![0u8; 4 * 4];
    let mut canvas = Canvas::new(&mut single, 4, 1).unwrap();
    canvas.fill_rounded_gradient(rect(0, 0, 4, 1), 0, top, bottom, 255);
    assert_eq!(pixel(&single, 4, 1, 0)[0], 127);
}

#[test]
fn a_translucent_fill_composites_over_the_destination() {
    let mut pixels = vec![0u8; 6 * 6 * 4];
    let mut canvas = Canvas::new(&mut pixels, 6, 6).unwrap();
    canvas.fill_rounded_gradient(rect(0, 0, 6, 6), 0, WHITE, WHITE, 255);
    canvas.fill_rounded_gradient(
        rect(0, 0, 6, 6),
        0,
        Color { r: 0, g: 0, b: 0 },
        Color { r: 0, g: 0, b: 0 },
        128,
    );
    assert_eq!(pixel(&pixels, 6, 3, 3), [127, 127, 127, 255]);
}

#[test]
fn oversized_empty_and_offscreen_rects_clip_without_panicking() {
    let mut pixels = vec![0u8; 5 * 5 * 4];
    let mut canvas = Canvas::new(&mut pixels, 5, 5).unwrap();
    for r in [
        rect(0, 0, 0, 5),
        rect(0, 0, 5, 0),
        rect(3, 3, 100, 100),
        rect(9, 9, 4, 4),
        rect(0, 0, 10_000_000, 3),
    ] {
        canvas.fill_rounded_gradient(r, 64, RED, WHITE, 255);
    }
    assert_eq!(pixel(&pixels, 5, 2, 1)[3], 255);
    canvas = Canvas::new(&mut pixels, 5, 5).unwrap();
    canvas.fill_rounded_gradient(rect(u32::MAX - 2, u32::MAX - 2, 9, 9), 4, RED, WHITE, 255);
    canvas.fill_rounded_gradient(rect(0, u32::MAX - 1, u32::MAX, 9), 4, RED, WHITE, 255);
}

/// The fill the runs replaced, pixel by pixel through `blend`: the
/// reference every rectangle must match byte for byte.
fn reference_fill(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    radius: u32,
    top: Color,
    bottom: Color,
    alpha: u8,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let radius = effective_radius(radius, rect.width, rect.height);
    let clip = Span {
        x: 0,
        width: canvas.width(),
    };
    for y in rect.y..rect.y.saturating_add(rect.height).min(canvas.height()) {
        let color = row_color(top, bottom, y - rect.y, rect.height);
        for x in rect.x..rect.x.saturating_add(rect.width).min(canvas.width()) {
            let coverage = rounded_coverage(x, y, rect, radius);
            if coverage == 0 {
                continue;
            }
            let blended = (u32::from(alpha) * u32::from(coverage) + 127) / 255;
            canvas.blend(
                i64::from(x),
                i64::from(y),
                blended.min(255) as u8,
                color,
                clip,
            );
        }
    }
}

#[test]
fn runs_match_the_pixel_by_pixel_fill_byte_for_byte() {
    // A deterministic spread of rectangles, radii (including ones the
    // rectangle cannot hold), alphas, gradients, partly off the canvas,
    // over a non-uniform, partly translucent destination.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |bound: u32| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % u64::from(bound)) as u32
    };
    for case in 0..400 {
        let (cw, ch) = (1 + next(40), 1 + next(30));
        let mut seed = vec![0u8; cw as usize * ch as usize * 4];
        for byte in &mut seed {
            *byte = next(256) as u8;
        }
        // A valid premultiplied pixel: no channel above alpha.
        for pixel in seed.as_chunks_mut::<4>().0 {
            let a = pixel[3];
            pixel[..3].iter_mut().for_each(|c| *c = (*c).min(a));
        }
        let r = rect(next(45), next(35), next(50), next(40));
        let radius = next(30);
        let top = Color {
            r: next(256) as u8,
            g: next(256) as u8,
            b: next(256) as u8,
        };
        let bottom = if next(2) == 0 {
            top
        } else {
            Color {
                r: next(256) as u8,
                g: next(256) as u8,
                b: next(256) as u8,
            }
        };
        let alpha = [0, 1, 128, 254, 255][next(5) as usize];
        let mut want = seed.clone();
        reference_fill(
            &mut Canvas::new(&mut want, cw, ch).unwrap(),
            r,
            radius,
            top,
            bottom,
            alpha,
        );
        let mut got = seed;
        Canvas::new(&mut got, cw, ch)
            .unwrap()
            .fill_rounded_gradient(r, radius, top, bottom, alpha);
        assert_eq!(
            got, want,
            "case {case}: {r:?} radius {radius} alpha {alpha}"
        );
    }
}

#[test]
fn an_edge_stripe_covers_only_its_columns_and_follows_the_corners() {
    let mut pixels = vec![0u8; 24 * 16 * 4];
    let mut canvas = Canvas::new(&mut pixels, 24, 16).unwrap();
    canvas.fill_rounded_edge(rect(2, 1, 20, 14), 6, 3, RED);
    // The straight rows: three opaque columns and nothing past them.
    for x in 2..5 {
        assert_eq!(pixel(&pixels, 24, x, 8), [0, 0, 200, 255], "x {x}");
    }
    assert_eq!(pixel(&pixels, 24, 5, 8), [0, 0, 0, 0]);
    assert_eq!(pixel(&pixels, 24, 1, 8), [0, 0, 0, 0]);
    // The cut corner stays clear and its edge is anti-aliased: the stripe
    // never leaves the rounded shape.
    assert_eq!(pixel(&pixels, 24, 2, 1), [0, 0, 0, 0]);
    let edge = pixel(&pixels, 24, 3, 1);
    assert!(edge[3] < 255, "{edge:?}");
    // Every stripe pixel lies inside the shape the fill itself would draw.
    let mut fill = vec![0u8; 24 * 16 * 4];
    let mut canvas = Canvas::new(&mut fill, 24, 16).unwrap();
    canvas.fill_rounded_gradient(rect(2, 1, 20, 14), 6, RED, RED, 255);
    for (stripe, whole) in pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(fill.as_chunks::<4>().0)
    {
        assert!(stripe[3] <= whole[3], "{stripe:?} outside {whole:?}");
    }
}

#[test]
fn an_edge_stripe_with_square_corners_or_too_wide_is_a_plain_fill() {
    let mut pixels = vec![0u8; 10 * 6 * 4];
    let mut canvas = Canvas::new(&mut pixels, 10, 6).unwrap();
    canvas.fill_rounded_edge(rect(1, 1, 4, 4), 0, 99, RED);
    for y in 1..5 {
        for x in 1..5 {
            assert_eq!(pixel(&pixels, 10, x, y), [0, 0, 200, 255], "{x},{y}");
        }
    }
    assert_eq!(pixel(&pixels, 10, 5, 2), [0, 0, 0, 0]);
}

#[test]
fn an_edge_stripe_is_clipped_to_the_canvas_and_ignores_empty_input() {
    let mut pixels = vec![0u8; 8 * 8 * 4];
    let mut canvas = Canvas::new(&mut pixels, 8, 8).unwrap();
    canvas.fill_rounded_edge(rect(6, 6, 20, 20), 4, 3, RED);
    canvas.fill_rounded_edge(rect(0, 0, 0, 5), 2, 3, RED);
    canvas.fill_rounded_edge(rect(0, 0, 5, 5), 2, 0, RED);
    canvas.fill_rounded_edge(rect(u32::MAX - 1, u32::MAX - 1, 9, 9), 4, 3, RED);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] != 255 || p[2] == 200)
    );
    // Nothing from the empty calls landed at the origin.
    assert_eq!(pixel(&pixels, 8, 0, 0), [0, 0, 0, 0]);
}

#[test]
fn large_corners_match_the_pixel_by_pixel_fill_byte_for_byte() {
    // The pill's radii: corner squares far wider than the anti-aliased
    // edge, in rectangles that hold them, are cut back, or sit partly off
    // the canvas.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |bound: u32| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % u64::from(bound)) as u32
    };
    for case in 0..120 {
        let (cw, ch) = (60 + next(180), 40 + next(140));
        let mut seed = vec![0u8; cw as usize * ch as usize * 4];
        for byte in &mut seed {
            *byte = next(256) as u8;
        }
        for pixel in seed.as_chunks_mut::<4>().0 {
            let a = pixel[3];
            pixel[..3].iter_mut().for_each(|c| *c = (*c).min(a));
        }
        let r = rect(next(60), next(40), 20 + next(220), 20 + next(160));
        let radius = [8, 24, 36, 64, 400][next(5) as usize];
        let top = Color {
            r: next(256) as u8,
            g: next(256) as u8,
            b: next(256) as u8,
        };
        let bottom = if next(2) == 0 { top } else { RED };
        let alpha = [1, 90, 200, 255][next(4) as usize];
        let mut want = seed.clone();
        reference_fill(
            &mut Canvas::new(&mut want, cw, ch).unwrap(),
            r,
            radius,
            top,
            bottom,
            alpha,
        );
        let mut got = seed;
        Canvas::new(&mut got, cw, ch)
            .unwrap()
            .fill_rounded_gradient(r, radius, top, bottom, alpha);
        assert_eq!(
            got, want,
            "case {case}: {r:?} radius {radius} alpha {alpha}"
        );
    }
}

#[test]
fn the_corner_search_finds_each_boundary() {
    let from = |lo: u32, hi: u32, at: u32| first_where(lo, hi, |x| x >= at);
    assert_eq!(from(0, 10, 0), 0);
    assert_eq!(from(0, 10, 7), 7);
    assert_eq!(from(0, 10, 10), 10);
    assert_eq!(from(0, 10, 99), 10, "no x holds: the end");
    assert_eq!(from(5, 5, 0), 5, "an empty range");
    assert_eq!(first_where(9, 3, |_| true), 9, "a reversed range is empty");
}
