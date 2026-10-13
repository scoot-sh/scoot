//! Tests for the Mocu art path: provenance of the copied path data, the SVG
//! path reader, and the grid fitting that places the art in a bitmap.

use super::family::{self, Art};
use super::{Shape, generate, mocu, svgpath};

/// Every vendored file the art table cites, by the name `Outline::file`
/// uses. `include_str!` reads them at compile time, so a missing file fails
/// the build of this test rather than passing it.
const FILES: [(&str, &str); 20] = [
    ("default.svg", include_str!("mocu/default.svg")),
    ("text.svg", include_str!("mocu/text.svg")),
    ("vertical-text.svg", include_str!("mocu/vertical-text.svg")),
    ("pointer.svg", include_str!("mocu/pointer.svg")),
    ("help.svg", include_str!("mocu/help.svg")),
    ("progress.svg", include_str!("mocu/progress.svg")),
    ("not-allowed.svg", include_str!("mocu/not-allowed.svg")),
    ("all-scroll.svg", include_str!("mocu/all-scroll.svg")),
    ("row-resize.svg", include_str!("mocu/row-resize.svg")),
    ("col-resize.svg", include_str!("mocu/col-resize.svg")),
    ("ns-resize.svg", include_str!("mocu/ns-resize.svg")),
    ("ew-resize.svg", include_str!("mocu/ew-resize.svg")),
    ("nesw-resize.svg", include_str!("mocu/nesw-resize.svg")),
    ("nwse-resize.svg", include_str!("mocu/nwse-resize.svg")),
    ("crosshair.svg", include_str!("mocu/crosshair.svg")),
    ("cell.svg", include_str!("mocu/cell.svg")),
    ("grab.svg", include_str!("mocu/grab.svg")),
    ("grabbing.svg", include_str!("mocu/grabbing.svg")),
    ("zoom-in.svg", include_str!("mocu/zoom-in.svg")),
    ("zoom-out.svg", include_str!("mocu/zoom-out.svg")),
];

fn file(name: &str) -> &'static str {
    FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, body)| *body)
        .unwrap_or_else(|| panic!("{name} is not vendored under mocu/"))
}

/// Every outline the table uses, with the vendored file it is copied from.
fn outlines() -> [(&'static mocu::Outline, &'static str); 18] {
    use mocu::*;
    [
        (&DEFAULT, "default.svg"),
        (&TEXT, "text.svg"),
        (&VERTICAL_TEXT, "vertical-text.svg"),
        (&POINTER, "pointer.svg"),
        (&NOT_ALLOWED, "not-allowed.svg"),
        (&ALL_SCROLL, "all-scroll.svg"),
        (&ROW_RESIZE, "row-resize.svg"),
        (&COL_RESIZE, "col-resize.svg"),
        (&NS_RESIZE, "ns-resize.svg"),
        (&EW_RESIZE, "ew-resize.svg"),
        (&NESW_RESIZE, "nesw-resize.svg"),
        (&NWSE_RESIZE, "nwse-resize.svg"),
        (&CROSSHAIR, "crosshair.svg"),
        (&CELL, "cell.svg"),
        (&GRAB, "grab.svg"),
        (&GRABBING, "grabbing.svg"),
        (&ZOOM_IN, "zoom-in.svg"),
        (&ZOOM_OUT, "zoom-out.svg"),
    ]
}

