//! Tests for `zwlr_gamma_control_manager_v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding the manager
//! and sending memfds exactly as `gammastep` does -- through a real [`State`]
//! with a real headless backend. What is under test is what the client
//! observes (`gamma_size`, `failed`, survival vs. protocol error) plus, for
//! the lifecycle cases, what the compositor still holds afterwards, which a
//! handler-level test cannot see.
//!
//! The client runs on its own thread while the test pumps the compositor;
//! each test is one linear script reporting back one result. See
//! `selection/tests.rs` for the shared shape.
//!
//! Two cases have no test here, deliberately:
//!
//! - `get_gamma_control` naming an unknown output: unreachable from a real
//!   client. `wl_output` objects are unforgeable -- the only one that can be
//!   named is the compositor's own, which always validates -- so the `failed`
//!   branch is defence for a multi-output future, not a reachable path today.
//! - Applying to real hardware: headless has no LUT, so everything here
//!   asserts accept/retire semantics. The `--tty` apply path is
//!   exercised on the dev VM (see the PR description), not in this file.
//!
//! These need a writable `$XDG_RUNTIME_DIR` for the same reason the other
//! compositor tests do.

use std::io::Write;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use scoot_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};

use super::FALLBACK_GAMMA_SIZE;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// How long a client script may take before the compositor counts as not
/// answering. Generous: a debug build on a VM.
const PATIENCE: Duration = Duration::from_secs(20);

/// A live compositor with a real headless backend, serving client threads
/// that each run one script to completion.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
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
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, 200, 200).expect("a headless backend");
        Self { event_loop, state }
    }

    /// Connects one client over a socket pair and runs `script` on its
    /// thread.
    fn run_client(
        &mut self,
        script: impl FnOnce(ClientConn) -> Result<String, String> + Send + 'static,
    ) -> JoinHandle<Result<String, String>> {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");
        std::thread::spawn(move || {
            let conn = ClientConn::new(client_end)?;
            script(conn)
        })
    }

    /// Pumps the compositor until the client thread reports back, failing the
    /// test if the compositor stops serving first.
    fn wait_for(&mut self, handle: JoinHandle<Result<String, String>>) -> Result<String, String> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if handle.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the client; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
        handle.join().expect("the client thread")
    }

    /// Runs one client script that is expected *not* to survive -- a request
    /// the compositor answers with a protocol error -- and returns what the
    /// dying client said. Survival panics.
    fn wait_for_disconnect(&mut self, handle: JoinHandle<Result<String, String>>) -> String {
        match self.wait_for(handle) {
            Ok(outcome) => {
                panic!("the client survived a request that should have been refused ({outcome})")
            }
            Err(error) => error,
        }
    }

    /// Dispatches until nothing more arrives, so destructions the client
    /// caused (an explicit destroy, or its disconnect) have run their
    /// server-side restore before the test inspects [`State`].
    fn settle(&mut self) {
        for _ in 0..20 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }
}

/// The client end of one connection: the socket, queue and dispatch state.
struct ClientConn {
    queue: wayland_client::EventQueue<Client>,
    client: Client,
}

