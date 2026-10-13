//! Tests for the procedural cursor family.
//!
//! All of this is pure arithmetic over a byte buffer, so none of it needs a
//! renderer, a client or a [`State`](crate::compositor::State) -- the tests
//! read the pixels [`generate`] produced directly. What they are actually
//! guarding is the things that are easy to get silently wrong in a
//! rasterizer and invisible to "it compiles": a shape drawing *outside* its
//! own bitmap (clipped away, so it never crashes -- it just quietly loses
//! an arrowhead or a badge dot), a shape drawing *nothing* at a size a
//! config is allowed to ask for, broken premultiplication (which
//! over-brightens under pixman's `Over`), and missing anti-aliasing (which
//! would mean the supersampler regressed to a 1-bit mask).

use super::*;
use crate::compositor::decorations::Appearance;

/// The family paints the tests draw with: the near-black default body, its
/// white auto-contrasting rim, and the ginger accent -- in the premultiplied
/// little-endian `Argb8888` the bitmaps are built in.
const FILL: [u8; 4] = [0x1a, 0x1a, 0x1a, 255];
const RIM: [u8; 4] = [255, 255, 255, 255];
const ACCENT: [u8; 4] = [0x29, 0x7f, 0xe5, 255];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

fn paint(shape: Shape, size: i32) -> Vec<u8> {
    generate(shape, size, FILL, RIM, Some(ACCENT))
}

/// The default cursor size, and the two ends of the range a config may ask
/// for. Every shape is checked at all three rather than at the default alone:
/// the smallest is where a shape collapses toward the honest dot and the
/// largest is where its arithmetic is furthest from the sizes it was drawn
/// against.
fn sizes() -> [i32; 3] {
    [
        Appearance::MIN_CURSOR_SIZE,
        Appearance::default().cursor_size,
        Appearance::MAX_CURSOR_SIZE,
    ]
}

/// The sizes the brief pins the bitmap-edge guarantees to: every shape at
/// each of these must keep its fill off the edge and wrapped in rim.
fn edge_sizes() -> [i32; 8] {
    [8, 12, 16, 24, 32, 48, 64, 256]
}

fn pixel(pixels: &[u8], size: i32, x: i32, y: i32) -> [u8; 4] {
    let idx = ((y * size + x) * 4) as usize;
    pixels[idx..idx + 4].try_into().expect("a whole pixel")
}

fn alpha(pixels: &[u8], size: i32, x: i32, y: i32) -> u8 {
    pixel(pixels, size, x, y)[3]
}

fn count(pixels: &[u8], size: i32, wanted: [u8; 4]) -> usize {
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(pixels, size, *x, *y) == wanted)
        .count()
}

// -------------------------------------------------------------------------
// The mapping
// -------------------------------------------------------------------------

#[test]
fn every_shape_has_its_own_slot() {
    assert_eq!(Shape::ALL.len(), Shape::COUNT);
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
        (CursorIcon::ContextMenu, Shape::Arrow),
        (CursorIcon::Text, Shape::Text),
        (CursorIcon::VerticalText, Shape::VerticalText),
        (CursorIcon::Crosshair, Shape::Crosshair),
        (CursorIcon::Cell, Shape::Precision),
        (CursorIcon::NResize, Shape::ResizeNs),
        (CursorIcon::SResize, Shape::ResizeNs),
        (CursorIcon::NsResize, Shape::ResizeNs),
        (CursorIcon::EResize, Shape::ResizeEw),
        (CursorIcon::WResize, Shape::ResizeEw),
        (CursorIcon::EwResize, Shape::ResizeEw),
        (CursorIcon::NeResize, Shape::ResizeNesw),
        (CursorIcon::SwResize, Shape::ResizeNesw),
        (CursorIcon::NeswResize, Shape::ResizeNesw),
        (CursorIcon::NwResize, Shape::ResizeNwse),
        (CursorIcon::SeResize, Shape::ResizeNwse),
        (CursorIcon::NwseResize, Shape::ResizeNwse),
        (CursorIcon::ColResize, Shape::SplitV),
        (CursorIcon::RowResize, Shape::SplitH),
        (CursorIcon::Move, Shape::Move),
        (CursorIcon::AllScroll, Shape::Move),
        (CursorIcon::AllResize, Shape::Move),
        (CursorIcon::Grab, Shape::Grab),
        (CursorIcon::Grabbing, Shape::Grabbing),
        (CursorIcon::Pointer, Shape::Hand),
        (CursorIcon::Progress, Shape::PointerBadge),
        (CursorIcon::Alias, Shape::PointerBadge),
        (CursorIcon::Copy, Shape::PointerBadge),
        (CursorIcon::Wait, Shape::WaitBadge),
        (CursorIcon::DndAsk, Shape::Busy),
        (CursorIcon::Help, Shape::Help),
        (CursorIcon::ZoomIn, Shape::ZoomIn),
        (CursorIcon::ZoomOut, Shape::ZoomOut),
        (CursorIcon::NotAllowed, Shape::NotAllowed),
        (CursorIcon::NoDrop, Shape::NotAllowed),
    ];
    for (icon, expected) in cases {
        assert_eq!(Shape::for_icon(icon), expected, "for {icon:?}");
    }
}

