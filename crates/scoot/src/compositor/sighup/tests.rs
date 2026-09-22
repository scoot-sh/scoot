//! `SIGHUP` end to end: real signals at a live compositor through the real
//! handler -- `libc::raise`, never a direct function call, because the signal
//! path is the feature.
//!
//! Every test here holds `SIGNAL_LOCK`: the handler and its wake fd are
//! process-global (last install wins), and `raise` delivers to the calling
//! thread's process, so two of these running concurrently would wake each
//! other's loops. Serialized, each install-raise-dispatch-assert sequence is
//! hermetic. (`nextest` isolates these in separate processes anyway; the lock
//! is for `cargo test`'s shared process.)
//!
//! Whatever these install must never be a process-wide `waitpid(-1)` drain,
//! for the reason `child_reaper`'s module doc states -- but nothing here
//! reaps at all: the SIGHUP drain only reloads, so foreign children are not
//! even a spurious wakeup here. Like every other live-`State` suite, these
//! need a writable `$XDG_RUNTIME_DIR` (`State::new` binds a real listening
//! socket) and a real `sh`, which the dev VM and any Unix test host have.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use scoot_core::{Action, Config};
use scoot_ipc::{Request, Response};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::{install, on_sighup};
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;
use crate::compositor::test_support::{
    Harness, assert_marker_never_appears, locker, marker_path, test_renderer, touch_entry,
    wait_for_marker,
};

/// Serializes every signal test in this module: the `SIGHUP` disposition and
/// the wake fd are process-global. A poisoned lock still yields its inner
/// guard -- a panicking holder must not wedge the rest of the suite behind
/// it.
static SIGNAL_LOCK: Mutex<()> = Mutex::new(());

/// How long a raised HUP may take to become an applied reload. Generous: a
/// debug build under a VM. A timeout, not a timing assertion -- a regression
/// fails in seconds rather than hanging the suite.
const PATIENCE: Duration = Duration::from_secs(10);

const CANVAS: i32 = 200;

/// A live `State` on a headless backend with `config_path` pointed at a temp
/// file holding `contents`, and the SIGHUP handler installed on its loop --
/// the shape `reload/tests.rs` builds its `State` in, plus the signal path.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    path: PathBuf,
    _dir: tempfile::TempDir,
    /// Held for the fixture's life: no two signal fixtures exist at once.
    _signal: MutexGuard<'static, ()>,
}

impl Fixture {
    fn with_config(contents: &str) -> Self {
        let signal = SIGNAL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, contents).expect("a config file");
        state.config_path = Some(path.clone());
        state.needs_render = false;

        install(&event_loop.handle()).expect("the SIGHUP handler");
        Self {
            event_loop,
            state,
            path,
            _dir: dir,
            _signal: signal,
        }
    }

    fn rewrite(&self, contents: &str) {
        fs::write(&self.path, contents).expect("a rewritten config file");
    }

    /// Raises a real `SIGHUP` at this process, then dispatches until `ready`
    /// answers true -- the handler runs synchronously inside `raise`, so by
    /// the time it returns the wakeup is pending and one dispatch serves it.
    fn raise_and_settle(&mut self, what: &str, ready: impl Fn(&Fixture) -> bool) {
        // SAFETY: raising `SIGHUP` at this process with the handler installed
        // above invokes the handler on this thread and returns; it delivers
        // no signal anywhere else.
        let raised = unsafe { libc::raise(libc::SIGHUP) };
        assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
        let deadline = Instant::now() + PATIENCE;
        while !ready(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what} after SIGHUP"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    fn gap(&self) -> i32 {
        self.state.world.config().gap
    }
}

/// Reads the current `SIGHUP` disposition without changing it.
fn sighup_disposition() -> libc::sigaction {
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new-action reads the current disposition into `current`;
    // `SIGHUP` is a valid signal number.
    let read = unsafe { libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut current) };
    assert_eq!(read, 0, "could not read the SIGHUP disposition");
    current
}

