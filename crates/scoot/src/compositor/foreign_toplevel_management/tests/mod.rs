//! Tests for `wlr-foreign-toplevel-management-unstable-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `zwlr_foreign_toplevel_manager_v1` and reacting to its events the way a
//! taskbar does, including sending its requests back -- through a real
//! [`State`], and assert on **the exact sequence of events the client
//! received**, in order.
//!
//! That is the point of every one of them: what can go wrong with a window
//! list is a client being told the wrong thing, or told it in the wrong order,
//! or not told at all. A `done` with nothing before it, a `title` on a handle
//! already `closed`, a window announced twice, an `activated` bit left on a
//! window that lost focus -- each is a perfectly sensible-looking call sequence
//! on the compositor side and a taskbar showing a session that does not exist.
//! And unlike the `ext-` list next door, this protocol can *act*: an
//! `activate` resolved onto the wrong window would focus the wrong one.
//!
//! Split three ways, by what a test is about rather than by size:
//!
//! - this file: the client script every suite shares, plus what a client is
//!   told as windows come and go, and the `stop`/disconnect lifecycle;
//! - [`requests`]: the control half -- `activate`, `close`, the requests that
//!   are accepted and ignored, and what the session lock does to all of them;
//! - [`outputs`]: `output_enter`, including the bind-order case the hook in
//!   `handlers.rs` exists for and the wrong-client send that would panic the
//!   compositor.
//!
//! Windows here are bare `xdg_toplevel`s with no buffer, for the same reason
//! `foreign_toplevel/tests.rs` uses them: what puts a window in scoot's lists
//! is the toplevel *existing* (see this protocol's own module doc on what "a
//! toplevel" means here), so nothing needs `wl_shm`.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self as client_handle, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self as client_manager, ZwlrForeignToplevelManagerV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

mod outputs;
mod requests;

/// The framebuffer these tests render into. Nothing here reads a pixel; the
/// backend exists so the compositor has a real output, as it does in a
/// session.
const CANVAS: i32 = 200;

/// How wide and tall the `on_demand` taskbar in [`Step::MapTaskbar`] is.
///
/// Anchored to the bottom-right corner, so [`TASKBAR_POINT`] is inside it and
/// outside any window: the layout puts columns from the left edge, and this
/// suite never opens enough of them to reach the corner.
const TASKBAR: u32 = 60;

/// A point inside that taskbar, for a click that really goes through the
/// pointer.
const TASKBAR_POINT: (f64, f64) = (CANVAS as f64 - 20.0, CANVAS as f64 - 20.0);

/// One protocol event, as the client saw it.
///
/// Toplevel handles are keyed by the order the client was told about them and
/// managers by the order it bound them, so an expectation can be written out
/// literally.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    /// `zwlr_foreign_toplevel_manager_v1.toplevel`
    Toplevel(u32),
    Title(u32, String),
    AppId(u32, String),
    /// `output_enter`, keyed by handle. Which `wl_output` is not recorded:
    /// scoot has exactly one, so the only interesting facts are that the
    /// event arrived and on which handle.
    OutputEnter(u32),
    OutputLeave(u32),
    /// The `state` array, decoded into the protocol's own `uint` values.
    State(u32, Vec<u32>),
    Done(u32),
    Closed(u32),
    /// `zwlr_foreign_toplevel_manager_v1.finished`, keyed by manager.
    Finished(u32),
    /// `xdg_toplevel.close` on the `index`-th window this client created --
    /// i.e. the compositor asking it to go, which is what the protocol's
    /// `close` request must turn into.
    AskedToClose(usize),
    /// The `ext-foreign-toplevel-list-v1` events, for the one test that checks
    /// the two protocols describe the same window list. Keyed by the order the
    /// client was told about *those* handles, which is its own sequence.
    ExtToplevel(u32),
    ExtTitle(u32, String),
    ExtAppId(u32, String),
    ExtClosed(u32),
}

/// The whole burst for one toplevel being announced, in the order this
/// compositor sends it: the handle, its two strings, the output it is on, its
/// state, and the `done` that closes the batch.
fn announced(key: u32, title: &str, app_id: &str, states: &[u32]) -> Vec<Seen> {
    vec![
        Seen::Toplevel(key),
        Seen::Title(key, title.to_string()),
        Seen::AppId(key, app_id.to_string()),
        Seen::OutputEnter(key),
        Seen::State(key, states.to_vec()),
        Seen::Done(key),
    ]
}

/// The batch a focus change sends to one handle.
fn activation(key: u32, states: &[u32]) -> Vec<Seen> {
    vec![Seen::State(key, states.to_vec()), Seen::Done(key)]
}

/// The protocol's `activated` state value, read off the same enum the
/// compositor encodes from.
fn activated() -> Vec<u32> {
    vec![u32::from(ToplevelState::Activated)]
}

/// Decodes a `state` array: the protocol's `uint` values in host byte order.
fn decode_states(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("a four-byte chunk")))
        .collect()
}

