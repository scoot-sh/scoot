//! Popup parent chains, driven through a real `wayland-client` connection:
//! how deep they may nest, and every way a client could try to make one
//! deeper, or loop it, after the fact.
//!
//! Every test asserts on what a client was told -- the protocol error that
//! ended it, a configure it received -- on real framebuffer pixels, or on a
//! *second* client still being served after the first did its worst. The
//! refusals exist to keep the compositor alive for everyone else, so that
//! last one is the claim that matters.
//!
//! The client script works in batches: each [`Step::Batch`] sends all of its
//! [`Op`]s and only then round-trips, so they reach the compositor in one
//! flush, and are dispatched in one go, the way a hostile client would send
//! them -- and the way a toolkit sends a burst of menu changes.
//!
//! Nothing here names a server-side symbol from `popup_parent.rs` (the cap
//! is spelled out as [`CAP`]), so this file compiles against the code before
//! it, which is how its tests were watched failing first.
//!
//! This file is the harness; the tests live in the submodules below, by
//! concern. Like every live-`State` suite here, these need a writable
//! `$XDG_RUNTIME_DIR`.
//!
//! The harness is shared with `subsurface_depth`'s tests, which is what its
//! `pub(in crate::compositor)` items are for: the client can also build
//! subsurface trees ([`Op::Sub`], see [`subsurfaces`]), under a window, a
//! popup or a plain surface, so one tree can hold both.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use scoot_core::{Rect, WindowId};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3;
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::{self, Harness, wait_for};

mod adopt;
mod bench;
mod bypass;
mod depth;
mod ime;
pub(in crate::compositor) mod subsurfaces;

pub(in crate::compositor) use subsurfaces::{Node, SUB_SIZE, SubOp};

/// The most popups a chain may hold -- `popup_parent::MAX_POPUP_DEPTH`,
/// spelled out so this file does not depend on it (see the module doc).
pub(in crate::compositor) const CAP: usize = 64;

/// The output's framebuffer, square.
pub(in crate::compositor) const CANVAS: i32 = 200;

// Colours, as the BGRA bytes an `Argb8888` buffer holds them in.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const BAR_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
pub(in crate::compositor) const POPUP_BGRA: [u8; 4] = [0x20, 0xE0, 0xE0, 0xFF];
/// The one popup a test is looking for.
pub(in crate::compositor) const MARKED_BGRA: [u8; 4] = [0xE0, 0x20, 0xE0, 0xFF];

/// Each popup's size, and its offset from its parent's window geometry: a
/// chain climbs one pixel right and down per level, so a 64-deep chain's
/// deepest popup sits 63 pixels in from its root's corner, still on screen.
pub(in crate::compositor) const POPUP_SIZE: i32 = 8;
pub(in crate::compositor) const STEP: i32 = 1;

pub(in crate::compositor) fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 0,
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// What a popup hangs off.
#[derive(Clone, Copy, Debug)]
pub(in crate::compositor) enum Parent {
    /// The `n`-th toplevel this client mapped.
    Window(usize),
    /// The `n`-th popup this client made.
    Popup(usize),
    /// The `n`-th bare, role-less `xdg_surface` this client made.
    Bare(usize),
    /// The `n`-th layer surface this client mapped, through
    /// `zwlr_layer_surface_v1.get_popup` on a popup created parentless.
    Layer(usize),
}

