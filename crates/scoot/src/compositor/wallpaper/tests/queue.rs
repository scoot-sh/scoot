//! The run queue's rules, one per test: one run at a time, newest wins, the
//! 40 s bound, a failure held for the next trigger (N1), a late run followed
//! by the newest, and the cap on abandoned runs.

use std::collections::HashMap;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::compositor::wallpaper::queue::{
    Ended, Exit, MAX_ABANDONED, PATIENCE, Queue, Waited, Want,
};

/// Raw `waitpid` statuses, as the kernel encodes them.
const EXIT_0: i32 = 0;
const EXIT_1: i32 = 1 << 8;
const EXIT_2: i32 = 2 << 8;
const KILLED: i32 = libc::SIGKILL;

/// A queue driven the way the glue drives it, with fake pids and a fake
/// process table.
struct Driver {
    queue: Queue,
    now: Instant,
    next_pid: u32,
    /// pid -> the JSON it ran, for every run started, in order.
    started: Vec<(u32, String)>,
    /// What `waitpid` answers per pid; absent means still running.
    exits: HashMap<u32, Waited>,
    ended: Vec<Ended>,
}

impl Driver {
    fn new() -> Self {
        Self {
            queue: Queue::default(),
            now: Instant::now(),
            next_pid: 100,
            started: Vec::new(),
            exits: HashMap::new(),
            ended: Vec::new(),
        }
    }

    /// Starts whatever the queue names (as `drive_wallpaper` does), and
    /// returns its pid.
    fn drive(&mut self) -> Option<u32> {
        let json = self.queue.next()?.json.clone();
        self.next_pid += 1;
        let pid = self.next_pid;
        self.queue.started(pid, self.now);
        self.started.push((pid, json));
        Some(pid)
    }

    fn submit(&mut self, json: &str) -> Option<u32> {
        self.queue
            .submit(OsString::from("scootbg"), json.to_owned());
        self.drive()
    }

    fn exit(&mut self, pid: u32, status: i32) -> Option<u32> {
        self.exits.insert(pid, Waited::Exited(status));
        self.reap()
    }

    fn reap(&mut self) -> Option<u32> {
        let exits = &mut self.exits;
        let ended = &mut self.ended;
        self.queue.reap(
            |pid| exits.remove(&pid).unwrap_or(Waited::Running),
            |end| ended.push(end),
        );
        self.drive()
    }

    fn advance(&mut self, by: Duration) {
        self.now += by;
    }

    fn jsons(&self) -> Vec<&str> {
        self.started.iter().map(|(_, json)| json.as_str()).collect()
    }
}

#[test]
fn exit_statuses_decode() {
    assert_eq!(Exit::from_status(EXIT_0), Exit::Success);
    assert_eq!(Exit::from_status(EXIT_1), Exit::Code(1));
    assert_eq!(Exit::from_status(EXIT_2), Exit::Code(2));
    assert_eq!(Exit::from_status(KILLED), Exit::Signal(libc::SIGKILL));
}

#[test]
fn nothing_runs_until_something_is_submitted() {
    let driver = Driver::new();
    assert!(driver.queue.next().is_none());
    assert!(driver.queue.is_empty());
    assert_eq!(driver.queue.deadline(), None);
}

#[test]
fn a_section_runs_at_once_when_nothing_is_in_flight() {
    let mut driver = Driver::new();
    let pid = driver.submit("A").expect("A starts");
    assert_eq!(driver.queue.running_pid(), Some(pid));
    assert!(driver.queue.next().is_none(), "one run at a time");
    assert_eq!(driver.exit(pid, EXIT_0), None, "nothing more to run");
    assert!(driver.queue.is_empty());
    assert_eq!(driver.queue.want(), Want::Idle);
    assert_eq!(driver.ended.len(), 1);
    assert_eq!(driver.ended[0].exit, Exit::Success);
    assert_eq!(driver.ended[0].command, OsString::from("scootbg"));
    assert!(!driver.ended[0].abandoned);
}

/// Two reloads while a run is in flight: only the newer of the two waiting
/// sections runs, after the one in flight, never beside it.
#[test]
fn only_the_newest_waiting_section_runs_next() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    assert_eq!(driver.submit("B"), None, "B waits for A");
    assert_eq!(driver.submit("C"), None, "C replaces B");
    let c = driver.exit(a, EXIT_0).expect("C starts once A exits");
    assert_eq!(driver.jsons(), ["A", "C"], "B never ran");
    assert_eq!(driver.exit(c, EXIT_0), None);
}

