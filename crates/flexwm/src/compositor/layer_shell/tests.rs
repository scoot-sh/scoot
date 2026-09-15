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
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity;
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
const POPUP_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];

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
    /// What the surface asks to do with the keyboard. The protocol's default
    /// is `None`, and so is every constructor's below -- a bar, a wallpaper
    /// and a notification popup all want exactly that.
    keyboard: KeyboardInteractivity,
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
            keyboard: KeyboardInteractivity::None,
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
            keyboard: KeyboardInteractivity::None,
        }
    }

    /// A launcher: a box in the top-left corner that reserves nothing and
    /// asks for every keystroke -- the shape `wofi`, `fuzzel` and a
    /// layer-shell lock screen all have.
    ///
    /// Anchored to the bottom-right corner rather than centred, because
    /// these tests click at specific coordinates: that corner is the one
    /// place on this canvas that never overlaps a window's own buffer (which
    /// is [`WINDOW_BUFFER`] square at the top-left), so "click the launcher"
    /// and "click the window" stay unambiguous.
    fn launcher(size: u32) -> Self {
        Self {
            layer: zwlr_layer_shell_v1::Layer::Overlay,
            anchor: zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Right,
            size: (size, size),
            exclusive_zone: 0,
            margin: (0, 0, 0, 0),
            keyboard: KeyboardInteractivity::Exclusive,
        }
    }

    fn with_keyboard(self, keyboard: KeyboardInteractivity) -> Self {
        Self { keyboard, ..self }
    }

    fn on_layer(self, layer: zwlr_layer_shell_v1::Layer) -> Self {
        Self { layer, ..self }
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
    /// `wl_surface.frame` on the `index`-th layer surface, followed by a
    /// commit so the callback moves from pending to current server-side.
    /// The client keeps the `wl_callback` proxy alive -- a careful client
    /// that only drops it when its own `done` arrives.
    RequestLayerFrame { index: usize },
    /// The DMS dismissal sequence on the `index`-th layer surface: destroy
    /// the layer role, attach a null buffer, commit -- and then keep the
    /// `wl_surface`, the frame-callback proxies and the whole connection
    /// alive. What a compositor sends afterwards is the entire question.
    DismissLayer { index: usize },
    /// Report how many `done` events each requested frame callback has seen.
    ReportFrames,
    /// Attach a null buffer to the `index`-th layer surface and commit: the
    /// protocol's own way for a surface to unmap itself without destroying
    /// the object, which a launcher does when it is dismissed and re-shown.
    UnmapLayer { index: usize },
    /// The other half of [`Step::UnmapLayer`]: show that same surface again.
    ///
    /// Not just "attach a buffer" -- Smithay's own `pre_commit_hook` resets a
    /// layer surface's cached state to `Default` when it unmaps
    /// (`got_unmapped` in `wlr_layer/mod.rs`), which is what the protocol
    /// asks for ("the surface returns to the state it had right after
    /// `get_layer_surface`"). So everything it was created with has to be
    /// said again, layer included, before it may be committed: a bare commit
    /// is answered with `invalid_size` rather than a configure, and a bare
    /// re-attach would come back on the `background` layer.
    RemapLayer { index: usize, color: [u8; 4] },
    /// `set_keyboard_interactivity` on an already-mapped layer surface,
    /// followed by a commit (the setting is double-buffered, so the commit
    /// is what makes it real).
    SetLayerKeyboard {
        index: usize,
        keyboard: KeyboardInteractivity,
    },
    /// Report what the client's own `wl_keyboard` has seen so far.
    ReportKeyboard,
    /// Create an `xdg_popup` on the first mapped toplevel and drive it
    /// through configure, ack, attach and a frame request, reporting
    /// whether the compositor ever configured it. See
    /// [`an_xdg_popup_configures_maps_draws_and_tears_down`].
    MapPopup { color: [u8; 4] },
    /// Destroy the popup [`Step::MapPopup`] made: `xdg_popup.destroy` +
    /// `xdg_surface.destroy` + `wl_surface.destroy`.
    DestroyPopup,
    /// Report how many `xdg_surface.configure` events the mapped popup has
    /// received in total -- the compositor must send exactly one (later
    /// commits stay quiet, as a non-reactive positioner requires).
    ReportPopupConfigures,
}

/// What the client reports back once a step is done.
enum Ack {
    Done,
    /// [`Step::MapPopup`]'s answer: did an `xdg_surface.configure` arrive
    /// for the popup?
    PopupConfigured(bool),
    /// [`Step::ReportPopupConfigures`]'s answer: total configures for the
    /// mapped popup.
    PopupConfigures(u32),
    /// [`Step::ReportKeyboard`]'s answer.
    Keyboard(KeyboardReport),
    /// [`Step::ReportFrames`]'s answer: per requested callback, how many
    /// `done` events arrived.
    Frames(Vec<u32>),
}

