//! The `exec` module against real children (`sh`, `printf`, `yes`,
//! `sleep`, which every test machine has), through the harness: what it
//! shows, how it is bounded, how it restarts, and what it leaves behind.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;
use crate::modules::harness::Harness;
use crate::modules::{Class, MAX_TEXT};

/// A scratch directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("scootbar-exec-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Fast restarts, so a test does not wait seconds.
const FAST: Restart = Restart {
    first: Duration::from_millis(20),
    max: Duration::from_millis(80),
    stable: Duration::from_secs(10),
};

fn settings(script: &str, format: Format) -> Settings {
    Settings {
        command: vec!["sh".into(), "-c".into(), script.into()],
        format,
        placeholder: "ph".into(),
        restart: FAST,
        icon: None,
        show_text: true,
    }
}

fn module(script: &str, format: Format) -> Harness {
    Harness::new(start("test", &settings(script, format)).expect("starts"))
}

/// Starting a module, as `scootbar daemon --check` does, runs nothing: the
/// command is started when the loop first wakes the module.
#[test]
fn starting_a_module_runs_no_command() {
    let dir = Dir::new("start-runs-nothing");
    let marker = dir.file("ran");
    let script = format!("touch {}; echo 1", marker.display());
    let started = start("test", &settings(&script, Format::Text)).expect("starts");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!marker.exists(), "start ran the command");
    drop(started);
    assert!(!marker.exists());
}

/// Drives the module until `done` holds, or fails.
fn until(harness: &mut Harness, what: &str, mut done: impl FnMut(&Harness) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(harness) {
        assert!(Instant::now() < deadline, "never {what}");
        let _ = harness.wait(Duration::from_millis(50));
    }
}

fn shows(harness: &Harness, text: &str) -> bool {
    harness.view().text() == text
}

