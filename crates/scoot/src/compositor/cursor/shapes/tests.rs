//! Tests for the procedural cursor shapes.
//!
//! All of this is pure arithmetic over a byte buffer, so none of it needs a
//! renderer, a client or a [`State`](crate::compositor::State) -- the tests
//! read the pixels [`generate`] produced directly. What they are actually
//! guarding is the things that are easy to get silently wrong in a
//! rasterizer and invisible to "it compiles": a shape drawing *outside* its
//! own bitmap (clipped away, so it never crashes -- it just quietly loses
//! an arrowhead), a shape drawing *nothing* at a size a config is allowed to
//! ask for, broken premultiplication (which over-brightens under pixman's
//! `Over`), and missing anti-aliasing (which would mean the supersampler
//! regressed to a 1-bit mask).

use super::*;
use crate::compositor::decorations::Appearance;

/// An opaque white fill and the black outline `Cursor::new` pairs with it,
/// in the premultiplied little-endian `Argb8888` the bitmaps are built in.
const FILL: [u8; 4] = [255, 255, 255, 255];
const OUTLINE: [u8; 4] = [0, 0, 0, 255];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// The default cursor size, and the two ends of the range a config may ask
/// for. Every shape is checked at all three rather than at the default alone:
/// the smallest is where a shape's fractions collapse toward a dot and the
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
fn the_arrow_points_with_its_tip_and_the_rest_are_centred() {
    let size = 16;
    let (tx, ty) = arrow_tip(size);
    // The tip sits inside the bitmap (room for outline and shadow), never at
    // the corner itself -- and the hotspot is exactly that tip.
    assert!(tx > 0 && ty > 0, "the tip needs its outline inset");
    assert_eq!(
        Shape::Arrow.hotspot(size),
        (tx, ty).into(),
        "the arrow's hotspot is its tip, not the bitmap corner"
    );
    for shape in Shape::ALL.into_iter().filter(|s| *s != Shape::Arrow) {
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
fn every_shape_actually_draws_something_at_every_allowed_size() {
    // Any coverage at all (flat or blended): a shape that draws nothing at
    // a size a config can ask for leaves the user pointerless. Exact flat
    // colors are asserted separately at usable sizes -- at 4px nothing is
    // fully covered, everything is a blend by construction.
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = generate(shape, size, FILL, OUTLINE);
            let drawn = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| alpha(&pixels, size, *x, *y) > 0)
                .count();
            assert!(drawn > 0, "{shape:?} at {size}px drew nothing at all");
        }
    }
    // ... while at real sizes the fills land as flat pixels (which is what
    // keeps small exact-pixel assertions elsewhere meaningful) and every
    // shape carries a halo -- flat outline or blends -- around its fill, so
    // it stays legible against content of a similar color. Thin strokes have
    // no *flat* outline pixel by construction (a 1px band around a 1.6px
    // stroke is all partial coverage), which is why the halo counts blends.
    for size in [
        Appearance::default().cursor_size,
        Appearance::MAX_CURSOR_SIZE,
    ] {
        for shape in Shape::ALL {
            let pixels = generate(shape, size, FILL, OUTLINE);
            assert!(
                count(&pixels, size, FILL) > 0,
                "{shape:?} at {size}px drew no flat fill at all"
            );
            let halo = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let p = pixel(&pixels, size, *x, *y);
                    p != FILL && p != TRANSPARENT
                })
                .count();
            assert!(
                halo > 0,
                "{shape:?} at {size}px has no outline halo, so it would be \
                 invisible against content of a similar color"
            );
        }
    }
}

