//! Tests for `ext-session-lock-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `ext_session_lock_manager_v1`, `xdg_wm_base`, `wl_seat` and `wl_output`
//! exactly as `swaylock` does -- through a real [`State`] with a real
//! `headless` backend, then render with the real [`PixmanRenderer`] and read
//! the framebuffer back. That is deliberate, and the same choice
//! `layer_shell/tests/` and `cursor/tests.rs` made for the same reason, but
//! it matters more here than anywhere else in this compositor: the claim
//! under test is "nothing that was on screen before the lock is on screen
//! after it", and that claim is about *pixels*. A test that asserted on which
//! enum variant the render path chose would pass just as happily against a
//! version that drew the window behind a transparent backdrop.
//!
//! The same goes for input: every focus assertion here is made from what the
//! *client* was told (`wl_keyboard.enter`, `wl_pointer.button`,
//! `xdg_toplevel.close`), never from a field inside the compositor, because
//! "the window did not receive that keystroke" is a claim about the wire.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.
//!
//! This file is the harness. The tests live in the submodules below, one per
//! concern.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use flexwm_ipc::{KeyCombo, Modifier, PointerButton, Request, Response};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
    wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2, zwp_input_method_v2, zwp_input_popup_surface_v2,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::{self, Harness, assert_pixel, contains, wait_for};

// The tests themselves, split by concern; everything below is the harness
// they share. See `docs/backlog/resolved/large-test-file-organization-done.md`
// for why, and `crate::compositor::test_support` for the half of the harness
// that is shared with the other real-client suites.
mod abandoned;
mod blanking;
mod first_click;
mod ime_popup;
mod input;
mod lifecycle;
mod per_output;
mod teardown;
mod vblank_confirm;

/// The framebuffer these tests render into.
const CANVAS: i32 = 120;
/// A window's buffer, deliberately smaller than any placement on this canvas
/// so its own pixels sit at the placement's top-left corner.
const WINDOW_BUFFER: i32 = 40;

// Colors as the BGRA bytes a pixman `Argb8888` buffer holds them in.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
/// An `overlay` layer-shell surface's colour -- the layer that draws in
/// *front* of ordinary windows, so it is the strongest version of "a
/// layer-shell client must not be visible while locked".
const OVERLAY_BGRA: [u8; 4] = [0x20, 0x20, 0xE0, 0xFF];
const LOCK_BGRA: [u8; 4] = [0xE0, 0x20, 0xE0, 0xFF];
/// What [`super::BACKDROP`] comes out as on screen.
const BLACK_BGRA: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
/// ...and [`super::ABANDONED_BACKDROP`].
const RED_BGRA: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];
/// What [`appearance`]'s `background_color` comes out as: the colour an
/// *unlocked* empty session shows, and therefore the one a locked frame must
/// never contain.
const BACKGROUND_BGRA: [u8; 4] = [0x56, 0x34, 0x12, 0xFF];
/// A background window's `xdg_popup`, deliberately unlike every colour above
/// so its presence over a lock screen reads unambiguously.
const XDG_POPUP_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];
/// An input-method candidate window, likewise unmistakable.
const IME_BGRA: [u8; 4] = [0x20, 0xE0, 0xE0, 0xFF];
/// Both popup kinds' buffers: small enough to sit anywhere on the canvas.
const POPUP_BUFFER: i32 = 24;

/// A palette nothing else in this compositor defaults to, so a pixel
/// assertion can only pass because the thing it names was actually drawn --
/// in particular the desktop background is a distinctive brown, which a
/// locked frame must never show.
fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 3,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(1.0, 0.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// Which of a client's own surfaces an `enter` event named, by the creation
/// order the test script addresses them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Window(usize),
    Lock(usize),
    /// A surface this client made that the script doesn't track. Nothing
    /// produces one today; it exists so a mismatch reads as a mismatch rather
    /// than as "no focus".
    Other,
}

/// Everything one client has been told, which is the only evidence these
/// tests accept about focus and input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Report {
    keyboard_focus: Option<Which>,
    keys: u32,
    pointer_focus: Option<Which>,
    buttons: u32,
    /// `xdg_toplevel.close` events -- what a `CloseFocused` action sends.
    closes: u32,
    /// `ext_session_lock_v1.locked` events, cumulative.
    locked: u32,
    /// ...and `finished`, which is how a refusal arrives.
    finished: u32,
}