#[derive(Default)]
struct TestClient {
    /// Bound on demand so a test can control whether the manager is bound
    /// before or after a window exists -- and, for `wl_output`, before or after
    /// the manager.
    manager_name: Option<(u32, u32)>,
    output_name: Option<(u32, u32)>,
    /// Every `wl_output` global in registry order, so a test with more than
    /// one output can bind a specific screen (see [`Step::BindOutputAt`]).
    /// [`Step::BindOutput`] keeps binding the last one announced, exactly as
    /// before -- with a single output the two are the same object.
    output_names: Vec<(u32, u32)>,
    list_name: Option<(u32, u32)>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    locks: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
    outputs: Vec<wl_output::WlOutput>,
    lists: Vec<ExtForeignToplevelListV1>,
    ext_handles: Vec<ExtForeignToplevelHandleV1>,
    /// Every event since the last [`Step::TakeLog`], in arrival order.
    log: Vec<Seen>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
    /// The size the compositor configured the taskbar at, once it has. A layer
    /// surface may only attach a buffer after acking a configure, and must draw
    /// at the size that configure carried -- which is the compositor's choice,
    /// not the client's.
    taskbar_size: Option<(u32, u32)>,
}

impl TestClient {
    fn manager_key(&self, manager: &ZwlrForeignToplevelManagerV1) -> u32 {
        key_of(&self.managers, manager)
    }

    fn handle_key(&self, handle: &ZwlrForeignToplevelHandleV1) -> u32 {
        key_of(&self.handles, handle)
    }
}

