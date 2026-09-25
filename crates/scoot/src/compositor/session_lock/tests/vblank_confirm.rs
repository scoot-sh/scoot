//! Confirming `locked` from the DRM vblank handler instead of the render loop.
//!
//! Under `--tty` a rendered blanked frame still has to survive `present` and
//! a page flip before it is on scanout, so `locked` waits for the vblank
//! confirming the flip that carries it -- with a bounded fallback that
//! confirms anyway rather than hanging the locker when that vblank can never
//! arrive (switched away, a flip discarded by a modeset). These tests drive
//! that wait through the same [`State`](crate::compositor::State) methods the
//! DRM event handler and the fallback timer call, with synthetic flips and
//! synthetic time: there is no DRM device in a harness, so the flip half is a
//! sequence number and the clock half is an `Instant` handed in, never read
//! inside. What the render loop itself records (`await_vblank` on present) is
//! covered by construction here -- headless has no presenter to defer to, so
//! its confirm-on-render path is pinned unchanged instead, and the `--tty`
//! half is unit-tested without hardware in `tty::flip_tracker`.

use std::time::{Duration, Instant};

use smithay::wayland::session_lock::SessionLockHandler;

use super::*;
use crate::compositor::session_lock::LOCK_VBLANK_TIMEOUT;

/// Drains the `Done` a `LockNoWait` step acknowledges with, so a later
/// `report()` reads its own answer rather than this stale one (see
/// `lifecycle.rs`'s `the_locked_event_waits_for_a_blanked_frame` for the
/// same idiom).
fn drain_lock_nowait_ack(fixture: &mut Fixture) {
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
}

/// Headless behavior is unchanged by the vblank wait: with no presenter to
/// defer to, a drawn frame confirms at once, and no wait is recorded.
#[test]
fn headless_render_still_confirms_immediately() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.render();
    let report = fixture.report();
    assert_eq!(report.locked, 1, "a drawn headless frame confirms at once");
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "nothing should still be pending after a headless confirm"
    );
    assert!(
        fixture.state.session_lock.blank_flips.is_empty(),
        "headless must not record a vblank wait"
    );
    assert!(
        fixture.state.session_lock.blank_deadline.is_none(),
        "headless must not arm the fallback deadline"
    );
}

/// A blanked frame handed to an in-flight flip sends no `locked` until that
/// flip's vblank arrives. The render target is taken away so no headless
/// render can confirm the lock the old way -- the only confirmation path
/// left is the vblank one under test.
#[test]
fn a_blanked_frame_without_its_vblank_sends_no_locked() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session locks as soon as the request arrives"
    );
    assert!(fixture.state.session_lock.pending.is_some());

    // What `render()` records after `Tty::present` issues flip 7 carrying the
    // blanked frame.
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(7), Instant::now());
    for _ in 0..20 {
        fixture.settle();
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "no vblank, no `locked`"
    );
    let report = fixture.report();
    assert_eq!(
        report.locked, 0,
        "the client must not hear `locked` before its blanked frame is on scanout"
    );
    assert_eq!(report.finished, 0);

    fixture.state.put_primary_backend(backend);
}

/// The tracked flip's vblank confirms the lock: the client hears `locked`
/// exactly once, for the frame that reached scanout.
#[test]
fn the_tracked_flips_vblank_confirms_the_lock() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(7), Instant::now());

    // The DRM vblank handler's own call, for flip 7's completion.
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(7));
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the tracked flip's vblank confirms the lock"
    );

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);
}

/// The exact gap this closes: the lock raced a flip already in flight, so
/// `present` skipped and the blanked frame is not aboard. The previous --
/// possibly unlocked -- frame's vblank must not confirm.
#[test]
fn a_vblank_for_the_previous_frame_does_not_confirm() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);

    // Flip 6 (the unlocked frame) was already in flight when the lock's
    // frame rendered, so nothing was recorded for it. Its vblank arrives:
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(6));
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "a vblank for a frame without the blank pixels must not confirm"
    );
    assert_eq!(fixture.report().locked, 0);

    // The re-render the skip armed presents the blanked frame as flip 8,
    // whose vblank does confirm.
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(8), Instant::now());
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(8));
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// A vblank arriving after the scanout bookkeeping was discarded (VT switch
/// back, hotplug modeset) carries no sequence -- it is a completion for a
/// flip that no longer exists, not for the tracked one, and must not confirm.
#[test]
fn a_stale_vblank_after_invalidation_does_not_confirm() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(7), Instant::now());

    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), None);
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "a stale completion must not confirm"
    );
    assert_eq!(fixture.report().locked, 0);

    // Recovery is the ordinary path: the next render presents the blanked
    // frame again and its vblank confirms.
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(8), Instant::now());
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(8));
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// The load-bearing safety property: no vblank within the bound confirms
/// anyway, so a locker never hangs forever -- while switched away, after a
/// discarded flip, or on hardware whose completions never arrive. The bound
/// itself is pinned: one second, no earlier, no later.
#[test]
fn no_vblank_within_the_bound_confirms_anyway() {
    assert_eq!(
        LOCK_VBLANK_TIMEOUT,
        Duration::from_secs(1),
        "the no-hang bound is one second"
    );
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let t0 = Instant::now();
    let _ = fixture
        .state
        .session_lock
        .await_vblank(scoot_core::OutputId(1), Some(7), t0);

    // Just inside the bound: still waiting -- the fallback must not fire
    // early on a merely slow vblank.
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT - Duration::from_millis(1));
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the fallback must not fire before its bound"
    );
    assert_eq!(fixture.report().locked, 0);

    // On the bound: confirms anyway, and the wait is taken exactly once.
    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(fixture.state.session_lock.pending.is_none());
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT + Duration::from_secs(60));
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    let report = fixture.report();
    assert_eq!(report.locked, 1);
}

