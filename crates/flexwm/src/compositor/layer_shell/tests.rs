//! Tests for layer-shell support.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `zwlr_layer_shell_v1` and `xdg_wm_base` exactly as `waybar` or `swaybg`
//! does -- through a real [`State`] with a real `headless` backend, then
//! render with the real [`PixmanRenderer`] and read the framebuffer back.
//! That is deliberate, and the same choice `cursor/tests.rs` made for the
//! same reason: what is under test is *where things end up on screen* and
//! *what the layout does about it*, and both are invisible to a test that
//! asserts on enum variants or calls the handler directly. The ordering bug
//! this feature could most easily have shipped -- a wallpaper drawn on top of
//! the focus ring -- looks completely correct at the type level.
//!
//! The pixel checks use a deliberately garish [`Appearance`] (see
//! [`appearance`]) so no assertion can pass by accident against a default
//! that happens to match, the same trick `scripts/smoke-test.sh` uses.
//!
//! Like `dispatch/tests.rs` and `cursor/tests.rs`, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (the client is inserted as a socket pair)
//! but which is created either way.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::{Config, Rect};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::Rectangle;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::State;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless::{self, Backend};
use crate::compositor::keybindings::Keybindings;
use crate::compositor::layer_shell::{ABOVE_WINDOWS, BELOW_WINDOWS};
use crate::compositor::state::ClientState;

/// The framebuffer these tests render into. Square and small: every
/// assertion below is a pixel coordinate, and a small canvas keeps them
/// readable while still leaving room for a bar, a window and its ring.
const CANVAS: i32 = 200;
/// The layout gap and focus-ring width [`Config::default`]/[`appearance`]
/// resolve to, spelled out because the expected geometry is written in terms
/// of them.
const GAP: i32 = 12;
const RING: i32 = 3;
/// The full usable area with nothing reserved: the whole canvas.
const WHOLE: Rect = Rect::new(0, 0, CANVAS, CANVAS);

// Colors, as the BGRA bytes a pixman `Argb8888` buffer holds them in. All
// four are distinct in every channel so a mix-up cannot read as a match.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const BAR_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const WALLPAPER_BGRA: [u8; 4] = [0x20, 0x20, 0xE0, 0xFF];
const RING_BGRA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];
const BACKGROUND_BGRA: [u8; 4] = [0x56, 0x34, 0x12, 0xFF];