/// Which of the client's own surfaces a `wl_keyboard.enter` named, by the
/// creation order the test script already addresses them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focused {
    Layer(usize),
    Window(usize),
    /// A surface this client made but the script doesn't track (nothing
    /// produces one today; it exists so a mismatch reads as a mismatch
    /// rather than as "no focus").
    Other,
}

/// What the client's `wl_keyboard` has actually been told -- the only
/// evidence that matters here, since "who holds keyboard focus" is a claim
/// about what reached a client, not about a field in the compositor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct KeyboardReport {
    /// Whose `enter` is outstanding, if any.
    focused: Option<Focused>,
    /// `wl_keyboard.key` events received, cumulative.
    keys: u32,
    /// `enter`/`leave` events received, cumulative -- so a test can tell
    /// "focus never moved" apart from "it left and came straight back".
    enters: u32,
    leaves: u32,
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
    seat: Option<wl_seat::WlSeat>,
    /// Created from the seat's `Capabilities` event, so the client never
    /// asks for a keyboard the compositor didn't advertise.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The surface the outstanding `wl_keyboard.enter` named. Stored as the
    /// raw `wl_surface` because this handler has no idea which of the
    /// script's surfaces it is; [`run_client`] resolves it at report time.
    keyboard_focus: Option<wl_surface::WlSurface>,
    keys: u32,
    enters: u32,
    leaves: u32,
    /// The size the compositor last configured each layer surface to, by
    /// creation order -- `None` until its first configure arrives.
    layer_sizes: Vec<Option<(u32, u32)>>,
    /// How many configures each of those has received, so a re-map can wait
    /// for a *new* one rather than for "a size", which it already has from
    /// before it unmapped. See [`Step::RemapLayer`].
    layer_configures: Vec<u32>,
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
    /// How many `xdg_surface.configure` events each toplevel or popup has
    /// received, by creation order -- so a test can tell "configured once"
    /// apart from "re-configured on every commit", which the protocol
    /// forbids for a non-reactive positioner.
    window_configures: Vec<u32>,
    /// How many `done` events each frame callback requested by
    /// [`Step::RequestLayerFrame`] has received, by request order.
    frame_dones: Vec<u32>,
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
                // 4, not 5: `on_demand` keyboard interactivity arrived in 4,
                // and nothing here needs 5's `set_exclusive_edge`.
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
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
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && client.keyboard.is_none()
        {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                client.keyboard_focus = Some(surface);
                client.enters += 1;
            }
            wl_keyboard::Event::Leave { .. } => {
                client.keyboard_focus = None;
                client.leaves += 1;
            }
            wl_keyboard::Event::Key { .. } => client.keys += 1,
            // Keymap (whose fd is simply dropped), modifiers and repeat info
            // all arrive too; none of them says anything about focus.
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
        if let xdg_surface::Event::Configure { serial } = event {
            if let Some(slot) = client.window_serials.get_mut(index.0) {
                *slot = Some(serial);
            }
            if let Some(seen) = client.window_configures.get_mut(index.0) {
                *seen = seen.saturating_add(1);
            }
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
                if let Some(seen) = client.layer_configures.get_mut(index.0) {
                    *seen = seen.saturating_add(1);
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {}
            _ => {}
        }
    }
}

/// Which requested frame callback a `done` belongs to, by request order.
struct FrameTag(usize);

impl Dispatch<wl_callback::WlCallback, FrameTag> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        tag: &FrameTag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(slot) = client.frame_dones.get_mut(tag.0)
        {
            *slot = slot.saturating_add(1);
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
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore xdg_popup::XdgPopup);
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

