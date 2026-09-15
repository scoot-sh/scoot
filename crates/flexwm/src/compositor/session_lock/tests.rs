//! Tests for `ext-session-lock-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `ext_session_lock_manager_v1`, `xdg_wm_base`, `wl_seat` and `wl_output`
//! exactly as `swaylock` does -- through a real [`State`] with a real
//! `headless` backend, then render with the real [`PixmanRenderer`] and read
//! the framebuffer back. That is deliberate, and the same choice
//! `layer_shell/tests.rs` and `cursor/tests.rs` made for the same reason, but
//! it matters more here than anywhere else in this compositor: the claim
//! under test is "nothing that was on screen before the lock is on screen
//! after it", and that claim is about *pixels*. A test that asserted on which
//! enum variant the render path chose would pass just as happily against a
//! version that drew the window behind a transparent backdrop.
//!
//! The same goes for input: every focus assertion below is made from what the
//! *client* was told (`wl_keyboard.enter`, `wl_pointer.button`,
//! `xdg_toplevel.close`), never from a field inside the compositor, because
//! "the window did not receive that keystroke" is a claim about the wire.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use flexwm_ipc::{KeyCombo, Modifier, PointerButton, Request, Response};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::Rectangle;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::State;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless::{self, Backend};
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

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
    /// `get_lock_surface` for the `index`-th lock object, then attach a
    /// buffer and commit *without* acking the configure -- the by-design
    /// `CommitBeforeFirstAck` kill, which must keep working.
    LockSurfaceNoAckWithBuffer { lock: usize },
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
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    output: Option<wl_output::WlOutput>,
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
            "wl_output" => client.output = Some(registry.bind(name, version.min(3), qh, ())),
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
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

/// Round-trips until the compositor has answered, or gives up.
///
/// Bounded by a *deadline*, not by a number of round trips, and that is not
/// interchangeable here: `locked` is sent from the render loop once a blanked
/// frame has been drawn, which is a 16ms frame tick away
/// (`headless::FRAME_INTERVAL`), while a hundred round trips against a
/// compositor being dispatched on another thread finish in a fraction of
/// that. Counting round trips would make this give up before the frame it is
/// waiting for could possibly have happened -- which it did, until this was a
/// deadline.
fn wait_for<T>(
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    what: &str,
    ready: impl Fn(&TestClient) -> Option<T>,
) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        if let Some(value) = ready(client) {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(format!("the compositor never sent {what}"));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// Runs one client half: binds the globals, then executes whatever steps the
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
    let manager = client
        .lock_manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1 -- the global is missing")?;
    let output = client.output.clone().ok_or("no wl_output")?;

    let mut windows: Vec<wl_surface::WlSurface> = Vec::new();
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
                let lock = locks.get(lock).cloned().ok_or("no such lock")?;
                let surface = compositor.create_surface(&qh, ());
                let index = lock_surfaces.len();
                client.lock_configures.push(None);
                let lock_surface =
                    lock.get_lock_surface(&surface, &output, &qh, SurfaceIndex(index));
                // The first configure is sent on binding the interface, and
                // its size is an *exact* requirement for the first buffer.
                let (serial, width, height) =
                    wait_for(&mut queue, &mut client, "a lock configure", |client| {
                        client.lock_configures[index]
                    })?;
                lock_surface.ack_configure(serial);
                if let Some(color) = color {
                    let buffer = solid_buffer(&shm, &qh, width as i32, height as i32, color);
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage(0, 0, width as i32, height as i32);
                }
                surface.commit();
                lock_surfaces.push((surface, lock_surface));
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

/// One connected client's end of the fixture.
struct ClientHandle {
    steps: Option<Sender<Step>>,
    acks: Receiver<Ack>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

/// A live compositor with a real headless backend and one or more connected
/// clients, each scripted a step at a time.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    clients: Vec<ClientHandle>,
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

        let mut fixture = Self {
            event_loop,
            state,
            clients: Vec::new(),
        };
        fixture.connect();
        fixture
    }

    /// Connects another client, returning its index.
    ///
    /// More than one is not a nicety here: taking over an abandoned lock is
    /// by definition something a *different* connection does, after the first
    /// one has gone.
    fn connect(&mut self) -> usize {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");
        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let thread = thread::spawn(move || run_client(client_end, step_rx, ack_tx));
        self.clients.push(ClientHandle {
            steps: Some(step_tx),
            acks: ack_rx,
            thread: Some(thread),
        });
        self.clients.len() - 1
    }

    /// Runs one step on client 0 to completion, then lets the compositor
    /// settle.
    fn run(&mut self, step: Step) -> Ack {
        self.run_on(0, step)
    }

    fn run_on(&mut self, client: usize, step: Step) -> Ack {
        self.clients[client]
            .steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let ack = self.wait_for_ack(client);
        self.settle();
        ack
    }

    /// Dispatches until `client` answers.
    ///
    /// A client that died instead of answering is reported with *its own*
    /// error (the protocol error it provoked, usually), not as a timeout.
    fn wait_for_ack(&mut self, client: usize) -> Ack {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.clients[client].acks.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self.clients[client]
                        .thread
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("client {client} stopped while waiting for a step: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client {client}; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// Sends a step the client is expected *not* to survive -- a request the
    /// compositor answers with a protocol error -- and dispatches until the
    /// client thread has gone, handing back its own error.
    fn run_expecting_disconnect(&mut self, step: Step) -> String {
        self.clients[0]
            .steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.clients[0].acks.try_recv() {
                Ok(_) => panic!("the client survived a request that should have been refused"),
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client 0 to be disconnected"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let error = self.clients[0]
            .thread
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

    /// Disconnects a client and waits for the compositor to notice -- the
    /// "the lock client died" case, as far as the compositor can tell.
    fn disconnect(&mut self, client: usize) {
        drop(self.clients[client].steps.take());
        if let Some(handle) = self.clients[client].thread.take() {
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
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

    /// Dispatches for `duration` without asking for anything, so the frame
    /// timer gets to run on its own -- which is the only way to test that the
    /// compositor redraws *by itself*.
    fn tick(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// Renders a frame and hands back its raw BGRA pixels.
    fn render(&mut self) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        self.pixels()
    }

    /// Reads the framebuffer back without rendering first -- what a
    /// screenshot would see of whatever is already there.
    fn pixels(&mut self) -> Vec<u8> {
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Without this a panicking test leaves client threads blocked on
        // `steps.recv()` forever, and the test binary never exits.
        for client in &mut self.clients {
            drop(client.steps.take());
        }
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    let index = ((y * CANVAS + x) * 4) as usize;
    pixels[index..index + 4].try_into().expect("a BGRA pixel")
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

/// Whether any pixel of the frame is `color`.
fn contains(pixels: &[u8], color: [u8; 4]) -> bool {
    (0..CANVAS).any(|y| (0..CANVAS).any(|x| pixel(pixels, x, y) == color))
}

// -- what is on screen ---------------------------------------------------

/// The headline guarantee: once locked, a window that was on screen is not
/// merely covered, it is not drawn at all -- and neither is the focus ring,
/// nor the configured desktop background, which is what an unlocked frame
/// clears to.
#[test]
fn locking_blanks_a_window_off_the_screen() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.render();
    assert!(
        contains(&before, WINDOW_BGRA),
        "the window should be on screen before the lock"
    );

    fixture.run(Step::Lock);
    let after = fixture.render();
    assert_whole_screen_is(
        &after,
        BLACK_BGRA,
        "a locked frame with no lock surface yet",
    );
}

/// The motivating case for this whole protocol: a layer-shell surface on the
/// `overlay` layer draws in front of every window, so "the lock screen is
/// drawn last" would not be enough -- it has to not be drawn at all.
#[test]
fn locking_blanks_layer_surfaces_too() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapOverlayLayer);
    let before = fixture.render();
    assert!(
        contains(&before, OVERLAY_BGRA),
        "the overlay layer surface should be on screen before the lock"
    );

    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let after = fixture.render();
    assert_whole_screen_is(&after, LOCK_BGRA, "a locked frame over an overlay layer");
}

/// The same, with a lock surface that has drawn: its pixels, and only its
/// pixels.
#[test]
fn only_the_lock_surface_is_drawn_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "a locked frame with a lock surface");
}

/// A lock surface that has acked its configure but never attached a buffer
/// must not leave whatever was underneath showing through the gap.
#[test]
fn a_lock_surface_with_no_buffer_yet_shows_the_backdrop() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurface {
        lock: 0,
        color: None,
    });
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, BLACK_BGRA, "a lock surface with no buffer");
}

