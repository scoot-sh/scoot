//! Wire tests for the pending-plane bound (see `pending_planes.rs`).
//!
//! A real `wayland-client` adds real memfd planes to real params objects on
//! a real pixman compositor. Every assertion on the count is paired with one
//! on the fds themselves. Each memfd is named with a tag unique to the test,
//! and the client closes its own copies once they are sent, so the fds in
//! this process's table that carry the tag are exactly the ones the
//! compositor still holds. So "the count drained" and "the fds closed" are
//! checked separately, and they have to agree.
//!
//! Pinned to pixman, like the other dma-buf suites: the subject is the params
//! object, not the renderer.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{wl_callback, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

use super::{
    MAX_PENDING_PLANES_PER_CLIENT, PRESSURE_GRACE_PENDING_PLANES, Refusal, plane_refusal,
};
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

/// `zwp_linux_buffer_params_v1.error.plane_idx`.
const PLANE_IDX: u32 = 1;

/// Size of every plane's memfd. The async `create` checks the plane against
/// the file's real size, so it must cover `STRIDE * HEIGHT`.
const PLANE_BYTES: u64 = 4096;
const WIDTH: i32 = 4;
const HEIGHT: i32 = 4;
const STRIDE: u32 = WIDTH as u32 * 4;

enum Step {
    /// Create `params` params objects and add `planes` planes to each,
    /// keeping all of them. A round trip after each params object, so a kill
    /// is reported with how far it got.
    Hoard { params: u32, planes: u32 },
    /// Add one plane with an out-of-range index to a fresh params object,
    /// which Smithay refuses with `plane_idx`.
    BadPlaneIndex,
    /// Consume every kept params object with the asynchronous `create`. The
    /// memfd planes are not dma-bufs, so pixman refuses each import and the
    /// client is sent `failed` and lives.
    ConsumeAll,
    /// Destroy every kept params object.
    DestroyAll,
}

enum Ack {
    Done,
    /// How many `failed` events the client has had so far.
    Failed(u32),
}

type Fixture = Harness<Step, Ack>;

/// A pixman compositor with one client connected, whose memfds carry `tag`.
fn start(tag: &'static str) -> Fixture {
    let mut fixture = Harness::headless_on(Appearance::default(), 32, RendererKind::Pixman);
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, tag));
    fixture
}

impl Fixture {
    fn done(&mut self, step: Step) {
        match self.run(step) {
            Ack::Done => {}
            Ack::Failed(_) => panic!("expected done"),
        }
    }

    fn done_on(&mut self, index: usize, step: Step) {
        match self.run_on(index, step) {
            Ack::Done => {}
            Ack::Failed(_) => panic!("expected done"),
        }
    }

    fn planes(&self) -> u32 {
        self.state.pending_planes.in_flight()
    }
}

/// How many fds in this process's table are memfds named with `tag`. With the
/// client's own copies closed, these are the compositor's.
fn held_fds(tag: &str) -> usize {
    let needle = format!("/memfd:{tag} ");
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|link| link.to_string_lossy().starts_with(&needle))
        .count()
}

// ---------------------------------------------------------------------------
// The bound
// ---------------------------------------------------------------------------

/// The review's shape, on a smaller scale: params objects with planes added
/// and never created. Before this bound nothing counted them; 220 x 4 held
/// 927 fds. Now the 33rd plane disconnects the client, which is the first
/// add of the ninth four-plane params object, and everything it held goes
/// with it.
#[test]
fn pending_planes_are_bounded_per_client() {
    const TAG: &str = "scoot-pp-bound";
    let mut fixture = start(TAG);
    let error = fixture.run_expecting_disconnect(Step::Hoard {
        params: 64,
        planes: 4,
    });
    assert!(
        error.contains("after 9 params")
            && error.contains(&format!("code {NO_MEMORY}"))
            && error.contains("wl_display")
            && error.contains("it has not created a buffer from"),
        "{error}"
    );
    assert_eq!(fixture.planes(), 0, "the killed client's count drained");
    assert_eq!(fixture.state.pending_planes.params_tracked(), 0);
    assert_eq!(held_fds(TAG), 0, "and its plane fds really closed");
}

#[test]
fn exactly_the_bound_is_allowed() {
    const TAG: &str = "scoot-pp-exact";
    let mut fixture = start(TAG);
    fixture.done(Step::Hoard {
        params: MAX_PENDING_PLANES_PER_CLIENT / 4,
        planes: 4,
    });
    assert_eq!(fixture.planes(), MAX_PENDING_PLANES_PER_CLIENT);
    assert_eq!(held_fds(TAG), MAX_PENDING_PLANES_PER_CLIENT as usize);
}