/// Sends everything a [`LayerSpec`] describes, none of which takes effect
/// until the next commit.
///
/// Shared by [`Step::CreateLayer`] and [`Step::RemapLayer`] rather than
/// written out twice, because they have to say exactly the same things: the
/// protocol puts an unmapped surface back in its just-created state, so a
/// re-map is a second first-commit. `set_layer` is in here for that reason
/// too, even though `get_layer_surface` already carried it -- the reset
/// takes the layer back to `background` with everything else.
fn describe_layer(layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, spec: LayerSpec) {
    layer.set_layer(spec.layer);
    layer.set_anchor(spec.anchor);
    layer.set_size(spec.size.0, spec.size.1);
    layer.set_exclusive_zone(spec.exclusive_zone);
    layer.set_keyboard_interactivity(spec.keyboard);
    let (top, right, bottom, left) = spec.margin;
    layer.set_margin(top, right, bottom, left);
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
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
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

    // The spec is kept beside each surface because a re-map has to say all
    // of it again (see [`Step::RemapLayer`]).
    let mut layers: Vec<(
        wl_surface::WlSurface,
        zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        LayerSpec,
    )> = Vec::new();
    // Kept so a popup can name its parent; a popup's parent is an
    // `xdg_surface`, not a `wl_surface`.
    let mut toplevels: Vec<xdg_surface::XdgSurface> = Vec::new();
    // ...and the `wl_surface`s under them, which is what a
    // `wl_keyboard.enter` names.
    let mut windows: Vec<wl_surface::WlSurface> = Vec::new();
    // Popups [`Step::MapPopup`] left mapped: surface, `xdg_surface`,
    // `xdg_popup` and serial-slot index, so [`Step::DestroyPopup`] can tear
    // them down in order and [`Step::ReportPopupConfigures`] can count
    // their configures.
    let mut popups: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_popup::XdgPopup,
        usize,
    )> = Vec::new();
    // Requested frame callbacks, kept alive so a `done` that arrives late
    // lands on a live proxy and is counted rather than killing the
    // connection outright.
    let mut frames: Vec<wl_callback::WlCallback> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match &step {
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                let index = client.window_serials.len();
                client.window_serials.push(None);
                client.window_configures.push(0);
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
                toplevels.push(xdg.clone());
                windows.push(surface);
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
                client.layer_configures.push(0);
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    spec.layer,
                    "flexwm-test".into(),
                    &qh,
                    SurfaceIndex(index),
                );
                describe_layer(&layer, spec);
                // The initial commit: no buffer, which is what the protocol
                // requires before the first configure.
                if !matches!(step, Step::CreateLayerWithoutCommit(_)) {
                    surface.commit();
                }
                layers.push((surface, layer, spec));
            }
            Step::MapLayer { index, color } => {
                let (index, color) = (*index, *color);
                let (surface, ..) = layers.get(index).cloned().ok_or("no such layer surface")?;
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
                let (surface, layer, _) = layers.get(*index).ok_or("no such layer surface")?;
                layer.destroy();
                surface.destroy();
            }
            Step::RequestLayerFrame { index } => {
                let (surface, ..) = layers.get(*index).ok_or("no such layer surface")?;
                let tag = FrameTag(client.frame_dones.len());
                client.frame_dones.push(0);
                // Kept alive in `frames`: dropping the proxy is what turns
                // a late `done` into a dead connection, and this client is
                // deliberately the careful kind -- the compositor must not
                // send one late in the first place.
                let callback = surface.frame(&qh, tag);
                surface.commit();
                frames.push(callback);
            }
            Step::DismissLayer { index } => {
                let (surface, layer, _) = layers.get(*index).ok_or("no such layer surface")?;
                layer.destroy();
                surface.attach(None, 0, 0);
                surface.commit();
            }
            Step::ReportFrames => {
                outcome = Ack::Frames(client.frame_dones.clone());
            }
            Step::UnmapLayer { index } => {
                let (surface, ..) = layers.get(*index).ok_or("no such layer surface")?;
                surface.attach(None, 0, 0);
                surface.commit();
            }
            Step::RemapLayer { index, color } => {
                let (index, color) = (*index, *color);
                let (surface, layer, spec) =
                    layers.get(index).cloned().ok_or("no such layer surface")?;
                describe_layer(&layer, spec);
                // Unmapping put the surface back in the initial-configure
                // stage, so this buffer-less commit earns a fresh configure
                // exactly like the first one did -- and it must be waited
                // for by *count*, not by "a size has arrived". The unmap's
                // own commit already provoked one (Smithay clears
                // `initial_configure_sent` when it resets the role), carrying
                // whatever `arrange` made of the reset, anchorless state:
                // measured at 2 configures and 100x100 for a 60x60 launcher
                // on this 200-square canvas, i.e. waiting for a size would
                // return the stale one instantly and draw at the wrong size.
                let seen = client.layer_configures.get(index).copied().unwrap_or(0);
                surface.commit();
                let (width, height) = wait_for_configure(&mut queue, &mut client, |client| {
                    let fresh = client.layer_configures.get(index).copied().unwrap_or(0) > seen;
                    fresh
                        .then(|| client.layer_sizes.get(index).copied().flatten())
                        .flatten()
                })?;
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, color);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
            }
            Step::SetLayerKeyboard { index, keyboard } => {
                let (surface, layer, _) = layers.get(*index).ok_or("no such layer surface")?;
                layer.set_keyboard_interactivity(*keyboard);
                // Double-buffered like everything else the surface asks for:
                // without this commit the compositor has heard nothing.
                surface.commit();
            }
            Step::ReportKeyboard => {
                let focused = client.keyboard_focus.as_ref().map(|focused| {
                    if let Some(index) = layers.iter().position(|(s, ..)| s == focused) {
                        Focused::Layer(index)
                    } else if let Some(index) = windows.iter().position(|s| s == focused) {
                        Focused::Window(index)
                    } else {
                        Focused::Other
                    }
                });
                outcome = Ack::Keyboard(KeyboardReport {
                    focused,
                    keys: client.keys,
                    enters: client.enters,
                    leaves: client.leaves,
                });
            }
            Step::MapPopup { color } => {
                let parent = toplevels.first().ok_or("no toplevel to hang a popup on")?;
                let surface = compositor.create_surface(&qh, ());
                let index = client.window_serials.len();
                client.window_serials.push(None);
                client.window_configures.push(0);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceIndex(index));
                let positioner = wm_base.create_positioner(&qh, ());
                // Both are required before `get_popup`, or the compositor
                // rightly answers with `invalid_positioner`.
                positioner.set_size(50, 50);
                positioner.set_anchor_rect(0, 0, 10, 10);
                let popup = xdg.get_popup(Some(parent), &positioner, &qh, ());
                surface.commit();
                // Ten round trips is far more than the one a configure needs
                // when a compositor sends it: `MapWindow` above gets its
                // toplevel's configure inside `wait_for_configure`'s first
                // few.
                for _ in 0..10 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    if client.window_serials[index].is_some() {
                        break;
                    }
                }
                let configured = client.window_serials[index].is_some();
                if configured {
                    // The same attach-after-ack sequence `MapWindow` runs:
                    // only a mapped popup proves the configure was usable,
                    // not just sent.
                    let serial = client.window_serials[index].ok_or("a popup serial")?;
                    xdg.ack_configure(serial);
                    let buffer = solid_buffer(&shm, &qh, 50, 50, *color);
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage(0, 0, 50, 50);
                    // A frame callback before the attach commit, the way
                    // [`Step::RequestLayerFrame`] does it -- kept alive in
                    // `frames` for the same reason.
                    let tag = FrameTag(client.frame_dones.len());
                    client.frame_dones.push(0);
                    let callback = surface.frame(&qh, tag);
                    surface.commit();
                    frames.push(callback);
                    popups.push((surface, xdg, popup, index));
                } else {
                    popup.destroy();
                    xdg.destroy();
                    surface.destroy();
                }
                positioner.destroy();
                outcome = Ack::PopupConfigured(configured);
            }
            Step::DestroyPopup => {
                let (surface, xdg, popup, _) = popups.pop().ok_or("no mapped popup to destroy")?;
                popup.destroy();
                xdg.destroy();
                surface.destroy();
            }
            Step::ReportPopupConfigures => {
                let (_, _, _, index) = popups.last().ok_or("no mapped popup to report")?;
                outcome = Ack::PopupConfigures(
                    client.window_configures.get(*index).copied().unwrap_or(0),
                );
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<Ack>,
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
            1.0,
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
    fn run(&mut self, step: Step) -> Ack {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        let ack = self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
        self.settle();
        ack
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
    fn run_expecting_disconnect(&mut self, step: Step) -> String {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.acks.try_recv() {
                Ok(_) => panic!("the client survived a request that should have been refused"),
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
        let error = self
            .client
            .take()
            .map(|handle| handle.join().expect("the client thread"))
            .and_then(Result::err)
            .expect("the client should have stopped with the protocol error it provoked");
        self.settle();
        error
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

    /// What the client's own `wl_keyboard` has been told so far.
    fn keyboard(&mut self) -> KeyboardReport {
        let Ack::Keyboard(report) = self.run(Step::ReportKeyboard) else {
            panic!("the keyboard probe should report what the client saw");
        };
        report
    }

    /// Per requested frame callback, how many `done` events arrived.
    fn frames(&mut self) -> Vec<u32> {
        let Ack::Frames(dones) = self.run(Step::ReportFrames) else {
            panic!("the frame probe should report what the client saw");
        };
        dones
    }

    /// Total configures the mapped popup has received.
    fn popup_configures(&mut self) -> u32 {
        let Ack::PopupConfigures(count) = self.run(Step::ReportPopupConfigures) else {
            panic!("the popup-configure probe should report what the client saw");
        };
        count
    }

    /// A left click at a point, press and release, the way a user makes one.
    fn click(&mut self, x: f64, y: f64) {
        self.state.pointer_move(x, y);
        self.state
            .pointer_button(flexwm_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(flexwm_ipc::PointerButton::Left, false);
        self.settle();
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

/// Whether any pixel in the framebuffer is `color` -- position-independent,
/// for surfaces whose exact placement the test does not pin down (a popup
/// lands where the positioner puts it; what matters is that it drew).
fn contains_color(pixels: &[u8], color: [u8; 4]) -> bool {
    pixels.chunks_exact(4).any(|pixel| pixel == color)
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

/// `msg outputs` reports the bar-reserved usable area, not just the full
/// output: with no bar they coincide, and a 30px bar narrows `usable`
/// while `rect` stays whole.
#[test]
fn outputs_reports_the_bar_reserved_usable_area() {
    use flexwm_ipc::{Request, Response};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);

    let Response::Outputs { outputs } = fixture.state.handle_request(Request::Outputs) else {
        panic!("outputs should answer with outputs");
    };
    assert_eq!(outputs.len(), 1, "one output in these tests");
    assert_eq!(
        outputs[0].rect,
        flexwm_ipc::Rect {
            x: 0,
            y: 0,
            width: CANVAS,
            height: CANVAS,
        },
        "rect is the whole output"
    );
    assert_eq!(
        outputs[0].usable, outputs[0].rect,
        "with no bar reserved, usable is the whole output"
    );

    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let Response::Outputs { outputs } = fixture.state.handle_request(Request::Outputs) else {
        panic!("outputs should answer with outputs");
    };
    assert_eq!(
        outputs[0].rect,
        flexwm_ipc::Rect {
            x: 0,
            y: 0,
            width: CANVAS,
            height: CANVAS,
        },
        "a bar reserves from usable, never from the output itself"
    );
    assert_eq!(
        outputs[0].usable,
        flexwm_ipc::Rect {
            x: 0,
            y: 30,
            width: CANVAS,
            height: CANVAS - 30,
        },
        "usable is the output minus the bar's 30px strip"
    );
    fixture.disconnect_client();
}

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
/// unmapped by `layer_destroyed`. See
/// `docs/backlog/protocols/layer-surface-bufferless-exclusive-zone.md` for the
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
// Frame callbacks across layer-surface teardown
// -------------------------------------------------------------------------

/// The control the test below is read against: a frame callback requested
/// on a mapped layer surface is completed by the next frame. Without this,
/// "no `done` arrived" below could pass because `done` never works at all
/// rather than because the teardown path is clean.
#[test]
fn a_frame_callback_on_a_mapped_layer_surface_is_completed() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.render();
    assert_eq!(
        fixture.frames(),
        vec![1],
        "one frame should complete one requested callback"
    );
    fixture.disconnect_client();
}

/// Dismissing a layer surface with a frame callback in flight must not
/// complete that callback afterwards: the client has torn the surface down
/// (`zwlr_layer_surface_v1.destroy` + null attach + commit, the exact
/// sequence DankMaterialShell sends when an overlay is dismissed), and a
/// `done` arriving for an object it no longer knows kills its whole
/// connection (measured 4/4 with Quickshell, exit 255 -- see
/// `docs/backlog/protocols/dms-enablement-gaps.md`, gap 1).
///
/// Read as a delta, not an absolute: whatever frames were already sent
/// before the teardown was dispatched are legitimate and counted in both
/// probes, so only a *new* `done` after the destroy fails this.
#[test]
fn dismissing_a_layer_surface_with_a_frame_callback_in_flight_sends_no_done() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::DismissLayer { index: 0 });
    let before = fixture.frames();
    // Several frames after the teardown was dispatched: none of them may
    // complete the dead surface's callback.
    fixture.render();
    fixture.render();
    fixture.render();
    assert_eq!(
        fixture.frames(),
        before,
        "no frame after the teardown may complete the dead callback"
    );
    fixture.disconnect_client();
}

/// More than one callback outstanding when the surface is dismissed: none
/// of them may complete afterwards either.
#[test]
fn dismissing_with_several_frame_callbacks_in_flight_sends_none() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::DismissLayer { index: 0 });
    let before = fixture.frames();
    assert_eq!(before.len(), 3);
    fixture.render();
    fixture.render();
    fixture.render();
    assert_eq!(
        fixture.frames(),
        before,
        "no frame after the teardown may complete any dead callback"
    );
    fixture.disconnect_client();
}

/// Hiding an already-hidden surface (a null commit on one that is already
/// unmapped) is meaningless but legal -- wlroots treats it as a no-op -- so
/// it must not kill the client either.
#[test]
fn unmapping_twice_without_remap_does_not_kill_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
    // Already hidden: a second null commit must be a silent no-op.
    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    // ...and the surface still works afterwards: re-showing it maps again.
    fixture.run(Step::RemapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.disconnect_client();
}

/// Rapid map/destroy churn: every destruction arms the neutralize path, and
/// the compositor must keep serving through all of it.
#[test]
fn rapid_map_and_destroy_cycles_leave_the_compositor_serving() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    for _ in 0..5 {
        let Ack::Done = fixture.run(Step::CreateLayer(LayerSpec::bar(10))) else {
            panic!("every step should acknowledge");
        };
    }
    // Map and destroy each of them in turn (indices 0..5 in creation order).
    for index in 0..5 {
        fixture.run(Step::MapLayer {
            index,
            color: BAR_BGRA,
        });
        fixture.run(Step::DestroyLayer { index });
    }
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS / 2,
        CANVAS / 2,
        BACKGROUND_BGRA,
        "nothing left drawn",
    );
    fixture.disconnect_client();
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
        // Absurd geometry *and* a demand for every keystroke, so the focus
        // path is exercised by this one too -- it must not take the keyboard,
        // because it never attaches a buffer (see `layer_focus`).
        keyboard: KeyboardInteractivity::Exclusive,
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

    let error = fixture.run_expecting_disconnect(Step::CreateLayer(LayerSpec {
        size: (u32::MAX, u32::MAX),
        ..LayerSpec::bar(30)
    }));

    // Refused with the protocol's own vocabulary -- `invalid_size` (code 1)
    // on the object that asked -- rather than by dropping the connection.
    // wayland-backend renders a protocol error as "Protocol error {code} on
    // object {interface}@{id}: {message}", so both halves are in the string
    // the client thread returned.
    assert!(
        error.contains("Protocol error 1 "),
        "the error should be invalid_size (1): {error}"
    );
    assert!(
        error.contains("zwlr_layer_surface_v1"),
        "the error should name the offending object: {error}"
    );

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
// Keyboard interactivity
//
// Every assertion below is on what the *client's* `wl_keyboard` was told --
// `enter`, `leave`, `key` -- rather than on a compositor-side field. "Who
// holds keyboard focus" is a claim about what reached a client, and a test
// that reads `State::clicked_layer` would pass just as happily with nothing
// ever sent over the wire.
//
// The coordinates: the launcher is [`LayerSpec::launcher`]'s 60-square box in
// the bottom-right corner (140..200 on both axes), a window's own buffer is
// [`WINDOW_BUFFER`] square at (12, 12), and (100, 100) is bare desktop --
// no window, no layer surface, nothing.
// -------------------------------------------------------------------------

