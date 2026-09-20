#![cfg(unix)]
//! `scootctl` must exit quietly when its stdout reader goes away.
//!
//! Each test serves one canned IPC reply from a fake Unix-socket server
//! (no compositor needed), runs the real `scootctl` binary against it via
//! `SCOOT_SOCKET`, and either reads stdout fully (normal path) or closes
//! the read end before the child can write (truncated path). The truncated
//! reply is ~1 MiB, far past the 64 KiB pipe buffer, so a small reply that
//! "fits and never errors" cannot weaken the test: the child necessarily
//! blocks on the closed pipe and must take EPIPE, which the Rust runtime
//! reports as an error (SIGPIPE stays ignored) instead of a signal.
//!
//! Pre-fix this fails with exit 101 (`failed printing to stdout: Broken
//! pipe`); post-fix the truncated path exits 0, matching the standard Unix
//! tool contract (`head -1` pipelines stay green under `pipefail`).
//!
//! The `scoot msg` alias shares this exact code path (it parses and runs
//! through this crate), so one binary's suite covers both entry points;
//! byte-equivalence between the two is pinned separately, by the smoke
//! test's equivalence section and scoot's alias unit test.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scootctl() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootctl"))
}

/// Builds a fresh Unix-socket path under `$TMPDIR` for one fixture test.
/// `tag` is a per-test abbreviation (`large`, `full`, `acc` for the
/// accept-timeout test) so a stray socket left behind by a crash still
/// names its owner.
///
/// The filename is deliberately short (~32 chars): macOS `SUN_LEN` is only
/// 104 bytes, and a Mac `$TMPDIR` alone can run ~50 (49 on this dev Mac),
/// so the old `scoot-epipe-test-{pid}-{name}-{nanos}.sock` shape (a
/// ~62-char filename, 111 total) failed at `bind` with `InvalidInput:
/// "path must be shorter than SUN_LEN"` while Linux (108-byte limit,
/// short `$TMPDIR`) stayed green.
///
/// Uniqueness across parallel tests comes from the pid (across processes —
/// nextest isolates each test in its own) plus the tag (across tests
/// sharing one `cargo test` process) plus the low 32 bits of `as_nanos`
/// as 8 hex digits (across sequential re-runs leaking a stale path, on
/// top of the `remove_file` at each call site). A residual collision
/// still fails loudly here at `bind`, never inside the server thread.
/// `std` only — no new deps for a test fixture.
fn socket_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32;
    std::env::temp_dir().join(format!(
        "scoot-ep-{}-{:08x}-{}.sock",
        std::process::id(),
        nanos,
        tag
    ))
}

/// How long the fake server waits for the `scootctl` child to connect
/// before failing loudly. A connect that never arrives used to wedge the
/// whole suite: `listener.accept()` blocks forever and `server.join()`
/// never returns (observed as a >840s stick under parallel nextest).
///
/// Sized at 3x the 10s read/write timeouts below: spawn + exec + connect
/// is strictly more work than one socket read, and it lands in tens of
/// milliseconds even under load (measured 2026-09-20 on the dev VM:
/// 0.019–0.113s for the connect-inclusive fixture tests inside a full
/// 1212-test parallel run, 0.020–0.090s solo) — so 30s is >250x margin
/// against a flaky-fast timeout, while still failing ~28x faster than the
/// observed wedge. See `docs/backlog/resolved/msg-broken-pipe-accept-hang-done.md`.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Poll interval for the non-blocking accept loop. Adds at most this much
/// latency to the normal connect path — noise next to a process spawn.
const ACCEPT_POLL: Duration = Duration::from_millis(10);

/// Accepts one connection, giving up loudly after `deadline` instead of
/// blocking forever. `UnixListener::accept()` has no timeout knob, so the
/// listener goes non-blocking and the loop polls it; the accepted socket
/// itself is blocking again on return (Linux `accept()` clears
/// `SOCK_NONBLOCK` for the new fd, and this is re-asserted explicitly so
/// no later reader inherits a surprising mode).
fn accept_with_deadline(
    listener: &UnixListener,
    deadline: Duration,
) -> std::io::Result<UnixStream> {
    listener.set_nonblocking(true)?;
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "fake IPC server: no client connected within {:?} \
                             — the scootctl child failed to spawn or connect",
                            deadline
                        ),
                    ));
                }
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Serves `reply_line` (one `\n`-terminated JSON line) to the first client
/// that connects, then goes away. Reads the request first so a client that
/// writes before reading never blocks on a full socket buffer.
///
/// Takes an already-bound listener rather than a path: binding on the
/// calling thread *before* the server thread and the `scootctl` child are
/// spawned is the happens-before edge that a thread-internal `bind()` (the
/// pre-fix shape) lacked. Without it, the child could `connect()` before
/// the server thread bound, take ENOENT, and exit 1 in milliseconds while
/// the server thread polled `WouldBlock` for the full 30s — a red
/// `TimedOut` naming the wrong cause (~18% of rapid isolation reruns,
/// observed 4/22 by the PR #179 reviewer). Program-order `bind` → spawn
/// closes the race completely, with no retry timing to size.
///
/// The accept carries a deadline: a child that never connects (failed
/// spawn, failed exec, connect hiccup under load) used to wedge the suite
/// forever via `server.join()`, so it is a loud panic naming the cause
/// instead. The panic message is preserved across the thread boundary by
/// `join_server` below.
fn serve_once(listener: UnixListener, reply_line: Vec<u8>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let stream = accept_with_deadline(&listener, ACCEPT_TIMEOUT).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        assert!(!request.is_empty(), "expected one request line");
        let mut writer = stream;
        writer.write_all(&reply_line).unwrap();
        writer.flush().unwrap();
    })
}

