//! Confirming `locked` once a `--nested` frame reaches the host.
//!
//! Under `--nested` the drawn frame is not always the shown one: a
//! read-back frame is dropped when the host holds both `wl_shm` buffers,
//! and a dma-buf frame is owed whenever no host buffer is usable. So
//! `locked` waits for the host's commit of the blanked frame -- recorded by
//! the render tail through [`State::note_nested_frame`], and by the
//! owed-frame hand-over through [`State::note_nested_handed_over`] -- with
//! the same bounded fallback as `--tty`'s vblank wait rather than hanging
//! the locker when the host never releases.
//!
//! These tests drive that wait through the same [`State`] methods the render
//! tail and the hand-over call, with synthetic commits and synthetic time:
//! there is no host in a harness, so the commit half is a bool and the clock
//! half is an `Instant` handed in, never read inside. What the render tail
//! itself routes (`host.is_some()` into `note_nested_frame` with
//! `frame.host_committed`) is covered by construction here -- headless has
//! no host to defer to, so its confirm-on-render path is pinned unchanged
//! instead (see `vblank_confirm.rs`), and the dma-buf hand-over's one-line
//! call is covered the same way, beside the live owed-frame suite in
//! `nested/gpu/tests/live.rs`.

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

/// A locked fixture whose render target is taken away, so no headless
/// render can confirm the lock the old way -- the only confirmation paths
/// left are the nested ones under test. Answers the fixture and its held
/// backend, which each test puts back before its final settle (the vblank
/// suite's own idiom).
fn pending_fixture() -> (Fixture, crate::compositor::render::Backend) {
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
    (fixture, backend)
}

fn primary_of(fixture: &Fixture) -> (scoot_core::OutputId, smithay::output::Output) {
    fixture
        .state
        .outputs
        .primary_entry()
        .map(|(id, output)| (id, output.clone()))
        .expect("an output")
}

/// A drawn-but-dropped read-back frame -- both host `wl_shm` buffers held,
/// say -- sends no `locked`, but records the draw and arms the fallback.
/// This is the exact gap the ticket closes: on the old confirm-on-draw path
/// this frame would have confirmed over the host's last unlocked frame.
#[test]
fn a_dropped_nested_frame_sends_no_locked_but_arms_the_fallback() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    assert!(
        fixture.state.note_nested_frame(id, &output, 1, false, t0),
        "the first dropped blank arms the fallback and its timer"
    );
    for _ in 0..20 {
        fixture.settle();
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "no host commit, no `locked`"
    );
    let report = fixture.report();
    assert_eq!(
        report.locked, 0,
        "the client must not hear `locked` before the host has the blanked frame"
    );
    assert_eq!(report.finished, 0);
    assert!(
        fixture.state.session_lock.blank_flips.is_empty(),
        "nested tracks no flip: the host's commit is the confirmation"
    );
    assert!(
        fixture.state.session_lock.blank_deadline.is_some(),
        "the fallback deadline must be armed, or a host that never releases hangs the locker"
    );

    fixture.state.put_primary_backend(backend);
}

/// The next frame the host does commit confirms: the client hears `locked`
/// exactly once, for the frame the host has.
#[test]
fn a_committed_nested_frame_confirms_the_lock() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    let _ = fixture.state.note_nested_frame(id, &output, 1, false, t0);

    // A re-render the skip re-armed, whose commit the host accepted.
    assert!(
        !fixture
            .state
            .note_nested_frame(id, &output, 1, true, t0 + Duration::from_millis(10)),
        "a wait already armed needs no second timer"
    );
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the committed blank confirms the lock"
    );
    assert!(
        fixture.state.session_lock.blank_deadline.is_none(),
        "confirming takes the wait, deadline with it"
    );
    fixture.state.put_primary_backend(backend);
    fixture.settle();
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);
}