/// The protocol's own words: "If a lock surface on an active output is
/// destroyed before the ext_session_lock_v1.unlock_and_destroy event is sent,
/// the compositor must fall back to rendering a solid color."
///
/// Asserted on the frame the compositor drew *by itself* -- [`Fixture::tick`]
/// then [`Fixture::pixels`], never [`Fixture::render`] -- because "falls back"
/// is a claim about what is on the display, and a fallback that waited for an
/// unrelated pointer motion to mark the screen dirty would pass a
/// `render()`-based test while leaving the destroyed surface's pixels up.
#[test]
fn destroying_a_lock_surface_falls_back_to_a_solid_color() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    assert!(contains(&fixture.render(), LOCK_BGRA));

    fixture.run(Step::DestroyLockSurface { index: 0 });
    fixture.tick(Duration::from_millis(120));
    let pixels = fixture.pixels();
    assert_whole_screen_is(&pixels, BLACK_BGRA, "after the lock surface was destroyed");
}

/// The same protocol sentence, for the destruction that reaches no other hook:
/// only the `ext_session_lock_surface_v1` role object goes, and the
/// `wl_surface` under it, the lock and the connection all stay.
///
/// That is legal, and it is what a locker does when an output is removed under
/// it -- so "the compositor must fall back to rendering a solid color" applies
/// with nothing else changing to prompt a redraw. Before `dispatch.rs`'s
/// `destroyed` hook existed this left the destroyed surface's last frame on
/// screen indefinitely: Smithay resets the surface's `last_acked` (so it stops
/// producing render elements) but nothing asked for the frame that would show
/// it gone, and no `wl_surface` or lock object destruction ran to ask on its
/// behalf.
#[test]
fn destroying_only_the_lock_surfaces_role_falls_back_without_waiting_for_damage() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    assert_whole_screen_is(&fixture.render(), LOCK_BGRA, "the lock surface");

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    // A whole frame period with nothing else going on: no pointer motion, no
    // commit, no colour change -- the damage a stale frame would otherwise be
    // waiting for.
    fixture.tick(Duration::from_millis(120));
    let unasked = fixture.pixels();
    assert!(
        fixture.state.session_lock.is_locked() && !fixture.state.session_lock.abandoned(),
        "the lock itself is untouched: only its surface's role object went"
    );
    assert_whole_screen_is(
        &unasked,
        BLACK_BGRA,
        "the frame the compositor drew by itself after the role object was destroyed",
    );
}