/// N1: a failed run of the newest section keeps it queued, not retried in a
/// loop at once, but on the next trigger.
#[test]
fn a_failed_run_is_held_for_the_next_trigger() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    assert_eq!(driver.exit(a, EXIT_1), None, "no immediate retry");
    assert_eq!(driver.queue.want(), Want::Held);
    driver.queue.trigger();
    let retry = driver.drive().expect("a trigger retries it");
    assert_eq!(driver.jsons(), ["A", "A"]);
    assert_eq!(driver.exit(retry, EXIT_0), None);
    assert_eq!(driver.queue.want(), Want::Idle);
}

/// A refused section (2) and a killed run are failures like any other.
#[test]
fn every_nonzero_end_is_a_failure() {
    for status in [EXIT_2, KILLED, 3 << 8] {
        let mut driver = Driver::new();
        let a = driver.submit("A").unwrap();
        driver.exit(a, status);
        assert_eq!(driver.queue.want(), Want::Held, "status {status:#x}");
    }
    // A pid reaped elsewhere ends with no status: not known to have applied.
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.exits.insert(a, Waited::Gone);
    driver.reap();
    assert_eq!(driver.ended[0].exit, Exit::Unknown);
    assert_eq!(driver.queue.want(), Want::Held);
}

/// A newer section supersedes a held one: it runs, the held one never does.
#[test]
fn a_new_section_supersedes_a_held_one() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.exit(a, EXIT_1);
    assert!(driver.submit("B").is_some());
    assert_eq!(driver.jsons(), ["A", "B"]);
}

/// An older run failing while a newer section waits does not hold the
/// newer one back.
#[test]
fn a_failed_older_run_does_not_hold_the_newer_section() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.submit("B");
    assert!(driver.exit(a, EXIT_1).is_some(), "B starts");
    assert_eq!(driver.jsons(), ["A", "B"]);
}

#[test]
fn a_spawn_that_fails_is_held_for_the_next_trigger() {
    let mut driver = Driver::new();
    driver
        .queue
        .submit(OsString::from("scootbg"), "A".to_owned());
    assert!(driver.queue.next().is_some());
    driver.queue.spawn_failed();
    assert!(driver.queue.next().is_none());
    assert_eq!(driver.queue.want(), Want::Held);
    driver.queue.trigger();
    assert!(driver.drive().is_some());
}

/// F4: a run is waited on for 40 s at most, then the newest queued section
/// runs; the abandoned one is still reaped and logged.
#[test]
fn a_run_is_abandoned_after_the_bound() {
    let mut driver = Driver::new();
    let a = driver.submit("{}").unwrap();
    assert_eq!(driver.queue.deadline(), Some(driver.now + PATIENCE));
    driver.submit("B");
    driver.advance(PATIENCE - Duration::from_millis(1));
    assert_eq!(driver.queue.expire(driver.now), None, "not yet");
    driver.advance(Duration::from_millis(1));
    assert_eq!(driver.queue.expire(driver.now), Some(a));
    assert_eq!(driver.queue.abandoned_count(), 1);
    let b = driver.drive().expect("B starts once A is abandoned");
    assert_eq!(driver.queue.running_pid(), Some(b));
    assert!(!driver.queue.is_empty());
    // The abandoned run still ends, and is logged as such.
    driver.exit(b, EXIT_0);
    driver.exit(a, EXIT_0);
    assert!(driver.ended.iter().any(|end| end.pid == a && end.abandoned));
}

/// With nothing queued, abandoning a run starts nothing.
#[test]
fn abandoning_the_only_run_starts_nothing() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.advance(PATIENCE);
    assert_eq!(driver.queue.expire(driver.now), Some(a));
    assert!(driver.drive().is_none());
    assert_eq!(driver.queue.deadline(), None);
}

