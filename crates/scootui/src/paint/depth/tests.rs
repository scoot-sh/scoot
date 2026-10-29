use super::*;

const BLACK: Color = Color { r: 0, g: 0, b: 0 };
const SOFT: ShadowSpec = ShadowSpec {
    offset_x: 0,
    offset_y: 4,
    blur: 12,
    alpha: 90,
};

fn card(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn shadow(spec: ShadowSpec, radius: u32, sizes: &[(u32, u32)]) -> Shadow {
    let mut shadow = Shadow::new(spec);
    shadow.prepare(radius, sizes.iter().copied());
    shadow
}

/// The shadow of `rect` on a transparent `width × height` canvas: the
/// alpha channel of every pixel, row-major.
fn alphas(shadow: &Shadow, width: u32, height: u32, rect: Rect, radius: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
    shadow.paint(&mut canvas, rect, radius, BLACK);
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[3])
        .collect()
}

/// The signed distance from `(x, y)`'s center to the rounded rectangle
/// `shape` (negative inside): the textbook SDF, the reference the tables
/// are checked against.
fn reference_distance(x: i64, y: i64, shape: (i64, i64, i64, i64), radius: f64) -> f64 {
    let (sx, sy, w, h) = shape;
    let (cx, cy) = (sx as f64 + w as f64 / 2.0, sy as f64 + h as f64 / 2.0);
    let qx = (x as f64 + 0.5 - cx).abs() - (w as f64 / 2.0 - radius);
    let qy = (y as f64 + 0.5 - cy).abs() - (h as f64 / 2.0 - radius);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - radius
}

#[test]
fn the_pad_is_the_reach_past_the_shape_per_side() {
    let pad = Shadow::new(SOFT).pad();
    assert_eq!(
        pad,
        Pad {
            top: 8,
            right: 12,
            bottom: 16,
            left: 12
        }
    );
    let glow = Shadow::new(ShadowSpec {
        offset_x: 0,
        offset_y: 0,
        blur: 16,
        alpha: 120,
    });
    assert_eq!(
        glow.pad(),
        Pad {
            top: 16,
            right: 16,
            bottom: 16,
            left: 16
        }
    );
    // An offset larger than the blur pushes the shadow off one side: that
    // side needs no room.
    let pushed = Shadow::new(ShadowSpec {
        offset_x: 20,
        offset_y: -20,
        blur: 8,
        alpha: 50,
    });
    assert_eq!(
        pushed.pad(),
        Pad {
            top: 28,
            right: 28,
            bottom: 0,
            left: 0
        }
    );
    assert!(
        Shadow::new(ShadowSpec {
            offset_x: 0,
            offset_y: 0,
            blur: 0,
            alpha: 9
        })
        .pad()
        .is_zero()
    );
}

#[test]
fn blur_and_offsets_are_clamped_not_trusted() {
    let huge = Shadow::new(ShadowSpec {
        offset_x: i32::MAX,
        offset_y: i32::MIN,
        blur: u32::MAX,
        alpha: 255,
    });
    let reach = MAX_BLUR + MAX_OFFSET as u32;
    assert_eq!(huge.pad().right, reach);
    assert_eq!(huge.pad().top, reach);
    assert_eq!(huge.edge.len(), 2 * MAX_BLUR as usize + 1);
}

#[test]
fn the_tables_equal_the_sdf_they_approximate_pixel_for_pixel() {
    // A card with 10 px corners, its shadow offset down: every pixel's
    // alpha is the falloff of the textbook distance to the offset shape.
    let radius = 10;
    let rect = card(30, 24, 60, 40);
    let shadow = shadow(SOFT, radius, &[(60, 40)]);
    let (width, height) = (120, 100);
    let got = alphas(&shadow, width, height, rect, radius);
    let shape = (30, 28, 60, 40);
    let mut painted = 0;
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let in_hole = {
                let (from, to) = covered_run(rect, radius, y);
                x >= from && x < to
            };
            let value = got[(y * i64::from(width) + x) as usize];
            if in_hole {
                assert_eq!(value, 0, "the covered body at ({x}, {y}) stays clear");
                continue;
            }
            let want = strength(&SOFT, reference_distance(x, y, shape, f64::from(radius)));
            assert_eq!(value, want, "alpha at ({x}, {y})");
            painted += u32::from(value != 0);
        }
    }
    assert!(painted > 1_000, "the shadow painted {painted} pixels");
}