/// A commit on the surviving `wl_surface` after its lock role was destroyed
/// is a safe no-op, not a client kill.
///
/// This used to post `CommitBeforeFirstAck` and drop the connection (the
/// lock-surface half of the post-unlock kill -- see
/// `docs/backlog/resolved/session-lock-post-destroy-commit-resolved.md`): Smithay's
/// destruction handler resets `last_acked`, and the next commit validated
/// against the reset. A real Quickshell client (Noctalia 4.7.7 / quickshell
/// 0.3.1) sends exactly this teardown on every unlock, so every screen
/// unlock cost the user their entire shell. The fix restores the acked
/// configure the reset dropped before the commit is delegated; see
/// `session_lock.rs`'s `prepare_post_destroy_lock_commit`.
#[test]
fn committing_a_lock_surface_after_its_role_was_destroyed_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    // The commit a teardown sends on the surface it kept alive.
    fixture.run(Step::CommitLockSurface { index: 0 });
    // The client must still be alive to answer this at all.
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the lock is still held by a live client");
    assert_eq!(report.finished, 0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session is still locked"
    );
    fixture.render();
    // ...and the lock still unlocks afterwards.
    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
}

/// The null half of the same teardown: `destroy` the role, null-attach,
/// commit -- what stock quickshell sends on every unlock, and what used to
/// die with `NullBuffer` on top of the `CommitBeforeFirstAck` above.
#[test]
fn null_commit_after_lock_role_destroy_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the client survived its own teardown");
    assert_eq!(report.finished, 0);
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the destroyed lock surface must not come back with the session"
    );
}

/// The full real-world order: the unlock itself goes through first (which
/// clears the compositor's surface list), and *then* the trailing role
/// destroy plus null commit arrives on a surface flexwm has forgotten. The
/// ack record has to outlive the surface list for exactly this reason.
#[test]
fn unlock_then_role_destroy_then_null_commit_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    // Still alive to answer: the unlock must not cost the client its shell.
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the destroyed lock surface must stay gone"
    );
}

/// A same-size buffer committed after the role destroy maps the surface
/// again rather than killing the client. Only its own locker's pixels on
/// its own lock screen -- every read still goes through `current` -- so
/// this is cosmetically odd, not a hole; a wrong-size one still dies with
/// `DimensionsMismatch`.
#[test]
fn commit_with_buffer_after_lock_role_destroy_survives() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::ReattachLockBuffer { index: 0 });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client survived redrawing after the destroy"
    );
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
}

/// Two commits after one destroy -- bare, then null. The second one is what
/// the `role_destroyed` flag is for: the first restore puts a value back
/// into `last_acked`, so only the flag still tells a destroyed role from a
/// live one.
#[test]
fn repeated_commits_after_lock_role_destroy_survive() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::CommitLockSurface { index: 0 });
    fixture.run(Step::NullCommitLockSurface { index: 0 });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client survived both trailing commits"
    );
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
}

/// The by-design half the carve-out must not touch: attaching a buffer and
/// committing *before* the first ack still kills the client. No ack was ever
/// recorded, so the interception leaves the commit alone to die loudly.
#[test]
fn committing_content_before_any_ack_still_kills_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let error = fixture.run_expecting_disconnect(Step::LockSurfaceNoAckWithBuffer { lock: 0 });
    assert!(
        error.contains("Broken pipe") || error.contains("Protocol error"),
        "the client should be dead: {error}"
    );
    // ...and only the client: the session stays locked and the compositor
    // keeps serving, which is what makes this a client kill rather than a
    // compositor crash.
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker must not unlock the session"
    );
    fixture.render();
}

/// The other by-design half: a null commit on a surface whose role is still
/// alive -- a mapped lock surface unmapping itself, which the protocol
/// forbids -- still dies with `NullBuffer`. The interception only reaches
/// destroyed-role surfaces, so a live role validates exactly as before.
#[test]
fn null_commit_on_a_live_mapped_lock_surface_still_kills_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    let error = fixture.run_expecting_disconnect(Step::NullCommitLockSurface { index: 0 });
    assert!(
        error.contains("Broken pipe") || error.contains("Protocol error"),
        "the client should be dead: {error}"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker must not unlock the session"
    );
    fixture.render();
}

/// A teardown commit while locked, followed by the client dying outright:
/// the session stays locked and reads as abandoned, same as any other dead
/// locker.
#[test]
fn commit_after_role_destroy_then_abandon_stays_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.run(Step::CommitLockSurface { index: 0 });
    assert!(fixture.state.session_lock.is_locked());

    fixture.disconnect(0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session must stay locked when the lock client disconnects"
    );
    assert!(
        fixture.state.session_lock.abandoned(),
        "and it must read as abandoned"
    );
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, RED_BGRA, "an abandoned lock");
}

