//! Tests for the procedural cursor shapes.
//!
//! All of this is pure arithmetic over a byte buffer, so none of it needs a
//! renderer, a client or a [`State`](crate::compositor::State) -- the tests
//! read the pixels [`generate`] produced directly. What they are actually
//! guarding is the two things that are easy to get silently wrong in a
//! rasterizer and invisible to "it compiles": a shape drawing *outside* its
//! own bitmap (clipped away by [`plot`], so it never crashes -- it just
//! quietly loses an arrowhead), and a shape drawing *nothing* at a size a
//! config is allowed to ask for.

use super::*;
use crate::compositor::decorations::Appearance;

/// An opaque white fill and the black outline `Cursor::new` pairs with it,
/// in the premultiplied little-endian `Argb8888` the bitmaps are built in.
const FILL: [u8; 4] = [255, 255, 255, 255];
const OUTLINE: [u8; 4] = [0, 0, 0, 255];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// The default cursor size, and the two ends of the range a config may ask
/// for. Every shape is checked at all three rather than at the default alone:
/// the smallest is where a shape's fractions collapse toward zero and the
/// largest is where its arithmetic is furthest from the sizes it was drawn
/// against.
fn sizes() -> [i32; 3] {
    [
        Appearance::MIN_CURSOR_SIZE,
        Appearance::default().cursor_size,
        Appearance::MAX_CURSOR_SIZE,
    ]
}

fn pixel(pixels: &[u8], size: i32, x: i32, y: i32) -> [u8; 4] {
    let idx = ((y * size + x) * 4) as usize;
    pixels[idx..idx + 4].try_into().expect("a whole pixel")
}

fn count(pixels: &[u8], size: i32, wanted: [u8; 4]) -> usize {
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(pixels, size, *x, *y) == wanted)
        .count()
}

/// Every shape except the arrow, which `cursor.rs` draws itself.
fn drawn_here() -> impl Iterator<Item = Shape> {
    Shape::ALL.into_iter().filter(|s| *s != Shape::Arrow)
}

// -------------------------------------------------------------------------
// The mapping
// -------------------------------------------------------------------------

#[test]
fn every_shape_has_its_own_slot() {
    for (slot, shape) in Shape::ALL.into_iter().enumerate() {
        assert_eq!(
            shape.index(),
            slot,
            "{shape:?} does not index the slot `Shape::ALL` puts it in; \
             `Cursor` would draw a different shape than the one requested"
        );
    }
}

#[test]
fn named_icons_map_to_the_shape_that_means_them() {
    let cases = [
        (CursorIcon::Default, Shape::Arrow),
        (CursorIcon::Text, Shape::Text),
        (CursorIcon::VerticalText, Shape::VerticalText),
        (CursorIcon::Crosshair, Shape::Crosshair),
        (CursorIcon::Cell, Shape::Crosshair),
        (CursorIcon::NResize, Shape::ResizeNs),
        (CursorIcon::SResize, Shape::ResizeNs),
        (CursorIcon::NsResize, Shape::ResizeNs),
        (CursorIcon::RowResize, Shape::ResizeNs),
        (CursorIcon::EResize, Shape::ResizeEw),
        (CursorIcon::WResize, Shape::ResizeEw),
        (CursorIcon::EwResize, Shape::ResizeEw),
        (CursorIcon::ColResize, Shape::ResizeEw),
        (CursorIcon::NeResize, Shape::ResizeNesw),
        (CursorIcon::SwResize, Shape::ResizeNesw),
        (CursorIcon::NeswResize, Shape::ResizeNesw),
        (CursorIcon::NwResize, Shape::ResizeNwse),
        (CursorIcon::SeResize, Shape::ResizeNwse),
        (CursorIcon::NwseResize, Shape::ResizeNwse),
        (CursorIcon::Move, Shape::Move),
        (CursorIcon::AllScroll, Shape::Move),
        (CursorIcon::AllResize, Shape::Move),
        (CursorIcon::Grab, Shape::Move),
        (CursorIcon::Grabbing, Shape::Move),
        (CursorIcon::NotAllowed, Shape::NotAllowed),
        (CursorIcon::NoDrop, Shape::NotAllowed),
        // Not drawn specifically, and deliberately: an arrow is a better
        // answer than a shape nobody can read at 16 pixels.
        (CursorIcon::Help, Shape::Arrow),
        (CursorIcon::Wait, Shape::Arrow),
        (CursorIcon::Progress, Shape::Arrow),
        (CursorIcon::Pointer, Shape::Arrow),
        (CursorIcon::ZoomIn, Shape::Arrow),
        (CursorIcon::ContextMenu, Shape::Arrow),
    ];
    for (icon, expected) in cases {
        assert_eq!(Shape::for_icon(icon), expected, "for {icon:?}");
    }
}

