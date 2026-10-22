//! Tests for the drawn cursor set.
//!
//! All of this is pure arithmetic over a byte buffer, so none of it needs a
//! renderer, a client or a [`State`](crate::compositor::State) -- the tests
//! read the pixels [`generate`] produced directly. What they guard is what
//! is easy to get silently wrong in a rasterizer and invisible to "it
//! compiles": a shape drawing *outside* its own bitmap (clipped away, so it
//! never crashes -- it just quietly loses an edge), a shape drawing
//! *nothing* at a size a config is allowed to ask for, broken
//! premultiplication (which over-brightens under pixman's `Over`), a client
//! asking for one cursor and getting another, and a hotspot that is not
//! where the user points.

use super::*;
use crate::compositor::decorations::Appearance;

/// Mocu's own white paints (its `Mocu-White` theme): fill and stroke, in
/// premultiplied little-endian `Argb8888`.
const FILL: [u8; 4] = [0xfc, 0xfc, 0xfc, 255];
const STROKE: [u8; 4] = [0x22, 0x22, 0x22, 255];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

fn paint(shape: Shape, size: i32) -> Vec<u8> {
    generate(shape, size, FILL, STROKE)
}

/// The default cursor size and the two ends of the range a config may ask
/// for, plus the physical sizes a fractional or 2x output rasterises.
fn sizes() -> [i32; 10] {
    [
        Appearance::MIN_CURSOR_SIZE,
        8,
        12,
        Appearance::default().cursor_size,
        20,
        24,
        30,
        32,
        48,
        Appearance::MAX_CURSOR_SIZE,
    ]
}

fn pixel(pixels: &[u8], size: i32, x: i32, y: i32) -> [u8; 4] {
    let idx = ((y * size + x) * 4) as usize;
    pixels[idx..idx + 4].try_into().expect("a whole pixel")
}

fn alpha(pixels: &[u8], size: i32, x: i32, y: i32) -> u8 {
    pixel(pixels, size, x, y)[3]
}

// -------------------------------------------------------------------------
// The mapping
// -------------------------------------------------------------------------

#[test]
fn every_shape_has_its_own_slot_and_its_own_file() {
    assert_eq!(Shape::ALL.len(), Shape::COUNT);
    assert_eq!(mocu::FILES.len(), Shape::COUNT);
    for (slot, shape) in Shape::ALL.into_iter().enumerate() {
        assert_eq!(shape.index(), slot, "{shape:?} indexes the wrong slot");
        assert_eq!(shape.name(), mocu::FILES[slot].0);
    }
    let mut names: Vec<&str> = Shape::ALL.iter().map(|s| s.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Shape::COUNT, "two shapes share a file");
}

#[test]
fn every_named_icon_draws_mocus_cursor_of_that_name() {
    // Every CSS cursor name the protocol carries, and the Mocu cursor it
    // draws: the same name, except the two Mocu aliases (`move` is Mocu's
    // `grabbing`; `all-resize`, which Mocu does not name, its `all-scroll`).
    let cases = [
        (CursorIcon::Default, "default"),
        (CursorIcon::ContextMenu, "context-menu"),
        (CursorIcon::Help, "help"),
        (CursorIcon::Pointer, "pointer"),
        (CursorIcon::Progress, "progress"),
        (CursorIcon::Wait, "wait"),
        (CursorIcon::Cell, "cell"),
        (CursorIcon::Crosshair, "crosshair"),
        (CursorIcon::Text, "text"),
        (CursorIcon::VerticalText, "vertical-text"),
        (CursorIcon::Alias, "alias"),
        (CursorIcon::Copy, "copy"),
        (CursorIcon::Move, "grabbing"),
        (CursorIcon::NoDrop, "no-drop"),
        (CursorIcon::NotAllowed, "not-allowed"),
        (CursorIcon::Grab, "grab"),
        (CursorIcon::Grabbing, "grabbing"),
        (CursorIcon::EResize, "e-resize"),
        (CursorIcon::NResize, "n-resize"),
        (CursorIcon::NeResize, "ne-resize"),
        (CursorIcon::NwResize, "nw-resize"),
        (CursorIcon::SResize, "s-resize"),
        (CursorIcon::SeResize, "se-resize"),
        (CursorIcon::SwResize, "sw-resize"),
        (CursorIcon::WResize, "w-resize"),
        (CursorIcon::EwResize, "ew-resize"),
        (CursorIcon::NsResize, "ns-resize"),
        (CursorIcon::NeswResize, "nesw-resize"),
        (CursorIcon::NwseResize, "nwse-resize"),
        (CursorIcon::ColResize, "col-resize"),
        (CursorIcon::RowResize, "row-resize"),
        (CursorIcon::AllScroll, "all-scroll"),
        (CursorIcon::ZoomIn, "zoom-in"),
        (CursorIcon::ZoomOut, "zoom-out"),
        (CursorIcon::DndAsk, "dnd-ask"),
        (CursorIcon::AllResize, "all-scroll"),
    ];
    for (icon, name) in cases {
        assert_eq!(Shape::for_icon(icon).name(), name, "for {icon:?}");
    }
    // Every shape is some icon's: nothing vendored goes unused.
    for shape in Shape::ALL {
        assert!(
            cases.iter().any(|(_, n)| *n == shape.name()),
            "{shape:?} is drawn for no icon"
        );
    }
}