/// The position of `proxy` in `known`, or `u32::MAX` for something the client
/// was never told about -- which shows up as a mismatch in an assertion rather
/// than as a panic in a dispatch handler.
fn key_of<P: Proxy + PartialEq>(known: &[P], proxy: &P) -> u32 {
    known
        .iter()
        .position(|other| other == proxy)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(u32::MAX)
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
            "zwlr_foreign_toplevel_manager_v1" => client.manager_name = Some((name, version)),
            "wl_output" => {
                client.output_name = Some((name, version));
                client.output_names.push((name, version));
            }
            "ext_foreign_toplevel_list_v1" => client.list_name = Some((name, version)),
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "ext_session_lock_manager_v1" => {
                client.locks = Some(registry.bind(name, version.min(1), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        manager: &ZwlrForeignToplevelManagerV1,
        event: client_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            client_manager::Event::Toplevel { toplevel } => {
                client.handles.push(toplevel);
                let key = u32::try_from(client.handles.len() - 1).expect("few toplevels");
                client.log.push(Seen::Toplevel(key));
            }
            client_manager::Event::Finished => {
                let key = client.manager_key(manager);
                client.log.push(Seen::Finished(key));
            }
            _ => {}
        }
    }

    event_created_child!(TestClient, ZwlrForeignToplevelManagerV1, [
        client_manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: client_handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = client.handle_key(handle);
        let seen = match event {
            client_handle::Event::Title { title } => Seen::Title(key, title),
            client_handle::Event::AppId { app_id } => Seen::AppId(key, app_id),
            client_handle::Event::OutputEnter { .. } => Seen::OutputEnter(key),
            client_handle::Event::OutputLeave { .. } => Seen::OutputLeave(key),
            client_handle::Event::State { state } => Seen::State(key, decode_states(&state)),
            client_handle::Event::Done => Seen::Done(key),
            client_handle::Event::Closed => Seen::Closed(key),
            _ => return,
        };
        client.log.push(seen);
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            client.ext_handles.push(toplevel);
            let key = u32::try_from(client.ext_handles.len() - 1).expect("few toplevels");
            client.log.push(Seen::ExtToplevel(key));
        }
    }

    event_created_child!(TestClient, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = key_of(&client.ext_handles, handle);
        let seen = match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => Seen::ExtTitle(key, title),
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => Seen::ExtAppId(key, app_id),
            ext_foreign_toplevel_handle_v1::Event::Closed => Seen::ExtClosed(key),
            _ => return,
        };
        client.log.push(seen);
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

/// A window's own index in creation order, so its configure -- and the
/// compositor's request that it close -- can be matched back to it.
struct WindowIndex(usize);

impl Dispatch<xdg_surface::XdgSurface, WindowIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &WindowIndex,
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

impl Dispatch<xdg_toplevel::XdgToplevel, WindowIndex> for TestClient {
    /// `xdg_toplevel.close` is the whole point of this impl: it is what the
    /// protocol's `close` request has to turn into, and the only way to see it
    /// is from the window's own client. `Configure` and the rest are ignored --
    /// nothing here lays anything out.
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &WindowIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            client.log.push(Seen::AskedToClose(index.0));
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TestClient {
    /// Acks the configure and records the size it carried, which is the only
    /// thing the taskbar needs from the compositor before it can draw.
    fn event(
        client: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer.ack_configure(serial);
            client.taskbar_size = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);

/// One instruction for the client thread.
enum Step {
    /// Bind another `zwlr_foreign_toplevel_manager_v1`.
    BindManager,
    /// The same, at a specific version -- 1 is the one whose `state` enum has
    /// no `fullscreen`.
    BindManagerAt(u32),
    /// Bind a `wl_output`. Separate from the manager so a test can choose the
    /// order, which is what the `output_bound` hook exists for.
    BindOutput,
    /// Bind the `index`-th `wl_output` global in registry order (0 is the
    /// primary output) -- for fixtures with more than one output, where a
    /// window is announced on the output it is on and a bind of any other
    /// output must stay silent.
    BindOutputAt(usize),
    /// Bind an `ext_foreign_toplevel_list_v1` as well, for the cross-protocol
    /// check.
    BindList,
    /// Map a real `on_demand` taskbar: a `zwlr_layer_surface_v1` in the
    /// bottom-right corner with a real `wl_shm` buffer behind it, which is
    /// what it takes to be in the compositor's layer map and therefore
    /// eligible to hold the keyboard.
    MapTaskbar,
    /// Create an `xdg_toplevel` (and ack its configure), with no title, no app
    /// id and no buffer.
    MapWindow,
    /// The same, with the title and app id set before the first commit -- what
    /// a real toolkit does.
    MapDescribedWindow {
        app_id: String,
        title: String,
    },
    /// Destroy the `index`-th window.
    CloseWindow(usize),
    SetTitle(usize, String),
    SetAppId(usize, String),
    /// `destroy` on the `index`-th toplevel handle, while its window is still
    /// open.
    DestroyHandle(usize),
    /// `stop` on the `index`-th manager.
    ///
    /// There is no `destroy` counterpart to script: this interface has no
    /// destructor *request* at all -- a manager object only ever goes away
    /// when the compositor answers `stop` with the `finished` destructor
    /// event, which is the one teardown the protocol defines.
    Stop(usize),
    /// `activate` on the `index`-th handle, naming this client's seat.
    Activate(usize),
    /// `close` on the `index`-th handle.
    RequestClose(usize),
    /// Every request this compositor accepts and ignores, on the `index`-th
    /// handle, including a deliberately invalid rectangle.
    RequestIgnoredStates(usize),
    /// `set_fullscreen` (no output) on the `index`-th handle.
    SetFullscreen(usize),
    /// `unset_fullscreen` on the `index`-th handle.
    UnsetFullscreen(usize),
    /// Open and immediately destroy `count` toplevels, in one burst, without
    /// waiting for anything in between.
    ChurnWindows(usize),
    /// Take the session lock, without ever creating a lock surface.
    LockSession,
    /// Hand back (and clear) everything seen so far.
    TakeLog,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
}

/// Runs the client half: binds what it needs, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let manager_global = client
        .manager_name
        .ok_or("no zwlr_foreign_toplevel_manager_v1 -- the global is missing")?;
    let mut windows: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )> = Vec::new();
    // Held so the lock is not released the moment it is taken.
    let mut lock: Option<ext_session_lock_v1::ExtSessionLockV1> = None;
    // Held for the same reason: dropping the buffer or the role object would
    // unmap the taskbar, which is exactly what the test needs to stay mapped.
    let mut taskbar: Option<(
        wl_surface::WlSurface,
        zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        wl_buffer::WlBuffer,
    )> = None;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match step {
            Step::BindManager => {
                let manager: ZwlrForeignToplevelManagerV1 =
                    registry.bind(manager_global.0, manager_global.1.min(VERSION), &qh, ());
                client.managers.push(manager);
            }
            Step::BindManagerAt(version) => {
                let manager: ZwlrForeignToplevelManagerV1 =
                    registry.bind(manager_global.0, manager_global.1.min(version), &qh, ());
                client.managers.push(manager);
            }
            Step::BindOutput => {
                let global = client.output_name.ok_or("no wl_output")?;
                let output: wl_output::WlOutput = registry.bind(global.0, global.1.min(4), &qh, ());
                client.outputs.push(output);
            }
            Step::BindOutputAt(index) => {
                let global = client
                    .output_names
                    .get(index)
                    .copied()
                    .ok_or_else(|| format!("no wl_output at index {index}"))?;
                let output: wl_output::WlOutput = registry.bind(global.0, global.1.min(4), &qh, ());
                client.outputs.push(output);
            }
            Step::BindList => {
                let global = client.list_name.ok_or("no ext_foreign_toplevel_list_v1")?;
                let list: ExtForeignToplevelListV1 =
                    registry.bind(global.0, global.1.min(1), &qh, ());
                client.lists.push(list);
            }
            Step::MapTaskbar => {
                let shell = client
                    .layer_shell
                    .clone()
                    .ok_or("no zwlr_layer_shell_v1 -- the global is missing")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                let surface = compositor.create_surface(&qh, ());
                let layer = shell.get_layer_surface(
                    &surface,
                    None,
                    zwlr_layer_shell_v1::Layer::Overlay,
                    "scoot-ftl-test-taskbar".into(),
                    &qh,
                    (),
                );
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer.set_size(TASKBAR, TASKBAR);
                layer.set_exclusive_zone(0);
                // The whole point: a surface that takes the keyboard when it
                // is clicked, and keeps it until something takes it away --
                // which is what DMS's and Noctalia's panels are.
                layer.set_keyboard_interactivity(
                    zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
                );
                // The first commit carries no buffer; the protocol requires
                // that before the first configure.
                surface.commit();
                let (width, height) =
                    wait_for(&mut queue, &mut client, "a taskbar configure", |client| {
                        client.taskbar_size
                    })?;
                let buffer = solid_buffer(&shm, &qh, width as i32, height as i32);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage_buffer(0, 0, width as i32, height as i32);
                surface.commit();
                taskbar = Some((surface, layer, buffer));
            }
            Step::MapWindow => windows.push(map_window(
                &compositor,
                &wm_base,
                &qh,
                &mut queue,
                &mut client,
                None,
            )?),
            Step::MapDescribedWindow { app_id, title } => windows.push(map_window(
                &compositor,
                &wm_base,
                &qh,
                &mut queue,
                &mut client,
                Some((app_id, title)),
            )?),
            Step::CloseWindow(index) => {
                // In this order, or the compositor answers with a protocol
                // error: an `xdg_surface` may only be destroyed after its role
                // object.
                let (surface, xdg, toplevel) = windows.get(index).ok_or("no such window")?.clone();
                toplevel.destroy();
                xdg.destroy();
                surface.destroy();
            }
            Step::SetTitle(index, title) => windows
                .get(index)
                .ok_or("no such window")?
                .2
                .set_title(title),
            Step::SetAppId(index, app_id) => windows
                .get(index)
                .ok_or("no such window")?
                .2
                .set_app_id(app_id),
            Step::DestroyHandle(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .destroy(),
            Step::Stop(index) => client.managers.get(index).ok_or("no such manager")?.stop(),
            Step::Activate(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .activate(&seat),
            Step::RequestClose(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .close(),
            Step::RequestIgnoredStates(index) => {
                let handle = client.handles.get(index).ok_or("no such toplevel handle")?;
                handle.set_maximized();
                handle.unset_maximized();
                handle.set_minimized();
                handle.unset_minimized();
                // A rectangle with a negative size, which wlroots answers with
                // an `invalid_rectangle` protocol error. scoot reads nothing
                // from the rectangle, so it must survive this rather than kill
                // the client over a hint it ignores.
                let surface = windows.first().ok_or("no window to hang a rectangle on")?;
                handle.set_rectangle(&surface.0, 0, 0, -1, -1);
            }
            Step::SetFullscreen(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .set_fullscreen(None),
            Step::UnsetFullscreen(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .unset_fullscreen(),
            Step::ChurnWindows(count) => {
                // No configure ack and no wait: the point is the compositor
                // seeing creation and destruction at the rate a client can
                // write them, not a well-behaved window.
                for _ in 0..count {
                    let surface = compositor.create_surface(&qh, ());
                    let index = client.window_serials.len();
                    client.window_serials.push(None);
                    let xdg = wm_base.get_xdg_surface(&surface, &qh, WindowIndex(index));
                    let toplevel = xdg.get_toplevel(&qh, WindowIndex(index));
                    surface.commit();
                    toplevel.destroy();
                    xdg.destroy();
                    surface.destroy();
                }
            }
            Step::LockSession => {
                let manager = client
                    .locks
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                lock = Some(manager.lock(&qh, ()));
            }
            Step::TakeLog => outcome = Ack::Log(std::mem::take(&mut client.log)),
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        if let Ack::Log(log) = &mut outcome {
            // Anything that arrived during the round trip above belongs to
            // this batch too: the step before a `TakeLog` may have provoked
            // events that were still in flight when it was acknowledged.
            log.extend(std::mem::take(&mut client.log));
        }
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    // Keeps both alive for the whole script rather than dropping them at the
    // first step boundary.
    drop(lock);
    drop(taskbar);
    Ok(())
}

/// A `width`x`height` opaque `wl_buffer` over a real memfd -- the same path any
/// toolkit takes, and what a layer surface needs before it counts as mapped.
///
/// The colour does not matter: nothing in this suite reads a pixel. What
/// matters is that a buffer exists at the size the compositor configured.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
) -> wl_buffer::WlBuffer {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-ftl-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len]).expect("a filled pool");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Creates one `xdg_toplevel`, optionally describing it first, and acks the
/// configure that comes back.
fn map_window(
    compositor: &wl_compositor::WlCompositor,
    wm_base: &xdg_wm_base::XdgWmBase,
    qh: &QueueHandle<TestClient>,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    described: Option<(String, String)>,
) -> Result<
    (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ),
    String,
> {
    let surface = compositor.create_surface(qh, ());
    let index = client.window_serials.len();
    client.window_serials.push(None);
    let xdg = wm_base.get_xdg_surface(&surface, qh, WindowIndex(index));
    let toplevel = xdg.get_toplevel(qh, WindowIndex(index));
    if let Some((app_id, title)) = described {
        toplevel.set_app_id(app_id);
        toplevel.set_title(title);
    }
    surface.commit();
    let serial = wait_for(queue, client, "a toplevel configure", |client| {
        client.window_serials[index]
    })?;
    xdg.ack_configure(serial);
    Ok((surface, xdg, toplevel))
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time. See [`crate::compositor::test_support`] for
/// everything that is not specific to this protocol.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// A fixture whose client holds the screen and the manager, in the order a
    /// real shell binds them (registry order is the server's choice, but a
    /// toolkit binds `wl_output` as part of coming up), with the (empty)
    /// initial burst already drained.
    fn bound() -> Self {
        let mut fixture = Self::new();
        fixture.run(Step::BindOutput);
        fixture.run(Step::BindManager);
        fixture.take_log();
        fixture
    }

    /// Everything client 0 has seen since the last call.
    fn take_log(&mut self) -> Vec<Seen> {
        self.take_log_on(0)
    }

    fn take_log_on(&mut self, client: usize) -> Vec<Seen> {
        match self.run_on(client, Step::TakeLog) {
            Ack::Log(log) => log,
            Ack::Done => panic!("the client answered a log request with nothing"),
        }
    }

    /// A left click at a point, press and release, the way a user makes one --
    /// the same helper `layer_shell/tests/mod.rs` uses, for the same reason:
    /// `State::clicked_layer` is only ever written by a click that really went
    /// through the pointer.
    fn click(&mut self, x: f64, y: f64) {
        self.state.pointer_move(x, y);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, false);
        self.settle();
    }

    /// The `wl_surface` the seat's keyboard focus is actually on, which is the
    /// only unambiguous answer to "where do keystrokes go".
    fn keyboard_surface(&self) -> Option<WlSurface> {
        self.state
            .seat
            .get_keyboard()
            .expect("a keyboard")
            .current_focus()
    }

    /// The `wl_surface` of the `id`-th window's toplevel.
    fn window_surface(&self, id: u64) -> WlSurface {
        self.state
            .windows
            .get(&WindowId(id))
            .and_then(Window::toplevel)
            .expect("a live window")
            .wl_surface()
            .clone()
    }

    /// How many windows the compositor is keeping handles for.
    fn tracked(&self) -> usize {
        self.state.foreign_toplevel_management.toplevels.len()
    }

    /// How many managers are still subscribed to new windows.
    fn managers(&self) -> usize {
        self.state.foreign_toplevel_management.managers.len()
    }

    /// How many handle objects the compositor holds for `window`, across every
    /// client -- the leak this module's bookkeeping could have.
    fn handles_for(&self, window: u64) -> usize {
        self.state
            .foreign_toplevel_management
            .toplevels
            .get(&WindowId(window))
            .map(|toplevel| toplevel.handles.len())
            .unwrap_or_default()
    }
}

// -- what a client is told -----------------------------------------------

#[test]
fn binding_with_no_windows_announces_nothing() {
    // The zero-window case: a bar started before anything else in the session
    // binds this global with an empty desktop behind it, and must simply be
    // told nothing -- not an empty `done`, which belongs to a *handle*.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager);
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.tracked(), 0);
    assert_eq!(fixture.managers(), 1);
}

#[test]
fn a_window_opened_after_binding_is_announced_then_activated() {
    // Two batches, on purpose: the window is announced from `add_window`
    // before the core has been told it exists, so its first `state` is empty,
    // and the focus it is about to be given arrives as its own batch from the
    // same `apply`. A client draws on `done`, so it sees one window that
    // becomes active, never a half-described one.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    let mut expected = announced(0, "", "", &[]);
    expected.extend(activation(0, &activated()));
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(fixture.tracked(), 1);
    assert_eq!(fixture.handles_for(1), 1);
}

#[test]
fn windows_that_already_exist_are_announced_at_bind_oldest_first() {
    // Registry order is the server's choice and a bar may well start after the
    // session is full of windows, so binding has to describe the world as it
    // already is -- including which window is focused, which is the second one
    // here, and in creation order, which is what the id-keyed map gives.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::MapDescribedWindow {
        app_id: "one".to_string(),
        title: "first".to_string(),
    });
    fixture.run(Step::MapDescribedWindow {
        app_id: "two".to_string(),
        title: "second".to_string(),
    });
    fixture.run(Step::BindManager);

    let mut expected = announced(0, "first", "one", &[]);
    expected.extend(announced(1, "second", "two", &activated()));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_window_created_with_a_title_reports_it_in_its_own_batch() {
    // What every real toolkit does: `get_toplevel`, then `set_app_id` and
    // `set_title`, then commit. The handle exists from `get_toplevel`, so the
    // description arrives as later batches rather than in the first one --
    // which is exactly what `done` is for, and why a client draws on `done`
    // rather than on each event.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapDescribedWindow {
        app_id: "org.scoot.Probe".to_string(),
        title: "a window".to_string(),
    });
    // The activation batch lands between the two: the window is announced and
    // then focused inside `add_window`, and the client's `set_app_id`/
    // `set_title` are separate requests that arrive after all of that.
    let mut expected = announced(0, "", "", &[]);
    expected.extend(activation(0, &activated()));
    expected.extend([
        Seen::AppId(0, "org.scoot.Probe".to_string()),
        Seen::Done(0),
        Seen::Title(0, "a window".to_string()),
        Seen::Done(0),
    ]);
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_title_change_is_one_title_event_and_one_done() {
    // The app id is *not* re-sent. `refresh_window` hands this module both
    // fields and does not say which moved, so the published snapshot is what
    // narrows it to one event -- the same wire traffic the `ext-` list
    // produces, where Smithay does the same comparison inside `send_title`.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapDescribedWindow {
        app_id: "steady".to_string(),
        title: "before".to_string(),
    });
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "after".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "after".to_string()), Seen::Done(0)],
    );
}

