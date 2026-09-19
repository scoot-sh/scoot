//! Tests for `wp_single_pixel_buffer_manager_v1`.
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the manager
//! global, creates real single-pixel buffers with real RGBA values, attaches
//! one to a real `xdg_toplevel` (scaled up through `wp_viewporter`, the shape
//! the spec points toolkits at), and the test asserts on the RGBA the
//! compositor stored and -- for the render test -- on the pixels the real
//! `PixmanRenderer` drew. That last part is the point: "the compositor
//! accepted the buffer" is a claim about the framebuffer, and a test that
//! asserted on a compositor-side field would pass just as happily against a
//! version that never drew it.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::renderer::buffer_dimensions;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer as ServerBuffer;
use smithay::utils::{Buffer as BufferCoord, Size};
use smithay::wayland::single_pixel_buffer::get_single_pixel_buffer;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1;
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, contains, wait_for};

/// The framebuffer these tests render into. Small on purpose: the render
/// test only needs one window's fill to be findable in it.
const CANVAS: i32 = 120;
/// The `wp_viewport.set_destination` square the render test scales its 1x1
/// buffer into -- big enough to count pixels in, small enough to leave no
/// doubt which surface they came from.
const DEST: i32 = 48;

/// Pure red, opaque, as the `u32` channels a client spells it and as the BGRA
/// bytes a pixman `Argb8888` framebuffer holds it in. `u32::MAX` is exactly
/// representable through the `f32` conversion the render path uses (both
/// endpoints round to the same `f32`, so the quotient is exactly 1.0), which
/// is why this color -- and not a mid-range one -- is what the pixel
/// assertion uses. Mid-range conversion (`0x80808080` to 128) is integer math
/// in `rgba8888` and pinned server-side instead.
const RED_U32: (u32, u32, u32, u32) = (u32::MAX, 0, 0, u32::MAX);
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];

/// One instruction for the client thread.
enum Step {
    /// Report which of the two globals the compositor advertised.
    ReportGlobals,
    /// `create_u32_rgba_buffer` with these channels; the buffer is held
    /// client-side under the returned index.
    CreateBuffer { r: u32, g: u32, b: u32, a: u32 },
    /// Map an `xdg_toplevel` showing buffer `index` scaled to `DEST`x`DEST`
    /// through `wp_viewporter`.
    MapWindow { index: usize },
    /// `wp_single_pixel_buffer_manager_v1.destroy`.
    DestroyManager,
    /// `wl_buffer.destroy` for buffer `index`.
    DestroyBuffer { index: usize },
}

/// What a client answers a [`Step`] with.
enum Ack {
    Globals { manager: bool, viewporter: bool },
    BufferCreated { index: usize, protocol_id: u32 },
    Done,
}

impl Ack {
    fn globals(self) -> (bool, bool) {
        match self {
            Ack::Globals {
                manager,
                viewporter,
            } => (manager, viewporter),
            _ => panic!("expected a globals report"),
        }
    }

