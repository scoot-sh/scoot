//! `scootbg daemon` against a real `scoot --headless`: the socket's whole
//! lifecycle, end to end, through the real binary.
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

use common::{Session, is_socket, signal, stderr_of, wait_exit};
use rustix::process::Signal;

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text:?}"))
}

#[test]
fn query_version_and_kill() {
    let Some(session) = Session::start("life") else {
        return;
    };
    let mut daemon = session.daemon();

    let query = session.run(&["query"]);
    assert!(query.status.success(), "{}", stderr(&query));
    assert_eq!(stdout(&query), "{\"type\":\"outputs\",\"outputs\":[]}\n");

    let version = session.run(&["version"]);
    assert!(version.status.success(), "{}", stderr(&version));
    let reply = json(&stdout(&version));
    assert_eq!(reply["type"], "version");
    assert_eq!(reply["protocol"], 1);
    assert_eq!(reply["version"], env!("CARGO_PKG_VERSION"));

    let kill = session.run(&["kill"]);
    assert!(kill.status.success(), "{}", stderr(&kill));
    assert_eq!(stdout(&kill), "");
    // `kill` returned, so the socket is already gone and the lock free.
    assert!(!session.socket().exists());
    assert!(wait_exit(&mut daemon).success());

    // With no daemon, the clients fail loudly.
    let query = session.run(&["query"]);
    assert_eq!(query.status.code(), Some(1));
    assert!(stderr(&query).contains("no scootbg daemon is running"));
    let kill = session.run(&["kill"]);
    assert_eq!(kill.status.code(), Some(1));
}

#[test]
fn a_new_daemon_starts_straight_after_kill() {
    let Some(session) = Session::start("again") else {
        return;
    };
    for _ in 0..3 {
        let mut daemon = session.daemon();
        let kill = session.run(&["kill"]);
        assert!(kill.status.success(), "{}", stderr(&kill));
        // No sleep: `kill` returning is the promise.
        let next = session.scootbg().arg("daemon").spawn().unwrap();
        assert!(wait_exit(&mut daemon).success());
        let mut next = next;
        let deadline = std::time::Instant::now() + common::PATIENCE;
        while !common::answers(&session.socket()) {
            assert!(next.try_wait().unwrap().is_none(), "the new daemon exited");
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.run(&["kill"]).status.success());
        assert!(wait_exit(&mut next).success());
    }
}

#[test]
fn a_second_daemon_is_refused_and_the_first_keeps_serving() {
    let Some(session) = Session::start("second") else {
        return;
    };
    let mut first = session.daemon();
    let second = session.run(&["daemon"]);
    assert_eq!(second.status.code(), Some(1));
    assert!(
        stderr(&second).contains("already running"),
        "{}",
        stderr(&second)
    );
    assert!(session.run(&["query"]).status.success());
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut first).success());
}