/// A 60-square launcher's own corner, and a point in the middle of the first
/// window's buffer.
const ON_LAUNCHER: (f64, f64) = (170.0, 170.0);
const ON_WINDOW: (f64, f64) = (17.0, 37.0);
const ON_DESKTOP: (f64, f64) = (100.0, 100.0);

/// The headline: an `exclusive` surface on `overlay` takes the keyboard the
/// moment it maps, the window is told it lost it, and typed keys really
/// arrive at the layer surface.
///
/// This is the case a launcher and a layer-shell lock screen both need, and
/// the one that was actively dangerous before: the surface used to map and
/// draw over everything while every keystroke went to the window behind it.
#[test]
fn an_exclusive_overlay_surface_takes_the_keyboard_when_it_maps() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();
    assert_eq!(
        before.focused,
        Some(Focused::Window(0)),
        "the window should start with the keyboard"
    );

    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a layer surface with no buffer yet must not take the keyboard"
    );

    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let mapped = fixture.keyboard();
    assert_eq!(
        mapped.focused,
        Some(Focused::Layer(0)),
        "an exclusive overlay surface should hold the keyboard once mapped"
    );
    assert_eq!(
        mapped.leaves,
        before.leaves + 1,
        "the window should have been told it lost the keyboard"
    );

    fixture.state.type_text("hi").expect("two typed characters");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(
        typed.keys,
        mapped.keys + 4,
        "two characters, pressed and released, should have reached the launcher"
    );
    assert_eq!(typed.focused, Some(Focused::Layer(0)));
    // Window focus -- the ring, `set_activated`, `flexwm msg windows` --
    // deliberately does not move: it tracks where focus returns to.
    assert!(fixture.state.focus.is_some(), "the window is still focused");
}