/// One instruction for a client thread.
enum Step {
    /// Map an `xdg_toplevel` with a solid [`WINDOW_BGRA`] buffer.
    MapWindow,
    /// `ext_session_lock_manager_v1.lock`, then round-trip until the
    /// compositor answers `locked` or `finished`.
    Lock,
    /// `get_lock_surface` on the `index`-th lock object for the one output,
    /// ack its configure and -- with a `color` -- attach a solid buffer of
    /// exactly the configured size. Without one, stop after the ack: a lock
    /// client that has not drawn anything yet.
    LockSurface { lock: usize, color: Option<[u8; 4]> },
    /// The same, but naming the one physical output through the client's
    /// *second* `wl_output` bind. Smithay refuses the same resource twice
    /// (`DuplicateOutput`, "Output is already locked") while accepting a
    /// output, so this -- and only this -- is how a second surface comes to
    /// exist on one output. The admission itself belongs to the
    /// duplicate-bind ticket; this step exists so the per-output suite can
    /// pin what every admitted surface is configured and drawn as.
    LockSurfaceSecondBind { lock: usize, color: Option<[u8; 4]> },
    /// Map a full-output `overlay` layer surface with a solid
    /// [`OVERLAY_BGRA`] buffer -- a bar or a launcher, drawn in front of
    /// every window.
    MapOverlayLayer,
    /// `ext_session_lock_surface_v1.destroy` plus the `wl_surface` under it --
    /// the protocol's "the compositor must fall back to rendering a solid
    /// color" case.
    DestroyLockSurface { index: usize },
    /// The same protocol case, but destroying *only* the
    /// `ext_session_lock_surface_v1` role object and keeping the `wl_surface`
    /// underneath -- and the lock, and the connection -- alive. Legal, and
    /// what a locker does when an output is removed under it, so it reaches
    /// none of the hooks the step above does.
    DestroyLockRoleOnly { index: usize },
    /// `unlock_and_destroy` on the `index`-th lock object.
    Unlock { lock: usize },
    /// `ext_session_lock_manager_v1.lock` without waiting for the compositor's
    /// answer -- the only way to hold the state a real locker is in *before*
    /// `locked` arrives, which is where both this module's ugliest cases live.
    LockNoWait,
    /// The whole hostile sequence as one client-side batch: `lock`,
    /// `get_lock_surface` (never mapped -- no ack, no buffer), then
    /// `ext_session_lock_v1.destroy`. Deliberately one batch: no compositor
    /// frame can interleave, so nothing about this needs a race to win.
    AttackLock,
    /// `ext_session_lock_v1.destroy` on the `lock`-th lock object -- legal
    /// before `locked` has been sent, and it leaves the `wl_surface` under any
    /// lock surface alive and the connection up.
    DestroyLock { lock: usize },
    /// A bare commit on the `index`-th lock surface's `wl_surface` -- what a
    /// locker teardown does after destroying the role object when it keeps
    /// the surface (and the connection) alive.
    CommitLockSurface { index: usize },
    /// A null commit (`attach(nil)` plus `commit`) on the `index`-th lock
    /// surface's `wl_surface` -- the second half of the real Quickshell
    /// unlock teardown (`destroy` the role, null-attach, commit).
    NullCommitLockSurface { index: usize },
    /// Attach a same-size buffer to the `index`-th lock surface's
    /// `wl_surface` and commit -- a content commit after whatever teardown
    /// came before it.
    ReattachLockBuffer { index: usize },
    /// Ack the latest configure on the `index`-th lock surface and redraw
    /// it at the configured size -- what a real locker does when the output
    /// it is on is resized. Unlike [`Step::ReattachLockBuffer`], which
    /// replays the already-acked size, this names the *new* configure, which
    /// must be acked before the commit or the compositor rightly kills the
    /// client for committing before its first ack.
    RedrawLockSurface { index: usize },
    /// `get_lock_surface` for the `index`-th lock object, then attach a
    /// buffer and commit *without* acking the configure -- the by-design
    /// `CommitBeforeFirstAck` kill, which must keep working.
    LockSurfaceNoAckWithBuffer { lock: usize },
    /// Map a non-grabbing `xdg_popup` on the `window`-th toplevel, with a
    /// solid [`XDG_POPUP_BGRA`] buffer. No grab deliberately: a grabbed menu
    /// is dismissed by the lock itself (see `input.rs`), while a mapped
    /// tooltip-style popup survives it -- still tracked against its window --
    /// which is exactly the shape the locked render path must not draw.
    MapXdgPopup { window: usize },
    /// Bind `zwp_text_input_manager_v3`'s text input and
    /// `zwp_input_method_manager_v2`'s input method on the seat. Run before
    /// focus lands where the field will be: Smithay only delivers
    /// `zwp_text_input_v3.enter` while an input-method instance already
    /// exists (see `input_method/tests.rs`).
    SetupIme,
    /// `zwp_text_input_v3.enable` + `commit`: this client's text field is
    /// ready, which is what activates the input method against it.
    EnableTextInput,
    /// `zwp_text_input_v3.disable` + `commit`: the field went away.
    DisableTextInput,
    /// Move the text cursor within the field, which the IME popup follows.
    /// Surface-local, like the protocol defines it.
    SetCursorRectangle { x: i32, y: i32, w: i32, h: i32 },
    /// `zwp_input_method_v2.get_input_popup_surface` plus a solid
    /// [`IME_BGRA`] buffer of `POPUP_BUFFER` square, committed at once.
    CreateImePopup,
    /// `wl_surface.frame` on the IME popup: arms [`ImeReport::ime_frame`].
    RequestImeFrame,
    /// `wl_surface.frame` on the `popup`-th xdg popup: arms that slot of
    /// [`ImeReport::xdg_frames`].
    RequestXdgPopupFrame { popup: usize },
    /// Hand back the IME state: activation, and which popup frame callbacks
    /// have arrived since they were last armed.
    ImeStatus,
    /// Hand back everything this client has seen.
    Report,
}