#[test]
fn an_app_id_change_is_one_app_id_event_and_one_done() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapDescribedWindow {
        app_id: "before".to_string(),
        title: "steady".to_string(),
    });
    fixture.take_log();

    fixture.run(Step::SetAppId(0, "after".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::AppId(0, "after".to_string()), Seen::Done(0)],
    );
}

#[test]
fn setting_the_same_title_again_tells_the_list_nothing() {
    // A terminal that re-sets the title it already has (every prompt, for some
    // shells) must not turn into wire traffic for every bar in the session.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(0, "steady".to_string()));
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "steady".to_string()));
    fixture.run(Step::SetTitle(0, "steady".to_string()));
    assert_eq!(fixture.take_log(), Vec::new());
}

#[test]
fn a_title_as_long_as_the_wire_allows_survives_the_round_trip() {
    // A client's title is its own to choose, and it reaches every watching
    // client verbatim. Nothing here truncates or validates it -- the wayland
    // message size is the only bound -- so this pins that a large-but-legal
    // one is forwarded whole rather than truncated, dropped, or turned into a
    // protocol error for the innocent taskbar watching.
    let title = "t".repeat(3000);
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::SetTitle(0, title.clone()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, title.clone()), Seen::Done(0)],
    );
    let snapshot = fixture
        .state
        .window_snapshots()
        .into_iter()
        .next()
        .expect("the window is in the IPC list");
    assert_eq!(snapshot.title, title);
}