/// Lock/unlock cycles in quick succession, mapping a surface every round:
/// the ack record grows with live surfaces and the session comes back
/// clean every time.
#[test]
fn rapid_lock_unlock_cycles_hold() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    for round in 0..5 {
        fixture.run(Step::Lock);
        fixture.run(Step::map_lock_surface(round));
        assert_whole_screen_is(
            &fixture.render(),
            LOCK_BGRA,
            &format!("round {round}: the lock surface"),
        );
        // Each `unlock_and_destroy` consumes its lock object, so the next
        // round uses the next one the client made.
        fixture.run(Step::Unlock { lock: round });
        assert!(
            !fixture.state.session_lock.is_locked(),
            "round {round}: the session should be unlocked"
        );
    }
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after the cycles"
    );
}

/// A clean unlock followed by a fresh lock from a different client: the
/// ordinary relock, distinct from the abandoned-lock takeover. The previous
/// locker's ack record must not leak into the new lock's validation.
#[test]
fn unlock_then_relock_with_a_new_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    fixture.run_on(second, Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the new client's lock surface");
    let report = fixture.report_of(second);
    assert_eq!(report.locked, 1);
    assert_eq!(report.finished, 0);

    fixture.run_on(second, Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after the second unlock"
    );
}

/// ...and the lock is still a real lock afterwards: the client that destroyed
/// its own surface still owns the session, can still unlock it, and the
/// orphaned `wl_surface` it kept alive does not survive that unlock.
#[test]
fn a_lock_whose_surfaces_role_was_destroyed_still_unlocks() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.tick(Duration::from_millis(120));

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    assert!(
        fixture.state.session_lock.surfaces.is_empty(),
        "the orphaned surface must not outlive the lock that made it"
    );
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the destroyed lock surface must not come back with the session"
    );
}

/// A button held down installs an implicit pointer grab (Smithay's
/// `DefaultGrab` sets a `ClickGrab` on every press), and a grab outlives focus
/// changes by design. So every lock transition has to drop it explicitly --
/// including the one that only destroys a role object, which is the shape the
/// reviewer's "hold the lock, click, drag, destroy the surface" sequence takes.
#[test]
fn a_lock_transition_drops_a_grab_a_click_left_behind() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.settle();
    assert!(
        fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .is_grabbed(),
        "a press with no release leaves the implicit click grab installed"
    );

    fixture.run(Step::DestroyLockRoleOnly { index: 0 });
    fixture.settle();
    assert!(
        !fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .is_grabbed(),
        "the grab must not survive a lock transition"
    );
}

/// Unlocking puts the session back exactly as it was, including the window
/// the client never redrew (it got no frame callbacks while locked).
#[test]
fn unlocking_brings_the_session_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.run(Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        contains(&pixels, WINDOW_BGRA),
        "the window should be back on screen after unlocking"
    );
    assert!(
        !contains(&pixels, LOCK_BGRA),
        "the lock surface must be gone after unlocking"
    );
}

// -- the `locked` event ---------------------------------------------------

/// `locked` may not be sent before a blanked frame exists, because a client
/// that suspends the machine on `locked` would otherwise race an unlocked
/// frame onto the screen.
///
/// Driven by taking the render target away rather than by timing: with no
/// backend, `render()` returns before drawing anything, which is exactly the
/// "no blanked frame yet" state, and no amount of dispatching can accidentally
/// satisfy it.
#[test]
fn the_locked_event_waits_for_a_blanked_frame() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let backend = fixture.state.backend.take().expect("a backend");

    fixture.clients[0]
        .steps
        .as_ref()
        .expect("the step channel")
        .send(Step::Lock)
        .expect("the client thread is still running");
    // The client's own `Step::Lock` gives up after 100 round trips without
    // either event, which is what this asserts: it cannot be confirmed.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
        if fixture.state.session_lock.is_locked() {
            break;
        }
    }
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session should be locked as soon as the request arrives"
    );
    // Several frame ticks' worth of dispatching, with no render target: the
    // client must still be waiting.
    for _ in 0..20 {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "the lock must not be confirmed before a frame has been drawn"
    );

    fixture.state.backend = Some(backend);
    fixture.render();
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the first drawn frame should confirm the lock"
    );
    // And now the client's own `Lock` step can finish.
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "the client should have been told `locked`"
    );
    assert_eq!(report.finished, 0);
}

/// A second lock request while a live client holds the lock is refused with
/// `finished`, not granted -- two clients must never both believe they own
/// the session.
#[test]
fn a_second_lock_is_refused_while_one_is_held() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.render();
    assert_eq!(fixture.report().locked, 1);

    fixture.run(Step::Lock);
    let report = fixture.report();
    assert_eq!(report.locked, 1, "the second lock must not be granted");
    assert_eq!(
        report.finished, 1,
        "the second lock must be told `finished`"
    );
    assert!(fixture.state.session_lock.is_locked());
}

// -- input ----------------------------------------------------------------

