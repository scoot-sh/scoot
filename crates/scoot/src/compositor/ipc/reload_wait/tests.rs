//! Tests for the deferred wallpaper-wait reload (see `super`).
//!
//! Driven through a real event loop with real IPC clients (socket pairs
//! through `accept`), not `handle_request` stubs: the bug this replaces
//! was about *when the loop gets its thread back*, which only a real loop
//! observes.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use scoot_core::Config;
use scoot_ipc::{Request, Response, decode, encode};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::super::connection::Limits;
use super::super::slots::Slots;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;

/// How long any "pump until something happens" loop waits.
const PATIENCE: Duration = Duration::from_secs(20);

/// A stand-in `scootbg`: records argv, waits while `hold` exists, exits
/// with `status`.
struct Stub {
    dir: tempfile::TempDir,
}

impl Stub {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let script = format!(
            r#"#!/bin/sh
d='{dir}'
line=''
for a in "$@"; do line="$line[$a]"; done
printf '%s\n' "$line" >>"$d/argv"
while [ -e "$d/hold" ]; do sleep 0.01; done
exit "$(cat "$d/status" 2>/dev/null || echo 0)"
"#,
            dir = dir.path().display()
        );
        let path = dir.path().join("scootbg");
        fs::write(&path, script).expect("the stub");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        Self { dir }
    }

    fn path(&self) -> std::path::PathBuf {
        self.dir.path().join("scootbg")
    }

    fn hold(&self) {
        fs::write(self.dir.path().join("hold"), "").unwrap();
    }

    fn release(&self) {
        let _ = fs::remove_file(self.dir.path().join("hold"));
    }
}

struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
    slots: Slots,
}

impl Harness {
    fn new() -> Self {
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
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state");
        headless::init(&mut state, 200, 200).expect("a render target");
        Self {
            event_loop,
            state,
            slots: Slots::new(),
        }
    }

    fn with_outputs(count: u32) -> Self {
        let mut harness = Self::new();
        for index in 2..=count {
            headless::add_output(&mut harness.state, &format!("headless-{index}"), 200, 200)
                .expect("another output");
        }
        harness
    }

    fn install_config(&mut self, contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, contents).expect("a config file");
        self.state.config_path = Some(path);
        dir
    }

    fn connect(&mut self) -> TestClient {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        super::super::accept(&mut self.state, server, &self.slots, Limits::REAL)
            .expect("the connection is taken");
        client
            .set_nonblocking(true)
            .expect("a non-blocking test client");
        TestClient {
            stream: client,
            received: Vec::new(),
        }
    }

    fn pump(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(1)), &mut self.state)
            .expect("a compositor dispatch");
        self.state.reap_children();
        // The wallpaper reaper answers ready waits; the tick drains slow
        // replies and times out hung ones. Both run here so tests do not
        // depend on the frame timer being armed.
        let _ = self.state.settle_reload_wait();
    }

    fn pump_until(&mut self, complaint: &str, mut settled: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !settled(self) {
            assert!(Instant::now() < deadline, "{complaint}");
            self.pump();
        }
    }
}

struct TestClient {
    stream: UnixStream,
    received: Vec<u8>,
}

impl TestClient {
    fn send(&mut self, request: &Request) {
        let line = encode(request).expect("encodable");
        match self.stream.write_all(line.as_bytes()) {
            Ok(()) => {}
            Err(e) => panic!("write failed: {e}"),
        }
    }

    fn collect(&mut self) {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => self.received.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(e) => panic!("read failed: {e}"),
            }
        }
    }

    fn take_line(&mut self) -> Option<String> {
        let end = self.received.iter().position(|b| *b == b'\n')?;
        let line: Vec<u8> = self.received.drain(..=end).collect();
        Some(String::from_utf8(line).expect("utf-8"))
    }

    fn expect_reply(&mut self, harness: &mut Harness) -> Response {
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.collect();
            if let Some(line) = self.take_line() {
                return decode(&line).expect("decodable");
            }
            assert!(Instant::now() < deadline, "no reply arrived");
            harness.pump();
        }
    }

    fn expect_silence(&mut self, harness: &mut Harness) {
        for _ in 0..40 {
            harness.pump();
            self.collect();
        }
        assert!(
            self.take_line().is_none(),
            "a reply arrived that should not have"
        );
    }
}

fn reloaded(response: Response) -> (Vec<String>, Vec<String>) {
    match response {
        Response::Reloaded {
            applied, refused, ..
        } => (applied, refused),
        other => panic!("expected reloaded, got {other:?}"),
    }
}

#[test]
fn a_wait_with_no_wallpaper_section_answers_at_once_like_plain() {
    let mut harness = Harness::new();
    let _dir = harness.install_config("[layout]\ngap = 20\n");
    let mut client = harness.connect();
    client.send(&Request::ReloadWaitWallpaper);
    let (applied, refused) = reloaded(client.expect_reply(&mut harness));
    assert!(applied.contains(&"layout.gap".to_owned()), "{applied:?}");
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        harness.state.pending_reload_wait.is_empty(),
        "no wallpaper run means no park"
    );
}