/// A palette nothing else in this compositor defaults to, so a pixel
/// assertion can only pass because the thing it names was actually drawn.
fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: RING,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(1.0, 0.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// Everything a layer surface is created with, in one place -- the protocol
/// takes them as five separate requests before the first commit.
#[derive(Clone, Copy)]
struct LayerSpec {
    layer: zwlr_layer_shell_v1::Layer,
    anchor: zwlr_layer_surface_v1::Anchor,
    size: (u32, u32),
    exclusive_zone: i32,
    /// `top`, `right`, `bottom`, `left`, in the protocol's own order.
    margin: (i32, i32, i32, i32),
}

impl LayerSpec {
    /// A full-width bar across the top of the screen, `height` tall,
    /// reserving exactly its own height.
    fn bar(height: u32) -> Self {
        Self {
            layer: zwlr_layer_shell_v1::Layer::Top,
            anchor: zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            size: (0, height),
            exclusive_zone: height as i32,
            margin: (0, 0, 0, 0),
        }
    }

    /// A wallpaper: the whole output, on the bottom-most layer, explicitly
    /// asking not to be pushed around (`exclusive_zone = -1`, the protocol's
    /// `DontCare`) -- which is what `swaybg` itself does.
    fn wallpaper() -> Self {
        Self {
            layer: zwlr_layer_shell_v1::Layer::Background,
            anchor: zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            size: (0, 0),
            exclusive_zone: -1,
            margin: (0, 0, 0, 0),
        }
    }
}

/// One instruction for the client thread. The two halves have to interleave
/// -- a layer surface may not attach a buffer until it has acked the
/// configure the compositor sends in answer to its first commit -- so the
/// client runs as a real thread and the script is shipped a step at a time,
/// the same harness shape `cursor/tests.rs` established.
enum Step {
    /// Map an `xdg_toplevel` with a solid `WINDOW_BGRA` buffer of
    /// [`WINDOW_BUFFER`] square. Becomes one of flexwm's windows.
    MapWindow,
    /// Create a layer surface and commit it *without* a buffer, which is what
    /// earns it its initial configure.
    CreateLayer(LayerSpec),
    /// Create a layer surface and set everything up, but never commit -- so
    /// none of it has taken effect yet, since every one of those requests is
    /// double-buffered state.
    CreateLayerWithoutCommit(LayerSpec),
    /// Attach a solid buffer of the size the compositor configured to the
    /// layer surface created by the `index`-th [`Step::CreateLayer`], and
    /// commit -- i.e. actually map it.
    MapLayer { index: usize, color: [u8; 4] },
    /// `zwlr_layer_surface_v1.destroy` on the `index`-th layer surface.
    DestroyLayer { index: usize },
}

/// How big a window's buffer is. Deliberately smaller than any placement
/// these tests produce, so a window's own pixels start exactly at its
/// placement's top-left corner and the focus ring around that placement is
/// never covered by it.
const WINDOW_BUFFER: i32 = 40;

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// The size the compositor last configured each layer surface to, by
    /// creation order -- `None` until its first configure arrives.
    layer_sizes: Vec<Option<(u32, u32)>>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`,
    /// by creation order, `None` until its first one arrives.
    ///
    /// Per surface, not one shared slot: with two windows mapped, the
    /// compositor re-configures the *first* one when the second changes the
    /// layout, and a single slot let that serial be acked against the second
    /// surface -- which Smithay rightly answers with "must ack the initial
    /// configure before attaching buffer", killing the client at random.
    /// (Found by this harness failing intermittently, not by inspection.)
    window_serials: Vec<Option<u32>>,
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
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            _ => {}
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
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

/// A surface's own index in creation order, so a configure can be matched
/// back to the surface it is for.
struct SurfaceIndex(usize);

impl Dispatch<xdg_surface::XdgSurface, SurfaceIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &SurfaceIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(slot) = client.window_serials.get_mut(index.0)
        {
            *slot = Some(serial);
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, SurfaceIndex> for TestClient {
    fn event(
        client: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &SurfaceIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                // A layer surface must ack before it may attach anything,
                // and the size it is told is the size it is expected to draw.
                surface.ack_configure(serial);
                if let Some(slot) = client.layer_sizes.get_mut(index.0) {
                    *slot = Some((width, height));
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {}
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A `width`x`height` `wl_buffer` filled with `color`, over a real memfd --
/// the same path any toolkit takes.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> wl_buffer::WlBuffer {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("flexwm-layer-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Round-trips until `configure` reports that the compositor has configured
/// the surface in question.
///
/// One round trip is *not* enough and cannot be made enough: a configure is
/// sent when the compositor's layout says so, which may be a dispatch cycle
/// or two after the request that provoked it (a second window changes the
/// first one's size; a bar's exclusive zone re-lays-out everything).
fn wait_for_configure<T>(
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    configure: impl Fn(&TestClient) -> Option<T>,
) -> Result<T, String> {
    for _ in 0..50 {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        if let Some(value) = configure(client) {
            return Ok(value);
        }
    }
    Err("the compositor never configured a surface".into())
}

/// Runs the client half: binds the globals, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<()>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let layer_shell = client
        .layer_shell
        .clone()
        .ok_or("no zwlr_layer_shell_v1 -- the global is missing")?;

    let mut layers: Vec<(
        wl_surface::WlSurface,
        zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    )> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match &step {
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                let index = client.window_serials.len();
                client.window_serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceIndex(index));
                let _toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                // The compositor answers with a configure, which has to be
                // acked before a buffer may be attached -- and which may
                // take more than one round trip to arrive.
                let serial = wait_for_configure(&mut queue, &mut client, |client| {
                    client.window_serials[index]
                })?;
                xdg.ack_configure(serial);
                let buffer = solid_buffer(&shm, &qh, WINDOW_BUFFER, WINDOW_BUFFER, WINDOW_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, WINDOW_BUFFER, WINDOW_BUFFER);
                surface.commit();
                // Later configures (a re-layout after a bar appears) are
                // acked but not redrawn: these tests assert on where the
                // window's own pixels land, and a fixed buffer size is what
                // makes that a fixed number.
            }
            Step::CreateLayer(spec) | Step::CreateLayerWithoutCommit(spec) => {
                let spec = *spec;
                let surface = compositor.create_surface(&qh, ());
                let index = layers.len();
                client.layer_sizes.push(None);
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    spec.layer,
                    "flexwm-test".into(),
                    &qh,
                    SurfaceIndex(index),
                );
                layer.set_anchor(spec.anchor);
                layer.set_size(spec.size.0, spec.size.1);
                layer.set_exclusive_zone(spec.exclusive_zone);
                let (top, right, bottom, left) = spec.margin;
                layer.set_margin(top, right, bottom, left);
                // The initial commit: no buffer, which is what the protocol
                // requires before the first configure.
                if !matches!(step, Step::CreateLayerWithoutCommit(_)) {
                    surface.commit();
                }
                layers.push((surface, layer));
            }
            Step::MapLayer { index, color } => {
                let (index, color) = (*index, *color);
                let (surface, _) = layers.get(index).cloned().ok_or("no such layer surface")?;
                // A layer surface may only attach a buffer once it has acked
                // a configure, and it must draw at the size that configure
                // carried -- which may take more than one round trip to
                // arrive, and is the compositor's choice, not the spec's.
                let (width, height) = wait_for_configure(&mut queue, &mut client, |client| {
                    client.layer_sizes[index]
                })?;
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, color);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
            }
            Step::DestroyLayer { index } => {
                let (surface, layer) = layers.get(*index).ok_or("no such layer surface")?;
                layer.destroy();
                surface.destroy();
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<()>,
    client: Option<JoinHandle<Result<(), String>>>,
}

impl Fixture {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            appearance(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, step_rx, ack_tx));

        Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
        }
    }

    /// Runs one client step to completion, then lets the compositor settle so
    /// anything the step provoked (a configure, a re-layout) has happened
    /// before the test looks.
    fn run(&mut self, step: Step) {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
        self.settle();
    }

    /// Dispatches until `channel` produces a value.
    ///
    /// A client that died instead of answering is reported with *its own*
    /// error (the protocol error it provoked, usually), not as a timeout:
    /// the channel disconnecting is exactly that case, and a bare "timed
    /// out" there costs ten seconds and says nothing. The deadline is for
    /// the other case -- a compositor that stopped serving without anyone
    /// noticing.
    fn wait_for<T>(&mut self, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self
                        .client
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("the client stopped while waiting for {what}: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// Sends a step the client is expected *not* to survive -- a request the
    /// compositor answers with a protocol error -- and dispatches until the
    /// client thread has gone.
    ///
    /// The signal is the ack channel's sender dropping: [`run_client`]
    /// returns its error instead of acknowledging the step, which
    /// disconnects the receiver here.
    fn run_expecting_disconnect(&mut self, step: Step) {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.acks.try_recv() {
                Ok(()) => panic!("the client survived a request that should have been refused"),
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the client to be disconnected"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
        self.client.take();
        self.settle();
    }

    /// A few dispatch cycles with nothing outstanding, so in-flight protocol
    /// traffic in both directions has been processed.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// Disconnects the client and waits for the compositor to notice.
    fn disconnect_client(&mut self) {
        drop(self.steps.take());
        if let Some(handle) = self.client.take() {
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
    }

    /// Renders a frame and hands back its raw BGRA pixels.
    fn render(&mut self) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        let backend = self.state.backend.as_mut().expect("a backend");
        let Backend {
            renderer, image, ..
        } = backend;
        let framebuffer = renderer.bind(image).expect("a framebuffer");
        let region = Rectangle::from_size((CANVAS, CANVAS).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
            .expect("a framebuffer readback");
        renderer
            .map_texture(&mapping)
            .expect("mapped pixels")
            .to_vec()
    }

    /// The usable area the core currently arranges windows within.
    fn usable(&self) -> Rect {
        *self
            .state
            .world
            .usable_areas()
            .first()
            .expect("the one output")
    }

    /// Where the core says the first window goes.
    fn window_rect(&self) -> Rect {
        self.state
            .world
            .arrange()
            .placements
            .first()
            .expect("a placed window")
            .rect
    }
}

impl Drop for Fixture {
    /// Closes the step channel, which is what ends [`run_client`]'s loop.
    ///
    /// Deliberately does *not* join while unwinding: a compositor-side panic
    /// leaves the client thread blocked in a roundtrip whose answer will
    /// never come (the server end of its socket outlives this `Drop`, so it
    /// sees no EOF either), and joining there turns a failing test into a
    /// hung one -- which is exactly what happened while writing these, and
    /// cost the real bug `reject_unrepresentable_layer_size` now guards
    /// against a diagnosis. The thread is released when `state`, and with it
    /// the server end, drops a moment later.
    fn drop(&mut self) {
        drop(self.steps.take());
        if std::thread::panicking() {
            return;
        }
        if let Some(handle) = self.client.take() {
            let _ = handle.join();
        }
    }
}

/// Reads one pixel out of a [`CANVAS`]-square BGRA framebuffer.
fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    let index = ((y * CANVAS + x) * 4) as usize;
    pixels[index..index + 4].try_into().expect("four bytes")
}

fn assert_pixel(pixels: &[u8], x: i32, y: i32, expected: [u8; 4], what: &str) {
    assert_eq!(
        pixel(pixels, x, y),
        expected,
        "{what}: wrong pixel at ({x}, {y})"
    );
}

// -------------------------------------------------------------------------
// Nothing changes when nothing uses the protocol
// -------------------------------------------------------------------------

/// The whole feature is invisible to a session with no layer surfaces: the
/// usable area is the output, and the window and its ring draw exactly where
/// they did before any of this existed. The baseline every other test's
/// numbers are read against.
#[test]
fn a_session_with_no_layer_surfaces_is_unchanged() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.usable(), WHOLE);
    let rect = fixture.window_rect();
    assert_eq!((rect.x, rect.y), (GAP, GAP));

    let pixels = fixture.render();
    assert_pixel(&pixels, rect.x, rect.y, WINDOW_BGRA, "the window");
    assert_pixel(&pixels, rect.x - RING, rect.y, RING_BGRA, "its focus ring");
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        BACKGROUND_BGRA,
        "the background",
    );
}

// -------------------------------------------------------------------------
// The render stack
// -------------------------------------------------------------------------

/// A `top` layer surface covers a window, which is the whole point of the
/// layer: a bar is not something a maximized window is allowed to hide.
#[test]
fn a_top_layer_surface_draws_in_front_of_a_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // No exclusive zone, so the window keeps its place and the bar overlaps
    // it -- exactly the case where "who is in front" is observable.
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let rect = fixture.window_rect();
    assert_eq!(
        (rect.x, rect.y),
        (GAP, GAP),
        "the window must not have moved"
    );
    let pixels = fixture.render();
    // Inside the window's own rectangle, but in the bar's rows.
    assert_pixel(&pixels, rect.x + 5, 15, BAR_BGRA, "the bar over the window");
    // ...and below the bar, the window itself.
    assert_pixel(&pixels, rect.x + 5, 35, WINDOW_BGRA, "the window below it");
}

/// The one ordering Smithay's own `space_render_elements` cannot express, and
/// the reason `render()` gathers layer surfaces itself: a full-screen
/// wallpaper belongs *behind* the focus ring, not on top of it. With the
/// elements in that fixed order this test fails with the ring pixel reading
/// as wallpaper.
#[test]
fn a_background_layer_surface_draws_behind_windows_and_the_focus_ring() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::wallpaper()));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });

    let rect = fixture.window_rect();
    let pixels = fixture.render();
    assert_pixel(&pixels, rect.x, rect.y, WINDOW_BGRA, "the window");
    assert_pixel(
        &pixels,
        rect.x - RING,
        rect.y,
        RING_BGRA,
        "the focus ring over the wallpaper",
    );
    // Somewhere no window and no ring reaches: the wallpaper, not the
    // compositor's own background color.
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        WALLPAPER_BGRA,
        "the wallpaper",
    );
}

/// A wallpaper explicitly asking not to be pushed around (`exclusive_zone =
/// -1`) covers the whole output and reserves nothing -- both halves of
/// `swaybg`'s behavior.
#[test]
fn a_dont_care_exclusive_zone_reserves_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::wallpaper()));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(
        fixture.window_rect(),
        Rect::new(GAP, GAP, 94 - GAP, CANVAS - 2 * GAP)
    );
}

