//! The first click on a fresh lock screen, before the mouse has moved.
//!
//! `new_surface` re-derives pointer focus while the lock surface is still
//! unmapped, so the hit test finds nothing; without a re-derivation on the
//! commit that maps it, `wl_pointer.enter` only arrives on the first mouse
//! move and the click before that reaches nobody. Every assertion here is
//! made from what the *client* was told, never from a field inside the
//! compositor.

use super::*;

/// The bug itself: mapping a lock surface must deliver `wl_pointer.enter`
/// to it at map-commit time, with the pointer never having moved -- and the
/// first click after that must reach it.
#[test]
fn pointer_enter_reaches_the_lock_surface_at_map_time_without_any_pointer_movement() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    // Deliberately no `pointer_move` anywhere in this test: the pointer has
    // never moved, so any `enter` the lock surface gets can only have come
    // from the map commit itself.
    assert_eq!(
        fixture.report().pointer_focus,
        None,
        "nothing should hold pointer focus before the lock surface maps"
    );

    fixture.run(Step::map_lock_surface(0));
    let mapped = fixture.report();
    assert_eq!(
        mapped.pointer_focus,
        Some(Which::Lock(0)),
        "mapping the lock surface must enter it under the unmoved pointer"
    );

    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let clicked = fixture.report();
    assert_eq!(
        clicked.pointer_focus,
        Some(Which::Lock(0)),
        "focus must not have moved off the lock surface"
    );
    assert_eq!(
        clicked.buttons,
        mapped.buttons + 2,
        "the first click, before any mouse movement, must reach the lock surface"
    );
}

/// The other half of the same recognition: a lock surface that acked its
/// configure but attached no buffer is unmapped, and must not hold pointer
/// focus until it maps.
#[test]
fn an_unmapped_lock_surface_holds_no_pointer_focus() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurface {
        lock: 0,
        color: None,
    });
    assert_eq!(
        fixture.report().pointer_focus,
        None,
        "a lock surface with no buffer must not take pointer focus before it maps"
    );
}

/// The symmetric case in reverse: unlocking must hand pointer focus back to
/// the window underneath without a mouse move. `unlock` already re-derives,
/// so this pins the behaviour rather than fixing it.
#[test]
fn pointer_focus_returns_to_the_window_on_unlock_without_any_pointer_movement() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    assert_eq!(
        fixture.report().pointer_focus,
        Some(Which::Window(0)),
        "the pointer should be over the window before the lock"
    );

    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::Unlock { lock: 0 });
    // No `pointer_move` since the one before the lock: the enter below can
    // only have come from the unlock itself.
    assert_eq!(
        fixture.report().pointer_focus,
        Some(Which::Window(0)),
        "unlocking must return pointer focus to the window underneath"
    );
}

/// Precedence: an `overlay` layer surface mapped before the lock must not
/// steal the map-commit enter -- while locked the hit test sees lock
/// surfaces only.
#[test]
fn an_overlay_layer_surface_does_not_steal_the_map_commit_enter() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapOverlayLayer);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    assert_eq!(
        fixture.report().pointer_focus,
        Some(Which::Lock(0)),
        "the lock surface must win pointer focus over an overlay layer surface"
    );
}
