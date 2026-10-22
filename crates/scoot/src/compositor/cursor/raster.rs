//! A tiny supersampled rasterizer for the cursor outlines.
//!
//! The caller describes a shape as a signed-distance field (negative inside
//! the fill, positive outside, in pixels -- [`sd_path`] over flattened
//! outlines). [`render`] evaluates it at `SS x SS` subpixel taps per output
//! pixel, once per bitmap at startup or on config reload -- never per frame
//! -- and paints fill plus a rim of the outline color around it into
//! premultiplied `Argb8888` bytes. [`over`] stacks the layers a cursor is
//! made of.
//!
//! Supersampling (rather than analytic coverage) is what keeps the two
//! render tiers identical: the bitmap is plain `Argb8888` pixels by the
//! time any renderer sees it, so pixman and the GPU tier composite
//! byte-identical input.
//!
//! # Coordinate convention
//!
//! Subpixel taps are at `(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS)` for the
//! output pixel `(x, y)` -- i.e. pixel centers sit on the half grid, the
//! same convention pixman samples with. A shape edge exactly on an integer
//! coordinate therefore covers its pixels fully or not at all, which is
//! what makes a grid-fitted edge crisp.

/// Subpixel taps per pixel axis at display sizes (see [`supersample`]).
pub const SS: i32 = 4;

/// Subpixel taps per pixel axis for `size`: 4x4 (16 taps) at the sizes a
/// cursor is actually drawn at, 2x2 above 64px, where every feature is tens
/// of pixels wide and 4 taps still resolve the edge. The build runs once per
/// startup or reload -- never per frame -- but a 256px bitmap at 16 taps is
/// ten million field evaluations; this keeps the largest allowed size
/// honest without touching small-size quality.
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

/// Signed distance from `(px, py)` to a compound polygon made of several
/// closed `subpaths`: negative inside, in pixels. Inside is decided by the
/// even-odd rule across *all* subpaths together, which is how a ring (an
/// outer circle with an inner one) or a lens with a handle gets its hole --
/// the same result SVG's nonzero rule gives for the vendored cursor art,
/// whose holes always wind against their outlines. Subpaths with fewer than
/// three vertices contribute nothing.
pub fn sd_path(px: f64, py: f64, subpaths: &[Vec<(f64, f64)>]) -> f64 {
    let mut outside = f64::INFINITY;
    for verts in subpaths.iter().filter(|v| v.len() >= 3) {
        for i in 0..verts.len() {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % verts.len()];
            outside = outside.min(sd_segment(px, py, ax, ay, bx, by));
        }
    }
    if inside(px, py, subpaths) {
        -outside
    } else {
        outside
    }
}

/// Whether `(px, py)` is inside the compound polygon `subpaths`, by the same
/// even-odd rule as [`sd_path`] -- without the distance, for callers that
/// only need the side (the spinner's mask).
pub fn inside(px: f64, py: f64, subpaths: &[Vec<(f64, f64)>]) -> bool {
    let mut inside = false;
    for verts in subpaths.iter().filter(|v| v.len() >= 3) {
        for i in 0..verts.len() {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % verts.len()];
            if (ay > py) != (by > py) && px < (bx - ax) * (py - ay) / (by - ay) + ax {
                inside = !inside;
            }
        }
    }
    inside
}

/// Composites the premultiplied `Argb8888` buffer `src` over `dst` in place
/// (`out = src + dst * (1 - src_alpha)`, per channel): how a shape's second
/// layer -- a badge with its own rim, the spinner -- lands over the first. Both buffers are
/// the same size; a shorter `src` composites only what it covers.
pub fn over(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0.iter())
    {
        let keep = 255 - u32::from(s[3]);
        for c in 0..4 {
            // Rounded `d * keep / 255`, exact for 8-bit premultiplied data.
            let scaled = (u32::from(d[c]) * keep + 127) / 255;
            d[c] = (u32::from(s[c]) + scaled).min(255) as u8;
        }
    }
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

/// One flat-color coat of paint: a fill plus the rim around it.
#[derive(Clone, Copy)]
pub struct Coat {
    /// Flat color, premultiplied little-endian `Argb8888`.
    pub fill: [u8; 4],
    /// Rim color, in the caller's color (see `shapes::generate`).
    pub outline: [u8; 4],
    /// How far past the fill edge the rim reaches, in pixels (0: no rim).
    pub outline_width: f64,
}