/// The pid a test command wrote to `file` (`echo $$ > file`).
fn pid_in(file: &std::path::Path) -> u32 {
    fs::read_to_string(file)
        .expect("the command wrote its pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// Whether process `pid` is gone from the process table: a zombie is not.
fn gone(pid: u32) -> bool {
    !std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Whether a process runs with exactly this command line (its arguments
/// joined by spaces).
fn running(cmdline: &str) -> bool {
    for entry in fs::read_dir("/proc").expect("proc").flatten() {
        let Ok(raw) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let text: Vec<u8> = raw.iter().map(|&b| if b == 0 { b' ' } else { b }).collect();
        if String::from_utf8_lossy(&text).trim_end() == cmdline {
            return true;
        }
    }
    false
}

// ---- what it shows ----

#[test]
fn it_shows_the_placeholder_then_the_first_line() {
    let mut harness = module("sleep 0.2; echo hello; sleep 30", Format::Text);
    assert_eq!(harness.view().text(), "ph");
    until(&mut harness, "showed the line", |h| shows(h, "hello"));
}

#[test]
fn the_last_line_of_a_read_is_what_is_shown() {
    let mut harness = module("printf 'a\\nb\\nc\\n'; sleep 30", Format::Text);
    until(&mut harness, "showed c", |h| shows(h, "c"));
}

#[test]
fn json_lines_set_the_class_and_tooltip() {
    let mut harness = module(
        r#"echo '{"text":"72%","class":"warn","tooltip":"low"}'; sleep 30"#,
        Format::Json,
    );
    until(&mut harness, "showed the json", |h| shows(h, "72%"));
    let view = harness.view();
    assert_eq!((view.class(), view.tooltip()), (Class::Warn, "low"));
}

#[test]
fn a_bad_line_is_ignored_and_what_was_shown_stays() {
    let mut harness = module(
        r#"echo '{"text":"ok"}'; sleep 0.1; echo 'not json'; echo '{"class":"loud"}'; sleep 30"#,
        Format::Json,
    );
    until(&mut harness, "showed ok", |h| shows(h, "ok"));
    // Give the two bad lines time to arrive and be refused.
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        let _ = harness.wait(Duration::from_millis(50));
    }
    assert_eq!(harness.view().text(), "ok");
}

#[test]
fn a_line_with_no_final_newline_is_shown_when_the_command_exits() {
    let mut harness = module("printf partial", Format::Text);
    until(&mut harness, "showed partial", |h| shows(h, "partial"));
}

#[test]
fn control_characters_in_the_output_never_reach_the_view() {
    let mut harness = module("printf '\\033[31mred\\tx\\r\\n'; sleep 30", Format::Text);
    until(&mut harness, "showed the line", |h| {
        h.view().text().contains("red")
    });
    assert!(!harness.view().text().chars().any(char::is_control));
    assert!(harness.view().text().len() <= MAX_TEXT);
}

#[test]
fn its_stdin_is_dev_null() {
    // A command that reads its stdin gets the end of file at once, not a
    // hang on the bar's own.
    let mut harness = module("read x; echo \"got:$x\"; sleep 30", Format::Text);
    until(&mut harness, "read the end of file", |h| shows(h, "got:"));
}

// ---- bounds ----

#[test]
fn a_line_past_the_bound_is_dropped_whole_and_the_next_one_shows() {
    let long = lines::MAX_LINE * 3;
    let script = format!("head -c {long} /dev/zero | tr '\\0' x; echo; echo after; sleep 30");
    let mut harness = module(&script, Format::Text);
    until(&mut harness, "showed the line after", |h| shows(h, "after"));
}

#[test]
fn a_flood_is_held_to_a_read_a_frame() {
    // `yes` prints as fast as the pipe takes it. The bar polls the pipe at
    // most once a frame, so the wakes in 400 ms are about 25, not tens of
    // thousands.
    let mut harness = module("yes", Format::Text);
    until(&mut harness, "started", |h| shows(h, "y"));
    let end = Instant::now() + Duration::from_millis(400);
    let mut wakes = 0;
    while Instant::now() < end {
        if harness.wait(Duration::from_millis(50)).is_some() {
            wakes += 1;
        }
    }
    let frames = 400 / FRAME.as_millis() as u32;
    assert!(wakes <= frames * 2 + 10, "{wakes} wakes in 400 ms");
    assert!(wakes >= 1, "never woke");
}

#[test]
fn a_hostile_stream_of_unterminated_text_holds_one_line_at_most() {
    // `Lines` alone: 256 MiB with no newline.
    let mut lines = Lines::default();
    let capacity = lines.capacity();
    let chunk = [b'x'; CHUNK];
    let mut delivered = 0;
    for _ in 0..(256 * 1024 * 1024 / CHUNK) {
        lines.feed(&chunk, |_| delivered += 1);
        assert!(lines.held() <= lines::MAX_LINE);
    }
    assert_eq!(delivered, 0, "a line of 256 MiB was delivered");
    assert_eq!(lines.capacity(), capacity, "the buffer grew");
    assert_eq!(lines.take_dropped(), 1);
    // The newline ends the dropped line; the next one is whole.
    let mut got = Vec::new();
    lines.feed(b"\nnext\n", |line| got.push(line.to_vec()));
    assert_eq!(got, [b"next".to_vec()]);
}

// ---- lines (pure) ----

#[test]
fn lines_split_across_chunks_and_a_line_at_the_bound_is_kept() {
    let mut lines = Lines::default();
    let mut got = Vec::new();
    for chunk in [&b"he"[..], b"llo\nwor", b"ld\n\nlast"] {
        lines.feed(chunk, |line| {
            got.push(String::from_utf8_lossy(line).into_owned());
        });
    }
    assert_eq!(got, ["hello", "world", ""]);
    lines.finish(|line| got.push(String::from_utf8_lossy(line).into_owned()));
    assert_eq!(got.last().map(String::as_str), Some("last"));
    // Exactly at the bound is kept; one past is dropped.
    let exact = vec![b'x'; lines::MAX_LINE];
    let mut kept = 0;
    lines.feed(&exact, |_| {});
    lines.feed(b"\n", |line| kept = line.len());
    assert_eq!(kept, lines::MAX_LINE);
    let over = vec![b'x'; lines::MAX_LINE + 1];
    let mut any = false;
    lines.feed(&over, |_| any = true);
    lines.feed(b"\n", |_| any = true);
    assert!(!any);
    assert_eq!(lines.take_dropped(), 1);
}

#[test]
fn a_dropped_line_cut_by_exit_stays_dropped() {
    let mut lines = Lines::default();
    lines.feed(&vec![b'x'; lines::MAX_LINE + 10], |_| {});
    let mut any = false;
    lines.finish(|_| any = true);
    assert!(!any);
}

#[test]
fn feeding_lines_allocates_nothing() {
    let mut lines = Lines::default();
    let chunk = b"a line\nanother one\npartial";
    lines.feed(chunk, |_| {});
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for _ in 0..1000 {
            lines.feed(chunk, |_| {});
            lines.feed(&[b'x'; 5000], |_| {});
            lines.feed(b"\n", |_| {});
        }
    });
    assert_eq!(allocations, 0);
}

// ---- restart ----

#[test]
fn the_wait_doubles_to_the_cap_and_a_stable_run_resets_it() {
    let policy = Restart {
        first: Duration::from_secs(1),
        max: Duration::from_secs(10),
        stable: Duration::from_secs(30),
    };
    let mut backoff = Backoff::new(policy);
    let quick = Duration::from_millis(5);
    let waits: Vec<u64> = (0..6).map(|_| backoff.after(quick).as_secs()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 10, 10]);
    // A run that lasted resets it, and that wait is the first again.
    assert_eq!(backoff.after(Duration::from_secs(30)).as_secs(), 1);
    assert_eq!(backoff.after(quick).as_secs(), 2);
    // Just under stable does not.
    assert_eq!(backoff.after(Duration::from_secs(29)).as_secs(), 4);
}

