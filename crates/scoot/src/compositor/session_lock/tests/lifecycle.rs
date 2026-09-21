//! Locking, unlocking, relocking, and when `locked` may be sent.
//!
//! Plus the edges: an empty session, an output that resizes under a lock
//! surface, and the global itself.

use super::*;

/// Lock/unlock cycles in quick succession, mapping a surface every round:
/// the ack record grows with live surfaces and the session comes back
/// clean every time.
#[test]
fn rapid_lock_unlock_cycles_hold() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    for round in 0..5 {
        fixture.run(Step::Lock);
        fixture.run(Step::map_lock_surface(round));
        assert_whole_screen_is(
            &fixture.render(),
            LOCK_BGRA,
            &format!("round {round}: the lock surface"),
        );
        // Each `unlock_and_destroy` consumes its lock object, so the next
        // round uses the next one the client made.
        fixture.run(Step::Unlock { lock: round });
        assert!(
            !fixture.state.session_lock.is_locked(),
            "round {round}: the session should be unlocked"
        );
    }
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after the cycles"
    );
}

/// A clean unlock followed by a fresh lock from a different client: the
/// ordinary relock, distinct from the abandoned-lock takeover. The previous
/// locker's ack record must not leak into the new lock's validation.
#[test]
fn unlock_then_relock_with_a_new_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    fixture.run_on(second, Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the new client's lock surface");
    let report = fixture.report_of(second);
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);

    fixture.run_on(second, Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after the second unlock"
    );
}

/// ...and the lock is still a real lock afterwards: the client that destroyed
/// its own surface still owns the session, can still unlock it, and the
/// orphaned `wl_surface` it kept alive does not survive that unlock.
#[test]
fn a_lock_whose_surfaces_role_was_destroyed_still_unlocks() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.tick(Duration::from_millis(120));

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    assert!(
        fixture.state.session_lock.surfaces.is_empty(),
        "the orphaned surface must not outlive the lock that made it"
    );
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the destroyed lock surface must not come back with the session"
    );
}

/// A button held down installs an implicit pointer grab (Smithay's
/// `DefaultGrab` sets a `ClickGrab` on every press), and a grab outlives focus
/// changes by design. So every lock transition has to drop it explicitly --
/// including the one that only destroys a role object, which is the shape the
/// reviewer's "hold the lock, click, drag, destroy the surface" sequence takes.
#[test]
fn a_lock_transition_drops_a_grab_a_click_left_behind() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.settle();
    assert!(
        fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .is_grabbed(),
        "a press with no release leaves the implicit click grab installed"
    );

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.settle();
    assert!(
        !fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .is_grabbed(),
        "the grab must not survive a lock transition"
    );
}

/// Unlocking puts the session back exactly as it was, including the window
/// the client never redrew (it got no frame callbacks while locked).
#[test]
fn unlocking_brings_the_session_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the lock surface must be gone after unlocking"
    );
}

// -- the `locked` event ---------------------------------------------------

/// `locked` may not be sent before a blanked frame exists, because a client
/// that suspends the machine on `locked` would otherwise race an unlocked
/// frame onto the screen.
///
/// Driven by taking the render target away rather than by timing: with no
/// backend, `render()` returns before drawing anything, which is exactly the
/// "no blanked frame yet" state, and no amount of dispatching can accidentally
/// satisfy it.
#[test]
fn the_locked_event_waits_for_a_blanked_frame() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let backend = fixture.state.take_primary_backend().expect("a backend");

    fixture.send_step(0, Step::Lock);
    // The client's own `Step::Lock` gives up on its five-second deadline
    // without either event, which is what this asserts: with no backend there
    // is no blanked frame, so the lock cannot be confirmed.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
        if fixture.state.session_lock.is_locked() {
            break;
        }
    }
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session should be locked as soon as the request arrives"
    );
    // Several frame ticks' worth of dispatching, with no render target: the
    // client must still be waiting.
    for _ in 0..20 {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the lock must not be confirmed before a frame has been drawn"
    );

    fixture.state.put_primary_backend(backend);
    fixture.render();
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the first drawn frame should confirm the lock"
    );
    // And now the client's own `Lock` step can finish.
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client should have been told `locked`"
    );
    assert_eq!(report.finished, 0);
}

/// A second lock request while a live client holds the lock is refused with
/// `finished`, not granted -- two clients must never both believe they own
/// the session.
#[test]
fn a_second_lock_is_refused_while_one_is_held() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.render();
    assert_eq!(fixture.report().locked, 1);

    fixture.run(Step::Lock);
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the second lock must not be granted");
    assert_eq!(
        report.finished, 1,
        "the second lock must be told `finished`"
    );
    assert!(fixture.state.session_lock.is_locked());
}

// -- edges ----------------------------------------------------------------

/// Zero windows, zero lock surfaces: the screen still blanks, and stays
/// blanked across repeated lock/unlock cycles. The repeat is the point --
/// under `--tty` the damage tracker is what decides whether the framebuffer
/// is repainted at all, and a blank drawn only by the clear colour could
/// legitimately report no damage on the second cycle.
#[test]
fn locking_an_empty_session_blanks_and_keeps_blanking() {
    let mut fixture = Fixture::new();
    for round in 0..3 {
        fixture.run(Step::Lock);
        let locked = fixture.render();
        assert_whole_screen_is(&locked, BLACK_BGRA, "a locked empty session");
        // The lock object is destroyed by `unlock_and_destroy`, so each round
        // needs the next one the client made.
        fixture.run(Step::Unlock { lock: round });
        let unlocked = fixture.render();
        assert_whole_screen_is(
            &unlocked,
            BACKGROUND_BGRA,
            &format!("round {round}: the unlocked empty session"),
        );
    }
}

/// Resizing the output while locked reconfigures the lock surfaces. Without
/// that, the client's next commit is a `dimensions_mismatch` protocol error --
/// i.e. a killed lock client on a locked session.
#[test]
fn resizing_the_output_reconfigures_the_lock_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.state.resize_output(CANVAS * 2, CANVAS * 2);
    fixture.settle();
    // The client acks and redraws at whatever size it was told; a mismatch
    // would have disconnected it, which `report` reports as a dead client.
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert!(fixture.state.session_lock.is_locked());
}

/// `wl_output` is what a lock client names its surface's screen with, and a
/// compositor that never advertised the manager global would leave a locker
/// with nothing to bind. Cheap, but it is the one thing every other test here
/// takes for granted.
#[test]
fn the_manager_global_is_advertised() {
    let mut fixture = Fixture::new();
    // `run_client` fails to start at all without it, so reaching a step at
    // all proves it -- asserted explicitly so a regression names itself.
    fixture.run(Step::MapWindow);
    assert!(!fixture.state.session_lock.is_locked());
}