#[test]
fn the_arrow_points_at_the_pointer_and_the_rest_are_centred() {
    let size = 16;
    assert_eq!(
        Shape::Arrow.hotspot(size),
        (0, 0).into(),
        "the arrow's tip is its top-left corner, so that is where the pointer is"
    );
    for shape in drawn_here() {
        assert_eq!(
            shape.hotspot(size),
            (size / 2, size / 2).into(),
            "{shape:?} is symmetric about its middle, so that is its hotspot"
        );
    }
}

// -------------------------------------------------------------------------
// The pixels
// -------------------------------------------------------------------------

#[test]
fn every_shape_fills_its_buffer_exactly() {
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = generate(shape, size, FILL, OUTLINE);
            assert_eq!(
                pixels.len(),
                (size * size * 4) as usize,
                "{shape:?} at {size}px produced the wrong number of bytes"
            );
        }
    }
}

#[test]
fn every_drawn_shape_actually_draws_something_at_every_allowed_size() {
    for size in sizes() {
        for shape in drawn_here() {
            let pixels = generate(shape, size, FILL, OUTLINE);
            assert!(
                count(&pixels, size, FILL) > 0,
                "{shape:?} at {size}px drew no fill at all -- its fractions \
                 collapsed to nothing at this size"
            );
            assert!(
                count(&pixels, size, OUTLINE) > 0,
                "{shape:?} at {size}px drew no outline, so it would be \
                 invisible against content of a similar colour"
            );
        }
    }
}

#[test]
fn the_arrow_is_not_drawn_here() {
    // `Cursor::new` routes `Shape::Arrow` to `cursor.rs`'s own
    // `generate_bitmap` instead of to this module (see `generate`'s doc).
    // This is that contract asserted rather than left as a comment: if a
    // future change starts drawing an arrow here, `Cursor::new`'s match would
    // be silently drawing the wrong one of the two.
    let size = 16;
    let pixels = generate(Shape::Arrow, size, FILL, OUTLINE);
    assert_eq!(
        count(&pixels, size, TRANSPARENT),
        (size * size) as usize,
        "`shapes::generate` drew an arrow; `Cursor::new` does not call it for one"
    );
}

