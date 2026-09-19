//! Tests for `zwp_tablet_manager_v2`.
//!
//! Every one of these drives a real `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the manager
//! global, takes a tablet seat off `wl_seat`, maps a painted toplevel, and
//! the test drives synthetic tool events through the same backend-neutral
//! `State::tablet_*` methods `tty/mod.rs`'s libinput arms call -- no
//! hardware, no uinput, no libinput in the loop (the dev VM has no
//! tablet-tool device: its QEMU tablet reports only the `pointer`
//! capability). The test then asserts on the tool *and* pointer events
//! that actually arrive on the wire: a pen moves the cursor and clicks,
//! and the test proves both halves rather than assuming one.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::input::{TabletToolCapabilities, TabletToolDescriptor, TabletToolType};
use smithay::input::tablet::TabletDescriptor;
use smithay::input::tablet::tool::AxisFrame;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::utils::{Logical, Point};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2, zwp_tablet_pad_v2, zwp_tablet_seat_v2, zwp_tablet_tool_v2, zwp_tablet_v2,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// The framebuffer square every test here renders into (nothing here reads
/// it back, but the headless backend behind it is what gives the compositor
/// its output -- and the output's extent is what the tool positions are
/// measured in, so a backend-less harness would pin every event at the
/// origin instead of moving it).
const CANVAS: i32 = 1600;

/// The side, in pixels, of the buffer each toplevel paints. Small, and far
/// smaller than the column the core lays the window out in -- so a test that
/// wants the tool over a window aims at the surface's own top-left corner,
/// not at the middle of its column (the shape `activation/tests.rs` uses).
const SURFACE: i32 = 64;

/// The pen every test drives: a pressure-capable pen with a fixed serial,
/// so the client can tell it apart from nothing else -- there is nothing
/// else, one tool per suite.
fn pen() -> TabletToolDescriptor {
    TabletToolDescriptor {
        tool_type: TabletToolType::Pen,
        hardware_serial: 42,
        hardware_id_wacom: 0,
        capabilities: TabletToolCapabilities::PRESSURE,
    }
}

/// The tablet the pen is in proximity with. The name is what the client's
/// `tablet_added` reporting counts, not what it matches on.
fn tablet() -> TabletDescriptor {
    TabletDescriptor {
        name: "test-tablet".to_string(),
        usb_id: Some((0x1234, 0x5678)),
        syspath: None,
    }
}

/// One instruction for the client thread.
enum Step {
    /// Report everything observed so far, clearing the buffers: pointer
    /// `enter`s/motions/buttons, keyboard `enter`s, and every tablet-seat
    /// and tool event since the last report (or startup).
    Report,
}

/// What a client answers a [`Step`] with.
enum Ack {
    /// Sent once at startup, before mapping: whether the registry round
    /// trip found the tablet-manager global.
    Started { manager: bool },
    /// The window is mapped and the client is parked holding it, carrying
    /// the toplevel's protocol id so the compositor side can aim the tool
    /// at it.
    Mapped { surface: u32 },
    /// The drained observations (see [`Step::Report`]).
    Report {
        pointer_enters: u32,
        pointer_motions: Vec<(f64, f64)>,
        pointer_buttons: Vec<u32>,
        keyboard_enters: u32,
        tablets: u32,
        tools: u32,
        proximity_in: u32,
        proximity_out: u32,
        tool_motions: Vec<(f64, f64)>,
        pressures: Vec<u32>,
        downs: u32,
        ups: u32,
        tool_buttons: Vec<(u32, u32)>,
    },
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A live compositor with one output and one connected client whose
    /// window is mapped, ready for the caller to point a tool at.
    fn start() -> (Self, Run) {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        let Ack::Started { manager } = fixture.wait_for_ack(0) else {
            panic!("the client reported being mapped before it reported its globals");
        };
        let Ack::Mapped { surface } = fixture.wait_for_ack(0) else {
            panic!("the client reported observations before it reported being mapped");
        };
        (fixture, Run { manager, surface })
    }

