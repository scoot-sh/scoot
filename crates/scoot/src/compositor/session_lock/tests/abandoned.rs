//! A lock with no live client behind it.
//!
//! Two ways to get there, and they are not the same: the client *died* while
//! holding a confirmed lock, or it gave the lock up on purpose with
//! `ext_session_lock_v1.destroy` before `locked` was ever sent -- which is
//! legal, and which independent review found a critical hole in.

use super::*;

/// The single most safety-critical behavior here: a dead lock client is not
/// evidence that the user wants their screen unlocked.
#[test]
fn the_session_stays_locked_when_the_lock_client_dies() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.disconnect(0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session must stay locked when the lock client disconnects"
    );
    assert!(
        fixture.state.session_lock.abandoned(),
        "and it must read as abandoned"
    );
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, RED_BGRA, "an abandoned lock");
}

/// ...and it has to turn red *by itself*, with nothing else prompting a
/// redraw.
///
/// This is the case real `--tty` hardware caught and the other tests here
/// missed: a client that destroyed its lock surface and *then* died leaves no
/// surface destruction to hang a redraw off, so the last frame drawn -- black
/// -- stayed on the display, and the user had no way to tell their locker had
/// crashed. Deliberately reads the framebuffer without rendering first
/// (`pixels`, not `render`), because "the test asked for a frame" is exactly
/// what hid it.
#[test]
fn an_abandoned_lock_turns_red_without_being_asked_to_redraw() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // The locker is a *separate* client from the one that owns the window,
    // which is both how a real session is arranged and what makes this test
    // non-vacuous: when one client owns both, its disconnect also destroys
    // the window, and `remove_window` asks for a redraw as a side effect --
    // masking the missing one entirely. (Written the other way first; the
    // negative control below passed, which is how that was found.)
    let locker = fixture.connect();
    fixture.run_on(locker, Step::Lock);
    fixture.run_on(locker, Step::map_lock_surface(0));
    fixture.render();
    fixture.run_on(locker, Step::DestroyLockSurface { index: 0 });
    assert_whole_screen_is(
        &fixture.render(),
        BLACK_BGRA,
        "a live lock with no surface left",
    );

    fixture.disconnect(locker);
    fixture.tick(Duration::from_millis(200));
    assert_whole_screen_is(
        &fixture.pixels(),
        RED_BGRA,
        "an abandoned lock, redrawn without anyone asking",
    );
}

/// The recovery path that stops a crashed locker being a permanently unusable
/// session: a new client takes the lock over, is confirmed immediately (the
/// outputs are already blank), and can unlock after authenticating.
#[test]
fn a_new_client_can_take_over_an_abandoned_lock_and_unlock() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.disconnect(0);
    assert!(fixture.state.session_lock.abandoned());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked, 1,
        "a new client must be able to take over an abandoned lock"
    );
    assert_eq!(report.finished, 0);
    assert!(
        !fixture.state.session_lock.abandoned(),
        "the lock now has a live owner again"
    );
    // Nothing of the dead client's is on screen any more, and the backdrop is
    // back to the ordinary locked black.
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "after the takeover");

    fixture.run_on(second, Step::map_lock_surface(0));
    assert_whole_screen_is(
        &fixture.render(),
        LOCK_BGRA,
        "the new client's lock surface",
    );

    fixture.run_on(second, Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    // The first client -- and with it its window -- is gone, so what an
    // unlocked session shows here is the ordinary desktop background. The
    // assertion that matters is that it is *not* the lock screen any more.
    assert_whole_screen_is(
        &fixture.render(),
        BACKGROUND_BGRA,
        "the session after the takeover unlocked it",
    );
}

/// A lock client that dies before its lock was ever confirmed must not leave
/// the session stuck refusing every later locker.
#[test]
fn a_replacement_locker_is_accepted_after_one_died_unconfirmed() {
    let mut fixture = Fixture::new();
    // No render between the lock and the disconnect, so the first lock is
    // still pending confirmation when its client goes away.
    fixture.send_step(0, Step::Lock);
    let _ = fixture.wait_for_ack(0);
    fixture.disconnect(0);
    assert!(fixture.state.session_lock.is_locked());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    fixture.render();
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked + report.finished,
        1,
        "exactly one of the two events must arrive"
    );
    assert_eq!(
        report.locked, 1,
        "a replacement locker must not be refused because a dead one is still pending"
    );
}