#[test]
fn the_wait_never_overflows() {
    let mut backoff = Backoff::new(Restart {
        first: Duration::MAX / 2 + Duration::from_secs(1),
        max: Duration::MAX,
        stable: Duration::MAX,
    });
    for _ in 0..8 {
        let _ = backoff.after(Duration::ZERO);
    }
}

#[test]
fn a_command_that_exits_is_started_again_with_a_growing_wait() {
    let dir = Dir::new("restart");
    let log = dir.file("runs");
    let mut harness = module(&format!("echo run >> {}", log.display()), Format::Text);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        let _ = harness.wait(Duration::from_millis(20));
    }
    let runs = fs::read_to_string(&log).unwrap_or_default().lines().count();
    // 0, +20, +40, +80, +80, ...: about 9 in 700 ms, and far from a spin.
    assert!((4..=12).contains(&runs), "{runs} runs in 700 ms");
}

#[test]
fn a_command_that_cannot_start_is_retried_and_the_module_stays_up() {
    let settings = Settings {
        command: vec!["/nonexistent/scootbar-exec-test".into()],
        format: Format::Text,
        placeholder: "ph".into(),
        restart: FAST,
        icon: None,
        show_text: true,
    };
    let mut harness = Harness::new(start("test", &settings).expect("starts"));
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        let _ = harness.wait(Duration::from_millis(20));
    }
    // Still waiting for its timer, still showing the placeholder.
    assert_eq!(harness.view().text(), "ph");
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn an_empty_command_is_a_refusal_not_a_panic() {
    let settings = Settings {
        command: Vec::new(),
        format: Format::Text,
        placeholder: String::new(),
        restart: FAST,
        icon: None,
        show_text: true,
    };
    let mut harness = Harness::new(start("test", &settings).expect("starts"));
    let _ = harness.wait(Duration::from_millis(100));
    assert!(harness.view().is_empty());
}

// ---- what it leaves behind ----

#[test]
fn a_child_that_exited_is_reaped_not_left_a_zombie() {
    let dir = Dir::new("reap");
    let pidfile = dir.file("pid");
    let mut harness = module(
        &format!("echo $$ > {}; echo done", pidfile.display()),
        Format::Text,
    );
    until(&mut harness, "ran the command", |h| shows(h, "done"));
    let pid = pid_in(&pidfile);
    // Its exit wakes the module through the pidfd, and the wake reaps it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gone(pid) {
        assert!(
            Instant::now() < deadline,
            "pid {pid} is still in the process table"
        );
        let _ = harness.wait(Duration::from_millis(20));
    }
}