/// Keyboard focus moves to the lock surface and every keystroke goes there,
/// not to the window that had focus a moment earlier.
#[test]
fn the_keyboard_reaches_only_the_lock_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Window(0)),
        "the window should have the keyboard before the lock"
    );

    fixture.run(Step::Lock);
    assert_eq!(
        fixture.report().keyboard_focus,
        None,
        "the window must lose the keyboard the moment the session locks, \
         even before a lock surface exists"
    );

    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();
    assert_eq!(before.keyboard_focus, Some(Which::Lock(0)));

    fixture.state.type_text("hello").expect("typed text");
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.keyboard_focus,
        Some(Which::Lock(0)),
        "focus must not have moved"
    );
    assert!(
        after.keys > before.keys,
        "the lock surface should have received the keystrokes"
    );
}

/// Pointer focus has to be moved *at* the lock, not merely hit-tested
/// afterwards: `wl_pointer.button` goes to whatever the pointer last entered,
/// so a click after locking would otherwise land in the window underneath.
#[test]
fn a_click_while_locked_does_not_reach_the_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // Over the window's own buffer, which starts at the placement's top-left
    // corner (gap 12, ring 3).
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    assert_eq!(
        fixture.report().pointer_focus,
        Some(Which::Window(0)),
        "the pointer should be over the window before the lock"
    );

    fixture.run(Step::Lock);
    let locked = fixture.report();
    assert_eq!(
        locked.pointer_focus, None,
        "the pointer must leave the window when the session locks"
    );

    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let clicked = fixture.report();
    assert_eq!(
        clicked.buttons, locked.buttons,
        "no button event may reach a client while nothing but the backdrop is up"
    );
    assert_eq!(clicked.pointer_focus, None);

    // ...and once a lock surface is up, the same click reaches *it*.
    fixture.run(Step::map_lock_surface(0));
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let on_lock = fixture.report();
    assert_eq!(on_lock.pointer_focus, Some(Which::Lock(0)));
    assert_eq!(
        on_lock.buttons,
        clicked.buttons + 2,
        "the click should reach the lock surface"
    );
}

/// A keybinding that runs an `Action` must not fire while locked: `Super+Q`
/// is `CloseFocused` by default, and a window being told to close from behind
/// a lock screen is exactly the bypass this gate exists for. The keystroke is
/// forwarded to the lock client instead of being swallowed.
#[test]
fn an_action_keybinding_does_not_fire_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();

    let combo = KeyCombo {
        modifiers: vec![Modifier::Super],
        key: "q".into(),
    };
    fixture.state.press(&combo).expect("a pressed combo");
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.closes, before.closes,
        "the window must not be told to close from behind a lock screen"
    );
    assert!(
        after.keys > before.keys,
        "the keystroke should have been forwarded to the lock client"
    );

    // The same combo works normally once unlocked, so this is a gate rather
    // than a broken binding.
    fixture.run(Step::Unlock { lock: 0 });
    fixture.state.press(&combo).expect("a pressed combo");
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        before.closes + 1,
        "the binding should work again after unlocking"
    );
}

/// Every IPC success carries the session-lock state it was built under,
/// so an agent learns an unlock landed from the very next reply: `true`
/// while locked (injected input is still served, reaching only the lock
/// screen), `false` once unlocked.
#[test]
fn ok_replies_carry_the_locked_state() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let response = fixture
        .state
        .handle_request(Request::Type { text: "x".into() });
    assert!(
        matches!(response, Response::Ok { locked: true }),
        "input served while locked should say so, got {response:?}"
    );

    fixture.run(Step::Unlock { lock: 0 });
    let response = fixture
        .state
        .handle_request(Request::Type { text: "x".into() });
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "input served unlocked should say so, got {response:?}"
    );
}

/// An IPC `action` bypasses input entirely, so it is refused outright while
/// locked -- and works again afterwards.
#[test]
fn ipc_actions_are_refused_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let response = fixture
        .state
        .handle_request(Request::Action(flexwm_ipc::Action::CloseFocused));
    assert!(
        matches!(response, Response::Error { .. }),
        "an IPC action must be refused while locked, got {response:?}"
    );
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        0,
        "the refused action must not have reached the window"
    );

    fixture.run(Step::Unlock { lock: 0 });
    let response = fixture
        .state
        .handle_request(Request::Action(flexwm_ipc::Action::CloseFocused));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the same action should work once unlocked, got {response:?}"
    );
    fixture.settle();
    assert_eq!(fixture.report().closes, 1);
}

/// The backstop behind both of the above, and the one thing standing between
/// an `ext-workspace-v1` client and rearranging the session from behind the
/// lock screen: `State::act` itself refuses while locked, so a caller added
/// later is safe without having to remember.
#[test]
fn the_action_path_itself_is_closed_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    fixture.state.act(flexwm_core::Action::CloseFocused);
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        0,
        "an action must not reach a window from behind a lock screen, \
         whichever caller asked for it"
    );

    fixture.run(Step::Unlock { lock: 0 });
    fixture.state.act(flexwm_core::Action::CloseFocused);
    fixture.settle();
    assert_eq!(fixture.report().closes, 1);
}

// -- the lock client dying ------------------------------------------------

/// The single most safety-critical behavior here: a dead lock client is not
/// evidence that the user wants their screen unlocked.
#[test]
fn the_session_stays_locked_when_the_lock_client_dies() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();

    fixture.disconnect(0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session must stay locked when the lock client disconnects"
    );
    assert!(
        fixture.state.session_lock.abandoned(),
        "and it must read as abandoned"
    );
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, RED_BGRA, "an abandoned lock");
}

