//! Popup constraint adjustment, driven through a real `wayland-client`
//! connection.
//!
//! Every test asserts on what the *client* was told -- the `x`, `y`,
//! `width`, `height` of the `xdg_popup.configure` it received, which is
//! the geometry relative to its parent's window geometry that the protocol
//! defines -- and, where "stays on its own output" is the claim, on real
//! framebuffer pixels. None asserts on a helper's return value: a helper
//! that computed the right rectangle and a handler that never used it would
//! pass that kind of test.
//!
//! Geometry, on a 200-square canvas with the default 12px gap: one window on
//! an output sits at `x = 12` (its left gap) and `y = 12` (its top gap, or
//! below a bar's exclusive zone plus the gap). The tests read the window's
//! rect back from the arrangement rather than hard-coding it, and express
//! each popup in terms of the output's edges, so what they pin is the rule
//! ("keep it inside this rectangle"), not the layout's arithmetic.
//!
//! This file is the harness; the tests live in the submodules below, by
//! concern. Like every live-`State` suite here, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::{OutputId, Rect, WindowId};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless;
use crate::compositor::test_support::{self, Harness, wait_for};

mod adversarial;
mod layer;
mod pure;
mod window;

use xdg_positioner::{Anchor, ConstraintAdjustment as Adjust, Gravity};

/// Each output's framebuffer, square.
const CANVAS: i32 = 200;
/// `Config::default`'s gap.
const GAP: i32 = 12;

// Colours, as the BGRA bytes an `Argb8888` buffer holds them in.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const BAR_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const POPUP_BGRA: [u8; 4] = [0x20, 0xE0, 0xE0, 0xFF];

fn appearance() -> Appearance {
    Appearance {
        // No ring: nothing here is about it, and it would only be one more
        // colour to keep distinct from the popup's.
        focus_ring_width: 0,
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

/// Everything a positioner is built from, in one value.
#[derive(Clone, Copy, Debug)]
struct Spec {
    size: (i32, i32),
    anchor_rect: (i32, i32, i32, i32),
    anchor: Anchor,
    gravity: Gravity,
    offset: (i32, i32),
    adjust: Adjust,
}

impl Spec {
    /// A `w` x `h` popup whose top-left corner sits at `(x, y)` in the
    /// parent's window geometry, growing right and down from a 1x1 anchor
    /// there: the shape of a context menu opened at the pointer.
    fn menu_at(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            size: (w, h),
            anchor_rect: (x, y, 1, 1),
            anchor: Anchor::TopLeft,
            gravity: Gravity::BottomRight,
            offset: (0, 0),
            adjust: Adjust::empty(),
        }
    }

    fn adjust(self, adjust: Adjust) -> Self {
        Self { adjust, ..self }
    }
}

/// What a popup hangs off.
#[derive(Clone, Copy, Debug)]
enum Parent {
    /// The `n`-th toplevel this client mapped.
    Window(usize),
    /// The `n`-th popup this client made.
    Popup(usize),
    /// The `n`-th layer surface this client mapped, through
    /// `zwlr_layer_surface_v1.get_popup` on a popup created parentless.
    Layer(usize),
}

enum Step {
    /// Create a toplevel, commit without a buffer, ack the configure that
    /// answers and draw at the size it names.
    MapWindow,
    /// `xdg_toplevel.set_fullscreen` with no output; ack the configure that
    /// answers and redraw at its size.
    Fullscreen { window: usize },
    /// A top-layer bar anchored to the top edge, `height` tall, reserving
    /// `exclusive` pixels, on the `output`-th `wl_output` (0-based, creation
    /// order).
    MapBar {
        output: usize,
        height: u32,
        exclusive: i32,
    },
    /// Create, configure, ack and map a popup; answers with the geometry
    /// its configure carried.
    Popup { parent: Parent, spec: Spec },
    /// Create a popup and commit it once, without waiting for (or expecting)
    /// a configure. Answers `Done`.
    CommitPopup { parent: Parent, spec: Spec },
    /// A popup created with no parent that nothing ever adopts -- never
    /// committed (committing it would be a protocol error). Answers `Done`.
    OrphanPopup,
    /// A popup whose parent is its own `xdg_surface`. Expected to be refused
    /// with a protocol error, which ends this client.
    SelfParentPopup,
    /// A two-popup loop: popup `A` of a bare, role-less `xdg_surface` `X`,
    /// then `X` made a popup whose parent is `A`. Expected to be refused
    /// like [`Step::SelfParentPopup`].
    LoopingPopups,
    /// `xdg_popup.reposition` with `spec` and `token`; ack and commit the
    /// configure that answers. Answers with the geometry it carried, and
    /// the token the `repositioned` event echoed.
    Reposition {
        popup: usize,
        spec: Spec,
        token: u32,
    },
}

/// A popup's configured geometry: `(x, y, width, height)`, relative to its
/// parent's window geometry.
type Geometry = (i32, i32, i32, i32);

#[derive(Debug)]
enum Ack {
    Done,
    Popup(Geometry),
    Repositioned { geometry: Geometry, token: u32 },
}

/// Which object an event belongs to.
#[derive(Clone, Copy)]
enum Role {
    Window(usize),
    Popup(usize),
    Layer(usize),
}

#[derive(Default)]
struct PopupRecord {
    geometry: Option<Geometry>,
    serial: Option<u32>,
    token: Option<u32>,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// Every `wl_output`, in the order the registry announced them -- the
    /// order the compositor created them in.
    outputs: Vec<wl_output::WlOutput>,
    /// Per toplevel: the size its pending `xdg_toplevel.configure` named.
    pending: Vec<(i32, i32)>,
    /// Per toplevel: every completed configure, `(serial, width, height)`.
    configures: Vec<Vec<(u32, i32, i32)>>,
    /// Per layer surface: the size of its newest (already acked) configure.
    layer_sizes: Vec<Option<(u32, u32)>>,
    popups: Vec<PopupRecord>,
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
            // v3: `xdg_popup.reposition` and `repositioned`.
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_output" => client
                .outputs
                .push(registry.bind(name, version.min(4), qh, ())),
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
        client: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Role::Popup(index) = *role else {
            return;
        };
        let Some(record) = client.popups.get_mut(index) else {
            return;
        };
        match event {
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            } => record.geometry = Some((x, y, width, height)),
            xdg_popup::Event::Repositioned { token } => record.token = Some(token),
            _ => {}
        }
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
                    && let Some(seen) = client.configures.get_mut(index)
                {
                    seen.push((serial, width, height));
                }
            }
            Role::Popup(index) => {
                if let Some(record) = client.popups.get_mut(index) {
                    record.serial = Some(serial);
                }
            }
            Role::Layer(_) => {}
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
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A `width`x`height` buffer of `color` over a real memfd. A zero size (a
/// configure that left the size to the client) draws 40 square.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> (wl_buffer::WlBuffer, i32, i32) {
    let width = if width > 0 { width } else { 40 };
    let height = if height > 0 { height } else { 40 };
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create(
        "scoot-popup-constraint-test",
        rustix::fs::MemfdFlags::CLOEXEC,
    )
    .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    (buffer, width, height)
}