#[test]
fn a_stale_socket_is_replaced() {
    let Some(session) = Session::start("stale") else {
        return;
    };
    // What a crashed daemon leaves: the file, nobody listening.
    drop(UnixListener::bind(session.socket()).unwrap());
    assert!(is_socket(&session.socket()));
    let mut daemon = session.daemon();
    assert!(session.run(&["query"]).status.success());
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

#[test]
fn the_compositor_going_away_ends_the_daemon_with_an_error() {
    let Some(mut session) = Session::start("gone") else {
        return;
    };
    let mut daemon = session.daemon();
    session.kill_compositor();
    let status = wait_exit(&mut daemon);
    assert_eq!(status.code(), Some(1), "{status}");
    let err = stderr_of(&mut daemon);
    assert!(err.contains("lost the connection"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(!session.socket().exists());
}

/// SIGTERM, SIGINT and SIGHUP keep their default action: the daemon dies
/// at once and its socket file stays. That must be harmless: the kernel
/// drops the lock with the process, clients meanwhile report "not running"
/// (connect gets ECONNREFUSED), and the next daemon replaces the file.
#[test]
fn a_signal_kills_the_daemon_and_the_next_one_replaces_its_socket() {
    let Some(session) = Session::start("signals") else {
        return;
    };
    let lock = session
        .runtime_dir()
        .join(format!("scootbg-{}.lock", session.wayland_display));
    for sig in [Signal::TERM, Signal::INT, Signal::HUP] {
        let mut daemon = session.daemon();
        signal(&daemon, sig);
        let status = wait_exit(&mut daemon);
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(sig.as_raw()),
            "{sig:?}: {status}"
        );
        // The dead daemon's socket file is still there, nothing listens.
        assert!(is_socket(&session.socket()), "{sig:?}: socket file gone");
        assert!(!common::answers(&session.socket()));
        // The lock was released with the process.
        let file = std::fs::OpenOptions::new().write(true).open(&lock).unwrap();
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .unwrap_or_else(|e| panic!("{sig:?}: lock still held: {e}"));
        drop(file);

        // Clients say "not running", with exit status 1.
        for command in ["query", "version", "kill"] {
            let out = session.run(&[command]);
            assert_eq!(out.status.code(), Some(1), "{sig:?} {command}");
            assert!(
                stderr(&out).contains("no scootbg daemon is running"),
                "{sig:?} {command}: {}",
                stderr(&out)
            );
        }
    }
    // The next daemon replaces the stale file and serves.
    let mut daemon = session.daemon();
    let query = session.run(&["query"]);
    assert!(query.status.success(), "{}", stderr(&query));
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
    assert!(!session.socket().exists());
}

#[test]
fn no_compositor_is_an_error_and_leaves_no_socket() {
    let scratch = common::Scratch::new("nocomp");
    let output = std::process::Command::new(common::scootbg_bin())
        .arg("daemon")
        .env("XDG_RUNTIME_DIR", &scratch.0)
        .env("WAYLAND_DISPLAY", "wayland-77")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot connect to the Wayland compositor"),
        "{}",
        stderr(&output)
    );
    assert!(!scratch.0.join("scootbg-wayland-77.sock").exists());
}

#[test]
fn missing_environment_is_an_error() {
    let output = std::process::Command::new(common::scootbg_bin())
        .arg("daemon")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("XDG_RUNTIME_DIR is not set"));

    let output = std::process::Command::new(common::scootbg_bin())
        .arg("query")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn usage_errors_exit_2_and_help_exits_0() {
    let run = |args: &[&str]| {
        std::process::Command::new(common::scootbg_bin())
            .args(args)
            .output()
            .unwrap()
    };
    for args in [
        &[][..],
        &["set", "#000000"],
        &["query", "extra"],
        &["bogus"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).contains("try"), "{args:?}");
    }
    for args in [&["--help"][..], &["daemon", "--help"], &["help", "kill"]] {
        let output = run(args);
        assert!(output.status.success(), "{args:?}");
        assert!(stdout(&output).starts_with("scootbg"));
    }
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert!(stdout(&version).starts_with("scootbg "));
}