#[test]
fn a_wait_parks_until_the_run_exits_and_the_loop_stays_live() {
    let stub = Stub::new();
    let mut harness = Harness::new();
    let command = stub.path().to_str().unwrap().to_owned();
    let _dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let mut waiter = harness.connect();
    waiter.send(&Request::ReloadWaitWallpaper);
    // The reload applied, but the reply is parked while the stub holds.
    stub.hold();
    // Pump once to let the reload run and park; the stub holds so no answer.
    // Note: hold file created after config install but before pump -- the
    // first run starts on reload and sees the hold.
    waiter.expect_silence(&mut harness);
    assert_eq!(
        harness.state.pending_reload_wait.len(),
        1,
        "one wait parks one reply"
    );
    // The loop stays live while parked: a concurrent version lands at once.
    let mut probe = harness.connect();
    probe.send(&Request::Version);
    let reply = probe.expect_reply(&mut harness);
    assert!(matches!(reply, Response::Version { .. }), "{reply:?}");
    // The parked reply still waits.
    waiter.collect();
    assert!(waiter.take_line().is_none(), "still parked");
    stub.release();
    let (applied, refused) = reloaded(waiter.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()), "{applied:?}");
    assert!(refused.is_empty(), "{refused:?}");
    assert!(harness.state.pending_reload_wait.is_empty());
}

#[test]
fn two_waits_park_two_replies() {
    let stub = Stub::new();
    stub.hold();
    let mut harness = Harness::new();
    let command = stub.path().to_str().unwrap().to_owned();
    let dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let mut first = harness.connect();
    let mut second = harness.connect();
    first.send(&Request::ReloadWaitWallpaper);
    // Let the first reload run (with the old file) and park before the file
    // changes: the file is read at serve time, so rewriting before any pump
    // would make both reloads read the new file and the second report
    // `Unchanged`. Pumping here parks the first on color A.
    first.expect_silence(&mut harness);
    assert_eq!(harness.state.pending_reload_wait.len(), 1);
    // Second reload changes the color so it reports wallpaper too (an
    // identical second reload would be `Unchanged` with empty applied, yet
    // still park its own run -- the queue runs even unchanged sections).
    std::fs::write(
        dir.path().join("config.toml"),
        format!("[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000002\"\n"),
    )
    .unwrap();
    second.send(&Request::ReloadWaitWallpaper);
    // Both park; neither answers while the first run holds.
    second.expect_silence(&mut harness);
    first.collect();
    assert!(first.take_line().is_none(), "first still parked");
    assert_eq!(harness.state.pending_reload_wait.len(), 2);
    stub.release();
    let (applied, _) = reloaded(first.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()));
    let (applied, _) = reloaded(second.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()));
    assert!(harness.state.pending_reload_wait.is_empty());
}

#[test]
fn a_disconnected_client_drops_its_park_without_leaking() {
    let stub = Stub::new();
    stub.hold();
    let mut harness = Harness::new();
    let command = stub.path().to_str().unwrap().to_owned();
    let _dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let mut waiter = harness.connect();
    waiter.send(&Request::ReloadWaitWallpaper);
    waiter.expect_silence(&mut harness);
    assert_eq!(harness.state.pending_reload_wait.len(), 1);
    // The client goes away while parked.
    drop(waiter);
    stub.release();
    // The run exits; the write fails and the entry drops.
    harness.pump_until("the park drops", |h| {
        h.state.pending_reload_wait.is_empty() && h.state.wallpaper.is_idle()
    });
    assert!(harness.state.pending_reload_wait.is_empty());
}

#[test]
fn a_plain_reload_never_parks_and_never_allocates_the_wait_queue() {
    let stub = Stub::new();
    let mut harness = Harness::new();
    let command = stub.path().to_str().unwrap().to_owned();
    let _dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let mut client = harness.connect();
    client.send(&Request::Reload);
    let (applied, _) = reloaded(client.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()));
    assert!(
        harness.state.pending_reload_wait.is_empty(),
        "the common path never touches the wait queue"
    );
    // Let the handed-over run finish so no fd leaks into other tests.
    harness.pump_until("the run reaps", |h| h.state.wallpaper.is_idle());
}

#[test]
fn a_wait_works_across_two_outputs() {
    let stub = Stub::new();
    let mut harness = Harness::with_outputs(2);
    let command = stub.path().to_str().unwrap().to_owned();
    let _dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000002\"\n"
    ));
    let mut waiter = harness.connect();
    waiter.send(&Request::ReloadWaitWallpaper);
    let (applied, refused) = reloaded(waiter.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()), "{applied:?}");
    assert!(refused.is_empty(), "{refused:?}");
    harness.pump_until("the run reaps", |h| h.state.wallpaper.is_idle());
}

#[test]
fn a_hung_run_times_out_handed_over_but_unwaited_on() {
    use std::time::Duration as StdDuration;

    let stub = Stub::new();
    stub.hold();
    let mut harness = Harness::new();
    let command = stub.path().to_str().unwrap().to_owned();
    let _dir = harness.install_config(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let mut waiter = harness.connect();
    waiter.send(&Request::ReloadWaitWallpaper);
    waiter.expect_silence(&mut harness);
    assert_eq!(harness.state.pending_reload_wait.len(), 1);
    // Age past the 30 s bound without parking the test thread for it.
    harness
        .state
        .age_reload_waits_for_test(StdDuration::from_secs(31));
    let (applied, refused) = reloaded(waiter.expect_reply(&mut harness));
    assert!(applied.contains(&"wallpaper".to_owned()), "{applied:?}");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("timed out")
            && refused[0].contains("handed over")
            && refused[0].contains("exit unwaited-on"),
        "{refused:?}"
    );
    // The hung run keeps going; its exit is still reaped and logged.
    stub.release();
    harness.pump_until("the run reaps", |h| {
        h.state.wallpaper.is_idle() && h.state.pending_reload_wait.is_empty()
    });
}