/// Installing the handler fully replaces the default terminate disposition.
///
/// Fail-first control for the live tests below: neuter `install` and this
/// goes red, proving the disposition assertion observes the install rather
/// than passing vacuously. (Neuter it in the live tests instead and the
/// test process dies of `SIGHUP` -- verified once by hand, recorded in the
/// resolved ticket -- which is the same fail-first shape one level up.)
#[test]
fn install_replaces_the_terminate_disposition() {
    let _signal = SIGNAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = sighup_disposition();
    let event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    install(&event_loop.handle()).expect("the SIGHUP handler");
    let after = sighup_disposition();
    assert_eq!(
        after.sa_sigaction, on_sighup as *const () as usize,
        "install did not point SIGHUP at the handler"
    );
    // Restore what was there, so the shared `cargo test` process is left as
    // found (under `nextest` this is a no-op on a per-test process).
    //
    // SAFETY: `before` was read from this same signal moments ago; restoring
    // it is always a valid `sigaction` call.
    let restored = unsafe { libc::sigaction(libc::SIGHUP, &before, std::ptr::null_mut()) };
    assert_eq!(restored, 0, "could not restore the SIGHUP disposition");
}

/// A HUP with the handler disconnected keeps the default meaning: terminate.
///
/// Forked, so the death happens to a child, not the suite: the child
/// restores `SIG_DFL` (disconnecting the handler), raises `SIGHUP`, and must
/// die by that signal -- proving the handler is what stands between a HUP
/// and a lost session. The child touches nothing but async-signal-safe calls
/// between `fork` and death.
#[test]
fn a_disconnected_handler_leaves_the_default_terminate_disposition() {
    let _signal = SIGNAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // SAFETY: `fork` in a multithreaded test binary copies only the calling
    // thread; the child below calls only `sigaction`, `raise` and `_exit`
    // (or dies first), all async-signal-safe.
    let child = unsafe { libc::fork() };
    assert!(child >= 0, "could not fork the disposition probe");
    if child == 0 {
        unsafe {
            let mut default: libc::sigaction = std::mem::zeroed();
            default.sa_sigaction = libc::SIG_DFL as *const () as usize;
            libc::sigemptyset(&mut default.sa_mask);
            libc::sigaction(libc::SIGHUP, &default, std::ptr::null_mut());
            libc::raise(libc::SIGHUP);
            // Unreached when the disposition is what it should be: `raise`
            // with `SIG_DFL` kills the child first.
            libc::_exit(42);
        }
    }
    let mut status = 0;
    // SAFETY: `child` is the probe forked above; a blocking `waitpid` on
    // exactly that pid can only collect that child.
    let reaped = unsafe { libc::waitpid(child, &mut status, 0) };
    assert_eq!(reaped, child, "could not collect the probe child");
    assert!(
        libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGHUP,
        "a HUP past a disconnected handler should kill with SIGHUP (status {status:#x})"
    );
}

/// A real HUP re-applies the config through the shared path and the session
/// stays up: the gap moves, and a follow-up IPC reload reports two empty
/// lists -- the second diffing against what the HUP applied.
#[test]
fn sighup_reloads_the_config_and_the_session_stays_up() {
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[layout]\ngap = 20\n");
    fixture.raise_and_settle("the HUP reload to apply", |fixture| fixture.gap() == 20);
    assert_eq!(fixture.gap(), 20);

    match fixture.state.handle_request(scoot_ipc::Request::Reload) {
        Response::Reloaded { applied, refused } => assert!(
            applied.is_empty() && refused.is_empty(),
            "the IPC reload after the HUP should change nothing: {applied:?} / {refused:?}"
        ),
        other => panic!("the session did not survive its own HUP: {other:?}"),
    }
}

/// A HUP with a malformed file keeps the running config and keeps running:
/// the shared path's failure semantics, unchanged by the trigger.
#[test]
fn sighup_with_a_malformed_file_keeps_the_running_config() {
    let mut fixture = Fixture::with_config("[layout]\ngap = 20\n");
    fixture.raise_and_settle("the HUP reload to apply", |fixture| fixture.gap() == 20);

    fixture.rewrite("this is not valid toml [[[");
    // SAFETY: as in `raise_and_settle`.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    // Settle without a readiness predicate: a failed reload changes nothing
    // observable, so there is nothing to wait *for* -- only to prove the
    // session still dispatches afterwards.
    fixture.settle();
    assert_eq!(
        fixture.gap(),
        20,
        "the failed HUP reload moved the running gap"
    );
    match fixture.state.handle_request(scoot_ipc::Request::Reload) {
        Response::Error { .. } => {}
        other => panic!("a malformed file should still fail loudly: {other:?}"),
    }
}