#[test]
fn a_small_card_uses_the_radius_it_can_hold() {
    // A 16 px tall card asked for radius 40 draws radius 8: the tables are
    // the ones for 8, and the shadow agrees with the SDF of that shape.
    let rect = card(20, 20, 50, 16);
    let shadow = shadow(SOFT, 40, &[(50, 16)]);
    assert_eq!(shadow.corners.len(), 1);
    assert_eq!(shadow.corners[0].radius, 8);
    let got = alphas(&shadow, 100, 70, rect, 40);
    for y in 0..70i64 {
        for x in 0..100i64 {
            let (from, to) = covered_run(rect, 8, y);
            if x >= from && x < to {
                continue;
            }
            let want = strength(&SOFT, reference_distance(x, y, (20, 24, 50, 16), 8.0));
            assert_eq!(got[(y * 100 + x) as usize], want, "({x}, {y})");
        }
    }
}

#[test]
fn the_edge_is_half_strength_and_the_falloff_is_monotonic() {
    let spec = ShadowSpec {
        offset_x: 0,
        offset_y: 0,
        blur: 12,
        alpha: 200,
    };
    let shadow = shadow(spec, 6, &[(80, 50)]);
    let rect = card(30, 30, 80, 50);
    let got = alphas(&shadow, 140, 110, rect, 6);
    // Down the middle column, from the card's bottom edge outward.
    let column: Vec<u8> = (80..110).map(|y| got[y * 140 + 70]).collect();
    // The first pixel outside is half a pixel past the edge: just under half.
    assert!(
        (90..=100).contains(&column[0]),
        "edge strength {}",
        column[0]
    );
    assert!(
        column.windows(2).all(|pair| pair[0] >= pair[1]),
        "{column:?}"
    );
    // Past the blur radius nothing is drawn.
    assert!(column[12..].iter().all(|&a| a == 0), "{column:?}");
    assert!(column[8] > 0, "{column:?}");
}

#[test]
fn an_offset_moves_the_shadow_and_a_zero_offset_is_symmetric() {
    let rect = card(40, 40, 60, 40);
    let down = alphas(&shadow(SOFT, 8, &[(60, 40)]), 140, 120, rect, 8);
    let above = down[(35 * 140 + 70) as usize];
    let below = down[(84 * 140 + 70) as usize];
    assert!(below > above, "below {below} above {above}");

    let glow = ShadowSpec {
        offset_x: 0,
        offset_y: 0,
        blur: 10,
        alpha: 150,
    };
    let flat = alphas(&shadow(glow, 8, &[(60, 40)]), 140, 120, rect, 8);
    for y in 0..120usize {
        for x in 0..140usize {
            // The card is symmetric about its center (70, 60).
            let (mx, my) = (139 - x, 119 - y);
            assert_eq!(
                flat[y * 140 + x],
                flat[y * 140 + mx],
                "mirror x at ({x}, {y})"
            );
            assert_eq!(
                flat[y * 140 + x],
                flat[my * 140 + x],
                "mirror y at ({x}, {y})"
            );
        }
    }
}

#[test]
fn the_body_of_a_card_is_never_painted() {
    // Opaque or not, the pixels the card covers outright stay as they were.
    let rect = card(20, 20, 50, 30);
    let shadow = shadow(SOFT, 12, &[(50, 30)]);
    let got = alphas(&shadow, 100, 80, rect, 12);
    for y in 20..50usize {
        for x in 20..70usize {
            let (from, to) = covered_run(rect, 12, y as i64);
            if (x as i64) >= from && (x as i64) < to {
                assert_eq!(got[y * 100 + x], 0);
            }
        }
    }
    // The covered run narrows through the corner rows and is whole after.
    assert_eq!(covered_run(rect, 12, 19), (0, 0));
    assert_eq!(covered_run(rect, 12, 50), (0, 0));
    assert_eq!(covered_run(rect, 12, 35), (20, 70));
    let (top_from, top_to) = covered_run(rect, 12, 20);
    assert_eq!(
        (top_from, top_to),
        (32, 58),
        "the top row is the straight run"
    );
    let (mid_from, _) = covered_run(rect, 12, 26);
    assert!(mid_from > 20 && mid_from < 32);
}