// -- a lock its own client gave up ----------------------------------------
//
// The nastiest state this module has, and the one independent review found a
// critical hole in: `ext_session_lock_v1.destroy` is *legal* before `locked`
// has been sent (Smithay's `lock.rs` refuses it only while its own
// `LockStatus` says this object holds the lock, and that stays `Unlocked`
// until the confirmation actually runs). A client that uses it keeps
// everything a dying client loses -- its connection, and the `wl_surface`
// under every lock surface it made -- so "is the surface alive" stops being
// the same question as "does this surface still belong to the lock that owns
// the session". Every test below was a live reproduction before it was a
// regression test; see this module's "Which lock surfaces count".

/// The lock is given up with a *mapped* surface still up. The abandoned screen
/// has to be the red indicator, not the former locker's own pixels -- showing
/// those is worse than useless: a user looking at what appears to be a working
/// lock screen has no way to tell that their real locker never took effect.
#[test]
fn a_lock_given_up_with_a_surface_up_still_shows_the_abandoned_screen() {
    let mut fixture = Fixture::new();
    // No render target, so the lock is accepted but never confirmed -- which
    // is exactly (and only) the window in which `destroy` is legal.
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    let pixels = fixture.render();
    assert!(
        fixture.state.session_lock.abandoned(),
        "the lock reads as abandoned"
    );
    assert_whole_screen_is(&pixels, RED_BGRA, "the documented abandoned screen");
}

/// ...and the same client stops receiving input the moment it gives the lock
/// up, without waiting for anyone to replace it.
///
/// Both halves are asserted from the wire, and the pointer half is the one
/// that a hit-test-only fix would miss: `wl_pointer.button` goes to whatever
/// the pointer last *entered*, so a surface that keeps pointer focus keeps
/// receiving clicks however the hit test answers.
#[test]
fn a_lock_given_up_takes_the_keyboard_and_the_pointer_with_it() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    let held = fixture.report();
    assert_eq!(
        held.keyboard_focus,
        Some(Which::Lock(0)),
        "the locker holds the keyboard while its lock is live"
    );
    assert_eq!(held.pointer_focus, Some(Which::Lock(0)), "and the pointer");

    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    fixture.state.type_text("password").expect("typed text");
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.keyboard_focus, None,
        "a client that gave up its lock must not still hold the keyboard \
         (before={held:?} after={after:?})"
    );
    assert_eq!(after.pointer_focus, None, "nor the pointer");
    assert_eq!(
        after.keys, held.keys,
        "and no keystroke may reach it (before={held:?} after={after:?})"
    );
    assert_eq!(after.buttons, held.buttons, "nor any click");
}

/// The surface must not survive a takeover either: the replacement locker's
/// screen is its own, and so is the keyboard.
///
/// Distinct from the test above rather than a stronger version of it, because
/// the two fail to different fixes. During the abandoned-but-not-yet-replaced
/// phase above, the stale surface's lock *is* still `owner`, so an ownership
/// check alone passes it; here it is not, so a liveness check alone passes it.
/// Only asking both questions closes both.
#[test]
fn a_surface_from_a_given_up_lock_does_not_survive_a_takeover() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    fixture.render();
    assert!(fixture.state.session_lock.abandoned());

    let b = fixture.connect();
    fixture.run_on(b, Step::Lock);
    fixture.run_on(b, Step::map_lock_surface(0));
    let pixels = fixture.render();
    let report_b = fixture.report_of(b);
    let report_a = fixture.report();
    assert_eq!(
        report_b.locked, 1,
        "the new locker was told it holds the lock"
    );
    assert_eq!(
        report_a.keyboard_focus, None,
        "the client that gave up its lock must not still hold the keyboard \
         (A={report_a:?} B={report_b:?})"
    );
    assert_eq!(
        report_a.pointer_focus, None,
        "nor the pointer (A={report_a:?} B={report_b:?})"
    );
    assert_eq!(
        report_b.keyboard_focus,
        Some(Which::Lock(0)),
        "the new locker must hold the keyboard (A={report_a:?} B={report_b:?})"
    );
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the new locker's own surface");
}