/// A HUP with no config file on disk behaves exactly like the IPC path with
/// the same file missing: a loud error, the running config untouched --
/// because both triggers run the same `reload_from`, which is the point.
#[test]
fn sighup_with_a_vanished_config_file_keeps_the_running_config() {
    let mut fixture = Fixture::with_config("[layout]\ngap = 20\n");
    fixture.raise_and_settle("the HUP reload to apply", |fixture| fixture.gap() == 20);

    fs::remove_file(&fixture.path).expect("the config file removes");
    // SAFETY: as in `raise_and_settle`.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    fixture.settle();
    assert_eq!(
        fixture.gap(),
        20,
        "the failed HUP reload moved the running gap"
    );
}

/// Two HUPs back to back are safe either way the eventfd coalesces them:
/// one reload for the burst, or two sequential ones with the second seeing
/// the first's results. The end state is the same -- the gap applied, the
/// session alive.
#[test]
fn rapid_hup_hup_is_safe_coalesced_or_sequential() {
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[layout]\ngap = 20\n");
    // SAFETY: two real signals; the handler runs synchronously in each
    // `raise`, so both wakeups are pending before the first dispatch.
    unsafe {
        assert_eq!(libc::raise(libc::SIGHUP), 0);
        assert_eq!(libc::raise(libc::SIGHUP), 0);
    }
    fixture.raise_and_settle("the HUP reloads to apply", |fixture| fixture.gap() == 20);
    assert_eq!(fixture.gap(), 20);
    match fixture.state.handle_request(scoot_ipc::Request::Reload) {
        Response::Reloaded { applied, refused } => assert!(
            applied.is_empty() && refused.is_empty(),
            "the burst should settle coherent: {applied:?} / {refused:?}"
        ),
        other => panic!("the session did not survive a HUP burst: {other:?}"),
    }
}

/// A HUP racing an IPC reload funnels into the same `State::reload` on the
/// same loop thread: the two run sequentially, never inside each other, and
/// the second reports two empty lists.
#[test]
fn hup_racing_an_ipc_reload_runs_sequentially_on_the_shared_path() {
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[layout]\ngap = 20\n");
    match fixture.state.handle_request(scoot_ipc::Request::Reload) {
        Response::Reloaded { applied, .. } => assert!(
            applied.contains(&"layout.gap".to_owned()),
            "the IPC reload should apply first: {applied:?}"
        ),
        other => panic!("the IPC reload did not apply: {other:?}"),
    }
    // The HUP now races a file that agrees with live state: served, and a
    // no-op by the shared diff.
    fixture.raise_and_settle("the session to stay dispatchable", |_| true);
    match fixture.state.handle_request(scoot_ipc::Request::Reload) {
        Response::Reloaded { applied, refused } => assert!(
            applied.is_empty() && refused.is_empty(),
            "HUP and IPC should converge, not diverge: {applied:?} / {refused:?}"
        ),
        other => panic!("the session did not survive the race: {other:?}"),
    }
    assert_eq!(fixture.gap(), 20);
}

/// `SIGHUP` and `SIGCHLD` compose: both handlers installed on one loop, a
/// real child exiting and a real HUP arriving together, and each drain runs
/// its own half -- the child reaped, the config reloaded, neither lost.
#[test]
fn sighup_and_sigchld_do_not_interfere() {
    let mut fixture = Fixture::with_config("");
    crate::compositor::child_reaper::install(&fixture.event_loop.handle(), &mut fixture.state)
        .expect("the SIGCHLD reaper");
    fixture.state.spawn(&["true".to_string()]);
    assert_eq!(
        fixture.state.spawned_children.len(),
        1,
        "the spawn must be tracked before the race"
    );
    fixture.rewrite("[layout]\ngap = 20\n");
    // SAFETY: one real HUP against an exiting child; either order of arrival
    // must serve both halves.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    let deadline = Instant::now() + PATIENCE;
    while !(fixture.state.spawned_children.is_empty() && fixture.gap() == 20) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the reap and the reload (tracked: {:?}, gap: {})",
            fixture.state.spawned_children,
            fixture.gap(),
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
}

