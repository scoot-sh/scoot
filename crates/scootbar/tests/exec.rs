//! The modules the config defines, on a headless scoot: a `button` that
//! launches a command, a `push` that `scootbar msg set` writes to, and an
//! `exec` that shows a command's output, with the bounds each is held to
//! checked against a running bar (a flooding child, a child that dies, the
//! descriptors it inherits, what is left behind on a reload).
//!
//! The bar draws in the seven-segment test font, 50 pixels to the em on a
//! 60-pixel bar, so what a module shows is read back off a screenshot.
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// The tests place button, push and exec modules (gated per test), so this
// file exists where any of them does (docs/scootbar/testing.md: the
// feature matrix); helpers used by only some kinds carry that gate too.
#![cfg(any(feature = "button", feature = "push", feature = "exec"))]

mod common;

#[cfg(feature = "exec")]
use std::collections::BTreeSet;
use std::fs;
#[cfg(any(feature = "button", feature = "exec"))]
use std::path::PathBuf;
#[cfg(any(feature = "push", feature = "exec"))]
use std::process::Output;
#[cfg(any(feature = "push", feature = "exec"))]
use std::time::Duration;
#[cfg(feature = "exec")]
use std::time::Instant;

use common::{Reaper, Session, rgb};
use common::{Shot, testfont};
#[cfg(feature = "exec")]
use common::{allowed_in_child, inheritable_fds, open_fds, settled_fds};
#[cfg(any(feature = "push", feature = "exec"))]
use serde_json::Value;

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const URGENT: &str = "#ff0000";
const HEIGHT: u32 = 60;
const EM: u32 = 50;
const BASELINE: i64 = 45;

/// The bar's config: the look, `lists` (`right = ["x"]`), and `tables`.
fn config(lists: &str, tables: &str) -> String {
    format!(
        "{lists}\n[bar]\nheight = {HEIGHT}\nfont-size = {EM}\n\
         [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\nurgent = \"{URGENT}\"\n\
         {tables}"
    )
}

struct Rig {
    session: Session,
    bar: Reaper,
    #[cfg(feature = "exec")]
    file: PathBuf,
    #[cfg(any(feature = "button", feature = "exec"))]
    dir: PathBuf,
}

impl Rig {
    fn start(tag: &str, lists: &str, tables: &str) -> Option<Self> {
        Self::start_with(tag, lists, tables, &[])
    }

    fn start_with(tag: &str, lists: &str, tables: &str, env: &[(&str, &str)]) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let dir = session.runtime_dir().join("out");
        fs::create_dir_all(&dir).unwrap();
        let file = session.runtime_dir().join("bar.toml");
        let tables = tables.replace("DIR", dir.to_str().unwrap());
        fs::write(&file, config(lists, &tables)).unwrap();
        let bar = Reaper(session.bar_with_env(&["--config", file.to_str().unwrap()], env));
        let mut rig = Self {
            session,
            bar,
            #[cfg(feature = "exec")]
            file,
            #[cfg(any(feature = "button", feature = "exec"))]
            dir,
        };
        rig.wait_drawn();
        Some(rig)
    }

    fn wait_drawn(&mut self) {
        self.session
            .wait_for(&mut self.bar.0, "the bar drawn", |session| {
                (session.scoot_screenshot(1).at(0, 0) == rgb(BAR)).then_some(())
            });
    }

    #[cfg(feature = "exec")]
    fn pid(&self) -> u32 {
        self.bar.0.id()
    }

    #[cfg(any(feature = "push", feature = "exec"))]
    fn msg(&self, args: &[&str]) -> Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    #[cfg(any(feature = "push", feature = "exec"))]
    fn query(&self) -> Value {
        let out = self.msg(&["query"]);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// What the bar's text reads back as, on output 1.
    #[cfg(feature = "push")]
    fn read(&self) -> String {
        read(&self.session.scoot_screenshot(1))
    }

    /// Waits until the bar's text reads as `text`.
    fn wait_text(&mut self, text: &str) {
        let what = format!("the bar to read `{text}`");
        self.session.wait_for(&mut self.bar.0, &what, |session| {
            (read(&session.scoot_screenshot(1)) == text).then_some(())
        });
    }

    /// Waits until `name` in the output directory has at least `n` lines.
    #[cfg(any(feature = "button", feature = "exec"))]
    fn wait_lines(&mut self, name: &str, n: usize) -> Vec<String> {
        let path = self.dir.join(name);
        self.session
            .wait_for(&mut self.bar.0, &format!("{n} lines in {name}"), |_| {
                let lines: Vec<String> = fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect();
                (lines.len() >= n).then_some(lines)
            })
    }

    #[cfg(feature = "exec")]
    fn reload(&self) -> Output {
        self.msg(&["reload"])
    }
}