/// The prompt-host case from the ticket's edge list: the first blanked
/// frame commits at once, so `locked` goes out with no wait armed at all --
/// and in particular the fallback must never fire for it later.
#[test]
fn a_prompt_commit_confirms_without_arming_a_wait() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    assert!(
        !fixture.state.note_nested_frame(id, &output, 1, true, t0),
        "nothing to bound, so no timer"
    );
    assert!(fixture.state.session_lock.pending.is_none());
    assert!(fixture.state.session_lock.blank_deadline.is_none());
    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);

    // The bound passing afterwards confirms nothing twice.
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT + Duration::from_secs(60));
    assert!(fixture.state.session_lock.pending.is_none());
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// The dma-buf path: a resize's first frame is drawn before the host has
/// created the new chain, so it is owed, not committed -- and the hand-over
/// once a buffer is usable confirms, without a second draw.
///
/// Feature-gated with the path itself: only dma-buf presentation owes
/// frames (see `nested/gpu.rs`), so this runs in the `gpu-scanout` suite,
/// like every other dma-buf suite -- verified on the dev VM, never in the
/// default CI run.
#[cfg(feature = "gpu-scanout")]
#[test]
fn an_owed_frame_handed_over_confirms() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    let _ = fixture.state.note_nested_frame(id, &output, 1, false, t0);
    assert!(fixture.state.session_lock.pending.is_some());

    // `Host::hand_over_owed_frame`'s own call, for the commit it just made.
    fixture.state.note_nested_handed_over(id, &output);
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the handed-over blank confirms the lock"
    );
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);

    // A late hand-over after the confirm is a no-op, never a second confirm.
    fixture.state.note_nested_handed_over(id, &output);
    assert!(fixture.state.session_lock.pending.is_none());
    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// The owed frame the hand-over carries is whatever the render target holds
/// -- which, when the lock raced the hand-over, is still the unlocked frame
/// from before it. That must not confirm: the lock's own render is already
/// pending, and its committed frame is what confirms.
///
/// Feature-gated with the path itself, like the test above.
#[cfg(feature = "gpu-scanout")]
#[test]
fn an_owed_frame_from_before_the_lock_does_not_confirm() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);

    // No blanked draw recorded for this lock yet: the hand-over of the old
    // frame is refused.
    fixture.state.note_nested_handed_over(id, &output);
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "an unlocked frame must not confirm the lock"
    );
    assert_eq!(fixture.report().locked, 0);

    // The lock's own blanked frame then draws (dropped again) and is handed
    // over: that one does confirm.
    let _ = fixture
        .state
        .note_nested_frame(id, &output, 1, false, Instant::now());
    fixture.state.note_nested_handed_over(id, &output);
    assert!(fixture.state.session_lock.pending.is_none());
    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// The load-bearing safety property, same bound as `--tty`: no host commit
/// within one second confirms anyway, so a host that stops releasing
/// buffers (hidden, minimised, stalled) never hangs the locker. The bound
/// itself is pinned: one second, no earlier, no later.
#[test]
fn no_commit_within_the_bound_confirms_anyway() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    let _ = fixture.state.note_nested_frame(id, &output, 1, false, t0);

    // Just inside the bound: still waiting -- the fallback must not fire
    // early on a merely slow host.
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

/// The resize-storm edge: redraws while the host is behind neither extend
/// the bound nor arm another timer. Without that a host that never releases
/// while the screen keeps changing would hold `locked` back forever -- the
/// same livelock the `--tty` wait closes by arming once.
#[test]
fn redraws_while_waiting_keep_the_first_bound() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    assert!(
        fixture.state.note_nested_frame(id, &output, 1, false, t0),
        "the first dropped blank arms the bound and its timer"
    );
    assert!(
        !fixture
            .state
            .note_nested_frame(id, &output, 1, false, t0 + Duration::from_millis(500)),
        "a redraw neither extends the bound nor needs another timer"
    );

    // The original bound confirms, exactly once -- no later.
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT - Duration::from_millis(1));
    assert!(fixture.state.session_lock.pending.is_some());
    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(fixture.state.session_lock.pending.is_none());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    assert_eq!(fixture.report().locked, 1);
}

/// Unlocking mid-wait cancels it: a later commit, hand-over or fallback for
/// the dead wait confirms nothing and locks nothing.
#[test]
fn unlock_cancels_the_nested_wait_without_a_stale_confirm() {
    let (mut fixture, backend) = pending_fixture();
    let (id, output) = primary_of(&fixture);
    let t0 = Instant::now();
    let _ = fixture.state.note_nested_frame(id, &output, 1, false, t0);

    SessionLockHandler::unlock(&mut fixture.state);
    assert!(!fixture.state.session_lock.is_locked());
    assert!(fixture.state.session_lock.pending.is_none());

    // A commit that lands after the unlock...
    let _ = fixture
        .state
        .note_nested_frame(id, &output, 1, true, t0 + Duration::from_millis(10));
    // ...a hand-over of the frame it drew while pending (dma-buf only)...
    #[cfg(feature = "gpu-scanout")]
    fixture.state.note_nested_handed_over(id, &output);
    // ...and the fallback bound passing long after...
    fixture
        .state
        .note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT + Duration::from_secs(60));
    assert!(fixture.state.session_lock.pending.is_none());
    assert!(!fixture.state.session_lock.is_locked());

    fixture.state.put_primary_backend(backend);
    fixture.settle();
    let report = fixture.report();
    assert_eq!(report.locked, 0);
}
