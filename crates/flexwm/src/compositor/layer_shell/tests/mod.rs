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
//!
//! This file is the harness. The tests live in the submodules below, one per
//! concern.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use flexwm_core::{Rect, Size};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
    wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
// The client half of `zwp_input_method_v2`, for the real IME keyboard grab
// [`Step::ImeGrabKeyboard`] holds: Smithay already enables the matching
// "server" side for its own input-method support, so this adds a feature
// to a crate already in the tree, not a new dependency -- the same shape
// `activation/tests/ime_grab.rs` already uses.
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2, zwp_input_method_manager_v2, zwp_input_method_v2,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::layer_shell::{ABOVE_WINDOWS, BELOW_WINDOWS};
use crate::compositor::test_support::{self, Harness, contains, wait_for};

// The tests themselves, split by concern; everything below is the harness
// they share. See `docs/backlog/resolved/large-test-file-organization-done.md`
// for why, and `crate::compositor::test_support` for the half of the harness
// that is shared with the other real-client suites.
mod adversarial;
mod frames;
mod input;
mod layout;
mod popup;
mod popup_serial;
mod teardown;

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

/// Three points every input test aims at: a 60-square launcher's own corner
/// ([`LayerSpec::launcher`] anchors it bottom-right, 140..200 on both axes),
/// the middle of the first window's own [`WINDOW_BUFFER`]-square buffer, and
/// bare desktop -- no window, no layer surface, nothing.
///
/// Here rather than in `input.rs` because `popup.rs` aims at them too.
const ON_LAUNCHER: (f64, f64) = (170.0, 170.0);
const ON_WINDOW: (f64, f64) = (17.0, 37.0);
const ON_DESKTOP: (f64, f64) = (100.0, 100.0);

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
    /// `wl_surface.destroy` on the `index`-th layer surface while keeping
    /// its `zwlr_layer_surface_v1` role object and the whole connection
    /// alive: the explicit form of the implicit-disconnect order where the
    /// surface dies before its role, so `layer_destroyed` never runs for
    /// it. What the `mapped_layers` sweep in `CompositorHandler::destroyed`
    /// exists for.
    DestroyLayerWlSurface { index: usize },
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
    /// Create an `xdg_popup` on `parent` and drive it through configure,
    /// ack, attach and a frame request, reporting whether the compositor
    /// ever configured it. See
    /// [`an_xdg_popup_configures_maps_draws_and_tears_down`].
    MapPopup {
        parent: PopupParent,
        color: [u8; 4],
        /// Whether to ask for an explicit grab (`xdg_popup.grab`) on the way
        /// up, and with which serial. Sent *before* the first commit, which
        /// is the only time it is legal: the pinned rev's
        /// `PopupSurface::pre_commit_hook` answers a grab requested after
        /// the popup has a buffer with `invalid_grab` and kills the client.
        grab: Option<GrabSource>,
    },
    /// Report the input serials this client has actually been sent -- the
    /// only honest way for a test to name "a real key serial" without
    /// re-deriving the compositor's counter.
    ReportSerials,
    /// Destroy the popup [`Step::MapPopup`] made last: `xdg_popup.destroy` +
    /// `xdg_surface.destroy` + `wl_surface.destroy`.
    DestroyPopup,
    /// Destroy the mapped popup and map a grabbing replacement for it in
    /// the *same* client turn: the destroy and the new grab leave in one
    /// flush, the way a toolkit replacing its menu sends them, so the
    /// compositor dispatches them adjacently with no reap in between. The
    /// replacement hangs off the window -- the old menu is gone, so nothing
    /// nests. How the serial-gate tests reproduce a menubar hover-switch
    /// exactly.
    ReplacePopup { color: [u8; 4], serial: u32 },
    /// Report how many `xdg_surface.configure` events the mapped popup has
    /// received in total -- the compositor must send exactly one (later
    /// commits stay quiet, as a non-reactive positioner requires).
    ReportPopupConfigures,
    /// Report how many `xdg_popup.popup_done` events this client has been
    /// sent across every popup it ever made -- which is the only evidence
    /// that the compositor dismissed one, since nothing else on the wire
    /// says so.
    ReportPopupDone,
    /// Report which of the client's surfaces its own `wl_pointer` was last
    /// told it entered.
    ReportPointer,
    /// `ext_session_lock_manager_v1.lock`, and nothing else: no lock surface
    /// is created, because what these tests ask of a lock is only that it
    /// takes input away from everything that is not it. The session is
    /// locked the moment the request is accepted (see `session_lock.rs`'s
    /// note on `owner`), which is the transition under test.
    LockSession,
    /// `get_input_method` + `grab_keyboard` through the real protocol, and
    /// hold the grab: the seat's keyboard is grabbed the way fcitx5 holds
    /// it while active (see
    /// `docs/backlog/resolved/popup-grab-blocked-by-ime-grab-done.md`), so a
    /// popup grab asked for afterwards meets the `taken` refusal in
    /// `State::grab_popup` (`compositor/popup.rs`).
    ImeGrabKeyboard,
    /// Send `release` on the grab [`Step::ImeGrabKeyboard`] holds, letting
    /// the seat go. Explicit rather than a drop: dropping the proxy sends
    /// nothing on the wire. The input-method object itself stays, the way
    /// an IME outlives one activation.
    ImeUngrabKeyboard,
}