impl Step {
    /// A lock surface that draws, which is what every test but one wants.
    fn map_lock_surface(lock: usize) -> Self {
        Self::LockSurface {
            lock,
            color: Some(LOCK_BGRA),
        }
    }
}

/// What a client answers with.
enum Ack {
    Done,
    Report(Report),
    ImeStatus(ImeReport),
}

/// Everything one client knows about its IME half: whether the compositor
/// told its input method it is active, and which popup frame callbacks have
/// arrived since they were last armed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ImeReport {
    activated: bool,
    ime_frame: bool,
    xdg_frames: Vec<u32>,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    output: Option<wl_output::WlOutput>,
    /// A second bind of the same `wl_output` global, bound up front so a
    /// step can name the one physical output through a different resource.
    /// Smithay's one-surface-per-output guard keys on resource identity, so
    /// this is the only way several lock surfaces can exist on one output
    /// (see `docs/backlog/protocols/lock-surface-duplicate-wl-output.md`,
    /// which owns the admission question; the per-output suite owns what
    /// happens to every surface once admitted).
    output2: Option<wl_output::WlOutput>,
    /// The `wl_output` global's name and version, kept so the second bind
    /// above can be made after the initial roundtrip.
    output_global: Option<(u32, u32)>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard_focus: Option<wl_surface::WlSurface>,
    pointer_focus: Option<wl_surface::WlSurface>,
    keys: u32,
    buttons: u32,
    closes: u32,
    locked: u32,
    finished: u32,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
    /// The size and serial the compositor configured each lock surface to,
    /// by creation order.
    lock_configures: Vec<Option<(u32, u32, u32)>>,
    /// ...and the same for each layer surface.
    layer_configures: Vec<Option<(u32, u32)>>,
    /// ...and the serial of each xdg popup's latest unacked configure.
    popup_configures: Vec<Option<u32>>,
    text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    input_method_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    input_method: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    /// Whether the compositor told the input method it is now active.
    activated: bool,
    /// The IME popup's surface, once [`Step::CreateImePopup`] ran.
    ime_popup: Option<wl_surface::WlSurface>,
    /// Its in-flight frame callback, kept alive so the compositor's answer
    /// has somewhere to arrive.
    ime_frame: Option<wl_callback::WlCallback>,
    /// Whether that callback's `done` arrived since it was last armed.
    ime_frame_done: bool,
    /// One in-flight frame callback per xdg popup, and how many `done`
    /// events each has collected.
    xdg_frames: Vec<Option<wl_callback::WlCallback>>,
    xdg_frame_dones: Vec<u32>,
}