/// The other half of the same guarantee, and the one that must not regress:
/// a bar, a wallpaper or a notification daemon asks for `none`, and nothing
/// about the keyboard changes for it -- ever.
#[test]
fn a_bar_that_wants_no_keyboard_never_takes_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();

    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let after = fixture.keyboard();
    assert_eq!(after.focused, Some(Focused::Window(0)));
    assert_eq!(
        (after.enters, after.leaves),
        (before.enters, before.leaves),
        "a `none` bar mapping must not move keyboard focus at all"
    );

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.keys, before.keys + 2, "keys should reach the window");
    assert_eq!(typed.focused, Some(Focused::Window(0)));

    // ...and clicking it still doesn't, which is the pointer-side half.
    fixture.click(100.0, 15.0);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// A surface that commits but never attaches a buffer has nothing on screen,
/// so it cannot have the keyboard however loudly it asks -- otherwise any
/// client could swallow every keystroke while drawing nothing at all.
#[test]
fn a_layer_surface_with_no_buffer_cannot_hold_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.focused, Some(Focused::Window(0)));
    assert_eq!(
        typed.keys,
        before.keys + 2,
        "the keystroke should have gone to the window, not the empty surface"
    );
    assert!(!fixture.state.keyboard_on_layer);
}

/// `exclusive` on `background` is where the spec hands the decision back
/// ("for the bottom and background layers, the compositor is allowed to use
/// normal focus semantics"), and flexwm's answer is click-to-focus: nothing
/// should be typing into a wallpaper by default.
#[test]
fn an_exclusive_background_surface_only_gets_the_keyboard_by_being_clicked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::wallpaper().with_keyboard(KeyboardInteractivity::Exclusive),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a wallpaper must not take the keyboard just by asking"
    );

    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    let clicked = fixture.keyboard();
    assert_eq!(
        clicked.focused,
        Some(Focused::Layer(0)),
        "clicking it should focus it, like any other window"
    );
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, clicked.keys + 2);

    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "clicking the window should take it back"
    );
}