/// One request (or a few that belong together), sent without a round trip.
#[derive(Clone, Copy, Debug)]
pub(in crate::compositor) enum Op {
    /// A popup of `parent`, committed once. It becomes the next popup index.
    Popup(Parent),
    /// `len` popups, each a child of the one before it, the first a child of
    /// `parent`: the next `len` popup indices, outermost first.
    Chain { parent: Parent, len: usize },
    /// An `xdg_surface` given no role. It becomes the next bare index.
    Bare,
    /// Gives bare `xdg_surface` `bare` a popup role under `parent`, which
    /// makes it the next popup index.
    PopupOnBare { bare: usize, parent: Parent },
    /// `get_popup` a second time on popup `popup`'s own `xdg_surface`, while
    /// its first `xdg_popup` is alive.
    GetPopupAgain { popup: usize, parent: Parent },
    /// A second `xdg_surface` for popup `popup`'s `wl_surface`, and
    /// `get_popup` on that, while the first `xdg_popup` is alive.
    SecondXdgSurface { popup: usize, parent: Parent },
    /// `xdg_popup.destroy` on popup `popup`, keeping its `xdg_surface`.
    Destroy(usize),
    /// Destroys popup `popup`'s `xdg_popup` and `xdg_surface`, then makes its
    /// `wl_surface` a popup again under `parent`, through a new
    /// `xdg_surface`, as a toolkit that re-showed a menu without replacing
    /// its surface would (the protocol allows it; GTK 3 does not do it).
    /// The popup keeps its index.
    Reincarnate { popup: usize, parent: Parent },
    /// `xdg_popup.reposition` on popup `popup`: one more walk up its chain.
    Reposition(usize),
    /// A popup created with a null parent, *not* committed (committing it
    /// unadopted is a protocol error). It becomes the next popup index.
    Parentless,
    /// A popup of `parent` that is *not* committed. It becomes the next
    /// popup index.
    Uncommitted(Parent),
    /// `zwlr_layer_surface_v1.get_popup` on popup `popup`, whatever it is.
    Adopt { popup: usize, layer: usize },
    /// `wl_surface.commit` on popup `popup`'s surface.
    Commit(usize),
    /// A round trip in the middle of the batch (see [`sync`]).
    ///
    /// Put right after the request a test expects refused, when more
    /// follows it: the client writes a long batch in 4 KiB pieces
    /// (wayland-backend's `MAX_BYTES_OUT`), so a refusal early in one can
    /// close the socket under a later piece, and the client then sees
    /// `EPIPE` instead of the error. With this, the refusal is read before
    /// anything else is written -- and if the request is *not* refused, the
    /// batch carries on and the test fails on the client surviving it.
    Sync,
    /// A request on the subsurface side of the tree (see [`subsurfaces`]).
    Sub(SubOp),
}

pub(in crate::compositor) enum Step {
    /// Create a toplevel, commit without a buffer, ack the configure that
    /// answers and draw at the size it names.
    MapWindow,
    /// A top-layer bar across the top edge, 20 pixels tall, no exclusive
    /// zone.
    MapBar,
    /// Every op in order, then a single round trip. Answers `Done`, or ends
    /// the client with the protocol error it provoked.
    Batch(Vec<Op>),
    /// Waits for each listed popup's configure, acks it, and draws: in
    /// [`MARKED_BGRA`] for `marked`, [`POPUP_BGRA`] otherwise.
    Map {
        popups: Vec<usize>,
        marked: Option<usize>,
    },
}

#[derive(Debug)]
pub(in crate::compositor) enum Ack {
    Done,
}

/// Which object an event belongs to.
#[derive(Clone, Copy)]
enum Role {
    Window(usize),
    Popup(usize),
    Layer(usize),
    /// Nothing is recorded for it.
    Ignored,
}

#[derive(Default)]
struct PopupRecord {
    serial: Option<u32>,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// Bound for every client, used only by `ime.rs`'s.
    seat: Option<wl_seat::WlSeat>,
    text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    input_method_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    /// The serial of the newest `wl_keyboard.enter` (`ime.rs` only).
    keyboard_enter: Option<u32>,
    /// Whether the input method is active (`ime.rs` only).
    ime_active: bool,
    /// Per toplevel: the size its pending `xdg_toplevel.configure` named.
    pending: Vec<(i32, i32)>,
    /// Per toplevel: its newest completed configure, `(serial, width,
    /// height)`.
    configured: Vec<Option<(u32, i32, i32)>>,
    /// Per layer surface: the size of its newest (already acked) configure.
    layer_sizes: Vec<Option<(u32, u32)>>,
    popups: Vec<PopupRecord>,
    /// Whether the newest `wl_display.sync` has been answered.
    synced: bool,
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            client.synced = true;
        }
    }
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
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_subcompositor" => {
                client.subcompositor = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
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