/// A re-presented flip restarts the bound and needs its own timer: after a
/// discard (scanout invalidated, session paused) the next render presents
/// the blanked frame again, and the old timer -- firing against the old
/// deadline -- no longer watches anything. Without a fresh timer a second
/// lost vblank would hang the locker past the new bound with nothing to
/// fire.
#[test]
fn a_represented_flip_restarts_the_bound_and_re_arms() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let t0 = Instant::now();
    assert!(
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(7), t0)
    );

    // The tracked flip is discarded without a completion (its vblank will
    // never arrive), and the next render presents the blanked frame again
    // as flip 8. That re-arms: the caller must watch the new deadline.
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), None);
    assert!(fixture.state.session_lock.pending.is_some());
    assert!(
        fixture.state.session_lock.await_vblank(
            scoot_core::OutputId(1),
            Some(8),
            t0 + Duration::from_millis(500)
        ),
        "a fresh flip restarts the bound, so it needs a fresh timer"
    );

    // The old bound passes with the wait still live -- the stale timer
    // firing here must not confirm.
    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the restarted bound, not the old one, owns the wait"
    );
    assert_eq!(fixture.report().locked, 0);

    // The new bound confirms, exactly once.
    fixture
        .state
        .note_blank_timeout(t0 + Duration::from_millis(500) + LOCK_VBLANK_TIMEOUT);
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}
#[test]
fn unlock_cancels_the_wait_without_a_stale_confirm() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let _ =
        fixture
            .state
            .session_lock
            .await_vblank(scoot_core::OutputId(1), Some(7), Instant::now());

    SessionLockHandler::unlock(&mut fixture.state);
    assert!(!fixture.state.session_lock.is_locked());
    assert!(fixture.state.session_lock.pending.is_none());

    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(7));
    assert!(fixture.state.session_lock.pending.is_none());
    assert!(!fixture.state.session_lock.is_locked());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 0);
}

/// Rapid lock/unlock cycling across the wait: the first lock's late vblank
/// must not confirm the second lock, and the second lock confirms on its own
/// flip's vblank alone.
///
/// Driven through the wire-reachable give-up path: the first client destroys
/// its lock before `locked` arrives (legal -- only `unlock_and_destroy` is
/// forbidden that early), so the session reads as abandoned with the dead
/// lock's wait still recorded, and the replacement's own `lock` sweeps it.
#[test]
fn rapid_cycles_do_not_cross_confirm() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.take_primary_backend().expect("a backend");
    // First lock, with its blanked frame aboard flip 7...
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    let t0 = Instant::now();
    let _ = fixture
        .state
        .session_lock
        .await_vblank(scoot_core::OutputId(1), Some(7), t0);
    // ...which the client gives up on. The session stays locked and reads
    // as abandoned, with the dead lock's wait still recorded.
    fixture.run(Step::DestroyLock { lock: 0 });
    assert!(fixture.state.session_lock.is_locked());
    assert!(fixture.state.session_lock.pending.is_some());

    // A fresh lock on the same connection: the dead-pending sweep clears the
    // way *and* the old wait, so the first flip's late vblank has nothing to
    // match against.
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    drain_lock_nowait_ack(&mut fixture);
    assert!(fixture.state.session_lock.pending.is_some());
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(7));
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the old flip must not confirm the new lock"
    );

    // The new lock defers on its own flip and confirms on its own vblank.
    let _ = fixture
        .state
        .session_lock
        .await_vblank(scoot_core::OutputId(1), Some(9), t0);
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(7));
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the old flip must not confirm the new lock either"
    );
    fixture
        .state
        .note_flip_completed(scoot_core::OutputId(1), Some(9));
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);
}