/// N1 end to end: `{}` hangs in `fsync` holding the lock; scoot moves on;
/// the next reload's run fails (lock held, no daemon); when the hung run
/// finally ends, the failed section is retried and lands last.
#[test]
fn the_hung_clear_then_a_failed_reload_then_the_retry() {
    let mut driver = Driver::new();
    let clear = driver.submit("{}").unwrap();
    driver.advance(PATIENCE);
    assert_eq!(driver.queue.expire(driver.now), Some(clear));
    let b = driver.submit("B").expect("B runs once {} is abandoned");
    assert_eq!(driver.exit(b, EXIT_1), None, "B failed: held, no loop");
    assert_eq!(driver.queue.want(), Want::Held);
    let retry = driver
        .exit(clear, EXIT_0)
        .expect("the hung run ending is a trigger: B again");
    assert_eq!(driver.jsons(), ["{}", "B", "B"]);
    assert_eq!(driver.exit(retry, EXIT_0), None);
    assert_eq!(driver.queue.want(), Want::Idle);
}

/// An abandoned older run that ends after the newest was applied may have
/// landed last: the newest runs again.
#[test]
fn a_late_older_run_is_followed_by_the_newest() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.advance(PATIENCE);
    driver.queue.expire(driver.now);
    let b = driver.submit("B").unwrap();
    assert_eq!(driver.exit(b, EXIT_0), None, "B applied");
    let again = driver.exit(a, EXIT_0).expect("A ended late: B again");
    assert_eq!(driver.jsons(), ["A", "B", "B"]);
    assert_eq!(driver.exit(again, EXIT_0), None);
}

/// ...even when that older run failed: its failure may have been partial.
#[test]
fn a_late_older_run_that_failed_is_followed_by_the_newest_too() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.advance(PATIENCE);
    driver.queue.expire(driver.now);
    let b = driver.submit("B").unwrap();
    driver.exit(b, EXIT_0);
    assert!(driver.exit(a, EXIT_1).is_some());
}

/// An abandoned run of the newest section that ends well means it is
/// applied: nothing more runs.
#[test]
fn a_late_success_of_the_newest_is_enough() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.advance(PATIENCE);
    driver.queue.expire(driver.now);
    assert_eq!(driver.queue.want(), Want::Idle);
    assert_eq!(
        driver.exit(a, EXIT_0),
        None,
        "A applied, late: nothing to do"
    );
    assert_eq!(driver.queue.want(), Want::Idle);
}

/// An abandoned run of the newest section that fails holds it, to be
/// retried on the next trigger (not immediately: it was not a trigger for
/// itself).
#[test]
fn a_late_failure_of_the_newest_is_held() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.advance(PATIENCE);
    driver.queue.expire(driver.now);
    assert_eq!(driver.exit(a, EXIT_1), None);
    assert_eq!(driver.queue.want(), Want::Held);
    driver.queue.trigger();
    assert!(driver.drive().is_some());
}

/// Each abandoned run is a live process: at the cap nothing new starts,
/// and one ending lets the next section run.
#[test]
fn abandoned_runs_are_capped() {
    let mut driver = Driver::new();
    let mut hung = Vec::new();
    for round in 0..MAX_ABANDONED {
        let pid = driver
            .submit(&format!("S{round}"))
            .expect("under the cap, each section runs");
        driver.advance(PATIENCE);
        assert_eq!(driver.queue.expire(driver.now), Some(pid));
        hung.push(pid);
    }
    assert_eq!(driver.queue.abandoned_count(), MAX_ABANDONED);
    assert_eq!(driver.submit("next"), None, "at the cap nothing starts");
    assert_eq!(driver.queue.want(), Want::Now, "but it stays wanted");
    assert!(
        driver.exit(hung[0], EXIT_0).is_some(),
        "one abandoned run ending lets the newest run"
    );
    assert_eq!(driver.started.last().unwrap().1, "next");
}

/// Reaping asks only about tracked pids and reports each end once.
#[test]
fn reaping_reports_each_end_once() {
    let mut driver = Driver::new();
    let a = driver.submit("A").unwrap();
    driver.reap();
    assert!(driver.ended.is_empty(), "still running: nothing reported");
    driver.exit(a, EXIT_0);
    driver.reap();
    assert_eq!(driver.ended.len(), 1);
    // An exit reported for a pid never tracked is never asked about.
    driver.exits.insert(9999, Waited::Exited(EXIT_1));
    driver.reap();
    assert_eq!(driver.ended.len(), 1);
    assert!(
        driver.exits.contains_key(&9999),
        "an untracked pid is not waited on"
    );
}