/// Which `wl_callback.done` arrived: the IME popup's, or an xdg popup's by
/// creation order.
#[derive(Clone, Copy)]
enum FrameWhich {
    Ime,
    Xdg(usize),
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
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "wl_output" => {
                client.output_global = Some((name, version));
                if client.output.is_none() {
                    client.output = Some(registry.bind(name, version.min(3), qh, ()))
                }
            }
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "zwp_text_input_manager_v3" => {
                client.text_input_manager = Some(registry.bind(name, version.min(1), qh, ()));
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
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && client.keyboard.is_none() {
                client.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && client.pointer.is_none() {
                client.pointer = Some(seat.get_pointer(qh, ()));
            }
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
            wl_keyboard::Event::Enter { surface, .. } => client.keyboard_focus = Some(surface),
            wl_keyboard::Event::Leave { .. } => client.keyboard_focus = None,
            wl_keyboard::Event::Key { .. } => client.keys += 1,
            _ => {}
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
            wl_pointer::Event::Button { .. } => client.buttons += 1,
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The one event that matters here: a `CloseFocused` action reaching
        // this window is exactly what must not happen from behind a lock.
        if let xdg_toplevel::Event::Close = event {
            client.closes += 1;
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => client.locked += 1,
            ext_session_lock_v1::Event::Finished => client.finished += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, SurfaceIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        index: &SurfaceIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
            && let Some(slot) = client.lock_configures.get_mut(index.0)
        {
            *slot = Some((serial, width, height));
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
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            if let Some(slot) = client.layer_configures.get_mut(index.0) {
                *slot = Some((width, height));
            }
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => client.activated = true,
            zwp_input_method_v2::Event::Deactivate => client.activated = false,
            _ => {}
        }
    }
}

/// An xdg popup's own index in creation order, distinct from
/// [`SurfaceIndex`] so a popup configure lands in [`TestClient::popup_configures`].
struct PopupIndex(usize);

impl Dispatch<xdg_surface::XdgSurface, PopupIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &PopupIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(slot) = client.popup_configures.get_mut(index.0)
        {
            *slot = Some(serial);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, FrameWhich> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        which: &FrameWhich,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            match *which {
                FrameWhich::Ime => client.ime_frame_done = true,
                FrameWhich::Xdg(index) => {
                    if let Some(slot) = client.xdg_frame_dones.get_mut(index) {
                        *slot += 1;
                    }
                }
            }
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);
wayland_client::delegate_noop!(TestClient: ignore xdg_popup::XdgPopup);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
wayland_client::delegate_noop!(TestClient: ignore zwp_text_input_v3::ZwpTextInputV3);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2
);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2
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
    let fd = rustix::fs::memfd_create("flexwm-lock-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// One `get_lock_surface` naming `output`, acked and optionally drawn: the
/// shared body of [`Step::LockSurface`] and [`Step::LockSurfaceSecondBind`],
/// which differ only in which bind of the one output they name.
#[allow(clippy::too_many_arguments)]
fn lock_surface_step(
    client: &mut TestClient,
    queue: &mut wayland_client::EventQueue<TestClient>,
    compositor: &wl_compositor::WlCompositor,
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    locks: &[ext_session_lock_v1::ExtSessionLockV1],
    lock_surfaces: &mut Vec<(
        wl_surface::WlSurface,
        ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    )>,
    lock: usize,
    output: wl_output::WlOutput,
    color: Option<[u8; 4]>,
) -> Result<(), String> {
    let lock = locks.get(lock).cloned().ok_or("no such lock")?;
    let surface = compositor.create_surface(qh, ());
    let index = lock_surfaces.len();
    client.lock_configures.push(None);
    let lock_surface = lock.get_lock_surface(&surface, &output, qh, SurfaceIndex(index));
    // The first configure is sent on binding the interface, and
    // its size is an *exact* requirement for the first buffer.
    let (serial, width, height) = wait_for(queue, client, "a lock configure", |client| {
        client.lock_configures[index]
    })?;
    lock_surface.ack_configure(serial);
    if let Some(color) = color {
        let buffer = solid_buffer(shm, qh, width as i32, height as i32, color);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
    }
    surface.commit();
    lock_surfaces.push((surface, lock_surface));
    Ok(())
}

/// Runs one client half: binds the globals, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    // The second bind of the same global, made up front: the bind request
    // is flushed by the roundtrips every step starts and ends with, so by
    // the time any step names `output2` the compositor has seen it.
    if let Some((name, version)) = client.output_global {
        client.output2 = Some(registry.bind(name, version.min(3), &qh, ()));
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    }

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let manager = client
        .lock_manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1 -- the global is missing")?;
    let output = client.output.clone().ok_or("no wl_output")?;
    let output2 = client.output2.clone().ok_or("no second wl_output")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;

    let mut windows: Vec<wl_surface::WlSurface> = Vec::new();
    // The `xdg_surface` under each window, so a popup can name its parent.
    let mut toplevels: Vec<xdg_surface::XdgSurface> = Vec::new();
    // Mapped xdg popups: surface, `xdg_surface` and `xdg_popup`.
    let mut xdg_popups: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_popup::XdgPopup,
    )> = Vec::new();
    let mut locks: Vec<ext_session_lock_v1::ExtSessionLockV1> = Vec::new();
    let mut lock_surfaces: Vec<(
        wl_surface::WlSurface,
        ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    )> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match step {
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                let index = client.window_serials.len();
                client.window_serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceIndex(index));
                let _toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "a toplevel configure", |client| {
                    client.window_serials[index]
                })?;
                xdg.ack_configure(serial);
                let buffer = solid_buffer(&shm, &qh, WINDOW_BUFFER, WINDOW_BUFFER, WINDOW_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, WINDOW_BUFFER, WINDOW_BUFFER);
                surface.commit();
                windows.push(surface);
                toplevels.push(xdg);
            }
            Step::MapXdgPopup { window } => {
                let parent = toplevels.get(window).cloned().ok_or("no such toplevel")?;
                let surface = compositor.create_surface(&qh, ());
                let index = client.popup_configures.len();
                client.popup_configures.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, PopupIndex(index));
                let positioner = wm_base.create_positioner(&qh, ());
                // Both are required before `get_popup`, or the compositor
                // rightly answers with `invalid_positioner`.
                positioner.set_size(POPUP_BUFFER, POPUP_BUFFER);
                positioner.set_anchor_rect(0, 0, 10, 10);
                let popup = xdg.get_popup(Some(&parent), &positioner, &qh, ());
                positioner.destroy();
                surface.commit();
                let serial = wait_for(
                    &mut queue,
                    &mut client,
                    "an xdg popup configure",
                    |client| client.popup_configures[index],
                )?;
                xdg.ack_configure(serial);
                let buffer = solid_buffer(&shm, &qh, POPUP_BUFFER, POPUP_BUFFER, XDG_POPUP_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, POPUP_BUFFER, POPUP_BUFFER);
                surface.commit();
                client.xdg_frames.push(None);
                client.xdg_frame_dones.push(0);
                xdg_popups.push((surface, xdg, popup));
            }
            Step::SetupIme => {
                let text_inputs = client
                    .text_input_manager
                    .clone()
                    .ok_or("no zwp_text_input_manager_v3")?;
                let input_methods = client
                    .input_method_manager
                    .clone()
                    .ok_or("no zwp_input_method_manager_v2")?;
                client.text_input = Some(text_inputs.get_text_input(&seat, &qh, ()));
                client.input_method = Some(input_methods.get_input_method(&seat, &qh, ()));
            }
            Step::EnableTextInput => {
                let text_input = client.text_input.clone().ok_or("no text input")?;
                text_input.enable();
                text_input.commit();
            }
            Step::DisableTextInput => {
                let text_input = client.text_input.clone().ok_or("no text input")?;
                text_input.disable();
                text_input.commit();
            }
            Step::SetCursorRectangle { x, y, w, h } => {
                let text_input = client.text_input.clone().ok_or("no text input")?;
                text_input.set_cursor_rectangle(x, y, w, h);
                text_input.commit();
            }
            Step::CreateImePopup => {
                let input_method = client.input_method.clone().ok_or("no input method")?;
                let popup = compositor.create_surface(&qh, ());
                input_method.get_input_popup_surface(&popup, &qh, ());
                let buffer = solid_buffer(&shm, &qh, POPUP_BUFFER, POPUP_BUFFER, IME_BGRA);
                popup.attach(Some(&buffer), 0, 0);
                popup.damage(0, 0, POPUP_BUFFER, POPUP_BUFFER);
                popup.commit();
                client.ime_popup = Some(popup);
            }
            Step::RequestImeFrame => {
                let popup = client.ime_popup.clone().ok_or("no IME popup")?;
                client.ime_frame_done = false;
                client.ime_frame = Some(popup.frame(&qh, FrameWhich::Ime));
                // The callback lands in the surface's *pending* state; only a
                // commit moves it to current, where the compositor's frame
                // pass drains it. A bare commit: no buffer, no damage, legal
                // on both popup roles.
                popup.commit();
            }
            Step::RequestXdgPopupFrame { popup } => {
                let (surface, _, _) = xdg_popups.get(popup).ok_or("no such xdg popup")?;
                client.xdg_frame_dones[popup] = 0;
                client.xdg_frames[popup] = Some(surface.frame(&qh, FrameWhich::Xdg(popup)));
                surface.commit();
            }
            Step::ImeStatus => {
                outcome = Ack::ImeStatus(ImeReport {
                    activated: client.activated,
                    ime_frame: client.ime_frame_done,
                    xdg_frames: client.xdg_frame_dones.clone(),
                });
            }
            Step::Lock => {
                let seen = client.locked + client.finished;
                let lock = manager.lock(&qh, ());
                locks.push(lock);
                // Exactly one of the two must arrive, and the protocol says
                // so in as many words: "In response to the creation of this
                // object the compositor must send either the locked or
                // finished event."
                wait_for(&mut queue, &mut client, "locked or finished", |client| {
                    (client.locked + client.finished > seen).then_some(())
                })?;
            }
            Step::LockSurface { lock, color } => {
                lock_surface_step(
                    &mut client,
                    &mut queue,
                    &compositor,
                    &shm,
                    &qh,
                    &locks,
                    &mut lock_surfaces,
                    lock,
                    output.clone(),
                    color,
                )?;
            }
            Step::LockSurfaceSecondBind { lock, color } => {
                lock_surface_step(
                    &mut client,
                    &mut queue,
                    &compositor,
                    &shm,
                    &qh,
                    &locks,
                    &mut lock_surfaces,
                    lock,
                    output2.clone(),
                    color,
                )?;
            }
            Step::MapOverlayLayer => {
                let layer_shell = client.layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?;
                let surface = compositor.create_surface(&qh, ());
                let index = client.layer_configures.len();
                client.layer_configures.push(None);
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    zwlr_layer_shell_v1::Layer::Overlay,
                    "flexwm-lock-test".into(),
                    &qh,
                    SurfaceIndex(index),
                );
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Bottom
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer.set_size(0, 0);
                // Reserve nothing: this is here to be *drawn*, not to shrink
                // the layout.
                layer.set_exclusive_zone(-1);
                surface.commit();
                let (width, height) =
                    wait_for(&mut queue, &mut client, "a layer configure", |client| {
                        client.layer_configures[index]
                    })?;
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, OVERLAY_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
            }
            Step::DestroyLockSurface { index } => {
                let (surface, lock_surface) =
                    lock_surfaces.get(index).ok_or("no such lock surface")?;
                lock_surface.destroy();
                surface.destroy();
            }
            Step::DestroyLockRoleOnly { index } => {
                let (_, lock_surface) = lock_surfaces.get(index).ok_or("no such lock surface")?;
                lock_surface.destroy();
            }
            Step::CommitLockSurface { index } => {
                let (surface, _) = lock_surfaces.get(index).ok_or("no such lock surface")?;
                surface.commit();
            }
            Step::NullCommitLockSurface { index } => {
                let (surface, _) = lock_surfaces.get(index).ok_or("no such lock surface")?;
                surface.attach(None, 0, 0);
                surface.commit();
            }
            Step::ReattachLockBuffer { index } => {
                let (surface, _) = lock_surfaces.get(index).ok_or("no such lock surface")?;
                let (_, width, height) =
                    client.lock_configures.get(index).copied().flatten().ok_or(
                        "the lock surface was never configured, so there is no size to redraw at",
                    )?;
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, LOCK_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
            }
            Step::RedrawLockSurface { index } => {
                let (surface, lock_surface) =
                    lock_surfaces.get(index).ok_or("no such lock surface")?;
                let (serial, width, height) =
                    client.lock_configures.get(index).copied().flatten().ok_or(
                        "the lock surface was never configured, so there is no size to redraw at",
                    )?;
                lock_surface.ack_configure(serial);
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, LOCK_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
            }
            Step::LockSurfaceNoAckWithBuffer { lock } => {
                let lock = locks.get(lock).cloned().ok_or("no such lock")?;
                let surface = compositor.create_surface(&qh, ());
                let index = lock_surfaces.len();
                client.lock_configures.push(None);
                let lock_surface =
                    lock.get_lock_surface(&surface, &output, &qh, SurfaceIndex(index));
                let (_, width, height) =
                    wait_for(&mut queue, &mut client, "a lock configure", |client| {
                        client.lock_configures[index]
                    })?;
                // Deliberately no ack: the configure's size is still an exact
                // requirement, so the buffer below matches -- the only thing
                // wrong with this commit is the missing ack.
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, LOCK_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
                lock_surfaces.push((surface, lock_surface));
            }
            Step::LockNoWait => {
                let lock = manager.lock(&qh, ());
                locks.push(lock);
            }
            Step::AttackLock => {
                let lock = manager.lock(&qh, ());
                let surface = compositor.create_surface(&qh, ());
                let index = lock_surfaces.len();
                client.lock_configures.push(None);
                let lock_surface =
                    lock.get_lock_surface(&surface, &output, &qh, SurfaceIndex(index));
                lock.destroy();
                lock_surfaces.push((surface, lock_surface));
                locks.push(lock);
            }
            Step::DestroyLock { lock } => {
                let lock = locks.get(lock).ok_or("no such lock")?;
                lock.destroy();
            }
            Step::Unlock { lock } => {
                let lock = locks.get(lock).ok_or("no such lock")?;
                lock.unlock_and_destroy();
            }
            Step::Report => {
                let which = |surface: &wl_surface::WlSurface| {
                    if let Some(index) = windows.iter().position(|s| s == surface) {
                        Which::Window(index)
                    } else if let Some(index) = lock_surfaces.iter().position(|(s, _)| s == surface)
                    {
                        Which::Lock(index)
                    } else {
                        Which::Other
                    }
                };
                outcome = Ack::Report(Report {
                    keyboard_focus: client.keyboard_focus.as_ref().map(&which),
                    keys: client.keys,
                    pointer_focus: client.pointer_focus.as_ref().map(&which),
                    buttons: client.buttons,
                    closes: client.closes,
                    locked: client.locked,
                    finished: client.finished,
                });
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one or more connected
/// clients, each scripted a step at a time. See
/// [`crate::compositor::test_support`] for everything that is not specific to
/// this protocol.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.connect();
        fixture
    }

    /// Connects another client, returning its index.
    ///
    /// More than one is not a nicety here: taking over an abandoned lock is
    /// by definition something a *different* connection does, after the first
    /// one has gone.
    fn connect(&mut self) -> usize {
        self.spawn(run_client)
    }

    /// What client 0 (or `client`) has been told so far.
    fn report(&mut self) -> Report {
        self.report_of(0)
    }

    fn report_of(&mut self, client: usize) -> Report {
        let Ack::Report(report) = self.run_on(client, Step::Report) else {
            panic!("the report step should report");
        };
        report
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}

/// Asserts every pixel of the frame is `color` -- the assertion that actually
/// says "nothing else is on screen", as opposed to sampling a few points a
/// leak could sit between.
fn assert_whole_screen_is(pixels: &[u8], color: [u8; 4], what: &str) {
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let found = pixel(pixels, x, y);
            assert_eq!(
                found, color,
                "{what}: pixel ({x}, {y}) is {found:?}, expected {color:?}"
            );
        }
    }
}