#[test]
fn no_shape_draws_fill_on_the_edge_of_its_own_bitmap() {
    // The safe box `draw` clamps every measurement into. A fill pixel on the
    // bitmap's edge has nowhere to put its outline on that side, so it would
    // meet whatever is under the cursor with no outline between them --
    // which is the one thing the outline exists to prevent. Checked at every
    // allowed size because the box is a fraction of it, and the smallest
    // size is where the fractions collapse.
    for size in sizes() {
        for shape in drawn_here() {
            let pixels = generate(shape, size, FILL, OUTLINE);
            for i in 0..size {
                for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
                    assert_ne!(
                        pixel(&pixels, size, x, y),
                        FILL,
                        "{shape:?} at {size}px puts fill on the bitmap edge at ({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn every_fill_pixel_is_wrapped_in_outline() {
    // The property `ink` exists for: no fill pixel may sit directly against a
    // transparent one, in any of the eight directions, or the shape has an
    // unoutlined edge there. A neighbour *outside* the bitmap counts as a
    // failure for the same reason -- there is no pixel there to hold the
    // outline, so the shape's edge is bare. Asserted over whole bitmaps
    // rather than spot checks, since it is cheap and it is the only thing
    // keeping a thin stroke legible.
    for size in sizes() {
        for shape in drawn_here() {
            let pixels = generate(shape, size, FILL, OUTLINE);
            for y in 0..size {
                for x in 0..size {
                    if pixel(&pixels, size, x, y) != FILL {
                        continue;
                    }
                    for oy in -1..=1 {
                        for ox in -1..=1 {
                            let (nx, ny) = (x + ox, y + oy);
                            assert!(
                                nx >= 0 && ny >= 0 && nx < size && ny < size,
                                "{shape:?} at {size}px has fill at ({x}, {y}), whose \
                                 ({nx}, {ny}) neighbour is off the bitmap -- its \
                                 outline is clipped there"
                            );
                            assert_ne!(
                                pixel(&pixels, size, nx, ny),
                                TRANSPARENT,
                                "{shape:?} at {size}px has fill at ({x}, {y}) \
                                 with bare transparency beside it at ({nx}, {ny})"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_shapes_are_distinguishable_from_one_another() {
    // The whole reason `wp-cursor-shape-v1` is worth implementing: a client
    // asking for `text` must not get the same pixels as one asking for
    // `crosshair`. Compared at the default size, which is the one a user
    // actually sees.
    let size = Appearance::default().cursor_size;
    let rendered: Vec<(Shape, Vec<u8>)> = Shape::ALL
        .into_iter()
        .filter(|s| *s != Shape::Arrow)
        .map(|shape| (shape, generate(shape, size, FILL, OUTLINE)))
        .collect();
    for (i, (shape, pixels)) in rendered.iter().enumerate() {
        for (other, other_pixels) in &rendered[i + 1..] {
            assert_ne!(
                pixels, other_pixels,
                "{shape:?} and {other:?} draw identical bitmaps at {size}px"
            );
        }
    }
}

#[test]
fn the_balanced_shapes_are_centred_on_the_bitmap() {
    // Each of these means "this moves along that axis" (or "here, exactly"),
    // which only reads correctly if neither end outweighs the other. Exact
    // mirror symmetry is not available to ask for -- a one-cell stroke on an
    // even-sized grid cannot straddle the half-pixel the mirror axis falls
    // on -- so what is asserted is the property that actually shows: the
    // centroid of everything drawn sits within one pixel of the bitmap's
    // middle. A rasterizer that biases one end of a stroke (the truncation
    // `stroke_line` deliberately rounds away) moves that centroid and shows
    // up here and nowhere else.
    for size in sizes() {
        for shape in [
            Shape::Text,
            Shape::VerticalText,
            Shape::Crosshair,
            Shape::ResizeNs,
            Shape::ResizeEw,
            Shape::ResizeNesw,
            Shape::ResizeNwse,
            Shape::Move,
            Shape::NotAllowed,
        ] {
            let pixels = generate(shape, size, FILL, OUTLINE);
            let mut drawn = 0.0;
            let (mut sum_x, mut sum_y) = (0.0, 0.0);
            for y in 0..size {
                for x in 0..size {
                    if pixel(&pixels, size, x, y) == TRANSPARENT {
                        continue;
                    }
                    drawn += 1.0;
                    sum_x += f64::from(x);
                    sum_y += f64::from(y);
                }
            }
            assert!(drawn > 0.0, "{shape:?} at {size}px drew nothing");
            let middle = f64::from(size - 1) / 2.0;
            let (cx, cy) = (sum_x / drawn, sum_y / drawn);
            assert!(
                (cx - middle).abs() <= 1.0 && (cy - middle).abs() <= 1.0,
                "{shape:?} at {size}px is centred on ({cx}, {cy}), not on ({middle}, {middle})"
            );
        }
    }
}

#[test]
fn a_shape_uses_the_colours_it_was_given_and_no_others() {
    // The bitmaps go straight to the scanout buffer, so a stray byte here is
    // a stray pixel on a real display. Three colours, no blending, no
    // antialiasing -- which is also what makes the assertions above able to
    // compare exact pixels.
    let size = 16;
    let fill = [1, 2, 3, 255];
    let outline = [4, 5, 6, 255];
    for shape in drawn_here() {
        let pixels = generate(shape, size, fill, outline);
        for y in 0..size {
            for x in 0..size {
                let pixel = pixel(&pixels, size, x, y);
                assert!(
                    pixel == fill || pixel == outline || pixel == TRANSPARENT,
                    "{shape:?} drew {pixel:?} at ({x}, {y}), which is neither \
                     colour it was given nor transparent"
                );
            }
        }
    }
}

/// Not an assertion: prints every shape as ASCII art so a human can look at
/// what the rasterizer actually produced. `#` is fill, `.` outline, space
/// transparent. Run with `cargo test -- --ignored --nocapture shape_art`.
#[test]
#[ignore = "prints the shapes for a human to look at; asserts nothing"]
fn shape_art() {
    let size = 24;
    for shape in Shape::ALL {
        let pixels = generate(shape, size, FILL, OUTLINE);
        println!("--- {shape:?} ---");
        for y in 0..size {
            let row: String = (0..size)
                .map(|x| match pixel(&pixels, size, x, y) {
                    FILL => '#',
                    OUTLINE => '.',
                    _ => ' ',
                })
                .collect();
            println!("|{row}|");
        }
    }
}