/// ...and it has to turn red *by itself*, with nothing else prompting a
/// redraw.
///
/// This is the case real `--tty` hardware caught and the other tests here
/// missed: a client that destroyed its lock surface and *then* died leaves no
/// surface destruction to hang a redraw off, so the last frame drawn -- black
/// -- stayed on the display, and the user had no way to tell their locker had
/// crashed. Deliberately reads the framebuffer without rendering first
/// (`pixels`, not `render`), because "the test asked for a frame" is exactly
/// what hid it.
#[test]
fn an_abandoned_lock_turns_red_without_being_asked_to_redraw() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // The locker is a *separate* client from the one that owns the window,
    // which is both how a real session is arranged and what makes this test
    // non-vacuous: when one client owns both, its disconnect also destroys
    // the window, and `remove_window` asks for a redraw as a side effect --
    // masking the missing one entirely. (Written the other way first; the
    // negative control below passed, which is how that was found.)
    let locker = fixture.connect();
    fixture.run_on(locker, Step::Lock);
    fixture.run_on(locker, Step::map_lock_surface(0));
    fixture.render();
    fixture.run_on(locker, Step::DestroyLockSurface { index: 0 });
    assert_whole_screen_is(
        &fixture.render(),
        BLACK_BGRA,
        "a live lock with no surface left",
    );

    fixture.disconnect(locker);
    fixture.tick(Duration::from_millis(200));
    assert_whole_screen_is(
        &fixture.pixels(),
        RED_BGRA,
        "an abandoned lock, redrawn without anyone asking",
    );
}

/// The recovery path that stops a crashed locker being a permanently unusable
/// session: a new client takes the lock over, is confirmed immediately (the
/// outputs are already blank), and can unlock after authenticating.
#[test]
fn a_new_client_can_take_over_an_abandoned_lock_and_unlock() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.render();
    fixture.disconnect(0);
    assert!(fixture.state.session_lock.abandoned());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked, 1,
        "a new client must be able to take over an abandoned lock"
    );
    assert_eq!(report.finished, 0);
    assert!(
        !fixture.state.session_lock.abandoned(),
        "the lock now has a live owner again"
    );
    // Nothing of the dead client's is on screen any more, and the backdrop is
    // back to the ordinary locked black.
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "after the takeover");

    fixture.run_on(second, Step::map_lock_surface(0));
    assert_whole_screen_is(
        &fixture.render(),
        LOCK_BGRA,
        "the new client's lock surface",
    );

    fixture.run_on(second, Step::Unlock { lock: 0 });
    assert!(!fixture.state.session_lock.is_locked());
    // The first client -- and with it its window -- is gone, so what an
    // unlocked session shows here is the ordinary desktop background. The
    // assertion that matters is that it is *not* the lock screen any more.
    assert_whole_screen_is(
        &fixture.render(),
        BACKGROUND_BGRA,
        "the session after the takeover unlocked it",
    );
}

/// A lock client that dies before its lock was ever confirmed must not leave
/// the session stuck refusing every later locker.
#[test]
fn a_replacement_locker_is_accepted_after_one_died_unconfirmed() {
    let mut fixture = Fixture::new();
    // No render between the lock and the disconnect, so the first lock is
    // still pending confirmation when its client goes away.
    fixture.clients[0]
        .steps
        .as_ref()
        .expect("the step channel")
        .send(Step::Lock)
        .expect("the client thread is still running");
    let _ = fixture.wait_for_ack(0);
    fixture.disconnect(0);
    assert!(fixture.state.session_lock.is_locked());

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    fixture.render();
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked + report.finished,
        1,
        "exactly one of the two events must arrive"
    );
    assert_eq!(
        report.locked, 1,
        "a replacement locker must not be refused because a dead one is still pending"
    );
}

// -- edges ----------------------------------------------------------------

/// Zero windows, zero lock surfaces: the screen still blanks, and stays
/// blanked across repeated lock/unlock cycles. The repeat is the point --
/// under `--tty` the damage tracker is what decides whether the framebuffer
/// is repainted at all, and a blank drawn only by the clear colour could
/// legitimately report no damage on the second cycle.
#[test]
fn locking_an_empty_session_blanks_and_keeps_blanking() {
    let mut fixture = Fixture::new();
    for round in 0..3 {
        fixture.run(Step::Lock);
        let locked = fixture.render();
        assert_whole_screen_is(&locked, BLACK_BGRA, "a locked empty session");
        // The lock object is destroyed by `unlock_and_destroy`, so each round
        // needs the next one the client made.
        fixture.run(Step::Unlock { lock: round });
        let unlocked = fixture.render();
        assert_whole_screen_is(
            &unlocked,
            BACKGROUND_BGRA,
            &format!("round {round}: the unlocked empty session"),
        );
    }
}