#[test]
fn closing_a_window_closes_its_handle() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0, "a handle outlived its window");
}

#[test]
fn a_window_closing_leaves_the_others_alone() {
    // One window's `closed` must not be another's: the handles are keyed by
    // `WindowId`, and a taskbar that dropped the wrong row would be showing a
    // window that is gone and hiding one that is not. Closing the *first* of
    // several is the shape most likely to expose an off-by-one in removal --
    // the hazard `foreign_toplevel.rs`'s own suite guards against upstream.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1
    fixture.run(Step::MapWindow); // window 2
    fixture.run(Step::MapWindow); // window 3
    fixture.run(Step::SetTitle(1, "survivor".to_string()));
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::Closed(0)),
        "the closed window's handle was not closed: {log:?}"
    );
    assert!(
        !log.iter()
            .any(|seen| matches!(seen, Seen::Closed(key) if *key != 0)),
        "a surviving window's handle was closed too: {log:?}"
    );

    fixture.run(Step::SetTitle(1, "still here".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(1, "still here".to_string()), Seen::Done(1)],
    );
    assert_eq!(fixture.tracked(), 2);
}

#[test]
fn a_late_bind_after_closing_the_first_of_several_sees_the_right_survivors() {
    // The same hazard from the other side: a client binding *after* a close
    // must be described the windows that are actually open, not the set that
    // existed before. Titles are the check rather than handle keys, because a
    // fresh bind numbers its handles from scratch.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::MapDescribedWindow {
        app_id: "a".to_string(),
        title: "first".to_string(),
    });
    fixture.run(Step::MapDescribedWindow {
        app_id: "b".to_string(),
        title: "second".to_string(),
    });
    fixture.run(Step::MapDescribedWindow {
        app_id: "c".to_string(),
        title: "third".to_string(),
    });
    fixture.run(Step::CloseWindow(0));
    fixture.take_log();

    fixture.run(Step::BindManager);
    let log = fixture.take_log();
    let titles: Vec<String> = log
        .iter()
        .filter_map(|seen| match seen {
            Seen::Title(_, title) => Some(title.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        titles,
        vec!["second".to_string(), "third".to_string()],
        "a fresh bind after closing the first window saw the wrong survivors: {log:?}"
    );
}

#[test]
fn a_reopened_window_gets_a_handle_of_its_own() {
    // Window ids only ever increase, so a handle a client kept after `closed`
    // can never be confused with the next window -- which is what makes it
    // safe for `activate` and `close` to resolve through the id stored in a
    // handle's user data.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CloseWindow(0));
    fixture.take_log();

    fixture.run(Step::MapWindow);
    let mut expected = announced(1, "", "", &[]);
    expected.extend(activation(1, &activated()));
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(
        fixture.handles_for(1),
        0,
        "the dead window is still tracked"
    );
    assert_eq!(fixture.handles_for(2), 1);
}

// -- focus, and the one state bit ----------------------------------------

#[test]
fn focus_moving_deactivates_one_window_and_activates_the_other() {
    // The `activated` bit is scoot's real window focus, re-derived from
    // `State::focus` on every change. Both windows are told, in one batch
    // each, and nothing else moves.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1, focused
    fixture.run(Step::MapWindow); // window 2, takes the focus
    fixture.take_log();

    fixture.state.act(Action::FocusWindowId(WindowId(1)));
    fixture.settle();
    let log = fixture.take_log();
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::Done(_)))
            .count(),
        2
    );
    assert!(
        log.contains(&Seen::State(0, activated())),
        "the newly focused window was not activated: {log:?}"
    );
    assert!(
        log.contains(&Seen::State(1, Vec::new())),
        "the window that lost focus was not deactivated: {log:?}"
    );
}

