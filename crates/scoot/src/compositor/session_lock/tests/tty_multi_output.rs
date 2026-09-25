//! `locked` under `--tty` with more than one output (milestone 19, phase E).
//!
//! Phase C made `locked` wait for every output's blanked frame, but its
//! `--tty` branch confirmed on the first matching vblank -- safe only while
//! `--tty` drove one screen. With one head per connector each head numbers
//! its own flips from zero, so a bare sequence number cannot say *which*
//! screen reached scanout: head 1's flip 0 completing would have confirmed a
//! lock whose blanked frame on head 2 (also flip 0) was still in flight,
//! leaving the second screen showing the live desktop after `locked` went
//! out. That is the exact failure the vblank-confirmation work exists to
//! prevent, now across screens.
//!
//! These drive the same `State` methods the DRM event handler and the
//! fallback timer call, with synthetic flips per output and synthetic time,
//! on a two-output headless harness whose render targets are taken away so
//! nothing can confirm the headless way underneath the test.

use std::time::{Duration, Instant};

use scoot_core::OutputId;

use super::*;
use crate::compositor::render::Backend;
use crate::compositor::session_lock::LOCK_VBLANK_TIMEOUT;

const FIRST: OutputId = OutputId(1);
const SECOND: OutputId = OutputId(2);
/// The second output's index in the client's `wl_output` registry.
const SECOND_INDEX: usize = 1;

/// A two-output compositor, one client, and a lock pending with no render
/// target anywhere -- so only the vblank path under test can confirm it.
fn pending_two_output_lock() -> (Fixture, Vec<(OutputId, Backend)>) {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    fixture.connect();
    let taken = [FIRST, SECOND]
        .into_iter()
        .map(|id| (id, fixture.state.take_backend(id).expect("a backend")))
        .collect();
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    assert!(fixture.state.session_lock.awaiting_blank());
    (fixture, taken)
}

fn put_back(fixture: &mut Fixture, taken: Vec<(OutputId, Backend)>) {
    for (id, backend) in taken {
        fixture.state.put_backend(id, backend);
    }
}

/// The fail-first pin for the whole phase: both heads issue flip 0 for their
/// blanked frames, and head 1's completion must not confirm while head 2's
/// is still out. Before per-output waits the second `await_vblank` simply
/// overwrote the first and either completion confirmed the lock.
#[test]
fn one_screens_vblank_does_not_confirm_a_two_screen_lock() {
    let (mut fixture, taken) = pending_two_output_lock();
    let now = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(0), now);
    let _ = fixture
        .state
        .session_lock
        .await_vblank(SECOND, Some(0), now);

    fixture.state.note_flip_completed(FIRST, Some(0));
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "head 2's blanked frame is still in flight: no `locked` yet"
    );

    fixture.state.note_flip_completed(SECOND, Some(0));
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "both screens have reached scanout: the lock confirms"
    );
    put_back(&mut fixture, taken);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1, "exactly one `locked`");
}

/// A completion carries its output: head 2 completing a flip whose number
/// happens to equal head 1's recorded one confirms nothing.
#[test]
fn another_screens_flip_number_is_not_a_match() {
    let (mut fixture, taken) = pending_two_output_lock();
    let now = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(5), now);
    let _ = fixture
        .state
        .session_lock
        .await_vblank(SECOND, Some(9), now);

    fixture.state.note_flip_completed(SECOND, Some(5));
    fixture.state.note_flip_completed(FIRST, Some(9));
    assert!(
        fixture.state.session_lock.confirmed.is_empty(),
        "neither completion names its own output's blanked flip"
    );

    fixture.state.note_flip_completed(FIRST, Some(5));
    assert!(fixture.state.session_lock.awaiting_blank());
    fixture.state.note_flip_completed(SECOND, Some(9));
    assert!(!fixture.state.session_lock.awaiting_blank());
    put_back(&mut fixture, taken);
}

/// The previous frame's vblank on a screen, the ordinary race, still does
/// not count for that screen; its own flip's does.
#[test]
fn a_stale_vblank_on_one_screen_leaves_that_screen_waiting() {
    let (mut fixture, taken) = pending_two_output_lock();
    let now = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(3), now);
    let _ = fixture
        .state
        .session_lock
        .await_vblank(SECOND, Some(3), now);
    fixture.state.note_flip_completed(SECOND, Some(3));
    fixture.state.note_flip_completed(FIRST, Some(2));
    fixture.state.note_flip_completed(FIRST, None);
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "head 1's own flip has not completed"
    );
    fixture.state.note_flip_completed(FIRST, Some(3));
    assert!(!fixture.state.session_lock.awaiting_blank());
    put_back(&mut fixture, taken);
}

/// The fallback bound still confirms when completions never come -- on
/// every screen at once, not only the one that armed it.
#[test]
fn the_fallback_confirms_both_screens_when_no_vblank_arrives() {
    let (mut fixture, taken) = pending_two_output_lock();
    let t0 = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(0), t0);
    let _ = fixture.state.session_lock.await_vblank(SECOND, Some(0), t0);
    fixture.state.note_flip_completed(FIRST, Some(0));
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT - Duration::from_millis(1));
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "the bound has not passed"
    );
    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "past the bound the lock confirms rather than hang the locker"
    );
    put_back(&mut fixture, taken);
}

/// Phase C's placeholder rule holds on the `--tty` path too: a screen whose
/// admitted lock surface has not drawn yet is not recorded by its vblank --
/// the flip carried the backdrop, not the locker's blank -- and the frame
/// that draws the surface records its own.
#[test]
fn a_placeholder_screens_vblank_is_not_its_blank() {
    let (mut fixture, taken) = pending_two_output_lock();
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND_INDEX,
        color: None,
    });
    let now = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(0), now);
    let _ = fixture
        .state
        .session_lock
        .await_vblank(SECOND, Some(0), now);
    fixture.state.note_flip_completed(FIRST, Some(0));
    fixture.state.note_flip_completed(SECOND, Some(0));
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "head 2 flipped the backdrop placeholder, not the locker's surface"
    );
    assert_eq!(fixture.state.session_lock.confirmed, vec![FIRST]);

    fixture.run(Step::ReattachLockBuffer { index: 1 });
    let _ = fixture
        .state
        .session_lock
        .await_vblank(SECOND, Some(1), now);
    fixture.state.note_flip_completed(SECOND, Some(1));
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "the frame carrying the drawn surface confirms"
    );
    put_back(&mut fixture, taken);
}

/// The fallback bound stands in for missing vblanks, never for a missing
/// surface: a placeholder screen is still unrecorded after the timeout, as
/// on the render-confirmed backends.
#[test]
fn the_fallback_does_not_confirm_over_a_placeholder_screen() {
    let (mut fixture, taken) = pending_two_output_lock();
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND_INDEX,
        color: None,
    });
    let t0 = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(FIRST, Some(0), t0);
    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "head 2's admitted surface has not drawn: no `locked` over its placeholder"
    );
    assert_eq!(fixture.state.session_lock.confirmed, vec![FIRST]);
    put_back(&mut fixture, taken);
}