    /// The drained observations of client `index`.
    fn report(&mut self, index: usize) -> Report {
        let Ack::Report {
            pointer_enters,
            pointer_motions,
            pointer_buttons,
            keyboard_enters,
            tablets,
            tools,
            proximity_in,
            proximity_out,
            tool_motions,
            pressures,
            downs,
            ups,
            tool_buttons,
        } = self.run_on(index, Step::Report)
        else {
            panic!("client {index} answered a report with something else");
        };
        Report {
            pointer_enters,
            pointer_motions,
            pointer_buttons,
            keyboard_enters,
            tablets,
            tools,
            proximity_in,
            proximity_out,
            tool_motions,
            pressures,
            downs,
            ups,
            tool_buttons,
        }
    }

    /// Drives one tool proximity event through the compositor and flushes
    /// the clients, so the next report answers it.
    fn proximity(&mut self, entering: bool, x: f64, y: f64, axis: AxisFrame) {
        self.state
            .tablet_proximity(&tablet(), &pen(), entering, x, y, axis);
        let _ = self.state.display_handle.flush_clients();
    }

    /// Drives one tool motion event through the compositor and flushes.
    fn motion(&mut self, x: f64, y: f64, axis: AxisFrame) {
        self.state.tablet_motion(&pen(), x, y, axis);
        let _ = self.state.display_handle.flush_clients();
    }

    /// Drives one tip event (and its click) through the compositor and
    /// flushes.
    fn tip(&mut self, down: bool, x: f64, y: f64) {
        self.state.tablet_tip(&pen(), down, x, y);
        let _ = self.state.display_handle.flush_clients();
    }

    /// Drives one barrel-button event through the compositor and flushes.
    fn button(&mut self, button: u32, pressed: bool) {
        self.state.tablet_button(&pen(), button, pressed);
        let _ = self.state.display_handle.flush_clients();
    }
}

/// What [`Fixture::start`] hands back: whether the client found the manager
/// global, plus its window's protocol id.
struct Run {
    manager: bool,
    surface: u32,
}

/// A drained [`Ack::Report`], named so assertions read as field accesses.
#[derive(Debug, Default)]
struct Report {
    pointer_enters: u32,
    pointer_motions: Vec<(f64, f64)>,
    /// Pointer button states in order: 1 is press, 0 is release.
    pointer_buttons: Vec<u32>,
    keyboard_enters: u32,
    tablets: u32,
    tools: u32,
    proximity_in: u32,
    proximity_out: u32,
    /// Surface-local tool motion coordinates.
    tool_motions: Vec<(f64, f64)>,
    pressures: Vec<u32>,
    downs: u32,
    ups: u32,
    /// `(button, state)` per tool button event: state 1 is press.
    tool_buttons: Vec<(u32, u32)>,
}

/// A compositor point inside client `index`'s window: the window's space
/// origin plus an offset inside its painted surface.
///
/// The surface is `SURFACE` pixels square at the window's own origin (the
/// shape `activation/tests.rs` relies on), so (30, 30) is well inside
/// whatever ring the decorations draw around it.
fn window_point(fixture: &Fixture, index: usize, surface: u32) -> (f64, f64) {
    let target: ServerSurface = fixture
        .client(index)
        .object_from_protocol_id(&fixture.state.display_handle, surface)
        .expect("the compositor still holds the client's surface");
    let window = fixture
        .state
        .space
        .elements()
        .find_map(|window| {
            let toplevel = window.toplevel()?;
            (toplevel.wl_surface() == &target).then(|| {
                fixture
                    .state
                    .space
                    .element_location(window)
                    .expect("a mapped window has a location")
            })
        })
        .expect("the client's surface is a mapped window");
    (f64::from(window.x) + 30.0, f64::from(window.y) + 30.0)
}

/// The surface-local coordinates behind a compositor point, resolved
/// through the compositor's own hit test rather than re-derived from the
/// layout, so a decoration offset can never silently skew an expectation.
fn surface_local(fixture: &Fixture, x: f64, y: f64) -> (f64, f64) {
    let (_, origin) = fixture
        .state
        .surface_under(Point::<f64, Logical>::from((x, y)))
        .expect("the aim point is over no surface");
    (x - origin.x, y - origin.y)
}