/// Which serial a [`Step::MapPopup`] grab passes to `xdg_popup.grab`.
#[derive(Clone, Copy)]
enum GrabSource {
    /// The last `wl_keyboard.key` serial this client was sent -- what a
    /// toolkit passes after a key-driven menu open.
    Key,
    /// A caller-chosen serial: a fabricated one, another client's, or an
    /// enter serial read back through [`Step::ReportSerials`]. How the
    /// serial-gate tests name exactly the serial under test.
    Serial(u32),
}

/// What an `xdg_popup` hangs off: a window's `xdg_toplevel`, or a layer
/// surface (`zwlr_layer_surface_v1.get_popup`, a bar's own dropdown).
#[derive(Clone, Copy)]
enum PopupParent {
    /// The first toplevel [`Step::MapWindow`] mapped.
    Window,
    /// The `index`-th layer surface, by creation order.
    Layer(usize),
    /// The `index`-th still-mapped popup -- a submenu, which is the shape
    /// that exercises nested grabs.
    Popup(usize),
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
    /// [`Step::ReportPopupDone`]'s answer: total `popup_done` events.
    PopupDone(u32),
    /// [`Step::ReportPointer`]'s answer.
    Pointer(Option<Focused>),
    /// [`Step::ReportKeyboard`]'s answer.
    Keyboard(KeyboardReport),
    /// [`Step::ReportSerials`]'s answer: the input serials this client has
    /// actually been sent.
    Serials(SerialReport),
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
    /// A popup, by the order [`Step::MapPopup`] created it.
    Popup(usize),
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

/// The input serials one client has actually been sent, by event kind --
/// the vocabulary the serial-gate tests grab with.
#[derive(Clone, Copy, Debug, Default)]
struct SerialReport {
    /// Last `wl_keyboard.key` serial, if any.
    key: Option<u32>,
    /// Last `wl_pointer.button` serial, if any.
    button: Option<u32>,
    /// Last `wl_pointer.enter` serial, if any.
    pointer_enter: Option<u32>,
    /// Last `wl_keyboard.enter` serial, if any.
    keyboard_enter: Option<u32>,
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
    /// ...and the pointer, the same way. What it was last told it entered is
    /// the only honest answer to "did that click reach the popup": the
    /// compositor's own hit test is what is under test, so reading it back
    /// would prove nothing.
    pointer: Option<wl_pointer::WlPointer>,
    pointer_focus: Option<wl_surface::WlSurface>,
    /// `ext_session_lock_manager_v1`, bound only so [`Step::LockSession`]
    /// can take a lock.
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    /// `zwp_input_method_manager_v2`, bound only so
    /// [`Step::ImeGrabKeyboard`] can take a real IME keyboard grab through
    /// the protocol.
    input_method_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    /// The input-method object and its held keyboard grab, kept alive for as
    /// long as the test wants the seat grabbed: sending `release` on the
    /// grab object lets it go, which is exactly what
    /// [`Step::ImeUngrabKeyboard`] does. (A bare drop sends nothing --
    /// measured with `WAYLAND_DEBUG=1`: no `release` on the wire and the
    /// seat stays grabbed -- so the ungrab step calls `release()`
    /// explicitly.)
    /// An IME is always somebody else's client; here the harness plays both
    /// parts, which the `taken` check under test cannot tell apart -- it
    /// only asks whether *a* grab holds the seat, not whose.
    input_method: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    ime_grab: Option<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2>,
    /// The surface the outstanding `wl_keyboard.enter` named. Stored as the
    /// raw `wl_surface` because this handler has no idea which of the
    /// script's surfaces it is; [`run_client`] resolves it at report time.
    keyboard_focus: Option<wl_surface::WlSurface>,
    keys: u32,
    enters: u32,
    leaves: u32,
    /// The serial of the last `wl_keyboard.key` this client was sent, which
    /// is the serial a real toolkit passes to `xdg_popup.grab` (GTK and Qt
    /// both track the last button/key/touch serial for exactly that).
    /// `None` until the client has actually been sent a key.
    last_key_serial: Option<u32>,
    /// The serial of the last `wl_pointer.button` this client was sent.
    /// `None` until it has actually been sent one.
    last_button_serial: Option<u32>,
    /// The serials of the last `wl_pointer.enter` and `wl_keyboard.enter`
    /// this client was sent. A toolkit whose last-seen event was an enter
    /// (Qt updates its grab serial on pointer enters; a hover-opened menu
    /// has no newer event at all) passes one of these to `xdg_popup.grab`,
    /// so the serial-gate tests need to name them exactly.
    last_pointer_enter_serial: Option<u32>,
    last_keyboard_enter_serial: Option<u32>,
    /// `xdg_popup.popup_done` events received, cumulative across every popup
    /// -- the compositor dismissing a popup is invisible on the wire
    /// otherwise.
    popup_dones: u32,
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
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "zwp_input_method_manager_v2" => {
                client.input_method_manager = Some(registry.bind(name, version.min(1), qh, ()));
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
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        if capabilities.contains(wl_seat::Capability::Keyboard) && client.keyboard.is_none() {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
        if capabilities.contains(wl_seat::Capability::Pointer) && client.pointer.is_none() {
            client.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

/// Only `enter`/`leave`/`button` are recorded: which surface the pointer is
/// on is the whole question a popup hit test raises, and motion/axis add
/// nothing to it -- while the button serial is what a toolkit passes to
/// `xdg_popup.grab` for a click-opened menu.
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
            wl_pointer::Event::Enter {
                surface, serial, ..
            } => {
                client.pointer_focus = Some(surface);
                client.last_pointer_enter_serial = Some(serial);
            }
            wl_pointer::Event::Leave { .. } => client.pointer_focus = None,
            wl_pointer::Event::Button { serial, .. } => {
                client.last_button_serial = Some(serial);
            }
            _ => {}
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
            wl_keyboard::Event::Enter {
                surface, serial, ..
            } => {
                client.keyboard_focus = Some(surface);
                client.enters += 1;
                client.last_keyboard_enter_serial = Some(serial);
            }
            wl_keyboard::Event::Leave { .. } => {
                client.keyboard_focus = None;
                client.leaves += 1;
            }
            wl_keyboard::Event::Key { serial, .. } => {
                client.keys += 1;
                client.last_key_serial = Some(serial);
            }
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

/// `xdg_popup.popup_done` is the compositor saying "I dismissed this" --
/// the only wire evidence a grab ended by anything other than the client's
/// own `destroy`, so it is counted rather than ignored.
impl Dispatch<xdg_popup::XdgPopup, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_popup::Event::PopupDone = event {
            client.popup_dones = client.popup_dones.saturating_add(1);
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
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);
// `locked`/`finished` both arrive here and neither is asserted on: these
// tests lock only to take input away, and `session_lock/tests.rs` owns
// everything about the lock's own lifecycle.
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);
// An IME's own objects are held, never read: the grab diverts keys to the
// grab object, and what matters here is only that the seat *is* grabbed,
// which the test asserts compositor-side.
wayland_client::delegate_noop!(TestClient: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(TestClient: ignore zwp_input_method_v2::ZwpInputMethodV2);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2
);

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

/// One mapped popup: its surface, `xdg_surface`, `xdg_popup`, and
/// serial-slot index into [`TestClient::window_serials`], so a destroy can
/// tear it down and a configure count can find it.
type PopupEntry = (
    wl_surface::WlSurface,
    xdg_surface::XdgSurface,
    xdg_popup::XdgPopup,
    usize,
);

/// Tears down the mapped popup [`Step::MapPopup`] made last:
/// `xdg_popup.destroy` + `xdg_surface.destroy` + `wl_surface.destroy`.
fn destroy_popup(popups: &mut Vec<PopupEntry>) -> Result<(), String> {
    let (surface, xdg, popup, _) = popups.pop().ok_or("no mapped popup to destroy")?;
    popup.destroy();
    xdg.destroy();
    surface.destroy();
    Ok(())
}

/// Creates an `xdg_popup` on `parent` and drives it through configure, ack,
/// attach and a frame request, reporting whether the compositor ever
/// configured it.
///
/// Shared by [`Step::MapPopup`] and [`Step::ReplacePopup`] rather than
/// written out twice: the two differ only in what runs before (nothing, or
/// a destroy in the same flush) and which serial the grab names.
///
/// `grab` is the serial to ask with, or `None` for no grab. It is sent
/// *before* the first commit, which is the only time it is legal: the pinned
/// rev's `PopupSurface::pre_commit_hook` answers a grab requested after the
/// popup has a buffer with `invalid_grab` and kills the client.
#[allow(clippy::too_many_arguments)]
fn map_popup(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    compositor: &wl_compositor::WlCompositor,
    shm: &wl_shm::WlShm,
    wm_base: &xdg_wm_base::XdgWmBase,
    seat: &wl_seat::WlSeat,
    layers: &[(
        wl_surface::WlSurface,
        zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        LayerSpec,
    )],
    toplevels: &[xdg_surface::XdgSurface],
    popups: &mut Vec<PopupEntry>,
    popup_surfaces: &mut Vec<wl_surface::WlSurface>,
    frames: &mut Vec<wl_callback::WlCallback>,
    parent: PopupParent,
    color: [u8; 4],
    grab: Option<u32>,
) -> Result<Ack, String> {
    let surface = compositor.create_surface(qh, ());
    let index = client.window_serials.len();
    client.window_serials.push(None);
    client.window_configures.push(0);
    let xdg = wm_base.get_xdg_surface(&surface, qh, SurfaceIndex(index));
    let positioner = wm_base.create_positioner(qh, ());
    // Both are required before `get_popup`, or the compositor
    // rightly answers with `invalid_positioner`.
    positioner.set_size(50, 50);
    positioner.set_anchor_rect(0, 0, 10, 10);
    // A layer-parented popup names *no* xdg parent here and gets
    // one from `zwlr_layer_surface_v1.get_popup` instead, which
    // is how the two protocols are specified to meet.
    let popup = match parent {
        PopupParent::Window => {
            let parent = toplevels.first().ok_or("no toplevel to hang a popup on")?;
            xdg.get_popup(Some(parent), &positioner, qh, ())
        }
        PopupParent::Layer(index) => {
            let (_, layer, _) = layers.get(index).ok_or("no such layer surface")?;
            let popup = xdg.get_popup(None, &positioner, qh, ());
            layer.get_popup(&popup);
            popup
        }
        PopupParent::Popup(index) => {
            let (_, parent, ..) = popups.get(index).ok_or("no such popup")?;
            xdg.get_popup(Some(parent), &positioner, qh, ())
        }
    };
    if let Some(serial) = grab {
        popup.grab(seat, serial);
    }
    surface.commit();
    // Ten round trips is far more than the one a configure needs
    // when a compositor sends it: `MapWindow` above gets its
    // toplevel's configure inside `wait_for`'s first few.
    for _ in 0..10 {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
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
        let buffer = solid_buffer(shm, qh, 50, 50, color);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 50, 50);
        // A frame callback before the attach commit, the way
        // [`Step::RequestLayerFrame`] does it -- kept alive in
        // `frames` for the same reason.
        let tag = FrameTag(client.frame_dones.len());
        client.frame_dones.push(0);
        let callback = surface.frame(qh, tag);
        surface.commit();
        frames.push(callback);
        popup_surfaces.push(surface.clone());
        popups.push((surface, xdg, popup, index));
    } else {
        popup.destroy();
        xdg.destroy();
        surface.destroy();
    }
    positioner.destroy();
    Ok(Ack::PopupConfigured(configured))
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
    // Named here rather than per step because `xdg_popup.grab` takes one.
    let seat = client.seat.clone().ok_or("no wl_seat")?;

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
    // Every popup surface ever mapped, in creation order and never removed,
    // so [`Focused::Popup`]'s index stays stable across a destroy -- unlike
    // `popups` above, which [`Step::DestroyPopup`] pops from.
    let mut popup_surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    // Held rather than dropped -- see [`Step::LockSession`].
    let mut locks: Vec<ext_session_lock_v1::ExtSessionLockV1> = Vec::new();
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
                let serial = wait_for(&mut queue, &mut client, "a toplevel configure", |client| {
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
                let (width, height) =
                    wait_for(&mut queue, &mut client, "a layer configure", |client| {
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
            Step::DestroyLayerWlSurface { index } => {
                let (surface, ..) = layers.get(*index).ok_or("no such layer surface")?;
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
                let (width, height) = wait_for(
                    &mut queue,
                    &mut client,
                    "a fresh layer configure",
                    |client| {
                        let fresh = client.layer_configures.get(index).copied().unwrap_or(0) > seen;
                        fresh
                            .then(|| client.layer_sizes.get(index).copied().flatten())
                            .flatten()
                    },
                )?;
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
                    } else if let Some(index) = popup_surfaces.iter().position(|s| s == focused) {
                        Focused::Popup(index)
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
            Step::MapPopup {
                parent,
                color,
                grab,
            } => {
                // The key-driven shape reads the serial the client was sent;
                // an explicit serial passes straight through.
                let serial = grab
                    .map(|source| match source {
                        GrabSource::Key => client.last_key_serial.ok_or(
                            "a grab needs an input serial; press a key into this client first",
                        ),
                        GrabSource::Serial(serial) => Ok(serial),
                    })
                    .transpose()?;
                outcome = map_popup(
                    &mut queue,
                    &mut client,
                    &qh,
                    &compositor,
                    &shm,
                    &wm_base,
                    &seat,
                    &layers,
                    &toplevels,
                    &mut popups,
                    &mut popup_surfaces,
                    &mut frames,
                    *parent,
                    *color,
                    serial,
                )?;
            }
            Step::ReplacePopup { color, serial } => {
                destroy_popup(&mut popups)?;
                outcome = map_popup(
                    &mut queue,
                    &mut client,
                    &qh,
                    &compositor,
                    &shm,
                    &wm_base,
                    &seat,
                    &layers,
                    &toplevels,
                    &mut popups,
                    &mut popup_surfaces,
                    &mut frames,
                    PopupParent::Window,
                    *color,
                    Some(*serial),
                )?;
            }
            Step::DestroyPopup => {
                destroy_popup(&mut popups)?;
            }
            Step::ReportPopupConfigures => {
                let (_, _, _, index) = popups.last().ok_or("no mapped popup to report")?;
                outcome = Ack::PopupConfigures(
                    client.window_configures.get(*index).copied().unwrap_or(0),
                );
            }
            Step::ReportPopupDone => outcome = Ack::PopupDone(client.popup_dones),
            Step::ReportSerials => {
                outcome = Ack::Serials(SerialReport {
                    key: client.last_key_serial,
                    button: client.last_button_serial,
                    pointer_enter: client.last_pointer_enter_serial,
                    keyboard_enter: client.last_keyboard_enter_serial,
                });
            }
            Step::ReportPointer => {
                outcome = Ack::Pointer(client.pointer_focus.as_ref().map(|focused| {
                    if let Some(index) = layers.iter().position(|(s, ..)| s == focused) {
                        Focused::Layer(index)
                    } else if let Some(index) = windows.iter().position(|s| s == focused) {
                        Focused::Window(index)
                    } else if let Some(index) = popup_surfaces.iter().position(|s| s == focused) {
                        Focused::Popup(index)
                    } else {
                        Focused::Other
                    }
                }));
            }
            Step::LockSession => {
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1 -- the global is missing")?;
                // Kept alive for the rest of the run: dropping the proxy
                // destroys the lock object, which is a legal way to abandon a
                // lock and would change what is under test.
                locks.push(manager.lock(&qh, ()));
            }
            Step::ImeGrabKeyboard => {
                let manager = client
                    .input_method_manager
                    .clone()
                    .ok_or("no zwp_input_method_manager_v2 -- the global is missing")?;
                let method = manager.get_input_method(&seat, &qh, ());
                // Held in the client struct so the seat stays grabbed:
                // letting go is `release()`, which is what
                // [`Step::ImeUngrabKeyboard`] sends.
                client.ime_grab = Some(method.grab_keyboard(&qh, ()));
                client.input_method = Some(method);
            }
            Step::ImeUngrabKeyboard => {
                let grab = client
                    .ime_grab
                    .take()
                    .ok_or("no held IME grab to release")?;
                grab.release();
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time. See [`crate::compositor::test_support`] for
/// everything that is not specific to this protocol.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// Disconnects the one client and waits for the compositor to notice.
    fn disconnect_client(&mut self) {
        self.disconnect(0);
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

    /// How many popups the compositor has tracked against the first layer
    /// surface, counted from the `PopupTree` the render path, the hit test
    /// and the frame-callback pass all walk.
    ///
    /// White-box, unlike everything else here, and deliberately so: a popup
    /// tracked *twice* is indistinguishable on the wire and in pixels (a
    /// duplicate draws in the same place, and frame callbacks are taken
    /// rather than copied when sent) right up until a dismissal removes one
    /// node and leaves the other on screen. Counting the tree is the only
    /// direct way to pin it.
    fn popups_on_first_layer(&self) -> usize {
        let output = self.state.output.as_ref().expect("the one output");
        let map = smithay::desktop::layer_map_for_output(output);
        let layer = map.layers().next().expect("a mapped layer surface");
        smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface()).count()
    }

    /// The input serials the client has actually been sent, by kind -- the
    /// only honest source of "a real key/button/enter serial" for the
    /// serial-gate tests.
    fn serials(&mut self) -> SerialReport {
        let Ack::Serials(report) = self.run(Step::ReportSerials) else {
            panic!("the serial probe should report what the client saw");
        };
        report
    }

    /// Total `xdg_popup.popup_done` events the client has been sent.
    fn popup_dones(&mut self) -> u32 {
        let Ack::PopupDone(count) = self.run(Step::ReportPopupDone) else {
            panic!("the popup-done probe should report what the client saw");
        };
        count
    }

    /// Which of the client's surfaces its `wl_pointer` was last told it
    /// entered.
    fn pointer_focus(&mut self) -> Option<Focused> {
        let Ack::Pointer(focused) = self.run(Step::ReportPointer) else {
            panic!("the pointer probe should report what the client saw");
        };
        focused
    }

    /// Presses and releases one unbound key, so the client is sent a real
    /// `wl_keyboard.key` whose serial it can pass to `xdg_popup.grab`.
    ///
    /// `a` deliberately: [`Keybindings::default`] binds nothing bare, so
    /// this is forwarded rather than intercepted -- and an intercepted key
    /// reaches no client and would leave the grab with no serial to use.
    fn press_a_key(&mut self) {
        self.state
            .press(&flexwm_ipc::KeyCombo {
                key: "a".into(),
                modifiers: Vec::new(),
            })
            .expect("a pressable combo");
        self.settle();
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

/// Reads one pixel out of a [`CANVAS`]-square BGRA framebuffer.
fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}

/// Where the first `color` pixel is, scanning in row order.
fn find_color(pixels: &[u8], color: [u8; 4]) -> Option<(f64, f64)> {
    test_support::find_color(pixels, CANVAS, color)
}

fn assert_pixel(pixels: &[u8], x: i32, y: i32, expected: [u8; 4], what: &str) {
    test_support::assert_pixel(pixels, CANVAS, x, y, expected, what);
}
