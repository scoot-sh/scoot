//! A client's CSD margin -- pixels it draws outside its `xdg_surface` window
//! geometry -- never reaches the screen under a rounded session (gh #818).
//!
//! Chrome's default custom frame is a 1px square `#D6D6D6` border drawn one
//! pixel outside its geometry. The rounded clip and the ring are both
//! computed from the geometry, so the border used to poke out square past
//! the ring's curve at all four corners. The client here is that shape made
//! exact: `margin` logical pixels of `FRAME` around a geometry of `WINDOW`.
//! Every assertion samples the rendered frame, at the three scales the other
//! suites in this module use.

use super::*;

/// Pure blue, opaque: distinct from the window, popup and (sampled) ring and
/// background colors.
const FRAME_BGRA: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

fn count(pixels: &[u8], color: [u8; 4]) -> usize {
    census(pixels).get(&color).copied().unwrap_or(0)
}

fn check_margin_is_clipped(scale: f64, margin: i32) {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, scale);
    fixture.run(Step::Framed {
        color: WINDOW_BGRA,
        frame: FRAME_BGRA,
        margin,
        band: Band::Floor,
    });
    let placement = fixture.placement();
    let pixels = fixture.render();

    assert_eq!(
        count(&pixels, FRAME_BGRA),
        0,
        "scale {scale}, margin {margin}: no pixel the client drew outside its \
         geometry may reach the frame"
    );
    // The geometry itself is untouched: its edge midpoints and its centre
    // are the client's content (the clip must not eat into it).
    let clip = clip_rect(placement, scale);
    for (px, py, what) in [
        (clip.loc.x + clip.size.w / 2, clip.loc.y, "top edge"),
        (
            clip.loc.x + clip.size.w / 2,
            clip.loc.y + clip.size.h - 1,
            "bottom edge",
        ),
        (clip.loc.x, clip.loc.y + clip.size.h / 2, "left edge"),
        (
            clip.loc.x + clip.size.w - 1,
            clip.loc.y + clip.size.h / 2,
            "right edge",
        ),
        (
            clip.loc.x + clip.size.w / 2,
            clip.loc.y + clip.size.h / 2,
            "centre",
        ),
    ] {
        assert_pixel(
            &pixels,
            CANVAS,
            px,
            py,
            WINDOW_BGRA,
            &format!("scale {scale}: the geometry's {what} stays the client's content"),
        );
    }
    // Nothing of the window color lies outside the geometry rect either.
    let mut outside = 0;
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let inside = x >= clip.loc.x
                && x < clip.loc.x + clip.size.w
                && y >= clip.loc.y
                && y < clip.loc.y + clip.size.h;
            if !inside && pixel(&pixels, CANVAS, x, y) == WINDOW_BGRA {
                outside += 1;
            }
        }
    }
    assert_eq!(
        outside, 0,
        "scale {scale}: content stays inside the geometry"
    );
    // The exact symptom of the issue: the pixel diagonally outside each
    // corner is not the client's frame.
    for (px, py) in [
        (clip.loc.x - 1, clip.loc.y - 1),
        (clip.loc.x + clip.size.w, clip.loc.y - 1),
        (clip.loc.x - 1, clip.loc.y + clip.size.h),
        (clip.loc.x + clip.size.w, clip.loc.y + clip.size.h),
    ] {
        assert_ne!(pixel(&pixels, CANVAS, px, py), FRAME_BGRA);
    }
}

#[test]
fn a_csd_margin_is_clipped_at_scale_one() {
    check_margin_is_clipped(1.0, 1);
}

#[test]
fn a_wide_csd_margin_is_clipped_at_scale_one() {
    // A drop shadow's worth of margin, not Chrome's hairline.
    check_margin_is_clipped(1.0, 20);
}

#[test]
fn a_csd_margin_is_clipped_at_scale_two() {
    check_margin_is_clipped(2.0, 1);
}

#[test]
fn a_csd_margin_is_clipped_at_scale_one_point_five() {
    check_margin_is_clipped(1.5, 1);
}

