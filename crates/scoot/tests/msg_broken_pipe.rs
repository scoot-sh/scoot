#![cfg(unix)]
//! `scoot msg` must exit quietly when its stdout reader goes away.
//!
//! Each test serves one canned IPC reply from a fake Unix-socket server
//! (no compositor needed), runs the real `scoot` binary against it via
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

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn scoot() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scoot"))
}

fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scoot-epipe-test-{}-{}-{}.sock",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Serves `reply_line` (one `\n`-terminated JSON line) to the first client
/// that connects, then goes away. Reads the request first so a client that
/// writes before reading never blocks on a full socket buffer.
fn serve_once(path: PathBuf, reply_line: Vec<u8>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let listener = UnixListener::bind(&path).unwrap();
        let (stream, _) = listener.accept().unwrap();
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

#[test]
fn closed_stdout_on_a_large_reply_exits_quietly() {
    let path = socket_path("large");
    let _ = std::fs::remove_file(&path);
    let server = serve_once(path.clone(), big_windows_reply());

    let mut child = Command::new(scoot())
        .env("SCOOT_SOCKET", &path)
        .args(["msg", "windows"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Close the read end before the child can write: the 1 MiB reply
    // cannot fit the pipe buffer, so the child must take EPIPE.
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    let _ = std::fs::remove_file(&path);
    server.join().unwrap();
    assert!(
        status.success(),
        "closed stdout should exit 0, got {status}"
    );
}

#[test]
fn closed_stdout_on_help_exits_quietly() {
    let mut child = Command::new(scoot())
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
    let server = serve_once(path.clone(), small_windows_reply());

    let output = Command::new(scoot())
        .env("SCOOT_SOCKET", &path)
        .args(["msg", "windows"])
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    server.join().unwrap();
    assert!(output.status.success(), "got {}", output.status);
    let text = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["windows"][0]["title"], "one");
}