#[test]
fn no_shape_draws_fill_on_the_edge_of_its_own_bitmap() {
    // A fill pixel on the bitmap's edge has no room for its outline on that
    // side, so it would meet whatever is under the cursor with no outline
    // between them -- the one thing the outline exists to prevent. Checked
    // at every allowed size because the insets are a fraction of it, and the
    // smallest size is where the fractions collapse.
    for size in sizes() {
        for shape in Shape::ALL {
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
fn every_fill_pixel_is_wrapped_in_non_transparent_pixels() {
    // The property the outline exists for: no fill pixel may sit directly
    // against a transparent one, in any of the eight directions, or the
    // shape has an unoutlined edge there. A neighbour *outside* the bitmap
    // counts as a failure for the same reason -- there is no pixel there to
    // hold the outline, so the shape's edge is bare. Neighbours are checked
    // for *any* coverage (alpha > 0) rather than exact outline bytes,
    // because anti-aliased edge pixels are blends, not flat colors.
    // Asserted over whole bitmaps rather than spot checks, since it is cheap
    // and it is the only thing keeping a thin stroke legible.
    for size in sizes() {
        for shape in Shape::ALL {
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
    // neither flat fill, flat outline nor transparent. A 1-bit mask would
    // draw the same silhouettes with none. Checked at the default size and
    // up, where every shape has room for real edges.
    for size in [Appearance::default().cursor_size, 48] {
        for shape in Shape::ALL {
            let pixels = generate(shape, size, FILL, OUTLINE);
            let blended = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let p = pixel(&pixels, size, *x, *y);
                    p != FILL && p != OUTLINE && p != TRANSPARENT
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
    // anti-aliased blends (which is where a straight-alpha mistake shows).
    for size in sizes() {
        for shape in Shape::ALL {
            let pixels = generate(shape, size, FILL, OUTLINE);
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
    for shape in Shape::ALL {
        let pixels = generate(shape, size, fill, outline);
        let mut exact = 0;
        for y in 0..size {
            for x in 0..size {
                let p = pixel(&pixels, size, x, y);
                if p == fill || p == outline || p == TRANSPARENT {
                    exact += 1;
                    continue;
                }
                // A blend of the two given colors with transparency: each
                // channel between the color's and zero, alpha in between.
                // (Upper bounds only: 0x44/0x55/0x66 already exceed
                // 0x11/0x22/0x33 on every channel, so they are the maxima.)
                let [b, g, r, a] = p;
                assert!(
                    a > 0 && a < 255,
                    "{shape:?} drew fully transparent/opaque {p:?} at ({x}, {y}) \
                     that matches no input"
                );
                assert!(
                    b <= 0x44 && g <= 0x55 && r <= 0x66,
                    "{shape:?} drew {p:?} at ({x}, {y}), outside either given color"
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
    // mirror symmetry is not available to ask for -- a stroke on an
    // even-sized grid cannot straddle the half-pixel the mirror axis falls
    // on, and the arrow's shadow is deliberately offset -- so what is
    // asserted is the property that actually shows: the centroid of everything
    // drawn sits within one pixel of the bitmap's middle. A rasterizer that
    // biases one end of a stroke moves that centroid and shows up here and
    // nowhere else. The arrow is exempt: its body is intentionally
    // asymmetric (and its shadow offset), its balance point is its tip.
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
fn the_arrow_tip_is_drawn_and_the_corner_is_not() {
    // The hotspot contract from the bitmap's side: the tip pixel itself is
    // covered (it is what the user is pointing with), while the bitmap's
    // own corner stays transparent (the tip's outline inset). The corner
    // check needs room for the inset, so it starts above the 4px minimum --
    // at 4px the whole bitmap is a dot with a halo (see
    // `bitmap_honors_the_requested_size`), and the fill-never-on-edge tests
    // pin down what that dot guarantees instead.
    for size in sizes() {
        let pixels = generate(Shape::Arrow, size, FILL, OUTLINE);
        let (tx, ty) = arrow_tip(size);
        assert_ne!(
            alpha(&pixels, size, tx, ty),
            0,
            "the arrow tip at ({tx}, {ty}) at {size}px is transparent"
        );
        if size > Appearance::MIN_CURSOR_SIZE {
            assert_eq!(
                pixel(&pixels, size, 0, 0),
                TRANSPARENT,
                "the bitmap corner at {size}px is drawn -- the tip inset is gone"
            );
        }
    }
}

/// Not an assertion: prints every shape as ASCII art so a human can look at
/// what the rasterizer actually produced. `#` is flat fill, `+` a blended
/// pixel, `.` flat outline, space transparent. Run with
/// `cargo test -- --ignored --nocapture shape_art`.
#[test]
#[ignore = "prints the shapes for a human to look at; asserts nothing"]
fn shape_art() {
    let size = 24;
    for shape in Shape::ALL {
        let pixels = generate(shape, size, FILL, OUTLINE);
        println!("--- {shape:?} ---");
        for y in 0..size {
            let row: String = (0..size)
                .map(|x| {
                    let p = pixel(&pixels, size, x, y);
                    if p == FILL {
                        '#'
                    } else if p == OUTLINE {
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
    dump_pngs(|shape, size| generate(shape, size, FILL, OUTLINE));
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

/// Not an assertion: times building all ten bitmaps at the default and
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
            bytes += generate(shape, size, FILL, OUTLINE).len();
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
