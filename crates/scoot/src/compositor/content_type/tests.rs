//! Tests for `wp_content_type_manager_v1`.
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the manager
//! global, maps an `xdg_toplevel` showing an opaque red shm buffer, labels
//! it with each content type in turn, and the test asserts the framebuffer
//! is byte-identical every time. That is the honest pin for a pure hint on
//! a renderer with no consumer for it: acceptance without effect, not
//! acceptance asserted on a compositor-side field.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::content_type::v1::client::{
    wp_content_type_manager_v1, wp_content_type_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer these tests render into. Small on purpose: one 64x64
/// window's fill is findable in it without room for doubt.
const CANVAS: i32 = 320;

/// The side, in pixels, of the opaque red buffer each toplevel paints.
const SURFACE: i32 = 64;

/// One instruction for the client thread.
enum Step {
    /// Report whether the compositor advertised the manager global.
    ReportGlobals,
    /// Map the red window and create its content-type object (still
    /// unlabeled: `none` until a type is set).
    MapWindow,
    /// `set_content_type` on the window's type object, then attach, damage
    /// and commit so the double-buffered hint applies.
    SetType { value: wp_content_type_v1::Type },
    /// `wp_content_type_manager_v1.destroy`.
    DestroyManager,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Globals { content_type: bool },
    Done,
}

impl Ack {
    fn content_type(self) -> bool {
        match self {
            Ack::Globals { content_type } => content_type,
            _ => panic!("expected a globals report"),
        }
    }
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn start() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }
}

/// The client end of the one test connection: just enough of a toolkit to
/// bind the manager, map a red toplevel, and drive its type object.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<wp_content_type_manager_v1::WpContentTypeManagerV1>,
    surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    content: Option<wp_content_type_v1::WpContentTypeV1>,
    /// The newest `xdg_surface.configure` serial, of the one window this
    /// client ever maps.
    window_serial: Option<u32>,
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
        // Version 1 of each is all this needs, so ask for exactly that and
        // stay independent of what the compositor advertises.
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wp_content_type_manager_v1::WpContentTypeManagerV1::interface().name
        {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClient {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Nothing in scoot pings today, but a client that ignores one is a
        // client that can be killed for it.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            client.window_serial = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wp_content_type_manager_v1::WpContentTypeManagerV1);
wayland_client::delegate_noop!(TestClient: ignore wp_content_type_v1::WpContentTypeV1);

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque red pixels, over a real
/// memfd -- the same path any toolkit takes.
fn red_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("scoot-content-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    // Little-endian `Argb8888`: bytes B, G, R, A -- opaque red.
    let pixel = [0x00u8, 0x00, 0xFF, 0xFF];
    let bytes: Vec<u8> = pixel.repeat(len / 4);
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        SURFACE,
        SURFACE,
        stride,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

/// Attach, damage and commit the window's buffer: what moves the pending
/// content type (and every other double-buffered state set since the last
/// commit) into the committed state.
fn commit_window(client: &TestClient) -> Result<(), String> {
    let surface = client.surface.clone().ok_or("no mapped window")?;
    let buffer = client.buffer.clone().ok_or("no buffer to commit")?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, SURFACE, SURFACE);
    surface.commit();
    Ok(())
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let ack = match step {
            Step::ReportGlobals => Ack::Globals {
                content_type: client.manager.is_some(),
            },
            Step::MapWindow => {
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                let manager = client
                    .manager
                    .clone()
                    .ok_or("no wp_content_type_manager_v1 -- the global is missing")?;
                // Mapped in two commits, the way the protocol asks: the
                // role-only commit first, then pixels once the compositor's
                // configure has been acked.
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_title("content".into());
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "an xdg configure", |client| {
                    client.window_serial
                })?;
                xdg.ack_configure(serial);
                let buffer = red_buffer(&shm, &qh)?;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, SURFACE, SURFACE);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                client.content = Some(manager.get_surface_content_type(&surface, &qh, ()));
                client.surface = Some(surface);
                client.buffer = Some(buffer);
                Ack::Done
            }
            Step::SetType { value } => {
                let content = client.content.clone().ok_or("no content object")?;
                content.set_content_type(value);
                commit_window(&client)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DestroyManager => {
                let manager = client.manager.take().ok_or("no manager to destroy")?;
                manager.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_manager_global_is_advertised() {
    let mut fixture = Fixture::start();
    assert!(
        fixture.run(Step::ReportGlobals).content_type(),
        "wp_content_type_manager_v1 was not advertised"
    );
}

#[test]
fn every_content_type_renders_unchanged() {
    use wp_content_type_v1::Type;
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow);
    // The unhinted frame: what every hinted frame below must match exactly.
    // A hint this compositor ignores changes no pixel by construction, and
    // this pins that rather than asserting on a stored field no render path
    // reads.
    let baseline = fixture.render();
    for value in [Type::Photo, Type::Video, Type::Game, Type::None] {
        fixture.run(Step::SetType { value });
        let pixels = fixture.render();
        assert_eq!(
            pixels, baseline,
            "content type {value:?} must not change a single pixel"
        );
    }
}

#[test]
fn destroying_the_manager_leaves_its_objects_working() {
    use wp_content_type_v1::Type;
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow);
    let baseline = fixture.render();
    fixture.run(Step::DestroyManager);
    // The spec says the child objects are unaffected: the surviving type
    // object must still accept a hint after its manager is gone, and the
    // frame must still match the unhinted one.
    fixture.run(Step::SetType { value: Type::Video });
    assert_eq!(
        fixture.render(),
        baseline,
        "a content object must keep working after its manager is destroyed"
    );
}