/// A spawned child starts with `SIGHUP` at `SIG_DFL` and an empty signal
/// mask.
///
/// Read from the child's own `/proc/self/status`, pinning what the module doc
/// promises: `SigIgn` carrying the bit would be an ignored disposition
/// surviving `exec` (breaking the child's own HUP handling -- shells,
/// `nohup`); `SigCgt` carrying it would mean the caught handler itself leaked
/// across `exec`, which the kernel resets for free; `SigBlk` carrying it
/// would be a blocked mask leaking through `Command::spawn`. Sensitivity is
/// proven below, in-test, by forcing `SIG_IGN` and watching the probe observe
/// it.
#[test]
fn a_spawned_child_sees_default_sighup_and_an_empty_mask() {
    /// The `SIGHUP` bit in a `/proc/self/status` signal mask: signal 1, and
    /// masks count from bit 0.
    const SIGHUP_BIT: u64 = 1 << (libc::SIGHUP as u32 - 1);

    fn probe(state: &mut State) -> String {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "scoot-spawn-sighup-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_file(&path);
        // The path travels as `$1`, so no filename ever goes through shell
        // quoting -- the same shape the reaper's suite uses.
        state.spawn(&[
            "sh".to_string(),
            "-c".to_string(),
            "grep -E '^(SigIgn|SigBlk|SigCgt):' /proc/self/status > \"$1\"".to_string(),
            "sh".to_string(),
            path.to_string_lossy().into_owned(),
        ]);
        let pids: Vec<u32> = state.spawned_children.iter().copied().collect();
        assert_eq!(pids.len(), 1, "the probe child must be tracked");
        let body = read_probe(&path);
        // SAFETY: `pid` is the exited probe child; a blocking `waitpid` on
        // it can only collect that child.
        let mut status = 0;
        let reaped = unsafe { libc::waitpid(pids[0] as libc::pid_t, &mut status, 0) };
        assert_eq!(
            reaped, pids[0] as libc::pid_t,
            "could not collect the probe child"
        );
        state.reap_children();
        assert!(state.spawned_children.is_empty());
        body
    }

    fn mask_of(body: &str, field: &str) -> u64 {
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
        u64::from_str_radix(hex.trim(), 16).unwrap_or_else(|_| {
            panic!("the probe child's {field} value is not hex ({hex:?}):\n{body}")
        })
    }

    let mut fixture = Fixture::with_config("");
    for line in ["SigIgn", "SigBlk", "SigCgt"] {
        let mask = mask_of(&probe(&mut fixture.state), line);
        assert_eq!(
            mask & SIGHUP_BIT,
            0,
            "a spawned child inherits SIGHUP in {line} (mask {mask:#x})"
        );
    }

    // Sensitivity: force `SIG_IGN` and watch the same probe observe it, so
    // the assertions above are known to bite. Restored afterwards, so the
    // shared `cargo test` process keeps its handler.
    let previous = sighup_disposition();
    let mut ignored: libc::sigaction = unsafe { std::mem::zeroed() };
    ignored.sa_sigaction = libc::SIG_IGN as *const () as usize;
    // SAFETY: an empty mask with `SIG_IGN` is always valid; restored below.
    unsafe {
        libc::sigemptyset(&mut ignored.sa_mask);
        assert_eq!(
            libc::sigaction(libc::SIGHUP, &ignored, std::ptr::null_mut()),
            0,
            "could not force SIG_IGN for the sensitivity probe"
        );
    }
    let observed = mask_of(&probe(&mut fixture.state), "SigIgn");
    // SAFETY: `previous` held this signal's disposition moments ago.
    unsafe {
        libc::sigaction(libc::SIGHUP, &previous, std::ptr::null_mut());
    }
    assert_ne!(
        observed & SIGHUP_BIT,
        0,
        "the probe cannot see an ignored SIGHUP -- the assertions above pass vacuously"
    );
}

