//! Floating windows through a real `wayland-client` connection.
//!
//! What is under test is what a client is *told* (the configure's size and
//! whether it carries the `tiled_*` states), where its window ends up, what
//! reaches the framebuffer (a floating window over the strip, its ring over
//! the window beneath it, nothing past its output, nothing over the lock),
//! and where the pointer lands -- none of which a test calling the core
//! directly can see.
//!
//! Windows draw at the size they were configured to, the way a real client
//! does, and at their own "natural" size when a configure leaves the size to
//! them (0x0) -- the shape a floating window's first frame has.
//!
//! Like every live-`State` suite here, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::{Placement, Rect, WindowId};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::dialog::v1::client::{xdg_dialog_v1, xdg_wm_dialog_v1};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::{self, Harness, wait_for};

mod auto;
mod scene;
mod toggle;

/// The framebuffer, square: room for two half-width columns and a dialog
/// clearly smaller than either.
const CANVAS: i32 = 240;
/// The natural size the test client draws at when a configure leaves the
/// size to it: small enough to sit inside a column with room around it.
const NATURAL: (i32, i32) = (60, 40);
/// The ring thickness [`appearance`] asks for.
const RING: i32 = 3;

// Colours, as the BGRA bytes an `Argb8888` pixman buffer holds them in, all
// distinct in every channel.
const TILED_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const DIALOG_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];
const OTHER_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const RING_BGRA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: RING,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(1.0, 0.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// How a toplevel describes itself before its first commit.
#[derive(Clone, Debug)]
struct Spec {
    color: [u8; 4],
    app_id: Option<&'static str>,
    title: Option<&'static str>,
    /// `set_parent` to the `n`-th toplevel this client made.
    parent: Option<usize>,
    /// `set_min_size` / `set_max_size`.
    min: Option<(i32, i32)>,
    max: Option<(i32, i32)>,
    /// `xdg_wm_dialog_v1.get_xdg_dialog` (and `set_modal`).
    dialog: bool,
    /// What it draws at when a configure leaves the size to it.
    natural: (i32, i32),
}

impl Spec {
    fn tiled() -> Self {
        Self {
            color: TILED_BGRA,
            app_id: None,
            title: None,
            parent: None,
            min: None,
            max: None,
            dialog: false,
            natural: NATURAL,
        }
    }

    fn dialog_of(parent: usize) -> Self {
        Self {
            color: DIALOG_BGRA,
            parent: Some(parent),
            ..Self::tiled()
        }
    }
}

/// One configure a toplevel was sent, as the client saw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Configured {
    serial: u32,
    width: i32,
    height: i32,
    fullscreen: bool,
    /// Whether it carried all four `tiled_*` states.
    tiled: bool,
    /// Whether it carried any of them.
    any_tiled: bool,
}

enum Step {
    /// Create a toplevel described by the spec, commit without a buffer,
    /// round-trip (so every configure the commit provoked has arrived), then
    /// ack the newest and draw for it.
    Map(Spec),
    /// Ack the newest configure and draw for it.
    Draw {
        window: usize,
    },
    /// Ack the newest configure, then draw `width`x`height` whatever it
    /// said: a client that will not take the size it was asked for.
    DrawSized {
        window: usize,
        width: i32,
        height: i32,
    },
    /// Every configure the `window`-th toplevel has been sent.
    Configures {
        window: usize,
    },
    /// `set_fullscreen` / `unset_fullscreen`, then wait for the answer.
    SetFullscreen {
        window: usize,
    },
    UnsetFullscreen {
        window: usize,
    },
    /// An `xdg_popup` on the `window`-th toplevel, `size` big, anchored at
    /// `anchor` (a rect in the parent's window geometry), growing down and
    /// right, asking for every adjustment; answers the geometry the
    /// compositor configured it at.
    Popup {
        window: usize,
        anchor: (i32, i32, i32, i32),
        size: (i32, i32),
    },
    /// Which of this client's surfaces the pointer last entered.
    ReportPointer,
    /// Lock the session and keep the lock (abandoned: it stays locked).
    LockSession,
}

enum Ack {
    Done,
    Configures(Vec<Configured>),
    Popup((i32, i32, i32, i32)),
    Pointer(Option<Entered>),
}

/// A surface the pointer entered, by the order the script created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entered {
    Window(usize),
    Other,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    wm_dialog: Option<xdg_wm_dialog_v1::XdgWmDialogV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_focus: Option<wl_surface::WlSurface>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    /// Per toplevel, by creation order: the `xdg_toplevel.configure` state
    /// waiting for its `xdg_surface.configure`, and every completed one.
    pending: Vec<Configured>,
    configures: Vec<Vec<Configured>>,
    /// Per toplevel: the serial it acked last.
    acked: Vec<Option<u32>>,
    /// Per popup: its configured geometry, once it has one.
    popups: Vec<Option<(i32, i32, i32, i32)>>,
}

