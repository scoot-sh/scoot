//! A tiny supersampled rasterizer for the procedural cursor shapes.
//!
//! Every pixel in this module's output is generated from this repository's
//! own vector geometry (see `shapes.rs`): no theme's pixel data is copied,
//! embedded or vendored anywhere, which is what keeps the fallback cursor
//! MIT-clean (see `CLAUDE.md`'s license note and `cursor.rs`'s module doc).
//!
//! # How it works
//!
//! The caller describes a shape as a signed-distance field ([`Field`]:
//! negative inside the fill, positive outside, in pixels) built from the
//! primitives below ([`sd_segment`], [`sd_circle`], [`sd_poly`], unioned
//! with [`union`]). [`render`] then evaluates that field at `SS x SS`
//! subpixel taps per output pixel, once, at startup or on config reload --
//! never per frame -- and composites fill, outline and an optional soft
//! shadow into premultiplied `Argb8888` bytes.
//!
//! Supersampling (rather than analytic coverage) is what keeps the two
//! render tiers identical: the bitmap is plain `Argb8888` pixels by the
//! time any renderer sees it, so pixman and a future GPU tier composite
//! byte-identical input. A 4x4 grid (16 taps, fewer past 64px -- see
//! [`supersample`]) is the whole anti-aliasing budget: cheap enough that
//! all ten shapes at the default size still build in milliseconds (see the
//! benchmark in the cursor report), smooth enough that a 1px outline
//! survives a bilinear upscale to a scale-2 output.
//!
//! # Coordinate convention
//!
//! Subpixel taps are at `(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS)` for the
//! output pixel `(x, y)` -- i.e. pixel centers sit on the half grid, the
//! same convention pixman samples with. A shape edge exactly on an integer
//! coordinate therefore covers half its edge pixels, which is what makes a
//! 1px stroke land evenly rather than rounding to one side.

/// Subpixel taps per pixel axis at display sizes (see [`supersample`]).
pub const SS: i32 = 4;

/// Subpixel taps per pixel axis for `size`: 4x4 (16 taps) at the sizes a
/// cursor is actually drawn at, 2x2 above 64px, where every feature is tens
/// of pixels wide and 4 taps still resolve the edge. The build runs once per
/// startup or reload -- never per frame -- but a 256px bitmap at 16 taps is
/// ten million field evaluations, seconds in a debug build; this keeps the
/// largest allowed size honest without touching small-size quality.
fn supersample(size: i32) -> i32 {
    if size > 64 { 2 } else { SS }
}

/// A soft drop shadow: the same field as the fill, sampled with an offset,
/// feathered over `feather` pixels, drawn as black at `alpha`.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    /// Offset of the shadow behind the fill, in pixels.
    pub dx: f64,
    /// Offset of the shadow behind the fill, in pixels.
    pub dy: f64,
    /// How far past the offset silhouette the shadow fades, in pixels.
    pub feather: f64,
    /// Peak alpha of the shadow (premultiplied black), 0.0..=1.0.
    pub alpha: f64,
}

/// Unsigned distance from `(px, py)` to the segment `a -> b`, in pixels.
pub fn sd_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let abx = bx - ax;
    let aby = by - ay;
    let len_sq = abx * abx + aby * aby;
    // A degenerate segment is a point; dividing by its zero length would
    // produce NaN and silently erase the stroke.
    if len_sq == 0.0 {
        return ((px - ax) * (px - ax) + (py - ay) * (py - ay)).sqrt();
    }
    let t = (((px - ax) * abx + (py - ay) * aby) / len_sq).clamp(0.0, 1.0);
    let cx = ax + abx * t;
    let cy = ay + aby * t;
    ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt()
}

/// Signed distance from `(px, py)` to the circle around `(cx, cy)` with
/// radius `r`: negative inside, in pixels.
pub fn sd_circle(px: f64, py: f64, cx: f64, cy: f64, r: f64) -> f64 {
    ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt() - r
}

