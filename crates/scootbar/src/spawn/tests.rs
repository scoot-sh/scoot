use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, Timespec, poll};

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

/// A scratch directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scootbar-spawn-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

/// Waits until every child has exited, reaping through the loop's own
/// path: poll the pidfds, then `reap`.
fn drain(spawner: &mut Spawner) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while spawner.running() > 0 {
        assert!(Instant::now() < deadline, "children never exited");
        let placeholder = rustix::fs::CWD;
        let mut fds: [PollFd<'_>; MAX_CHILDREN] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
        let mut len = 0;
        spawner.sources(&mut fds, &mut len);
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        let _ = poll(&mut fds[..len], Some(&timeout));
        spawner.reap();
    }
}

/// Whether a process with this pid still has a table entry: a zombie
/// has one, a reaped child does not.
fn exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[test]
fn a_command_runs_with_its_arguments_and_no_shell() {
    let dir = Dir::new("noshell");
    // A shell would split this at the space and treat `;` as a separator.
    let name = dir.path("a b;c");
    let mut spawner = Spawner::default();
    spawner
        .spawn(&argv(&["touch", name.to_str().expect("utf-8 path")]))
        .expect("spawned");
    drain(&mut spawner);
    assert!(name.exists(), "the argument was split or interpreted");
}

#[test]
fn an_unknown_program_is_an_error_naming_it() {
    let mut spawner = Spawner::default();
    let error = spawner
        .spawn(&argv(&["/nonexistent/scootbar-test-program"]))
        .expect_err("not found");
    assert!(error.contains("scootbar-test-program"), "{error}");
    assert_eq!(spawner.running(), 0);
}

#[test]
fn an_empty_command_is_an_error() {
    let mut spawner = Spawner::default();
    assert!(spawner.spawn(&[]).is_err());
}

#[test]
fn an_exited_child_is_reaped_not_left_a_zombie() {
    let mut spawner = Spawner::default();
    spawner.spawn(&argv(&["true"])).expect("spawned");
    let pid = spawner.live[0].child.id();
    drain(&mut spawner);
    assert_eq!(spawner.running(), 0);
    assert!(!exists(pid), "pid {pid} is still in the process table");
}