/// The whole attack, from a client that needs no race, no crash and no
/// test-only surgery: `lock`, `get_lock_surface`, `destroy`, in one batch.
///
/// Before the ownership filter existed, that left a surface registered
/// forever, and the *next* locker to run -- the user's real one -- put its
/// screen up while this client kept the keyboard. Every key of the password
/// went to the attacker, and the locker that never saw them could never
/// authenticate and so could never unlock.
#[test]
fn no_client_can_steal_the_lock_screen_by_giving_up_a_lock() {
    let mut fixture = Fixture::new();
    fixture.run(Step::AttackLock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the hostile client locked the session"
    );
    assert!(
        fixture.state.session_lock.abandoned(),
        "...and immediately gave the lock up"
    );
    // Its lock surface was never acked and never drew, so this is also the
    // unmapped half of the abandoned-screen check.
    assert_whole_screen_is(
        &fixture.render(),
        RED_BGRA,
        "an abandoned lock whose surface never drew",
    );

    // The user's real locker comes along and takes over.
    let real = fixture.connect();
    fixture.run_on(real, Step::Lock);
    fixture.run_on(real, Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the real locker's screen");

    // Typed *after* the real lock screen is up: this is the password.
    fixture.state.type_text("password").expect("typed text");
    fixture.settle();
    let attacker = fixture.report();
    let locker = fixture.report_of(real);
    assert_eq!(
        attacker.keyboard_focus, None,
        "a client that holds no lock must not hold the lock screen's keyboard \
         (attacker={attacker:?} locker={locker:?})"
    );
    assert_eq!(
        attacker.keys, 0,
        "and must not have been sent a single keystroke \
         (attacker={attacker:?} locker={locker:?})"
    );
    assert_eq!(
        locker.keyboard_focus,
        Some(Which::Lock(0)),
        "the real locker must hold the keyboard (attacker={attacker:?} locker={locker:?})"
    );
    assert!(
        locker.keys > 0,
        "and must be the one that received the keystrokes \
         (attacker={attacker:?} locker={locker:?})"
    );
}

/// A takeover may only be confirmed without drawing a frame when the lock it
/// replaces had actually drawn one. Replace a lock that never did, and the
/// unlocked session is still on the display -- telling the new client `locked`
/// there hands it exactly the guarantee the event exists to provide at exactly
/// the moment it is false.
///
/// The first lock is left unconfirmed by taking the render target away, so no
/// timing luck is involved: with no backend no frame can be drawn at all.
#[test]
fn a_takeover_waits_for_the_blanked_frame_the_replaced_lock_never_drew() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert!(
        contains(&fixture.render(), WINDOW_BGRA),
        "the window is on screen before any lock"
    );

    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.disconnect(0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session is locked, by a client that is now gone"
    );
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "...and its lock was never confirmed"
    );

    // The replacement's own `Lock` step blocks until the compositor answers,
    // and it must not answer yet, so it is sent by hand -- the same pattern
    // `the_locked_event_waits_for_a_blanked_frame` uses.
    let second = fixture.connect();
    fixture.send_step(second, Step::Lock);
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "a takeover of a lock that never drew a blanked frame must wait for one"
    );
    // Which is the entire point, stated in pixels: reading the framebuffer
    // without rendering first shows what a client told `locked` here would
    // have been told about.
    fixture.state.backend = Some(backend);
    assert!(
        contains(&fixture.pixels(), WINDOW_BGRA),
        "the unlocked session is still the last frame drawn"
    );

    // Now let the frame happen; only then may the confirmation go out.
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "the blanked frame");
    assert!(fixture.state.session_lock.pending.is_none());
    let ack = fixture.wait_for_ack(second);
    assert!(matches!(ack, Ack::Done));
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked, 1,
        "the takeover is confirmed, once it is true"
    );
    assert_eq!(report.finished, 0);
}

/// The other half of that rule, so the fix above cannot have been "never
/// fast-confirm": replacing a lock that *had* drawn its blanked frame is still
/// confirmed with no further frame, because the screen genuinely is blank and
/// stays blank across the handover.
///
/// Driven the same way round as the test above -- the render target is taken
/// away *before* the second lock, so the only way this client can be told
/// `locked` at all is without a frame.
#[test]
fn a_takeover_of_a_confirmed_lock_is_still_confirmed_immediately() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "the blanked frame");
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the first lock was confirmed by that frame"
    );
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.disconnect(0);

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "no frame is owed: the outputs were blank before this lock and are \
         blank after it"
    );
    assert_eq!(fixture.report_of(second).locked, 1);
    fixture.state.backend = Some(backend);
}