/// A HUP applies while the session is locked, per the shared path's decision:
/// nothing in the applied set can disclose locked content, so refusing under
/// lock would strand an agent that edited the file mid-lock.
///
/// The lock is a real one -- the shared [`locker`] client, which sends the
/// lock request and parks holding it -- because `is_locked` is defined by a
/// live lock object, not a flag this test could set.
#[test]
fn sighup_applies_while_the_session_is_locked() {
    let _signal = SIGNAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut fixture: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "").expect("a config file");
    fixture.state.config_path = Some(path.clone());
    install(&fixture.event_loop.handle()).expect("the SIGHUP handler");

    let client = fixture.spawn(locker);
    fixture.wait_for_ack(client);
    let deadline = Instant::now() + PATIENCE;
    while !fixture.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the lock request never landed; the HUP test below would pass unlocked"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }

    fs::write(&path, "[layout]\ngap = 20\n").expect("a rewritten config file");
    // SAFETY: the handler is installed on this fixture's loop above.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    let deadline = Instant::now() + PATIENCE;
    while fixture.state.world.config().gap != 20 {
        assert!(
            Instant::now() < deadline,
            "the HUP reload did not apply under lock"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    assert!(
        fixture.state.session_lock.is_locked(),
        "the HUP reload must not disturb the lock it applied under"
    );
}

/// A HUP runs new autostart entries through the same spawn delta as an IPC
/// reload: the shared `State::reload` owns the policy, not the trigger.
/// (The IPC half pins the exact reply lists; here the marker file is the
/// assertion, since a HUP has no reply channel.)
#[test]
fn hup_runs_new_autostart_entries_through_the_shared_delta() {
    let marker = marker_path("hup-new");
    let entry = touch_entry(&marker);
    let mut fixture = Fixture::with_config(&format!("[autostart]\ncommands = [\"{entry}\"]\n"));
    // SAFETY: as in `raise_and_settle`.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    fixture.settle();
    wait_for_marker(&marker);
    assert_eq!(
        fixture.state.startup_autostart,
        vec![Action::Spawn(vec![
            "touch".to_owned(),
            marker.to_string_lossy().into_owned()
        ])],
        "the HUP must advance the snapshot like an IPC reload does"
    );
}

/// ...while a HUP must not re-run old ones: entries the snapshot already
/// holds stay silent, so a HUP on an unchanged file spawns nothing. The
/// snapshot is seeded here to what `run` would have written had the session
/// started from this file (startup drained them).
#[test]
fn hup_does_not_rerun_old_autostart_entries() {
    let marker = marker_path("hup-old");
    let entry = touch_entry(&marker);
    let mut fixture = Fixture::with_config(&format!("[autostart]\ncommands = [\"{entry}\"]\n"));
    fixture.state.startup_autostart = vec![Action::Spawn(vec![
        "touch".to_owned(),
        marker.to_string_lossy().into_owned(),
    ])];
    // SAFETY: as in `raise_and_settle`.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    fixture.settle();
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, refused } => assert!(
            applied.is_empty() && refused.is_empty(),
            "the HUP should have decided nothing new: {applied:?} / {refused:?}"
        ),
        other => panic!("the session did not survive its own HUP: {other:?}"),
    }
    assert_marker_never_appears(&marker);
}

/// A HUP under lock shares the skip-and-defer: new spawn entries neither
/// run, nor advance the snapshot -- the first unlocked reload runs them.
#[test]
fn hup_with_new_spawn_entries_skips_while_locked() {
    let _signal = SIGNAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut fixture: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "").expect("a config file");
    fixture.state.config_path = Some(path.clone());
    install(&fixture.event_loop.handle()).expect("the SIGHUP handler");

    let client = fixture.spawn(locker);
    fixture.wait_for_ack(client);
    let deadline = Instant::now() + PATIENCE;
    while !fixture.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the lock request never landed; the HUP below would pass unlocked"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }

    let marker = marker_path("hup-locked");
    let entry = touch_entry(&marker);
    fs::write(&path, format!("[autostart]\ncommands = [\"{entry}\"]\n"))
        .expect("a rewritten config file");
    // SAFETY: the handler is installed on this fixture's loop above.
    let raised = unsafe { libc::raise(libc::SIGHUP) };
    assert_eq!(raised, 0, "raising SIGHUP at the test process failed");
    fixture.tick(Duration::from_millis(500));
    assert_marker_never_appears(&marker);
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "a locked HUP must not advance the snapshot -- the entry stays pending"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "the HUP must not disturb the lock it skipped under"
    );
}

impl Fixture {
    /// A few dispatch cycles with nothing outstanding, so the loop has served
    /// the wake source and the session's aliveness is what is asserted next.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }
}

/// Waits for the probe child's file to appear with content, reads it, and
/// removes the file.
///
/// Content, not mere existence: the shell creates the file before writing to
/// it, so an early read would see an empty file and assert on nothing.
fn read_probe(path: &Path) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(body) = fs::read_to_string(path)
            && !body.is_empty()
        {
            let _ = fs::remove_file(path);
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "the spawned child never wrote its status file: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
