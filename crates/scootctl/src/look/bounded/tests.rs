//! Tests for the bounded runner: the timeout really bounds, output is
//! capped, and clean runs pass through. Hermetic and fast: short
//! timeouts, small outputs, scratch only.

use std::path::Path;
use std::time::{Duration, Instant};

use super::{MAX_OUTPUT_BYTES, run_bounded};

#[test]
fn clean_exit_returns_its_output() {
    let out = run_bounded(Path::new("echo"), &["hello"], Duration::from_secs(10)).expect("runs");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"hello\n");
}

#[test]
fn missing_binary_is_a_spawn_error() {
    let error = run_bounded(
        Path::new("/nonexistent-helper-xyz"),
        &[],
        Duration::from_secs(10),
    )
    .expect_err("missing refuses");
    assert!(
        matches!(error, super::BoundedError::Spawn(_)),
        "a missing binary reports at spawn: {error}"
    );
}

#[test]
fn forking_child_is_bounded_by_the_deadline() {
    // The review's breaking input against the shared runner directly:
    // the grandchild holds the pipes 8 s; the 2 s deadline still wins.
    let start = Instant::now();
    let error = run_bounded(
        Path::new("/bin/sh"),
        &["-c", "sleep 8 & exec sleep 0"],
        Duration::from_secs(2),
    )
    .expect_err("the forked run times out");
    let elapsed = start.elapsed();
    assert!(
        matches!(error, super::BoundedError::Timeout { secs: 2, .. }),
        "a timeout names its deadline: {error}"
    );
    assert!(
        elapsed < Duration::from_secs(6),
        "took {elapsed:?}, not 8 s"
    );
}

#[test]
fn flooding_child_is_killed_at_the_cap() {
    // `yes` would fill the pipe forever; the cap kills it instead, with
    // at most one cap of bytes kept.
    let error = run_bounded(
        Path::new("/bin/sh"),
        &["-c", "yes | head -c 1000000"],
        Duration::from_secs(30),
    )
    .expect_err("a flooding run is killed");
    assert!(
        error.to_string().contains(&MAX_OUTPUT_BYTES.to_string()),
        "the refusal names the byte cap: {error}"
    );
}

#[test]
fn stderr_is_drained_beside_stdout() {
    // Both pipes drain while the child runs: a chatty stderr beside a
    // quiet stdout must not wedge the run either.
    let out = run_bounded(
        Path::new("/bin/sh"),
        &["-c", "echo out; echo err >&2"],
        Duration::from_secs(10),
    )
    .expect("runs");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"out\n");
    assert_eq!(out.stderr, b"err\n");
}