/// Resizing the output while locked reconfigures the lock surfaces. Without
/// that, the client's next commit is a `dimensions_mismatch` protocol error --
/// i.e. a killed lock client on a locked session.
#[test]
fn resizing_the_output_reconfigures_the_lock_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    fixture.state.resize_output(CANVAS * 2, CANVAS * 2);
    fixture.settle();
    // The client acks and redraws at whatever size it was told; a mismatch
    // would have disconnected it, which `report` reports as a dead client.
    let report = fixture.report();
    assert_eq!(report.locked, 1);
    assert!(fixture.state.session_lock.is_locked());
}

/// `wl_output` is what a lock client names its surface's screen with, and a
/// compositor that never advertised the manager global would leave a locker
/// with nothing to bind. Cheap, but it is the one thing every other test here
/// takes for granted.
#[test]
fn the_manager_global_is_advertised() {
    let mut fixture = Fixture::new();
    // `run_client` fails to start at all without it, so reaching a step at
    // all proves it -- asserted explicitly so a regression names itself.
    fixture.run(Step::MapWindow);
    assert!(!fixture.state.session_lock.is_locked());
}
// -- a lock its own client gave up ----------------------------------------
//
// The nastiest state this module has, and the one independent review found a
// critical hole in: `ext_session_lock_v1.destroy` is *legal* before `locked`
// has been sent (Smithay's `lock.rs` refuses it only while its own
// `LockStatus` says this object holds the lock, and that stays `Unlocked`
// until the confirmation actually runs). A client that uses it keeps
// everything a dying client loses -- its connection, and the `wl_surface`
// under every lock surface it made -- so "is the surface alive" stops being
// the same question as "does this surface still belong to the lock that owns
// the session". Every test below was a live reproduction before it was a
// regression test; see this module's "Which lock surfaces count".

/// The lock is given up with a *mapped* surface still up. The abandoned screen
/// has to be the red indicator, not the former locker's own pixels -- showing
/// those is worse than useless: a user looking at what appears to be a working
/// lock screen has no way to tell that their real locker never took effect.
#[test]
fn a_lock_given_up_with_a_surface_up_still_shows_the_abandoned_screen() {
    let mut fixture = Fixture::new();
    // No render target, so the lock is accepted but never confirmed -- which
    // is exactly (and only) the window in which `destroy` is legal.
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    let pixels = fixture.render();
    assert!(
        fixture.state.session_lock.abandoned(),
        "the lock reads as abandoned"
    );
    assert_whole_screen_is(&pixels, RED_BGRA, "the documented abandoned screen");
}

/// ...and the same client stops receiving input the moment it gives the lock
/// up, without waiting for anyone to replace it.
///
/// Both halves are asserted from the wire, and the pointer half is the one
/// that a hit-test-only fix would miss: `wl_pointer.button` goes to whatever
/// the pointer last *entered*, so a surface that keeps pointer focus keeps
/// receiving clicks however the hit test answers.
#[test]
fn a_lock_given_up_takes_the_keyboard_and_the_pointer_with_it() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    let held = fixture.report();
    assert_eq!(
        held.keyboard_focus,
        Some(Which::Lock(0)),
        "the locker holds the keyboard while its lock is live"
    );
    assert_eq!(held.pointer_focus, Some(Which::Lock(0)), "and the pointer");

    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    fixture.state.type_text("password").expect("typed text");
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.keyboard_focus, None,
        "a client that gave up its lock must not still hold the keyboard \
         (before={held:?} after={after:?})"
    );
    assert_eq!(after.pointer_focus, None, "nor the pointer");
    assert_eq!(
        after.keys, held.keys,
        "and no keystroke may reach it (before={held:?} after={after:?})"
    );
    assert_eq!(after.buttons, held.buttons, "nor any click");
}

/// The surface must not survive a takeover either: the replacement locker's
/// screen is its own, and so is the keyboard.
///
/// Distinct from the test above rather than a stronger version of it, because
/// the two fail to different fixes. During the abandoned-but-not-yet-replaced
/// phase above, the stale surface's lock *is* still `owner`, so an ownership
/// check alone passes it; here it is not, so a liveness check alone passes it.
/// Only asking both questions closes both.
#[test]
fn a_surface_from_a_given_up_lock_does_not_survive_a_takeover() {
    let mut fixture = Fixture::new();
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.run(Step::map_lock_surface(0));
    fixture.run(Step::DestroyLock { lock: 0 });
    fixture.state.backend = Some(backend);
    fixture.render();
    assert!(fixture.state.session_lock.abandoned());

    let b = fixture.connect();
    fixture.run_on(b, Step::Lock);
    fixture.run_on(b, Step::map_lock_surface(0));
    let pixels = fixture.render();
    let report_b = fixture.report_of(b);
    let report_a = fixture.report();
    assert_eq!(
        report_b.locked, 1,
        "the new locker was told it holds the lock"
    );
    assert_eq!(
        report_a.keyboard_focus, None,
        "the client that gave up its lock must not still hold the keyboard \
         (A={report_a:?} B={report_b:?})"
    );
    assert_eq!(
        report_a.pointer_focus, None,
        "nor the pointer (A={report_a:?} B={report_b:?})"
    );
    assert_eq!(
        report_b.keyboard_focus,
        Some(Which::Lock(0)),
        "the new locker must hold the keyboard (A={report_a:?} B={report_b:?})"
    );
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the new locker's own surface");
}

