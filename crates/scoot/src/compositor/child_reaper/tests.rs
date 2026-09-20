//! The reaper end to end: real children through the real [`State::spawn`],
//! asserting on the real process table.
//!
//! Three properties, each a live process rather than a `Command` inspection:
//!
//! - without the reaper, an exited child stays a zombie (the bug, and the
//!   fail-first control proving the probe below can actually see one);
//! - with it, a burst of fast-exiting children is collected with no `Z`
//!   left behind;
//! - a spawned child starts with `SIGCHLD` at `SIG_DFL` and an empty signal
//!   mask, pinning both the `SIG_IGN` shortcut and the signalfd-block
//!   regressions (read from the child's own `/proc/self/status`).
//!
//! Whatever these install must never be a process-wide `waitpid(-1)` drain:
//! the `scoot` unit-test binary's own fork/waitpid children
//! (`ipc/accept/tests.rs`, `wayland_accept/tests.rs`) share this process
//! under `cargo test`, and a `-1` drain would reap them out from under their
//! tests -- green under `nextest`, red in CI. The drain only ever `waitpid`s
//! pids `State::spawn` tracked, so those foreign children are at most a
//! spurious wakeup. Like every other live-`State` suite here, these need a
//! writable `$XDG_RUNTIME_DIR` (`State::new` binds a real listening socket)
//! and a real `sh`/`true`, which the dev VM and any Unix test host have.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::install;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// A live compositor with no backend and no clients: the reaper needs a
/// `State` (for its tracked set) and an event loop (for its wake source),
/// nothing else.
type Fixture = Harness<(), ()>;

/// How long a spawned `true` may take to exit, and a reaped child to vanish
/// from the process table. Generous: a debug build under a VM. A timeout, not
/// a timing assertion -- a regression fails in seconds rather than hanging
/// the suite.
const PATIENCE: Duration = Duration::from_secs(10);

/// The `SIGCHLD` bit in a `/proc/self/status` signal mask: signal 17, and
/// masks count from bit 0.
const SIGCHLD_BIT: u64 = 1 << (libc::SIGCHLD as u32 - 1);

/// The state letter of `pid` from `/proc/<pid>/stat`, or `None` once the pid
/// has left the process table entirely (reaped, not merely exited).
///
/// Parsed after the *last* `)` because the comm field may itself contain
/// parens; `true` never does, but the helper should not depend on that.
fn proc_state(pid: u32) -> Option<char> {
    let body = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = body.rfind(')')? + 1;
    body[after_comm..].split_whitespace().next()?.chars().next()
}

/// The one pid `fixture`'s `State::spawn` tracked. Each test spawns a known
/// number of children and reads them back before any drain can run, so the
/// set holds exactly what the test started.
fn tracked_pid(fixture: &Fixture) -> u32 {
    let pids: Vec<u32> = fixture.state.spawned_children.iter().copied().collect();
    assert_eq!(
        pids.len(),
        1,
        "expected exactly one tracked child, found {pids:?}"
    );
    pids[0]
}

/// Dispatches until every pid in `pids` has left the process table and the
/// tracked set is empty, or fails after [`PATIENCE`].
fn dispatch_until_reaped(fixture: &mut Fixture, pids: &[u32]) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
        if pids.iter().all(|pid| proc_state(*pid).is_none())
            && fixture.state.spawned_children.is_empty()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {pids:?} to be reaped (states: {:?}, still tracked: {:?})",
            pids.iter().map(|pid| proc_state(*pid)).collect::<Vec<_>>(),
            fixture.state.spawned_children,
        );
    }
}

/// An exited child with no reaper installed stays a zombie.
///
/// The fail-first control for the test below: it proves the probe (`Z` in
/// `/proc/<pid>/stat`) can actually see an unreaped child, so that test's
/// "no `Z`" assertion would go red against an implementation that never
/// reaps. The zombie is collected explicitly at the end, so the shared
/// `cargo test` process does not accumulate one per run -- and that tail
/// doubles as the `ECHILD` edge of the drain (already reaped by the blocking
/// `waitpid`, forgotten by `reap_children`).
#[test]
fn an_exited_child_without_the_reaper_stays_a_zombie() {
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    fixture.state.spawn(&["true".to_string()]);
    let pid = tracked_pid(&fixture);

    let deadline = Instant::now() + PATIENCE;
    loop {
        if proc_state(pid) == Some('Z') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the spawned child never even exited; the reaper test below would pass vacuously"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        fixture.state.spawned_children.contains(&pid),
        "nothing reaps here, so the child must still be tracked"
    );

    // SAFETY: `pid` is the exited child this test spawned; a blocking
    // `waitpid` on it can only collect that child, and it is already a
    // zombie, so this returns immediately rather than hanging the suite.
    let mut status = 0;
    let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
    assert_eq!(
        reaped, pid as libc::pid_t,
        "could not collect the probe child"
    );
    fixture.state.reap_children();
    assert!(
        fixture.state.spawned_children.is_empty(),
        "an already-reaped pid must leave the tracked set (ECHILD), not leak in it"
    );
}

