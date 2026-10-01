//! What `scootbar msg subscribe` exits with, against a daemon that says
//! each of the ways a subscription ends. No compositor is needed: the
//! daemon here is a socket that writes what it is told and closes, which
//! is all the client ever sees of the real one.
//!
//! The point is what a script may conclude. Only two endings are
//! distinguishable (a `dropped` line, and a line cut mid-way); a stream
//! that simply ends is exit 0 whether the daemon went away or dropped a
//! subscriber it could not write to, and the docs say so.

use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Output};

const SUBSCRIBED: &str = "{\"type\":\"subscribed\",\"events\":[\"module\",\"output\"]}\n";
const EVENT: &str = "{\"type\":\"output\",\"change\":\"added\",\"name\":\"DP-1\"}\n";
const DROPPED: &str = "{\"type\":\"dropped\"}\n";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sb-sub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `scootbar msg subscribe` against a daemon that writes `bytes` and closes.
fn subscribed_to_a_daemon_that_sends(tag: &str, bytes: &str) -> Output {
    let scratch = Scratch::new(tag);
    let listener = UnixListener::bind(scratch.0.join("scootbar-fake.sock")).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut stream, _) = listener.accept().unwrap();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap() > 0 && byte[0] != b'\n' {}
            let _ = stream.write_all(bytes.as_bytes());
        });
        let output = Command::new(env!("CARGO_BIN_EXE_scootbar"))
            .args(["msg", "subscribe"])
            .env("XDG_RUNTIME_DIR", &scratch.0)
            .env("WAYLAND_DISPLAY", "fake")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        // A command that failed before connecting (a path error) leaves the
        // daemon thread in `accept`, which the scope would wait for for ever
        // and the test would hang instead of fail: one connection to the
        // still-open listener lets it end.
        let _ = std::os::unix::net::UnixStream::connect(scratch.0.join("scootbar-fake.sock"));
        output
    })
}

fn said(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_dropped_line_is_printed_and_the_command_fails() {
    let output =
        subscribed_to_a_daemon_that_sends("dropped", &format!("{SUBSCRIBED}{EVENT}{DROPPED}"));
    let (stdout, stderr) = said(&output);
    // Every line, the last one included, so a script reading the stream sees it.
    assert_eq!(stdout, format!("{SUBSCRIBED}{EVENT}{DROPPED}"));
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("dropped this subscription"), "{stderr}");
    // The line is sent only to a subscriber evicted at a line boundary; one
    // that was too slow is never sent it, so the line names no cause.
    assert!(!stderr.contains("fast enough"), "{stderr}");
    assert!(stderr.contains("subscribe again, then query"), "{stderr}");
}

#[test]
fn a_stream_cut_inside_a_line_fails() {
    let output = subscribed_to_a_daemon_that_sends(
        "cut",
        &format!("{SUBSCRIBED}{}", &EVENT[..EVENT.len() - 10]),
    );
    let (stdout, stderr) = said(&output);
    assert_eq!(stdout, SUBSCRIBED, "the cut line is not printed");
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("middle of a line"), "{stderr}");
}

#[test]
fn a_stream_that_just_ends_is_exit_0_and_says_nothing_of_why() {
    // The daemon exiting and a drop it could not announce (a socket too
    // full to take a line, or a write that happened to stop on a newline)
    // look the same, so a script treats every end as "resubscribe, then
    // query". This pins that the command does not pretend to know.
    let output = subscribed_to_a_daemon_that_sends("ended", &format!("{SUBSCRIBED}{EVENT}"));
    let (stdout, stderr) = said(&output);
    assert_eq!(stdout, format!("{SUBSCRIBED}{EVENT}"));
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "");
}
