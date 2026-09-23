//! Tests for client fullscreen, through a real `wayland-client` connection.
//!
//! The same approach `layer_shell/tests` takes, for the same reason: what is
//! under test is what a client is *told* (the configure's size and its
//! `fullscreen` state bit), what reaches the framebuffer (a window edge to
//! edge, a bar hidden, a notification kept), and where a click lands -- none
//! of which a test that calls the handler directly can see.
//!
//! The canvas is small and square, with the default layout gap and a
//! garish palette, so every pixel assertion can only pass because the thing
//! it names was drawn. Windows draw at whatever size they were configured
//! to, the way a real client does, so "the window covers the output" is a
//! claim about pixels, not about a rectangle in the core.
//!
//! Like every live-`State` suite here, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::{Rect, WindowId};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
#[cfg(feature = "gpu-scanout")]
use wayland_protocols::wp::alpha_modifier::v1::client::{
    wp_alpha_modifier_surface_v1, wp_alpha_modifier_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::{self, Harness, wait_for};

mod drawing;
mod neighbours;
#[cfg(feature = "gpu-scanout")]
mod primary_direct;
#[cfg(feature = "gpu-scanout")]
mod scanout_feedback;
mod transitions;

/// The framebuffer, square. Room for two half-width columns, a bar and a
/// notification, and small enough that coordinates stay readable.
const CANVAS: i32 = 200;
/// The whole output, which a covering fullscreen window must equal.
const OUTPUT: Rect = Rect::new(0, 0, CANVAS, CANVAS);
/// `Config::default`'s gap, which the harness's `State` lays out with.
const GAP: i32 = 12;
/// The bar [`Step::CreateLayer`] maps with [`Layer::Bar`]: across the top,
/// reserving its own height.
const BAR_HEIGHT: u32 = 20;
/// The notification [`Layer::Notification`] maps: a small overlay box in the
/// top-right corner, reserving nothing.
const NOTE_SIZE: u32 = 30;

// Colours, as the BGRA bytes an `Argb8888` pixman buffer holds them in, all
// distinct in every channel.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const OTHER_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];
const BAR_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const NOTE_BGRA: [u8; 4] = [0x20, 0x20, 0xE0, 0xFF];
const RING_BGRA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];
const BACKGROUND_BGRA: [u8; 4] = [0x56, 0x34, 0x12, 0xFF];

fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 3,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(1.0, 0.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// Which layer-shell surface [`Step::CreateLayer`] maps.
#[derive(Clone, Copy, Debug)]
enum Layer {
    /// A bar on the `top` layer, across the top edge, reserving its height.
    Bar,
    /// A notification on the `overlay` layer, top-right, reserving nothing.
    Notification,
    /// A launcher asking for every keystroke (`exclusive`), bottom-left, on
    /// the given layer.
    Launcher(zwlr_layer_shell_v1::Layer),
    /// A dock down the left edge, on the `top` layer, reserving its width
    /// ([`DOCK_WIDTH`]) -- a left exclusive zone.
    Dock,
    /// A wallpaper on the `background` layer, the whole output, reserving
    /// nothing: opaque (an `Xrgb8888` buffer) or not (`Argb8888`, no opaque
    /// region). Under a covering fullscreen window it stays in the frame's
    /// element list, which is what `render::primary_direct`'s rule 6 has to
    /// see past.
    #[cfg(feature = "gpu-scanout")]
    Wallpaper { opaque: bool },
    /// The same wallpaper as a `wp_single_pixel_buffer_manager_v1` buffer
    /// scaled over the output with `wp_viewporter`: opaque, black or not.
    /// Smithay's walk drops such a covering buffer and clears the frame to
    /// its colour instead.
    #[cfg(feature = "gpu-scanout")]
    PixelWallpaper { black: bool },
}

/// The width [`Layer::Dock`] reserves at the left edge.
const DOCK_WIDTH: u32 = 20;

/// One configure a toplevel was sent, as the client saw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Configured {
    serial: u32,
    width: i32,
    height: i32,
    fullscreen: bool,
}