/// A burst of fast-exiting children through the real `State::spawn` is
/// collected with no zombie left behind.
///
/// Eight `true`s at once, not one: `SIGCHLD` coalesces, so one signal can
/// stand for the whole burst, and the drain must sweep the full tracked set
/// per wakeup rather than reaping once per signal. Verified fail-first by
/// running this exact body with the `install` line removed: it times out
/// with all eight still `Z`.
#[test]
fn the_reaper_collects_a_burst_of_exited_children() {
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    let handle = fixture.event_loop.handle();
    install(&handle, &mut fixture.state).expect("the SIGCHLD reaper");
    for _ in 0..8 {
        fixture.state.spawn(&["true".to_string()]);
    }
    let pids: Vec<u32> = fixture.state.spawned_children.iter().copied().collect();
    assert_eq!(pids.len(), 8, "every spawn must be tracked: {pids:?}");
    dispatch_until_reaped(&mut fixture, &pids);
}

/// A spawned child starts with `SIGCHLD` at `SIG_DFL` and an empty signal
/// mask.
///
/// Read from the child's own `/proc/self/status`, which pins both
/// regressions nothing else would catch: `SigIgn` carrying the bit is the
/// `signal(SIGCHLD, SIG_IGN)` shortcut (survives `execve`, breaks the
/// child's own `wait()`); `SigBlk` carrying it is the signalfd design's
/// process-wide block leaking through `Command::spawn` (libstd resets
/// SIGPIPE only). `SigCgt` carrying it would mean the caught handler itself
/// leaked across `exec`, which the kernel resets for free -- asserted as
/// the third leg of the same table.
#[test]
fn a_spawned_child_sees_default_sigchld_and_an_empty_mask() {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "scoot-spawn-sigchld-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    // The path travels as `$1`, so no filename ever goes through shell
    // quoting -- the same shape `activation/tests/spawn.rs` uses.
    fixture.state.spawn(&[
        "sh".to_string(),
        "-c".to_string(),
        "grep -E '^(SigIgn|SigBlk|SigCgt):' /proc/self/status > \"$1\"".to_string(),
        "sh".to_string(),
        path.to_string_lossy().into_owned(),
    ]);
    let pid = tracked_pid(&fixture);
    let body = read_probe(&path);
    for line in ["SigIgn", "SigBlk", "SigCgt"] {
        let mask = parse_mask(&body, line);
        assert_eq!(
            mask & SIGCHLD_BIT,
            0,
            "a spawned child inherits SIGCHLD in {line} (mask {mask:#x}):\n{body}"
        );
    }
    // The probe child has exited (it wrote its file just before doing so);
    // collect it explicitly rather than depending on any installed reaper,
    // so this test stays deterministic whatever else in this process did.
    //
    // SAFETY: `pid` is the exited probe child; a blocking `waitpid` on it
    // can only collect that child.
    let mut status = 0;
    let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
    assert_eq!(
        reaped, pid as libc::pid_t,
        "could not collect the probe child"
    );
    fixture.state.reap_children();
    assert!(fixture.state.spawned_children.is_empty());
}

/// Waits for the probe child's file to appear with content, reads it, and
/// removes the file.
///
/// Content, not mere existence: the shell creates the file before writing to
/// it, so an early read would see an empty file and assert on nothing.
fn read_probe(path: &Path) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(body) = std::fs::read_to_string(path)
            && !body.is_empty()
        {
            let _ = std::fs::remove_file(path);
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "the spawned child never wrote its status file: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Parses one `SigIgn`/`SigBlk`/`SigCgt` hex mask out of a `/proc` status
/// dump. Fails loud on a missing or malformed line: asserting a defaulted
/// zero would pass against a probe that never ran.
fn parse_mask(body: &str, field: &str) -> u64 {
    let line = body
        .lines()
        .find(|line| line.starts_with(field))
        .unwrap_or_else(|| {
            panic!("the probe child reported no {field} line -- it observes nothing:\n{body}")
        });
    let hex = line
        .split(':')
        .nth(1)
        .unwrap_or_else(|| panic!("the probe child's {field} line has no value:\n{body}"));
    u64::from_str_radix(hex.trim(), 16)
        .unwrap_or_else(|_| panic!("the probe child's {field} value is not hex ({hex:?}):\n{body}"))
}
