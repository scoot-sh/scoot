//! Lock surfaces are per-output, and flexwm has one output.
//!
//! The protocol sends `locked` only once a locked frame has been presented on
//! *all* outputs, and sizes each lock surface to its own output. With exactly
//! one output both halves collapse: the first blanked frame on the one output
//! confirms the lock no matter how many surfaces exist, and every surface is
//! configured to that output's size. What lands here are the pins for the
//! halves no other test asserts: several surfaces from one lock sharing the
//! single output (each configured to it, all drawn, the first holding the
//! keyboard), and a resize reconfiguring every one of them.
//!
//! The zero-surface confirm half needs no new test: `Step::Lock` waits for
//! `locked`, so `the_locked_event_waits_for_a_blanked_frame` and
//! `locking_an_empty_session_blanks_and_keeps_blanking` already prove the
//! single output's blanked frame confirms with no surface at all. The
//! multi-output revisit lives in `headless.rs`'s `OUTPUT_ID` doc.

use super::*;

/// Every lock surface one lock puts up is configured to the single output's
/// size and drawn onto it: the first-created on top, the first holding the
/// keyboard, and the second revealed whole once the first is destroyed.
///
/// Pixel-proven, not struct-proven: a surface configured to any other size
/// would have its first commit refused with `dimensions_mismatch` (the client
/// dead, its surface gone), so two fullscreen draws in creation order plus
/// the keyboard on the first is exactly "both were configured to this
/// output".
#[test]
fn every_lock_surface_is_configured_to_the_single_output() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::LockSurfaceSecondBind {
        lock: 0,
        color: Some(WINDOW_BGRA),
    });
    let pixels = fixture.render();
    assert_whole_screen_is(
        &pixels,
        LOCK_BGRA,
        "the first-created lock surface draws on top",
    );
    let report = fixture.report();
    assert_eq!(
        report.keyboard_focus,
        Some(Which::Lock(0)),
        "the first current surface holds the keyboard"
    );

    // The second surface was composited all along underneath: destroying the
    // first reveals it whole, not the backdrop.
    fixture.run(Step::DestroyLockSurface { index: 0 });
    fixture.tick(Duration::from_millis(120));
    let pixels = fixture.pixels();
    assert_whole_screen_is(
        &pixels,
        WINDOW_BGRA,
        "the second lock surface, revealed by destroying the first",
    );
    let report = fixture.report();
    assert_eq!(
        report.keyboard_focus,
        Some(Which::Lock(1)),
        "the keyboard falls to the surviving surface"
    );
}

/// `configure_all` reaches every surface, not just the first.
///
/// Two halves. The pixel half: after a shrink both surfaces ack the resize's
/// configure and redraw at it (a surface naming any other size is killed for
/// a mismatch, so two live redraws plus `locked: 1` is the wire proof both
/// were reconfigured). The iteration half is white-box on purpose: the
/// harness readback covers the old canvas, so "the *second* surface was
/// reconfigured" is asserted on the pending sizes `configure_all` wrote --
/// a surface it missed would still name the old size here.
#[test]
fn resizing_the_output_reconfigures_every_lock_surface() {
    const SMALL: i32 = CANVAS / 2;
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::LockSurfaceSecondBind {
        lock: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.state.resize_output(SMALL, SMALL);
    fixture.settle();
    for (index, surface) in fixture.state.session_lock.surfaces.iter().enumerate() {
        let size = surface.with_pending_state(|state| state.size);
        assert_eq!(
            size,
            Some((SMALL as u32, SMALL as u32).into()),
            "surface {index} should be reconfigured to the resized output"
        );
    }
    // Each acks the resize's configure and redraws at it; a surface the
    // resize never reconfigured still names the old size, which no longer
    // matches and kills the client on commit.
    fixture.run(Step::RedrawLockSurface { index: 0 });
    fixture.run(Step::RedrawLockSurface { index: 1 });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the redraws at the reconfigured size must not have killed the client"
    );
    let pixels = fixture.render();
    assert_eq!(
        test_support::pixel(&pixels, CANVAS, 10, 10),
        LOCK_BGRA,
        "both surfaces redrew at the resized output"
    );
}
