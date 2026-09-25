//! The legitimate shapes that come closest to wayland-backend's unclaimed-fd
//! cap, through a real [`State`] and a real `wayland-client` (its pure-Rust
//! backend, this crate's). The raw-socket pins are in `backend_queue.rs`.
//!
//! The check runs before each read, so it counts fds a client has sent ahead
//! of the requests that will claim them, and well-behaved clients get that
//! far ahead two ways:
//!
//! - **One big flush.** A flush carrying more than 28 fds sends them 28 per
//!   `sendmsg` with one byte each, ahead of the rest of the bytes. The Rust
//!   client grows its outgoing buffer without limit, so every fd-carrying
//!   request it queued since its last flush goes out that way.
//! - **Backpressure.** A client whose socket filled (the compositor busy, or
//!   stopped) keeps queueing, and when the socket drains its fds go out
//!   28 at a time ahead of all the bytes still buffered in front of their
//!   requests. The Rust client does this, and so does libwayland's since its
//!   buffers became unbounded (review of PR #241 measured a stock libwayland
//!   1.26 client disconnected this way at 140 fds under the old fixed 128).
//!
//! These pin both against the cap this process's clients actually get
//! (`backend_queued_fds` of the soft limit, which building a `State`
//! raised): 1024, libwayland-server's own bound, on any machine whose hard
//! limit allows 8192 fds.
//!
//! The fd-carrying request is `zwlr_gamma_control_v1.set_gamma`: its handler
//! reads the ramp at offset zero and drops the fd at once, so nothing but
//! the queue ever holds these fds (a pool's fd would count against the
//! client's 512-fd ledger and close later, on Smithay's drop thread), and
//! one ramp memfd can be sent any number of times.
//!
//! [`State`]: crate::compositor::State

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_callback, wl_compositor, wl_output, wl_region, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};

use super::backend_queue::QUEUE_ERROR;
use crate::compositor::decorations::Appearance;
use crate::compositor::dispatch::tests::hold_flood_lock;
use crate::compositor::fd_pressure::backend_queued_fds;
use crate::compositor::nofile;
use crate::compositor::test_support::Harness;

enum Step {
    /// Bind the gamma manager, get a control and learn its size, make a
    /// region to pad with.
    Setup,
    /// `count` `set_gamma` requests queued, then one flush and a round trip.
    Batch { count: usize },
    /// With the compositor not dispatching: fill the socket with fd-less
    /// requests until three flushes in a row would block, pad with 24 KiB
    /// more, queue `count` `set_gamma` requests, and say so on `queued`.
    /// Not acknowledged: the test does not dispatch until `queued` fires.
    FillThenQueue { count: usize, queued: Sender<()> },
    /// Flush everything queued (waiting out `WouldBlock`), then a round trip.
    Drain,
}

type Fixture = Harness<Step, ()>;

/// The cap every client in this process is created with: the fork reads the
/// soft limit per client, and building the fixture's `State` raised it.
fn cap() -> usize {
    let soft = nofile::raise().expect("RLIMIT_NOFILE is readable").soft;
    backend_queued_fds(soft) as usize
}

/// The largest one-flush batch of `set_gamma` a pure-Rust client can send and
/// still be served at `cap`: its flush puts 28 fds per `sendmsg` ahead of the
/// bytes while more than 28 remain, so `28 * floor(cap / 28)` are queued
/// before the last write, which carries the rest with the bytes that claim
/// them. (The one-byte chunks complete a `set_gamma` only every eight, too
/// few to move the boundary at 128 or 1024.)
fn largest_one_flush_batch(cap: usize) -> usize {
    28 * (cap / 28) + 28
}

fn start() -> Fixture {
    let mut fixture = Fixture::headless(Appearance::default(), 32);
    fixture.spawn(run_client);
    fixture.run(Step::Setup);
    fixture
}

#[test]
fn the_largest_one_flush_batch_is_served() {
    let _flood = hold_flood_lock();
    let mut fixture = start();
    let count = largest_one_flush_batch(cap());
    fixture.run(Step::Batch { count });
}

#[test]
fn one_more_in_the_same_flush_is_disconnected() {
    let _flood = hold_flood_lock();
    let mut fixture = start();
    let count = largest_one_flush_batch(cap()) + 1;
    let error = fixture.run_expecting_disconnect(Step::Batch { count });
    assert!(
        error.contains(QUEUE_ERROR) && error.contains("on wl_display"),
        "{count} in one flush: {error}"
    );
}

/// Backpressure: every one of the `count` fds is queued ahead of bytes still
/// buffered in front of its request, so the boundary is the cap itself.
fn fill_then_queue(count: usize) -> Result<(), String> {
    let mut fixture = start();
    let (queued, filled) = channel();
    fixture.send_step(0, Step::FillThenQueue { count, queued });
    // Deliberately not dispatching while the client fills and queues.
    filled
        .recv_timeout(Duration::from_secs(30))
        .expect("the client filled its socket and queued its requests");
    fixture.run_or_disconnect(Step::Drain)
}

