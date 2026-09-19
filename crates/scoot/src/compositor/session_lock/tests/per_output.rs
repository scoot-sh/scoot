//! One live lock surface per physical output.
//!
//! The protocol allows exactly one lock surface per output: "Attempting to
//! create more than one lock surface for a given output is a
//! `duplicate_output` protocol error." Smithay enforces that per `wl_output`
//! *resource* (`locked_outputs.contains(&output)` in its `GetLockSurface`
//! handler -- naming the same bind twice dies there, while naming the same
//! physical output through a second bind of the global is admitted), so
//! scoot enforces it per physical `Output` in `new_surface`: a second
//! *live* surface for an output the lock already covers is refused with the
//! protocol's own `duplicate_output` error, which kills the offending client
//! the way every other protocol error here does.
//!
//! Live, not sticky: destroying the surface -- role *and* `wl_surface`, so
//! the compositor forgets it -- frees the output for a rebuild. A locker
//! reconstructing its UI must not die for replacing a surface it tore down;
//! what stays refused is two surfaces alive at once, whichever binds named
//! the output. (Same-resource destroy-then-rebuild still dies inside
//! Smithay's own never-shrinking `locked_outputs` list before scoot is ever
//! asked: a pre-existing Smithay-side stickiness this item does not touch.)
//!
//! With exactly one output the rule collapses to "one live lock surface":
//! the first blanked frame confirms whatever the surface state (see
//! `blanking`), a resize reaches the one surface, and the multi-output
//! revisit lives in `outputs.rs`'s `Outputs::primary` doc.

use super::*;

/// A second live surface for the output the lock already covers -- named
/// through a different `wl_output` bind, the shape Smithay's
/// resource-identity guard admits -- is refused with the protocol's own
/// `duplicate_output` error: the client dies, the session stays locked, and
/// the compositor keeps serving.
#[test]
fn a_second_live_surface_for_the_same_output_is_refused() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));

    let error = fixture.run_expecting_disconnect(Step::LockSurfaceSecondBind {
        lock: 0,
        color: Some(LOCK_BGRA),
    });
    // The refusal is the protocol's own `duplicate_output` (code 3) posted
    // on the lock object -- the client's own backend reports it as
    // `Protocol error 3 on object ext_session_lock_v1@N: ...`, so this pins
    // the exact error, not just "the client is dead". (The object number
    // varies run to run and is deliberately not pinned.)
    assert!(
        error.contains("Protocol error 3 on object ext_session_lock_v1@")
            && error.contains("Output already has a lock surface"),
        "the client should die refused with duplicate_output: {error}"
    );
    // ...and only the client: the session stays locked and the compositor
    // keeps serving, which is what makes this a client kill rather than a
    // compositor crash.
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker must not unlock the session"
    );
    fixture.render();
}

/// Destroying the surface frees the output: role and `wl_surface` both gone,
/// so the compositor forgets it, and putting a replacement up through the
/// second bind -- the rebuild shape a locker uses after an output hotplug --
/// is admitted, configured, and drawn like any first surface.
#[test]
fn destroying_the_surface_frees_the_output_for_a_rebuild() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::DestroyLockSurface { index: 0 });

    fixture.run(Step::LockSurfaceSecondBind {
        lock: 0,
        color: Some(LOCK_BGRA),
    });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the rebuilt surface must not have killed the client"
    );
    assert_eq!(
        report.keyboard_focus,
        Some(Which::Lock(1)),
        "the rebuilt surface holds the keyboard"
    );
    let pixels = fixture.render();
    assert_whole_screen_is(
        &pixels,
        LOCK_BGRA,
        "the rebuilt lock surface draws whole-screen",
    );
}
