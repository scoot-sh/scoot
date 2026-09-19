//! What a client can put on the wire that no toolkit would.
//!
//! Every number behind a layer surface's geometry arrives as a raw `i32` or
//! `u32`, so the extremes have to be survivable rather than merely unlikely.

use super::*;

// -------------------------------------------------------------------------
// Adversarial values
// -------------------------------------------------------------------------

/// Every number behind a layer surface's geometry arrives as a raw `i32`
/// off the wire. A client sending the extremes must not be able to overflow
/// the zone arithmetic, the layout or the renderer -- this is a debug build,
/// so an overflow anywhere along that path panics the test rather than
/// wrapping quietly.
#[test]
fn extreme_geometry_from_a_client_cannot_break_the_compositor() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        layer: zwlr_layer_shell_v1::Layer::Top,
        anchor: zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
        // The largest size that is representable at all -- see
        // `a_size_that_does_not_fit_i32_is_refused` for what happens past it.
        size: (i32::MAX as u32, i32::MAX as u32),
        exclusive_zone: i32::MAX,
        margin: (i32::MAX, i32::MAX, i32::MAX, i32::MAX),
        // Absurd geometry *and* a demand for every keystroke, so the focus
        // path is exercised by this one too -- it must not take the keyboard,
        // because it never attaches a buffer (see `layer_focus`).
        keyboard: KeyboardInteractivity::Exclusive,
    }));

    // Deliberately never given a buffer: an `i32::MAX`-square one cannot be
    // allocated by anyone. The geometry still reaches `arrange`, the zone it
    // leaves still reaches the core, and the frame is still rendered.
    let usable = fixture.usable();
    assert_eq!(
        usable,
        usable.intersection(WHOLE),
        "the usable area escaped the output"
    );
    let rect = fixture.window_rect();
    assert!(rect.w >= 1 && rect.h >= 1, "{rect:?}");
    let pixels = fixture.render();
    assert_eq!(pixels.len(), (CANVAS * CANVAS * 4) as usize);
    // Pointer hit-testing walks the same geometry, adding the layer's own
    // location to a surface-local point: with the location saturated near
    // `i32::MAX`, a plain `i32` add there would panic in this build.
    for (x, y) in [(0.0, 0.0), (1.0, 1.0), (99.0, 99.0), (199.0, 199.0)] {
        let _ = fixture.state.surface_under((x, y).into());
        let _ = fixture
            .state
            .layer_surface_under(&ABOVE_WINDOWS, (x, y).into());
    }
    // Still serving afterwards: another layer surface is created, arranged
    // and configured normally, and the zone stays inside the output. (It is
    // deliberately left without a buffer: the extreme surface above has
    // already reserved everything, so this one is configured to zero height
    // and there is no buffer to make for it.)
    fixture.run(Step::CreateLayer(LayerSpec::bar(10)));
    let usable = fixture.usable();
    assert_eq!(usable, usable.intersection(WHOLE));
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
}

/// `set_size` takes two `uint`s, and the pinned Smithay rev converts them
/// with a bare `as i32` into a `Size` whose constructor `debug_assert!`s on
/// negative dimensions -- so before `dispatch.rs`'s
/// `reject_unrepresentable_layer_size` guard, this exact request panicked the
/// whole compositor (found by running an earlier version of the test above,
/// which used `u32::MAX`). The offending client must be the only casualty.
#[test]
fn a_size_that_does_not_fit_i32_is_refused_without_taking_the_compositor_down() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();

    let error = fixture.run_expecting_disconnect(Step::CreateLayer(LayerSpec {
        size: (u32::MAX, u32::MAX),
        ..LayerSpec::bar(30)
    }));

    // Refused with the protocol's own vocabulary -- `invalid_size` (code 1)
    // on the object that asked -- rather than by dropping the connection.
    // wayland-backend renders a protocol error as "Protocol error {code} on
    // object {interface}@{id}: {message}", so both halves are in the string
    // the client thread returned.
    assert!(
        error.contains("Protocol error 1 "),
        "the error should be invalid_size (1): {error}"
    );
    assert!(
        error.contains("zwlr_layer_surface_v1"),
        "the error should name the offending object: {error}"
    );

    // The compositor is still here, still laying out, still drawing. The
    // dead client's window went with it, so `before` is only used to prove
    // there *was* a working layout to lose.
    assert_ne!(before.w, 0);
    assert_eq!(fixture.usable(), WHOLE);
    assert!(fixture.state.world.arrange().placements.is_empty());
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "nothing drawn");
    assert_pixel(
        &pixels,
        CANVAS / 2,
        CANVAS / 2,
        BACKGROUND_BGRA,
        "the window is gone with its client",
    );
}