#[test]
fn closing_the_focused_window_activates_the_survivor_and_nothing_else() {
    // `remove_window` writes `State::focus` directly, bypassing `set_focus`,
    // which is exactly why the activation refresh re-derives from that field
    // instead of being handed "the window that lost focus". The dead window's
    // handle must be `closed` and never written to again.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1
    fixture.run(Step::MapWindow); // window 2, focused
    fixture.take_log();

    fixture.run(Step::CloseWindow(1));
    let log = fixture.take_log();
    assert_eq!(
        log,
        vec![Seen::Closed(1), Seen::State(0, activated()), Seen::Done(0),],
        "closing the focused window told the client the wrong thing",
    );
}

#[test]
fn the_last_window_closing_leaves_no_activation_behind() {
    // The one-window case, which is also the "nothing is focused" case: there
    // is nobody left to activate, and the compositor must not try to write to
    // the handle it has just closed.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0);
    assert_eq!(fixture.state.focus, None);
}

// -- more than one manager, and more than one client ---------------------

#[test]
fn two_managers_in_one_client_each_get_their_own_handle() {
    // A client may bind the global more than once (a shell with two widgets
    // watching windows). Each binding is its own object tree, and both are
    // kept in step afterwards.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    fixture.run(Step::BindManager);
    assert_eq!(
        fixture.take_log(),
        announced(1, "", "", &activated()),
        "the second manager did not see the window that already existed",
    );
    assert_eq!(fixture.handles_for(1), 2);

    fixture.run(Step::SetTitle(0, "both".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::Title(0, "both".to_string()),
            Seen::Done(0),
            Seen::Title(1, "both".to_string()),
            Seen::Done(1),
        ],
    );
}