impl ClientConn {
    fn new(stream: UnixStream) -> Result<Self, String> {
        let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = Client::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        Ok(Self { queue, client })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.client)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Everything the gamma client binds or observes.
#[derive(Default)]
struct Client {
    manager: Option<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1>,
    output: Option<wl_output::WlOutput>,
    /// One slot per control created, in creation order: the last `gamma_size`
    /// seen on it, and whether it has been told `failed`.
    sizes: Vec<Option<u32>>,
    failed: Vec<bool>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "zwlr_gamma_control_manager_v1" => {
                client.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_output" => client.output = Some(registry.bind(name, version.min(4), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
        _: zwlr_gamma_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Which control in creation order an event belongs to: `get_gamma_control`
/// hands over the object first and its `gamma_size` after.
struct ControlIndex(usize);

impl Dispatch<zwlr_gamma_control_v1::ZwlrGammaControlV1, ControlIndex> for Client {
    fn event(
        client: &mut Self,
        _: &zwlr_gamma_control_v1::ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        index: &ControlIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(slot) = client.sizes.get_mut(index.0) {
                    *slot = Some(size);
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                if let Some(slot) = client.failed.get_mut(index.0) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);

/// Rounds the client's queue until `ready` sees what the test is waiting for.
fn wait_for_event(
    conn: &mut ClientConn,
    what: &str,
    mut ready: impl FnMut(&Client) -> bool,
) -> Result<(), String> {
    for _ in 0..50 {
        conn.roundtrip()?;
        if ready(&conn.client) {
            return Ok(());
        }
    }
    Err(format!("the compositor never sent {what}"))
}

/// Binds the manager and the output, or says which one is missing. Every
/// gamma test starts here.
fn manager_and_output(
    conn: &ClientConn,
) -> Result<
    (
        zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
        wl_output::WlOutput,
    ),
    String,
> {
    let manager = conn
        .client
        .manager
        .clone()
        .ok_or("no zwlr_gamma_control_manager_v1 -- the global is missing")?;
    let output = conn.client.output.clone().ok_or("no wl_output")?;
    Ok((manager, output))
}

/// A correctly-sized ramp fd: `3 * size` little-endian `u16` entries over a
/// memfd, the way `gammastep` sends them. `entry` fills every slot (tests
/// use distinctive values so a mix-up reads as a mismatch, not a pass).
fn ramp_fd(size: u32, entry: u16) -> std::fs::File {
    use rustix::fs::{MemfdFlags, memfd_create};

    let fd = memfd_create("scoot-gamma-test", MemfdFlags::CLOEXEC).expect("a memfd");
    let mut file = std::fs::File::from(fd);
    for _ in 0..3 * size {
        file.write_all(&entry.to_le_bytes()).expect("a filled ramp");
    }
    file
}

/// A memfd holding exactly `len` zero bytes.
fn sized_fd(len: usize) -> std::fs::File {
    use rustix::fs::{MemfdFlags, memfd_create};

    let fd = memfd_create("scoot-gamma-test", MemfdFlags::CLOEXEC).expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0u8; len]).expect("a filled file");
    file
}

/// Asserts a disconnect report is the `invalid_gamma` refusal: the client
/// sees the numeric code (`invalid_gamma` is value 1) on the control object,
/// not the enum name.
fn assert_invalid_gamma(error: &str) {
    assert!(
        error.contains("Protocol error 1") && error.contains("zwlr_gamma_control_v1"),
        "expected an invalid_gamma protocol error, saw: {error}",
    );
}

#[test]
fn gamma_size_then_accept() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        if conn.client.sizes[0] != Some(FALLBACK_GAMMA_SIZE) {
            return Err(format!(
                "expected gamma_size {}, saw {:?}",
                FALLBACK_GAMMA_SIZE, conn.client.sizes[0],
            ));
        }
        // A correctly-sized ramp is accepted: the client hears nothing back
        // (the protocol has no acknowledgement) and -- the actual assertion
        // -- survives to tell about it.
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x8000);
        control.set_gamma(file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        if conn.client.failed[0] {
            return Err("a valid set_gamma was answered with failed".into());
        }
        control.destroy();
        Ok("gamma_size advertised and a valid ramp accepted".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("gamma_size advertised and a valid ramp accepted"),
    );
    // The destroy restored the default: nothing current.
    harness.settle();
    assert!(
        harness.state.gamma_control.current.is_empty(),
        "destroying the current control must retire it"
    );
}

#[test]
fn gamma_wrong_size_refused() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        // Eleven bytes are neither empty nor a ramp: `invalid_gamma`.
        let file = sized_fd(11);
        control.set_gamma(file.as_fd());
        // The error arrives as a disconnect; anything after it never runs.
        for _ in 0..50 {
            conn.roundtrip()?;
        }
        Ok("survived an invalid set_gamma".into())
    });
    let error = harness.wait_for_disconnect(handle);
    assert_invalid_gamma(&error);
}

#[test]
fn gamma_empty_fd_refused() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        let file = sized_fd(0);
        control.set_gamma(file.as_fd());
        for _ in 0..50 {
            conn.roundtrip()?;
        }
        Ok("survived an empty set_gamma".into())
    });
    let error = harness.wait_for_disconnect(handle);
    assert_invalid_gamma(&error);
}

#[test]
fn gamma_oversized_fd_refused() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        // A megabyte where 1536 bytes belong: refused after reading only a
        // bounded prefix (`expected` plus one 4096-byte chunk -- positioned
        // reads overshoot the stop by less than a chunk), not mapped wholesale.
        let file = sized_fd(1024 * 1024);
        control.set_gamma(file.as_fd());
        for _ in 0..50 {
            conn.roundtrip()?;
        }
        Ok("survived an oversized set_gamma".into())
    });
    let error = harness.wait_for_disconnect(handle);
    assert_invalid_gamma(&error);
}

#[test]
fn gamma_second_control_transfers() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let first = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        // A ramp on the first control, so the transfer has something to
        // supersede -- and so a mix-up (second control treated as a no-op
        // duplicate) reads as a missing `failed`, not a pass.
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x1111);
        first.set_gamma(file.as_fd());
        conn.roundtrip()?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let second = manager.get_gamma_control(&output, &qh, ControlIndex(1));
        wait_for_event(&mut conn, "failed on the first control", |client| {
            client.failed[0]
        })?;
        wait_for_event(&mut conn, "gamma_size on the second control", |client| {
            client.sizes[1].is_some()
        })?;
        // The new control works: a ramp through it is accepted.
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x2222);
        second.set_gamma(file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        if conn.client.failed[1] {
            return Err("a valid set_gamma on the new control was answered with failed".into());
        }
        Ok("control transferred to the second client object".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("control transferred to the second client object"),
    );
}