// -------------------------------------------------------------------------
// Hotspots
// -------------------------------------------------------------------------

#[test]
fn every_hotspot_lies_inside_its_bitmap() {
    for size in sizes() {
        for shape in Shape::ALL {
            let hot = shape.hotspot(size);
            assert!(
                hot.x >= 0 && hot.y >= 0 && hot.x < size && hot.y < size,
                "{shape:?} at {size}px hotspots outside its bitmap at {hot:?}"
            );
        }
    }
}

#[test]
fn the_arrows_point_with_their_tip() {
    // Every arrow cursor's hotspot is the arrow's tip pixel: the tip is
    // drawn there, the arrow's body just below it is fill, and nothing
    // above or left of it is fill (only stroke and shadow sit there).
    let arrows = [
        Shape::Default,
        Shape::ContextMenu,
        Shape::Help,
        Shape::Progress,
        Shape::Alias,
        Shape::Copy,
        Shape::NoDrop,
    ];
    for size in [16, 20, 24, 32, 48, 256] {
        let tip = Shape::Default.hotspot(size);
        for shape in arrows {
            assert_eq!(shape.hotspot(size), tip, "{shape:?} at {size}px");
            let pixels = paint(shape, size);
            assert_ne!(
                alpha(&pixels, size, tip.x, tip.y),
                0,
                "{shape:?} at {size}px"
            );
            let deep = size / 8;
            assert_eq!(
                pixel(&pixels, size, tip.x + 1, tip.y + 2 + deep),
                FILL,
                "{shape:?} at {size}px: no arrow body below the tip"
            );
            for (x, y) in [(tip.x - 1, tip.y), (tip.x, tip.y - 1)] {
                assert_ne!(
                    pixel(&pixels, size, x, y),
                    FILL,
                    "{shape:?} at {size}px draws fill beside its tip at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn tiny_sizes_draw_a_centred_dot_under_the_pointer() {
    // Below 8px no shape is legible, so every one draws the honest dot at
    // the middle -- which is also the hotspot there.
    for size in [Appearance::MIN_CURSOR_SIZE, 5, 6, 7] {
        for shape in Shape::ALL {
            assert_eq!(shape.hotspot(size), (size / 2, size / 2).into());
            let pixels = paint(shape, size);
            assert_ne!(
                alpha(&pixels, size, size / 2, size / 2),
                0,
                "{shape:?} at {size}px"
            );
        }
    }
}

// -------------------------------------------------------------------------
// The pixels
// -------------------------------------------------------------------------

#[test]
fn every_shape_draws_into_exactly_its_buffer_at_every_size() {
    // The right byte count, and some coverage: a shape that draws nothing
    // at a size a config can ask for leaves the user pointerless.
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            assert_eq!(
                pixels.len(),
                (size * size * 4) as usize,
                "{shape:?} at {size}px"
            );
            assert!(
                pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
                "{shape:?} at {size}px drew nothing"
            );
        }
    }
}

#[test]
fn no_stroke_is_ever_clipped_by_the_bitmap_edge() {
    // The stroke is what separates the cursor from whatever is under it, so
    // it must be whole on every side: every fully covered fill pixel sits a
    // full stroke width inside the bitmap, and a stroke of that width (the
    // widest any layer draws at that size) fits between it and the edge.
    for size in sizes().into_iter().filter(|s| *s >= 8) {
        let rim = place::rim(2.0, size) as i32;
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            for y in 0..size {
                for x in 0..size {
                    if pixel(&pixels, size, x, y) != FILL {
                        continue;
                    }
                    assert!(
                        x >= rim && y >= rim && x < size - rim && y < size - rim,
                        "{shape:?} at {size}px has fill at ({x}, {y}), inside its stroke width of the edge"
                    );
                }
            }
        }
    }
}

#[test]
fn every_fill_pixel_is_wrapped_in_drawn_pixels() {
    // No fill pixel sits directly against a transparent one, in any of the
    // eight directions: the stroke is always between the fill and the
    // background.
    for size in [8, 12, 16, 20, 24, 30, 32, 48] {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            for y in 0..size {
                for x in 0..size {
                    if pixel(&pixels, size, x, y) != FILL {
                        continue;
                    }
                    for (ox, oy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        assert_ne!(
                            alpha(&pixels, size, x + ox, y + oy),
                            0,
                            "{shape:?} at {size}px: fill at ({x}, {y}) touches transparency"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_pixel_is_premultiplied_correctly() {
    // The bitmaps go straight to the scanout buffer, where pixman composites
    // them with `Over` -- defined over premultiplied components. A channel
    // byte greater than the alpha byte would over-brighten.
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = paint(shape, size);
            for p in pixels.as_chunks::<4>().0.iter() {
                assert!(
                    p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3],
                    "{shape:?} at {size}px drew {p:?}, not premultiplied"
                );
            }
        }
    }
}

#[test]
fn a_shape_uses_the_paints_it_was_given() {
    // Fully covered pixels are exactly the paints asked for, with no channel
    // reordering (four distinct bytes each, so a swap would show); every
    // other pixel is a blend no brighter than the brightest paint (give or
    // take a byte of rounding where layers composite).
    let size = 48;
    let fill = [0x11, 0x22, 0x33, 0xff];
    let stroke = [0x44, 0x55, 0x66, 0xff];
    for shape in Shape::ALL {
        let pixels = generate(shape, size, fill, stroke);
        let (mut f, mut s) = (0, 0);
        for p in pixels.as_chunks::<4>().0.iter() {
            let p: [u8; 4] = *p;
            f += usize::from(p == fill);
            s += usize::from(p == stroke);
            assert!(
                p[0] <= 0x45 && p[1] <= 0x56 && p[2] <= 0x67,
                "{shape:?} drew {p:?}"
            );
        }
        assert!(
            f > 0 && s > 0,
            "{shape:?} at {size}px: {f} fill, {s} stroke pixels"
        );
    }
}

#[test]
fn a_translucent_fill_makes_the_whole_cursor_translucent() {
    // The shadow follows the fill's alpha, so a translucent cursor has no
    // opaque pixel anywhere (where layers overlap they composite, so their
    // alphas add, but never to opaque).
    let fill = [0x7e, 0x7e, 0x7e, 0x80];
    let stroke = [0x11, 0x11, 0x11, 0x80];
    for shape in Shape::ALL {
        let pixels = generate(shape, 24, fill, stroke);
        assert!(
            pixels.as_chunks::<4>().0.iter().all(|p| p[3] < 0xf0),
            "{shape:?} drew an opaque pixel from translucent paints"
        );
    }
}

#[test]
fn every_shape_is_distinct_and_anti_aliased() {
    // A client asking for `text` must not get the same pixels as one asking
    // for `crosshair`, and every shape has blended edge pixels (a 1-bit
    // mask would have none).
    let size = Appearance::default().cursor_size;
    let rendered: Vec<(Shape, Vec<u8>)> = Shape::ALL
        .into_iter()
        .map(|shape| (shape, paint(shape, size)))
        .collect();
    for (i, (shape, pixels)) in rendered.iter().enumerate() {
        for (other, other_pixels) in &rendered[i + 1..] {
            assert_ne!(
                pixels, other_pixels,
                "{shape:?} and {other:?} draw the same"
            );
        }
        let blended = pixels.as_chunks::<4>().0.iter().any(|p| {
            let p: [u8; 4] = *p;
            p != FILL && p != STROKE && p != TRANSPARENT
        });
        assert!(blended, "{shape:?} has no blended pixels");
    }
}

/// The physical bitmaps are natively rasterised, not resampled: a 32px
/// render differs from a bilinear upscale of the 16px render of the same
/// shape.
#[test]
fn hidpi_buffers_are_natively_rasterised_not_upscaled() {
    for shape in [
        Shape::Default,
        Shape::Text,
        Shape::Pointer,
        Shape::AllScroll,
    ] {
        let small = paint(shape, 16);
        let native = paint(shape, 32);
        let upscaled = upscale_bilinear(&small, 16, 32);
        assert_ne!(native, upscaled, "{shape:?}: the 32px bitmap is an upscale");
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
            let gx = (f64::from(x) + 0.5) * f64::from(from) / f64::from(to) - 0.5;
            let gy = (f64::from(y) + 0.5) * f64::from(from) / f64::from(to) - 0.5;
            let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
            let (fx, fy) = (gx - f64::from(x0), gy - f64::from(y0));
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