/// `on_demand` is click-to-focus on any layer, and -- the half the spec is
/// explicit about -- the user has to be able to click *out* of it again.
#[test]
fn an_on_demand_surface_is_focused_and_unfocused_by_clicking() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "on_demand must wait to be clicked"
    );

    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    // Bare desktop: no window, no layer surface. Still a way out.
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "clicking bare desktop should release an on_demand surface"
    );

    // ...and so is a window.
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// Clicking a bar that wants no keyboard is also a way out of an `on_demand`
/// surface: the click is a deliberate act somewhere else, and the surface
/// the user left should not keep the keys.
#[test]
fn clicking_a_bar_releases_an_on_demand_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: BAR_BGRA,
    });

    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(100.0, 15.0);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a click on the bar should have released the launcher"
    );
}

/// The keyboard comes back when the surface holding it goes away -- the one
/// thing a launcher's whole lifecycle depends on.
#[test]
fn destroying_an_exclusive_surface_returns_the_keyboard_to_the_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::DestroyLayer { index: 0 });
    let after = fixture.keyboard();
    assert_eq!(after.focused, Some(Focused::Window(0)));
    assert!(!fixture.state.keyboard_on_layer);

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(
        fixture.keyboard().keys,
        after.keys + 2,
        "typing should reach the window again"
    );
}

