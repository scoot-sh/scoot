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
//! primitives below ([`sd_segment`], [`sd_circle`], [`sd_path`], unioned
//! with [`union`]). [`render`] then evaluates that field at `SS x SS`
//! subpixel taps per output pixel, once, at startup or on config reload --
//! never per frame -- and composites fill, outline and the optional accent
//! mark into premultiplied `Argb8888` bytes.
//!
//! Supersampling (rather than analytic coverage) is what keeps the two
//! render tiers identical: the bitmap is plain `Argb8888` pixels by the
//! time any renderer sees it, so pixman and a future GPU tier composite
//! byte-identical input. A 4x4 grid (16 taps, fewer past 64px -- see
//! [`supersample`]) is the whole anti-aliasing budget: cheap enough that
//! all twenty-two shapes at the default size still build in milliseconds (see the
//! benchmark in the cursor report), smooth enough that a 1px rim
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

/// Signed distance from `(px, py)` to a compound polygon made of several
/// closed `subpaths`: negative inside, in pixels. Inside is decided by the
/// even-odd rule across *all* subpaths together, which is how a ring (an
/// outer circle with an inner one) or a lens with a handle gets its hole --
/// the same result SVG's nonzero rule gives for the vendored cursor art,
/// whose holes always wind against their outlines. Subpaths with fewer than
/// three vertices contribute nothing.
pub fn sd_path(px: f64, py: f64, subpaths: &[Vec<(f64, f64)>]) -> f64 {
    let mut outside = f64::INFINITY;
    let mut inside = false;
    for verts in subpaths.iter().filter(|v| v.len() >= 3) {
        for i in 0..verts.len() {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % verts.len()];
            outside = outside.min(sd_segment(px, py, ax, ay, bx, by));
            if (ay > py) != (by > py) && px < (bx - ax) * (py - ay) / (by - ay) + ax {
                inside = !inside;
            }
        }
    }
    if inside { -outside } else { outside }
}

/// Composites the premultiplied `Argb8888` buffer `src` over `dst` in place
/// (`out = src + dst * (1 - src_alpha)`, per channel): how a shape's second
/// coat -- a badge with its own rim -- lands over the first. Both buffers are
/// the same size; a shorter `src` composites only what it covers.
pub fn over(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let keep = 255 - u32::from(s[3]);
        for c in 0..4 {
            // Rounded `d * keep / 255`, exact for 8-bit premultiplied data.
            let scaled = (u32::from(d[c]) * keep + 127) / 255;
            d[c] = (u32::from(s[c]) + scaled).min(255) as u8;
        }
    }
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

/// One flat-color coat of paint over the bitmap: a fill plus the rim around
/// it. The body always wears one; the accent wears one only when the config
/// enables it.
#[derive(Clone, Copy)]
pub struct Coat {
    /// Flat color, premultiplied little-endian `Argb8888`.
    pub fill: [u8; 4],
    /// Rim color, in the caller's color (the body's auto-contrasting rim --
    /// see `shapes::generate`).
    pub outline: [u8; 4],
    /// How far past the fill edge the rim reaches, in pixels.
    pub outline_width: f64,
}

/// The accent coat: the family mark's own signed-distance field plus the
/// paint for it. `None` (accent disabled, or a shape with no mark) skips the
/// field entirely, so an accent-free build costs what the old one did.
pub struct Accent<'a> {
    /// Signed distance to the accent mark's edge, negative inside, in pixels
    /// -- the same convention as the body field [`render`] takes.
    pub field: &'a dyn Fn(f64, f64) -> f64,
    /// The mark's paint.
    pub coat: Coat,
}