#[test]
fn a_csd_margin_is_clipped_at_scale_one_point_two_five() {
    check_margin_is_clipped(1.25, 3);
}

/// A toolkit whose frame band is rounded to whole buffer pixels can straddle
/// the geometry edge at a fractional scale: the edge falls between two
/// pixels, and the compositor rounds it to the nearest one, the same way the
/// ring's inner edge is. A frame pixel inside the rounded edge is then the
/// content's own boundary pixel (the staircase still rounds it); the contract
/// is that none lies *outside* it, where it would show in the gap.
fn check_straddling_frame_stays_inside_the_edge(scale: f64, margin: i32) {
    let mut fixture = Fixture::with_radius_at_scale(10, 4, scale);
    fixture.run(Step::Framed {
        color: WINDOW_BGRA,
        frame: FRAME_BGRA,
        margin,
        band: Band::Round,
    });
    let placement = fixture.placement();
    let pixels = fixture.render();
    let clip = clip_rect(placement, scale);
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let inside = x >= clip.loc.x
                && x < clip.loc.x + clip.size.w
                && y >= clip.loc.y
                && y < clip.loc.y + clip.size.h;
            assert!(
                inside || pixel(&pixels, CANVAS, x, y) != FRAME_BGRA,
                "scale {scale}: frame pixel at ({x}, {y}) outside the geometry {clip:?}"
            );
        }
    }
}

#[test]
fn a_rounded_frame_band_stays_inside_the_edge_at_one_point_five() {
    check_straddling_frame_stays_inside_the_edge(1.5, 1);
}

#[test]
fn a_rounded_frame_band_stays_inside_the_edge_at_one_point_two_five() {
    check_straddling_frame_stays_inside_the_edge(1.25, 3);
}

/// `corner_radius = 0` wraps nothing and clips nothing (the module's square
/// baseline): the client's frame still draws. This pins the choice, so a
/// later change to it is a decision rather than a drift.
#[test]
fn a_square_session_still_draws_the_margin() {
    let mut fixture = Fixture::with_radius_at_scale(0, 4, 1.0);
    fixture.run(Step::Framed {
        color: WINDOW_BGRA,
        frame: FRAME_BGRA,
        margin: 1,
        band: Band::Floor,
    });
    let pixels = fixture.render();
    assert!(
        count(&pixels, FRAME_BGRA) > 0,
        "radius 0 leaves the surface tree unclipped"
    );
}

/// A popup lies outside its parent's geometry by design (a menu hanging off
/// an edge): the margin clip is the toplevel tree's, and must not touch it.
/// The popup here grows up and left from the geometry origin, wholly outside
/// the rect, and still draws every pixel.
#[test]
fn a_popup_outside_the_geometry_is_not_clipped() {
    const SIZE: i32 = 3;
    let mut fixture = Fixture::with_radius_at_scale(10, 4, 1.0);
    fixture.run(Step::Framed {
        color: WINDOW_BGRA,
        frame: FRAME_BGRA,
        margin: 1,
        band: Band::Floor,
    });
    fixture.run(Step::PopupOutside {
        color: POPUP_BGRA,
        size: SIZE,
    });
    let placement = fixture.placement();
    let pixels = fixture.render();
    assert_eq!(
        count(&pixels, POPUP_BGRA),
        (SIZE * SIZE) as usize,
        "the popup draws in full"
    );
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            if pixel(&pixels, CANVAS, x, y) == POPUP_BGRA {
                assert!(
                    x < placement.x || y < placement.y,
                    "the popup at ({x}, {y}) must lie outside the geometry rect \
                     {placement:?}, or this test proves nothing"
                );
            }
        }
    }
    assert_eq!(
        count(&pixels, FRAME_BGRA),
        0,
        "the toplevel is still clipped"
    );
}