#[test]
fn the_pidfd_becomes_ready_when_the_child_exits() {
    let mut spawner = Spawner::default();
    spawner.spawn(&argv(&["true"])).expect("spawned");
    {
        let placeholder = rustix::fs::CWD;
        let mut fds: [PollFd<'_>; MAX_CHILDREN] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
        let mut len = 0;
        spawner.sources(&mut fds, &mut len);
        assert_eq!(len, 1, "one child, one fd");
        let timeout = Timespec {
            tv_sec: 10,
            tv_nsec: 0,
        };
        assert_eq!(poll(&mut fds[..len], Some(&timeout)).expect("poll"), 1);
        assert!(fds[0].revents().contains(PollFlags::IN));
    }
    spawner.reap();
    assert_eq!(spawner.running(), 0);
    assert_eq!(spawner.poll_timeout(), None, "pidfds need no timer");
}

#[test]
fn a_running_child_is_not_reaped_early() {
    let mut spawner = Spawner::default();
    spawner.spawn(&argv(&["sleep", "30"])).expect("spawned");
    spawner.reap();
    assert_eq!(spawner.running(), 1);
    kill_all(&mut spawner);
}

#[test]
fn without_pidfds_children_are_reaped_on_a_timeout() {
    let mut spawner = Spawner {
        pidfds: false,
        ..Spawner::default()
    };
    assert_eq!(spawner.poll_timeout(), None, "nothing running, no timer");
    spawner.spawn(&argv(&["true"])).expect("spawned");
    let pid = spawner.live[0].child.id();
    assert_eq!(spawner.poll_timeout(), Some(FALLBACK_POLL));
    let mut len = 0;
    let placeholder = rustix::fs::CWD;
    let mut fds: [PollFd<'_>; 2] =
        std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
    spawner.sources(&mut fds, &mut len);
    assert_eq!(len, 0, "no pidfd to poll");
    wait_for("the fallback reap", || {
        spawner.reap();
        spawner.running() == 0
    });
    assert!(!exists(pid));
    assert_eq!(spawner.poll_timeout(), None, "idle again: no timer");
}

fn kill_all(spawner: &mut Spawner) {
    for live in &mut spawner.live {
        let _ = live.child.kill();
    }
    drain(spawner);
}

#[test]
fn the_number_of_children_is_bounded() {
    let mut spawner = Spawner::default();
    for _ in 0..MAX_CHILDREN {
        spawner
            .spawn(&argv(&["sleep", "30"]))
            .expect("under the cap");
    }
    let error = spawner
        .spawn(&argv(&["sleep", "30"]))
        .expect_err("over the cap");
    assert!(error.contains("sleep"), "{error}");
    assert_eq!(spawner.running(), MAX_CHILDREN);
    // A slot frees when a child exits, without anyone asking.
    let pid = spawner.live[0].child.id();
    let _ = spawner.live[0].child.kill();
    wait_for("the killed child to be reaped", || {
        spawner.reap();
        !exists(pid)
    });
    spawner
        .spawn(&argv(&["sleep", "30"]))
        .expect("a slot is free");
    kill_all(&mut spawner);
}

#[test]
fn a_flood_of_short_commands_never_grows_the_table() {
    let mut spawner = Spawner::default();
    let mut refused = 0;
    for _ in 0..200 {
        if spawner.spawn(&argv(&["true"])).is_err() {
            refused += 1;
        }
        assert!(spawner.running() <= MAX_CHILDREN);
    }
    drain(&mut spawner);
    assert!(refused < 200, "nothing ever ran");
}

#[test]
fn the_child_leads_its_own_process_group() {
    let dir = Dir::new("pgrp");
    let out = dir.path("out");
    let script = format!(
        "ps -o pgid= -p $$ > {}; ps -o pid= -p $$ >> {}",
        out.to_str().expect("utf-8 path"),
        out.to_str().expect("utf-8 path")
    );
    let mut spawner = Spawner::default();
    spawner
        .spawn(&argv(&["sh", "-c", &script]))
        .expect("spawned");
    drain(&mut spawner);
    let text = std::fs::read_to_string(&out).expect("output");
    let mut numbers = text.split_whitespace();
    let (pgid, pid) = (numbers.next(), numbers.next());
    assert!(pgid.is_some() && pgid == pid, "pgid {pgid:?}, pid {pid:?}");
}

/// The descriptor numbers a `sh` child sees, with `held` open in this
/// process (the child's own `ls` opens its directory as 3).
fn child_fds(dir: &Dir) -> Vec<i32> {
    let out = dir.path("out");
    let script = format!("ls /proc/self/fd > {}", out.to_str().expect("utf-8 path"));
    let mut spawner = Spawner::default();
    spawner
        .spawn(&argv(&["sh", "-c", &script]))
        .expect("spawned");
    drain(&mut spawner);
    std::fs::read_to_string(&out)
        .expect("output")
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

#[test]
fn a_child_inherits_none_of_the_callers_descriptors() {
    // A descriptor opened the way `std` and `rustix` open everything here
    // is close-on-exec, so the child sees only stdio and what it opened
    // itself. (The running bar's own descriptors are checked by
    // `tests/exec.rs`.)
    let dir = Dir::new("fds");
    let file = std::fs::File::open("/dev/null").expect("open");
    let held = rustix::io::fcntl_dupfd_cloexec(&file, 100).expect("dup");
    let fds = child_fds(&dir);
    assert!(!fds.contains(&held.as_raw_fd()), "fd leaked: {fds:?}");
    assert!(fds.iter().all(|&fd| fd <= 3), "{fds:?}");
}

#[test]
fn the_fd_check_sees_a_descriptor_that_is_inherited() {
    // The control: a descriptor without close-on-exec does reach the
    // child, so the test above would have failed on one.
    let dir = Dir::new("fds-control");
    let file = std::fs::File::open("/dev/null").expect("open");
    let leaked = rustix::io::fcntl_dupfd_cloexec(&file, 100).expect("dup");
    rustix::io::fcntl_setfd(&leaked, rustix::io::FdFlags::empty()).expect("clear CLOEXEC");
    let fds = child_fds(&dir);
    assert!(fds.contains(&leaked.as_raw_fd()), "{fds:?}");
}
