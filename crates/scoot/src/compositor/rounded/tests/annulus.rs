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
use crate::compositor::decorations::Color;
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
    drawn: Rect,
    configured: i32,
    thickness: i32,
    scale: f64,
    what: &str,
) {
    fixture.state.appearance.corner_radius = configured;
    fixture.state.appearance.focus_ring_width = thickness;
    fixture.settle();
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
                    let drawn = fixture.placement();
                    assert_annulus_on_fixture(
                        &mut fixture,
                        drawn,
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

/// Half-transparent orange: over the dark background it blends to a color
/// distinct from both, so a double-blended overlap would read as a different
/// pixel from the band.
const TRANSLUCENT_RING: Color = Color::new(1.0, 0.5, 0.0, 0.5);

/// Which of the three things a pixel can be in the self-rounded corner
/// model: outside the ring's outer arc, ring (the band plus the crescent
/// the backdrop fills), or the client's own opaque content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    Background,
    Ring,
    Window,
}

/// Pins a self-rounded (unbound) window's whole frame against the model:
/// background outside the outer rounded rect, window color inside the
/// client's own rounded rect, ring color everywhere between -- the compositor
/// band and the crescent where the client rounds wider than the clip.
///
/// Every pixel is checked except those with a differently-classified
/// neighbour: fractional scales shift each staircase by up to a pixel (the
/// ring origin and the clip round separately), so a boundary pixel may land
/// on either side. A wedge, a background sliver in the crescent, or a
/// missing backdrop moves whole interior regions, which this still catches.
/// `client_radius` is logical, scaled the way the client scales it. Returns
/// how many steady ring pixels sit inside the clip rect (the crescent), so
/// a caller can require the case to have bitten.
fn assert_self_rounded_frame(
    pixels: &[u8],
    drawn: Rect,
    scale: f64,
    configured: i32,
    thickness: i32,
    client_radius: i32,
    what: &str,
) -> usize {
    let clip = clip_rect(drawn, scale);
    let radius_inner = physical_radius(configured, clip, scale);
    let thickness_phys = (f64::from(thickness) * scale).round() as i32;
    let radius_outer = radius_inner + thickness_phys;
    let (ring_loc, _, canvas) = ring_layout(drawn, thickness, scale);
    let origin: Point<i32, Physical> = ring_loc.to_i32_round();
    let outer = Rectangle::new(origin, canvas);
    let client_phys = (f64::from(client_radius) * scale).round() as i32;

    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let xx = clip.loc.x + clip.size.w / 2;
    let ring = (outer.loc.y..clip.loc.y)
        .rev()
        .map(|yy| pixel(pixels, CANVAS, xx, yy))
        .find(|px| *px != bg && *px != WINDOW_BGRA)
        .unwrap_or_else(|| panic!("{what}: a ring pixel above the clip"));

    let zone = |x: i32, y: i32| -> Zone {
        let in_outer = row_span(radius_outer, y, outer)
            .map(|(l, r)| x >= l && x < r)
            .unwrap_or(false);
        if !in_outer {
            return Zone::Background;
        }
        let row = y - clip.loc.y;
        if row < 0 || row >= clip.size.h {
            return Zone::Ring;
        }
        let edge = row.min(clip.size.h - 1 - row);
        let cut = if client_phys > 0 && edge < client_phys {
            cut_width(client_phys, edge)
        } else {
            0
        };
        // Content shows where the client drew it and the compositor's own
        // clip keeps it: the client's curve intersected with the clip's.
        let kept = row_span(radius_inner, y, clip)
            .map(|(l, r)| x >= l && x < r)
            .unwrap_or(false);
        if kept && x >= clip.loc.x + cut && x < clip.loc.x + clip.size.w - cut {
            Zone::Window
        } else {
            Zone::Ring
        }
    };
    let mut crescent_pixels = 0;
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let here = zone(x, y);
            let steady = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .all(|(dx, dy)| zone(x + dx, y + dy) == here);
            if !steady {
                continue;
            }
            let want = match here {
                Zone::Background => bg,
                Zone::Ring => ring,
                Zone::Window => WINDOW_BGRA,
            };
            assert_pixel(
                pixels,
                CANVAS,
                x,
                y,
                want,
                &format!("{what}: ({x}, {y}) should read as {here:?}"),
            );
            // Ring inside the clip rect and past the compositor's own
            // staircase: the crescent the backdrop is for.
            if here == Zone::Ring
                && x >= clip.loc.x
                && x < clip.loc.x + clip.size.w
                && y >= clip.loc.y
                && y < clip.loc.y + clip.size.h
            {
                crescent_pixels += 1;
            }
        }
    }
    crescent_pixels
}