/// The shapes that point with a feature of their art (a tip, a fingertip,
/// the lens) rather than with their middle.
const POINTING: [Shape; 9] = [
    Shape::Arrow,
    Shape::PointerBadge,
    Shape::WaitBadge,
    Shape::Help,
    Shape::Hand,
    Shape::Grab,
    Shape::Grabbing,
    Shape::ZoomIn,
    Shape::ZoomOut,
];

#[test]
fn every_hotspot_is_mocus_carried_into_the_bitmap() {
    for size in [16, 20, 24, 32, 48, 256] {
        for shape in Shape::ALL {
            let hot = shape.hotspot(size);
            assert!(
                hot.x >= 0 && hot.y >= 0 && hot.x < size && hot.y < size,
                "{shape:?} at {size}px hotspots outside its bitmap at {hot:?}"
            );
            if POINTING.contains(&shape) {
                // A pointing shape's hotspot pixel is part of the drawing:
                // the user points with what they see.
                let pixels = paint(shape, size);
                assert_ne!(
                    alpha(&pixels, size, hot.x, hot.y),
                    0,
                    "{shape:?} at {size}px points with a transparent pixel {hot:?}"
                );
            } else {
                assert_eq!(
                    hot,
                    (size / 2, size / 2).into(),
                    "{shape:?} is symmetric about its middle, so that is its hotspot"
                );
            }
        }
    }
}