#[test]
fn under_backpressure_the_cap_is_served() {
    let _flood = hold_flood_lock();
    let cap = cap();
    if let Err(error) = fill_then_queue(cap) {
        panic!("{cap} fds queued behind a full socket were refused: {error}");
    }
}

#[test]
fn under_backpressure_one_past_the_cap_is_disconnected() {
    let _flood = hold_flood_lock();
    let count = cap() + 1;
    let error = fill_then_queue(count).expect_err("one past the cap was served");
    assert!(
        error.contains(QUEUE_ERROR) && error.contains("on wl_display"),
        "{count} behind a full socket: {error}"
    );
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    manager: Option<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1>,
    output: Option<wl_output::WlOutput>,
    gamma_size: Option<u32>,
    failed: bool,
    synced: bool,
}

struct Session {
    control: zwlr_gamma_control_v1::ZwlrGammaControlV1,
    region: wl_region::WlRegion,
    ramp: std::fs::File,
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<()>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let mut session: Option<Session> = None;
    while let Ok(step) = steps.recv() {
        match step {
            Step::Setup => {
                conn.display().get_registry(&qh, ());
                sync(&conn, &mut queue, &mut client)?;
                let manager = client.manager.clone().ok_or("no gamma manager")?;
                let output = client.output.clone().ok_or("no wl_output")?;
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let control = manager.get_gamma_control(&output, &qh, ());
                while client.gamma_size.is_none() {
                    sync(&conn, &mut queue, &mut client)?;
                }
                let size = client.gamma_size.unwrap_or_default();
                let mut ramp = std::fs::File::from(
                    rustix::fs::memfd_create("scoot-fdq-ramp", rustix::fs::MemfdFlags::CLOEXEC)
                        .map_err(|e| e.to_string())?,
                );
                for _ in 0..3 * size {
                    ramp.write_all(&0x8000u16.to_le_bytes())
                        .map_err(|e| e.to_string())?;
                }
                let region = compositor.create_region(&qh, ());
                sync(&conn, &mut queue, &mut client)?;
                session = Some(Session {
                    control,
                    region,
                    ramp,
                });
            }
            Step::Batch { count } => {
                let s = session.as_ref().ok_or("no setup")?;
                for _ in 0..count {
                    s.control.set_gamma(s.ramp.as_fd());
                }
                sync(&conn, &mut queue, &mut client)?;
            }
            Step::FillThenQueue { count, queued } => {
                let s = session.as_ref().ok_or("no setup")?;
                let mut blocked = 0;
                while blocked < 3 {
                    s.region.add(0, 0, 1, 1);
                    match conn.flush() {
                        Ok(()) => blocked = 0,
                        Err(WaylandError::Io(error))
                            if error.kind() == std::io::ErrorKind::WouldBlock =>
                        {
                            blocked += 1;
                        }
                        Err(error) => return Err(why(&conn, error)),
                    }
                }
                // More than one read's worth behind the socket, so the fds'
                // requests are still buffered when the fds have all arrived.
                for _ in 0..1024 {
                    s.region.add(0, 0, 1, 1);
                }
                for _ in 0..count {
                    s.control.set_gamma(s.ramp.as_fd());
                }
                queued.send(()).map_err(|e| e.to_string())?;
                continue;
            }
            Step::Drain => sync(&conn, &mut queue, &mut client)?,
        }
        if client.failed {
            return Err("the gamma control failed".into());
        }
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A round trip that cannot lose a protocol error to `EPIPE` (the shape and
/// the race `client_fds/tests/shm.rs`'s `sync` documents), waiting out
/// `WouldBlock` while a full socket drains.
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // Read what the compositor sends meanwhile, or both sides
                // could fill and wait on each other.
                let _ = queue.dispatch_pending(client);
                if let Some(guard) = conn.prepare_read() {
                    let _ = guard.read();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                let _ = queue.dispatch_pending(client);
                if let Some(guard) = conn.prepare_read() {
                    let _ = guard.read();
                }
                let _ = queue.dispatch_pending(client);
                return Err(why(conn, error));
            }
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| why(conn, e))?;
    }
    Ok(())
}

fn why(conn: &Connection, error: impl std::fmt::Display) -> String {
    match conn.protocol_error() {
        Some(error) => format!(
            "protocol error code {} on {}@{}: {}",
            error.code, error.object_interface, error.object_id, error.message
        ),
        None => error.to_string(),
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
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
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "zwlr_gamma_control_manager_v1" => {
                client.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_output" if client.output.is_none() => {
                client.output = Some(registry.bind(name, version.min(4), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_gamma_control_v1::ZwlrGammaControlV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwlr_gamma_control_v1::ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => client.gamma_size = Some(size),
            zwlr_gamma_control_v1::Event::Failed => client.failed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        client.synced = true;
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_region::WlRegion);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1);