/// Maps an unbound self-rounded window (radius `client_radius`, drawn at
/// most `max_size`) and returns the fixture with the rect it really drew.
fn self_rounded_fixture(
    scale: f64,
    client_radius: i32,
    max_size: Option<(i32, i32)>,
) -> (Fixture, Rect) {
    let mut fixture = Fixture::with_radius_at_scale(8, 1, scale);
    fixture.run(Step::SelfRoundedCustom {
        color: WINDOW_BGRA,
        radius: client_radius,
        max_size,
    });
    let slot = fixture.placement();
    let drawn = match max_size {
        Some((mw, mh)) => Rect::new(slot.x, slot.y, slot.w.min(mw), slot.h.min(mh)),
        None => slot,
    };
    (fixture, drawn)
}

/// A translucent ring color reads as one uniform band: the backdrop and the
/// band overlap in the strips, and a double-blended overlap would show as a
/// different pixel (and fail the "ring-colored" bins). Bound and unbound
/// square clients, so the backdrop is both absent and present.
#[test]
fn a_translucent_ring_is_one_uniform_annulus() {
    for scale in [1.0, 1.5, 2.0] {
        for unbound in [false, true] {
            let mut fixture = Fixture::with_radius_at_scale(8, 1, scale);
            fixture.state.appearance.focus_ring_active_color = TRANSLUCENT_RING;
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
            for configured in [8, 16] {
                for thickness in [2, 4] {
                    let drawn = fixture.placement();
                    assert_annulus_on_fixture(
                        &mut fixture,
                        drawn,
                        configured,
                        thickness,
                        scale,
                        &format!(
                            "translucent: radius {configured} thickness {thickness} scale {scale} {}",
                            if unbound { "unbound" } else { "bound" }
                        ),
                    );
                }
            }
        }
    }
}

/// The color the translucent tests rely on really is a blend: neither the
/// background nor the opaque version of the same hue.
#[test]
fn the_translucent_ring_is_a_blend() {
    let ring_above = |color: Color| {
        let mut fixture = Fixture::with_radius_at_scale(8, 4, 1.0);
        fixture.state.appearance.focus_ring_active_color = color;
        fixture.run(Step::Window { color: WINDOW_BGRA });
        let drawn = fixture.placement();
        let pixels = fixture.render();
        let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
        (
            bg,
            pixel(&pixels, CANVAS, drawn.x + drawn.w / 2, drawn.y - 2),
        )
    };
    let (bg, blend) = ring_above(TRANSLUCENT_RING);
    let (_, opaque) = ring_above(Color::new(1.0, 0.5, 0.0, 1.0));
    assert_ne!(blend, bg, "the translucent ring must draw");
    assert_ne!(blend, opaque, "the translucent ring must not be opaque");
}

