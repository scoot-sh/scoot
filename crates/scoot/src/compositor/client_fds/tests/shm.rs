//! Wire tests for pool fds a surface keeps after the client destroyed both the
//! `wl_buffer` and the pool: the shape
//! `docs/backlog/resolved/buffer-fds-past-their-object-done.md` is about.
//!
//! A real `wayland-client` creates a pool and a buffer on a tagged memfd,
//! attaches and commits it on a fresh surface, and destroys the buffer and the
//! pool, once per surface. The client closes its own copy of each memfd once
//! it is sent, so the fds in this process's table carrying the tag are
//! exactly the ones the compositor keeps, and the mappings carrying it in
//! `/proc/self/maps` are the compositor's too. Every assertion on the
//! ledger's count is paired with one on those.
//!
//! Pinned to pixman: the subject is what the surface keeps, not the renderer.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use super::super::{Kind, MAX_FDS_PER_CLIENT};
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::dispatch::tests::{ensure_dispatch_flood_headroom, hold_flood_lock};
use crate::compositor::test_support::Harness;

/// `wl_shm.error.invalid_stride`: what every `create_pool` refusal posts.
const INVALID_STRIDE: u32 = 1;

enum Step {
    /// For each of `surfaces` fresh surfaces: create a pool and a buffer,
    /// attach and commit it, destroy the buffer and the pool, and keep the
    /// surface. A round trip after each, so a kill is reported with how many
    /// surfaces were already held.
    Hold { surfaces: u32 },
    /// Destroy every kept surface.
    DestroySurfaces,
}

type Fixture = Harness<Step, ()>;

fn start(tag: &'static str) -> Fixture {
    let mut fixture = Harness::headless_on(Appearance::default(), 32, RendererKind::Pixman);
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, tag));
    fixture
}

/// How many fds in this process's table are memfds named with `tag`.
fn held_fds(tag: &str) -> usize {
    let needle = format!("/memfd:{tag} ");
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|link| link.to_string_lossy().starts_with(&needle))
        .count()
}

/// How many mappings in this process are of memfds named with `tag`.
fn held_mappings(tag: &str) -> usize {
    let needle = format!("/memfd:{tag} ");
    std::fs::read_to_string("/proc/self/maps")
        .expect("/proc/self/maps")
        .lines()
        .filter(|line| line.contains(&needle))
        .count()
}

/// Dispatches until the fds tagged `tag` are down to `expected`. Smithay
/// closes a dropped pool's fd on its own "Shm dropping thread", a moment
/// after the pool is dropped, so a count read straight after a destroy can
/// still see it.
fn settle_until_fds(fixture: &mut Fixture, tag: &str, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while held_fds(tag) != expected {
        assert!(
            Instant::now() < deadline,
            "{} fds tagged {tag} still open, expected {expected}",
            held_fds(tag)
        );
        fixture.settle();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The ticket's shape, measured before this ledger at 200 surfaces holding
/// 200 fds with 0 buffers and 0 pools counted. Every one of those fds (and
/// its mapping) is counted now, against the client that sent it.
#[test]
fn fds_a_surface_keeps_after_its_buffer_and_pool_die_are_counted() {
    const TAG: &str = "scoot-cfd-kept";
    const SURFACES: u32 = 200;
    let mut fixture = start(TAG);
    fixture.run(Step::Hold { surfaces: SURFACES });
    assert_eq!(
        held_fds(TAG),
        SURFACES as usize,
        "the surfaces keep the fds"
    );
    assert_eq!(held_mappings(TAG), SURFACES as usize, "and a mapping each");
    assert_eq!(fixture.state.wl_buffers.buffers_in_flight(), 0);
    assert_eq!(fixture.state.shm_pools.pools_in_flight(), 0);
    let client = fixture.client(0).id();
    assert_eq!(fixture.state.client_fds.held_by(&client), SURFACES);
    assert_eq!(
        fixture.state.client_fds.in_flight(Some(Kind::Pool)),
        SURFACES,
        "a sweep finds every one still open"
    );
}

/// The loop, run to the bound: the pool that would make the 513th fd is
/// refused, the client is disconnected, every fd and mapping it held closes,
/// and the next client is served.
#[test]
fn the_surface_loop_is_refused_at_the_fd_bound() {
    const TAG: &str = "scoot-cfd-bound";
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(u64::from(MAX_FDS_PER_CLIENT));
    let mut fixture = start(TAG);
    let error = fixture.run_expecting_disconnect(Step::Hold {
        surfaces: MAX_FDS_PER_CLIENT + 64,
    });
    assert!(
        error.contains(&format!("after {MAX_FDS_PER_CLIENT} surfaces held"))
            && error.contains(&format!("code {INVALID_STRIDE} on wl_shm"))
            && error.contains(&format!("and the maximum is {MAX_FDS_PER_CLIENT}")),
        "{error}"
    );
    settle_until_fds(&mut fixture, TAG, 0);
    assert_eq!(
        held_mappings(TAG),
        0,
        "the killed client's mappings went too"
    );
    assert_eq!(fixture.state.client_fds.in_flight(None), 0);

    let second = fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, TAG));
    fixture.run_on(second, Step::Hold { surfaces: 2 });
    let id = fixture.client(second).id();
    assert_eq!(fixture.state.client_fds.held_by(&id), 2);
}