#[test]
fn gamma_destroying_superseded_control_restores_nothing() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let first = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x1111);
        first.set_gamma(file.as_fd());
        conn.roundtrip()?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let second = manager.get_gamma_control(&output, &qh, ControlIndex(1));
        wait_for_event(&mut conn, "failed on the first control", |client| {
            client.failed[0]
        })?;
        wait_for_event(&mut conn, "gamma_size on the second control", |client| {
            client.sizes[1].is_some()
        })?;
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x2222);
        second.set_gamma(file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        if conn.client.failed[1] {
            return Err("a valid set_gamma on the new control was answered with failed".into());
        }
        // Destroying the superseded control must not restore the default
        // over the live one's ramp: the destroy gate restores only for the
        // current control. The proof is a third control -- if the second is
        // still live when it arrives, the transfer fails the second; if a
        // restore already retired it, nothing fails.
        first.destroy();
        conn.roundtrip()?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let third = manager.get_gamma_control(&output, &qh, ControlIndex(2));
        wait_for_event(&mut conn, "gamma_size on the third control", |client| {
            client.sizes[2].is_some()
        })?;
        wait_for_event(
            &mut conn,
            "failed on the still-live second control",
            |client| client.failed[1],
        )?;
        third.destroy();
        Ok("destroying the superseded control left the live one alone".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("destroying the superseded control left the live one alone"),
    );
    // Destroying the live third control restored the default: nothing current.
    harness.settle();
    assert!(
        harness.state.gamma_control.current.is_empty(),
        "destroying the current control must retire it"
    );
}

#[test]
fn gamma_disconnect_restores_default() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        let file = ramp_fd(FALLBACK_GAMMA_SIZE, 0x3333);
        control.set_gamma(file.as_fd());
        conn.roundtrip()?;
        // Return with the connection still open: dropping `conn` here
        // disconnects mid-control, which must restore the default exactly
        // like an explicit destroy.
        Ok("disconnecting with a live control".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("disconnecting with a live control"),
    );
    harness.settle();
    assert!(
        harness.state.gamma_control.current.is_empty(),
        "a disconnect must retire the live control"
    );
}

#[test]
fn gamma_pipe_that_never_delivers_is_refused() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        // A pipe with the write end held open and nothing written: a
        // blocking read would park the compositor here. The non-blocking
        // read sees nothing and refuses instead.
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "a pipe");
        let read_end = unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[0]) };
        let _write_end = unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[1]) };
        control.set_gamma(read_end.as_fd());
        // The error arrives as a disconnect; `wait_for_disconnect` asserts
        // the client does *not* come back from these round trips.
        for _ in 0..50 {
            conn.roundtrip()?;
        }
        Ok("survived a never-delivering set_gamma".into())
    });
    let error = harness.wait_for_disconnect(handle);
    assert_invalid_gamma(&error);
}

#[test]
fn gamma_rapid_sets_stay_alive() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        conn.client.sizes.push(None);
        conn.client.failed.push(false);
        let control = manager.get_gamma_control(&output, &qh, ControlIndex(0));
        wait_for_event(&mut conn, "gamma_size", |client| client.sizes[0].is_some())?;
        // Two hundred back-to-back ramps: `set_gamma` at well past any real
        // daemon's rate must neither kill the client nor lose the control.
        //
        // Flushed every 64, with no round trip between: still back to back,
        // but under wayland-backend's received-fd bound. This client (on
        // `wayland-client`'s pure-Rust backend) grows its outgoing buffer
        // without limit and sends every fd past the
        // last 28 of a flush ahead of the requests that claim them, so a
        // single flush of more than 140 fd-carrying requests leaves more than
        // 128 unclaimed and is disconnected before scoot sees a request (see
        // `fd_pressure/tests/backend_queue.rs`). Batching is the client
        // library's shape, not the rate under test here.
        for i in 0..200u16 {
            let file = ramp_fd(FALLBACK_GAMMA_SIZE, i);
            control.set_gamma(file.as_fd());
            if i % 64 == 63 {
                conn.queue.flush().map_err(|e| e.to_string())?;
            }
        }
        conn.roundtrip()?;
        conn.roundtrip()?;
        if conn.client.failed[0] {
            return Err("rapid valid set_gamma calls were answered with failed".into());
        }
        Ok("two hundred rapid ramps accepted".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("two hundred rapid ramps accepted"),
    );
}