// -------------------------------------------------------------------------
// Exclusive zones
// -------------------------------------------------------------------------

/// The headline behavior: a bar that reserves its own height takes that
/// height away from where windows are arranged -- and the window's pixels
/// really move, not just its placement rectangle.
#[test]
fn an_exclusive_zone_moves_windows_out_of_the_way() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));
    let after = fixture.window_rect();
    assert_eq!(after.y, 30 + GAP, "the window starts below the bar");
    assert_eq!(after.x, before.x, "nothing reserved horizontal space");
    assert_eq!(
        after.h,
        before.h - 30,
        "the window lost exactly the bar's height"
    );

    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BAR_BGRA, "the bar");
    assert_pixel(&pixels, after.x, after.y, WINDOW_BGRA, "the moved window");
    assert_pixel(
        &pixels,
        after.x - RING,
        after.y,
        RING_BGRA,
        "the ring around the moved window",
    );
    // The row the window used to start on is now above it: bar or background,
    // never the window.
    assert_ne!(
        pixel(&pixels, after.x, before.y),
        WINDOW_BGRA,
        "the window should no longer reach its old position"
    );
}

/// Two bars on the same edge stack: the second is placed below the first and
/// the reserved strip is the sum, which is what `LayerMap` means by
/// arranging exclusive surfaces first and shrinking the zone as it goes.
#[test]
fn two_bars_on_the_same_edge_stack_their_exclusive_zones() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::CreateLayer(LayerSpec::bar(20)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: WALLPAPER_BGRA,
    });

    assert_eq!(fixture.usable(), Rect::new(0, 50, CANVAS, CANVAS - 50));
    assert_eq!(fixture.window_rect().y, 50 + GAP);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BAR_BGRA, "the first bar");
    assert_pixel(&pixels, CANVAS / 2, 40, WALLPAPER_BGRA, "the second bar");
}