#[test]
fn a_hard_shadow_has_an_antialiased_edge_and_no_reach() {
    let spec = ShadowSpec {
        offset_x: 6,
        offset_y: 6,
        blur: 0,
        alpha: 255,
    };
    let shadow = shadow(spec, 0, &[(20, 20)]);
    assert_eq!(
        shadow.pad(),
        Pad {
            top: 0,
            right: 6,
            bottom: 6,
            left: 0
        }
    );
    let got = alphas(&shadow, 50, 50, card(10, 10, 20, 20), 0);
    assert_eq!(got[(34 * 50 + 34) as usize], 255, "inside the offset shape");
    assert_eq!(got[(36 * 50 + 36) as usize], 0, "past it");
    assert_eq!(
        got[(34 * 50 + 12) as usize],
        0,
        "left of the shape, nothing"
    );
}

#[test]
fn painting_clips_to_the_canvas_and_survives_extremes() {
    let shadow = shadow(SOFT, 8, &[(60, 40)]);
    for canvas_size in [(0, 0), (1, 1), (7, 3), (200, 1)] {
        let mut pixels = vec![0u8; canvas_size.0 as usize * canvas_size.1 as usize * 4];
        let mut canvas = Canvas::new(&mut pixels, canvas_size.0, canvas_size.1).unwrap();
        for rect in [
            card(0, 0, 60, 40),
            card(u32::MAX - 10, u32::MAX - 10, 60, 40),
            card(0, 0, 0, 0),
            card(5, 5, u32::MAX, u32::MAX),
            card(u32::MAX, 0, u32::MAX, 5),
        ] {
            shadow.paint(&mut canvas, rect, 8, BLACK);
            shadow.paint(&mut canvas, rect, u32::MAX, BLACK);
        }
    }
}

#[test]
fn an_unprepared_radius_draws_square_corners_instead_of_panicking() {
    let unprepared = Shadow::new(SOFT);
    let got = alphas(&unprepared, 100, 80, card(20, 20, 50, 30), 12);
    assert!(got.iter().any(|&a| a > 0));
}

#[test]
fn prepare_shares_tables_by_radius_and_drops_the_unused() {
    let mut shadow = Shadow::new(SOFT);
    shadow.prepare(12, [(300, 60), (300, 90), (300, 200)]);
    assert_eq!(shadow.corners.len(), 1, "any height of one radius shares");
    shadow.prepare(12, [(300, 60), (300, 18)]);
    let mut radii: Vec<u32> = shadow.corners.iter().map(|corner| corner.radius).collect();
    radii.sort_unstable();
    assert_eq!(
        radii,
        [9, 12],
        "a short card needs its own, the old tables go"
    );
    shadow.prepare(12, std::iter::empty());
    assert!(shadow.corners.is_empty());
}

#[test]
fn the_shadow_composites_over_what_is_there() {
    // Over an opaque pixel: darker, still opaque. Over a transparent one:
    // the shadow's own alpha, premultiplied black (colors stay zero).
    let shadow = shadow(SOFT, 8, &[(40, 30)]);
    let mut pixels = vec![0u8; 80 * 70 * 4];
    for pixel in pixels.as_chunks_mut::<4>().0.iter_mut().take(80 * 19) {
        pixel.copy_from_slice(&[200, 200, 200, 255]);
    }
    let mut canvas = Canvas::new(&mut pixels, 80, 70).unwrap();
    shadow.paint(&mut canvas, card(20, 20, 40, 30), 8, BLACK);
    let top_row = &pixels[(16 * 80 + 40) * 4..][..4];
    assert_eq!(top_row[3], 255, "opaque stays opaque");
    assert!(top_row[0] < 200, "and darker: {top_row:?}");
    let empty = &pixels[(53 * 80 + 40) * 4..][..4];
    assert_eq!(&empty[..3], &[0, 0, 0]);
    assert!(empty[3] > 0 && empty[3] <= SOFT.alpha);
    // A tinted glow premultiplies its color by the strength.
    let mut tinted = vec![0u8; 80 * 70 * 4];
    let mut canvas = Canvas::new(&mut tinted, 80, 70).unwrap();
    let ember = Color {
        r: 255,
        g: 128,
        b: 0,
    };
    shadow.paint(&mut canvas, card(20, 20, 40, 30), 8, ember);
    let lit = &tinted[(53 * 80 + 40) * 4..][..4];
    assert_eq!(lit[3], empty[3]);
    assert_eq!(
        lit[2], lit[3],
        "red is full strength: premultiplied equals alpha"
    );
    assert!(lit[0] == 0 && lit[1] > 0 && lit[1] < lit[3]);
}
