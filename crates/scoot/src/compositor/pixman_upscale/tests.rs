//! Pixel tests for upscaled `wl_shm` surfaces under pixman.
//!
//! Every test here drives a real `wayland-client` connection through a real
//! [`State`](crate::compositor::State) with a real headless backend, maps one
//! `xdg_toplevel` showing a solid `#c03020` shm buffer, renders, and asserts
//! on framebuffer bytes -- never on which enum variant a path chose. The
//! fill is opaque, so premultiplied and straight alpha agree and the exact
//! byte value is representable through either renderer: what the assertions
//! pin is what pixman's repeat mode does at the edge, not float rounding.
//!
//! Window placement is read from the live arrangement, never hard-coded, so
//! these survive layout default changes that preserve the tiling contract.
//! Corners are square (`corner_radius = 0`) and the ring paints outside the
//! placement, so the placement rect is exactly window fill -- the census in
//! `rounded::tests::radius_zero_renders_the_square_baseline_exactly` proves
//! that shape, and the centre-pixel assertion below re-proves it per test.
//!
//! Like every other live-`State` suite, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::Rect;
use smithay::utils::{Physical, Rectangle};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::rounded::clip_rect;
use crate::compositor::test_support::{Harness, assert_pixel, wait_for};

/// The framebuffer these tests render into. Small on purpose: one window's
/// fill is findable in it without room for doubt.
const CANVAS: i32 = 320;

/// The ticket's color: `#c03020`, opaque. As the BGRA bytes a pixman
/// `Argb8888` framebuffer holds it in -- and, with the X byte set, as the
/// bytes of an `Xrgb8888` client buffer too.
const FILL_BGRA: [u8; 4] = [0x20, 0x30, 0xC0, 0xFF];

/// One instruction for the client thread: map the one toplevel, drawing its
/// solid buffer at a size the compositor must scale to the window.
#[derive(Clone, Copy)]
enum Step {
    /// A 1x1 `Xrgb8888` buffer scaled to the configured size through
    /// `wp_viewporter` -- the ticket's first case, and the `Operation::Src`
    /// arm (opaque source).
    ViewportedTiny,
    /// A logical-size `Argb8888` buffer with no viewport -- a scale-unaware
    /// client; on a scale-2 output the compositor upscales it, the ticket's
    /// second case.
    LogicalSize,
    /// A double-size `Argb8888` buffer shrunk to the configured size
    /// through `wp_viewporter` -- a downscale, which the fix must leave
    /// byte-alone.
    DownscaledBuffer,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Done,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A pixman session at scale 1.0, whatever `SCOOT_TEST_RENDERER` says:
    /// the subject here is pixman's repeat mode, which a GLES run would
    /// silently stop exercising.
    fn start() -> Self {
        let mut fixture = Harness::headless_on(
            Appearance {
                corner_radius: 0,
                ..Appearance::default()
            },
            CANVAS,
            RendererKind::Pixman,
        );
        fixture.spawn(run_client);
        fixture
    }

    /// The live arrangement's first (and here only) placement, in logical
    /// pixels.
    fn placement(&mut self) -> Rect {
        let placements: Vec<Rect> = {
            self.settle();
            self.state
                .world
                .arrange()
                .placements
                .iter()
                .map(|placement| placement.rect)
                .collect()
        };
        assert_eq!(placements.len(), 1, "these tests map exactly one window");
        placements[0]
    }
}