/// The client end of one test connection: enough of a toolkit to map a
/// painted toplevel, hold a `wl_pointer` and a `wl_keyboard` on its seat,
/// and take a tablet seat off the tablet manager to watch tools through.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    surface: Option<wl_surface::WlSurface>,
    /// The tablet manager, bound at registry time; the seat object is taken
    /// once the `wl_seat` exists (see `run_client`). `None` is what the
    /// advertisement test fails on.
    manager: Option<zwp_tablet_manager_v2::ZwpTabletManagerV2>,
    tablet_seat: Option<zwp_tablet_seat_v2::ZwpTabletSeatV2>,
    pointer_enters: u32,
    pointer_motions: Vec<(f64, f64)>,
    pointer_buttons: Vec<u32>,
    keyboard_enters: u32,
    tablets: u32,
    tools: u32,
    proximity_in: u32,
    proximity_out: u32,
    tool_motions: Vec<(f64, f64)>,
    pressures: Vec<u32>,
    downs: u32,
    ups: u32,
    tool_buttons: Vec<(u32, u32)>,
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
        // Version 1 of each is all this needs, so ask for exactly that and
        // stay independent of what the compositor advertises.
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name {
            client.seat = Some(registry.bind(name, version.min(5), qh, ()));
        } else if interface == zwp_tablet_manager_v2::ZwpTabletManagerV2::interface().name {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl TestClient {
    /// Whether the registry round trip found the tablet-manager global.
    fn globals(&self) -> bool {
        self.manager.is_some()
    }

    /// The drained observations, clearing the buffers.
    fn drain(&mut self) -> Ack {
        let report = Ack::Report {
            pointer_enters: self.pointer_enters,
            pointer_motions: std::mem::take(&mut self.pointer_motions),
            pointer_buttons: std::mem::take(&mut self.pointer_buttons),
            keyboard_enters: self.keyboard_enters,
            tablets: self.tablets,
            tools: self.tools,
            proximity_in: self.proximity_in,
            proximity_out: self.proximity_out,
            tool_motions: std::mem::take(&mut self.tool_motions),
            pressures: std::mem::take(&mut self.pressures),
            downs: self.downs,
            ups: self.ups,
            tool_buttons: std::mem::take(&mut self.tool_buttons),
        };
        self.pointer_enters = 0;
        self.keyboard_enters = 0;
        self.tablets = 0;
        self.tools = 0;
        self.proximity_in = 0;
        self.proximity_out = 0;
        self.downs = 0;
        self.ups = 0;
        report
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
            wl_pointer::Event::Enter { .. } => client.pointer_enters += 1,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => client.pointer_motions.push((surface_x, surface_y)),
            wl_pointer::Event::Button { state, .. } => {
                client.pointer_buttons.push(u32::from(
                    state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed),
                ));
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
        if let wl_keyboard::Event::Enter { .. } = event {
            client.keyboard_enters += 1;
        }
    }
}

impl Dispatch<zwp_tablet_manager_v2::ZwpTabletManagerV2, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_tablet_manager_v2::ZwpTabletManagerV2,
        _: zwp_tablet_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_tablet_seat_v2::ZwpTabletSeatV2, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_tablet_seat_v2::ZwpTabletSeatV2,
        event: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_seat_v2::Event::TabletAdded { .. } => client.tablets += 1,
            zwp_tablet_seat_v2::Event::ToolAdded { .. } => client.tools += 1,
            _ => {}
        }
    }

    // The seat's `tablet_added`/`tool_added` events carry `new_id`s the
    // client materializes into proxies: without this the queue panics on
    // the first tablet announcement (`Missing event_created_child
    // specialization`). Opcodes are the XML's event order: 0 is
    // `tablet_added`, 1 is `tool_added`, 2 is `pad_added` (which never
    // fires -- see the module doc -- but must still map).
    wayland_client::event_created_child!(TestClient, zwp_tablet_seat_v2::ZwpTabletSeatV2, [
        0 => (zwp_tablet_v2::ZwpTabletV2, ()),
        1 => (zwp_tablet_tool_v2::ZwpTabletToolV2, ()),
        2 => (zwp_tablet_pad_v2::ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_v2::ZwpTabletV2, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_tablet_v2::ZwpTabletV2,
        _: zwp_tablet_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_tablet_tool_v2::ZwpTabletToolV2, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_tablet_tool_v2::ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_tool_v2::Event::ProximityIn { .. } => client.proximity_in += 1,
            zwp_tablet_tool_v2::Event::ProximityOut => client.proximity_out += 1,
            zwp_tablet_tool_v2::Event::Motion { x, y } => client.tool_motions.push((x, y)),
            zwp_tablet_tool_v2::Event::Pressure { pressure } => client.pressures.push(pressure),
            zwp_tablet_tool_v2::Event::Down { .. } => client.downs += 1,
            zwp_tablet_tool_v2::Event::Up => client.ups += 1,
            zwp_tablet_tool_v2::Event::Button {
                button,
                state,
                serial: _,
            } => {
                client.tool_buttons.push((button, state.into()));
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

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore zwp_tablet_pad_v2::ZwpTabletPadV2);

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque pixels, over a real memfd --
/// the same path any toolkit takes (the shape `activation/tests.rs` uses).
///
/// Needed for one reason: a toplevel that never attaches a buffer has an
/// empty bounding box, so `Space::element_under` can never find it and the
/// tool can never be over anything.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("scoot-tablet-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        SURFACE,
        SURFACE,
        stride,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let pointer = seat.get_pointer(&qh, ());
    client.pointer = Some(pointer);
    let keyboard = seat.get_keyboard(&qh, ());
    client.keyboard = Some(keyboard);
    // The tablet seat every test below watches tools through. Taken up
    // front: the protocol hangs every tablet and tool off it, and
    // re-taking it per step would be a second seat for the same `wl_seat`
    // rather than a fresh start.
    if let Some(manager) = client.manager.clone() {
        client.tablet_seat = Some(manager.get_tablet_seat(&seat, &qh, ()));
    }

    // Mapped in two commits, the way the protocol asks: the role-only commit
    // first, then pixels once the compositor's configure has been acked (the
    // `xdg_surface` handler above does that as the event arrives).
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title("tablet".to_string());
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    surface.attach(Some(&solid_buffer(&shm, &qh)?), 0, 0);
    surface.damage(0, 0, SURFACE, SURFACE);
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    client.surface = Some(surface.clone());
    let surface_id = surface.id().protocol_id();

    // Two startup answers, in order: the globals report first (it is what
    // the advertisement test asserts on), then the mapped report the
    // compositor side waits for before pointing a tool at the window.
    acks.send(Ack::Started {
        manager: client.globals(),
    })
    .map_err(|e| e.to_string())?;
    acks.send(Ack::Mapped {
        surface: surface_id,
    })
    .map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        // Drain anything the compositor sent since the last step -- in
        // particular the `enter`/`motion`/tool events the test just caused
        // -- before acting, so a report answers what happened rather than
        // what is still in flight.
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::Report => {
                acks.send(client.drain()).map_err(|e| e.to_string())?;
                continue;
            }
        }
    }
    Ok(())
}