/// Signed distance from `(px, py)` to the polygon `verts`: negative inside,
/// in pixels. Either winding works; a degenerate (fewer than 3 vertices)
/// polygon is "everything is outside".
pub fn sd_poly(px: f64, py: f64, verts: &[(f64, f64)]) -> f64 {
    if verts.len() < 3 {
        return f64::INFINITY;
    }
    // Outside distance: the closest edge.
    let mut outside = f64::INFINITY;
    for i in 0..verts.len() {
        let (ax, ay) = verts[i];
        let (bx, by) = verts[(i + 1) % verts.len()];
        outside = outside.min(sd_segment(px, py, ax, ay, bx, by));
    }
    // Inside test: even-odd rule on a ray to +x.
    let mut inside = false;
    for i in 0..verts.len() {
        let (ax, ay) = verts[i];
        let (bx, by) = verts[(i + 1) % verts.len()];
        if (ay > py) != (by > py) && px < (bx - ax) * (py - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    if inside { -outside } else { outside }
}

/// Union of two fields: inside either shape reads as inside.
pub fn union(a: f64, b: f64) -> f64 {
    a.min(b)
}

/// Straight-alpha `rgb` plus alpha, all 0.0..=1.0, decoded from one
/// premultiplied little-endian `Argb8888` `[B, G, R, A]` pixel.
fn straight_from_premul(pixel: [u8; 4]) -> (f64, f64, f64, f64) {
    let a = f64::from(pixel[3]) / 255.0;
    if a <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (
        (f64::from(pixel[2]) / 255.0) / a,
        (f64::from(pixel[1]) / 255.0) / a,
        (f64::from(pixel[0]) / 255.0) / a,
        a,
    )
}

/// Renders `field` (signed distance to the fill edge, negative inside, in
/// pixels) into a fresh `size * size * 4` premultiplied `Argb8888` buffer.
///
/// `fill`/`outline` are single premultiplied `[B, G, R, A]` pixels, the same
/// layout `cursor.rs` has always used; the outline is composited verbatim in
/// the caller's color (which `Cursor` always builds as black at the fill's
/// own alpha -- see `outline_pair` -- the color policy lives there, not
/// here). `outline_width` is how far past the fill edge the outline
/// reaches, in pixels. `shadow` (usually `None` except for the arrow -- a
/// soft shadow on 1px line art reads as blur, not depth) is composited
/// underneath both, always black.
///
/// `bounds` is the field's tight bounding box `(min_x, min_y, max_x, max_y)`
/// already expanded by the caller past every layer that can paint
/// (outline, shadow, one pixel of bleed): taps outside it are transparent
/// without evaluating the field, which is what keeps large bitmaps cheap.
/// `None` evaluates everywhere (the unit tests' choice).
///
/// Layer order, back to front: transparent, shadow, outline, fill -- each
/// `Over` the last in premultiplied space, so a translucent fill genuinely
/// composites instead of replacing.
pub fn render(
    size: i32,
    field: &dyn Fn(f64, f64) -> f64,
    fill: [u8; 4],
    outline: [u8; 4],
    outline_width: f64,
    shadow: Option<Shadow>,
    bounds: Option<[f64; 4]>,
) -> Vec<u8> {
    let (fr, fg, fb, fa) = straight_from_premul(fill);
    let (orr, ogg, obb, oa) = straight_from_premul(outline);
    let ss = supersample(size);
    let taps = f64::from(ss * ss);
    let step = 1.0 / f64::from(ss);
    let [bx0, by0, bx1, by1] =
        bounds.unwrap_or([f64::from(0), f64::from(0), f64::from(size), f64::from(size)]);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut fill_cov = 0.0;
            let mut outline_cov = 0.0;
            let mut shadow_cov = 0.0;
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = f64::from(x) + (f64::from(sx) + 0.5) * step;
                    let py = f64::from(y) + (f64::from(sy) + 0.5) * step;
                    if px < bx0 || px > bx1 || py < by0 || py > by1 {
                        continue;
                    }
                    let d = field(px, py);
                    if d <= 0.0 {
                        fill_cov += 1.0;
                    } else if d <= outline_width {
                        outline_cov += 1.0;
                    } else if let Some(sh) = shadow {
                        // Only past the outline: a shadow under the fill or
                        // the outline is invisible when they are opaque, and
                        // wrong when they are translucent (its alpha would
                        // add to theirs, so a 50% fill would no longer blend
                        // halfway with the background).
                        let ds = field(px - sh.dx, py - sh.dy);
                        if ds <= 0.0 {
                            shadow_cov += 1.0;
                        } else if ds < sh.feather {
                            shadow_cov += 1.0 - ds / sh.feather;
                        }
                    }
                }
            }
            fill_cov /= taps;
            outline_cov /= taps;
            shadow_cov /= taps;
            // Back to front, premultiplied `Over`: out = src + dst * (1 - a).
            let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
            if let Some(sh) = shadow {
                let sa = (shadow_cov * sh.alpha).clamp(0.0, 1.0);
                a = sa;
                // Black: rgb stays 0.
            }
            // The outline in the caller's own color, scaled by its coverage
            // -- at full coverage this is byte-identical to the flat outline
            // the old bitmaps wrote. (`Cursor` always passes black at the
            // fill's alpha; that policy lives in `outline_pair`, not here.)
            let ola = (outline_cov * oa).clamp(0.0, 1.0);
            r = orr * ola + r * (1.0 - ola);
            g = ogg * ola + g * (1.0 - ola);
            b = obb * ola + b * (1.0 - ola);
            a = ola + a * (1.0 - ola);
            let fla = (fill_cov * fa).clamp(0.0, 1.0);
            r = fr * fla + r * (1.0 - fla);
            g = fg * fla + g * (1.0 - fla);
            b = fb * fla + b * (1.0 - fla);
            a = fla + a * (1.0 - fla);
            let idx = ((y * size + x) * 4) as usize;
            pixels[idx] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
            pixels[idx + 1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
            pixels[idx + 2] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
            pixels[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_segment_measures_distance_to_its_point() {
        assert_eq!(sd_segment(3.0, 4.0, 0.0, 0.0, 0.0, 0.0), 5.0);
    }

    #[test]
    fn a_circle_is_negative_inside_and_positive_outside() {
        assert!(sd_circle(0.0, 0.0, 0.0, 0.0, 5.0) < 0.0);
        assert_eq!(sd_circle(5.0, 0.0, 0.0, 0.0, 5.0), 0.0);
        assert!(sd_circle(6.0, 0.0, 0.0, 0.0, 5.0) > 0.0);
    }

    #[test]
    fn a_square_is_negative_inside_regardless_of_winding() {
        let ccw = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let cw = [(0.0, 0.0), (0.0, 4.0), (4.0, 4.0), (4.0, 0.0)];
        assert!(sd_poly(2.0, 2.0, &ccw) < 0.0);
        assert!(sd_poly(2.0, 2.0, &cw) < 0.0);
        assert!(sd_poly(6.0, 2.0, &ccw) > 0.0);
        assert!(sd_poly(6.0, 2.0, &cw) > 0.0);
    }

    #[test]
    fn a_degenerate_polygon_contains_nothing() {
        assert!(sd_poly(0.0, 0.0, &[]).is_infinite());
        assert!(sd_poly(0.0, 0.0, &[(0.0, 0.0)]).is_infinite());
    }

    #[test]
    fn a_fully_covered_field_writes_the_fill_verbatim() {
        let pixels = render(
            4,
            &|_, _| -1.0,
            [10, 20, 30, 255],
            [0, 0, 0, 255],
            1.0,
            None,
            None,
        );
        assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn an_empty_field_stays_transparent() {
        let pixels = render(
            4,
            &|_, _| f64::INFINITY,
            [10, 20, 30, 255],
            [0, 0, 0, 255],
            1.0,
            None,
            None,
        );
        assert!(pixels.iter().all(|b| *b == 0));
    }
}