/// Legitimate churn never ratchets toward the bound: fds a client stopped
/// keeping are forgotten (their numbers come back on its next pools, or a
/// sweep finds them closed), so after releasing 300 it can keep the whole
/// bound again.
#[test]
fn released_fds_stop_counting() {
    const TAG: &str = "scoot-cfd-released";
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(u64::from(MAX_FDS_PER_CLIENT));
    let mut fixture = start(TAG);
    fixture.run(Step::Hold { surfaces: 300 });
    fixture.run(Step::DestroySurfaces);
    settle_until_fds(&mut fixture, TAG, 0);
    assert_eq!(held_mappings(TAG), 0);
    assert_eq!(fixture.state.client_fds.in_flight(None), 0);
    fixture.run(Step::Hold {
        surfaces: MAX_FDS_PER_CLIENT,
    });
    assert_eq!(held_fds(TAG), MAX_FDS_PER_CLIENT as usize);
    assert_eq!(
        fixture.state.client_fds.in_flight(Some(Kind::Pool)),
        MAX_FDS_PER_CLIENT
    );
}

/// A disconnect releases everything: the surfaces go, and with them the fds.
/// The dead client's records linger until a sweep or their numbers come back,
/// which is bounded by the fd table (see the module doc), and never counts
/// against anyone else.
#[test]
fn a_disconnect_closes_every_kept_fd() {
    const TAG: &str = "scoot-cfd-disconnect";
    let mut fixture = start(TAG);
    fixture.run(Step::Hold { surfaces: 40 });
    fixture.disconnect(0);
    settle_until_fds(&mut fixture, TAG, 0);
    assert_eq!(fixture.state.client_fds.in_flight(None), 0);
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    synced: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
}

/// Names memfds uniquely across every client of every test in the process.
static SERIAL: AtomicU32 = AtomicU32::new(0);

fn run_client(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<()>,
    tag: &'static str,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let mut surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    while let Ok(step) = steps.recv() {
        match step {
            Step::Hold { surfaces: count } => {
                for _ in 0..count {
                    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
                    let fd = rustix::fs::memfd_create(
                        format!("{tag} {serial}"),
                        rustix::fs::MemfdFlags::CLOEXEC,
                    )
                    .map_err(|e| e.to_string())?;
                    rustix::fs::ftruncate(&fd, 4096).map_err(|e| e.to_string())?;
                    let pool = shm.create_pool(fd.as_fd(), 4096, &qh, ());
                    let buffer = pool.create_buffer(0, 4, 4, 16, wl_shm::Format::Argb8888, &qh, ());
                    let surface = compositor.create_surface(&qh, ());
                    surface.attach(Some(&buffer), 0, 0);
                    surface.commit();
                    buffer.destroy();
                    pool.destroy();
                    let held = surfaces.len();
                    surfaces.push(surface);
                    sync(&conn, &mut queue, &mut client)
                        .map_err(|error| format!("after {held} surfaces held: {error}"))?;
                    // The compositor has its own copy now.
                    drop(fd);
                }
            }
            Step::DestroySurfaces => {
                for surface in surfaces.drain(..) {
                    surface.destroy();
                }
            }
        }
        sync(&conn, &mut queue, &mut client)?;
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A round trip that cannot lose a protocol error to `EPIPE`, the same
/// shape (and for the same race) as `drm_syncobj/tests.rs`'s `sync`.
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
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
                _ => {}
            }
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
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