#[test]
fn a_child_holds_none_of_the_callers_descriptors() {
    // Not "nothing above stdio": this process may itself have been started
    // with descriptors open (a CI runner does), which every child inherits.
    let _serial = crate::testfds::serialize();
    let dir = Dir::new("fds");
    let out = dir.file("fds");
    let held = std::fs::File::open("/dev/null").expect("open");
    let fd = rustix::io::fcntl_dupfd_cloexec(&held, 100).expect("dup");
    let inherited = crate::testfds::inheritable();
    let script = format!(
        "ls /proc/self/fd > {}; echo listed; sleep 30",
        out.display()
    );
    let mut harness = module(&script, Format::Text);
    until(&mut harness, "listed", |h| shows(h, "listed"));
    let fds: std::collections::BTreeSet<i32> = fs::read_to_string(&out)
        .expect("listing")
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    use std::os::fd::AsRawFd;
    assert!(!fds.contains(&fd.as_raw_fd()), "fd leaked: {fds:?}");
    let allowed = crate::testfds::allowed_in_child(&inherited);
    let extra: Vec<_> = fds.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "the child holds {extra:?} beyond {allowed:?}: {fds:?}"
    );
    drop(fd);
}

#[test]
fn dropping_the_module_kills_the_command_and_everything_it_started() {
    let dir = Dir::new("drop");
    let pidfile = dir.file("pid");
    let script = format!(
        "echo $$ > {}; sleep 311 & echo started; wait",
        pidfile.display()
    );
    let mut harness = module(&script, Format::Text);
    until(&mut harness, "started", |h| shows(h, "started"));
    // The script printed `started` right after it forked the worker, which
    // may not have exec'd `sleep` yet: wait for it, never look at one moment
    // (6 runs in 300 failed here under four spinning CPUs).
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running("sleep 311") {
        assert!(Instant::now() < deadline, "the worker is not running");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = pid_in(&pidfile);
    drop(harness);
    let deadline = Instant::now() + Duration::from_secs(5);
    while running("sleep 311") {
        assert!(Instant::now() < deadline, "the worker outlived the module");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The leader was waited for, not left a zombie.
    assert!(gone(pid), "pid {pid} is still in the process table");
}

#[test]
fn a_worker_left_holding_the_pipe_does_not_hang_the_module() {
    // The shell exits at once; its background `sleep` keeps the pipe open,
    // so there is never an end of file. The exit is seen through the
    // pidfd, the worker is killed, and the command is started again.
    let dir = Dir::new("worker");
    let log = dir.file("runs");
    let script = format!("echo run >> {}; sleep 313 & echo hi", log.display());
    let mut harness = module(&script, Format::Text);
    until(&mut harness, "restarted", |_| {
        fs::read_to_string(&log).unwrap_or_default().lines().count() >= 3
    });
    assert!(harness.view().text() == "hi" || harness.view().text() == "ph");
    // Each run's worker was killed with its group: none outlives the module.
    drop(harness);
    let deadline = Instant::now() + Duration::from_secs(5);
    while running("sleep 313") {
        assert!(Instant::now() < deadline, "a worker was left behind");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- the module's own contract ----

#[test]
fn it_polls_one_fd_while_waiting_and_at_most_two_while_running() {
    let mut harness = module("sleep 30", Format::Text);
    assert_eq!(harness.source_count(), 1, "its timer");
    // Started by the first wake.
    let _ = harness.wait(Duration::from_millis(500));
    assert!(harness.source_count() <= 2, "{}", harness.source_count());
    assert!(harness.source_count() >= 1);
}

#[test]
fn it_takes_no_set_value_and_no_pointer_of_its_own() {
    let mut module = start("test", &settings("sleep 30", Format::Text)).expect("starts");
    assert!(module.on_set(&serde_json::json!("x")).is_err());
    assert!(!module.handles_input());
}