fn positioner(
    wm_base: &xdg_wm_base::XdgWmBase,
    qh: &QueueHandle<TestClient>,
    spec: Spec,
) -> xdg_positioner::XdgPositioner {
    let positioner = wm_base.create_positioner(qh, ());
    positioner.set_size(spec.size.0, spec.size.1);
    let (x, y, w, h) = spec.anchor_rect;
    positioner.set_anchor_rect(x, y, w, h);
    positioner.set_anchor(spec.anchor);
    positioner.set_gravity(spec.gravity);
    positioner.set_offset(spec.offset.0, spec.offset.1);
    positioner.set_constraint_adjustment(spec.adjust);
    positioner
}

struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
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
/// the run: a popup whose objects drop is dismissed.
#[derive(Default)]
struct Made {
    windows: Vec<Toplevel>,
    popups: Vec<Popup>,
    layers: Vec<Layer>,
}

impl Made {
    /// `xdg_surface.get_popup` on `xdg` for `parent` -- for a layer surface,
    /// parentless and then adopted through `zwlr_layer_surface_v1.get_popup`,
    /// the way a bar builds its dropdown.
    fn get_popup(
        &self,
        xdg: &xdg_surface::XdgSurface,
        positioner: &xdg_positioner::XdgPositioner,
        qh: &QueueHandle<TestClient>,
        index: usize,
        parent: Parent,
    ) -> Result<xdg_popup::XdgPopup, String> {
        Ok(match parent {
            Parent::Window(i) => {
                let parent = &self.windows.get(i).ok_or("no such window")?.xdg;
                xdg.get_popup(Some(parent), positioner, qh, Role::Popup(index))
            }
            Parent::Popup(i) => {
                let parent = &self.popups.get(i).ok_or("no such popup")?.xdg;
                xdg.get_popup(Some(parent), positioner, qh, Role::Popup(index))
            }
            Parent::Layer(i) => {
                let popup = xdg.get_popup(None, positioner, qh, Role::Popup(index));
                self.layers
                    .get(i)
                    .ok_or("no such layer")?
                    .layer
                    .get_popup(&popup);
                popup
            }
        })
    }
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

/// Acks the newest configure and draws at its size.
fn draw(
    client: &TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    window: &Toplevel,
    index: usize,
) -> Result<(), String> {
    let (serial, width, height) = client
        .configures
        .get(index)
        .and_then(|all| all.last().copied())
        .ok_or("no configure to draw for")?;
    window.xdg.ack_configure(serial);
    let (buffer, width, height) = solid_buffer(shm, qh, width, height, WINDOW_BGRA);
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage(0, 0, width, height);
    window.surface.commit();
    Ok(())
}

/// Waits for popup `index`'s configure, acks it, and draws at the size it
/// names. Hands back the geometry.
fn map_popup(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    popup: &Popup,
    index: usize,
) -> Result<Geometry, String> {
    let (serial, geometry) = wait_for(queue, client, "a popup configure", |client| {
        let record = client.popups.get(index)?;
        Some((record.serial?, record.geometry?))
    })?;
    popup.xdg.ack_configure(serial);
    let (buffer, w, h) = solid_buffer(shm, qh, geometry.2, geometry.3, POPUP_BGRA);
    popup.surface.attach(Some(&buffer), 0, 0);
    popup.surface.damage(0, 0, w, h);
    popup.surface.commit();
    queue.roundtrip(client).map_err(|e| e.to_string())?;
    Ok(geometry)
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

    let mut made = Made::default();
    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::MapWindow => {
                let index = made.windows.len();
                client.pending.push((0, 0));
                client.configures.push(Vec::new());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Window(index));
                let toplevel = xdg.get_toplevel(&qh, Role::Window(index));
                surface.commit();
                wait_for_configure(&mut queue, &mut client, index, 0)?;
                let window = Toplevel {
                    surface,
                    xdg,
                    toplevel,
                };
                draw(&client, &qh, &shm, &window, index)?;
                made.windows.push(window);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Fullscreen { window } => {
                let seen = client.configures[window].len();
                made.windows[window].toplevel.set_fullscreen(None);
                wait_for_configure(&mut queue, &mut client, window, seen)?;
                draw(&client, &qh, &shm, &made.windows[window], window)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::MapBar {
                output,
                height,
                exclusive,
            } => {
                let index = made.layers.len();
                client.layer_sizes.push(None);
                let surface = compositor.create_surface(&qh, ());
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    Some(client.outputs.get(output).ok_or("no such wl_output")?),
                    zwlr_layer_shell_v1::Layer::Top,
                    "bar".into(),
                    &qh,
                    Role::Layer(index),
                );
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer.set_size(0, height);
                layer.set_exclusive_zone(exclusive);
                surface.commit();
                let (width, height) =
                    wait_for(&mut queue, &mut client, "a layer configure", |client| {
                        client.layer_sizes[index]
                    })?;
                let (buffer, w, h) = solid_buffer(&shm, &qh, width as i32, height as i32, BAR_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, w, h);
                surface.commit();
                made.layers.push(Layer {
                    _surface: surface,
                    layer,
                });
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Popup { parent, spec } => {
                let index = made.popups.len();
                client.popups.push(PopupRecord::default());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(index));
                let positioner = positioner(&wm_base, &qh, spec);
                let popup = made.get_popup(&xdg, &positioner, &qh, index, parent)?;
                positioner.destroy();
                surface.commit();
                let popup = Popup {
                    surface,
                    xdg,
                    popup,
                };
                let geometry = map_popup(&mut queue, &mut client, &qh, &shm, &popup, index)?;
                made.popups.push(popup);
                Ack::Popup(geometry)
            }
            Step::CommitPopup { parent, spec } => {
                let index = made.popups.len();
                client.popups.push(PopupRecord::default());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(index));
                let positioner = positioner(&wm_base, &qh, spec);
                let popup = made.get_popup(&xdg, &positioner, &qh, index, parent)?;
                positioner.destroy();
                surface.commit();
                made.popups.push(Popup {
                    surface,
                    xdg,
                    popup,
                });
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::SelfParentPopup => {
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(usize::MAX));
                let positioner = positioner(&wm_base, &qh, Spec::menu_at(0, 0, 40, 40));
                let popup = xdg.get_popup(Some(&xdg), &positioner, &qh, Role::Popup(usize::MAX));
                // In the same flush: a reposition walks up the popup's parent
                // chain too, and must not find the loop still there.
                popup.reposition(&positioner, 1);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                return Err("a self-parented popup was not refused".into());
            }
            Step::LoopingPopups => {
                let bare = compositor.create_surface(&qh, ());
                let bare_xdg = wm_base.get_xdg_surface(&bare, &qh, Role::Popup(usize::MAX));
                let child = compositor.create_surface(&qh, ());
                let child_xdg = wm_base.get_xdg_surface(&child, &qh, Role::Popup(usize::MAX));
                let positioner = positioner(&wm_base, &qh, Spec::menu_at(0, 0, 40, 40));
                let child_popup =
                    child_xdg.get_popup(Some(&bare_xdg), &positioner, &qh, Role::Popup(usize::MAX));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let bare_popup =
                    bare_xdg.get_popup(Some(&child_xdg), &positioner, &qh, Role::Popup(usize::MAX));
                // In the same flush, on both popups of the loop (one refused,
                // one accepted and tracked): each reposition walks up the chain.
                bare_popup.reposition(&positioner, 1);
                child_popup.reposition(&positioner, 2);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                return Err("a looping popup chain was not refused".into());
            }
            Step::OrphanPopup => {
                let index = made.popups.len();
                client.popups.push(PopupRecord::default());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(index));
                let positioner = positioner(&wm_base, &qh, Spec::menu_at(0, 0, 40, 40));
                let popup = xdg.get_popup(None, &positioner, &qh, Role::Popup(index));
                positioner.destroy();
                made.popups.push(Popup {
                    surface,
                    xdg,
                    popup,
                });
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Reposition { popup, spec, token } => {
                let record = client.popups.get_mut(popup).ok_or("no such popup")?;
                record.serial = None;
                record.geometry = None;
                record.token = None;
                let target = made.popups.get(popup).ok_or("no such popup")?;
                let positioner = positioner(&wm_base, &qh, spec);
                target.popup.reposition(&positioner, token);
                positioner.destroy();
                let (serial, geometry, token) =
                    wait_for(&mut queue, &mut client, "a reposition", |client| {
                        let record = client.popups.get(popup)?;
                        Some((record.serial?, record.geometry?, record.token?))
                    })?;
                target.xdg.ack_configure(serial);
                target.surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Repositioned { geometry, token }
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn one_output() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// Two side-by-side outputs, `x = 0..200` and `200..400`.
    fn two_outputs() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
            .expect("a second output");
        fixture.settle();
        fixture.spawn(run_client);
        fixture
    }

