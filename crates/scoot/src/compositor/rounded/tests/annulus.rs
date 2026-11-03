//! The focus ring as a true concentric annulus: at every corner, the ring's
//! radial thickness equals the straight-edge thickness within one device
//! pixel, with no ring-colored pixel outside the outer arc and none inside
//! the window clip.
//!
//! This is the test that was missing through six earlier corner fixes. The
//! painted ring itself (`paint_ring`) has always been concentric; what broke
//! it was the CSD backdrop: four solid `max(radius, 24)` squares drawn under
//! unbound windows. A square sticks out past a rounded outer arc by
//! `(sqrt(2) - 1) * radius_outer` along the diagonal, so every unbound window
//! -- including the golden suite's own magenta client, which never binds
//! `xdg-decoration` -- grew a filled wedge ~8px thick where the ring should
//! be 2px. The committed goldens blessed the wedge, and every existing pixel
//! suite checked the outer edge only on bound (SSD, backdrop-free) windows,
//! so nothing failed.
//!
//! The sweep below renders a focused window in the headless harness and, for
//! each corner, bins the ring-colored pixels by polar angle around the arcs'
//! shared centre and measures their radial extent. It covers radii x
//! thicknesses x scales x both client kinds (bound square via `Window`,
//! unbound square via `Framed` with zero margin -- same opaque pixels, no
//! decoration object, so the CSD rule backs it). A second test pins the real
//! CSD behavior that must survive: a self-rounding client's crescent still
//! reads as ring color, but nothing ring-colored leaves the outer arc.

use super::*;
use crate::compositor::rounded::row_span;
use smithay::utils::{Physical, Point, Rectangle};

fn pixel_at(pixels: &[u8], x: i32, y: i32) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= CANVAS || y >= CANVAS {
        return None;
    }
    Some(
        pixels[(y * CANVAS + x) as usize * 4..][..4]
            .try_into()
            .expect("in bounds"),
    )
}

