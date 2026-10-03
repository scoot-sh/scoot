//! A real `dbus-daemon`, for the tests that must meet one: the fake bus
//! speaks what this crate's author understood the protocol to be, and a
//! daemon is where that was wrong before (it drops a reply with no
//! destination, answers `REJECTED` to a hex uid in the auth line).
//!
//! The daemon is the system's (`dbus-daemon` on `PATH`), private to the
//! test: its own socket in its own directory, killed and removed on drop.
//! A machine without one skips, saying so, unless
//! `SCOOTBAR_REQUIRE_DBUS_DAEMON` is set, which makes that a failure (CI
//! sets it, as the integration tests do for `scoot`).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::conn::{Conn, Event};

pub struct Daemon {
    child: Child,
    dir: PathBuf,
}

impl Daemon {
    /// A private session bus, or `None` (the test skips) without
    /// `dbus-daemon`.
    pub fn spawn() -> Option<Self> {
        let found = Command::new("dbus-daemon")
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            assert!(
                std::env::var_os("SCOOTBAR_REQUIRE_DBUS_DAEMON").is_none(),
                "SCOOTBAR_REQUIRE_DBUS_DAEMON is set but there is no dbus-daemon on PATH"
            );
            eprintln!("skipped: no dbus-daemon on PATH");
            return None;
        }
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "scootbar-dbus-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a private directory for the daemon");
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--address"])
            .arg(format!("unix:path={}", dir.join("bus").display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon starts");
        let daemon = Self { child, dir };
        for _ in 0..500 {
            if daemon.path().exists() {
                return Some(daemon);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("dbus-daemon never made its socket");
    }

    /// The bus socket.
    pub fn path(&self) -> PathBuf {
        self.dir.join("bus")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Pumps `conn` until `want` picks an event (returned), or fails the
/// test after ten seconds: the bus never answered.
pub fn until(conn: &mut Conn, mut want: impl FnMut(&Event) -> bool) -> Event {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let start = std::time::Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the bus never sent what the test waits for"
        );
        let (events, _capped) = conn.pump();
        if let Some(event) = events.into_iter().find(|event| want(event)) {
            return event;
        }
        let fd = conn.as_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        let _ = poll(&mut fds, Some(&timeout));
    }
}

/// Reads from `stream` until `want` has been read, byte by byte.
fn read_until(stream: &mut UnixStream, want: &[u8]) {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(want) {
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        got.push(byte[0]);
    }
}

/// Plays a daemon's side of auth and `Hello` on `stream` (the client
/// names itself `:1.7`), and returns. Blocking, with a ten-second read
/// timeout so a client that stops talking fails the test.
pub fn serve_setup(stream: &mut UnixStream) {
    use super::proto::{Message, Writer, frame_at};
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    read_until(stream, b"AUTH EXTERNAL\r\n");
    stream.write_all(b"DATA\r\n").unwrap();
    read_until(stream, b"DATA\r\n");
    stream.write_all(b"OK 0123456789abcdef\r\n").unwrap();
    read_until(stream, b"BEGIN\r\n");
    // The `Hello` call: framed, then answered.
    let mut staged = Vec::new();
    let mut chunk = [0u8; 4096];
    let len = loop {
        if let Ok(Some(len)) = frame_at(&staged) {
            break len;
        }
        let n = stream.read(&mut chunk).unwrap();
        staged.extend_from_slice(&chunk[..n]);
    };
    let call = Message::parse(&staged[..len]).unwrap();
    assert_eq!(call.member, Some("Hello"));
    let mut body = Writer::new();
    body.str(":1.7");
    let body = body.take_body().unwrap();
    let mut reply = Writer::new();
    reply.begin_return_to(1, ":1.7", call.serial, "s");
    reply.raw(&body);
    stream.write_all(&reply.finish().unwrap()).unwrap();
}