/// ...and when it unmaps itself with a null buffer, which is how a
/// layer-shell client hides without destroying its surface. The surface is
/// still in the layer map at this point, so nothing but the buffer test in
/// `layer_focus` catches this.
#[test]
fn unmapping_an_exclusive_surface_returns_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
    assert!(!fixture.state.keyboard_on_layer);
}

/// A surface that *stops* asking for the keyboard has to lose it, which is
/// the transition a gate on "does this surface want keys" would silently
/// miss -- there is nothing left asking, so nothing would trigger a refresh.
#[test]
fn a_surface_that_commits_none_gives_the_keyboard_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::None,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a surface that commits `none` must give the keyboard back"
    );
}

/// A click is *spent*, not stored forever. An `on_demand` surface that gave
/// the keyboard back by committing `none` must not take it again the next
/// time it asks for `on_demand` -- there has been no new click, and the
/// focus ring, `set_activated` and `flexwm msg windows` all still name the
/// window, so the keystrokes would go somewhere nothing on screen points at.
///
/// `none` <-> `on_demand` is the normal lifecycle for such a client (a bar
/// collapsing and re-opening a search field, a notification daemon closing
/// and re-opening an inline reply), not an edge case.
#[test]
fn an_on_demand_surface_does_not_recapture_the_keyboard_after_committing_none() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::None,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    let back = fixture.keyboard();
    assert_eq!(
        back.focused,
        Some(Focused::Window(0)),
        "asking for `on_demand` again must not re-focus the surface: the \
         click that focused it was spent when it committed `none`"
    );
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the spent click should have been forgotten"
    );
    assert!(!fixture.state.keyboard_on_layer);

    // ...and the keys really follow, not just the `enter`.
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.keys, back.keys + 2, "typing should reach the window");
    assert_eq!(typed.focused, Some(Focused::Window(0)));

    // The surface is not broken, just no longer focused for free: a real
    // click still gives it the keyboard.
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// The same rule by the other route a client takes: unmapping with a null
/// buffer and showing itself again -- exactly what a launcher does when it
/// is dismissed and re-shown. `layer_focus` reads `Never` for the unmapped
/// surface, so the click is spent there too.
#[test]
fn an_on_demand_surface_does_not_recapture_the_keyboard_when_it_maps_again() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    fixture.run(Step::RemapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let back = fixture.keyboard();
    assert_eq!(
        back.focused,
        Some(Focused::Window(0)),
        "mapping again must not re-focus the surface without a new click"
    );
    assert!(fixture.state.clicked_layer.is_none());
    assert!(!fixture.state.keyboard_on_layer);
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, back.keys + 2);

    // It really did map again -- its pixels are back in the corner, so the
    // assertion above is about focus, not about a surface that never
    // returned.
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        BAR_BGRA,
        "the re-mapped launcher",
    );
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// The transition that must *not* change: `exclusive` relaxing to
/// `on_demand` keeps the keyboard when the surface was clicked while it held
/// it. That path never passes through `Never`, so forgetting a spent click
/// must not touch it -- the click is what carries the focus over, which is
/// what `click_layer` records an `exclusive` surface's click for.
#[test]
fn an_exclusive_surface_that_relaxes_to_on_demand_keeps_a_click() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    let relaxed = fixture.keyboard();
    assert_eq!(
        relaxed.focused,
        Some(Focused::Layer(0)),
        "a clicked surface that relaxes to `on_demand` should keep the keyboard"
    );
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, relaxed.keys + 2);

    // ...and it is genuinely `on_demand` now, not stuck: a click elsewhere
    // takes the keyboard back, which `exclusive` would have refused.
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// The control for the test above: without a click there is nothing to carry
/// over, so the same relaxation gives the keyboard back to the window. This
/// is what makes that test a statement about the click rather than about
/// `on_demand` surfaces keeping focus in general.
#[test]
fn an_exclusive_surface_that_relaxes_to_on_demand_unclicked_gives_the_keyboard_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "nothing clicked it, so `on_demand` has no focus to hold on to"
    );
    assert!(fixture.state.clicked_layer.is_none());
}