#[test]
fn the_arrow_family_points_with_the_arrows_tip() {
    // The tip pixel is the hotspot: fill is drawn at and just inside it,
    // never above or left of it (only the rim is there), so the click lands
    // on the very point of the arrow at every size and on every badge.
    for size in [16, 20, 24, 32, 48, 256] {
        let tip = Shape::Arrow.hotspot(size);
        for shape in [
            Shape::Arrow,
            Shape::PointerBadge,
            Shape::WaitBadge,
            Shape::Help,
        ] {
            assert_eq!(shape.hotspot(size), tip, "{shape:?} at {size}px");
            let pixels = generate(shape, size, FILL, RIM, None);
            let deep = size / 8;
            assert_eq!(
                pixel(&pixels, size, tip.x + 1, tip.y + 2 + deep),
                FILL,
                "{shape:?} at {size}px: the arrow body is not just below its tip"
            );
            for (x, y) in [
                (tip.x - 1, tip.y),
                (tip.x, tip.y - 1),
                (tip.x - 1, tip.y - 1),
            ] {
                assert_ne!(
                    pixel(&pixels, size, x, y),
                    FILL,
                    "{shape:?} at {size}px draws fill above or left of its tip at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn tiny_sizes_draw_a_centred_dot_under_the_pointer() {
    // Below 8px no shape is legible, so every one draws the honest dot at
    // the middle -- which is also the hotspot there, so the dot sits under
    // the pointer rather than beside it.
    for size in [Appearance::MIN_CURSOR_SIZE, 5, 6, 7] {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            assert_eq!(
                shape.hotspot(size),
                (size / 2, size / 2).into(),
                "{shape:?} at {size}px hotspots away from its dot"
            );
            assert_ne!(
                alpha(&pixels, size, size / 2, size / 2),
                0,
                "{shape:?} at {size}px drew no dot at the middle"
            );
        }
    }
}

// -------------------------------------------------------------------------
// The pixels
// -------------------------------------------------------------------------

#[test]
fn every_shape_fills_its_buffer_exactly() {
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            assert_eq!(
                pixels.len(),
                (size * size * 4) as usize,
                "{shape:?} at {size}px produced the wrong number of bytes"
            );
        }
    }
}

#[test]
fn every_shape_actually_draws_something_at_every_allowed_size() {
    // Any coverage at all (flat or blended): a shape that draws nothing at
    // a size a config can ask for leaves the user pointerless.
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            let drawn = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| alpha(&pixels, size, *x, *y) > 0)
                .count();
            assert!(drawn > 0, "{shape:?} at {size}px drew nothing at all");
        }
    }
    // ... while at real sizes the fills land as flat pixels (which is what
    // keeps small exact-pixel assertions elsewhere meaningful) and every
    // shape carries a halo -- flat rim or blends -- around its fill, so
    // it stays legible against content of a similar color. Thin strokes have
    // no *flat* rim pixel by construction (a 1px band around a 2px
    // stroke is all partial coverage), which is why the halo counts blends.
    // (Flat means body *or* accent: the busy paw wears its print in accent
    // over a same-silhouette underlay, so it lands no body-colored pixel.)
    for size in [
        Appearance::default().cursor_size,
        Appearance::MAX_CURSOR_SIZE,
    ] {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            assert!(
                count(&pixels, size, FILL) + count(&pixels, size, ACCENT) > 0,
                "{shape:?} at {size}px drew no flat fill at all"
            );
            let halo = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let p = pixel(&pixels, size, *x, *y);
                    p != FILL && p != ACCENT && p != TRANSPARENT
                })
                .count();
            assert!(
                halo > 0,
                "{shape:?} at {size}px has no rim halo, so it would be \
                 invisible against content of a similar color"
            );
        }
    }
}