/// A layer surface that asks for a zone but never commits has not said
/// anything yet: every one of those requests is double-buffered state, and
/// nothing may be reserved on the strength of a pending one.
#[test]
fn a_layer_surface_that_never_commits_reserves_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayerWithoutCommit(LayerSpec::bar(30)));

    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "nothing drawn");
}

/// **Deliberate, and worth pinning down**: a bar reserves its exclusive zone
/// from its *initial* commit -- the buffer-less one the protocol requires
/// before the first configure -- not from the commit that first gives it a
/// buffer. So there is a window, normally a frame or two long, where the
/// layout has made room for a bar that is not drawing yet.
///
/// That is Smithay's `LayerMap` behavior (it arranges every surface mapped
/// into it, buffer or not) and flexwm takes it as-is rather than
/// reimplementing `arrange` to filter on mapped-ness. It self-heals in both
/// directions that matter: a client that unmaps has its cached state reset
/// by Smithay's own pre-commit hook, and one that dies has its surface
/// unmapped by `layer_destroyed`. See `ROADMAP.md`'s backlog entry for the
/// case this does leave open -- a client that commits and then never attaches
/// anything holds the space for as long as it stays connected.
#[test]
fn a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));

    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));
    // ...and nothing is drawn there yet, which is the other half of the
    // statement: reserved, not painted.
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS / 2,
        15,
        BACKGROUND_BGRA,
        "nothing drawn yet",
    );
}

