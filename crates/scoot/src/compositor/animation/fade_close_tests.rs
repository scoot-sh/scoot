//! A real-client pin for the open-only fade.
//!
//! The unit suite in `tests.rs` drives `update_slides` with no Wayland
//! clients, so it cannot see what a real close does: foot destroys its
//! `wl_surface` on close, and the retained close fade then yields no
//! elements (a live headless run showed the closed slot at background on
//! the very first frame). These tests drive a real `wayland-client`
//! toplevel showing an opaque red shm buffer through a real `State` with
//! `[animation]` enabled, and assert on both the fade list and the pixels
//! a real renderer drew:
//!
//! - a new window starts exactly one open fade and first draws
//!   transparent (no fully-opaque pixel), then opaque once it lands;
//! - destroying the toplevel and its surface (what foot does on close)
//!   leaves no fade behind and the pixels snap to background.
//!
//! The second test fails against the close-fade design (the close retains
//! a fade entry that only prunes a `duration` later); it passes with the
//! open-only fade. Like the other real-client suites here, these need a
//! writable `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, contains, wait_for};

/// The framebuffer these tests render into. Small on purpose: one 64x64
/// window's fill is findable in it without room for doubt.
const CANVAS: i32 = 320;

/// The side, in pixels, of the opaque red buffer the toplevel paints.
const SURFACE: i32 = 64;

/// Opaque red, as the BGRA bytes a pixman `Argb8888` framebuffer holds it
/// in.
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];

/// The fade length these tests run with: the same 500 ms the live run that
/// found the bugs used, so "first draws transparent" cannot be a race.
const FADE: Duration = Duration::from_millis(500);

/// One instruction for the client thread.
enum Step {
    /// Map the red window (two commits: role-only, then pixels after the
    /// compositor's configure).
    MapWindow,
    /// Destroy the toplevel and its surface, the way foot closes: no null
    /// commit, no goodbye frame — the objects are just gone.
    CloseWindow,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Done,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn start() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.state.animation.enabled = true;
        fixture.state.animation.duration = FADE;
        fixture.spawn(run_client);
        fixture
    }
}

/// The client end of the one test connection: just enough of a toolkit to
/// map a red toplevel and then destroy it like foot does.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    surface: Option<wl_surface::WlSurface>,
    xdg: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    buffer: Option<wl_buffer::WlBuffer>,
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

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque red pixels, over a real
/// memfd -- the same path any toolkit takes.
fn red_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("scoot-fade-test", rustix::fs::MemfdFlags::CLOEXEC)
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
        match step {
            Step::MapWindow => {
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                // Mapped in two commits, the way the protocol asks: the
                // role-only commit first, then pixels once the compositor's
                // configure has been acked.
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_title("fade".into());
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
                client.surface = Some(surface);
                client.xdg = Some(xdg);
                client.toplevel = Some(toplevel);
                client.buffer = Some(buffer);
            }
            Step::CloseWindow => {
                // Foot's close: destroy the objects outright. No null-buffer
                // commit first, so the compositor never sees a goodbye frame.
                let toplevel = client.toplevel.take().ok_or("no mapped window")?;
                let xdg = client.xdg.take().ok_or("no mapped window")?;
                let surface = client.surface.take().ok_or("no mapped window")?;
                toplevel.destroy();
                xdg.destroy();
                surface.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
        }
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[test]
fn a_new_window_fades_in_from_transparent() {
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow);
    // Exactly one open fade for the real window: the fade the live run
    // showed working (body alpha ~0.12 -> ~0.56 -> 1.0 over 500 ms).
    assert_eq!(
        fixture.state.fades.len(),
        1,
        "a new window must start exactly one open fade"
    );
    // First frame: the fade starts at alpha 0, so no fully-opaque window
    // pixel may show yet.
    assert!(
        !contains(&fixture.render(), RED_BGRA),
        "a window mid-open-fade must draw translucent, never fully opaque"
    );
    // Past the deadline the fade lands and prunes, and the window is opaque.
    fixture.tick(FADE + Duration::from_millis(300));
    fixture.state.prune_slides(Instant::now());
    assert!(
        fixture.state.fades.is_empty(),
        "a landed open fade must prune so the compositor idles"
    );
    assert!(
        contains(&fixture.render(), RED_BGRA),
        "a settled window must draw fully opaque"
    );
}

#[test]
fn a_close_snaps_with_no_fade_left_behind() {
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow);
    // Let the open fade land first, so the close is a close of a settled
    // window, not a close-during-open.
    fixture.tick(FADE + Duration::from_millis(300));
    fixture.state.prune_slides(Instant::now());
    assert!(fixture.state.fades.is_empty());
    assert!(
        contains(&fixture.render(), RED_BGRA),
        "precondition: the settled window must be on screen first"
    );
    // Destroy the surface outright, like foot: the close must snap, leaving
    // no fade entry behind. (Against the close-fade design this fails: the
    // close retains a fade that only prunes a duration later, while drawing
    // nothing — dead surfaces yield no elements.)
    fixture.run(Step::CloseWindow);
    fixture.tick(Duration::from_millis(100));
    fixture.state.prune_slides(Instant::now());
    assert!(
        fixture.state.fades.is_empty(),
        "a close snaps: no closing fade may be retained"
    );
    assert!(
        !contains(&fixture.render(), RED_BGRA),
        "the closed window must be gone from the framebuffer immediately"
    );
}