/// A `windows` reply whose pretty-printed form is ~1 MiB: `msg windows`
/// pretty-prints, so the compact wire form only needs to clear ~512 KiB.
fn big_windows_reply() -> Vec<u8> {
    let mut windows = String::from(r#"{"type":"windows","windows":["#);
    for id in 0..2000 {
        if id > 0 {
            windows.push(',');
        }
        // ~250 bytes per window on the wire; the `title` dominates.
        windows.push_str(&format!(
            r#"{{"id":{id},"app_id":"test","title":"window {id} {}","output":0,"rect":{{"x":0,"y":0,"width":800,"height":600}},"visible":true,"focused":false}}"#,
            "t".repeat(150)
        ));
    }
    windows.push_str("]}\n");
    assert!(
        windows.len() > 512 * 1024,
        "reply must dwarf the pipe buffer"
    );
    windows.into_bytes()
}

fn small_windows_reply() -> Vec<u8> {
    br#"{"type":"windows","windows":[{"id":1,"app_id":"test","title":"one","output":0,"rect":{"x":0,"y":0,"width":800,"height":600},"visible":true,"focused":true}]}
"#
    .to_vec()
}

/// Joins a `serve_once` thread, preserving the fixture's own panic message.
/// `JoinHandle::unwrap()` would discard it behind `Any { .. }`, turning a
/// named cause ("no client connected within ...") into an anonymous join
/// failure — the loudness this fixture exists for.
fn join_server(server: std::thread::JoinHandle<()>) {
    if let Err(payload) = server.join() {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn closed_stdout_on_a_large_reply_exits_quietly() {
    let path = socket_path("large");
    let _ = std::fs::remove_file(&path);
    // Bind here, before the server thread or the child exists, so the
    // child's `connect()` cannot race the `bind()` (see `serve_once`).
    // A collision fails here, loudly, instead of inside the thread.
    let listener = UnixListener::bind(&path).unwrap();
    let server = serve_once(listener, big_windows_reply());

    let mut child = Command::new(scootctl())
        .env("SCOOT_SOCKET", &path)
        .args(["windows"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Close the read end before the child can write: the 1 MiB reply
    // cannot fit the pipe buffer, so the child must take EPIPE.
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    let _ = std::fs::remove_file(&path);
    join_server(server);
    assert!(
        status.success(),
        "closed stdout should exit 0, got {status}"
    );
}

#[test]
fn closed_stdout_on_help_exits_quietly() {
    let mut child = Command::new(scootctl())
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "closed stdout should exit 0, got {status}"
    );
}

#[test]
fn a_full_read_is_unchanged() {
    let path = socket_path("full");
    let _ = std::fs::remove_file(&path);
    // Bind before spawn — same race as above (see `serve_once`).
    let listener = UnixListener::bind(&path).unwrap();
    let server = serve_once(listener, small_windows_reply());

    let output = Command::new(scootctl())
        .env("SCOOT_SOCKET", &path)
        .args(["windows"])
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    join_server(server);
    assert!(output.status.success(), "got {}", output.status);
    let text = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["windows"][0]["title"], "one");
}

/// The accept deadline fires instead of hanging: with no client ever
/// connecting, `accept_with_deadline` must return `TimedOut` at the
/// deadline, naming the cause. Deterministic under load — nothing can
/// complete the accept, so the lower bound cannot flake; the upper bound
/// is deliberately generous (15x the 2s deadline) so only a true wedge
/// fails it.
#[test]
fn accept_timeout_fires_without_a_client() {
    let path = socket_path("acc");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let deadline = Duration::from_secs(2);
    let start = Instant::now();
    let err = accept_with_deadline(&listener, deadline).unwrap_err();
    let elapsed = start.elapsed();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(
        err.to_string().contains("no client connected"),
        "timeout must name the cause, got: {err}"
    );
    assert!(
        elapsed >= deadline,
        "returned before the deadline: {elapsed:?}"
    );
    assert!(
        elapsed < deadline * 15,
        "deadline did not fire promptly: {elapsed:?}"
    );
    drop(listener);
    let _ = std::fs::remove_file(&path);
}