/// Destroying a bar gives its space back, immediately -- a launcher that
/// reserved space and quit must not leave a dead strip behind.
#[test]
fn destroying_a_bar_returns_its_space() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_ne!(fixture.window_rect(), before);

    fixture.run(Step::DestroyLayer { index: 0 });
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
    assert_pixel(
        &pixels,
        before.x,
        before.y,
        WINDOW_BGRA,
        "the window is back",
    );
}

/// ...and so does a client that simply dies with its bar mapped, which is
/// the case a compositor actually meets (a crashed `waybar`). The surface is
/// torn down implicitly, in an order nothing here controls.
#[test]
fn a_client_that_disconnects_returns_its_bars_space() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));

    fixture.disconnect_client();
    // The window went with the client too, so only the zone is assertable --
    // which is the thing that would otherwise be stuck forever.
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
}

// -------------------------------------------------------------------------
// Adversarial values
// -------------------------------------------------------------------------

/// Every number behind a layer surface's geometry arrives as a raw `i32`
/// off the wire. A client sending the extremes must not be able to overflow
/// the zone arithmetic, the layout or the renderer -- this is a debug build,
/// so an overflow anywhere along that path panics the test rather than
/// wrapping quietly.
#[test]
fn extreme_geometry_from_a_client_cannot_break_the_compositor() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        layer: zwlr_layer_shell_v1::Layer::Top,
        anchor: zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
        // The largest size that is representable at all -- see
        // `a_size_that_does_not_fit_i32_is_refused` for what happens past it.
        size: (i32::MAX as u32, i32::MAX as u32),
        exclusive_zone: i32::MAX,
        margin: (i32::MAX, i32::MAX, i32::MAX, i32::MAX),
    }));

    // Deliberately never given a buffer: an `i32::MAX`-square one cannot be
    // allocated by anyone. The geometry still reaches `arrange`, the zone it
    // leaves still reaches the core, and the frame is still rendered.
    let usable = fixture.usable();
    assert_eq!(
        usable,
        usable.intersection(WHOLE),
        "the usable area escaped the output"
    );
    let rect = fixture.window_rect();
    assert!(rect.w >= 1 && rect.h >= 1, "{rect:?}");
    let pixels = fixture.render();
    assert_eq!(pixels.len(), (CANVAS * CANVAS * 4) as usize);
    // Pointer hit-testing walks the same geometry, adding the layer's own
    // location to a surface-local point: with the location saturated near
    // `i32::MAX`, a plain `i32` add there would panic in this build.
    for (x, y) in [(0.0, 0.0), (1.0, 1.0), (99.0, 99.0), (199.0, 199.0)] {
        let _ = fixture.state.surface_under((x, y).into());
        let _ = fixture
            .state
            .layer_surface_under(&ABOVE_WINDOWS, (x, y).into());
    }
    // Still serving afterwards: another layer surface is created, arranged
    // and configured normally, and the zone stays inside the output. (It is
    // deliberately left without a buffer: the extreme surface above has
    // already reserved everything, so this one is configured to zero height
    // and there is no buffer to make for it.)
    fixture.run(Step::CreateLayer(LayerSpec::bar(10)));
    let usable = fixture.usable();
    assert_eq!(usable, usable.intersection(WHOLE));
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
}

