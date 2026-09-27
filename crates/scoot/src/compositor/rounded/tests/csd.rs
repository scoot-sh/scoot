//! The CSD backdrop: a window that decorates itself (never created a
//! `zxdg_toplevel_decoration_v1` object, the way GTK never does) rounds its
//! own corners at a radius nothing on the wire reports, so the ring's
//! tighter inner edge leaves a background crescent at every corner
//! (`docs/backlog/resolved/client-rounded-corners-vs-ring-done.md`). The ring path
//! backs such a window with a solid rect in its own ring color, drawn
//! directly under its drawn rect: the client's own alpha shapes the visible
//! part, so the corners read as the ring hugging the client's curve.
//!
//! Every test here maps one window drawing a full-size buffer with
//! transparent rounded corners (client radius 12, past the configured 10 --
//! the libadwaita shape) and samples the drawn rect's own corner pixel: with
//! the backdrop it is ring color, without it (an SSD window, or no content
//! at all) it is background. Scale stays 1.0 -- the live evidence covers
//! 1.5 -- so buffer and logical pixels coincide.

use super::*;

/// Maps one self-rounded window and renders: `decorate` creates the
/// decoration object first (the SSD twin), and the returned pixels are the
/// frame.
fn render_self_rounded(fixture: &mut Fixture, color: [u8; 4], decorate: bool) -> Vec<u8> {
    fixture.run(Step::SelfRounded { color, decorate });
    fixture.render()
}

/// The frame's background and ring colors, sampled where only they can be:
/// the canvas corner is never covered (one window on an empty output), and
/// the top bar's middle row is always ring.
fn samples(pixels: &[u8], drawn: Rect) -> ([u8; 4], [u8; 4]) {
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let ring = pixel(pixels, CANVAS, drawn.x + drawn.w / 2, drawn.y - 2);
    assert_ne!(ring, bg, "the top bar must draw");
    (bg, ring)
}

/// The crescent pixels: the first two the compositor's clip keeps on the
/// drawn rect's top row. The ring's own band ends where the clip's staircase
/// does (`cut_width(RADIUS, 0)` from the corner), and the client's wider
/// curve starts two pixels further in (`cut_width(CLIENT_RADIUS, 0)`), so
/// exactly these pixels are backdrop-or-background: ring paint never reaches
/// them and client content never does either. Sampling the rect's corner
/// itself would prove nothing -- the ring tiles the clip's cut there on
/// every path.
fn crescent(drawn: Rect) -> [(i32, i32); 2] {
    let cut = cut_width(10, 0);
    assert!(
        cut + 1 < cut_width(CLIENT_RADIUS, 0),
        "the configured radius must stay well under the client's"
    );
    [(drawn.x + cut, drawn.y), (drawn.x + cut + 1, drawn.y)]
}

/// A window that never negotiated decorations gets the backdrop: the
/// crescent between the clip's staircase and the client's own curve reads
/// as the ring, not the background.
#[test]
fn an_undecorated_self_rounded_window_backdrops_its_corners_in_ring_color() {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, 1.0);
    let pixels = render_self_rounded(&mut fixture, WINDOW_BGRA, false);
    let drawn = fixture.placement();
    let (bg, ring) = samples(&pixels, drawn);

    for (px, py) in crescent(drawn) {
        assert_pixel(
            &pixels,
            CANVAS,
            px,
            py,
            ring,
            "the client's own corner must read as the ring",
        );
    }
    assert_pixel(
        &pixels,
        CANVAS,
        drawn.x + drawn.w / 2,
        drawn.y + drawn.h / 2,
        WINDOW_BGRA,
        "the window's middle is still its own content",
    );
    let corners = census(&pixels);
    assert!(
        corners.get(&bg).copied().unwrap_or(0) < (CANVAS * CANVAS) as usize,
        "the frame must draw more than background"
    );
}

/// The SSD twin -- the same buffer, but the client created a decoration
/// object and the session told it `ServerSide` -- gets no backdrop: its
/// corner pixel is background, and the frame is byte-identical to a session
/// that never knew about backdrops.
#[test]
fn a_decorated_window_gets_no_backdrop() {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, 1.0);
    let pixels = render_self_rounded(&mut fixture, WINDOW_BGRA, true);
    assert_eq!(
        fixture.state.decoration_bound,
        std::collections::HashSet::from([WindowId(1)]),
        "creating the decoration object must record the negotiation"
    );
    let drawn = fixture.placement();
    let (bg, _) = samples(&pixels, drawn);

    for (px, py) in crescent(drawn) {
        assert_pixel(
            &pixels,
            CANVAS,
            px,
            py,
            bg,
            "an SSD window's crescent stays background: no backdrop",
        );
    }
    assert_pixel(
        &pixels,
        CANVAS,
        drawn.x + drawn.w / 2,
        drawn.y + drawn.h / 2,
        WINDOW_BGRA,
        "the window's middle is still its own content",
    );
}

/// With `prefer_no_csd = false` the mode last sent decides: a bound window
/// told `ClientSide` at its own request backdrops like an unbound one.
#[test]
fn a_client_side_mode_at_its_own_request_backdrops() {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Harness::headless(
        Appearance {
            corner_radius: RADIUS,
            focus_ring_width: THICKNESS,
            prefer_no_csd: false,
            ..Appearance::default()
        },
        CANVAS,
    );
    fixture.spawn(run_client);
    fixture.run(Step::SelfRounded {
        color: WINDOW_BGRA,
        decorate: true,
    });
    fixture.run(Step::RequestClientSide);
    let pixels = fixture.render();
    let drawn = fixture.placement();
    let (_, ring) = samples(&pixels, drawn);

    for (px, py) in crescent(drawn) {
        assert_pixel(
            &pixels,
            CANVAS,
            px,
            py,
            ring,
            "a ClientSide window backdrops whatever negotiated it",
        );
    }
}

/// A window with nothing committed yet gets the hollow ring but no backdrop:
// its interior is background, not a solid ring-colored fill.
#[test]
fn a_window_with_no_commit_gets_a_ring_but_no_backdrop_fill() {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, 1.0);
    fixture.run(Step::Uncommitted);
    let pixels = fixture.render();
    let placements = fixture.placements();
    assert_eq!(
        placements.len(),
        1,
        "the window is placed before it commits"
    );
    let drawn = placements[0];
    let (bg, ring) = samples(&pixels, drawn);

    assert_pixel(
        &pixels,
        CANVAS,
        drawn.x + drawn.w / 2,
        drawn.y + drawn.h / 2,
        bg,
        "no content means no backdrop fill",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        drawn.x + drawn.w / 2,
        drawn.y - 2,
        ring,
        "the hollow ring still draws",
    );
}