#[test]
fn no_shape_draws_fill_on_the_edge_of_its_own_bitmap() {
    // A fill pixel on the bitmap's edge has no room for its rim on that
    // side, so it would meet whatever is under the cursor with no rim
    // between them -- the one thing the rim exists to prevent. Checked at
    // the brief's pinned sizes (badge dots are what would escape here: the
    // "?" dot at 24px is the closest any mark comes) and at the allowed
    // extremes, where the fractions collapse (4px) or stretch (256px).
    // Round 5 extends the pinned sizes down to 8 and 12 (F3): Text at 8px
    // put flat rim on the last column, badges at 8/12px touched with
    // blends -- both fixed by growing the tiny-size insets.
    for size in edge_sizes().into_iter().chain(sizes()).collect::<Vec<_>>() {
        for shape in Shape::ALL {
            for accent in [None, Some(ACCENT)] {
                let pixels = generate(shape, size, FILL, RIM, accent);
                for i in 0..size {
                    for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
                        assert_ne!(
                            pixel(&pixels, size, x, y),
                            FILL,
                            "{shape:?} at {size}px puts fill on the bitmap edge at ({x}, {y})"
                        );
                        assert_ne!(
                            pixel(&pixels, size, x, y),
                            RIM,
                            "{shape:?} at {size}px puts flat rim on the bitmap edge at ({x}, {y})"
                        );
                        if let Some(mark) = accent {
                            assert_ne!(
                                pixel(&pixels, size, x, y),
                                mark,
                                "{shape:?} at {size}px puts accent on the bitmap edge at ({x}, {y})"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn no_shape_touches_the_bitmap_edge_at_12px_and_up() {
    // Strict transparency (F3): at 12px and above no pixel of any kind --
    // flat or blend -- may sit on the outer edge row/column. The rim's job
    // is to sit between the fill and the edge, so an edge pixel with any
    // coverage means the insets are too tight there. At 8px the tiny-size
    // insets keep flat fill/rim/accent off the edge (pinned above) but
    // blends may still touch; that is the documented small-size limit.
    for size in edge_sizes()
        .into_iter()
        .filter(|s| *s >= 12)
        .collect::<Vec<_>>()
    {
        for shape in Shape::ALL {
            for accent in [None, Some(ACCENT)] {
                let pixels = generate(shape, size, FILL, RIM, accent);
                for i in 0..size {
                    for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
                        assert_eq!(
                            alpha(&pixels, size, x, y),
                            0,
                            "{shape:?} at {size}px touches the bitmap edge at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_fill_pixel_is_wrapped_in_non_transparent_pixels() {
    // The property the rim exists for: no fill pixel may sit directly
    // against a transparent one, in any of the eight directions, or the
    // shape has an unrimmed edge there. A neighbour *outside* the bitmap
    // counts as a failure for the same reason -- there is no pixel there to
    // hold the rim, so the shape's edge is bare. Neighbours are checked
    // for *any* coverage (alpha > 0) rather than exact rim bytes,
    // because anti-aliased edge pixels are blends, not flat colors.
    // Asserted over whole bitmaps rather than spot checks, since it is cheap
    // and it is the only thing keeping a thin stroke legible.
    for size in edge_sizes() {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            for y in 0..size {
                for x in 0..size {
                    let p = pixel(&pixels, size, x, y);
                    if p != FILL && p != ACCENT {
                        continue;
                    }
                    for oy in -1..=1 {
                        for ox in -1..=1 {
                            let (nx, ny) = (x + ox, y + oy);
                            assert!(
                                nx >= 0 && ny >= 0 && nx < size && ny < size,
                                "{shape:?} at {size}px has fill at ({x}, {y}), whose \
                                 ({nx}, {ny}) neighbour is off the bitmap -- its \
                                 rim is clipped there"
                            );
                            assert_ne!(
                                alpha(&pixels, size, nx, ny),
                                0,
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
fn every_shape_is_anti_aliased() {
    // The property the supersampler exists for: at least one pixel that is
    // neither flat fill, flat rim, flat accent nor transparent. A 1-bit mask
    // would draw the same silhouettes with none. Checked at the default size
    // and up, where every shape has room for real edges.
    for size in [Appearance::default().cursor_size, 48] {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            let blended = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let p = pixel(&pixels, size, *x, *y);
                    p != FILL && p != RIM && p != ACCENT && p != TRANSPARENT
                })
                .count();
            assert!(
                blended > 0,
                "{shape:?} at {size}px has no blended edge pixels -- \
                 the supersampler regressed to a 1-bit mask"
            );
        }
    }
}

#[test]
fn every_pixel_is_premultiplied_correctly() {
    // The bitmaps go straight to the scanout buffer, where pixman composites
    // them with `Over` -- which is defined over premultiplied components. A
    // channel byte greater than the alpha byte is not a valid premultiplied
    // pixel and would over-brighten instead of blending. Every byte of every
    // shape at every size must satisfy `channel <= alpha`, including the
    // anti-aliased blends and the accent layer (which is where a
    // straight-alpha mistake shows).
    for size in edge_sizes() {
        for shape in Shape::ALL {
            for accent in [None, Some(ACCENT)] {
                let pixels = generate(shape, size, FILL, RIM, accent);
                for y in 0..size {
                    for x in 0..size {
                        let [b, g, r, a] = pixel(&pixels, size, x, y);
                        assert!(
                            b <= a && g <= a && r <= a,
                            "{shape:?} at {size}px drew {b},{g},{r},{a} at ({x}, {y}), \
                             which is not premultiplied"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_shape_uses_the_colors_it_was_given() {
    // Fully-covered pixels -- and there are many at 32px -- must be exactly
    // the colors asked for, with no channel reordering. Uses colors whose
    // four bytes are all distinct, so a swap anywhere would change the
    // assertion. Anti-aliased blends are exempt (they are mixes by design)
    // and are covered by `every_pixel_is_premultiplied_correctly` instead.
    let size = 32;
    let fill = [0x11, 0x22, 0x33, 0xff];
    let outline = [0x44, 0x55, 0x66, 0xff];
    let accent = [0x77, 0x88, 0x99, 0xff];
    for shape in Shape::ALL {
        let pixels = generate(shape, size, fill, outline, Some(accent));
        let mut exact = 0;
        for y in 0..size {
            for x in 0..size {
                let p = pixel(&pixels, size, x, y);
                if p == fill || p == outline || p == accent || p == TRANSPARENT {
                    exact += 1;
                    continue;
                }
                // A blend of the given colors: each channel between the
                // color's and zero. (Upper bounds only: the accent already
                // exceeds body and rim on every channel, so it is the
                // maximum. Fully opaque mixes are legitimate with three
                // layers -- an accent edge inside a fully-covered body still
                // composites to alpha 255 -- so only the channels are pinned
                // here, not the alpha.)
                let [b, g, r, a] = p;
                assert!(
                    a > 0,
                    "{shape:?} drew transparent {p:?} at ({x}, {y}) that matches no input"
                );
                assert!(
                    b <= 0x77 && g <= 0x88 && r <= 0x99,
                    "{shape:?} drew {p:?} at ({x}, {y}), outside every given color"
                );
            }
        }
        assert!(
            exact > 0,
            "{shape:?} at {size}px drew no exact pixels at all"
        );
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
        .map(|shape| (shape, paint(shape, size)))
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
fn disabling_the_accent_keeps_every_shape_but_removes_the_mark() {
    // `cursor_accent = "none"` must never cost the pointer: every shape still
    // draws, and no accent-colored pixel survives anywhere.
    let size = Appearance::default().cursor_size;
    for shape in Shape::ALL {
        let pixels = generate(shape, size, FILL, RIM, None);
        let drawn = (0..size)
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .filter(|(x, y)| alpha(&pixels, size, *x, *y) > 0)
            .count();
        assert!(drawn > 0, "{shape:?} vanished with the accent off");
        assert_eq!(
            count(&pixels, size, ACCENT),
            0,
            "{shape:?} still wears the accent with it disabled"
        );
    }
}

#[test]
fn the_accent_mark_lands_where_the_shape_promises() {
    // One pinned accent pixel per marked family, so a refactor that silently
    // drops a mark (or moves it off the shape) fails here rather than only
    // in a screenshot. Positions were read off the renders the contact
    // sheet judged, at the 16px default and at 24px (Mocu's own grid, one
    // unit per pixel).
    let pins: [(Shape, i32, (i32, i32), &str); 9] = [
        (Shape::Arrow, 16, (5, 11), "the arrow's tail tip"),
        (Shape::Arrow, 24, (8, 16), "the arrow's tail tip"),
        (Shape::Hand, 16, (5, 11), "the hand's cuff"),
        (Shape::ResizeEw, 16, (8, 8), "the resize shaft"),
        (Shape::Move, 16, (8, 8), "the move centre"),
        (Shape::Crosshair, 16, (8, 8), "the crosshair centre"),
        (Shape::NotAllowed, 16, (5, 5), "the not-allowed slash"),
        (Shape::SplitV, 16, (8, 8), "the split divider"),
        (Shape::ResizeNs, 16, (8, 8), "the resize shaft"),
    ];
    for (shape, size, (x, y), what) in pins {
        let pixels = paint(shape, size);
        assert_eq!(
            pixel(&pixels, size, x, y),
            ACCENT,
            "{what} left its place: {shape:?} at {size}px ({x}, {y})"
        );
    }
}

#[test]
fn the_balanced_shapes_are_centred_on_the_bitmap() {
    // Each of these means "this moves along that axis" (or "here, exactly"),
    // which only reads correctly if neither end outweighs the other. Exact
    // mirror symmetry is not available to ask for -- a stroke on an
    // even-sized grid cannot straddle the half-pixel the mirror axis falls
    // on -- so what is asserted is the property that actually shows: the
    // centroid of everything drawn sits within one pixel of the bitmap's
    // middle. A rasterizer or a grid fit that biases one end of a stroke
    // moves that centroid and shows up here and nowhere else. The pointing
    // shapes are exempt: the arrow family, the hands and the magnifiers are
    // asymmetric by design and aim with a feature of their art (see
    // `every_hotspot_is_mocus_carried_into_the_bitmap`).
    for size in sizes() {
        for shape in [
            Shape::Text,
            Shape::VerticalText,
            Shape::Crosshair,
            Shape::Precision,
            Shape::ResizeNs,
            Shape::ResizeEw,
            Shape::ResizeNesw,
            Shape::ResizeNwse,
            Shape::Move,
            Shape::NotAllowed,
            Shape::Busy,
            Shape::SplitV,
            Shape::SplitH,
        ] {
            let pixels = paint(shape, size);
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
fn tiny_paw_badges_drop_their_toes_instead_of_blurring_them() {
    // The 24-grid simplification rule ([`super::design`]): a paw toe
    // narrower than TOE_DROP_BELOW never rasterises -- the 8px pointer badge
    // keeps its pad alone. A full print keeps all four prims.
    assert_eq!(super::paw::prims(0.0, 0.0, 1.0).len(), 1);
    assert_eq!(super::paw::prims(0.0, 0.0, 4.0).len(), 4);
    // ... and the badge still draws at 8px (pad only, no toes).
    let pixels = paint(Shape::PointerBadge, 8);
    assert_ne!(alpha(&pixels, 8, 5, 5), 0);
}

/// The fractional-scale physical bitmap sizes hold their rims too: 20px
/// (16 at 1.25), 30px (24 at 1.25) and 40px are real bitmaps a hidpi
/// session rasterises, so they pin the same strict edge transparency as
/// the design sizes above.
#[test]
fn hidpi_physical_sizes_keep_their_rims_inside() {
    for size in [20, 30, 40] {
        for shape in Shape::ALL {
            for accent in [None, Some(ACCENT)] {
                let pixels = generate(shape, size, FILL, RIM, accent);
                for i in 0..size {
                    for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
                        assert_eq!(
                            alpha(&pixels, size, x, y),
                            0,
                            "{shape:?} at {size}px touches the bitmap edge at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

/// The physical bitmaps are natively rasterised, not resampled: a 32px
/// render differs from a bilinear upscale of the 16px render of the same
/// shape. (The hidpi path renders the 24-grid SDF at the target size; it
/// never renders at 24 and downsamples -- the report's side-by-side sheets
/// judge the same property by eye.)
#[test]
fn hidpi_buffers_are_natively_rasterised_not_upscaled() {
    for shape in [Shape::Arrow, Shape::Text, Shape::Hand, Shape::Move] {
        let small = generate(shape, 16, FILL, RIM, Some(ACCENT));
        let native = generate(shape, 32, FILL, RIM, Some(ACCENT));
        let upscaled = upscale_bilinear(&small, 16, 32);
        let differing = native
            .chunks_exact(4)
            .zip(upscaled.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 0,
            "{shape:?}: the 32px bitmap is pixel-identical to an upscale \
             of the 16px one -- it was resampled, not rendered"
        );
    }
}

/// Edge-clamped bilinear upscale of a premultiplied `Argb8888` bitmap, the
/// same sampling the renderer applies to a logical-size buffer on a 2x
/// output.
fn upscale_bilinear(pixels: &[u8], from: i32, to: i32) -> Vec<u8> {
    let sample = |x: i32, y: i32, channel: usize| -> f64 {
        let x = x.clamp(0, from - 1);
        let y = y.clamp(0, from - 1);
        f64::from(pixels[((y * from + x) * 4 + channel as i32) as usize])
    };
    let mut out = vec![0u8; (to * to * 4) as usize];
    for y in 0..to {
        for x in 0..to {
            // Source coordinate of the destination pixel's centre.
            let gx = (f64::from(x) + 0.5) * f64::from(from) / f64::from(to) - 0.5;
            let gy = (f64::from(y) + 0.5) * f64::from(from) / f64::from(to) - 0.5;
            let x0 = gx.floor() as i32;
            let y0 = gy.floor() as i32;
            let fx = gx - f64::from(x0);
            let fy = gy - f64::from(y0);
            for channel in 0..4 {
                let top = sample(x0, y0, channel) * (1.0 - fx) + sample(x0 + 1, y0, channel) * fx;
                let bottom =
                    sample(x0, y0 + 1, channel) * (1.0 - fx) + sample(x0 + 1, y0 + 1, channel) * fx;
                out[((y * to + x) * 4 + channel as i32) as usize] =
                    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[test]
fn the_arrow_tip_is_drawn_and_the_corner_is_not() {
    // The hotspot contract from the bitmap's side: the tip pixel itself is
    // covered (it is what the user is pointing with), while the bitmap's
    // own corner stays transparent (the tip's rim inset).
    for size in [16, 24, 32, 48, 64, 256] {
        for shape in [
            Shape::Arrow,
            Shape::PointerBadge,
            Shape::WaitBadge,
            Shape::Help,
        ] {
            let pixels = paint(shape, size);
            let tip = shape.hotspot(size);
            assert_ne!(
                alpha(&pixels, size, tip.x, tip.y),
                0,
                "{shape:?}'s tip at ({}, {}) at {size}px is transparent",
                tip.x,
                tip.y
            );
            assert_eq!(
                pixel(&pixels, size, 0, 0),
                TRANSPARENT,
                "{shape:?}'s bitmap corner at {size}px is drawn -- the tip inset is gone"
            );
        }
        // ... and the glove's fingertip likewise.
        let pixels = paint(Shape::Hand, size);
        let tip = Shape::Hand.hotspot(size);
        assert_ne!(
            alpha(&pixels, size, tip.x, tip.y),
            0,
            "the hand's fingertip at ({}, {}) at {size}px is transparent",
            tip.x,
            tip.y
        );
    }
}

/// Not an assertion: prints every shape as ASCII art so a human can look at
/// what the rasterizer actually produced. `#` is flat fill, `o` flat accent,
/// `+` a blended pixel, `.` flat rim, space transparent. Run with
/// `cargo test -- --ignored --nocapture shape_art`.
#[test]
#[ignore = "prints the shapes for a human to look at; asserts nothing"]
fn shape_art() {
    let size = 24;
    for shape in Shape::ALL {
        let pixels = paint(shape, size);
        println!("--- {shape:?} ---");
        for y in 0..size {
            let row: String = (0..size)
                .map(|x| {
                    let p = pixel(&pixels, size, x, y);
                    if p == FILL {
                        '#'
                    } else if p == ACCENT {
                        'o'
                    } else if p == RIM {
                        '.'
                    } else if p == TRANSPARENT {
                        ' '
                    } else {
                        '+'
                    }
                })
                .collect();
            println!("|{row}|");
        }
    }
}

/// Not an assertion: writes evidence PNGs -- every shape composited over a
/// light and a dark background at output scales 1, 1.5 and 2 -- to
/// `$SCOOT_CURSOR_PNG_DIR` (or `/tmp/cursor-pngs` when unset). Run with
/// `cargo test -p scoot -- --ignored --nocapture png_dumps`.
///
/// The upscale replicates what the compositor does with these bitmaps: the
/// logical-size buffer is bilinearly upscaled to physical pixels (edge
/// clamp, the `Repeat::Pad` contract in `pixman_upscale.rs`) and composited
/// over the background. What these PNGs show is therefore what a frame
/// shows, without needing a display.
#[test]
#[ignore = "writes evidence PNGs; asserts nothing"]
fn png_dumps() {
    dump_pngs(|shape, size| generate(shape, size, FILL, RIM, Some(ACCENT)));
}

/// The bitmap source behind [`png_dumps`], split out so the before/after
/// comparison in the cursor report can run the same renderer against the old
/// bitmaps: pass the old generator instead.
fn dump_pngs(make: impl Fn(Shape, i32) -> Vec<u8>) {
    let dir = std::env::var("SCOOT_CURSOR_PNG_DIR").unwrap_or_else(|_| "/tmp/cursor-pngs".into());
    // Nearest-neighbor magnification of the composited output, for looking at
    // pixels rather than shapes. 0 (unset) means no magnified copies.
    let mag: u32 = std::env::var("SCOOT_CURSOR_MAGNIFY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let size = Appearance::default().cursor_size;
    // White, and scoot's own background (the dark a cursor must survive).
    let backgrounds: [(&str, [u8; 4]); 2] = [
        ("light", [255, 255, 255, 255]),
        ("dark", [0x19, 0x14, 0x14, 255]),
    ];
    for shape in Shape::ALL {
        let bitmap = make(shape, size);
        for (bg_name, bg) in backgrounds {
            for scale in [1.0, 1.5, 2.0] {
                let (rgb, w, h) = composite_scaled(&bitmap, size, scale, bg);
                let scale_name = if scale == 1.0 {
                    "1".to_string()
                } else {
                    scale.to_string()
                };
                let path = format!("{dir}/{shape:?}_{bg_name}_x{scale_name}.png");
                write_rgb(&path, &rgb, w, h);
                if mag > 0 {
                    let mw = w * mag;
                    let mut big = Vec::with_capacity((mw * mw * 3) as usize);
                    for oy in 0..h {
                        for _ in 0..mag {
                            for ox in 0..w {
                                let px = &rgb[((oy * w + ox) * 3) as usize..][..3];
                                for _ in 0..mag {
                                    big.extend_from_slice(px);
                                }
                            }
                        }
                    }
                    write_rgb(
                        &format!("{dir}/{shape:?}_{bg_name}_x{scale_name}_mag{mag}.png"),
                        &big,
                        mw,
                        mw,
                    );
                }
            }
        }
    }
    println!("cursor PNGs written to {dir}/ (size {size})");
}

/// Writes straight RGB bytes as a PNG file.
fn write_rgb(path: &str, rgb: &[u8], w: u32, h: u32) {
    use std::io::BufWriter;
    let file = std::fs::File::create(path).unwrap_or_else(|e| panic!("create {path}: {e}"));
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap_or_else(|e| panic!("png header for {path}: {e}"))
        .write_image_data(rgb)
        .unwrap_or_else(|e| panic!("png data for {path}: {e}"));
}

/// Bilinearly upscales a premultiplied `Argb8888` bitmap by `scale` (edge
/// clamp, like the renderer's `Repeat::Pad`) and composites it over the
/// opaque `bg`, returning straight RGB bytes.
fn composite_scaled(bitmap: &[u8], size: i32, scale: f64, bg: [u8; 4]) -> (Vec<u8>, u32, u32) {
    let sample = |x: i32, y: i32, c: usize| -> f64 {
        let x = x.clamp(0, size - 1);
        let y = y.clamp(0, size - 1);
        f64::from(bitmap[((y * size + x) * 4 + c as i32) as usize]) / 255.0
    };
    let w = (f64::from(size) * scale).round() as u32;
    let h = w;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let bgf = [
        f64::from(bg[2]) / 255.0,
        f64::from(bg[1]) / 255.0,
        f64::from(bg[0]) / 255.0,
    ];
    for oy in 0..h {
        for ox in 0..w {
            // Output pixel center back in source space.
            let sx = (f64::from(ox) + 0.5) / scale - 0.5;
            let sy = (f64::from(oy) + 0.5) / scale - 0.5;
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - sx.floor();
            let fy = sy - sy.floor();
            let mut px = [0.0; 4];
            for (c, slot) in px.iter_mut().enumerate() {
                let top = sample(x0, y0, c) * (1.0 - fx) + sample(x0 + 1, y0, c) * fx;
                let bottom = sample(x0, y0 + 1, c) * (1.0 - fx) + sample(x0 + 1, y0 + 1, c) * fx;
                *slot = top * (1.0 - fy) + bottom * fy;
            }
            // `Over` an opaque background: out is opaque, so premultiplied
            // is straight.
            let a = px[3].clamp(0.0, 1.0);
            for c in [2, 1, 0] {
                let v = px[c] + bgf[c] * (1.0 - a);
                rgb.push((v * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    (rgb, w, h)
}

/// Not an assertion: times building all bitmaps at the default and
/// maximum sizes, the cost `Cursor::new`/`rebuild` pays once per startup or
/// reload. Prints to stdout; run with `--nocapture`. Used for the
/// before/after numbers in the cursor report.
#[test]
#[ignore = "prints bitmap build timings; asserts nothing"]
fn bench_build() {
    for size in [
        Appearance::default().cursor_size,
        Appearance::MAX_CURSOR_SIZE,
    ] {
        let start = std::time::Instant::now();
        let mut bytes = 0;
        for shape in Shape::ALL {
            bytes += paint(shape, size).len();
        }
        let elapsed = start.elapsed();
        println!(
            "built {} shapes at {size}px ({} bytes) in {:?}",
            Shape::COUNT,
            bytes,
            elapsed
        );
    }
}