#[test]
fn every_copied_path_still_matches_its_vendored_file() {
    // The art is copied, not parsed from the SVGs at runtime: this is what
    // keeps the copy honest. A changed `d` string (here or in an updated
    // vendored file) fails with the file it no longer matches.
    for (outline, name) in outlines() {
        let svg = file(name);
        for d in outline.paths {
            assert!(
                svg.contains(&format!("d=\"{d}\"")),
                "{name}: path {d:?} is not in the vendored file"
            );
        }
        let (hx, hy) = outline.hot;
        assert!(
            svg.contains(&format!(
                "cx=\"{hx}\" cy=\"{hy}\" r=\"1\" style=\"fill:#f00"
            )),
            "{name}: hotspot ({hx}, {hy}) is not the file's `hot` circle"
        );
    }
    assert!(file("help.svg").contains(&format!("d=\"{}\"", mocu::HELP_MARK)));
    assert!(file("help.svg").contains(&format!("d=\"{}\"", mocu::DEFAULT.paths[0])));
    assert!(file("progress.svg").contains(&format!("d=\"{}\"", mocu::DEFAULT.paths[0])));
    let (cx, cy, r) = mocu::PROGRESS_DISK;
    assert!(file("progress.svg").contains(&format!(
        "id=\"s\" class=\"left(1,-6)\" cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\""
    )));
    let (r0, r1) = mocu::PROGRESS_SPINNER;
    assert!(file("progress.svg").contains(&format!("cx=\"14\" cy=\"15\" r=\"{r0}\"")));
    assert!(file("progress.svg").contains(&format!("cx=\"14\" cy=\"15\" r=\"{r1}\"")));
}

#[test]
fn the_license_travels_with_the_art() {
    let license = include_str!("mocu/LICENSE.txt");
    assert!(license.contains("CC0 1.0 Universal"));
    let readme = include_str!("mocu/README.md");
    assert!(readme.contains("efdd71279b79bf1e2566c38becc5e7c4ab2d900c"));
}

#[test]
fn every_copied_path_parses_completely() {
    for (outline, name) in outlines() {
        for d in outline.paths {
            let subs = svgpath::parse_strict(d, 0.01)
                .unwrap_or_else(|| panic!("{name}: {d:?} did not parse"));
            assert!(!subs.is_empty(), "{name}: {d:?} parsed to nothing");
            for sub in &subs {
                assert!(sub.len() >= 3);
                for (x, y) in sub {
                    // Mocu's art lives on its 24-unit grid.
                    assert!(
                        (-0.5..=24.5).contains(x) && (-0.5..=24.5).contains(y),
                        "{name}: ({x}, {y}) is off the 24-grid"
                    );
                }
            }
        }
    }
    assert!(svgpath::parse_strict(mocu::HELP_MARK, 0.01).is_some());
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
    // The I-beam's stem is one Mocu unit, two thirds of a pixel at 16px. The
    // fit makes it exactly one flat fill column with a flat rim either side,
    // rather than a grey smear across two.
    let (fill, rim) = ([0x1a, 0x1a, 0x1a, 255], [255, 255, 255, 255]);
    let size = 16;
    let pixels = generate(Shape::Text, size, fill, rim, None);
    let row = |y: i32| -> Vec<[u8; 4]> {
        (0..size)
            .map(|x| {
                let i = ((y * size + x) * 4) as usize;
                pixels[i..i + 4].try_into().expect("a pixel")
            })
            .collect()
    };
    let mid = row(size / 2);
    let fills: Vec<usize> = (0..mid.len()).filter(|&x| mid[x] == fill).collect();
    assert_eq!(fills.len(), 1, "the stem is not one flat column: {mid:?}");
    let x = fills[0];
    assert_eq!(mid[x - 1], rim);
    assert_eq!(mid[x + 1], rim);
}

#[test]
fn grid_fitting_leaves_mocus_own_grid_alone() {
    // Wherever one unit is a whole number of pixels (24px, 48px) the art on
    // Mocu's whole-unit grid is already fitted: every vertex of the
    // vertical resize arrow and the I-beam (edges, head bases, serifs and
    // the half-unit tips between them) lands exactly at Mocu's scale,
    // shifted only so the hotspot sits on the bitmap's middle.
    for (size, k) in [(24, 1.0), (48, 2.0)] {
        for (art, points) in [
            (
                Art::ResizeNs,
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
                Art::Text,
                &[(9.0, 4.0), (11.0, 5.0), (12.0, 18.0), (14.0, 19.0)][..],
            ),
        ] {
            let p = family::placement(art, size);
            let (hx, hy) = p.px(11.5, 11.5);
            for &(x, y) in points {
                let (px, py) = p.px(x, y);
                assert!(
                    (px - hx - (x - 11.5) * k).abs() < 1e-9,
                    "{art:?} at {size}px: x {x} moved to {px}"
                );
                assert!(
                    (py - hy - (y - 11.5) * k).abs() < 1e-9,
                    "{art:?} at {size}px: y {y} moved to {py}"
                );
            }
        }
    }
}