enum Step {
    /// Create a toplevel, commit without a buffer, then ack the configure
    /// that answers and draw `color` at the size it names. With
    /// `fullscreen_first`, `set_fullscreen` is sent before that first
    /// commit -- what `foot --fullscreen` does.
    MapWindow {
        color: [u8; 4],
        fullscreen_first: bool,
    },
    /// `xdg_toplevel.set_fullscreen` on the `window`-th toplevel, naming
    /// the `output`-th `wl_output` in registry order (or none), then wait
    /// for the configure that answers. Does not ack it.
    SetFullscreen {
        window: usize,
        output: Option<usize>,
    },
    /// `xdg_toplevel.unset_fullscreen`, then wait for the answer.
    UnsetFullscreen { window: usize },
    /// Ack the newest configure and draw at the size it names.
    Draw { window: usize, color: [u8; 4] },
    /// Ack the newest configure, then draw a `width`x`height` buffer
    /// anyway -- a client that will not go as small as it was asked.
    DrawSized {
        window: usize,
        width: i32,
        height: i32,
    },
    /// Draw a `width`x`height` buffer and commit it *without* acking the
    /// newest configure -- a client still finishing a frame at its old size
    /// when a new configure arrives.
    DrawUnacked {
        window: usize,
        width: i32,
        height: i32,
    },
    /// Attach a null buffer and commit: the protocol's unmap.
    Unmap { window: usize },
    /// Map an unmapped toplevel again: commit without a buffer, then ack
    /// the newest configure and draw. Answers with the configure it used.
    Remap { window: usize, color: [u8; 4] },
    /// Report every configure the `window`-th toplevel has been sent.
    Configures { window: usize },
    /// Map a layer-shell surface.
    CreateLayer(Layer),
    /// Report which of this client's surfaces the pointer last entered.
    ReportPointer,
    /// `ext_session_lock_manager_v1.lock`, held and never released -- an
    /// abandoned lock stays locked, which is all these tests need.
    LockSession,
    /// `wp_alpha_modifier_surface_v1.set_multiplier` on the `window`-th
    /// toplevel's surface, then a commit with no new buffer (the factor is
    /// double-buffered surface state).
    #[cfg(feature = "gpu-scanout")]
    SetAlpha { window: usize, multiplier: u32 },
    /// Ack the newest configure and draw at its size in `Xrgb8888`, with no
    /// opaque region: an opaque-format buffer, the commonest real covering
    /// window (Mesa's default EGL config, mpv, games).
    #[cfg(feature = "gpu-scanout")]
    DrawXrgb { window: usize },
    /// Declare the `window`-th toplevel's surface opaque as a whole
    /// (`wl_surface.set_opaque_region` with a region larger than any size it
    /// will be given; Smithay clips it to the buffer), then commit. Smithay
    /// applies an opaque region only when the buffer or its view next
    /// changes, so a window that is already drawn needs a draw after this. What a
    /// video player or game with an alpha-format buffer does to be scanned
    /// out over a non-black background.
    #[cfg(feature = "gpu-scanout")]
    SetOpaque { window: usize },
    /// The same, as `stripes` vertical rectangles that together cover the
    /// output and none of which covers it alone -- the shape that takes
    /// the rectangle subtraction in `render::primary_direct`'s rule 6.
    #[cfg(feature = "gpu-scanout")]
    SetOpaqueStripes { window: usize, stripes: i32 },
    /// `zwp_linux_dmabuf_v1.get_surface_feedback` for the `window`-th
    /// toplevel's surface: what a v4+ Mesa client does for every EGL window.
    #[cfg(feature = "gpu-scanout")]
    SurfaceFeedback { window: usize },
    /// Report every complete feedback (`done`-terminated) the `window`-th
    /// toplevel's surface feedback object has received.
    #[cfg(feature = "gpu-scanout")]
    Feedbacks { window: usize },
}

enum Ack {
    Done,
    Configured(Configured),
    Configures(Vec<Configured>),
    Pointer(Option<Entered>),
    #[cfg(feature = "gpu-scanout")]
    Feedbacks(Vec<scanout_feedback::SeenFeedback>),
}

