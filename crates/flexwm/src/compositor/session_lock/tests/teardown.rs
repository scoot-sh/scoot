//! Commits on a lock surface whose role object has already been destroyed.
//!
//! Legal, and what a real locker sends on every unlock -- which used to cost
//! the user their entire shell (see
//! `docs/backlog/resolved/session-lock-post-destroy-commit-resolved.md`). The
//! by-design kills that must keep working are here too, because what makes
//! the carve-out correct is that it did *not* widen to cover them.

use super::*;

/// A commit on the surviving `wl_surface` after its lock role was destroyed
/// is a safe no-op, not a client kill.
///
/// This used to post `CommitBeforeFirstAck` and drop the connection (the
/// lock-surface half of the post-unlock kill -- see
/// `docs/backlog/resolved/session-lock-post-destroy-commit-resolved.md`): Smithay's
/// destruction handler resets `last_acked`, and the next commit validated
/// against the reset. A real Quickshell client (Noctalia 4.7.7 / quickshell
/// 0.3.1) sends exactly this teardown on every unlock, so every screen
/// unlock cost the user their entire shell. The fix restores the acked
/// configure the reset dropped before the commit is delegated; see
/// `session_lock.rs`'s `prepare_post_destroy_lock_commit`.
#[test]
fn committing_a_lock_surface_after_its_role_was_destroyed_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    // The commit a teardown sends on the surface it kept alive.
    fixture.run(Step::CommitLockSurface { index: 0 });
    // The client must still be alive to answer this at all.
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the lock is still held by a live client");
    assert_eq!(report.finished, 0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session is still locked"
    );
    fixture.render();
    // ...and the lock still unlocks afterwards.
    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
}

/// The null half of the same teardown: `destroy` the role, null-attach,
/// commit -- what stock quickshell sends on every unlock, and what used to
/// die with `NullBuffer` on top of the `CommitBeforeFirstAck` above.
#[test]
fn null_commit_after_lock_role_destroy_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the client survived its own teardown");
    assert_eq!(report.finished, 0);
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
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

/// The full real-world order: the unlock itself goes through first (which
/// clears the compositor's surface list), and *then* the trailing role
/// destroy plus null commit arrives on a surface flexwm has forgotten. The
/// ack record has to outlive the surface list for exactly this reason.
#[test]
fn unlock_then_role_destroy_then_null_commit_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    // Still alive to answer: the unlock must not cost the client its shell.
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the destroyed lock surface must stay gone"
    );
}

/// A same-size buffer committed after the role destroy maps the surface
/// again rather than killing the client. Only its own locker's pixels on
/// its own lock screen -- every read still goes through `current` -- so
/// this is cosmetically odd, not a hole; a wrong-size one still dies with
/// `DimensionsMismatch`.
#[test]
fn commit_with_buffer_after_lock_role_destroy_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::ReattachLockBuffer { index: 0 });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client survived redrawing after the destroy"
    );
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
}

/// Two commits after one destroy -- bare, then null. The second one is what
/// the `role_destroyed` flag is for: the first restore puts a value back
/// into `last_acked`, so only the flag still tells a destroyed role from a
/// live one.
#[test]
fn repeated_commits_after_lock_role_destroy_survive() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::CommitLockSurface { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client survived both trailing commits"
    );
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
}

/// The by-design half the carve-out must not touch: attaching a buffer and
/// committing *before* the first ack still kills the client. No ack was ever
/// recorded, so the interception leaves the commit alone to die loudly.
#[test]
fn committing_content_before_any_ack_still_kills_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let error = fixture.run_expecting_disconnect(Step::LockSurfaceNoAckWithBuffer { lock: 0 });
    assert!(
        error.contains("Broken pipe") || error.contains("Protocol error"),
        "the client should be dead: {error}"
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

/// The other by-design half: a null commit on a surface whose role is still
/// alive -- a mapped lock surface unmapping itself, which the protocol
/// forbids -- still dies with `NullBuffer`. The interception only reaches
/// destroyed-role surfaces, so a live role validates exactly as before.
#[test]
fn null_commit_on_a_live_mapped_lock_surface_still_kills_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    let error = fixture.run_expecting_disconnect(Step::NullCommitLockSurface { index: 0 });
    assert!(
        error.contains("Broken pipe") || error.contains("Protocol error"),
        "the client should be dead: {error}"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker must not unlock the session"
    );
    fixture.render();
}

/// A teardown commit while locked, followed by the client dying outright:
/// the session stays locked and reads as abandoned, same as any other dead
/// locker.
#[test]
fn commit_after_role_destroy_then_abandon_stays_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::CommitLockSurface { index: 0 });
    assert!(fixture.state.session_lock.is_locked());

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