impl Dispatch<xdg_toplevel::XdgToplevel, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (xdg_toplevel::Event::Configure { width, height, .. }, Role::Window(index)) =
            (event, *role)
            && let Some(pending) = client.pending.get_mut(index)
        {
            *pending = (width, height);
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, Role> for TestClient {
    fn event(
        _: &mut Self,
        _: &xdg_popup::XdgPopup,
        _: xdg_popup::Event,
        _: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_surface::XdgSurface, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        match *role {
            Role::Window(index) => {
                if let Some(&(width, height)) = client.pending.get(index)
                    && let Some(slot) = client.configured.get_mut(index)
                {
                    *slot = Some((serial, width, height));
                }
            }
            Role::Popup(index) => {
                if let Some(record) = client.popups.get_mut(index) {
                    record.serial = Some(serial);
                }
            }
            Role::Layer(_) | Role::Ignored => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, Role> for TestClient {
    fn event(
        client: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            },
            Role::Layer(index),
        ) = (event, *role)
        {
            surface.ack_configure(serial);
            if let Some(slot) = client.layer_sizes.get_mut(index) {
                *slot = Some((width, height));
            }
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(TestClient: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3
);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2
);

/// A `width`x`height` buffer of `color` over a real memfd. A zero size (a
/// configure that left the size to the client) draws 40 square.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> Result<(wl_buffer::WlBuffer, i32, i32), String> {
    let width = if width > 0 { width } else { 40 };
    let height = if height > 0 { height } else { 40 };
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-popup-parent-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok((buffer, width, height))
}

struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
}

struct Popup {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    popup: xdg_popup::XdgPopup,
}

struct Layer {
    // Held for the run: dropping a layer surface's objects unmaps it.
    _surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
}

/// Everything the client script made, by the order it made it -- held for
/// the run.
#[derive(Default)]
struct Made {
    windows: Vec<Toplevel>,
    popups: Vec<Popup>,
    bare: Vec<(wl_surface::WlSurface, xdg_surface::XdgSurface)>,
    layers: Vec<Layer>,
    /// Plain surfaces, subsurfaces or not ([`SubOp`]).
    surfaces: Vec<subsurfaces::Surface>,
    /// The two buffers every drawn subsurface shares, made on first use.
    sub_buffers: Option<subsurfaces::Buffers>,
}

/// The client's globals, once bound.
struct Globals {
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    subcompositor: wl_subcompositor::WlSubcompositor,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
}

impl Globals {
    /// A positioner for an [`POPUP_SIZE`]-square popup [`STEP`] pixels in
    /// from its parent's window-geometry corner, asking for no adjustment.
    fn positioner(&self, qh: &QueueHandle<TestClient>) -> xdg_positioner::XdgPositioner {
        let positioner = self.wm_base.create_positioner(qh, ());
        positioner.set_size(POPUP_SIZE, POPUP_SIZE);
        positioner.set_anchor_rect(STEP, STEP, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner
    }
}

impl Made {
    /// The parent's `xdg_surface`, or `None` for a layer surface's popup,
    /// which is created parentless and adopted afterwards.
    fn parent_xdg(&self, parent: Parent) -> Result<Option<&xdg_surface::XdgSurface>, String> {
        Ok(Some(match parent {
            Parent::Window(i) => &self.windows.get(i).ok_or("no such window")?.xdg,
            Parent::Popup(i) => &self.popups.get(i).ok_or("no such popup")?.xdg,
            Parent::Bare(i) => &self.bare.get(i).ok_or("no such bare xdg_surface")?.1,
            Parent::Layer(_) => return Ok(None),
        }))
    }

    /// `get_popup` on `xdg` for `parent`, adopting it into a layer surface
    /// if that is the parent.
    fn get_popup(
        &self,
        globals: &Globals,
        qh: &QueueHandle<TestClient>,
        xdg: &xdg_surface::XdgSurface,
        parent: Parent,
        role: Role,
    ) -> Result<xdg_popup::XdgPopup, String> {
        let positioner = globals.positioner(qh);
        let popup = xdg.get_popup(self.parent_xdg(parent)?, &positioner, qh, role);
        if let Parent::Layer(i) = parent {
            self.layers
                .get(i)
                .ok_or("no such layer")?
                .layer
                .get_popup(&popup);
        }
        positioner.destroy();
        Ok(popup)
    }

    /// A new popup of `parent`, committed once.
    fn new_popup(
        &mut self,
        client: &mut TestClient,
        globals: &Globals,
        qh: &QueueHandle<TestClient>,
        parent: Parent,
    ) -> Result<usize, String> {
        let index = self.popups.len();
        client.popups.push(PopupRecord::default());
        let surface = globals.compositor.create_surface(qh, ());
        let xdg = globals
            .wm_base
            .get_xdg_surface(&surface, qh, Role::Popup(index));
        let popup = self.get_popup(globals, qh, &xdg, parent, Role::Popup(index))?;
        surface.commit();
        self.popups.push(Popup {
            surface,
            xdg,
            popup,
        });
        Ok(index)
    }
}

/// A round trip that puts everything queued -- the requests *and* the
/// `sync` -- on the wire in one flush.
///
/// `EventQueue::roundtrip` would do, but for two things. A flush of its
/// own first, then the round trip's `sync` in a second write, races the
/// refusal: the compositor can kill the client and close the socket in
/// between, and the client then sees `EPIPE` on that second write instead
/// of the protocol error waiting in its receive buffer. And a chain
/// thousands deep is more than the socket buffer holds, which a plain flush
/// reports as `WouldBlock`; this waits that out while the compositor reads.
/// Every batch whose error a test asserts on has nothing after the refused
/// request but this `sync`, or an [`Op::Sync`].
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run_op(
    op: Op,
    client: &mut TestClient,
    globals: &Globals,
    qh: &QueueHandle<TestClient>,
    made: &mut Made,
) -> Result<(), String> {
    match op {
        Op::Popup(parent) => {
            made.new_popup(client, globals, qh, parent)?;
        }
        Op::Chain { parent, len } => {
            let mut parent = parent;
            for _ in 0..len {
                parent = Parent::Popup(made.new_popup(client, globals, qh, parent)?);
            }
        }
        Op::Bare => {
            let surface = globals.compositor.create_surface(qh, ());
            let xdg = globals.wm_base.get_xdg_surface(&surface, qh, Role::Ignored);
            made.bare.push((surface, xdg));
        }
        Op::PopupOnBare { bare, parent } => {
            let index = made.popups.len();
            client.popups.push(PopupRecord::default());
            let (surface, xdg) = made.bare.get(bare).ok_or("no such bare xdg_surface")?;
            let (surface, xdg) = (surface.clone(), xdg.clone());
            let popup = made.get_popup(globals, qh, &xdg, parent, Role::Popup(index))?;
            surface.commit();
            made.popups.push(Popup {
                surface,
                xdg,
                popup,
            });
        }
        Op::GetPopupAgain { popup, parent } => {
            let xdg = made.popups.get(popup).ok_or("no such popup")?.xdg.clone();
            made.get_popup(globals, qh, &xdg, parent, Role::Ignored)?;
        }
        Op::SecondXdgSurface { popup, parent } => {
            let surface = made
                .popups
                .get(popup)
                .ok_or("no such popup")?
                .surface
                .clone();
            let xdg = globals.wm_base.get_xdg_surface(&surface, qh, Role::Ignored);
            made.get_popup(globals, qh, &xdg, parent, Role::Ignored)?;
        }
        Op::Destroy(popup) => {
            made.popups
                .get(popup)
                .ok_or("no such popup")?
                .popup
                .destroy();
        }
        Op::Reincarnate { popup, parent } => {
            let old = made.popups.get(popup).ok_or("no such popup")?;
            old.popup.destroy();
            old.xdg.destroy();
            let surface = old.surface.clone();
            *client.popups.get_mut(popup).ok_or("no such popup")? = PopupRecord::default();
            let xdg = globals
                .wm_base
                .get_xdg_surface(&surface, qh, Role::Popup(popup));
            let new = made.get_popup(globals, qh, &xdg, parent, Role::Popup(popup))?;
            surface.commit();
            made.popups[popup] = Popup {
                surface,
                xdg,
                popup: new,
            };
        }
        Op::Sync => unreachable!("`Step::Batch` runs `Op::Sync` itself"),
        Op::Sub(op) => subsurfaces::run(op, globals, qh, made)?,
        Op::Parentless => {
            let index = made.popups.len();
            client.popups.push(PopupRecord::default());
            let surface = globals.compositor.create_surface(qh, ());
            let xdg = globals
                .wm_base
                .get_xdg_surface(&surface, qh, Role::Popup(index));
            let positioner = globals.positioner(qh);
            let popup = xdg.get_popup(None, &positioner, qh, Role::Popup(index));
            positioner.destroy();
            made.popups.push(Popup {
                surface,
                xdg,
                popup,
            });
        }
        Op::Uncommitted(parent) => {
            let index = made.popups.len();
            client.popups.push(PopupRecord::default());
            let surface = globals.compositor.create_surface(qh, ());
            let xdg = globals
                .wm_base
                .get_xdg_surface(&surface, qh, Role::Popup(index));
            let popup = made.get_popup(globals, qh, &xdg, parent, Role::Popup(index))?;
            made.popups.push(Popup {
                surface,
                xdg,
                popup,
            });
        }
        Op::Adopt { popup, layer } => {
            let popup = &made.popups.get(popup).ok_or("no such popup")?.popup;
            made.layers
                .get(layer)
                .ok_or("no such layer")?
                .layer
                .get_popup(popup);
        }
        Op::Commit(popup) => {
            made.popups
                .get(popup)
                .ok_or("no such popup")?
                .surface
                .commit();
        }
        Op::Reposition(popup) => {
            let positioner = globals.positioner(qh);
            made.popups
                .get(popup)
                .ok_or("no such popup")?
                .popup
                .reposition(&positioner, 1);
            positioner.destroy();
        }
    }
    Ok(())
}

pub(in crate::compositor) fn run_client(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let globals = Globals {
        compositor: client.compositor.clone().ok_or("no wl_compositor")?,
        shm: client.shm.clone().ok_or("no wl_shm")?,
        wm_base: client.wm_base.clone().ok_or("no xdg_wm_base")?,
        subcompositor: client.subcompositor.clone().ok_or("no wl_subcompositor")?,
        layer_shell: client.layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?,
    };

    let mut made = Made::default();
    while let Ok(step) = steps.recv() {
        match step {
            Step::MapWindow => map_window(&mut queue, &mut client, &globals, &qh, &mut made)?,
            Step::MapBar => map_bar(&mut queue, &mut client, &globals, &qh, &mut made)?,
            Step::Batch(ops) => {
                for op in ops {
                    match op {
                        Op::Sync => sync(&conn, &mut queue, &mut client)?,
                        op => run_op(op, &mut client, &globals, &qh, &mut made)?,
                    }
                }
                sync(&conn, &mut queue, &mut client)?;
            }
            Step::Map { popups, marked } => {
                for index in popups {
                    let serial = wait_for(&mut queue, &mut client, "a popup configure", |c| {
                        c.popups.get(index)?.serial
                    })?;
                    let popup = made.popups.get(index).ok_or("no such popup")?;
                    popup.xdg.ack_configure(serial);
                    let color = if marked == Some(index) {
                        MARKED_BGRA
                    } else {
                        POPUP_BGRA
                    };
                    let (buffer, w, h) =
                        solid_buffer(&globals.shm, &qh, POPUP_SIZE, POPUP_SIZE, color)?;
                    popup.surface.attach(Some(&buffer), 0, 0);
                    popup.surface.damage(0, 0, w, h);
                    popup.surface.commit();
                }
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
        }
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn map_window(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    globals: &Globals,
    qh: &QueueHandle<TestClient>,
    made: &mut Made,
) -> Result<(), String> {
    let index = made.windows.len();
    client.pending.push((0, 0));
    client.configured.push(None);
    let surface = globals.compositor.create_surface(qh, ());
    let xdg = globals
        .wm_base
        .get_xdg_surface(&surface, qh, Role::Window(index));
    let toplevel = xdg.get_toplevel(qh, Role::Window(index));
    surface.commit();
    let (serial, width, height) = wait_for(queue, client, "a toplevel configure", |c| {
        *c.configured.get(index)?
    })?;
    xdg.ack_configure(serial);
    let (buffer, width, height) = solid_buffer(&globals.shm, qh, width, height, WINDOW_BGRA)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, width, height);
    surface.commit();
    queue.roundtrip(client).map_err(|e| e.to_string())?;
    made.windows.push(Toplevel {
        surface,
        xdg,
        _toplevel: toplevel,
    });
    Ok(())
}

fn map_bar(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    globals: &Globals,
    qh: &QueueHandle<TestClient>,
    made: &mut Made,
) -> Result<(), String> {
    let index = made.layers.len();
    client.layer_sizes.push(None);
    let surface = globals.compositor.create_surface(qh, ());
    let layer = globals.layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Top,
        "bar".into(),
        qh,
        Role::Layer(index),
    );
    layer.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );
    layer.set_size(0, 20);
    surface.commit();
    let (width, height) = wait_for(queue, client, "a layer configure", |c| {
        *c.layer_sizes.get(index)?
    })?;
    let (buffer, w, h) = solid_buffer(&globals.shm, qh, width as i32, height as i32, BAR_BGRA)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, w, h);
    surface.commit();
    queue.roundtrip(client).map_err(|e| e.to_string())?;
    made.layers.push(Layer {
        _surface: surface,
        layer,
    });
    Ok(())
}

pub(in crate::compositor) type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// One output, one client, one mapped window.
    pub(in crate::compositor) fn with_window() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_client);
        fixture.run(Step::MapWindow);
        fixture
    }

    /// Runs `ops` in one flush, which the client must survive.
    pub(in crate::compositor) fn batch(&mut self, ops: Vec<Op>) {
        let Ack::Done = self.run(Step::Batch(ops));
    }

    /// Runs `ops` in one flush, which must end the client with a protocol
    /// error; hands the error back.
    pub(in crate::compositor) fn refused(&mut self, ops: Vec<Op>) -> String {
        self.run_expecting_disconnect(Step::Batch(ops))
    }

    /// Runs `ops` in one flush, then draws a frame whatever came of it, and
    /// only then hands back how it ended: `Ok` if the client survived, its
    /// protocol error if not. See [`Harness::run_or_disconnect`].
    pub(in crate::compositor) fn attacked(&mut self, ops: Vec<Op>) -> Result<(), String> {
        let outcome = self.run_or_disconnect(Step::Batch(ops));
        self.render();
        outcome.map(|Ack::Done| ())
    }

    /// Maps `popups`, drawing `marked` in [`MARKED_BGRA`].
    pub(in crate::compositor) fn map(
        &mut self,
        popups: impl IntoIterator<Item = usize>,
        marked: Option<usize>,
    ) {
        let Ack::Done = self.run(Step::Map {
            popups: popups.into_iter().collect(),
            marked,
        });
    }

    /// The core id of the `index`-th window, in creation order.
    fn id(&self, index: usize) -> WindowId {
        let mut ids: Vec<WindowId> = self.state.windows.keys().copied().collect();
        ids.sort();
        ids[index]
    }

    /// Window `index`'s placement rect, in global coordinates.
    pub(in crate::compositor) fn rect_of(&self, index: usize) -> Rect {
        self.state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
            .rect
    }

    /// Where the `depth`-th popup of a chain rooted at window `index` has its
    /// top-left corner (depth 1 is the window's own popup).
    pub(in crate::compositor) fn chain_corner(&self, index: usize, depth: usize) -> (i32, i32) {
        let rect = self.rect_of(index);
        let depth = i32::try_from(depth).expect("a small depth");
        (rect.x + STEP * depth, rect.y + STEP * depth)
    }
}

/// A second client connects, maps a window and a popup of it, and the
/// popup is drawn: the compositor survived whatever the first client did,
/// and still serves.
pub(in crate::compositor) fn still_serving(fixture: &mut Fixture) {
    let second = fixture.spawn(run_client);
    let Ack::Done = fixture.run_on(second, Step::MapWindow);
    let Ack::Done = fixture.run_on(second, Step::Batch(vec![Op::Popup(Parent::Window(0))]));
    let Ack::Done = fixture.run_on(
        second,
        Step::Map {
            popups: vec![0],
            marked: Some(0),
        },
    );
    let pixels = fixture.render();
    assert!(
        test_support::contains(&pixels, MARKED_BGRA),
        "the second client's popup was not drawn"
    );
}

pub(in crate::compositor) fn pixel(pixels: &[u8], (x, y): (i32, i32)) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}