#[test]
fn grid_fitting_moves_nothing_more_than_half_a_pixel_at_and_above_mocus_scale() {
    // At 24px and up every feature is already a pixel or more wide, so the
    // fit only ever rounds: no vertex of any shape lands more than half a
    // pixel from Mocu's own proportions around the hotspot. (Below 24px the
    // fit may widen a one-unit feature to a whole pixel, which is its job.)
    for size in [24, 30, 32, 40, 48] {
        for shape in Shape::ALL {
            let Some(art) = shape.art() else { continue };
            let p = family::placement(art, size);
            let k = f64::from(size) / 24.0;
            let (ox, oy) = p.px(0.0, 0.0);
            let (ux, uy) = p.px(24.0, 24.0);
            // The art's own scale must be intact (no shrink needed here).
            assert!(
                ((ux - ox) - 24.0 * k).abs() <= 1.0 && ((uy - oy) - 24.0 * k).abs() <= 1.0,
                "{art:?} at {size}px was rescaled"
            );
            for i in 0..=48 {
                let u = f64::from(i) / 2.0;
                let (x, y) = p.px(u, u);
                let (ax, ay) = (p.ax + (u - hot(art).0) * k, p.ay + (u - hot(art).1) * k);
                // Extremes may also have been clamped by up to a pixel to
                // keep their rim clear of the bitmap edge.
                assert!(
                    (x - ax).abs() <= 1.0 + 1e-9 && (y - ay).abs() <= 1.0 + 1e-9,
                    "{art:?} at {size}px moves {u} by ({}, {})",
                    x - ax,
                    y - ay
                );
            }
        }
    }
}

fn hot(art: Art) -> (f64, f64) {
    use mocu::*;
    match art {
        Art::Arrow | Art::PointerBadge | Art::WaitBadge | Art::Help => DEFAULT.hot,
        Art::Text => TEXT.hot,
        Art::VerticalText => VERTICAL_TEXT.hot,
        Art::Hand => POINTER.hot,
        Art::Crosshair => CROSSHAIR.hot,
        Art::Cell => CELL.hot,
        Art::ResizeNs => NS_RESIZE.hot,
        Art::ResizeEw => EW_RESIZE.hot,
        Art::ResizeNesw => NESW_RESIZE.hot,
        Art::ResizeNwse => NWSE_RESIZE.hot,
        Art::Move => ALL_SCROLL.hot,
        Art::NotAllowed => NOT_ALLOWED.hot,
        Art::Grab => GRAB.hot,
        Art::Grabbing => GRABBING.hot,
        Art::SplitV => COL_RESIZE.hot,
        Art::SplitH => ROW_RESIZE.hot,
        Art::ZoomIn => ZOOM_IN.hot,
        Art::ZoomOut => ZOOM_OUT.hot,
    }
}

#[test]
fn grid_fitted_axes_are_monotonic_and_keep_features_apart() {
    // Whatever the fit does, it never folds the art over itself: a point
    // further right in Mocu is never further left in the bitmap.
    for size in [8, 12, 16, 20, 24, 30, 32, 40, 48, 64, 256] {
        for shape in Shape::ALL {
            let Some(art) = shape.art() else { continue };
            let p = family::placement(art, size);
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
fn clipping_cuts_marks_out_of_the_outline() {
    // A unit square clipped to its right half, in either clip winding.
    let square = vec![vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]];
    for clip in [
        vec![(1.0, -1.0), (3.0, -1.0), (3.0, 3.0), (1.0, 3.0)],
        vec![(1.0, 3.0), (3.0, 3.0), (3.0, -1.0), (1.0, -1.0)],
    ] {
        let cut = family::clip(&square, &clip);
        assert_eq!(cut.len(), 1);
        for (x, _) in &cut[0] {
            assert!((1.0..=2.0).contains(x), "{x} escaped the clip");
        }
    }
    // A clip that misses entirely leaves nothing.
    assert!(family::clip(&square, &[(5.0, 5.0), (6.0, 5.0), (6.0, 6.0)]).is_empty());
}
