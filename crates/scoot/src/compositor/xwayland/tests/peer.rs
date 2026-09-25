//! The Wayland side of the live suites: one ordinary client that can hold a
//! real window (so an X window has something to steal focus *from*), watch
//! the wlr foreign-toplevel list the way a taskbar does (and `activate` or
//! `close` from it), and lock the session.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self as ext_handle, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self as ext_list, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self as handle, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self as manager, ZwlrForeignToplevelManagerV1,
};

use crate::compositor::test_support::wait_for;

mod clip;
mod ime;

pub(super) use clip::{ClipStep, Which};
pub(super) use ime::ImeStep;

/// What a test tells the peer to do.
#[derive(Debug)]
pub(super) enum Step {
    /// Map an xdg toplevel titled `title`, drawn in `color` at whatever size
    /// it is configured to.
    Map { title: &'static str, color: [u8; 4] },
    /// Bind `zwlr_foreign_toplevel_manager_v1`.
    BindTaskbar,
    /// Bind `ext_foreign_toplevel_list_v1`.
    BindExtList,
    /// Report every toplevel the `ext-` list has announced and not closed,
    /// as `(title, app_id)`, in announcement order.
    ExtToplevels,
    /// Report every toplevel the taskbar has been told about and not seen
    /// close, as `(title, app_id)`, in announcement order.
    Toplevels,
    /// `activate` the toplevel the taskbar knows as `title`.
    Activate(String),
    /// `close` the toplevel the taskbar knows as `title`.
    Close(String),
    /// `ext_session_lock_manager_v1.lock`, and hold it.
    Lock,
    /// A clipboard step (see `peer/clip.rs`).
    Clip(ClipStep),
    /// An input-method step (see `peer/ime.rs`).
    Ime(ImeStep),
}

/// What the peer answers.
#[derive(Debug)]
pub(super) enum Ack {
    Done,
    Toplevels(Vec<(String, String)>),
    /// A selection's offered mime types, `None` when there is no selection.
    Mimes(Option<Vec<String>>),
    /// What a receive read, or why it could not.
    Bytes(Result<Vec<u8>, String>),
    /// A count.
    Count(usize),
    /// Per read, whether it has ended.
    Ends(Vec<bool>),
    /// Key codes an input method received.
    Keys(Vec<u32>),
}

#[derive(Default)]
struct Peer {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    locks: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    manager_name: Option<(u32, u32)>,
    list_name: Option<(u32, u32)>,
    /// The taskbar's handles, with what each was last told.
    handles: Vec<Known>,
    /// The `ext-` list's handles, the same way.
    ext_handles: Vec<ExtKnown>,
    /// Per window: the newest configure's serial and size, if unacked.
    configures: Vec<Option<(u32, i32, i32)>>,
    /// The clipboard half (see `peer/clip.rs`).
    clip: clip::Clip,
    /// The input-method half (see `peer/ime.rs`).
    ime: ime::Ime,
}

struct ExtKnown {
    handle: ExtForeignToplevelHandleV1,
    title: String,
    app_id: String,
    closed: bool,
}

struct Known {
    handle: ZwlrForeignToplevelHandleV1,
    title: String,
    app_id: String,
    closed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Peer {
    fn event(
        peer: &mut Self,
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
            "wl_compositor" => peer.compositor = Some(registry.bind(name, version.min(4), qh, ())),
            "wl_shm" => peer.shm = Some(registry.bind(name, 1, qh, ())),
            "xdg_wm_base" => peer.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_seat" => peer.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "ext_session_lock_manager_v1" => peer.locks = Some(registry.bind(name, 1, qh, ())),
            "zwlr_foreign_toplevel_manager_v1" => peer.manager_name = Some((name, version)),
            "ext_foreign_toplevel_list_v1" => peer.list_name = Some((name, version)),
            "wl_data_device_manager" => {
                peer.clip.data_manager = Some(registry.bind(name, version.min(3), qh, ()));
            }
            "zwp_primary_selection_device_manager_v1" => {
                peer.clip.primary_manager = Some(registry.bind(name, 1, qh, ()));
            }
            "zwp_input_method_manager_v2" => {
                peer.ime.manager = Some(registry.bind(name, 1, qh, ()));
            }
            "zwlr_data_control_manager_v1" => {
                peer.clip.control_manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            peer.handles.push(Known {
                handle: toplevel,
                title: String::new(),
                app_id: String::new(),
                closed: false,
            });
        }
    }

    event_created_child!(Peer, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(known) = peer.handles.iter_mut().find(|known| &known.handle == proxy) else {
            return;
        };
        match event {
            handle::Event::Title { title } => known.title = title,
            handle::Event::AppId { app_id } => known.app_id = app_id,
            handle::Event::Closed => known.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_list::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_list::Event::Toplevel { toplevel } = event {
            peer.ext_handles.push(ExtKnown {
                handle: toplevel,
                title: String::new(),
                app_id: String::new(),
                closed: false,
            });
        }
    }

    event_created_child!(Peer, ExtForeignToplevelListV1, [
        ext_list::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        proxy: &ExtForeignToplevelHandleV1,
        event: ext_handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(known) = peer
            .ext_handles
            .iter_mut()
            .find(|known| &known.handle == proxy)
        else {
            return;
        };
        match event {
            ext_handle::Event::Title { title } => known.title = title,
            ext_handle::Event::AppId { app_id } => known.app_id = app_id,
            ext_handle::Event::Closed => known.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Peer {
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

impl Dispatch<xdg_toplevel::XdgToplevel, usize> for Peer {
    fn event(
        peer: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && let Some(slot) = peer.configures.get_mut(*index)
        {
            let serial = slot.map_or(0, |(serial, _, _)| serial);
            *slot = Some((serial, width, height));
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for Peer {
    fn event(
        peer: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(slot) = peer.configures.get_mut(*index)
        {
            let (_, width, height) = slot.unwrap_or((0, 0, 0));
            *slot = Some((serial, width, height));
        }
    }
}

wayland_client::delegate_noop!(Peer: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Peer: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Peer: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Peer: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Peer: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Peer: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Peer: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(Peer: ignore ext_session_lock_v1::ExtSessionLockV1);

/// A `width`x`height` buffer of `color` over a real memfd.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<Peer>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = width * 4;
    let len = usize::try_from(stride * height).map_err(|e| e.to_string())?;
    let fd = rustix::fs::memfd_create("scoot-xwayland-peer", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), stride * height, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

/// The peer's script, for [`Harness::spawn`](crate::compositor::test_support::Harness::spawn).
pub(super) fn peer(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut peer = Peer::default();
    queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
    let compositor = peer.compositor.clone().ok_or("no wl_compositor")?;
    let shm = peer.shm.clone().ok_or("no wl_shm")?;
    let wm_base = peer.wm_base.clone().ok_or("no xdg_wm_base")?;
    // Held for the run: dropping a window or the lock would end it.
    let mut windows = Vec::new();
    let mut managers = Vec::new();
    let mut managers_ext = Vec::new();
    let mut locks = Vec::new();
    loop {
        // A peer with its selection devices bound keeps dispatching between
        // steps, as a real client's loop does: its sources have to answer
        // `send` while the test is busy pumping the compositor for an X
        // read. Every other test's peer only dispatches inside a step.
        let step = if peer.clip.bound() {
            match steps.recv_timeout(std::time::Duration::from_millis(2)) {
                Ok(step) => step,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    clip::pump(&conn, &mut queue, &mut peer)?;
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match steps.recv() {
                Ok(step) => step,
                Err(_) => break,
            }
        };
        let ack = match step {
            Step::Map { title, color } => {
                let index = peer.configures.len();
                peer.configures.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, index);
                let toplevel = xdg.get_toplevel(&qh, index);
                toplevel.set_title(title.into());
                toplevel.set_app_id("peer".into());
                surface.commit();
                let (serial, width, height) =
                    wait_for(&mut queue, &mut peer, "a toplevel configure", |peer| {
                        peer.configures.get(index).copied().flatten()
                    })?;
                xdg.ack_configure(serial);
                let (width, height) = (width.max(1), height.max(1));
                let buffer = solid_buffer(&shm, &qh, width, height, color)?;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                windows.push((surface, xdg, toplevel, buffer));
                Ack::Done
            }
            Step::BindTaskbar => {
                let (name, version) = peer.manager_name.ok_or("no wlr foreign-toplevel manager")?;
                managers.push(registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(
                    name,
                    version.min(3),
                    &qh,
                    (),
                ));
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::BindExtList => {
                let (name, version) = peer.list_name.ok_or("no ext foreign-toplevel list")?;
                managers_ext.push(registry.bind::<ExtForeignToplevelListV1, _, _>(
                    name,
                    version.min(1),
                    &qh,
                    (),
                ));
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::ExtToplevels => {
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Toplevels(
                    peer.ext_handles
                        .iter()
                        .filter(|known| !known.closed)
                        .map(|known| (known.title.clone(), known.app_id.clone()))
                        .collect(),
                )
            }
            Step::Toplevels => {
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Toplevels(
                    peer.handles
                        .iter()
                        .filter(|known| !known.closed)
                        .map(|known| (known.title.clone(), known.app_id.clone()))
                        .collect(),
                )
            }
            Step::Activate(title) => {
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                let seat = peer.seat.clone().ok_or("no wl_seat")?;
                let known = peer
                    .handles
                    .iter()
                    .find(|known| !known.closed && known.title == title)
                    .ok_or_else(|| format!("the taskbar knows no {title:?}"))?;
                known.handle.activate(&seat);
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Close(title) => {
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                let known = peer
                    .handles
                    .iter()
                    .find(|known| !known.closed && known.title == title)
                    .ok_or_else(|| format!("the taskbar knows no {title:?}"))?;
                known.handle.close();
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Lock => {
                let manager = peer
                    .locks
                    .as_ref()
                    .ok_or("no ext_session_lock_manager_v1")?;
                locks.push(manager.lock(&qh, ()));
                queue.roundtrip(&mut peer).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Clip(step) => clip::step(&mut peer, &mut queue, &conn, step)?,
            Step::Ime(step) => ime::step(&mut peer, &mut queue, step)?,
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}
