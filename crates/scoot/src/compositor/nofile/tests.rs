//! Tests for the fd-limit raise and its restore in children.
//!
//! The decision ([`target_soft`]) is pure and pinned exactly. The restore is
//! pinned end to end: a real child through the real `State::spawn`, reading
//! its own limits, the way `child_reaper/tests.rs` reads its signal state.
//! Building a `State` raises the limit (see `State::new`), so these run with
//! the limit a session runs with.

use std::time::{Duration, Instant};

use super::{RAISED_SOFT_CAP, raise, target_soft};
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

const INFINITY: u64 = u64::MAX;

#[test]
fn the_target_is_the_hard_limit_capped() {
    assert_eq!(target_soft(1024, 524_288), RAISED_SOFT_CAP);
    assert_eq!(target_soft(1024, 4096), 4096);
    assert_eq!(target_soft(1024, INFINITY), RAISED_SOFT_CAP);
}

#[test]
fn a_hard_limit_of_1024_changes_nothing() {
    // The container case: nothing to raise, and the queue cap stays 128.
    assert_eq!(target_soft(1024, 1024), 1024);
    assert_eq!(
        crate::compositor::fd_pressure::backend_queued_fds(1024),
        128
    );
}

#[test]
fn the_target_never_lowers_a_soft_limit() {
    assert_eq!(target_soft(200_000, 524_288), 200_000);
    assert_eq!(target_soft(INFINITY, INFINITY), INFINITY);
}

#[test]
fn raise_is_idempotent() {
    assert_eq!(raise(), raise());
}

/// A child started through `State::spawn` gets the soft limit the process
/// was started with, and the same hard limit, whatever the raise did.
#[test]
fn a_spawned_child_gets_the_original_soft_limit() {
    let mut fixture: Harness<(), ()> = Harness::bare(Appearance::default());
    let limits = raise().expect("RLIMIT_NOFILE is readable on this machine");
    // A path the shell needs no quoting for: the pid and a per-process
    // counter, nothing else.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!(
        "scoot-nofile-child-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&out);
    let script = format!(
        "ulimit -Sn > {path}.tmp; ulimit -Hn >> {path}.tmp; mv {path}.tmp {path}",
        path = out.display()
    );
    assert!(
        fixture
            .state
            .spawn(&["sh".to_owned(), "-c".to_owned(), script]),
        "sh could not be spawned"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let written = loop {
        if let Ok(text) = std::fs::read_to_string(&out) {
            break text;
        }
        assert!(
            Instant::now() < deadline,
            "the child never wrote its limits"
        );
        fixture.settle();
        std::thread::sleep(Duration::from_millis(5));
    };
    let _ = std::fs::remove_file(&out);
    let mut lines = written.lines();
    let soft = lines.next().expect("a soft limit line");
    let hard = lines.next().expect("a hard limit line");
    let render = |value: u64| {
        if value == INFINITY {
            "unlimited".to_owned()
        } else {
            value.to_string()
        }
    };
    assert_eq!(
        soft,
        render(limits.original_soft),
        "the child's soft limit (the process runs at {})",
        limits.soft
    );
    assert_eq!(hard, render(limits.hard), "the child's hard limit");
}
