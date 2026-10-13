//! Tests for reading Mocu: the vendored files, the SVG reader, and the
//! placement that puts the art in a bitmap.

use super::{Shape, generate, mocu, place, svgpath};

/// Mocu's own white paints (its `Mocu-White` theme): fill and stroke.
const FILL: [u8; 4] = [0xfc, 0xfc, 0xfc, 255];
const STROKE: [u8; 4] = [0x22, 0x22, 0x22, 255];

#[test]
fn every_vendored_file_reads_into_its_layers_and_hotspot() {
    for (name, svg) in mocu::FILES {
        let art = mocu::read(svg, 0.01);
        // The arrow-plus-badge cursors and the magnifiers are two outlines;
        // the rest one.
        let two = [
            "help",
            "progress",
            "context-menu",
            "alias",
            "copy",
            "no-drop",
            "zoom-in",
            "zoom-out",
            "dnd-ask",
        ];
        let want = if two.contains(&name) { 2 } else { 1 };
        assert_eq!(art.layers.len(), want, "{name}: wrong layer count");
        for layer in &art.layers {
            assert!(
                [1.75, 2.0].contains(&layer.stroke),
                "{name}: stroke {}",
                layer.stroke
            );
            assert!(!layer.subs.is_empty(), "{name}: an empty outline");
            for (x, y) in layer.subs.iter().flatten() {
                assert!(
                    (-0.5..=24.5).contains(x) && (-0.5..=24.5).contains(y),
                    "{name}: ({x}, {y}) is off the 24-grid"
                );
            }
        }
        // The hotspot is the file's `hot` circle, verbatim.
        let (hx, hy) = art.hot;
        assert!(
            svg.contains(&format!(
                "cx=\"{hx}\" cy=\"{hy}\" r=\"1\" style=\"fill:#f00"
            )),
            "{name}: hotspot ({hx}, {hy}) is not the file's"
        );
        // Only the two busy cursors carry the spinner: a disk seen through
        // 38 grey wedges, a white dot and a black hub.
        let busy = name == "progress" || name == "wait";
        assert_eq!(art.spinner.is_some(), busy, "{name}: spinner");
        if let Some(spinner) = &art.spinner {
            assert_eq!(spinner.mask.len(), 40, "{name}: mask entries");
            assert_eq!(spinner.mask[0].1, 1.0, "{name}: the leading wedge is white");
            assert_eq!(spinner.mask[37].1, 0.0, "{name}: the last wedge is black");
            assert_eq!(spinner.mask[38].1, 1.0, "{name}: the dot is white");
            assert_eq!(spinner.mask[39].1, 0.0, "{name}: the hub is black");
        }
    }
}