    /// Parks the pointer in the middle of output `index` (1-based), which is
    /// where the next window or bar opens.
    fn pointer_on(&mut self, index: u64) {
        let output = self.output_rect(index);
        self.state.pointer_move(
            f64::from(output.x + output.w / 2),
            f64::from(output.y + output.h / 2),
        );
        self.settle();
    }

    /// Output `index`'s (1-based) global logical rectangle.
    fn output_rect(&self, index: u64) -> Rect {
        let output = self.state.outputs.get(OutputId(index)).expect("an output");
        let geometry = self
            .state
            .space
            .output_geometry(output)
            .expect("a mapped output");
        Rect::new(
            geometry.loc.x,
            geometry.loc.y,
            geometry.size.w,
            geometry.size.h,
        )
    }

    fn done(&mut self, step: Step) {
        let ack = self.run(step);
        assert!(matches!(ack, Ack::Done), "{ack:?}");
    }

    /// Maps a window and hands back its index.
    fn window(&mut self) -> usize {
        let index = self.state.windows.len();
        self.done(Step::MapWindow);
        index
    }

    fn popup(&mut self, parent: Parent, spec: Spec) -> Geometry {
        match self.run(Step::Popup { parent, spec }) {
            Ack::Popup(geometry) => geometry,
            other => panic!("expected a popup configure, got {other:?}"),
        }
    }

    /// The core id of the `index`-th window this client mapped (ids only
    /// increment, and each suite here has one client).
    fn id(&self, index: usize) -> WindowId {
        let mut ids: Vec<WindowId> = self.state.windows.keys().copied().collect();
        ids.sort();
        ids[index]
    }

    /// Window `index`'s placement rect -- where its window geometry origin
    /// is mapped, in global coordinates.
    fn rect_of(&self, index: usize) -> Rect {
        self.state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
            .rect
    }

    /// Draws every output and hands back output `index`'s (1-based) pixels.
    fn frame(&mut self, index: u64) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        self.pixels_of(OutputId(index))
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}