/// Bad clients must not wedge or crash the daemon: garbage, an oversized
/// line, invalid UTF-8, a flood of idle connections. A plain `query`
/// still works afterwards.
#[test]
fn hostile_clients_do_not_wedge_the_daemon() {
    let Some(session) = Session::start("hostile") else {
        return;
    };
    let mut daemon = session.daemon();
    let socket = session.socket();
    let connect = || {
        let stream = UnixStream::connect(&socket).unwrap();
        stream.set_read_timeout(Some(common::PATIENCE)).unwrap();
        stream
    };
    let reply_to = |stream: &UnixStream| {
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        json(&line)
    };

    // Garbage and random bytes on one connection: an error per line, and
    // the connection keeps working.
    let mut stream = connect();
    stream.write_all(b"not json\n").unwrap();
    assert_eq!(reply_to(&stream)["type"], "error");
    let mut noise = vec![0u8; 4096];
    let mut x: u32 = 0x2545_f491;
    for byte in &mut noise {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *byte = (x as u8) | 1;
    }
    for byte in &mut noise {
        if *byte == b'\n' {
            *byte = b'x';
        }
    }
    noise.push(b'\n');
    stream.write_all(&noise).unwrap();
    assert_eq!(reply_to(&stream)["type"], "error");
    // The array form of a request is not a request: it must not stop the
    // daemon.
    stream.write_all(b"[1,\"kill\"]\n").unwrap();
    let array = reply_to(&stream);
    assert_eq!(array["type"], "error");
    assert!(
        daemon.try_wait().unwrap().is_none(),
        "[1,\"kill\"] stopped it"
    );
    stream
        .write_all(b"{\"protocol\":2,\"type\":\"query\"}\n")
        .unwrap();
    let wrong = reply_to(&stream);
    assert!(wrong["message"].as_str().unwrap().contains("protocol"));
    stream
        .write_all(b"{\"protocol\":1,\"type\":\"query\"}\n")
        .unwrap();
    assert_eq!(reply_to(&stream)["type"], "outputs");

    // A 1 MB line: an error, then the connection is closed.
    let mut big = connect();
    let payload = vec![b'a'; 1 << 20];
    // The daemon closes after 64 KiB; the rest of the write may fail.
    let _ = big.write_all(&payload);
    let mut line = String::new();
    let _ = BufReader::new(&big).read_line(&mut line);
    assert!(line.contains("longer than"), "{line:?}");

    // More idle connections than the daemon serves at once: the oldest are
    // closed, and a fresh client is still answered.
    let idle: Vec<UnixStream> = (0..100).map(|_| connect()).collect();
    let query = session.run(&["query"]);
    assert!(query.status.success(), "{}", stderr(&query));
    drop(idle);

    // A client that sends and vanishes before the reply.
    for _ in 0..20 {
        let mut s = connect();
        s.write_all(b"{\"protocol\":1,\"type\":\"query\"}\n")
            .unwrap();
        drop(s);
    }

    assert!(session.run(&["query"]).status.success());
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon died");
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
    let err = stderr_of(&mut daemon);
    assert!(!err.contains("panicked"), "{err}");
}