/// `set_size` takes two `uint`s, and the pinned Smithay rev converts them
/// with a bare `as i32` into a `Size` whose constructor `debug_assert!`s on
/// negative dimensions -- so before `dispatch.rs`'s
/// `reject_unrepresentable_layer_size` guard, this exact request panicked the
/// whole compositor (found by running an earlier version of the test above,
/// which used `u32::MAX`). The offending client must be the only casualty.
#[test]
fn a_size_that_does_not_fit_i32_is_refused_without_taking_the_compositor_down() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();

    fixture.run_expecting_disconnect(Step::CreateLayer(LayerSpec {
        size: (u32::MAX, u32::MAX),
        ..LayerSpec::bar(30)
    }));

    // The compositor is still here, still laying out, still drawing. The
    // dead client's window went with it, so `before` is only used to prove
    // there *was* a working layout to lose.
    assert_ne!(before.w, 0);
    assert_eq!(fixture.usable(), WHOLE);
    assert!(fixture.state.world.arrange().placements.is_empty());
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "nothing drawn");
    assert_pixel(
        &pixels,
        CANVAS / 2,
        CANVAS / 2,
        BACKGROUND_BGRA,
        "the window is gone with its client",
    );
}

// -------------------------------------------------------------------------
// Input
// -------------------------------------------------------------------------