/// The bar's text read back from the top rows of `shot`, spaces left out.
fn read(shot: &Shot) -> String {
    let background = rgb(BAR);
    let ink = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as u32) < shot.width
            && (y as u32) < HEIGHT
            && shot.at(x as u32, y as u32) != background
    };
    let columns: Vec<i64> = (0..i64::from(shot.width))
        .filter(|&x| (0..i64::from(HEIGHT)).any(|y| ink(x, y)))
        .collect();
    let (Some(&left), Some(&right)) = (columns.first(), columns.last()) else {
        return String::new();
    };
    testfont::decode(ink, left, right, BASELINE, f64::from(EM))
}

#[cfg(feature = "exec")]
fn has_color(shot: &Shot, color: [u8; 3]) -> bool {
    (0..shot.width).any(|x| (0..HEIGHT).any(|y| shot.at(x, y) == color))
}

/// Whether a process runs with exactly this command line.
#[cfg(feature = "exec")]
fn running(cmdline: &str) -> bool {
    for entry in fs::read_dir("/proc").unwrap().flatten() {
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

#[cfg(any(feature = "push", feature = "exec"))]
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[cfg(feature = "exec")]
#[test]
fn an_exec_module_shows_what_its_command_prints() {
    let tables = "[exec.out]\ncommand = [\"sh\", \"-c\", \"echo 12:34; sleep 600\"]\n";
    let Some(mut rig) = Rig::start("exec-shows", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.wait_text("12:34");
    let queried = rig.query();
    let module = &queried["modules"][0];
    assert_eq!(module["id"], "out");
    assert_eq!(module["text"], "12:34");
    assert_eq!(module["section"], "right");
}

#[cfg(feature = "exec")]
#[test]
fn an_exec_json_line_sets_the_text_and_the_class_color() {
    let tables = "[exec.out]\nformat = \"json\"\ncommand = [\"sh\", \"-c\", \
                  \"echo '{\\\"text\\\":\\\"9\\\",\\\"class\\\":\\\"urgent\\\"}'; sleep 600\"]\n";
    let Some(mut rig) = Rig::start("exec-json", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.wait_text("9");
    rig.session
        .wait_for(&mut rig.bar.0, "the urgent color", |session| {
            has_color(&session.scoot_screenshot(1), rgb(URGENT)).then_some(())
        });
    assert_eq!(rig.query()["modules"][0]["class"], "urgent");
}

#[cfg(feature = "push")]
#[test]
fn push_set_changes_the_bar_and_a_bad_value_changes_nothing() {
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(mut rig) = Rig::start("push-set", "right = [\"status\"]\n", tables) else {
        return;
    };
    rig.wait_text("0");
    let set = rig.msg(&["set", "status", "\"42\""]);
    assert!(set.status.success(), "{}", stderr(&set));
    rig.wait_text("42");
    // The query agrees with the screenshot.
    assert_eq!(rig.query()["modules"][0]["text"], "42");
    // An object with a class.
    let set = rig.msg(&["set", "status", r#"{"text":"7","class":"urgent"}"#]);
    assert!(set.status.success(), "{}", stderr(&set));
    rig.wait_text("7");
    assert_eq!(rig.query()["modules"][0]["class"], "urgent");
    // Refusals are named, and the bar keeps what it showed.
    for (value, why) in [
        (r#"{"text":5}"#, "`text` takes a string"),
        (r#"{"class":"loud"}"#, "`class` takes"),
        (r#"{"version":2}"#, "`version`"),
        ("[1]", "JSON object"),
    ] {
        let refused = rig.msg(&["set", "status", value]);
        assert!(!refused.status.success(), "{value}");
        assert!(
            stderr(&refused).contains(why),
            "{value}: {}",
            stderr(&refused)
        );
    }
    assert_eq!(rig.read(), "7");
    // `null` clears it, and the module takes no space.
    let cleared = rig.msg(&["set", "status", "null"]);
    assert!(cleared.status.success(), "{}", stderr(&cleared));
    rig.wait_text("");
}

#[cfg(all(feature = "button", feature = "push"))]
#[test]
fn set_names_what_cannot_take_a_value() {
    let tables = "[button.b]\ntext = \"1\"\n[push.p]\n";
    let Some(rig) = Rig::start("set-refusals", "left = [\"b\", \"p\"]\n", tables) else {
        return;
    };
    let on_button = rig.msg(&["set", "b", "\"x\""]);
    assert!(!on_button.status.success());
    assert!(
        stderr(&on_button).contains("takes no set value"),
        "{}",
        stderr(&on_button)
    );
    let missing = rig.msg(&["set", "nope", "\"x\""]);
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("not placed"),
        "{}",
        stderr(&missing)
    );
    let malformed = rig.msg(&["set", "no way", "\"x\""]);
    assert!(!malformed.status.success());
    assert!(
        stderr(&malformed).contains("module id"),
        "{}",
        stderr(&malformed)
    );
}

#[cfg(feature = "button")]
#[test]
fn a_button_click_runs_its_command() {
    let tables = "[button.go]\ntext = \"7\"\n\
                  on-click = { exec = [\"sh\", \"-c\", \"echo pressed >> DIR/log\"] }\n";
    let Some(mut rig) = Rig::start("button-click", "left = [\"go\"]\n", tables) else {
        return;
    };
    rig.wait_text("7");
    // The button is at the left edge: its middle is a few tens of pixels in.
    let reply = rig
        .session
        .scoot_ipc(r#"{"type":"click","x":30,"y":30,"button":"left"}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(rig.wait_lines("log", 1), ["pressed"]);
}

#[cfg(feature = "exec")]
#[test]
fn a_flooding_command_costs_the_bar_no_memory_and_no_descriptors() {
    let tables = "[exec.out]\ncommand = [\"yes\", \"flood\"]\n";
    let Some(mut rig) = Rig::start("exec-flood", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.session
        .wait_for(&mut rig.bar.0, "the first line", |session| {
            (!read(&session.scoot_screenshot(1)).is_empty()).then_some(())
        });
    let pid = rig.pid();
    let fds = settled_fds(pid);
    let rss = |pid: u32| -> u64 {
        fs::read_to_string(format!("/proc/{pid}/status"))
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
            .unwrap()
    };
    let cpu = |pid: u32| -> u64 {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let close = stat.rfind(')').unwrap();
        let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        // utime and stime: fields 14 and 15 of the stat line.
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    let (rss0, cpu0) = (rss(pid), cpu(pid));
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(3));
    let (rss1, cpu1) = (rss(pid), cpu(pid));
    let seconds = started.elapsed().as_secs_f64();
    // Jiffies are 10 ms: the bar's CPU share over the flood.
    let share = (cpu1 - cpu0) as f64 / 100.0 / seconds;
    eprintln!(
        "flood: rss {rss0} -> {rss1} kB, cpu {:.1}%, fds {fds}",
        share * 100.0
    );
    assert!(rss1 <= rss0 + 1024, "rss grew {rss0} -> {rss1} kB");
    assert!(
        share < 0.25,
        "the bar spent {:.0}% of a core on a flood",
        share * 100.0
    );
    assert_eq!(open_fds(pid), fds, "descriptors moved");
    // And the bar is still alive and showing a line.
    assert!(rig.bar.0.try_wait().unwrap().is_none());
}

#[cfg(feature = "exec")]
#[test]
fn a_command_that_dies_is_started_again_by_the_bar() {
    let tables = "[exec.out]\ncommand = [\"sh\", \"-c\", \"echo run >> DIR/runs\"]\n";
    let Some(mut rig) = Rig::start("exec-restart", "right = [\"out\"]\n", tables) else {
        return;
    };
    // 0 s, +1 s, +2 s (the defaults): three runs by about 3.5 s, not a spin.
    rig.wait_lines("runs", 3);
    std::thread::sleep(Duration::from_millis(500));
    let runs = fs::read_to_string(rig.dir.join("runs"))
        .unwrap()
        .lines()
        .count();
    assert!(runs <= 5, "{runs} runs: it is spinning");
    // Nothing is left a zombie.
    let zombies = fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| fs::read_to_string(entry.path().join("stat")).ok())
        .filter(|stat| {
            let close = stat.rfind(')').unwrap_or(0);
            let mut rest = stat[close + 1..].split_whitespace();
            rest.next() == Some("Z") && rest.next().and_then(|p| p.parse().ok()) == Some(rig.pid())
        })
        .count();
    assert_eq!(zombies, 0);
}

#[cfg(feature = "exec")]
#[test]
fn a_command_holds_none_of_the_bars_descriptors() {
    let tables = "[exec.out]\ncommand = [\"sh\", \"-c\", \
                  \"ls /proc/self/fd > DIR/fds; echo 1; sleep 600\"]\n";
    // What this process was handed by whatever started it (a CI runner may
    // leave descriptors open): the bar inherits them and so does its
    // command. Anything else is the bar's own, and must not reach it.
    let inherited = inheritable_fds("self");
    let Some(mut rig) = Rig::start("exec-fds", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.wait_text("1");
    let held = settled_fds(rig.pid());
    assert!(held > 6, "the bar holds only {held} descriptors");
    // The bar holds no descriptor without close-on-exec but those it was
    // started with, whatever the command below happens to list.
    let allowed_in_bar: BTreeSet<i32> = inherited.iter().copied().chain(0..=2).collect();
    let leaky: Vec<_> = inheritable_fds(rig.pid())
        .difference(&allowed_in_bar)
        .copied()
        .collect();
    assert!(
        leaky.is_empty(),
        "the bar holds {leaky:?} without close-on-exec, which no command may inherit"
    );
    let fds: BTreeSet<i32> = fs::read_to_string(rig.dir.join("fds"))
        .unwrap()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    let allowed = allowed_in_child(&inherited);
    let extra: Vec<_> = fds.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "inherited {extra:?} beyond {allowed:?}: {fds:?} (the bar has {held})"
    );
}

/// The `SigIgn` mask of `pid`.
#[cfg(feature = "exec")]
fn ignored_signals(pid: impl std::fmt::Display) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("SigIgn:"))
        .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
        .expect("SigIgn in /proc/PID/status")
}

#[cfg(feature = "exec")]
#[test]
fn a_command_gets_the_default_sigpipe_not_the_bars_ignored_one() {
    // A Rust program ignores `SIGPIPE`; ignored signals survive `exec`. The
    // command is what the guard (a Rust program, started through
    // `/proc/self/exe`) execs, so it would inherit the ignore, and a
    // `yes | head` in a script would then spin on `EPIPE` instead of ending
    // by the signal. `std` puts the default back in the child before it
    // execs; this pins that, so a `std` change cannot silently end it. Only
    // `SIGPIPE`'s own bit is read: a harness may hand down others (`SIGHUP`
    // under `nohup`, `SIGQUIT`), and those are not this test's.
    const SIGPIPE_BIT: u64 = 1 << (13 - 1);
    let tables = "[exec.out]\ncommand = [\"sh\", \"-c\", \
                  \"grep SigIgn /proc/self/status > DIR/sigign; echo 1; sleep 600\"]\n";
    let Some(mut rig) = Rig::start("exec-sigpipe", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.wait_text("1");
    let lines = rig.wait_lines("sigign", 1);
    let command = lines[0]
        .strip_prefix("SigIgn:")
        .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
        .unwrap_or_else(|| panic!("unreadable: {:?}", lines[0]));
    assert_eq!(
        command & SIGPIPE_BIT,
        0,
        "the command has SIGPIPE ignored: SigIgn {command:#x}"
    );
    // The control: the bar itself does ignore it, so the test would see an
    // inherited one.
    assert_ne!(
        ignored_signals(rig.pid()) & SIGPIPE_BIT,
        0,
        "the bar does not ignore SIGPIPE, so this test checks nothing"
    );
}

#[cfg(feature = "exec")]
#[test]
fn a_command_that_is_not_found_is_one_warning_a_restart_naming_it() {
    let tables = "[exec.out]\ncommand = [\"scootbar-no-such-program\"]\n";
    let Some(mut rig) = Rig::start("exec-notfound", "right = [\"out\"]\n", tables) else {
        return;
    };
    rig.session
        .wait_for(&mut rig.bar.0, "the restart warning", |session| {
            session
                .bar_stderr()
                .contains("command not found")
                .then_some(())
        });
    let stderr = rig.session.bar_stderr();
    // One line says it, from the bar, not a second from the guard before it.
    assert!(
        !stderr.contains("cannot run"),
        "the guard spoke as well:\n{stderr}"
    );
    assert!(
        stderr.contains("exit status: 127, command not found"),
        "{stderr}"
    );
    // And the bar is up, with the module restarting.
    assert!(rig.bar.0.try_wait().unwrap().is_none());
}

#[cfg(feature = "exec")]
#[test]
fn a_command_that_is_not_executable_says_so_once() {
    let tables = "[exec.out]\ncommand = [\"DIR/not-a-program\"]\n";
    let Some(mut rig) = Rig::start("exec-noexec", "right = [\"out\"]\n", tables) else {
        return;
    };
    fs::write(rig.dir.join("not-a-program"), "#!/bin/sh\n").unwrap();
    rig.session
        .wait_for(&mut rig.bar.0, "the restart warning", |session| {
            session
                .bar_stderr()
                .contains("not executable")
                .then_some(())
        });
    let stderr = rig.session.bar_stderr();
    assert!(!stderr.contains("cannot run"), "{stderr}");
    assert!(
        stderr.contains("exit status: 126, not executable"),
        "{stderr}"
    );
}

#[cfg(feature = "exec")]
#[test]
fn a_reload_replaces_the_command_and_kills_the_old_one_with_its_workers() {
    let first = "[exec.out]\ncommand = [\"sh\", \"-c\", \"sleep 421 & echo 1; wait\"]\n";
    let Some(mut rig) = Rig::start("exec-reload", "right = [\"out\"]\n", first) else {
        return;
    };
    rig.wait_text("1");
    // The worker was forked just before the line was printed: it may not
    // have exec'd `sleep` yet.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running("sleep 421") {
        assert!(Instant::now() < deadline, "the worker is not running");
        std::thread::sleep(Duration::from_millis(10));
    }
    let second = "[exec.out]\ncommand = [\"sh\", \"-c\", \"echo 2; sleep 600\"]\n";
    fs::write(&rig.file, config("right = [\"out\"]\n", second)).unwrap();
    let reloaded = rig.reload();
    assert!(reloaded.status.success(), "{}", stderr(&reloaded));
    rig.wait_text("2");
    let deadline = Instant::now() + Duration::from_secs(5);
    while running("sleep 421") {
        assert!(
            Instant::now() < deadline,
            "the old command's worker outlived the reload"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Kills the bar with `signal` and waits for its command, a shell loop
/// that would otherwise run for ever, to be gone.
#[cfg(feature = "exec")]
fn a_command_ends_with_a_bar_that_dies_of(tag: &str, signal: rustix::process::Signal) {
    // A shell loop is the command that is not ended by the closing of its
    // pipe: the shell never writes, `date` and `echo` do, and the shell is
    // what has to die.
    let script = format!("while :; do echo 5; sleep 1; done # {tag}");
    let tables = format!("[exec.out]\ncommand = [\"sh\", \"-c\", \"{script}\"]\n");
    let Some(mut rig) = Rig::start(tag, "right = [\"out\"]\n", &tables) else {
        return;
    };
    rig.wait_text("5");
    let cmdline = format!("sh -c {script}");
    assert!(running(&cmdline), "the loop is not running");
    let bar = rustix::process::Pid::from_raw(rig.pid() as i32).unwrap();
    rustix::process::kill_process(bar, signal).unwrap();
    // Bounded: a bar that inherited the signal as ignored (a harness run
    // under `nohup` ignores `SIGHUP`, a background job of a shell
    // `SIGINT`) would otherwise hang this test for ever.
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = rig.bar.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the bar is still running after {signal:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success(), "the bar exited cleanly: {status}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while running(&cmdline) {
        assert!(
            Instant::now() < deadline,
            "the loop outlived the bar ({signal:?}): it runs on, reparented"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(feature = "exec")]
#[test]
fn a_command_ends_with_a_bar_that_is_killed() {
    a_command_ends_with_a_bar_that_dies_of("exec-sigkill", rustix::process::Signal::KILL);
}

#[cfg(feature = "exec")]
#[test]
fn a_command_ends_with_a_bar_that_is_terminated() {
    // `SIGINT`, `SIGHUP` and a crash (`SIGABRT`, what a panic does under
    // `panic = "abort"`) are the same to the kernel; they are not tests
    // because a test harness may start with them ignored, which the bar
    // would inherit (see `docs/scootbar/backlog/resolved/
    // exec-push-button-modules-done.md`, decision 7, for the measurement).
    a_command_ends_with_a_bar_that_dies_of("exec-sigterm", rustix::process::Signal::TERM);
}

#[cfg(feature = "push")]
#[test]
fn a_burst_of_pushes_in_one_turn_is_a_few_redraws() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let trace = [("WAYLAND_DEBUG", "1")];
    let Some(mut rig) = Rig::start_with("push-burst", "right = [\"status\"]\n", tables, &trace)
    else {
        return;
    };
    rig.wait_text("0");
    // The control socket: `scootbar-DISPLAY.sock` in the runtime directory.
    let socket = fs::read_dir(rig.session.runtime_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("scootbar-") && n.ends_with(".sock"))
        })
        .expect("the bar's control socket");
    std::thread::sleep(Duration::from_millis(300));
    let commits = |text: &str| text.matches(".commit()").count();
    let before = commits(&rig.session.bar_stderr());
    let mut stream = UnixStream::connect(&socket).unwrap();
    let mut burst = String::new();
    let sets = 200;
    for i in 0..sets {
        burst.push_str(&format!(
            "{{\"protocol\":1,\"type\":\"set\",\"id\":\"status\",\"value\":\"{}\"}}\n",
            i % 10
        ));
    }
    stream.write_all(burst.as_bytes()).unwrap();
    let mut replies = BufReader::new(&stream);
    for _ in 0..sets {
        let mut line = String::new();
        replies.read_line(&mut line).unwrap();
        assert!(line.contains("\"ok\""), "{line}");
    }
    std::thread::sleep(Duration::from_millis(300));
    let drawn = commits(&rig.session.bar_stderr()) - before;
    assert!(drawn < sets / 4, "{drawn} commits for {sets} sets");
    // The last value stands.
    rig.wait_text("9");
}

#[cfg(feature = "exec")]
#[test]
fn too_many_exec_modules_are_refused_by_name() {
    let Some(session) = Session::scoot("exec-many", 1, "") else {
        return;
    };
    let mut tables = String::new();
    let mut names = Vec::new();
    for i in 0..9 {
        tables.push_str(&format!("[exec.e{i}]\ncommand = [\"true\"]\n"));
        names.push(format!("\"e{i}\""));
    }
    let file = session.runtime_dir().join("bar.toml");
    fs::write(
        &file,
        config(&format!("right = [{}]\n", names.join(", ")), &tables),
    )
    .unwrap();
    let out = session
        .scootbar()
        .args(["daemon", "--font"])
        .arg(session.font())
        .arg("--config")
        .arg(&file)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("at most 8 exec modules"), "{said}");
}