/// Two surfaces both demanding exclusive focus: `overlay` beats `top`, the
/// same order everything else in this compositor stacks them in, and the
/// keyboard falls to the next one down when the winner goes away rather than
/// to the window.
#[test]
fn the_front_most_exclusive_surface_wins_and_focus_falls_back_when_it_goes() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).on_layer(zwlr_layer_shell_v1::Layer::Top),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(1)),
        "the overlay surface should win over the top one"
    );

    fixture.run(Step::DestroyLayer { index: 1 });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the remaining exclusive surface should take the keyboard"
    );
    fixture.run(Step::DestroyLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// The escape hatch, and the reason it is safe to let a full-screen client
/// take every keystroke: keybindings are matched before anything is
/// forwarded, so `Super+h` still works -- and so, on `--tty`, do the
/// `Ctrl+Alt+F<n>` VT switches that use the identical path.
#[test]
fn a_keybinding_still_fires_while_a_layer_surface_holds_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let before = fixture.keyboard();
    assert_eq!(before.focused, Some(Focused::Layer(0)));

    let first = fixture.state.world.arrange().placements[0].id;
    assert_ne!(
        fixture.state.focus,
        Some(first),
        "the second window should hold window focus"
    );
    fixture
        .state
        .press(&flexwm_ipc::KeyCombo {
            key: "h".into(),
            modifiers: vec![flexwm_ipc::Modifier::Super],
        })
        .expect("a pressable combo");
    fixture.settle();

    assert_eq!(
        fixture.state.focus,
        Some(first),
        "Super+h should still move window focus"
    );
    let after = fixture.keyboard();
    assert_eq!(
        after.focused,
        Some(Focused::Layer(0)),
        "the layer surface should still hold the keyboard"
    );
    assert_eq!(
        after.keys,
        before.keys + 2,
        "only the Super modifier's own press and release should have been \
         forwarded; the bound `h` must have been intercepted"
    );
}

/// No windows at all: the surface takes the keyboard, and when it goes the
/// keyboard goes nowhere rather than to a window that doesn't exist.
#[test]
fn an_exclusive_surface_with_no_windows_at_all_is_handled() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::DestroyLayer { index: 0 });
    let after = fixture.keyboard();
    assert_eq!(after.focused, None, "nothing left to hold the keyboard");
    assert!(!fixture.state.keyboard_on_layer);
    assert!(fixture.state.clicked_layer.is_none());
    // The frame after all that still renders.
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
}

/// A client that simply dies while its layer surface holds the keyboard --
/// the crash-mid-operation case. Nothing is left pointing at it, and the
/// compositor keeps serving.
#[test]
fn a_client_that_disconnects_while_holding_the_keyboard_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    assert!(fixture.state.clicked_layer.is_some());

    fixture.disconnect_client();
    assert!(
        fixture.state.clicked_layer.is_none(),
        "a dead client's surface must not be held here"
    );
    assert!(!fixture.state.keyboard_on_layer);
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
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

// -------------------------------------------------------------------------
// Known gaps, pinned so the fix has a failing test to turn green
// -------------------------------------------------------------------------

/// An `xdg_popup` gets its initial configure, maps, draws and tears down
/// without taking the compositor with it.
///
/// This is the fix for `docs/backlog/protocols/xdg-popup-never-configured.md`
/// proving itself: it replaces the pinned-gap test that asserted no popup
/// is ever configured (deleted with that entry), and inverts its
/// assertion -- and then goes further,
/// because a configure the client cannot use is no fix. The popup acks,
/// attaches a buffer and maps; its pixels reach the framebuffer (which is
/// what "no popup maps at all" denied); its frame callback completes
/// (`Window::send_frame` covers popup surfaces, and this is the test that
/// would catch it if that ever stopped); exactly one configure arrived
/// (later commits stay quiet, as a non-reactive positioner requires); and
/// destroying the popup leaves the compositor serving.
///
/// What this deliberately does *not* cover is popup input: pointer
/// hit-testing stops at the window tree, keyboard focus never moves onto
/// the popup, and `grab` is still a no-op -- menus show but cannot be
/// clicked yet. Follow-ups, not this fix.
#[test]
fn an_xdg_popup_configures_maps_draws_and_tears_down() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);

    let before = fixture.render();
    assert!(
        !contains_color(&before, POPUP_BGRA),
        "the popup color should be absent before any popup exists"
    );

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup { color: POPUP_BGRA }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "the popup never got its initial configure -- the fix regressed"
    );

    let pixels = fixture.render();
    assert!(
        contains_color(&pixels, POPUP_BGRA),
        "the mapped popup's pixels should reach the framebuffer"
    );
    assert_eq!(
        fixture.frames(),
        vec![1],
        "one frame should complete the popup's requested callback"
    );
    assert_eq!(
        fixture.popup_configures(),
        1,
        "the popup should be configured exactly once -- later commits stay quiet"
    );

    fixture.run(Step::DestroyPopup);
    // ...and the compositor survived the whole lifecycle, still serving.
    assert_eq!(fixture.usable(), WHOLE);
    let after = fixture.render();
    assert_eq!(after.len(), (CANVAS * CANVAS * 4) as usize);
    assert!(
        !contains_color(&after, POPUP_BGRA),
        "the destroyed popup's pixels should be gone"
    );
    fixture.disconnect_client();
}