/// The manager global is advertised: the client's registry round trip found
/// `zwp_tablet_manager_v2`.
#[test]
fn tablet_manager_is_advertised() {
    let (mut fixture, run) = Fixture::start();
    assert!(
        run.manager,
        "the client never saw zwp_tablet_manager_v2 in the registry"
    );
    let _ = fixture.report(0);
}

/// Proximity announces the tablet and the tool, and the cursor follows the
/// pen: the client sees `tablet_added` + `tool_added` on its tablet seat
/// and `proximity_in` + `motion` on the tool. The pointer half arrives as
/// an `enter` -- Smithay sends no `motion` on a focus change, only the
/// enter carrying the arrival point, so the motion half is proven by the
/// follow-up move below, not by the arrival itself.
#[test]
fn pen_proximity_announces_tablet_and_tool() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    let (x, y) = window_point(&fixture, 0, run.surface);
    let (lx, ly) = surface_local(&fixture, x, y);
    fixture.proximity(true, x, y, AxisFrame::new().pressure(0.5));
    let report = fixture.report(0);
    assert_eq!(report.tablets, 1, "no tablet_added for the pen's tablet");
    assert_eq!(report.tools, 1, "no tool_added for the pen");
    assert_eq!(report.proximity_in, 1, "no proximity_in on the tool");
    assert_eq!(
        report.tool_motions,
        vec![(lx, ly)],
        "the tool never saw the arrival motion at surface-local coordinates"
    );
    // Half pressure, truncated onto the wire: 0.5 * 65535 = 32767.5, and
    // Smithay's `normalize` casts rather than rounds.
    assert_eq!(report.pressures, vec![32767]);
    assert!(
        report.pointer_enters >= 1,
        "the cursor never entered the window the pen arrived over"
    );
    // The cursor follows the pen: a move after the arrival reports a
    // pointer motion at the new surface-local coordinates.
    let (x2, y2) = (x + 10.0, y + 8.0);
    let (lx2, ly2) = surface_local(&fixture, x2, y2);
    fixture.motion(x2, y2, AxisFrame::new());
    let moved = fixture.report(0);
    assert_eq!(
        moved.tool_motions,
        vec![(lx2, ly2)],
        "the tool motion arrived at the wrong surface-local coordinates"
    );
    assert_eq!(
        moved.pointer_motions,
        vec![(lx2, ly2)],
        "the cursor never followed the pen: {:?}",
        moved.pointer_motions
    );
}