    fn buffer(self) -> (usize, u32) {
        match self {
            Ack::BufferCreated { index, protocol_id } => (index, protocol_id),
            _ => panic!("expected a created buffer"),
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

    /// The server-side `WlBuffer` for a client-reported protocol id.
    fn server_buffer(&self, protocol_id: u32) -> ServerBuffer {
        self.client(0)
            .object_from_protocol_id::<ServerBuffer>(&self.state.display_handle, protocol_id)
            .expect("the compositor still holds the client's buffer object")
    }
}

/// The client end of the one test connection: just enough of a toolkit to
/// bind the manager, mint solid-color buffers, and map a toplevel showing
/// one of them.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    manager: Option<wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
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
        } else if interface
            == wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1::interface().name
        {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wp_viewporter::WpViewporter::interface().name {
            client.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
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
        // Nothing in flexwm pings today, but a client that ignores one is a
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
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1);
wayland_client::delegate_noop!(TestClient: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(TestClient: ignore wp_viewport::WpViewport);

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    // The manager and the viewporter are *not* required up front: the
    // absence half of the global test is a client that binds what exists
    // and reports what does not.
    let mut buffers: Vec<wl_buffer::WlBuffer> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let ack = match step {
            Step::ReportGlobals => Ack::Globals {
                manager: client.manager.is_some(),
                viewporter: client.viewporter.is_some(),
            },
            Step::CreateBuffer { r, g, b, a } => {
                let manager = client
                    .manager
                    .clone()
                    .ok_or("no wp_single_pixel_buffer_manager_v1 -- the global is missing")?;
                let buffer = manager.create_u32_rgba_buffer(r, g, b, a, &qh, ());
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let protocol_id = buffer.id().protocol_id();
                buffers.push(buffer);
                Ack::BufferCreated {
                    index: buffers.len() - 1,
                    protocol_id,
                }
            }
            Step::MapWindow { index } => {
                let buffer = buffers.get(index).ok_or("no such buffer")?.clone();
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_title("single-pixel".into());
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "an xdg configure", |client| {
                    client.window_serial
                })?;
                xdg.ack_configure(serial);
                // The spec's own recipe: a 1x1 buffer scaled to a real size
                // through viewporter, so the window is big enough to find.
                let viewporter = client
                    .viewporter
                    .clone()
                    .ok_or("no wp_viewporter -- the global is missing")?;
                let viewport = viewporter.get_viewport(&surface, &qh, ());
                viewport.set_destination(DEST, DEST);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, DEST, DEST);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DestroyManager => {
                let manager = client.manager.take().ok_or("no manager to destroy")?;
                manager.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DestroyBuffer { index } => {
                let buffer = buffers.get(index).ok_or("no such buffer")?;
                buffer.destroy();
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
    let (manager, viewporter) = fixture.run(Step::ReportGlobals).globals();
    assert!(
        manager,
        "wp_single_pixel_buffer_manager_v1 was not advertised"
    );
    assert!(
        viewporter,
        "wp_viewporter was not advertised -- the spec's own scaling recipe for these buffers needs it"
    );
}

#[test]
fn created_buffers_carry_their_rgba_values_and_use_no_shm() {
    let mut fixture = Fixture::start();
    let (_, red_id) = fixture
        .run(Step::CreateBuffer {
            r: RED_U32.0,
            g: RED_U32.1,
            b: RED_U32.2,
            a: RED_U32.3,
        })
        .buffer();
    // The boundary the spec calls out: every channel's valid range is the
    // full `uint`, so 0 and `u32::MAX` must both be storable, not clamped.
    let (_, ends_id) = fixture
        .run(Step::CreateBuffer {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        })
        .buffer();

    let red_buffer = fixture.server_buffer(red_id);
    let red =
        get_single_pixel_buffer(&red_buffer).expect("a created buffer is a single-pixel buffer");
    assert_eq!((red.r, red.g, red.b, red.a), RED_U32);
    assert_eq!(red.rgba8888(), [255, 0, 0, 255]);
    assert_eq!(red.rgba32f(), [1.0, 0.0, 0.0, 1.0]);
    assert!(!red.has_alpha(), "an opaque buffer has no alpha");

    let ends_buffer = fixture.server_buffer(ends_id);
    let ends =
        get_single_pixel_buffer(&ends_buffer).expect("a created buffer is a single-pixel buffer");
    assert_eq!((ends.r, ends.g, ends.b, ends.a), (0, 0, 0, 0));
    assert_eq!(ends.rgba32f(), [0.0, 0.0, 0.0, 0.0]);
    assert!(ends.has_alpha(), "a zero-alpha buffer has alpha");

    // Mid-range conversion is integer math, pinned exactly: 0x80808080 is
    // 128/255ths of u32::MAX, rounded to the nearest step.
    let (_, mid_id) = fixture
        .run(Step::CreateBuffer {
            r: 0x8080_8080,
            g: 0x8080_8080,
            b: 0x8080_8080,
            a: u32::MAX,
        })
        .buffer();
    let mid_buffer = fixture.server_buffer(mid_id);
    let mid =
        get_single_pixel_buffer(&mid_buffer).expect("a created buffer is a single-pixel buffer");
    assert_eq!(mid.rgba8888(), [128, 128, 128, 255]);

    // Smithay knows these buffers as 1x1, which is what makes the viewport
    // destination -- not the buffer -- decide the surface size.
    let size: Option<Size<i32, BufferCoord>> = buffer_dimensions(&fixture.server_buffer(red_id));
    assert_eq!(size, Some(Size::from((1, 1))));

    // No pool was allocated for any of this: single-pixel buffers bypass
    // `wl_shm` entirely, so the per-client pool budget must read untouched.
    assert_eq!(
        fixture.state.shm_pools.pools_in_flight(),
        0,
        "creating single-pixel buffers must not claim shm pool budget"
    );
    // But all three buffers count toward the per-client *buffer* budget:
    // the release hook cannot tell buffer kinds apart, so a selective
    // count would drift fail-open (see `wl_buffers.rs`).
    assert_eq!(
        fixture.state.wl_buffers.buffers_in_flight(),
        3,
        "creating single-pixel buffers must claim buffer budget"
    );
}

#[test]
fn an_attached_single_pixel_buffer_renders_its_color() {
    let mut fixture = Fixture::start();
    let (index, _) = fixture
        .run(Step::CreateBuffer {
            r: RED_U32.0,
            g: RED_U32.1,
            b: RED_U32.2,
            a: RED_U32.3,
        })
        .buffer();
    fixture.run(Step::MapWindow { index });

    let pixels = fixture.render();
    assert!(
        contains(&pixels, RED_BGRA),
        "the single-pixel buffer's color must reach the framebuffer"
    );
    let filled = pixels
        .chunks_exact(4)
        .filter(|pixel| *pixel == RED_BGRA.as_slice())
        .count();
    assert!(
        filled >= (DEST as usize) * (DEST as usize) / 2,
        "a {DEST}x{DEST} viewport destination should fill on that order of pixels, found {filled}"
    );
}

#[test]
fn destroying_the_manager_leaves_its_buffers_usable() {
    let mut fixture = Fixture::start();
    let (index, protocol_id) = fixture
        .run(Step::CreateBuffer {
            r: RED_U32.0,
            g: RED_U32.1,
            b: RED_U32.2,
            a: RED_U32.3,
        })
        .buffer();
    fixture.run(Step::DestroyManager);
    // The spec: child objects created via the manager are unaffected by its
    // destruction. The client surviving this step at all already proves no
    // protocol error was posted; mapping and rendering from the orphaned
    // buffer proves it still names a real buffer.
    fixture.run(Step::MapWindow { index });
    let buffer = fixture.server_buffer(protocol_id);
    let data = get_single_pixel_buffer(&buffer).expect("the orphaned buffer still has its color");
    assert_eq!((data.r, data.g, data.b, data.a), RED_U32);

    let pixels = fixture.render();
    assert!(
        contains(&pixels, RED_BGRA),
        "a buffer orphaned by its manager's destruction must still draw"
    );
}

#[test]
fn destroying_an_attached_buffer_keeps_client_and_compositor_alive() {
    let mut fixture = Fixture::start();
    let (index, _) = fixture
        .run(Step::CreateBuffer {
            r: RED_U32.0,
            g: RED_U32.1,
            b: RED_U32.2,
            a: RED_U32.3,
        })
        .buffer();
    fixture.run(Step::MapWindow { index });
    fixture.run(Step::DestroyBuffer { index });
    // What is asserted here is survival, not pixels: destroying a `wl_buffer`
    // its surface still names is legal, and must cost neither the client
    // (no protocol error) nor the compositor (no panic on the next render).
    // The client answering another step proves the first half; a render
    // plus a still-serving compositor proves the second.
    let pixels = fixture.render();
    assert_eq!(
        pixels.len(),
        (CANVAS as usize) * (CANVAS as usize) * 4,
        "a render after the destroy must still produce a full frame"
    );
    let (manager, _) = fixture.run(Step::ReportGlobals).globals();
    assert!(
        manager,
        "the client must still be connected after the destroy"
    );
}