/// The whole attack, from a client that needs no race, no crash and no
/// test-only surgery: `lock`, `get_lock_surface`, `destroy`, in one batch.
///
/// Before the ownership filter existed, that left a surface registered
/// forever, and the *next* locker to run -- the user's real one -- put its
/// screen up while this client kept the keyboard. Every key of the password
/// went to the attacker, and the locker that never saw them could never
/// authenticate and so could never unlock.
#[test]
fn no_client_can_steal_the_lock_screen_by_giving_up_a_lock() {
    let mut fixture = Fixture::new();
    fixture.run(Step::AttackLock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the hostile client locked the session"
    );
    assert!(
        fixture.state.session_lock.abandoned(),
        "...and immediately gave the lock up"
    );
    // Its lock surface was never acked and never drew, so this is also the
    // unmapped half of the abandoned-screen check.
    assert_whole_screen_is(
        &fixture.render(),
        RED_BGRA,
        "an abandoned lock whose surface never drew",
    );

    // The user's real locker comes along and takes over.
    let real = fixture.connect();
    fixture.run_on(real, Step::Lock);
    fixture.run_on(real, Step::map_lock_surface(0));
    let pixels = fixture.render();
    assert_whole_screen_is(&pixels, LOCK_BGRA, "the real locker's screen");

    // Typed *after* the real lock screen is up: this is the password.
    fixture.state.type_text("password").expect("typed text");
    fixture.settle();
    let attacker = fixture.report();
    let locker = fixture.report_of(real);
    assert_eq!(
        attacker.keyboard_focus, None,
        "a client that holds no lock must not hold the lock screen's keyboard \
         (attacker={attacker:?} locker={locker:?})"
    );
    assert_eq!(
        attacker.keys, 0,
        "and must not have been sent a single keystroke \
         (attacker={attacker:?} locker={locker:?})"
    );
    assert_eq!(
        locker.keyboard_focus,
        Some(Which::Lock(0)),
        "the real locker must hold the keyboard (attacker={attacker:?} locker={locker:?})"
    );
    assert!(
        locker.keys > 0,
        "and must be the one that received the keystrokes \
         (attacker={attacker:?} locker={locker:?})"
    );
}

/// A takeover may only be confirmed without drawing a frame when the lock it
/// replaces had actually drawn one. Replace a lock that never did, and the
/// unlocked session is still on the display -- telling the new client `locked`
/// there hands it exactly the guarantee the event exists to provide at exactly
/// the moment it is false.
///
/// The first lock is left unconfirmed by taking the render target away, so no
/// timing luck is involved: with no backend no frame can be drawn at all.
#[test]
fn a_takeover_waits_for_the_blanked_frame_the_replaced_lock_never_drew() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert!(
        contains(&fixture.render(), WINDOW_BGRA),
        "the window is on screen before any lock"
    );

    let backend = fixture.state.backend.take().expect("a backend");
    fixture.run(Step::LockNoWait);
    fixture.disconnect(0);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session is locked, by a client that is now gone"
    );
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "...and its lock was never confirmed"
    );

    // The replacement's own `Lock` step blocks until the compositor answers,
    // and it must not answer yet, so it is sent by hand -- the same pattern
    // `the_locked_event_waits_for_a_blanked_frame` uses.
    let second = fixture.connect();
    fixture.clients[second]
        .steps
        .as_ref()
        .expect("the step channel")
        .send(Step::Lock)
        .expect("the client thread is still running");
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    assert!(
        fixture.state.session_lock.pending.is_some(),
        "a takeover of a lock that never drew a blanked frame must wait for one"
    );
    // Which is the entire point, stated in pixels: reading the framebuffer
    // without rendering first shows what a client told `locked` here would
    // have been told about.
    fixture.state.backend = Some(backend);
    assert!(
        contains(&fixture.pixels(), WINDOW_BGRA),
        "the unlocked session is still the last frame drawn"
    );

    // Now let the frame happen; only then may the confirmation go out.
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "the blanked frame");
    assert!(fixture.state.session_lock.pending.is_none());
    let ack = fixture.wait_for_ack(second);
    assert!(matches!(ack, Ack::Done));
    let report = fixture.report_of(second);
    assert_eq!(
        report.locked, 1,
        "the takeover is confirmed, once it is true"
    );
    assert_eq!(report.finished, 0);
}

/// The other half of that rule, so the fix above cannot have been "never
/// fast-confirm": replacing a lock that *had* drawn its blanked frame is still
/// confirmed with no further frame, because the screen genuinely is blank and
/// stays blank across the handover.
///
/// Driven the same way round as the test above -- the render target is taken
/// away *before* the second lock, so the only way this client can be told
/// `locked` at all is without a frame.
#[test]
fn a_takeover_of_a_confirmed_lock_is_still_confirmed_immediately() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Lock);
    assert_whole_screen_is(&fixture.render(), BLACK_BGRA, "the blanked frame");
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the first lock was confirmed by that frame"
    );
    let backend = fixture.state.backend.take().expect("a backend");
    fixture.disconnect(0);

    let second = fixture.connect();
    fixture.run_on(second, Step::Lock);
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "no frame is owed: the outputs were blank before this lock and are \
         blank after it"
    );
    assert_eq!(fixture.report_of(second).locked, 1);
    fixture.state.backend = Some(backend);
}