/// A pen lifting and approaching again does not re-announce the tablet:
/// the client sees one `tablet_added` for the physical device, then a
/// second `proximity_in` on the already-known tool. Re-adding per approach
/// would make clients watch one device vanish (`removed`) and reappear on
/// every pen lift.
#[test]
fn second_proximity_in_does_not_reannounce_the_tablet() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.proximity(true, x, y, AxisFrame::new());
    let first = fixture.report(0);
    assert_eq!(first.tablets, 1, "no tablet_added for the pen's tablet");
    assert_eq!(first.tools, 1, "no tool_added for the pen");
    fixture.proximity(false, x, y, AxisFrame::new());
    let _ = fixture.report(0);
    fixture.proximity(true, x, y, AxisFrame::new());
    let second = fixture.report(0);
    assert_eq!(
        second.tablets, 0,
        "the second approach re-announced the tablet"
    );
    assert_eq!(second.tools, 0, "the second approach re-announced the tool");
    assert_eq!(
        second.proximity_in, 1,
        "the second approach produced no proximity_in"
    );
}

/// Motion after proximity reports the tool's movement at surface-local
/// coordinates and keeps the cursor with it. The pressure axis rides along
/// only when it changed: a hover motion carries no pressure event.
#[test]
fn pen_motion_reports_movement_without_stale_pressure() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.proximity(true, x, y, AxisFrame::new().pressure(0.5));
    let _ = fixture.report(0);
    let (x2, y2) = (x + 10.0, y + 8.0);
    let (lx, ly) = surface_local(&fixture, x2, y2);
    fixture.motion(x2, y2, AxisFrame::new());
    let report = fixture.report(0);
    assert_eq!(
        report.tool_motions,
        vec![(lx, ly)],
        "the tool motion arrived at the wrong surface-local coordinates"
    );
    assert!(
        report.pressures.is_empty(),
        "an unchanged pressure axis was restated: {:?}",
        report.pressures
    );
    assert!(
        !report.pointer_motions.is_empty(),
        "the cursor stopped following the pen after proximity"
    );
}