#[test]
fn two_clients_see_the_same_window() {
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    for client in 0..2 {
        fixture.run_on(client, Step::BindOutput);
        fixture.run_on(client, Step::BindManager);
    }
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.run_on(0, Step::MapWindow);
    let mut expected = announced(0, "", "", &[]);
    expected.extend(activation(0, &activated()));
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(fixture.take_log_on(1), expected);
    assert_eq!(fixture.handles_for(1), 2);

    // The window belongs to client 0; client 1 is told when it goes.
    fixture.run_on(0, Step::CloseWindow(0));
    assert_eq!(fixture.take_log_on(1), vec![Seen::Closed(0)]);
}

#[test]
fn a_client_that_disconnects_stops_being_watched() {
    // The ordinary end of a bar's life. Nothing may be left pointing at it,
    // and every other client's view must survive it.
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    for client in 0..2 {
        fixture.run_on(client, Step::BindOutput);
        fixture.run_on(client, Step::BindManager);
    }
    fixture.run_on(1, Step::MapWindow);
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.disconnect(0);
    assert_eq!(fixture.managers(), 1, "a dead client's manager was kept");
    assert_eq!(fixture.handles_for(1), 1, "a dead client's handle was kept");

    fixture.run_on(1, Step::MapWindow);
    fixture.run_on(1, Step::CloseWindow(0));
    let log = fixture.take_log_on(1);
    assert!(
        log.contains(&Seen::Toplevel(1)) && log.contains(&Seen::Closed(0)),
        "the surviving client stopped being served: {log:?}"
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn destroying_a_handle_early_costs_no_one_else_anything() {
    // Legal, and the protocol says so: a client may destroy a handle while its
    // window is still open (it just will not get another one for that window).
    // What must not happen is the compositor trying to send `closed` on the
    // dead object, or the *other* client losing its own event.
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    for client in 0..2 {
        fixture.run_on(client, Step::BindOutput);
        fixture.run_on(client, Step::BindManager);
    }
    fixture.run_on(0, Step::MapWindow);
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.run_on(0, Step::DestroyHandle(0));
    assert_eq!(fixture.handles_for(1), 1, "the destroyed handle was kept");

    fixture.run_on(0, Step::SetTitle(0, "unheard".to_string()));
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a destroyed handle was still being written to"
    );
    assert_eq!(
        fixture.take_log_on(1),
        vec![Seen::Title(0, "unheard".to_string()), Seen::Done(0)],
    );

    fixture.run_on(0, Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.take_log_on(1), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0);
}

// -- stop ----------------------------------------------------------------

#[test]
fn stop_finishes_the_manager_and_no_later_window_is_announced() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), vec![Seen::Finished(0)]);
    assert_eq!(fixture.managers(), 0);

    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a stopped manager was still being told about new windows"
    );
    // The window itself is still tracked -- `stop` is one client's choice, not
    // a change to the compositor.
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn a_handle_from_before_stop_still_reports_changes() {
    // `stop` says "no more *toplevel* events", not "forget the handles I
    // already have": the protocol's own teardown sequence is stop, wait for
    // `finished`, then destroy the handles -- which a client cannot do safely
    // if the compositor has already stopped telling it what they are doing.
    // wlroots keeps them served for the same reason.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Stop(0));
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "after stop".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "after stop".to_string()), Seen::Done(0)],
    );

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
}