/// Two `kill`s at once: each succeeds (the loser's connection may be
/// closed unanswered as the daemon exits, which is still the stop it asked
/// for) or finds no daemon; neither hangs, and the daemon exits once.
#[test]
fn a_double_kill_is_harmless() {
    let Some(session) = Session::start("double") else {
        return;
    };
    let mut daemon = session.daemon();
    let kill = || {
        session
            .scootbg()
            .arg("kill")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let (a, b) = (kill(), kill());
    let a = a.wait_with_output().unwrap();
    let b = b.wait_with_output().unwrap();
    assert!(a.status.success() || b.status.success());
    for out in [&a, &b] {
        assert!(
            out.status.success() || stderr(out).contains("no scootbg daemon"),
            "{}",
            stderr(out)
        );
    }
    assert!(wait_exit(&mut daemon).success());
    assert!(!session.socket().exists());
}

/// Out of file descriptors, `kill` still gets through.
///
/// The daemon starts with its normal limit and is left to settle (serving,
/// spare fd retaken); then its `RLIMIT_NOFILE` is lowered, at run time, to
/// exactly the fds it holds, so every later accept fails with `EMFILE`
/// however many fds it inherited (a CI runner hands children more than a
/// developer shell; a fixed limit at spawn broke on one). The first client
/// is admitted on the spare's fd, each later one by closing the oldest
/// client. The daemon neither goes deaf nor spins. Needs util-linux
/// `prlimit`: skipped without it, or a failure under
/// `SCOOTBG_REQUIRE_SCOOT`.
#[test]
fn out_of_file_descriptors_kill_still_works() {
    let Some(session) = Session::start("nofile") else {
        return;
    };
    if std::process::Command::new("prlimit")
        .arg("--version")
        .output()
        .is_err()
    {
        // Same rule as the compositor itself: CI must not pass by skipping.
        assert!(
            std::env::var_os("SCOOTBG_REQUIRE_SCOOT").is_none(),
            "SCOOTBG_REQUIRE_SCOOT is set but there is no prlimit (util-linux) on PATH"
        );
        eprintln!("skipped -- no prlimit on PATH");
        return;
    }
    let mut daemon = session.daemon();
    let pid = daemon.id();
    // One more accept, so the spare the start-up probe spent is retaken
    // and the count below includes it.
    assert!(common::answers(&session.socket()));
    let held = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .count();
    let limit = format!("--nofile={held}:{held}");
    let lowered = std::process::Command::new("prlimit")
        .args(["--pid", &pid.to_string(), &limit])
        .output()
        .unwrap();
    assert!(
        lowered.status.success(),
        "prlimit --pid: {}",
        stderr(&lowered)
    );

    // Idle clients the daemon has to make room for, repeatedly.
    let idle: Vec<UnixStream> = (0..5)
        .map(|_| UnixStream::connect(session.socket()).unwrap())
        .collect();
    for _ in 0..3 {
        let query = session.run(&["query"]);
        assert!(query.status.success(), "{}", stderr(&query));
    }
    // The limit really bound: the daemon is at it, not below.
    let now = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .count();
    assert!(now <= held, "{now} fds open against a limit of {held}");
    // Busy-looping would show as CPU time.
    let ticks = || {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields: Vec<&str> = stat
            .rsplit(')')
            .next()
            .unwrap()
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    let before = ticks();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        ticks(),
        before,
        "the daemon used CPU while idle: it is spinning"
    );

    let started = std::time::Instant::now();
    let kill = session.run(&["kill"]);
    assert!(kill.status.success(), "{}", stderr(&kill));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(wait_exit(&mut daemon).success());
    assert!(!session.socket().exists());
    drop(idle);
}

/// Starts `scootbg daemon` with stderr a pipe whose read end is already
/// closed, so any write to it fails with EPIPE, and waits until it answers.
fn daemon_with_broken_stderr(session: &Session, extra_env: &[(&str, &str)]) -> std::process::Child {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut command = session.scootbg();
    command
        .arg("daemon")
        .stdout(std::process::Stdio::null())
        .stderr(writer);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    command.spawn().unwrap()
}

/// S2 of the #268 review: with stderr broken, the compositor going away
/// must still be a clean exit 1 with the socket removed, not a panic
/// (exit 101 in a debug build, SIGABRT under the release profile).
#[test]
fn a_broken_stderr_does_not_turn_compositor_exit_into_a_crash() {
    let Some(mut session) = Session::start("brokenerr") else {
        return;
    };
    let mut daemon = daemon_with_broken_stderr(&session, &[]);
    let deadline = std::time::Instant::now() + common::PATIENCE;
    while !common::answers(&session.socket()) {
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "the daemon exited early"
        );
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    session.kill_compositor();
    let status = wait_exit(&mut daemon);
    assert_eq!(status.code(), Some(1), "{status}");
    assert!(!session.socket().exists());
}

/// The crash hook, end to end: `WAYLAND_DEBUG=client` makes
/// `wayland-backend` trace every message with `eprintln!`, which panics on
/// the broken stderr during start-up. That must be exit 1 with the socket
/// removed, the same as any lost connection, not an abort.
#[test]
fn a_dependency_printing_to_a_broken_stderr_exits_1_and_removes_the_socket() {
    let Some(session) = Session::start("debugerr") else {
        return;
    };
    let mut daemon = daemon_with_broken_stderr(&session, &[("WAYLAND_DEBUG", "client")]);
    let status = wait_exit(&mut daemon);
    assert_eq!(status.code(), Some(1), "{status}");
    assert!(!session.socket().exists());
    // The lock is free: a normal daemon starts at once.
    let mut next = session.daemon();
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut next).success());
}