/// A surface's index in creation order, by kind.
#[derive(Clone, Copy)]
enum Role {
    Window(usize),
    Popup(usize),
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
            // v3: tiled states exist from v2 on, so a floating window's
            // configure can be seen to drop them.
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "xdg_wm_dialog_v1" => {
                client.wm_dialog = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
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

impl Dispatch<xdg_toplevel::XdgToplevel, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            },
            Role::Window(index),
        ) = (event, role)
            && let Some(pending) = client.pending.get_mut(*index)
        {
            let has = |wanted: xdg_toplevel::State| {
                states
                    .chunks_exact(4)
                    .filter_map(|bytes| bytes.try_into().ok().map(u32::from_ne_bytes))
                    .any(|state| state == wanted as u32)
            };
            let tiled_states = [
                xdg_toplevel::State::TiledLeft,
                xdg_toplevel::State::TiledRight,
                xdg_toplevel::State::TiledTop,
                xdg_toplevel::State::TiledBottom,
            ];
            *pending = Configured {
                serial: 0,
                width,
                height,
                fullscreen: has(xdg_toplevel::State::Fullscreen),
                tiled: tiled_states.into_iter().all(has),
                any_tiled: tiled_states.into_iter().any(has),
            };
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, Role> for TestClient {
    fn event(
        client: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        match role {
            Role::Window(index) => {
                if let Some(pending) = client.pending.get(*index).copied()
                    && let Some(seen) = client.configures.get_mut(*index)
                {
                    seen.push(Configured { serial, ..pending });
                }
            }
            // A popup is acked at once: its geometry is all these tests read.
            Role::Popup(_) => xdg.ack_configure(serial),
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            },
            Role::Popup(index),
        ) = (event, role)
            && let Some(slot) = client.popups.get_mut(*index)
        {
            *slot = Some((x, y, width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore xdg_wm_dialog_v1::XdgWmDialogV1);
wayland_client::delegate_noop!(TestClient: ignore xdg_dialog_v1::XdgDialogV1);
wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);

/// A `width`x`height` buffer of `color` over a real memfd.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> wl_buffer::WlBuffer {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-floating-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// One toplevel the script made, with what it draws.
struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    color: [u8; 4],
    natural: (i32, i32),
    // Held for the run: dropping it would end the dialog hint.
    _dialog: Option<xdg_dialog_v1::XdgDialogV1>,
}

/// Acks the newest configure (unless it is the one acked last) and draws
/// `width`x`height`, or the configure's size, or the window's natural size
/// where the configure left it to the client.
fn draw(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    window: &Toplevel,
    index: usize,
    size: Option<(i32, i32)>,
) -> Result<(), String> {
    let newest = client
        .configures
        .get(index)
        .and_then(|all| all.last().copied())
        .ok_or("no configure to draw for")?;
    if client.acked.get(index).copied().flatten() != Some(newest.serial) {
        window.xdg.ack_configure(newest.serial);
        if let Some(slot) = client.acked.get_mut(index) {
            *slot = Some(newest.serial);
        }
    }
    let (width, height) = size.unwrap_or((
        if newest.width > 0 {
            newest.width
        } else {
            window.natural.0
        },
        if newest.height > 0 {
            newest.height
        } else {
            window.natural.1
        },
    ));
    let buffer = solid_buffer(shm, qh, width, height, window.color);
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage(0, 0, width, height);
    window.surface.commit();
    Ok(())
}

fn wait_for_configure(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    window: usize,
    seen: usize,
) -> Result<(), String> {
    wait_for(queue, client, "a toplevel configure", |client| {
        (client.configures.get(window)?.len() > seen).then_some(())
    })
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

    let mut windows: Vec<Toplevel> = Vec::new();
    // Held for the run, like the buffers: a dropped popup is dismissed.
    let mut popups = Vec::new();
    let mut locks = Vec::new();
    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::Map(spec) => {
                let index = windows.len();
                client.pending.push(Configured::default());
                client.configures.push(Vec::new());
                client.acked.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Window(index));
                let toplevel = xdg.get_toplevel(&qh, Role::Window(index));
                if let Some(app_id) = spec.app_id {
                    toplevel.set_app_id(app_id.into());
                }
                if let Some(title) = spec.title {
                    toplevel.set_title(title.into());
                }
                if let Some(parent) = spec.parent {
                    toplevel.set_parent(Some(&windows[parent].toplevel));
                }
                if let Some((w, h)) = spec.min {
                    toplevel.set_min_size(w, h);
                }
                if let Some((w, h)) = spec.max {
                    toplevel.set_max_size(w, h);
                }
                let dialog = if spec.dialog {
                    let manager = client.wm_dialog.clone().ok_or("no xdg_wm_dialog_v1")?;
                    let dialog = manager.get_xdg_dialog(&toplevel, &qh, ());
                    dialog.set_modal();
                    Some(dialog)
                } else {
                    None
                };
                surface.commit();
                // Everything the first commit provoked, the second
                // configure a window that floats gets included.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                wait_for_configure(&mut queue, &mut client, index, 0)?;
                let window = Toplevel {
                    surface,
                    xdg,
                    toplevel,
                    color: spec.color,
                    natural: spec.natural,
                    _dialog: dialog,
                };
                draw(&mut client, &qh, &shm, &window, index, None)?;
                windows.push(window);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Draw { window } => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                draw(&mut client, &qh, &shm, &windows[window], window, None)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::DrawSized {
                window,
                width,
                height,
            } => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                draw(
                    &mut client,
                    &qh,
                    &shm,
                    &windows[window],
                    window,
                    Some((width, height)),
                )?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Configures { window } => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Configures(client.configures[window].clone())
            }
            Step::SetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.set_fullscreen(None);
                wait_for_configure(&mut queue, &mut client, window, seen)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::UnsetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.unset_fullscreen();
                wait_for_configure(&mut queue, &mut client, window, seen)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Popup {
                window,
                anchor,
                size,
            } => {
                let index = client.popups.len();
                client.popups.push(None);
                let positioner = wm_base.create_positioner(&qh, ());
                positioner.set_size(size.0, size.1);
                positioner.set_anchor_rect(anchor.0, anchor.1, anchor.2, anchor.3);
                positioner.set_anchor(xdg_positioner::Anchor::BottomRight);
                positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
                positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::all());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(index));
                let popup = xdg.get_popup(
                    Some(&windows[window].xdg),
                    &positioner,
                    &qh,
                    Role::Popup(index),
                );
                surface.commit();
                let geometry = wait_for(&mut queue, &mut client, "a popup configure", |client| {
                    client.popups[index]
                })?;
                popups.push((surface, xdg, popup, positioner));
                Ack::Popup(geometry)
            }
            Step::ReportPointer => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let entered = client.pointer_focus.as_ref().map(|focus| {
                    windows
                        .iter()
                        .position(|w| &w.surface == focus)
                        .map_or(Entered::Other, Entered::Window)
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
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// Maps a window and returns its index.
    fn map(&mut self, spec: Spec) -> usize {
        let index = self.state.windows.len();
        assert!(matches!(self.run(Step::Map(spec)), Ack::Done));
        index
    }

    fn done(&mut self, step: Step) {
        assert!(matches!(self.run(step), Ack::Done));
    }

    fn configures(&mut self, window: usize) -> Vec<Configured> {
        match self.run(Step::Configures { window }) {
            Ack::Configures(all) => all,
            _ => panic!("expected configures"),
        }
    }

    fn last_configure(&mut self, window: usize) -> Configured {
        *self.configures(window).last().expect("a configure")
    }

    fn popup(&mut self, window: usize, anchor: (i32, i32, i32, i32), size: (i32, i32)) -> Rect {
        match self.run(Step::Popup {
            window,
            anchor,
            size,
        }) {
            Ack::Popup((x, y, w, h)) => Rect::new(x, y, w, h),
            _ => panic!("expected a popup geometry"),
        }
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

    fn placement(&self, index: usize) -> Placement {
        *self
            .state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
    }

    fn floating(&self, index: usize) -> bool {
        self.state.world.is_floating(self.id(index))
    }

    fn act(&mut self, action: scoot_core::Action) {
        self.state.act(action);
        self.settle();
    }

    fn click(&mut self, x: f64, y: f64) {
        self.state.pointer_move(x, y);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, false);
        self.settle();
    }

    /// The window snapshot IPC `windows` reports for the `index`-th window.
    fn snapshot(&mut self, index: usize) -> scoot_ipc::WindowSnapshot {
        let id = self.id(index).0;
        match self.state.handle_request(scoot_ipc::Request::Windows) {
            scoot_ipc::Response::Windows { windows } => windows
                .into_iter()
                .find(|w| w.id == id)
                .expect("the window is listed"),
            other => panic!("expected windows, got {other:?}"),
        }
    }

    /// Every tiled window's rect: what "the strip" looks like.
    fn strip(&self) -> Vec<(WindowId, Rect)> {
        self.state
            .world
            .arrange()
            .placements
            .iter()
            .filter(|p| !p.floating)
            .map(|p| (p.id, p.rect))
            .collect()
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}

/// The centre of a rect, for "centred on" assertions.
fn centre(rect: Rect) -> (i32, i32) {
    (rect.x + rect.w / 2, rect.y + rect.h / 2)
}