/// A surface the pointer entered, by the order the script created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entered {
    Window(usize),
    Layer(usize),
    Other,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_focus: Option<wl_surface::WlSurface>,
    outputs: Vec<wl_output::WlOutput>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    #[cfg(feature = "gpu-scanout")]
    alpha_modifier: Option<wp_alpha_modifier_v1::WpAlphaModifierV1>,
    #[cfg(feature = "gpu-scanout")]
    dmabuf: Option<
        wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    >,
    /// Per toplevel: the surface feedback being received, and every
    /// complete one.
    #[cfg(feature = "gpu-scanout")]
    feedback: scanout_feedback::Feedbacks,
    #[cfg(feature = "gpu-scanout")]
    single_pixel: Option<
        wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1,
    >,
    #[cfg(feature = "gpu-scanout")]
    viewporter: Option<wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter>,
    /// Per toplevel, by creation order: the `xdg_toplevel.configure` state
    /// waiting for its `xdg_surface.configure`, and every completed one.
    pending: Vec<Configured>,
    configures: Vec<Vec<Configured>>,
    /// Per toplevel: the serial it acked last.
    acked: Vec<Option<u32>>,
    /// Per layer surface: the newest configure's `(serial, width, height)`.
    layer_configures: Vec<Option<(u32, u32, u32)>>,
}