// ---------------------------------------------------------------------------
// Pairing: every way a plane's fd leaves releases its unit, exactly once
// ---------------------------------------------------------------------------

/// Consuming a params object hands its planes to a `Dmabuf`. Here pixman
/// refuses the import and the dma-buf is dropped, so the fds close and the
/// client lives. Its bound is back to full, so the cap is not ratcheted by
/// imports that fail.
#[test]
fn a_consumed_params_object_releases_its_planes() {
    const TAG: &str = "scoot-pp-consume";
    let mut fixture = start(TAG);
    fixture.done(Step::Hoard {
        params: 4,
        planes: 2,
    });
    assert_eq!(fixture.planes(), 8);
    assert_eq!(held_fds(TAG), 8);
    let Ack::Failed(failed) = fixture.run(Step::ConsumeAll) else {
        panic!("expected the failed count");
    };
    assert_eq!(failed, 4, "every import was answered failed, softly");
    assert_eq!(fixture.planes(), 0);
    assert_eq!(fixture.state.pending_planes.params_tracked(), 0);
    assert_eq!(held_fds(TAG), 0);
    // Destroying the consumed objects afterwards releases nothing twice.
    fixture.done(Step::DestroyAll);
    assert_eq!(fixture.planes(), 0);
    fixture.done(Step::Hoard {
        params: MAX_PENDING_PLANES_PER_CLIENT / 4,
        planes: 4,
    });
    assert_eq!(fixture.planes(), MAX_PENDING_PLANES_PER_CLIENT);
}

#[test]
fn a_destroyed_params_object_releases_its_planes() {
    const TAG: &str = "scoot-pp-destroy";
    let mut fixture = start(TAG);
    fixture.done(Step::Hoard {
        params: MAX_PENDING_PLANES_PER_CLIENT / 4,
        planes: 4,
    });
    fixture.done(Step::DestroyAll);
    assert_eq!(fixture.planes(), 0);
    assert_eq!(fixture.state.pending_planes.params_tracked(), 0);
    assert_eq!(held_fds(TAG), 0);
    // The whole bound is available again.
    fixture.done(Step::Hoard {
        params: MAX_PENDING_PLANES_PER_CLIENT / 4,
        planes: 4,
    });
    assert_eq!(fixture.planes(), MAX_PENDING_PLANES_PER_CLIENT);
}

#[test]
fn a_disconnect_releases_every_pending_plane() {
    const TAG: &str = "scoot-pp-disconnect";
    let mut fixture = start(TAG);
    fixture.done(Step::Hoard {
        params: 5,
        planes: 3,
    });
    assert_eq!(fixture.planes(), 15);
    fixture.disconnect(0);
    assert_eq!(fixture.planes(), 0);
    assert_eq!(fixture.state.pending_planes.params_tracked(), 0);
    assert_eq!(held_fds(TAG), 0);
}

/// An `add` Smithay refuses was claimed before delegation. Its client dies,
/// and the claim goes with the params object's destruction rather than
/// staying behind as a phantom.
#[test]
fn an_add_smithay_refuses_leaves_no_phantom() {
    const TAG: &str = "scoot-pp-refused";
    let mut fixture = start(TAG);
    fixture.done(Step::Hoard {
        params: 1,
        planes: 2,
    });
    let error = fixture.run_expecting_disconnect(Step::BadPlaneIndex);
    assert!(
        error.contains(&format!("code {PLANE_IDX}")) && error.contains("zwp_linux_buffer_params_v1"),
        "{error}"
    );
    assert_eq!(fixture.planes(), 0);
    assert_eq!(fixture.state.pending_planes.params_tracked(), 0);
    assert_eq!(held_fds(TAG), 0);
}

/// Per client: one client's releases never touch another's count, and one
/// client at its bound does not stop another from adding.
#[test]
fn the_bound_is_per_client() {
    const TAG: &str = "scoot-pp-two";
    let mut fixture = start(TAG);
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, TAG));
    fixture.done_on(
        0,
        Step::Hoard {
            params: MAX_PENDING_PLANES_PER_CLIENT / 4,
            planes: 4,
        },
    );
    fixture.done_on(
        1,
        Step::Hoard {
            params: 2,
            planes: 4,
        },
    );
    fixture.done_on(1, Step::DestroyAll);
    let first = fixture.client(0).id();
    assert_eq!(
        fixture.state.pending_planes.live_for(&first),
        MAX_PENDING_PLANES_PER_CLIENT,
        "the other client's releases left this one's count alone"
    );
    fixture.done_on(
        1,
        Step::Hoard {
            params: MAX_PENDING_PLANES_PER_CLIENT / 4,
            planes: 4,
        },
    );
    assert_eq!(fixture.planes(), 2 * MAX_PENDING_PLANES_PER_CLIENT);
}