/// Asserts the annulus for one focused window on an already-mapped fixture,
/// mutating only the appearance: reuses one client mapping across a sweep of
/// radii and thicknesses so the 90-combo gate finishes well under nextest's
/// timeout. The window stays mapped; only the ring paint inputs change, which
/// is exactly what the key (and the repaint) covers. `unbound` selected the
/// client kind at map time: bound square (`Window`, SSD, no backdrop) or
/// unbound square (`Framed` with zero margin -- same opaque pixels, never
/// creates the decoration object, so the CSD rule backs it, the golden
/// client's shape).
fn assert_annulus_on_fixture(
    fixture: &mut Fixture,
    configured: i32,
    thickness: i32,
    scale: f64,
    what: &str,
) {
    fixture.state.appearance.corner_radius = configured;
    fixture.state.appearance.focus_ring_width = thickness;
    let drawn = fixture.placement();
    let pixels = fixture.render();

    let clip = clip_rect(drawn, scale);
    let radius_inner = physical_radius(configured, clip, scale);
    assert!(
        radius_inner > 1,
        "{what}: needs a radius that cuts (got {radius_inner})"
    );
    let thickness_phys = (f64::from(thickness) * scale).round() as i32;
    assert!(thickness_phys > 0, "{what}: needs a visible ring band");
    let radius_outer = radius_inner + thickness_phys.max(0);
    let (ring_loc, _, canvas) = ring_layout(drawn, thickness, scale);
    let origin: Point<i32, Physical> = ring_loc.to_i32_round();
    let outer = Rectangle::new(origin, canvas);
    assert!(
        radius_outer * 2 <= canvas.h.min(canvas.w),
        "{what}: outer corners must not meet"
    );

    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    assert_ne!(bg, WINDOW_BGRA, "{what}: background sample");
    let (x, _y, w, _h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);
    // The ring color, sampled from the top straight run: the first non-window,
    // non-background pixel above the clip at the window's horizontal middle.
    // A fixed offset (`y - thickness/2 - 1`) misses when fractional rounding
    // leaves a 1px band where `thickness_phys` says 2 (origin vs clip phases),
    // so scan the band instead.
    let mut ring = None;
    for yy in (outer.loc.y..clip.loc.y).rev() {
        let xx = x + w / 2;
        let px = pixel_at(&pixels, xx, yy).expect("in bounds");
        if px != bg && px != WINDOW_BGRA {
            ring = Some(px);
            break;
        }
    }
    let Some(ring) = ring else {
        panic!("{what}: a ring pixel above the clip");
    };

    // The arcs' shared centre per corner (inner and outer are concentric: the
    // outer rect is the clip expanded by the physical thickness, up to the
    // same rounding the paint consumes). The quadrant signs select the corner
    // (`sx`, `sy`): the arc lives where `dx` and `dy` from the centre carry
    // those signs.
    let corners = [
        (
            outer.loc.x + radius_outer,
            outer.loc.y + radius_outer,
            -1.0,
            -1.0,
            "tl",
        ),
        (
            outer.loc.x + outer.size.w - radius_outer,
            outer.loc.y + radius_outer,
            1.0,
            -1.0,
            "tr",
        ),
        (
            outer.loc.x + radius_outer,
            outer.loc.y + outer.size.h - radius_outer,
            -1.0,
            1.0,
            "bl",
        ),
        (
            outer.loc.x + outer.size.w - radius_outer,
            outer.loc.y + outer.size.h - radius_outer,
            1.0,
            1.0,
            "br",
        ),
    ];
    // Seven rays per quadrant: the cardinals, the diagonal, and the points a
    // staircase actually steps through. Each bin owns the ring pixels whose
    // polar angle around the centre falls within half a step, so a thin
    // staircase arc still lands pixels in every bin while a wedge cannot
    // hide between rays.
    let angles: [f64; 7] = [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0];
    for (cx, cy, sx, sy, name) in corners {
        // The corner's bounding square plus a 5px margin for the background
        // check: everything ring-colored in here must belong to the annulus.
        let (bx0, bx1) = if sx < 0.0 {
            (outer.loc.x.min(cx - radius_outer - 5), cx + 2)
        } else {
            (
                cx - 2,
                (outer.loc.x + outer.size.w).max(cx + radius_outer + 5),
            )
        };
        let (by0, by1) = if sy < 0.0 {
            (outer.loc.y.min(cy - radius_outer - 5), cy + 2)
        } else {
            (
                cy - 2,
                (outer.loc.y + outer.size.h).max(cy + radius_outer + 5),
            )
        };
        let mut bins: Vec<Vec<f64>> = vec![Vec::new(); angles.len()];
        for yy in by0..by1 {
            for xx in bx0..bx1 {
                let Some(px) = pixel_at(&pixels, xx, yy) else {
                    continue;
                };
                if px != ring {
                    continue;
                }
                // Pixel centres: the staircase is pinned against this rule.
                let dx = f64::from(xx) + 0.5 - f64::from(cx);
                let dy = f64::from(yy) + 0.5 - f64::from(cy);
                // Keep the quadrant: straight-edge runs belong to the bars,
                // not to this corner's arc.
                if dx * sx < 0.0 || dy * sy < 0.0 {
                    continue;
                }
                let r = dx.hypot(dy);
                // No ring outside the outer arc (the wedge) and none inside
                // the clip (a backdrop leak under opaque content): the
                // global pin, per pixel rather than per ray.
                assert!(
                    r <= f64::from(radius_outer) + 1.0,
                    "{what}: {name}: ring pixel at ({xx}, {yy}) r={r:.2} outside the outer arc {radius_outer}"
                );
                assert!(
                    r >= f64::from(radius_inner) - 1.0,
                    "{what}: {name}: ring pixel at ({xx}, {yy}) r={r:.2} inside the clip {radius_inner}"
                );
                // Angle from the corner's west/east axis toward north/south:
                // 0 is the straight side, 90 the other one.
                let phi = (dy.abs().atan2(dx.abs())).to_degrees();
                let mut best = 0;
                let mut best_d = f64::INFINITY;
                for (i, deg) in angles.iter().enumerate() {
                    let d = (phi - deg).abs();
                    if d < best_d {
                        best_d = d;
                        best = i;
                    }
                }
                // Only the nearest bin owns the pixel, so bins stay narrow;
                // a pixel exactly between two still lands somewhere.
                if best_d <= 7.5 + 1e-9 {
                    bins[best].push(r);
                }
            }
        }
        for (i, deg) in angles.iter().enumerate() {
            let bin = &bins[i];
            assert!(
                !bin.is_empty(),
                "{what}: {name} {deg}deg: no ring pixel on this ray"
            );
            let min_r = bin.iter().copied().fold(f64::INFINITY, f64::min);
            let max_r = bin.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let thick = max_r - min_r;
            // Pixel centres on a 1px staircase quantize the extent down by
            // up to a pixel: a 1px ring can read as 0 extent. The wedge reads
            // as several pixels over, so it still fails loudly.
            assert!(
                thick <= f64::from(thickness_phys) + 1.0,
                "{what}: {name} {deg}deg: radial extent {thick:.2}px, want <={}px",
                thickness_phys + 1,
            );
            // The band must actually span the thickness: its inner edge near
            // the clip and its outer edge near the outer arc. A missing side
            // (a hole in the ring) reads as a thin extent hugging one arc.
            // Fractional rounding can shift the two arcs' centres by a pixel
            // (origin vs clip phases), so the inner edge gets three pixels of
            // slack here; the thickness extent below stays at one, which is
            // what catches the wedge.
            assert!(
                min_r <= f64::from(radius_inner) + 3.0,
                "{what}: {name} {deg}deg: ring starts at r={min_r:.2}, want near the clip {radius_inner}"
            );
            assert!(
                max_r >= f64::from(radius_outer) - 1.0,
                "{what}: {name} {deg}deg: ring ends at r={max_r:.2}, want near the outer arc {radius_outer}"
            );
        }
    }

    // Every outer-corner row: the cut is background -- the concentric-arc pin
    // the shoulder fix introduced, now also on unbound windows, where the
    // square backdrop used to fill the cut. The first kept pixel is not
    // pinned here: fractional rounding can leave a strip a row short
    // (background where a taller strip paints ring) or align the two
    // staircases so a thin ring has no pixel on one side of one row (window
    // where ring sits a row above and below). The polar bins above already
    // require ring in every angular bin with the full radial span, which is
    // what catches holes; this loop is the wedge pin.
    for row in 0..radius_outer {
        let cut = cut_width(radius_outer, row);
        if cut == 0 {
            continue;
        }
        for (py, how) in [
            (origin.y + row, "top"),
            (origin.y + canvas.h - 1 - row, "bottom"),
        ] {
            let (left, right) = (origin.x + cut, origin.x + canvas.w - 1 - cut);
            assert_pixel(
                &pixels,
                CANVAS,
                left - 1,
                py,
                bg,
                &format!("{what}: {how} outer row {row}, cut on the left"),
            );
            assert_pixel(
                &pixels,
                CANVAS,
                right + 1,
                py,
                bg,
                &format!("{what}: {how} outer row {row}, cut on the right"),
            );
        }
    }
    // No ring-colored pixel anywhere outside the outer rounded rect and none
    // inside the clip: the global pin behind the per-angle bins.
    for yy in 0..CANVAS {
        let outer_row = row_span(radius_outer, yy, outer);
        let inner_row = row_span(radius_inner, yy, clip);
        for xx in 0..CANVAS {
            let Some(px) = pixel_at(&pixels, xx, yy) else {
                continue;
            };
            if px != ring {
                continue;
            }
            let in_outer = outer_row.map(|(l, r)| xx >= l && xx < r).unwrap_or(false);
            assert!(
                in_outer,
                "{what}: ring pixel at ({xx}, {yy}) outside the outer arc"
            );
            let in_inner = inner_row.map(|(l, r)| xx >= l && xx < r).unwrap_or(false);
            // Inside the clip the window covers everything except its own
            // transparent crescent -- but these are square clients, fully
            // opaque, so any ring pixel inside the clip is a backdrop leak.
            assert!(
                !in_inner,
                "{what}: ring pixel at ({xx}, {yy}) inside the clip"
            );
        }
    }
}