#[test]
fn every_path_in_every_vendored_file_parses_completely() {
    for (name, svg) in mocu::FILES {
        for d in svg
            .split(" d=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
        {
            assert!(
                svgpath::parse_strict(d, 0.01).is_some(),
                "{name}: {d:?} did not parse"
            );
        }
    }
}

#[test]
fn the_license_travels_with_the_art() {
    let license = include_str!("mocu/LICENSE.txt");
    assert!(license.contains("CC0 1.0 Universal"));
    let readme = include_str!("mocu/README.md");
    assert!(readme.contains("efdd71279b79bf1e2566c38becc5e7c4ab2d900c"));
}

#[test]
fn the_reader_skips_what_it_does_not_understand() {
    // Garbage and truncation read as empty art, never a panic.
    for svg in [
        "",
        "<",
        "<svg",
        "<use xlink:href=\"#x\" style=\"stroke-width:2\"/>",
        "<<>>",
    ] {
        let art = mocu::read(svg, 0.1);
        assert!(art.layers.is_empty() && art.spinner.is_none());
    }
}

#[test]
fn the_path_reader_handles_relative_implicit_and_compact_syntax() {
    // `m` then bare pairs are implicit relative linetos; `h`/`v` relative.
    let subs = svgpath::parse_strict("m1 1 2 0 0 2h-2z", 0.1).expect("parses");
    assert_eq!(
        subs,
        vec![vec![(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]]
    );
    // Absolute forms, and numbers run together the way SVG allows.
    let subs = svgpath::parse_strict("M0,0L4-0V-.5e1H0Z", 0.1).expect("parses");
    assert_eq!(
        subs,
        vec![vec![(0.0, 0.0), (4.0, 0.0), (4.0, -5.0), (0.0, -5.0)]]
    );
    // Two subpaths.
    let subs = svgpath::parse_strict("m0 0h1v1h-1zm2 0h1v1h-1z", 0.1).expect("parses");
    assert_eq!(subs.len(), 2);
    // Garbage ends the strict parse ...
    assert!(svgpath::parse_strict("m0 0h1v1q1 1 2 2z", 0.1).is_none());
    // ... while the forgiving one keeps what came before and never panics.
    let _ = svgpath::parse("m0 0h1v1q1 1 2 2z", 0.1);
    let _ = svgpath::parse("", 0.1);
    let _ = svgpath::parse("a", 0.1);
    let _ = svgpath::parse("m 1e400 nan", 0.1);
}

#[test]
fn arcs_flatten_onto_their_circle_within_tolerance() {
    // Two half-circle arcs of radius 5 around (5, 5): every flattened point
    // sits on the circle, and the chord sagitta respects the tolerance.
    let tol = 0.01;
    let subs = svgpath::parse_strict("m0 5a5 5 0 0 0 10 0 5 5 0 0 0-10 0z", tol).expect("parses");
    assert_eq!(subs.len(), 1);
    let pts = &subs[0];
    assert!(
        pts.len() > 16,
        "a circle of radius 5 at 0.01 needs real segments"
    );
    for (x, y) in pts {
        let r = ((x - 5.0).powi(2) + (y - 5.0).powi(2)).sqrt();
        assert!((r - 5.0).abs() < 1e-9, "({x}, {y}) is off the circle: {r}");
    }
    for w in pts.windows(2) {
        let (mx, my) = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        let r = ((mx - 5.0).powi(2) + (my - 5.0).powi(2)).sqrt();
        assert!(
            5.0 - r <= tol + 1e-9,
            "a chord sags {} past the tolerance",
            5.0 - r
        );
    }
}

#[test]
fn grid_fitting_keeps_one_unit_stems_one_whole_pixel_at_16px() {
    // The I-beam's stem is one Mocu unit, two thirds of a pixel at 16px.
    // The fit makes it exactly one flat fill column with flat stroke either
    // side, rather than a grey smear across two. (The before/after picture
    // is in the round-8 cursor report: without the fit the hands lose their
    // fingers too.)
    let size = 16;
    let pixels = generate(Shape::Text, size, FILL, STROKE);
    let row: Vec<[u8; 4]> = (0..size)
        .map(|x| {
            let i = ((size / 2 * size + x) * 4) as usize;
            pixels[i..i + 4].try_into().expect("a pixel")
        })
        .collect();
    let fills: Vec<usize> = (0..row.len()).filter(|&x| row[x] == FILL).collect();
    assert_eq!(fills.len(), 1, "the stem is not one flat column: {row:?}");
    assert_eq!(row[fills[0] - 1], STROKE);
    assert_eq!(row[fills[0] + 1], STROKE);
}

#[test]
fn at_24px_and_48px_mocus_grid_lands_exactly_where_mocu_puts_it() {
    // At 24px scoot places Mocu where Mocu's own build does: one unit per
    // pixel, every vertex at its own coordinates. At 48px every vertex is
    // at twice them -- the fit changes nothing where a unit is whole pixels.
    for (size, k) in [(24, 1.0), (48, 2.0)] {
        for (shape, points) in [
            (
                Shape::NsResize,
                &[
                    (6.0, 15.0),
                    (11.5, 21.0),
                    (17.0, 15.0),
                    (13.0, 8.0),
                    (11.5, 2.0),
                    (10.0, 8.0),
                ][..],
            ),
            (
                Shape::Text,
                &[(9.0, 4.0), (11.0, 5.0), (12.0, 18.0), (14.0, 19.0)][..],
            ),
            (
                Shape::Default,
                &[(1.0, 1.0), (1.0, 14.75), (6.66, 17.28)][..],
            ),
        ] {
            let p = place::place(&shape.art(size), size);
            for &(x, y) in points {
                let (px, py) = p.px(x, y);
                assert!(
                    (px - x * k).abs() < 1e-9 && (py - y * k).abs() < 1e-9,
                    "{shape:?} at {size}px: ({x}, {y}) landed at ({px}, {py})"
                );
            }
        }
    }
}

#[test]
fn at_24px_every_hotspot_is_mocus_own() {
    for shape in Shape::ALL {
        let (hx, hy) = shape.art(24).hot;
        assert_eq!(
            shape.hotspot(24),
            (hx.floor() as i32, hy.floor() as i32).into(),
            "{shape:?}"
        );
    }
}

#[test]
fn grid_fitting_moves_nothing_more_than_rounding_at_and_above_mocus_scale() {
    // At 24px and up every feature is already a pixel or more wide, so the
    // fit only ever rounds: across all of a shape, the offset of each point
    // from Mocu's own proportions (`u * k`) varies by at most a pixel --
    // half a pixel of rounding either way around one common shift.
    for size in [24, 30, 32, 40, 48, 64] {
        let k = f64::from(size) / 24.0;
        for shape in Shape::ALL {
            let p = place::place(&shape.art(size), size);
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for i in 0..=48 {
                let u = f64::from(i) / 2.0;
                let (x, y) = p.px(u, u);
                for (j, d) in [x - u * k, y - u * k].into_iter().enumerate() {
                    lo[j] = lo[j].min(d);
                    hi[j] = hi[j].max(d);
                }
            }
            for j in 0..2 {
                assert!(
                    hi[j] - lo[j] <= 1.0 + 1e-9,
                    "{shape:?} at {size}px: offsets span {} px on axis {j}",
                    hi[j] - lo[j]
                );
            }
        }
    }
}

#[test]
fn grid_fitted_axes_never_fold() {
    // Whatever the fit does, it never folds the art over itself: a point
    // further right in Mocu is never further left in the bitmap.
    for size in [8, 12, 16, 20, 24, 30, 32, 40, 48, 64, 256] {
        for shape in Shape::ALL {
            let p = place::place(&shape.art(size), size);
            let mut last = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for i in 0..=96 {
                let u = f64::from(i) / 4.0;
                let (x, y) = p.px(u, u);
                assert!(
                    x >= last.0 && y >= last.1,
                    "{shape:?} at {size}px folds at {u}"
                );
                last = (x, y);
            }
        }
    }
}

#[test]
fn the_spinner_fades_around_its_ring() {
    // Mocu's first animation frame: the ring is opaque stroke color at the
    // top and fades to nothing going counterclockwise -- read off the
    // stroke paint's alpha on a 48px wait cursor, left of and right of the
    // top of the ring.
    let size = 48;
    let pixels = generate(Shape::Wait, size, FILL, STROKE);
    let p = place::place(&Shape::Wait.art(size), size);
    let at = |x: f64, y: f64| {
        let (px, py) = p.px(x, y);
        let i = ((py as i32 * size + px as i32) * 4) as usize;
        pixels[i]
    };
    // On the ring (radius 5.75 around (12, 12)), 20 degrees either side of
    // the top: counterclockwise the wedges are near-white, so the dark
    // stroke paint shows; clockwise there is no wedge (the fan stops at
    // 333 degrees), so the light fill shows; a quarter turn counterclockwise
    // the wedge is mid-grey, in between. (`SVG` `rotate(-a)` turns
    // counterclockwise on screen.)
    let (dx, dy) = (2.0, (5.75f64 * 5.75 - 4.0).sqrt());
    let ccw = at(12.0 - dx, 12.0 - dy);
    let left = at(12.0 - 5.75, 12.0);
    let cw = at(12.0 + dx, 12.0 - dy);
    assert!(ccw < left && left < cw, "ccw {ccw}, left {left}, cw {cw}");
}