// ---------------------------------------------------------------------------
// The decision, both boundaries, without filling the fd table
// ---------------------------------------------------------------------------

#[test]
fn the_cap_refuses_at_the_cap_whatever_the_table() {
    assert_eq!(plane_refusal(MAX_PENDING_PLANES_PER_CLIENT - 1, || false), None);
    assert_eq!(
        plane_refusal(MAX_PENDING_PLANES_PER_CLIENT, || false),
        Some(Refusal::Cap)
    );
}

#[test]
fn at_the_grace_passes_even_under_pressure() {
    assert_eq!(plane_refusal(PRESSURE_GRACE_PENDING_PLANES, || true), None);
}

#[test]
fn past_the_grace_refuses_only_under_pressure() {
    assert_eq!(
        plane_refusal(PRESSURE_GRACE_PENDING_PLANES + 1, || true),
        Some(Refusal::Pressure)
    );
    assert_eq!(plane_refusal(PRESSURE_GRACE_PENDING_PLANES + 1, || false), None);
}

#[test]
fn under_the_grace_the_table_is_never_observed() {
    for live in 0..=PRESSURE_GRACE_PENDING_PLANES {
        assert_eq!(
            plane_refusal(live, || panic!("observed the table at {live}")),
            None
        );
    }
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    synced: bool,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    params: Vec<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1>,
    failed: u32,
}

/// Counts memfds across every client of every test in the process, so two
/// fds never share a name even within one tag.
static PLANE_SERIAL: AtomicU32 = AtomicU32::new(0);

fn plane(tag: &str) -> Result<OwnedFd, String> {
    let serial = PLANE_SERIAL.fetch_add(1, Ordering::Relaxed);
    let fd = rustix::fs::memfd_create(format!("{tag} {serial}"), rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    rustix::fs::ftruncate(&fd, PLANE_BYTES).map_err(|e| e.to_string())?;
    Ok(fd)
}

fn run_client(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
    tag: &'static str,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    if client.dmabuf.is_none() {
        return Err("no zwp_linux_dmabuf_v1 global".into());
    }

    while let Ok(step) = steps.recv() {
        let dmabuf = client.dmabuf.clone().ok_or("no dmabuf")?;
        let ack = match step {
            Step::Hoard { params, planes } => {
                for n in 0..params {
                    let object = dmabuf.create_params(&qh, ());
                    // The client's own copies live only until the round
                    // trip below has sent them.
                    let mut sent = Vec::new();
                    for index in 0..planes {
                        let fd = plane(tag)?;
                        object.add(fd.as_fd(), index, 0, STRIDE, 0, 0);
                        sent.push(fd);
                    }
                    client.params.push(object);
                    sync(&conn, &mut queue, &mut client)
                        .map_err(|error| format!("after {} params: {error}", n + 1))?;
                    drop(sent);
                }
                Ack::Done
            }
            Step::BadPlaneIndex => {
                let object = dmabuf.create_params(&qh, ());
                let fd = plane(tag)?;
                object.add(fd.as_fd(), 7, 0, STRIDE, 0, 0);
                client.params.push(object);
                sync(&conn, &mut queue, &mut client)?;
                Ack::Done
            }
            Step::ConsumeAll => {
                for object in &client.params {
                    object.create(
                        WIDTH,
                        HEIGHT,
                        u32::from_ne_bytes(*b"XR24"),
                        zwp_linux_buffer_params_v1::Flags::empty(),
                    );
                }
                sync(&conn, &mut queue, &mut client)?;
                Ack::Failed(client.failed)
            }
            Step::DestroyAll => {
                for object in client.params.drain(..) {
                    object.destroy();
                }
                Ack::Done
            }
        };
        sync(&conn, &mut queue, &mut client)?;
        acks.send(ack).map_err(|e| e.to_string())?;
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
                std::thread::sleep(std::time::Duration::from_millis(1));
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
            && interface == "zwp_linux_dmabuf_v1"
        {
            client.dmabuf = Some(registry.bind(name, version.min(3), qh, ()));
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

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            client.failed += 1;
        }
    }

    wayland_client::event_created_child!(TestClient, zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, [
        zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (wayland_client::protocol::wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: <zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wayland_client::protocol::wl_buffer::WlBuffer, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_buffer::WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