/// The ring is a concentric annulus for both client kinds: bound (SSD, no
/// backdrop) and unbound square (CSD rule, backdrop clipped to the outer
/// arc). Sweeps radii x thicknesses x scales.
#[test]
fn ring_corner_annulus() {
    // One mapping per scale x client kind; radii and thicknesses mutate only
    // the appearance and repaint. A fresh fixture per combo would remap the
    // window 90 times and time out under nextest; this maps 10 windows.
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        for unbound in [false, true] {
            let mut fixture = Fixture::with_radius_at_scale(8, 1, scale);
            if unbound {
                fixture.run(Step::Framed {
                    color: WINDOW_BGRA,
                    frame: [0x00, 0x00, 0x00, 0xFF],
                    margin: 0,
                    band: Band::Floor,
                });
            } else {
                fixture.run(Step::Window { color: WINDOW_BGRA });
            }
            for configured in [8, 12, 16] {
                for thickness in [1, 2, 4] {
                    assert_annulus_on_fixture(
                        &mut fixture,
                        configured,
                        thickness,
                        scale,
                        &format!(
                            "radius {configured} thickness {thickness} scale {scale} {}",
                            if unbound { "unbound" } else { "bound" }
                        ),
                    );
                }
            }
        }
    }
}

/// A self-rounding client keeps its crescent (the backdrop's reason to
/// exist) and still gets no wedge: the crescent reads as ring color, the
/// outer cut reads as background.
#[test]
fn csd_crescent_without_wedge() {
    for scale in [1.0, 1.5, 2.0] {
        let mut fixture = Fixture::with_radius_at_scale(10, 4, scale);
        fixture.run(Step::SelfRounded {
            color: WINDOW_BGRA,
            decorate: false,
        });
        let drawn = fixture.placement();
        let pixels = fixture.render();
        let clip = clip_rect(drawn, scale);
        let radius_inner = physical_radius(10, clip, scale);
        let thickness_phys = (4.0 * scale).round() as i32;
        let radius_outer = radius_inner + thickness_phys;
        let (ring_loc, _, canvas) = ring_layout(drawn, 4, scale);
        let origin: Point<i32, Physical> = ring_loc.to_i32_round();
        let outer = Rectangle::new(origin, canvas);
        let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
        let (x, y, w, _h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);
        let ring: [u8; 4] = pixels
            [((y - thickness_phys / 2 - 1) * CANVAS + x + w / 2) as usize * 4..][..4]
            .try_into()
            .expect("in bounds");
        // The crescent: the first pixels the compositor keeps past its own
        // staircase but inside the client's wider curve.
        let cut_inner = cut_width(radius_inner, 0);
        let cut_client = cut_width((f64::from(CLIENT_RADIUS) * scale).round() as i32, 0);
        assert!(
            cut_inner + 1 < cut_client,
            "scale {scale}: the client must round wider than the compositor"
        );
        for (px, py) in [(x + cut_inner, y), (x + cut_inner + 1, y)] {
            assert_pixel(
                &pixels,
                CANVAS,
                px,
                py,
                ring,
                &format!("scale {scale}: the crescent must read as ring"),
            );
        }
        // No wedge: every outer-corner cut pixel is background.
        for row in 0..radius_outer {
            let cut = cut_width(radius_outer, row);
            if cut == 0 {
                continue;
            }
            for py in [origin.y + row, origin.y + canvas.h - 1 - row] {
                assert_pixel(
                    &pixels,
                    CANVAS,
                    origin.x + cut - 1,
                    py,
                    bg,
                    &format!("scale {scale}: outer row {row}, no wedge on the left"),
                );
                assert_pixel(
                    &pixels,
                    CANVAS,
                    origin.x + canvas.w - cut,
                    py,
                    bg,
                    &format!("scale {scale}: outer row {row}, no wedge on the right"),
                );
            }
        }
        let _ = outer;
    }
}