/// Renders `field` (signed distance to the fill edge, negative inside, in
/// pixels) into a fresh `size * size * 4` premultiplied `Argb8888` buffer:
/// the rim, then the fill over it.
///
/// `mask`, when given, scales the fill's opacity tap by tap (`0.0..=1.0`):
/// how Mocu's busy spinner is seen through its wedge mask. `bounds` is the
/// field's bounding box, already expanded by the caller past the rim and one
/// pixel of bleed: taps outside it are transparent without evaluating the
/// field. `None` evaluates everywhere.
///
/// # Skipping taps that cannot differ
///
/// The fields here are 1-Lipschitz (true distances, or offsets and minima
/// of them), so one evaluation at a pixel's centre bounds the field at all
/// of its taps: none sits further than `reach` (the tap grid's half
/// diagonal) from the centre. When that bound puts every tap in the same
/// band -- all fill, all rim, or all clear -- the pixel's coverage is known
/// without the other taps, and the result is bit-identical to evaluating
/// them. Only pixels an edge crosses pay for the full grid, which keeps the
/// many-edged outlines cheap at large sizes. A masked render samples every
/// tap: its opacity varies inside the fill.
pub fn render(
    size: i32,
    field: &dyn Fn(f64, f64) -> f64,
    coat: Coat,
    mask: Option<&dyn Fn(f64, f64) -> f64>,
    bounds: Option<[f64; 4]>,
) -> Vec<u8> {
    let (fr, fg, fb, fa) = straight_from_premul(coat.fill);
    let (orr, ogg, obb, oa) = straight_from_premul(coat.outline);
    let rim = coat.outline_width;
    let ss = supersample(size);
    let taps = f64::from(ss * ss);
    let step = 1.0 / f64::from(ss);
    let reach = (0.5 - step / 2.0) * std::f64::consts::SQRT_2;
    let [bx0, by0, bx1, by1] = bounds.unwrap_or([0.0, 0.0, f64::from(size), f64::from(size)]);
    // Which band a tap at distance `d` lands in: fill, rim or clear.
    let band = |d: f64| -> u8 {
        if d <= 0.0 {
            0
        } else if d <= rim {
            1
        } else {
            2
        }
    };
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let in_bounds =
                cx - reach >= bx0 && cx + reach <= bx1 && cy - reach >= by0 && cy + reach <= by1;
            let uniform = (in_bounds && mask.is_none())
                .then(|| {
                    let d = field(cx, cy);
                    let lo = band(d - reach);
                    (lo == band(d + reach)).then_some(lo)
                })
                .flatten();
            let (fill_cov, outline_cov) = match uniform {
                Some(0) => (1.0, 0.0),
                Some(1) => (0.0, 1.0),
                Some(_) => (0.0, 0.0),
                None => {
                    let (mut f, mut o) = (0.0, 0.0);
                    for sy in 0..ss {
                        for sx in 0..ss {
                            let px = f64::from(x) + (f64::from(sx) + 0.5) * step;
                            let py = f64::from(y) + (f64::from(sy) + 0.5) * step;
                            if px < bx0 || px > bx1 || py < by0 || py > by1 {
                                continue;
                            }
                            match band(field(px, py)) {
                                0 => f += mask.map_or(1.0, |m| m(px, py).clamp(0.0, 1.0)),
                                1 => o += 1.0,
                                _ => {}
                            }
                        }
                    }
                    (f / taps, o / taps)
                }
            };
            // Fill and rim cover disjoint sets of taps, so the pixel is their
            // coverage-weighted sum (premultiplied): a pixel half fill, half
            // rim is fully covered, not the 75% that compositing one over the
            // other as if they overlapped would give.
            let ola = (outline_cov * oa).clamp(0.0, 1.0);
            let fla = (fill_cov * fa).clamp(0.0, 1.0);
            let r = fr * fla + orr * ola;
            let g = fg * fla + ogg * ola;
            let b = fb * fla + obb * ola;
            let a = (fla + ola).min(1.0);
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

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn a_degenerate_segment_measures_distance_to_its_point() {
        assert_eq!(sd_segment(3.0, 4.0, 0.0, 0.0, 0.0, 0.0), 5.0);
    }

    #[test]
    fn a_square_is_negative_inside_regardless_of_winding() {
        let ccw = vec![square(0.0, 0.0, 4.0, 4.0)];
        let mut cw = ccw.clone();
        cw[0].reverse();
        for path in [&ccw, &cw] {
            assert!(sd_path(2.0, 2.0, path) < 0.0);
            assert!(sd_path(6.0, 2.0, path) > 0.0);
            assert_eq!(sd_path(2.0, 1.0, path), -1.0);
            assert_eq!(sd_path(7.0, 2.0, path), 3.0);
        }
    }

    #[test]
    fn a_degenerate_path_contains_nothing() {
        assert!(sd_path(0.0, 0.0, &[]).is_infinite());
        assert!(sd_path(0.0, 0.0, &[vec![(0.0, 0.0)], vec![(1.0, 1.0), (2.0, 2.0)]]).is_infinite());
    }

    #[test]
    fn an_inner_subpath_is_a_hole() {
        // The even-odd rule across subpaths makes the inner one a hole
        // whichever way it winds, which is how Mocu's rings get theirs.
        for reverse in [false, true] {
            let mut inner = square(2.0, 2.0, 6.0, 6.0);
            if reverse {
                inner.reverse();
            }
            let ring = vec![square(0.0, 0.0, 8.0, 8.0), inner];
            assert!(sd_path(1.0, 4.0, &ring) < 0.0, "the band is inside");
            assert_eq!(sd_path(4.0, 4.0, &ring), 2.0, "the hole is outside");
        }
    }

    #[test]
    fn over_composites_premultiplied_pixels() {
        let mut dst = vec![10, 20, 30, 255, 0, 0, 0, 0, 100, 100, 100, 255];
        let src = vec![0, 0, 0, 0, 40, 50, 60, 128, 255, 255, 255, 255];
        over(&mut dst, &src);
        assert_eq!(&dst[0..4], &[10, 20, 30, 255]);
        assert_eq!(&dst[4..8], &[40, 50, 60, 128]);
        assert_eq!(&dst[8..12], &[255, 255, 255, 255]);
        let mut dst = vec![200, 200, 200, 255];
        over(&mut dst, &[64, 64, 64, 128]);
        assert_eq!(dst, vec![164, 164, 164, 255]);
    }

    const COAT: Coat = Coat {
        fill: [10, 20, 30, 255],
        outline: [200, 210, 220, 255],
        outline_width: 1.5,
    };

    #[test]
    fn a_fully_covered_field_writes_the_fill_verbatim() {
        let pixels = render(4, &|_, _| -1.0, COAT, None, None);
        assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn an_empty_field_stays_transparent() {
        let pixels = render(4, &|_, _| f64::INFINITY, COAT, None, None);
        assert!(pixels.iter().all(|b| *b == 0));
    }

    #[test]
    fn a_pixel_split_between_fill_and_rim_is_fully_covered() {
        // Left half fill, right half rim: opaque, the two paints averaged.
        let coat = Coat {
            fill: [0, 0, 200, 255],
            outline: [100, 0, 0, 255],
            outline_width: 10.0,
        };
        let pixels = render(1, &|x, _| x - 0.5, coat, None, None);
        assert_eq!(pixels, vec![50, 0, 100, 255]);
    }

    #[test]
    fn a_mask_scales_the_fill_and_leaves_the_rim() {
        // Half opacity everywhere: the fill lands at half alpha, while a
        // pure-rim pixel is untouched by the mask.
        let half = |_: f64, _: f64| 0.5;
        let pixels = render(4, &|_, _| -1.0, COAT, Some(&half), None);
        assert_eq!(&pixels[0..4], &[5, 10, 15, 128]);
        let pixels = render(4, &|_, _| 1.0, COAT, Some(&half), None);
        assert_eq!(&pixels[0..4], &[200, 210, 220, 255]);
    }

    /// The centre-sample shortcut in [`render`] must be invisible: the same
    /// bytes as sampling every tap, at a size on the 4x4 grid and one on the
    /// 2x2 grid. The reference is the same render through a mask of one,
    /// which samples every tap and scales nothing.
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
            let one = |_: f64, _: f64| 1.0;
            let fast = render(size, &body, COAT, None, None);
            let slow = render(size, &body, COAT, Some(&one), None);
            assert_eq!(fast, slow, "size {size}");
        }
    }
}