/// The client end of the one test connection: just enough of a toolkit to
/// map a solid-fill toplevel with or without a viewport.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// The newest `xdg_surface.configure` serial and `xdg_toplevel`
    /// configured size, of the one window this client ever maps.
    window_serial: Option<u32>,
    window_size: Option<(i32, i32)>,
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
        if interface == wl_compositor::WlCompositor::interface().name {
            // Version 4: `surface.attach`/`commit` are v1, but
            // `damage_buffer` needs v4.
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && width > 0
            && height > 0
        {
            client.window_size = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(TestClient: ignore wp_viewport::WpViewport);

/// A `w` x `h` solid-[`FILL_BGRA`] `wl_buffer` over a real memfd -- the same
/// path any toolkit takes.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    w: i32,
    h: i32,
    format: wl_shm::Format,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = w * 4;
    let len = (stride * h) as usize;
    let fd = rustix::fs::memfd_create("scoot-upscale-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    let bytes: Vec<u8> = FILL_BGRA.iter().copied().cycle().take(len).collect();
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, w, h, stride, format, qh, ());
    pool.destroy();
    Ok(buffer)
}

/// Map the one toplevel: role-only commit first, then -- once the
/// compositor's configure names a size -- ack it and commit pixels sized per
/// `step`. A viewport destination always names the configured size, without
/// which a buffer that is not exactly the surface size would draw at its own
/// size from the surface origin instead of scaling to it.
fn map(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    step: Step,
    keep: &mut Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
        wl_buffer::WlBuffer,
    )>,
) -> Result<(), String> {
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let surface = compositor.create_surface(qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, qh, ());
    let toplevel = xdg.get_toplevel(qh, ());
    toplevel.set_title("upscale".into());
    surface.commit();
    // A real toolkit sizes its buffer to the configure: wait for one
    // carrying a size, so the drawn window matches the placement the
    // assertions are derived from.
    let (w, h) = wait_for(queue, client, "a sized configure", |client| {
        client.window_size
    })?;
    let serial = wait_for(queue, client, "an xdg serial", |client| {
        client.window_serial
    })?;
    xdg.ack_configure(serial);
    // Buffer size and viewport per step, against the configured size.
    let (bw, bh, format, viewport) = match step {
        Step::ViewportedTiny => (1, 1, wl_shm::Format::Xrgb8888, true),
        Step::LogicalSize => (w, h, wl_shm::Format::Argb8888, false),
        Step::DownscaledBuffer => (2 * w, 2 * h, wl_shm::Format::Argb8888, true),
    };
    if viewport {
        let viewporter = client
            .viewporter
            .clone()
            .ok_or("no wp_viewporter -- the global is missing")?;
        let port = viewporter.get_viewport(&surface, qh, ());
        port.set_destination(w, h);
        // Forgotten, not dropped: releasing the viewport could release
        // surface state the compositor still reads, and there is only ever
        // one window, so the end of the client is the whole lifetime story.
        std::mem::forget(port);
    }
    let buffer = solid_buffer(&shm, qh, bw, bh, format)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, bw, bh);
    surface.commit();
    queue.roundtrip(client).map_err(|e| e.to_string())?;
    // Held so the mapped surface stays alive for the run.
    keep.push((surface, xdg, toplevel, buffer));
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
    // Held so every mapped surface stays alive for the run.
    let mut keep = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        map(&mut queue, &mut client, &qh, step, &mut keep)?;
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// The window's physical rect on the frame, derived from the live placement
/// -- never hard-coded -- through the same logical-to-physical conversion
/// the rounded clip uses.
fn physical_rect(fixture: &mut Fixture, scale: f64) -> Rectangle<i32, Physical> {
    let placement = fixture.placement();
    let rect = clip_rect(placement, scale);
    assert!(
        rect.size.w >= 8 && rect.size.h >= 8,
        "the test needs a window big enough that its centre clears the 1-px edge band, found {rect:?}"
    );
    rect
}

/// Corners, edge midpoints and centre of `rect` must all be exactly the
/// fill. The centre is the control: bilinear taps fully inside a solid
/// buffer are exact under any repeat mode, so a wrong centre means the
/// rect -- not the edge -- is what moved, and the test says so rather than
/// blaming the renderer.
fn assert_edges_exact(pixels: &[u8], rect: Rectangle<i32, Physical>, what: &str) {
    let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
    assert_pixel(
        pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        FILL_BGRA,
        &format!("{what}: centre must be fill (control)"),
    );
    for (px, py, name) in [
        (x, y, "top-left corner"),
        (x + w - 1, y, "top-right corner"),
        (x, y + h - 1, "bottom-left corner"),
        (x + w - 1, y + h - 1, "bottom-right corner"),
        (x + w / 2, y, "top edge midpoint"),
        (x + w / 2, y + h - 1, "bottom edge midpoint"),
        (x, y + h / 2, "left edge midpoint"),
        (x + w - 1, y + h / 2, "right edge midpoint"),
    ] {
        assert_pixel(
            pixels,
            CANVAS,
            px,
            py,
            FILL_BGRA,
            &format!("{what}: {name}"),
        );
    }
}

/// A 1x1 `Xrgb8888` buffer viewported to the window: every on-screen pixel
/// is an upscale, and under `Repeat::None` the edge taps read transparent
/// black -- corners fade hardest, edge midpoints halfway. Pinned exact.
#[test]
fn a_viewported_single_pixel_buffer_keeps_its_edge_pixels() {
    let mut fixture = Fixture::start();
    fixture.run(Step::ViewportedTiny);
    let rect = physical_rect(&mut fixture, 1.0);
    let pixels = fixture.render();
    assert_edges_exact(&pixels, rect, "1x1 XRGB viewported to the window");
}

/// A scale-unaware client on a scale-2 output: a logical-size buffer the
/// compositor upscales 2x. Same fade, same pin -- and renderer-agnostic, so
/// a `SCOOT_TEST_RENDERER=gles` run proves GLES never had it (clamp to
/// edge), while the default pixman run is the one that failed before the
/// fork fix.
#[test]
fn a_logical_size_buffer_on_a_scale_two_output_keeps_its_edge_pixels() {
    let mut fixture: Fixture = Harness::headless_scaled(
        Appearance {
            corner_radius: 0,
            ..Appearance::default()
        },
        CANVAS,
        2.0,
    );
    fixture.spawn(run_client);
    fixture.run(Step::LogicalSize);
    let rect = physical_rect(&mut fixture, 2.0);
    let pixels = fixture.render();
    assert_edges_exact(&pixels, rect, "logical-size buffer on a scale-2 output");
}

/// A double-size buffer shrunk to the window: a downscale, which reads its
/// taps from inside the texture either way. Exact before the fork fix and
/// after -- the pin is that the fix changed nothing here.
#[test]
fn a_downscaled_buffer_keeps_its_edge_pixels() {
    let mut fixture = Fixture::start();
    fixture.run(Step::DownscaledBuffer);
    let rect = physical_rect(&mut fixture, 1.0);
    let pixels = fixture.render();
    assert_edges_exact(&pixels, rect, "double-size buffer viewported down");
}