/// A translucent ring behind a client that rounds wider than the clip: the
/// crescent (backdrop over the background) must equal the band, not a
/// darker overlap.
#[test]
#[ignore = "real bug: the strips are shorter than the backdrop square (docs/backlog/core/csd-backdrop-truncated-by-strip.md)"]
fn a_translucent_ring_hugs_a_rounder_client_without_a_seam() {
    for scale in [1.0, 1.5] {
        let (mut fixture, drawn) = self_rounded_fixture(scale, 24, None);
        fixture.state.appearance.focus_ring_active_color = TRANSLUCENT_RING;
        // A thin ring on a wide radius: the backdrop square's corner sits
        // well past the outer arc here (by `(sqrt(2) * radius - radius_outer)`
        // along the diagonal), so an unclipped backdrop would show as a
        // wedge. Client radius 24 stays rounder than the configured 16.
        fixture.state.appearance.corner_radius = 16;
        fixture.state.appearance.focus_ring_width = 2;
        let pixels = fixture.render();
        let crescent = assert_self_rounded_frame(
            &pixels,
            drawn,
            scale,
            16,
            2,
            24,
            &format!("translucent crescent scale {scale}"),
        );
        assert!(crescent > 20, "scale {scale}: only {crescent} crescent px");
    }
}

/// A window smaller than twice the radius: the clamped corners meet, the
/// strip canvases are short and the backdrop side clamps to the window. The
/// ring is still one concentric annulus with nothing outside the outer arc.
#[test]
fn a_window_smaller_than_twice_the_radius_keeps_the_annulus() {
    for scale in [1.0, 1.5, 2.0] {
        for size in [(30, 30), (44, 26)] {
            let (mut fixture, drawn) = self_rounded_fixture(scale, 0, Some(size));
            assert_eq!((drawn.w, drawn.h), size, "the window must draw small");
            for configured in [16, 40] {
                for thickness in [1, 2, 4] {
                    assert_annulus_on_fixture(
                        &mut fixture,
                        drawn,
                        configured,
                        thickness,
                        scale,
                        &format!(
                            "small {size:?}: radius {configured} thickness {thickness} scale {scale}"
                        ),
                    );
                }
            }
        }
    }
}

/// The same tiny window with a client rounder than the clamped clip: the
/// backdrop's side clamp must still fill the crescent.
#[test]
#[ignore = "real bug: the strips are shorter than the backdrop square (docs/backlog/core/csd-backdrop-truncated-by-strip.md)"]
fn a_tiny_rounder_client_still_gets_its_crescent() {
    for scale in [1.0, 2.0] {
        let (mut fixture, drawn) = self_rounded_fixture(scale, 24, Some((60, 60)));
        for (configured, thickness) in [(8, 2), (40, 4)] {
            fixture.state.appearance.corner_radius = configured;
            fixture.state.appearance.focus_ring_width = thickness;
            let pixels = fixture.render();
            assert_self_rounded_frame(
                &pixels,
                drawn,
                scale,
                configured,
                thickness,
                24,
                &format!("tiny radius {configured} thickness {thickness} scale {scale}"),
            );
        }
    }
}

/// The backdrop's original job: a client rounder than the configured radius
/// (24 against 8 and 12) leaves a crescent between its curve and the ring,
/// which must stay ring-colored -- no background sliver inside the outer
/// arc, nothing ring-colored outside it.
#[test]
#[ignore = "real bug: the strips are shorter than the backdrop square (docs/backlog/core/csd-backdrop-truncated-by-strip.md)"]
fn a_rounder_client_keeps_a_ring_colored_crescent() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let (mut fixture, drawn) = self_rounded_fixture(scale, 24, None);
        for configured in [8, 12] {
            for thickness in [2, 4] {
                fixture.state.appearance.corner_radius = configured;
                fixture.state.appearance.focus_ring_width = thickness;
                let pixels = fixture.render();
                let crescent = assert_self_rounded_frame(
                    &pixels,
                    drawn,
                    scale,
                    configured,
                    thickness,
                    24,
                    &format!("radius {configured} thickness {thickness} scale {scale}"),
                );
                assert!(
                    crescent > 20,
                    "radius {configured} thickness {thickness} scale {scale}: only {crescent} crescent px"
                );
            }
        }
    }
}