/// Renders `field` (signed distance to the fill edge, negative inside, in
/// pixels) into a fresh `size * size * 4` premultiplied `Argb8888` buffer.
///
/// `body` is the body's paint (fill plus rim, both single premultiplied
/// `[B, G, R, A]` pixels in the same layout `cursor.rs` has always used; the
/// rim is composited verbatim in the caller's color, which `Cursor` always
/// builds as the auto-contrasting rim at the fill's own alpha -- see
/// `contrast_rim` -- the color policy lives there, not here). `accent`
/// (usually `Some` -- the family's ginger mark, optionally ringed with a
/// hairline of the rim color) is composited over the body; `None` skips its
/// field entirely.
///
/// `bounds` is the union of both fields' tight bounding boxes, already
/// expanded by the caller past every layer that can paint (rims, one pixel
/// of bleed): taps outside it are transparent without evaluating either
/// field. `None` evaluates everywhere (the unit tests' choice).
///
/// # Skipping taps that cannot differ
///
/// Every field here is 1-Lipschitz (a true distance, or a min/offset of
/// true distances), so one evaluation at a pixel's centre bounds the field at
/// all of its taps: no tap sits further than `reach` (the tap grid's half
/// diagonal) from the centre. When that bound puts every tap in the same
/// band -- all fill, all rim, or all clear -- the pixel's coverage is known
/// without the other taps, and the result is bit-identical to evaluating
/// them. Only pixels an edge actually crosses pay for the full grid, which is
/// what keeps the many-edged outline paths cheap at large sizes.
///
/// Layer order, back to front: transparent, body rim, body fill, accent rim,
/// accent fill -- each `Over` the last in premultiplied space, so a
/// translucent fill genuinely composites instead of replacing.
pub fn render(
    size: i32,
    field: &dyn Fn(f64, f64) -> f64,
    body: Coat,
    accent: Option<Accent<'_>>,
    bounds: Option<[f64; 4]>,
) -> Vec<u8> {
    let (fr, fg, fb, fa) = straight_from_premul(body.fill);
    let (orr, ogg, obb, oa) = straight_from_premul(body.outline);
    let outline_width = body.outline_width;
    let (ar, ag, ab, aa, aorr, aogg, aobb, aoa, aow) = match &accent {
        Some(mark) => {
            let (r, g, b, alpha) = straight_from_premul(mark.coat.fill);
            let (or, og, ob, oa) = straight_from_premul(mark.coat.outline);
            (r, g, b, alpha, or, og, ob, oa, mark.coat.outline_width)
        }
        None => (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    };
    let ss = supersample(size);
    let taps = f64::from(ss * ss);
    let step = 1.0 / f64::from(ss);
    // Half the diagonal of the tap grid: the furthest any tap sits from its
    // pixel's centre.
    let reach = (0.5 - step / 2.0) * std::f64::consts::SQRT_2;
    let [bx0, by0, bx1, by1] =
        bounds.unwrap_or([f64::from(0), f64::from(0), f64::from(size), f64::from(size)]);
    // Which band of a coat a tap at distance `d` lands in.
    let band = |d: f64, rim: f64| -> u8 {
        if d <= 0.0 {
            0
        } else if d <= rim {
            1
        } else {
            2
        }
    };
    // The band every tap of a pixel shares, if the centre distance `d`
    // proves they all share one.
    let uniform = |d: f64, rim: f64| -> Option<u8> {
        let lo = band(d - reach, rim);
        (lo == band(d + reach, rim)).then_some(lo)
    };
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            // The bounds skip applies per tap, so only shortcut pixels whose
            // taps all sit inside the bounds.
            let in_bounds =
                cx - reach >= bx0 && cx + reach <= bx1 && cy - reach >= by0 && cy + reach <= by1;
            let body_band = if in_bounds {
                uniform(field(cx, cy), outline_width)
            } else {
                None
            };
            let accent_band = match (&accent, in_bounds) {
                (None, _) => Some(2),
                (Some(mark), true) => uniform((mark.field)(cx, cy), aow),
                (Some(_), false) => None,
            };
            let (mut fill_cov, mut outline_cov, mut afill_cov, mut aoutline_cov) =
                (0.0, 0.0, 0.0, 0.0);
            if let (Some(b), Some(a)) = (body_band, accent_band) {
                match b {
                    0 => fill_cov = 1.0,
                    1 => outline_cov = 1.0,
                    _ => {}
                }
                match a {
                    0 => afill_cov = 1.0,
                    1 => aoutline_cov = 1.0,
                    _ => {}
                }
            } else {
                for sy in 0..ss {
                    for sx in 0..ss {
                        let px = f64::from(x) + (f64::from(sx) + 0.5) * step;
                        let py = f64::from(y) + (f64::from(sy) + 0.5) * step;
                        if px < bx0 || px > bx1 || py < by0 || py > by1 {
                            continue;
                        }
                        match body_band {
                            Some(0) => fill_cov += 1.0,
                            Some(1) => outline_cov += 1.0,
                            Some(_) => {}
                            None => match band(field(px, py), outline_width) {
                                0 => fill_cov += 1.0,
                                1 => outline_cov += 1.0,
                                _ => {}
                            },
                        }
                        if let Some(mark) = accent.as_ref() {
                            let ab = match accent_band {
                                Some(b) => b,
                                None => band((mark.field)(px, py), aow),
                            };
                            match ab {
                                0 => afill_cov += 1.0,
                                1 => aoutline_cov += 1.0,
                                _ => {}
                            }
                        }
                    }
                }
                fill_cov /= taps;
                outline_cov /= taps;
                afill_cov /= taps;
                aoutline_cov /= taps;
            }
            // Back to front, premultiplied `Over`: out = src + dst * (1 - a).
            // The outline in the caller's own color, scaled by its coverage
            // -- at full coverage this is byte-identical to the flat outline
            // the old bitmaps wrote. (`Cursor` always passes the contrasting
            // rim at the fill's alpha; that policy lives in `contrast_rim`,
            // not here.)
            let ola = (outline_cov * oa).clamp(0.0, 1.0);
            let (mut r, mut g, mut b, mut a) = (orr * ola, ogg * ola, obb * ola, ola);
            let fla = (fill_cov * fa).clamp(0.0, 1.0);
            r = fr * fla + r * (1.0 - fla);
            g = fg * fla + g * (1.0 - fla);
            b = fb * fla + b * (1.0 - fla);
            a = fla + a * (1.0 - fla);
            // The accent mark over the body, with its own hairline of the
            // same rim color when it has one: a halo for a mark that floats
            // off the body (the paw badge), none for one inlaid in it.
            let aola = (aoutline_cov * aoa).clamp(0.0, 1.0);
            r = aorr * aola + r * (1.0 - aola);
            g = aogg * aola + g * (1.0 - aola);
            b = aobb * aola + b * (1.0 - aola);
            a = aola + a * (1.0 - aola);
            let afla = (afill_cov * aa).clamp(0.0, 1.0);
            r = ar * afla + r * (1.0 - afla);
            g = ag * afla + g * (1.0 - afla);
            b = ab * afla + b * (1.0 - afla);
            a = afla + a * (1.0 - afla);
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
        let ccw = vec![vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]];
        let cw = vec![vec![(0.0, 0.0), (0.0, 4.0), (4.0, 4.0), (4.0, 0.0)]];
        assert!(sd_path(2.0, 2.0, &ccw) < 0.0);
        assert!(sd_path(2.0, 2.0, &cw) < 0.0);
        assert!(sd_path(6.0, 2.0, &ccw) > 0.0);
        assert!(sd_path(6.0, 2.0, &cw) > 0.0);
        // Exact distances, both sides.
        assert_eq!(sd_path(2.0, 1.0, &ccw), -1.0);
        assert_eq!(sd_path(7.0, 2.0, &ccw), 3.0);
    }

    #[test]
    fn a_degenerate_path_contains_nothing() {
        assert!(sd_path(0.0, 0.0, &[]).is_infinite());
        assert!(sd_path(0.0, 0.0, &[vec![(0.0, 0.0)], vec![(1.0, 1.0), (2.0, 2.0)]]).is_infinite());
    }

    #[test]
    fn an_inner_subpath_is_a_hole() {
        // A ring of square outlines: the even-odd rule across subpaths makes
        // the inner one a hole whichever way it winds, which is how Mocu's
        // lens and not-allowed ring get theirs.
        for inner in [
            vec![(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)],
            vec![(2.0, 2.0), (2.0, 6.0), (6.0, 6.0), (6.0, 2.0)],
        ] {
            let ring = vec![vec![(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)], inner];
            assert!(sd_path(1.0, 4.0, &ring) < 0.0, "the band is inside");
            assert_eq!(
                sd_path(4.0, 4.0, &ring),
                2.0,
                "the hole is outside, 2 from its edge"
            );
        }
    }

    #[test]
    fn over_composites_premultiplied_pixels() {
        let mut dst = vec![10, 20, 30, 255, 0, 0, 0, 0, 100, 100, 100, 255];
        let src = vec![0, 0, 0, 0, 40, 50, 60, 128, 255, 255, 255, 255];
        over(&mut dst, &src);
        // Transparent source keeps the destination; source over nothing is
        // the source; opaque source replaces.
        assert_eq!(&dst[0..4], &[10, 20, 30, 255]);
        assert_eq!(&dst[4..8], &[40, 50, 60, 128]);
        assert_eq!(&dst[8..12], &[255, 255, 255, 255]);
        // Half over opaque: src + dst * (1 - 128/255).
        let mut dst = vec![200, 200, 200, 255];
        over(&mut dst, &[64, 64, 64, 128]);
        assert_eq!(dst, vec![64 + 100, 64 + 100, 64 + 100, 128 + 127]);
    }

    /// The shortcut in [`render`] (one centre sample when it proves every
    /// tap lands in the same band) must be invisible: same bytes as sampling
    /// every tap. Checked against a brute-force reference at a size that
    /// takes the 4x4 grid and one that takes the 2x2 grid, with an accent
    /// that has a hairline and one that does not.
    #[test]
    fn the_centre_sample_shortcut_changes_no_pixel() {
        let ring = [
            vec![(3.0, 3.0), (60.0, 5.0), (58.0, 61.0), (4.0, 57.0)],
            vec![(20.0, 20.0), (40.0, 22.0), (38.0, 40.0), (21.0, 39.0)],
        ];
        for size in [24, 96] {
            let scale = f64::from(size) / 64.0;
            let scaled: Vec<Vec<(f64, f64)>> = ring
                .iter()
                .map(|s| s.iter().map(|(x, y)| (x * scale, y * scale)).collect())
                .collect();
            let body = |x: f64, y: f64| sd_path(x, y, &scaled);
            let c = f64::from(size) / 2.0;
            let mark = move |x: f64, y: f64| sd_circle(x, y, c * 0.4, c * 0.4, c * 0.2);
            for aow in [0.0, 0.5] {
                let coat = Coat {
                    fill: [10, 20, 30, 255],
                    outline: [200, 210, 220, 255],
                    outline_width: 1.5,
                };
                let accent = Coat {
                    fill: [40, 120, 230, 255],
                    outline: [200, 210, 220, 255],
                    outline_width: aow,
                };
                let fast = render(
                    size,
                    &body,
                    coat,
                    Some(Accent {
                        field: &mark,
                        coat: accent,
                    }),
                    None,
                );
                let slow = reference(size, &body, coat, &mark, accent);
                assert_eq!(fast, slow, "size {size}, accent hairline {aow}");
            }
        }
    }

    /// Every tap of every pixel, no shortcut: the definition `render` must
    /// match.
    fn reference(
        size: i32,
        body: &dyn Fn(f64, f64) -> f64,
        coat: Coat,
        mark: &dyn Fn(f64, f64) -> f64,
        accent: Coat,
    ) -> Vec<u8> {
        let ss = supersample(size);
        let step = 1.0 / f64::from(ss);
        let taps = f64::from(ss * ss);
        let (fr, fg, fb, fa) = straight_from_premul(coat.fill);
        let (or, og, ob, oa) = straight_from_premul(coat.outline);
        let (ar, ag, ab, aa) = straight_from_premul(accent.fill);
        let (aor, aog, aob, aoa) = straight_from_premul(accent.outline);
        let mut out = vec![0u8; (size * size * 4) as usize];
        for y in 0..size {
            for x in 0..size {
                let (mut f, mut o, mut af, mut ao) = (0.0, 0.0, 0.0, 0.0);
                for sy in 0..ss {
                    for sx in 0..ss {
                        let px = f64::from(x) + (f64::from(sx) + 0.5) * step;
                        let py = f64::from(y) + (f64::from(sy) + 0.5) * step;
                        let d = body(px, py);
                        if d <= 0.0 {
                            f += 1.0;
                        } else if d <= coat.outline_width {
                            o += 1.0;
                        }
                        let da = mark(px, py);
                        if da <= 0.0 {
                            af += 1.0;
                        } else if da <= accent.outline_width {
                            ao += 1.0;
                        }
                    }
                }
                let layers = [
                    (o / taps * oa, or, og, ob),
                    (f / taps * fa, fr, fg, fb),
                    (ao / taps * aoa, aor, aog, aob),
                    (af / taps * aa, ar, ag, ab),
                ];
                let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                for (la, lr, lg, lb) in layers {
                    let la = la.clamp(0.0, 1.0);
                    r = lr * la + r * (1.0 - la);
                    g = lg * la + g * (1.0 - la);
                    b = lb * la + b * (1.0 - la);
                    a = la + a * (1.0 - la);
                }
                let i = ((y * size + x) * 4) as usize;
                out[i] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
                out[i + 1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
                out[i + 2] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
                out[i + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }

    #[test]
    fn a_fully_covered_field_writes_the_fill_verbatim() {
        let pixels = render(
            4,
            &|_, _| -1.0,
            Coat {
                fill: [10, 20, 30, 255],
                outline: [0, 0, 0, 255],
                outline_width: 1.0,
            },
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
            Coat {
                fill: [10, 20, 30, 255],
                outline: [0, 0, 0, 255],
                outline_width: 1.0,
            },
            None,
            None,
        );
        assert!(pixels.iter().all(|b| *b == 0));
    }

    #[test]
    fn a_fully_covered_accent_writes_its_fill_over_the_body() {
        // The accent composites over the body with its own flat color: at
        // full coverage on both fields the pixel is the accent fill.
        let pixels = render(
            4,
            &|_, _| -1.0,
            Coat {
                fill: [10, 20, 30, 255],
                outline: [0, 0, 0, 255],
                outline_width: 1.0,
            },
            Some(Accent {
                field: &|_, _| -1.0,
                coat: Coat {
                    fill: [40, 50, 60, 255],
                    outline: [0, 0, 0, 255],
                    outline_width: 1.0,
                },
            }),
            None,
        );
        assert_eq!(&pixels[0..4], &[40, 50, 60, 255]);
    }

    #[test]
    fn an_accent_outside_its_mark_leaves_the_body_alone() {
        // Same body coverage, but the accent field is empty: the pixel is
        // the body fill, proving the accent adds nothing where it is not.
        let pixels = render(
            4,
            &|_, _| -1.0,
            Coat {
                fill: [10, 20, 30, 255],
                outline: [0, 0, 0, 255],
                outline_width: 1.0,
            },
            Some(Accent {
                field: &|_, _| f64::INFINITY,
                coat: Coat {
                    fill: [40, 50, 60, 255],
                    outline: [0, 0, 0, 255],
                    outline_width: 1.0,
                },
            }),
            None,
        );
        assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
    }
}