/// A surface's index in creation order.
struct Index(usize);

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
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "wl_output" => client
                .outputs
                .push(registry.bind(name, version.min(4), qh, ())),
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            #[cfg(feature = "gpu-scanout")]
            "wp_alpha_modifier_v1" => {
                client.alpha_modifier = Some(registry.bind(name, version.min(1), qh, ()));
            }
            // v5, the version Mesa's EGL and quickshell bind (see
            // `dmabuf.rs`): feedback objects, `main_device` still sent.
            #[cfg(feature = "gpu-scanout")]
            "wp_single_pixel_buffer_manager_v1" => {
                client.single_pixel = Some(registry.bind(name, version.min(1), qh, ()));
            }
            #[cfg(feature = "gpu-scanout")]
            "wp_viewporter" => {
                client.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
            }
            #[cfg(feature = "gpu-scanout")]
            "zwp_linux_dmabuf_v1" => {
                client.dmabuf = Some(registry.bind(name, version.min(5), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TestClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && client.pointer.is_none()
        {
            client.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => client.pointer_focus = Some(surface),
            wl_pointer::Event::Leave { .. } => client.pointer_focus = None,
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

impl Dispatch<xdg_toplevel::XdgToplevel, Index> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure {
            width,
            height,
            states,
        } = event
            && let Some(pending) = client.pending.get_mut(index.0)
        {
            let fullscreen = states
                .chunks_exact(4)
                .filter_map(|bytes| bytes.try_into().ok().map(u32::from_ne_bytes))
                .any(|state| state == xdg_toplevel::State::Fullscreen as u32);
            *pending = Configured {
                serial: 0,
                width,
                height,
                fullscreen,
            };
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, Index> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(pending) = client.pending.get(index.0).copied()
            && let Some(seen) = client.configures.get_mut(index.0)
        {
            seen.push(Configured { serial, ..pending });
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, Index> for TestClient {
    fn event(
        client: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            if let Some(slot) = client.layer_configures.get_mut(index.0) {
                *slot = Some((serial, width, height));
            }
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(TestClient: ignore wayland_client::protocol::wl_region::WlRegion);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(
    TestClient: ignore wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1
);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(
    TestClient: ignore wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter
);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(
    TestClient: ignore wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport
);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(TestClient: ignore wp_alpha_modifier_v1::WpAlphaModifierV1);
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(
    TestClient: ignore wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1
);

/// A `width`x`height` buffer of `color` over a real memfd. A zero size (a
/// configure that left the size to the client) draws 40 square.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> (wl_buffer::WlBuffer, i32, i32) {
    solid_buffer_in(shm, qh, width, height, color, wl_shm::Format::Argb8888)
}

/// [`solid_buffer`] in `format`.
fn solid_buffer_in(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
    format: wl_shm::Format,
) -> (wl_buffer::WlBuffer, i32, i32) {
    let width = if width > 0 { width } else { 40 };
    let height = if height > 0 { height } else { 40 };
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-fullscreen-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, format, qh, ());
    pool.destroy();
    (buffer, width, height)
}

/// One toplevel the script made.
struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

/// Waits for the `window`-th toplevel's configure count to pass `seen`, and
/// hands back the newest.
fn wait_for_configure(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    window: usize,
    seen: usize,
) -> Result<Configured, String> {
    wait_for(queue, client, "a toplevel configure", |client| {
        let all = client.configures.get(window)?;
        (all.len() > seen).then(|| all.last().copied()).flatten()
    })
}

/// Acks the newest configure and draws `color` at its size.
fn draw(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    window: &Toplevel,
    index: usize,
    color: [u8; 4],
) -> Result<Configured, String> {
    let newest = client
        .configures
        .get(index)
        .and_then(|all| all.last().copied())
        .ok_or("no configure to draw for")?;
    ack_newest(client, window, index, newest.serial);
    let (buffer, width, height) = solid_buffer(shm, qh, newest.width, newest.height, color);
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage(0, 0, width, height);
    window.surface.commit();
    Ok(newest)
}

/// Acks `serial` unless it is the one this toplevel acked last: acking the
/// same configure twice is a protocol error, and a redraw with no newer
/// configure in between has nothing new to ack.
fn ack_newest(client: &mut TestClient, window: &Toplevel, index: usize, serial: u32) {
    if client.acked.get(index).copied().flatten() != Some(serial) {
        window.xdg.ack_configure(serial);
        if let Some(slot) = client.acked.get_mut(index) {
            *slot = Some(serial);
        }
    }
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let layer_shell = client.layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?;

    let mut windows: Vec<Toplevel> = Vec::new();
    let mut layers: Vec<(
        wl_surface::WlSurface,
        zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    )> = Vec::new();
    let mut locks: Vec<ext_session_lock_v1::ExtSessionLockV1> = Vec::new();
    // Per toplevel, the alpha-modifier object once one was asked for: a
    // second `get_surface` for the same surface is a protocol error.
    #[cfg(feature = "gpu-scanout")]
    let mut alphas: Vec<Option<wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1>> =
        Vec::new();
    // Buffers stay referenced until the compositor is done with them; a
    // test this short simply keeps them all.
    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::MapWindow {
                color,
                fullscreen_first,
            } => {
                let index = windows.len();
                client.pending.push(Configured::default());
                client.configures.push(Vec::new());
                client.acked.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Index(index));
                let toplevel = xdg.get_toplevel(&qh, Index(index));
                if fullscreen_first {
                    toplevel.set_fullscreen(None);
                }
                surface.commit();
                wait_for_configure(&mut queue, &mut client, index, 0)?;
                let window = Toplevel {
                    surface,
                    xdg,
                    toplevel,
                };
                draw(&mut client, &qh, &shm, &window, index, color)?;
                windows.push(window);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::SetFullscreen { window, output } => {
                let seen = client.configures[window].len();
                let output = output.map(|i| client.outputs[i].clone());
                windows[window].toplevel.set_fullscreen(output.as_ref());
                let configured = wait_for_configure(&mut queue, &mut client, window, seen)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Configured(
                    client.configures[window]
                        .last()
                        .copied()
                        .unwrap_or(configured),
                )
            }
            Step::UnsetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.unset_fullscreen();
                let configured = wait_for_configure(&mut queue, &mut client, window, seen)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Configured(
                    client.configures[window]
                        .last()
                        .copied()
                        .unwrap_or(configured),
                )
            }
            Step::Draw { window, color } => {
                draw(&mut client, &qh, &shm, &windows[window], window, color)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DrawSized {
                window,
                width,
                height,
            } => {
                let newest = client.configures[window]
                    .last()
                    .copied()
                    .ok_or("no configure to ack")?;
                ack_newest(&mut client, &windows[window], window, newest.serial);
                let (buffer, width, height) = solid_buffer(&shm, &qh, width, height, WINDOW_BGRA);
                let surface = &windows[window].surface;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DrawUnacked {
                window,
                width,
                height,
            } => {
                let (buffer, width, height) = solid_buffer(&shm, &qh, width, height, WINDOW_BGRA);
                let surface = &windows[window].surface;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Unmap { window } => {
                let surface = &windows[window].surface;
                surface.attach(None, 0, 0);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Remap { window, color } => {
                let seen = client.configures[window].len();
                windows[window].surface.commit();
                // The compositor may already have answered the unmap itself
                // with the configure this re-map needs; either way, the
                // newest one is what a client acks.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                if client.configures[window].len() == seen && seen == 0 {
                    wait_for_configure(&mut queue, &mut client, window, seen)?;
                }
                let used = draw(&mut client, &qh, &shm, &windows[window], window, color)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Configured(used)
            }
            Step::Configures { window } => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Configures(client.configures[window].clone())
            }
            Step::CreateLayer(kind) => {
                let index = layers.len();
                client.layer_configures.push(None);
                let surface = compositor.create_surface(&qh, ());
                let (layer, anchor, size, zone) = match kind {
                    Layer::Bar => (
                        zwlr_layer_shell_v1::Layer::Top,
                        zwlr_layer_surface_v1::Anchor::Top
                            | zwlr_layer_surface_v1::Anchor::Left
                            | zwlr_layer_surface_v1::Anchor::Right,
                        (0, BAR_HEIGHT),
                        BAR_HEIGHT as i32,
                    ),
                    Layer::Notification => (
                        zwlr_layer_shell_v1::Layer::Overlay,
                        zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Right,
                        (NOTE_SIZE, NOTE_SIZE),
                        0,
                    ),
                    Layer::Dock => (
                        zwlr_layer_shell_v1::Layer::Top,
                        zwlr_layer_surface_v1::Anchor::Top
                            | zwlr_layer_surface_v1::Anchor::Bottom
                            | zwlr_layer_surface_v1::Anchor::Left,
                        (DOCK_WIDTH, 0),
                        DOCK_WIDTH as i32,
                    ),
                    Layer::Launcher(layer) => (
                        layer,
                        zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Left,
                        (NOTE_SIZE, NOTE_SIZE),
                        0,
                    ),
                    #[cfg(feature = "gpu-scanout")]
                    Layer::Wallpaper { .. } | Layer::PixelWallpaper { .. } => (
                        zwlr_layer_shell_v1::Layer::Background,
                        zwlr_layer_surface_v1::Anchor::all(),
                        (0, 0),
                        -1,
                    ),
                };
                let role = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    layer,
                    "fullscreen-test".into(),
                    &qh,
                    Index(index),
                );
                role.set_anchor(anchor);
                role.set_size(size.0, size.1);
                role.set_exclusive_zone(zone);
                if let Layer::Launcher(_) = kind {
                    role.set_keyboard_interactivity(
                        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
                    );
                }
                surface.commit();
                let (_, width, height) =
                    wait_for(&mut queue, &mut client, "a layer configure", |client| {
                        client.layer_configures[index]
                    })?;
                #[cfg(feature = "gpu-scanout")]
                if let Layer::PixelWallpaper { black } = kind {
                    let pixels = client
                        .single_pixel
                        .clone()
                        .ok_or("no wp_single_pixel_buffer_manager_v1")?;
                    let viewporter = client.viewporter.clone().ok_or("no wp_viewporter")?;
                    let channel = if black { 0 } else { u32::MAX / 2 };
                    let buffer =
                        pixels.create_u32_rgba_buffer(channel, channel, channel, u32::MAX, &qh, ());
                    let viewport = viewporter.get_viewport(&surface, &qh, ());
                    viewport.set_destination(width as i32, height as i32);
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage(0, 0, width as i32, height as i32);
                    surface.commit();
                    layers.push((surface, role));
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    acks.send(Ack::Done).map_err(|e| e.to_string())?;
                    continue;
                }
                let color = match kind {
                    Layer::Bar | Layer::Dock => BAR_BGRA,
                    #[cfg(feature = "gpu-scanout")]
                    Layer::Wallpaper { .. } | Layer::PixelWallpaper { .. } => OTHER_BGRA,
                    Layer::Notification | Layer::Launcher(_) => NOTE_BGRA,
                };
                let format = match kind {
                    #[cfg(feature = "gpu-scanout")]
                    Layer::Wallpaper { opaque: true } => wl_shm::Format::Xrgb8888,
                    _ => wl_shm::Format::Argb8888,
                };
                let (buffer, width, height) =
                    solid_buffer_in(&shm, &qh, width as i32, height as i32, color, format);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                layers.push((surface, role));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::ReportPointer => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let entered = client.pointer_focus.as_ref().map(|focus| {
                    if let Some(i) = windows.iter().position(|w| &w.surface == focus) {
                        Entered::Window(i)
                    } else if let Some(i) = layers.iter().position(|(s, _)| s == focus) {
                        Entered::Layer(i)
                    } else {
                        Entered::Other
                    }
                });
                Ack::Pointer(entered)
            }
            Step::LockSession => {
                let manager = client
                    .lock_manager
                    .as_ref()
                    .ok_or("no ext_session_lock_manager_v1")?;
                locks.push(manager.lock(&qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            #[cfg(feature = "gpu-scanout")]
            Step::DrawXrgb { window } => {
                let newest = client.configures[window]
                    .last()
                    .copied()
                    .ok_or("no configure to ack")?;
                ack_newest(&mut client, &windows[window], window, newest.serial);
                let (buffer, width, height) = solid_buffer_in(
                    &shm,
                    &qh,
                    newest.width,
                    newest.height,
                    WINDOW_BGRA,
                    wl_shm::Format::Xrgb8888,
                );
                let surface = &windows[window].surface;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            #[cfg(feature = "gpu-scanout")]
            Step::SetOpaqueStripes { window, stripes } => {
                let surface = &windows[window].surface;
                let region = compositor.create_region(&qh, ());
                let width = CANVAS / stripes.max(1);
                for stripe in 0..stripes {
                    let end = if stripe == stripes - 1 {
                        1 << 16
                    } else {
                        width
                    };
                    region.add(stripe * width, 0, end, 1 << 16);
                }
                surface.set_opaque_region(Some(&region));
                region.destroy();
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            #[cfg(feature = "gpu-scanout")]
            Step::SetOpaque { window } => {
                let surface = &windows[window].surface;
                let region = compositor.create_region(&qh, ());
                region.add(0, 0, 1 << 16, 1 << 16);
                surface.set_opaque_region(Some(&region));
                region.destroy();
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            #[cfg(feature = "gpu-scanout")]
            Step::SurfaceFeedback { window } => {
                let dmabuf = client.dmabuf.clone().ok_or("no zwp_linux_dmabuf_v1")?;
                client.feedback.expect(window);
                dmabuf.get_surface_feedback(&windows[window].surface, &qh, Index(window));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            #[cfg(feature = "gpu-scanout")]
            Step::Feedbacks { window } => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Feedbacks(client.feedback.complete(window))
            }
            #[cfg(feature = "gpu-scanout")]
            Step::SetAlpha { window, multiplier } => {
                if alphas.len() <= window {
                    alphas.resize(window + 1, None);
                }
                let surface = &windows[window].surface;
                if alphas[window].is_none() {
                    let manager = client
                        .alpha_modifier
                        .as_ref()
                        .ok_or("no wp_alpha_modifier_v1")?;
                    alphas[window] = Some(manager.get_surface(surface, &qh, ()));
                }
                if let Some(modifier) = &alphas[window] {
                    modifier.set_multiplier(multiplier);
                }
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        Self::with_appearance(appearance())
    }

    fn with_appearance(appearance: Appearance) -> Self {
        let mut fixture = Harness::headless(appearance, CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    fn map(&mut self, color: [u8; 4]) {
        self.map_with(color, false);
    }

    fn map_with(&mut self, color: [u8; 4], fullscreen_first: bool) {
        let ack = self.run(Step::MapWindow {
            color,
            fullscreen_first,
        });
        assert!(matches!(ack, Ack::Done));
    }

    fn configured(&mut self, step: Step) -> Configured {
        match self.run(step) {
            Ack::Configured(configured) => configured,
            _ => panic!("expected a configure"),
        }
    }

    fn configures(&mut self, window: usize) -> Vec<Configured> {
        match self.run(Step::Configures { window }) {
            Ack::Configures(all) => all,
            _ => panic!("expected configures"),
        }
    }

    fn done(&mut self, step: Step) {
        assert!(matches!(self.run(step), Ack::Done));
    }

    fn pointer(&mut self) -> Option<Entered> {
        match self.run(Step::ReportPointer) {
            Ack::Pointer(entered) => entered,
            _ => panic!("expected a pointer report"),
        }
    }

    /// The core id of the `index`-th window this client mapped (ids only
    /// increment, and this suite has one client).
    fn id(&self, index: usize) -> WindowId {
        let mut ids: Vec<WindowId> = self.state.windows.keys().copied().collect();
        ids.sort();
        ids[index]
    }

    fn rect_of(&self, index: usize) -> Rect {
        self.state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
            .rect
    }

    /// The surface holding the seat's keyboard, compositor-side.
    fn keyboard_focus(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        self.state.seat.get_keyboard()?.current_focus()
    }

    /// The `index`-th window's `wl_surface`, compositor-side.
    fn window_surface(
        &self,
        index: usize,
    ) -> smithay::reexports::wayland_server::protocol::wl_surface::WlSurface {
        self.state
            .windows
            .get(&self.id(index))
            .and_then(smithay::desktop::Window::toplevel)
            .expect("a toplevel")
            .wl_surface()
            .clone()
    }

    fn click(&mut self, x: f64, y: f64) {
        self.state.pointer_move(x, y);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, false);
        self.settle();
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}
