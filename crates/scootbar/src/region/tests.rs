use super::*;

fn rects(width: u32, height: u32, radius: u32) -> Vec<Rect> {
    let mut out = Vec::new();
    input_rects(width, height, radius, &mut out);
    out
}

/// Whether `(x, y)` is in any of `rects`.
fn hit(rects: &[Rect], x: u32, y: u32) -> bool {
    rects
        .iter()
        .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

#[test]
fn square_is_the_one_whole_rectangle() {
    assert_eq!(
        rects(100, 20, 0),
        [Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 20
        }]
    );
}

#[test]
fn a_cut_corner_is_out_and_the_edges_and_the_middle_are_in() {
    let shape = rects(200, 40, 20);
    for (x, y) in [(0, 0), (199, 0), (0, 39), (199, 39), (1, 1), (2, 3)] {
        assert!(!hit(&shape, x, y), "({x},{y}) is a cut corner");
    }
    for (x, y) in [(20, 0), (180, 0), (0, 20), (199, 20), (100, 39), (10, 10)] {
        assert!(hit(&shape, x, y), "({x},{y}) is the bar");
    }
}

#[test]
fn every_covered_pixel_is_clickable_and_the_shape_is_symmetric() {
    // The pixels the paint draws at any coverage are all in: no visible
    // pixel is dead. The four corners mirror.
    let (w, h, r) = (120u32, 36u32, 14u32);
    let shape = rects(w, h, r);
    let corners = crate::paint::Corners::new(r);
    for y in 0..r {
        for x in 0..r {
            let covered = corners.coverage_at(x, y) > 0;
            if covered {
                assert!(hit(&shape, x, y), "covered ({x},{y}) is not clickable");
            }
            let mirrors = [(w - 1 - x, y), (x, h - 1 - y), (w - 1 - x, h - 1 - y)];
            for (mx, my) in mirrors {
                assert_eq!(hit(&shape, mx, my), hit(&shape, x, y), "({x},{y}) mirror");
            }
        }
    }
}

#[test]
fn the_rectangles_do_not_overlap_and_stay_few() {
    for (w, h, r) in [(100, 28, 14), (100, 28, 3), (28, 28, 14), (1600, 1024, 512)] {
        let shape = rects(w, h, r);
        assert!(shape.len() <= 2 * effective_radius(r, w, h) as usize + 1);
        let area: u64 = shape
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum();
        // Overlap would count a pixel twice: the area must equal the
        // union, checked over the whole shape for the small cases.
        if u64::from(w) * u64::from(h) <= 100_000 {
            let union = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| hit(&shape, x, y))
                .count() as u64;
            assert_eq!(area, union, "{w}x{h} r{r}: rectangles overlap");
        }
    }
}

#[test]
fn a_radius_too_big_for_the_bar_is_cut_back_and_nothing_underflows() {
    // A 10 x 6 bar can hold a radius of 3.
    assert_eq!(effective_radius(1000, 10, 6), 3);
    let shape = rects(10, 6, 1000);
    assert!(hit(&shape, 5, 3));
    assert!(!hit(&shape, 0, 0));
    for (w, h) in [(0, 0), (0, 5), (5, 0), (1, 1), (2, 1), (1, 2)] {
        for r in [0, 1, 7, u32::MAX] {
            let _ = rects(w, h, r);
        }
    }
    assert!(rects(0, 5, 3).is_empty());
}