/// A pen tap clicks: tip down delivers the tool's `down` *and* a pointer
/// press, tip up the tool's `up` *and* the release -- and the press moves
/// keyboard focus onto the tapped window, like a mouse click.
#[test]
fn pen_tip_clicks_and_focuses_like_a_mouse() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    // A second window, mapped last so it holds keyboard focus: the tap has
    // to *move* focus to prove anything, not merely land on it.
    fixture.spawn(run_client);
    let Ack::Started { manager } = fixture.wait_for_ack(1) else {
        panic!("the second client reported being mapped before its globals");
    };
    assert!(manager);
    let Ack::Mapped { surface: _ } = fixture.wait_for_ack(1) else {
        panic!("the second client reported observations before being mapped");
    };
    let _ = fixture.report(0);
    let second = fixture.report(1);
    assert!(
        second.keyboard_enters >= 1,
        "mapping the second window never gave it keyboard focus; \
         the tap below would prove nothing"
    );
    // Tap the first window: down, then up, draining in between so the two
    // halves answer separately.
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.proximity(true, x, y, AxisFrame::new());
    let _ = fixture.report(0);
    fixture.tip(true, x, y);
    let down = fixture.report(0);
    assert_eq!(down.downs, 1, "no tool down for the pen tap");
    assert_eq!(
        down.pointer_buttons,
        vec![1],
        "the pen tap never pressed the pointer: {:?}",
        down.pointer_buttons
    );
    // The press is what focuses: the keyboard `enter` arrives with the
    // down report, not the up one -- assert it here, before this report
    // is drained.
    assert!(
        down.keyboard_enters >= 1,
        "the pen tap never moved keyboard focus onto the tapped window"
    );
    fixture.tip(false, x, y);
    let up = fixture.report(0);
    assert_eq!(up.ups, 1, "no tool up for the pen lift");
    assert_eq!(
        up.pointer_buttons,
        vec![0],
        "the pen lift never released the pointer: {:?}",
        up.pointer_buttons
    );
}

/// A barrel button is tool-only: the tool sees the exact button number and
/// state, and the pointer sees no click at all -- there is no mapping from
/// a stylus button onto a mouse button to synthesize.
#[test]
fn pen_barrel_button_is_tool_only() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.proximity(true, x, y, AxisFrame::new());
    let _ = fixture.report(0);
    fixture.button(3, true);
    fixture.button(3, false);
    let report = fixture.report(0);
    assert_eq!(
        report.tool_buttons,
        vec![(3, 1), (3, 0)],
        "the barrel button never reached the tool intact: {:?}",
        report.tool_buttons
    );
    assert!(
        report.pointer_buttons.is_empty(),
        "a stylus button synthesized a pointer click: {:?}",
        report.pointer_buttons
    );
}

/// Leaving proximity ends the tool stream but parks the cursor: the tool
/// sees `proximity_out`, and later motion still moves the pointer (the
/// device is physically there) while the tool -- correctly -- hears
/// nothing more.
#[test]
fn pen_proximity_out_parks_cursor_and_ends_tool_stream() {
    let (mut fixture, run) = Fixture::start();
    assert!(run.manager);
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.proximity(true, x, y, AxisFrame::new());
    let _ = fixture.report(0);
    fixture.proximity(false, x, y, AxisFrame::new());
    let out = fixture.report(0);
    assert_eq!(out.proximity_out, 1, "no proximity_out on the tool");
    fixture.motion(x + 5.0, y, AxisFrame::new());
    let after = fixture.report(0);
    assert!(
        after.tool_motions.is_empty(),
        "the tool heard motion after leaving proximity: {:?}",
        after.tool_motions
    );
    assert!(
        !after.pointer_motions.is_empty(),
        "the cursor froze when the tool left proximity"
    );
}

/// Unit pins, no compositor: the synthetic descriptors the tests drive
/// through `State::tablet_*` name a pen and a tablet, so a client reading
/// the `type` and `name` events sees what was actually announced.
#[test]
fn synthetic_tool_names_a_pen() {
    assert_eq!(pen().tool_type, TabletToolType::Pen);
    assert_eq!(pen().hardware_serial, 42);
    assert_eq!(tablet().name, "test-tablet");
}

/// `ButtonState::Pressed` is 1 on the pointer wire: the tap test above
/// asserts `pointer_buttons == vec![1]`, which only means "press" under
/// this mapping. Pinned here rather than assumed from the toolkit.
#[test]
fn pointer_button_state_mapping() {
    use wayland_client::protocol::wl_pointer::ButtonState;
    assert_eq!(ButtonState::Pressed as u32, 1);
    assert_eq!(ButtonState::Released as u32, 0);
}