/// The pointer reaches a bar rather than the window behind it, and still
/// reaches the window everywhere else. Without this a bar draws but cannot
/// be clicked, which is most of what a bar is for.
#[test]
fn the_pointer_finds_a_bar_in_front_of_a_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let rect = fixture.window_rect();
    let window_surface = fixture
        .state
        .windows
        .values()
        .next()
        .and_then(|window| window.toplevel().map(|t| t.wl_surface().clone()))
        .expect("a mapped window");

    // Over the bar, in a column the window also occupies.
    let over_bar = ((rect.x + 5) as f64, 15.0).into();
    let (found, _) = fixture
        .state
        .surface_under(over_bar)
        .expect("something under the pointer at the bar");
    assert_ne!(found, window_surface, "the window swallowed a bar click");
    assert!(
        fixture
            .state
            .layer_surface_under(&ABOVE_WINDOWS, over_bar)
            .is_some(),
        "the bar should be found on the layers above windows"
    );
    assert!(
        fixture
            .state
            .layer_surface_under(&BELOW_WINDOWS, over_bar)
            .is_none(),
        "a top-layer bar must not answer for the layers below windows"
    );

    // Below the bar, the window itself still answers.
    let over_window = ((rect.x + 5) as f64, (rect.y + 25) as f64).into();
    let (found, _) = fixture
        .state
        .surface_under(over_window)
        .expect("something under the pointer at the window");
    assert_eq!(found, window_surface);
}

/// Clicking a bar must not activate the window behind it. The control half
/// -- clicking the window itself -- is what keeps this from passing
/// vacuously on a compositor that never focuses anything.
#[test]
fn clicking_a_bar_does_not_refocus_the_window_behind_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let placements = fixture.state.world.arrange().placements;
    let first = placements[0];
    let focused = fixture.state.focus;
    assert_ne!(
        focused,
        Some(first.id),
        "the second window should hold focus"
    );

    // A click over the bar, in the first window's own column.
    fixture.state.pointer_move((first.rect.x + 5) as f64, 15.0);
    fixture
        .state
        .pointer_button(flexwm_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(flexwm_ipc::PointerButton::Left, false);
    assert_eq!(
        fixture.state.focus, focused,
        "a click on the bar moved window focus"
    );

    // ...and the same click on the window below the bar does focus it.
    fixture
        .state
        .pointer_move((first.rect.x + 5) as f64, (first.rect.y + 25) as f64);
    fixture
        .state
        .pointer_button(flexwm_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(flexwm_ipc::PointerButton::Left, false);
    assert_eq!(
        fixture.state.focus,
        Some(first.id),
        "a click on the window should have focused it"
    );
}

// -------------------------------------------------------------------------
// Output changes
// -------------------------------------------------------------------------

/// Resizing the output re-anchors every layer surface against the new mode
/// and re-derives the zone from it, rather than leaving a bar sized for the
/// old screen or a reservation measured against it.
#[test]
fn resizing_the_output_re_arranges_bars_and_the_zone() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let smaller = CANVAS / 2;
    fixture.state.resize_output(smaller, smaller);
    fixture.settle();

    assert_eq!(
        fixture.usable(),
        Rect::new(0, 30, smaller, smaller - 30),
        "the zone should follow the new mode"
    );
    let rect = fixture.window_rect();
    assert_eq!(rect.y, 30 + GAP);
    assert!(
        rect.right() <= smaller,
        "the window should fit the new mode: {rect:?}"
    );
}