/// A second frame after a content-only commit still has no margin: the clip
/// is per draw, not a one-time property of the first.
#[test]
fn the_margin_stays_clipped_across_frames() {
    let mut fixture = Fixture::with_radius_at_scale(10, 4, 1.0);
    fixture.run(Step::Framed {
        color: WINDOW_BGRA,
        frame: FRAME_BGRA,
        margin: 1,
        band: Band::Floor,
    });
    for _ in 0..3 {
        let pixels = fixture.render();
        assert_eq!(count(&pixels, FRAME_BGRA), 0);
    }
}

/// A client that sets no window geometry has the whole surface as its
/// geometry: nothing to clip, and no rect to blank the window with.
#[test]
fn a_window_without_a_margin_is_unaffected() {
    let mut fixture = Fixture::with_radius_at_scale(10, 4, 1.0);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();
    let clip = clip_rect(placement, 1.0);
    assert_pixel(
        &pixels,
        CANVAS,
        clip.loc.x + clip.size.w / 2,
        clip.loc.y,
        WINDOW_BGRA,
        "the top edge",
    );
    assert_eq!(
        count(&pixels, WINDOW_BGRA),
        (clip.size.w * clip.size.h) as usize - corner_cut_pixels(10),
        "the window draws its whole rect less the four staircases"
    );
}

fn corner_cut_pixels(radius: i32) -> usize {
    4 * (0..radius)
        .map(|row| cut_width(radius, row) as usize)
        .sum::<usize>()
}

// -- Rounded::clipped_to: the opaque region ------------------------------------

fn opaque_area(wrapped: &impl Element) -> i32 {
    wrapped
        .opaque_regions(1.0.into())
        .into_iter()
        .map(|rect| rect.size.w * rect.size.h)
        .sum()
}

/// The element-relative opaque region never reaches past the bounds: a
/// region that did would let the damage tracker skip what is below pixels the
/// clipped draw leaves untouched. The stub claims its whole 200x150 rect
/// opaque, exactly as a client buffer with a margin does.
#[test]
fn the_opaque_region_is_cut_to_the_bounds() {
    let geom = Rectangle::<i32, Physical>::new((40, 30).into(), (200, 150).into());
    // The geometry: one pixel in from the buffer on every side.
    let bounds = Rectangle::<i32, Physical>::new((41, 31).into(), (198, 148).into());
    let wrapped = Rounded::new(
        OpaqueStub {
            id: Id::new(),
            geom,
        },
        bounds,
        8,
    )
    .clipped_to(bounds);
    let regions: Vec<_> = wrapped.opaque_regions(1.0.into()).into_iter().collect();
    for rect in &regions {
        let global = Rectangle::new(rect.loc + geom.loc, rect.size);
        assert_eq!(
            global.intersection(bounds),
            Some(global),
            "{global:?} escapes the bounds {bounds:?}"
        );
    }
    assert_eq!(opaque_area(&wrapped), 198 * 148 - 4 * 8 * 8);
}

/// Empty bounds clip nothing (a window with no usable geometry draws as it
/// always did), and with no bounds the region is the corner-cut one alone.
#[test]
fn empty_bounds_clip_nothing() {
    let geom = Rectangle::<i32, Physical>::new((40, 30).into(), (200, 150).into());
    let wrapped = Rounded::new(
        OpaqueStub {
            id: Id::new(),
            geom,
        },
        geom,
        8,
    )
    .clipped_to(Rectangle::new((0, 0).into(), (0, 0).into()));
    assert_eq!(opaque_area(&wrapped), 200 * 150 - 4 * 8 * 8);
}

/// Relocation moves the bounds with the clip: the capture path re-renders a
/// region of the output as a relocation of everything in it.
#[test]
fn relocating_moves_the_bounds_with_the_element() {
    let geom = Rectangle::<i32, Physical>::new((40, 30).into(), (200, 150).into());
    let bounds = Rectangle::<i32, Physical>::new((41, 31).into(), (198, 148).into());
    let make = || {
        Rounded::new(
            OpaqueStub {
                id: Id::new(),
                geom,
            },
            bounds,
            8,
        )
        .clipped_to(bounds)
    };
    let before = opaque_area(&make());
    let moved = make().relocate((-25, -9).into());
    assert_eq!(opaque_area(&moved), before);
}