#[test]
fn stopping_a_manager_twice_over_is_survivable() {
    // Not well-behaved -- the protocol says a client must send no further
    // requests after `stop` -- and it may not take the compositor with it.
    //
    // What is actually exercised is a *client-side* swallow, and saying so is
    // the point of this comment: `finished` is a destructor event here (unlike
    // the `ext-` list's, which is not), so by the time the round trip after the
    // first `stop` returns, wayland-client has marked the proxy dead and the
    // generated `stop()` drops the second request before it is written. The
    // compositor therefore sees one `stop`, answers one `finished`, and
    // unregisters once -- which is what the assertions below pin, and why a
    // *second* `finished` would be the failure rather than a protocol error.
    // Pinning it matters because the other order (send both before the round
    // trip) is a client's to choose and the compositor must survive either.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::Stop(0));
    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), vec![Seen::Finished(0)]);
    assert_eq!(fixture.managers(), 0);

    // The handles it made are still served, which is what the protocol's own
    // teardown (stop, wait for `finished`, then destroy them) requires. The
    // second window is not announced to the stopped manager, but it does take
    // the focus -- and the first window's handle is told so, which is the
    // `State(0, [])` below.
    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(0, "orphaned handle".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::State(0, Vec::new()),
            Seen::Done(0),
            Seen::Title(0, "orphaned handle".to_string()),
            Seen::Done(0),
        ],
    );
    assert_eq!(fixture.tracked(), 2);

    // Still serving: a fresh manager binds and sees both windows.
    fixture.run(Step::BindManager);
    let log = fixture.take_log();
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::Toplevel(_)))
            .count(),
        2,
        "a fresh manager did not see both windows: {log:?}"
    );
}

#[test]
fn windows_opened_and_destroyed_at_full_rate_leave_nothing_behind() {
    // The maximum rate a client can actually reach: 200 toplevels created and
    // destroyed in one burst, with no configure ack and nothing waited for in
    // between -- so the compositor sees creation and destruction of the same
    // window in a single dispatch. What is under test is that each one is
    // announced and closed exactly once and that nothing is left tracked
    // afterwards, which is the leak this module's bookkeeping could have.
    const CHURN: usize = 200;

    let mut fixture = Fixture::bound();
    fixture.run(Step::ChurnWindows(CHURN));
    let log = fixture.take_log();
    let toplevels = log
        .iter()
        .filter(|seen| matches!(seen, Seen::Toplevel(_)))
        .count();
    let closed = log
        .iter()
        .filter(|seen| matches!(seen, Seen::Closed(_)))
        .count();
    assert_eq!(toplevels, CHURN, "not every window was announced");
    assert_eq!(closed, CHURN, "not every window was closed");
    assert_eq!(fixture.tracked(), 0, "handles outlived their windows");
    assert_eq!(fixture.managers(), 1, "the manager was dropped");
    assert!(
        fixture.state.windows.is_empty(),
        "the compositor kept windows the client destroyed"
    );
}

// -- the two window lists, side by side ----------------------------------

#[test]
fn both_window_lists_describe_the_same_windows_to_one_client() {
    // scoot publishes its windows twice, through this protocol and through
    // `ext-foreign-toplevel-list-v1`, from the same three lifecycle events. A
    // client bound to both -- which a shell hedging its bets really would be --
    // must see one window list described twice, never two that disagree about
    // what exists.
    let mut fixture = Fixture::bound();
    fixture.run(Step::BindList);
    fixture.take_log();

    fixture.run(Step::MapDescribedWindow {
        app_id: "org.scoot.Probe".to_string(),
        title: "shared".to_string(),
    });
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::Toplevel(_)))
            .count(),
        2,
    );
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::ExtToplevel(_)))
            .count(),
        2,
        "the two protocols announced a different number of windows: {log:?}"
    );
    // The same strings reach both, for the same window, keyed the same way:
    // each list numbered its handles in announcement order, and both were
    // announced from the same `add_window`.
    for (wlr, ext) in [
        (
            Seen::Title(0, "shared".to_string()),
            Seen::ExtTitle(0, "shared".to_string()),
        ),
        (
            Seen::AppId(0, "org.scoot.Probe".to_string()),
            Seen::ExtAppId(0, "org.scoot.Probe".to_string()),
        ),
    ] {
        assert!(
            log.contains(&wlr),
            "the wlr list never said {wlr:?}: {log:?}"
        );
        assert!(
            log.contains(&ext),
            "the ext list never said {ext:?}: {log:?}"
        );
    }

    // ...and both are told about the same close, for the same window.
    fixture.run(Step::CloseWindow(0));
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::Closed(0)) && log.contains(&Seen::ExtClosed(0)),
        "the two protocols disagreed about a window closing: {log:?}"
    );

    // And both agree with the third list scoot publishes, its own IPC one.
    let snapshots = fixture.state.window_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(fixture.tracked(), 1);
}

// -- the session lock ----------------------------------------------------

#[test]
fn the_window_list_stays_live_while_the_session_is_locked() {
    // Deliberate, and the same answer `scoot msg windows` and the `ext-` list
    // give (see this module's doc and `docs/protocols.md`'s lock section): a
    // process that can reach this socket is inside the trust boundary already,
    // and sending `closed` for windows that did not close would be a lie a
    // taskbar could not recover from. The *requests* are refused; see
    // `requests.rs`.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::LockSession);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the test never locked the session"
    );
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "locking the session churned the window list"
    );

    fixture.run(Step::SetTitle(0, "behind the lock".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "behind the lock".to_string()), Seen::Done(0)],
    );

    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::Toplevel(1)),
        "a window opened behind the lock screen was not announced: {log:?}"
    );
}
