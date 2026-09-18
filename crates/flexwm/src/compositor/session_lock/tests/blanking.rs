//! What is on screen while the session is locked.
//!
//! The headline guarantee of the whole protocol, and the one that is only a
//! claim about *pixels*: a test asserting on which enum variant the render
//! path chose would pass just as happily against a version that drew the
//! window behind a transparent backdrop.

use super::*;

/// The headline guarantee: once locked, a window that was on screen is not
/// merely covered, it is not drawn at all -- and neither is the focus ring,
/// nor the configured desktop background, which is what an unlocked frame
/// clears to.
#[test]
fn locking_blanks_a_window_off_the_screen() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.render();
    assert!(
        contains(&before, WINDOW_BGRA),
        "the window should be on screen before the lock"
    );

    fixture.run(Step::Lock);
    let after = fixture.render();
    assert_whole_screen_is(
        &after,
        BLACK_BGRA,
        "a locked frame with no lock surface yet",
    );
}

/// The motivating case for this whole protocol: a layer-shell surface on the
/// `overlay` layer draws in front of every window, so "the lock screen is
/// drawn last" would not be enough -- it has to not be drawn at all.
#[test]
fn locking_blanks_layer_surfaces_too() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapOverlayLayer);
    let before = fixture.render();
    assert!(
        contains(&before, OVERLAY_BGRA),
        "the overlay layer surface should be on screen before the lock"
    );

    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let after = fixture.render();
    assert_whole_screen_is(&after, LOCK_BGRA, "a locked frame over an overlay layer");
}

/// The same, with a lock surface that has drawn: its pixels, and only its
/// pixels.
#[test]
fn only_the_lock_surface_is_drawn_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "a locked frame with a lock surface");
}

/// A lock surface that has acked its configure but never attached a buffer
/// must not leave whatever was underneath showing through the gap.
#[test]
fn a_lock_surface_with_no_buffer_yet_shows_the_backdrop() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurface {
        lock: 0,
        color: None,
    });
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, BLACK_BGRA, "a lock surface with no buffer");
}

/// The lock sequence, in order: input is captured the moment the request is
/// accepted, the framebuffer blanks on the first frame after that, and
/// `locked` goes out only once the blanked frame exists.
///
/// This is the ordering the blank-timing ticket asked about
/// (`docs/backlog/protocols/session-lock-blank-timing.md`): there *is* a
/// window in which the desktop is still in the framebuffer while input is
/// already locked -- between the accept and the first frame -- and that
/// direction is deliberate. The reverse (blank pixels on screen while
/// keystrokes still reach the window underneath) would leak the password
/// into the unlocked session, and delaying the blank until the locker draws
/// (the niri shape, up to a second) would keep rendering the unlocked
/// session for that whole wait.
///
/// Driven with the render target taken away so no frame tick can blank or
/// confirm early: after the accept, the stale framebuffer still holds the
/// desktop, nothing is confirmed, and both focuses have already left the
/// window.
#[test]
fn lock_captures_input_before_the_first_blanked_frame_confirms_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // The pointer over the window's own buffer, so "the pointer left it"
    // means something.
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    let before = fixture.report();
    assert_eq!(
        before.keyboard_focus,
        Some(Which::Window(0)),
        "the window should have the keyboard before the lock"
    );
    assert_eq!(
        before.pointer_focus,
        Some(Which::Window(0)),
        "the pointer should be over the window before the lock"
    );
    assert!(
        contains(&fixture.render(), WINDOW_BGRA),
        "the window should be on screen before the lock"
    );

    // No render target: the accept runs, but no frame can blank the screen
    // or confirm the lock.
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session locks as soon as the request arrives"
    );
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "with no frame drawn, the blanked frame is still outstanding"
    );

    // Input already captured, before any pixel moved.
    let accepted = fixture.report();
    assert_eq!(
        accepted.keyboard_focus, None,
        "the window must lose the keyboard at accept, before the first blanked frame"
    );
    assert_eq!(
        accepted.pointer_focus, None,
        "the pointer must leave the window at accept, before the first blanked frame"
    );
    assert_eq!(
        accepted.locked, 0,
        "the client must not hear `locked` before a blanked frame exists"
    );
    assert_eq!(accepted.finished, 0);

    // The framebuffer still holds the desktop: the accepted-but-unconfirmed
    // window, read back without rendering anything new.
    fixture.state.backend = Some(backend);
    let stale = fixture.pixels();
    assert!(
        contains(&stale, WINDOW_BGRA),
        "no frame has blanked the screen yet, so the desktop pixels are still there"
    );

    // The first frame blanks, and only then confirms.
    let blanked = fixture.render();
    assert_whole_screen_is(
        &blanked,
        BLACK_BGRA,
        "the first frame after the accept blanks the screen",
    );
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the first drawn frame should confirm the lock"
    );
    let confirmed = fixture.report();
    assert_eq!(
        confirmed.locked, 1,
        "the client should have been told `locked`, but only after the blank"
    );
    assert_eq!(confirmed.finished, 0);
}

/// The protocol's own words: "If a lock surface on an active output is
/// destroyed before the ext_session_lock_v1.unlock_and_destroy event is sent,
/// the compositor must fall back to rendering a solid color."
///
/// Asserted on the frame the compositor drew *by itself* -- [`Fixture::tick`]
/// then [`Fixture::pixels`], never [`Fixture::render`] -- because "falls back"
/// is a claim about what is on the display, and a fallback that waited for an
/// unrelated pointer motion to mark the screen dirty would pass a
/// `render()`-based test while leaving the destroyed surface's pixels up.
#[test]
fn destroying_a_lock_surface_falls_back_to_a_solid_color() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    assert!(contains(&fixture.render(), LOCK_BGRA));

    fixture.run(Step::DestroyLockSurface { index: 0 });
    fixture.tick(Duration::from_millis(120));
    let pixels = fixture.pixels();
    assert_whole_screen_is(&pixels, BLACK_BGRA, "after the lock surface was destroyed");
}

/// The same protocol sentence, for the destruction that reaches no other hook:
/// only the `ext_session_lock_surface_v1` role object goes, and the
/// `wl_surface` under it, the lock and the connection all stay.
///
/// That is legal, and it is what a locker does when an output is removed under
/// it -- so "the compositor must fall back to rendering a solid color" applies
/// with nothing else changing to prompt a redraw. Before `dispatch.rs`'s
/// `destroyed` hook existed this left the destroyed surface's last frame on
/// screen indefinitely: Smithay resets the surface's `last_acked` (so it stops
/// producing render elements) but nothing asked for the frame that would show
/// it gone, and no `wl_surface` or lock object destruction ran to ask on its
/// behalf.
#[test]
fn destroying_only_the_lock_surfaces_role_falls_back_without_waiting_for_damage() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    assert_whole_screen_is(&fixture.render(), LOCK_BGRA, "the lock surface");

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    // A whole frame period with nothing else going on: no pointer motion, no
    // commit, no colour change -- the damage a stale frame would otherwise be
    // waiting for.
    fixture.tick(Duration::from_millis(120));
    let unasked = fixture.pixels();
    assert!(
        fixture.state.session_lock.is_locked() && !fixture.state.session_lock.abandoned(),
        "the lock itself is untouched: only its surface's role object went"
    );
    assert_whole_screen_is(
        &unasked,
        BLACK_BGRA,
        "the frame the compositor drew by itself after the role object was destroyed",
    );
}
