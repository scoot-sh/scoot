//! Tests for `zwp_relative_pointer_manager_v1` and the
//! `zwp_pointer_constraints_v1` global it pairs with.
//!
//! Every one of these drives *real* `wayland-client` connections through a
//! real [`State`](crate::compositor::State): the client binds both manager
//! globals, maps a painted toplevel, takes a pointer lock or confinement
//! through the constraints global, and the test asserts on the
//! `relative_motion` events that actually arrive on the wire -- pinned to
//! exact numbers, not just event presence. The deltas a test asserts are
//! derived from the motion the *test* injected (`pointer_move` positions or
//! `pointer_move_relative` pairs), so a compositor that emitted the wrong
//! vector fails the assertion rather than merely emitting nothing.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::utils::{Logical, Point};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};
use flexwm_ipc::PointerButton;

/// The framebuffer square every test here renders into (nothing here reads
/// it back, but the headless backend behind it is what gives the compositor
/// its output -- and the output's extent is what `pointer_move_relative`
/// clamps against, so a backend-less harness would pin every such move at
/// the origin instead of moving it). Wide enough for two windows to sit side
/// by side with room for the pointer to travel between them.
const CANVAS: i32 = 1600;

/// The side, in pixels, of the buffer each toplevel paints. Small, and far
/// smaller than the column the core lays the window out in -- so a test that
/// wants the pointer over a window aims at the surface's own top-left
/// corner, not at the middle of its column (the shape
/// `activation/tests.rs` uses).
const SURFACE: i32 = 64;

/// One instruction for the client thread.
enum Step {
    /// Report everything observed so far, clearing the buffers: `enter`s,
    /// `wl_pointer.motion`s, `relative_motion`s and lock/confinement
    /// transitions since the last report (or startup).
    Report,
    /// `lock_pointer` on the window with no region and a `Persistent`
    /// lifetime, waiting for the `locked` event.
    Lock,
    /// `lock_pointer` like [`Step::Lock`], but without waiting: for taking
    /// a lock with no focus, where no `locked` event can arrive yet.
    Arm,
    /// Destroy the locked pointer, releasing the lock.
    Unlock,
    /// `confine_pointer` on the window with no region and a `Persistent`
    /// lifetime, waiting for the `confined` event.
    Confine,
    /// Destroy the confined pointer, releasing the confinement.
    Unconfine,
    /// `confine_pointer` with an explicit region (surface-local rects),
    /// waiting for the `confined` event. An empty list confines with no
    /// region, like [`Step::Confine`].
    ConfineRegion { rects: Vec<(i32, i32, i32, i32)> },
    /// `confine_pointer` with an explicit region like
    /// [`Step::ConfineRegion`], but without waiting: for arming a
    /// confinement with no focus, where no `confined` event can arrive yet.
    ArmRegion { rects: Vec<(i32, i32, i32, i32)> },
    /// `ext_session_lock_manager_v1.lock`, waiting for the `locked` event.
    /// Only the locker script answers this (see `run_locker`).
    TakeSessionLock,
    /// `unlock_and_destroy` on the session lock, waiting for `finished`.
    /// Only the locker script answers this.
    ReleaseSessionLock,
    /// `get_lock_surface` on the locker's lock for the one output, ack its
    /// configure and attach a fullscreen buffer. Only the locker answers
    /// this: it is what gives the locked session a surface pointer focus
    /// can land on.
    MapLockSurface,
    /// Hand back what the locker's pointer has seen since the last report.
    /// Only the locker answers this.
    LockerReport,
}

/// What a client answers a [`Step`] with.
enum Ack {
    /// Sent once at startup, before mapping: which of the two manager
    /// globals the client's registry round trip found.
    Started { relative: bool, constraints: bool },
    /// The window is mapped and the client is parked holding it, carrying
    /// the toplevel's protocol id so the compositor side can aim the
    /// pointer at it.
    Mapped { surface: u32 },
    /// The drained observations (see [`Step::Report`]).
    Report {
        enters: u32,
        motions: Vec<(f64, f64)>,
        relatives: Vec<(f64, f64, f64, f64)>,
        locked: bool,
        unlocked: bool,
        confined: bool,
        unconfined: bool,
        buttons: u32,
        axis: u32,
    },
    /// The `locked` event arrived.
    Locked,
    /// The `confined` event arrived.
    Confined,
    /// The session `locked` event arrived (locker client only).
    SessionLocked,
    /// The session unlock was requested and flushed (locker client only).
    /// Unlocking itself is event-silent -- `finished` arrives only on a
    /// refused lock -- so the test proves the unlock by locking again
    /// afterwards: a still-locked session would refuse with `finished`.
    SessionReleased,
    /// What the locker's pointer has seen since the last report (locker
    /// client only).
    LockerReport {
        enters: u32,
        motions: Vec<(f64, f64)>,
        buttons: u32,
        axis: u32,
    },
    /// Anything else a step answers when there is nothing to count.
    Done,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A live compositor with one output and one connected client whose
    /// window is mapped, ready for the caller to point at.
    fn start() -> (Self, Run) {
        // Headless, not bare: the backend's output is what the core lays
        // out in and what the relative-motion clamp reads, and a harness
        // with no output clamps every `pointer_move_relative` to the
        // origin. Nothing here reads the framebuffer back.
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        let Ack::Started {
            relative,
            constraints,
        } = fixture.wait_for_ack(0)
        else {
            panic!("the client reported being mapped before it reported its globals");
        };
        let Ack::Mapped { surface } = fixture.wait_for_ack(0) else {
            panic!("the client reported observations before it reported being mapped");
        };
        (
            fixture,
            Run {
                relative,
                constraints,
                surface,
            },
        )
    }

    /// The drained observations of client `index`.
    fn report(&mut self, index: usize) -> Report {
        let Ack::Report {
            enters,
            motions,
            relatives,
            locked,
            unlocked,
            confined,
            unconfined,
            buttons,
            axis,
        } = self.run_on(index, Step::Report)
        else {
            panic!("client {index} answered a report with something else");
        };
        Report {
            enters,
            motions,
            relatives,
            locked,
            unlocked,
            confined,
            unconfined,
            buttons,
            axis,
        }
    }

    /// What the locker client's pointer has seen since its last report.
    fn locker_report(&mut self, index: usize) -> LockerReport {
        let Ack::LockerReport {
            enters,
            motions,
            buttons,
            axis,
        } = self.run_on(index, Step::LockerReport)
        else {
            panic!("client {index} answered a locker report with something else");
        };
        LockerReport {
            enters,
            motions,
            buttons,
            axis,
        }
    }

    /// Moves the pointer over client `index`'s window and drains the
    /// resulting `enter`, so later reports start clean. Hands the drained
    /// arrival report back: the focus move itself is a teleport onto the
    /// surface from wherever the pointer was, which reports no relative
    /// motion (nobody had focus when it began -- see `super`), and tests
    /// assert that rather than assuming it.
    fn focus(&mut self, index: usize, surface: u32) -> Report {
        let (x, y) = window_point(self, index, surface);
        self.state.pointer_move(x, y);
        let _ = self.state.display_handle.flush_clients();
        let report = self.report(index);
        assert!(
            report.enters >= 1,
            "the pointer never entered client {index}'s window at ({x}, {y}); \
             the layout moved and this test is aiming at bare desktop"
        );
        report
    }

    /// The pointer's current absolute position, for asserting a lock held it
    /// (or an unlock freed it).
    fn pointer_at(&self) -> (f64, f64) {
        let location = self
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .current_location();
        (location.x, location.y)
    }
}

/// What [`Fixture::start`] hands back: the two globals the client found,
/// plus its window's protocol id.
struct Run {
    relative: bool,
    constraints: bool,
    surface: u32,
}

/// A drained [`Ack::Report`], named so assertions read as field accesses.
#[derive(Debug, Default)]
struct Report {
    enters: u32,
    motions: Vec<(f64, f64)>,
    /// `(dx, dy, dx_unaccel, dy_unaccel)` per `relative_motion`.
    relatives: Vec<(f64, f64, f64, f64)>,
    locked: bool,
    unlocked: bool,
    confined: bool,
    unconfined: bool,
    buttons: u32,
    axis: u32,
}

/// A drained [`Ack::LockerReport`]: what the locker's pointer saw.
#[derive(Debug)]
struct LockerReport {
    enters: u32,
    motions: Vec<(f64, f64)>,
    buttons: u32,
    axis: u32,
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

/// The surface origin behind a compositor point: what region rects are
/// measured in. Resolved through the compositor's own hit test rather than
/// re-derived from the layout, so a decoration offset can never silently
/// skew a rect.
fn surface_origin(fixture: &Fixture, x: f64, y: f64) -> (f64, f64) {
    let (_, origin) = fixture
        .state
        .surface_under(Point::<f64, Logical>::from((x, y)))
        .expect("the aim point is over no surface");
    (origin.x, origin.y)
}

/// Asserts exact integrality: every rect and vector in the region tests is
/// small-integer arithmetic, so a fractional layout would make `assert_eq`
/// on floats meaningless rather than approximately right. Fails loudly
/// instead of flaking.
fn whole(value: f64, what: &str) -> i32 {
    assert_eq!(
        value.fract(),
        0.0,
        "{what} is fractional; these tests need integer geometry"
    );
    value as i32
}

/// The client end of one test connection: enough of a toolkit to map a
/// painted toplevel, hold a `wl_pointer` and a relative-pointer object on
/// it, and take locks and confinements.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    surface: Option<wl_surface::WlSurface>,
    constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    locked: Option<zwp_locked_pointer_v1::ZwpLockedPointerV1>,
    confined: Option<zwp_confined_pointer_v1::ZwpConfinedPointerV1>,
    /// The `wl_region` a region confinement was built from, held alive for
    /// the step's duration: the compositor copies the attributes at
    /// request time, but holding it removes any lifetime question.
    region: Option<wl_region::WlRegion>,
    /// The relative-pointer manager, bound at registry time; the relative
    /// object itself is created once the pointer exists (see `run_client`).
    /// `None` is what the advertisement test fails on.
    pending_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
    enters: u32,
    motions: Vec<(f64, f64)>,
    relatives: Vec<(f64, f64, f64, f64)>,
    locked_seen: bool,
    unlocked_seen: bool,
    confined_seen: bool,
    unconfined_seen: bool,
    buttons: u32,
    axis: u32,
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
        } else if interface
            == zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1::interface().name
        {
            let manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1 =
                registry.bind(name, version.min(1), qh, ());
            // Bound now, used once the pointer exists below: the object is
            // what the globals test observes, and every other test needs it
            // before its first step.
            client.pending_manager = Some(manager);
        } else if interface == zwp_pointer_constraints_v1::ZwpPointerConstraintsV1::interface().name
        {
            client.constraints = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl TestClient {
    /// Whether the registry round trip found each manager global.
    fn globals(&self) -> (bool, bool) {
        (self.pending_manager.is_some(), self.constraints.is_some())
    }

    /// The drained observations, clearing the buffers.
    fn drain(&mut self) -> Ack {
        let report = Ack::Report {
            enters: self.enters,
            motions: std::mem::take(&mut self.motions),
            relatives: std::mem::take(&mut self.relatives),
            locked: std::mem::take(&mut self.locked_seen),
            unlocked: std::mem::take(&mut self.unlocked_seen),
            confined: std::mem::take(&mut self.confined_seen),
            unconfined: std::mem::take(&mut self.unconfined_seen),
            buttons: std::mem::take(&mut self.buttons),
            axis: std::mem::take(&mut self.axis),
        };
        self.enters = 0;
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
            wl_pointer::Event::Enter { .. } => client.enters += 1,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => client.motions.push((surface_x, surface_y)),
            wl_pointer::Event::Button { .. } => client.buttons += 1,
            wl_pointer::Event::Axis { .. } => client.axis += 1,
            _ => {}
        }
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion {
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
            ..
        } = event
        {
            client.relatives.push((dx, dy, dx_unaccel, dy_unaccel));
        }
    }
}

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => client.locked_seen = true,
            zwp_locked_pointer_v1::Event::Unlocked => client.unlocked_seen = true,
            _ => {}
        }
    }
}

impl Dispatch<zwp_confined_pointer_v1::ZwpConfinedPointerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1,
        event: zwp_confined_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_confined_pointer_v1::Event::Confined => client.confined_seen = true,
            zwp_confined_pointer_v1::Event::Unconfined => client.unconfined_seen = true,
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
wayland_client::delegate_noop!(TestClient: ignore wl_region::WlRegion);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);

/// `confine_pointer` with an explicit region built from surface-local
/// rects, for [`Step::ConfineRegion`] and [`Step::ArmRegion`]. An empty
/// list confines with no region. The region object is held on the client
/// for the step's duration (see the field doc).
fn confine_with_region(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    rects: Vec<(i32, i32, i32, i32)>,
) -> Result<(), String> {
    let constraints = client.constraints.clone().ok_or("no constraints global")?;
    let surface = client.surface.clone().ok_or("no surface to confine")?;
    let pointer = client.pointer.clone().ok_or("no pointer to confine")?;
    let region = if rects.is_empty() {
        None
    } else {
        let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
        let region = compositor.create_region(qh, ());
        for (x, y, width, height) in rects {
            region.add(x, y, width, height);
        }
        client.region = Some(region.clone());
        Some(region)
    };
    let confined = constraints.confine_pointer(
        &surface,
        &pointer,
        region.as_ref(),
        zwp_pointer_constraints_v1::Lifetime::Persistent,
        qh,
        (),
    );
    client.confined = Some(confined);
    Ok(())
}

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque pixels, over a real memfd --
/// the same path any toolkit takes (the shape `activation/tests.rs` uses).
///
/// Needed for one reason: a toplevel that never attaches a buffer has an
/// empty bounding box, so `Space::element_under` can never find it and the
/// pointer can never be over anything.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("flexwm-relative-test", rustix::fs::MemfdFlags::CLOEXEC)
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

/// Dispatches until `flag` fires or the compositor stops answering.
fn wait_for_flag(
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    what: &str,
    flag: impl Fn(&TestClient) -> bool,
) -> Result<(), String> {
    wait_for(queue, client, what, |seen| flag(seen).then_some(()))
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
    client.pointer = Some(pointer.clone());
    // The relative-pointer object every test below reads its deltas off.
    // Created up front (not per step): the protocol allows one per pointer,
    // and re-creating it per step would be a second object for the same
    // pointer rather than a fresh start.
    if let Some(manager) = client.pending_manager.clone() {
        manager.get_relative_pointer(&pointer, &qh, ());
    }

    // Mapped in two commits, the way the protocol asks: the role-only commit
    // first, then pixels once the compositor's configure has been acked (the
    // `xdg_surface` handler above does that as the event arrives).
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title("relative".to_string());
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
    // compositor side waits for before pointing at the window.
    let (relative, constraints) = client.globals();
    acks.send(Ack::Started {
        relative,
        constraints,
    })
    .map_err(|e| e.to_string())?;
    acks.send(Ack::Mapped {
        surface: surface_id,
    })
    .map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        // Drain anything the compositor sent since the last step -- in
        // particular the `enter`/`motion`/`relative_motion` the test just
        // caused -- before acting, so a report answers what happened rather
        // than what is still in flight.
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::Report => {
                acks.send(client.drain()).map_err(|e| e.to_string())?;
                continue;
            }
            Step::Lock => {
                let constraints = client.constraints.clone().ok_or("no constraints global")?;
                let surface = client.surface.clone().ok_or("no surface to lock")?;
                let pointer = client.pointer.clone().ok_or("no pointer to lock")?;
                let locked = constraints.lock_pointer(
                    &surface,
                    &pointer,
                    None,
                    zwp_pointer_constraints_v1::Lifetime::Persistent,
                    &qh,
                    (),
                );
                client.locked = Some(locked);
                wait_for_flag(&mut queue, &mut client, "the locked event", |seen| {
                    seen.locked_seen
                })?;
                acks.send(Ack::Locked).map_err(|e| e.to_string())?;
                continue;
            }
            Step::Unlock => {
                let locked = client.locked.take().ok_or("no lock to release")?;
                locked.destroy();
            }
            Step::Arm => {
                let constraints = client.constraints.clone().ok_or("no constraints global")?;
                let surface = client.surface.clone().ok_or("no surface to lock")?;
                let pointer = client.pointer.clone().ok_or("no pointer to lock")?;
                let locked = constraints.lock_pointer(
                    &surface,
                    &pointer,
                    None,
                    zwp_pointer_constraints_v1::Lifetime::Persistent,
                    &qh,
                    (),
                );
                client.locked = Some(locked);
            }
            Step::Confine => {
                let constraints = client.constraints.clone().ok_or("no constraints global")?;
                let surface = client.surface.clone().ok_or("no surface to confine")?;
                let pointer = client.pointer.clone().ok_or("no pointer to confine")?;
                let confined = constraints.confine_pointer(
                    &surface,
                    &pointer,
                    None,
                    zwp_pointer_constraints_v1::Lifetime::Persistent,
                    &qh,
                    (),
                );
                client.confined = Some(confined);
                wait_for_flag(&mut queue, &mut client, "the confined event", |seen| {
                    seen.confined_seen
                })?;
                acks.send(Ack::Confined).map_err(|e| e.to_string())?;
                continue;
            }
            Step::Unconfine => {
                let confined = client.confined.take().ok_or("no confinement to release")?;
                confined.destroy();
            }
            Step::ConfineRegion { rects } => {
                confine_with_region(&mut client, &qh, rects)?;
                wait_for_flag(&mut queue, &mut client, "the confined event", |seen| {
                    seen.confined_seen
                })?;
                acks.send(Ack::Confined).map_err(|e| e.to_string())?;
                continue;
            }
            Step::ArmRegion { rects } => {
                confine_with_region(&mut client, &qh, rects)?;
            }
            Step::TakeSessionLock
            | Step::ReleaseSessionLock
            | Step::MapLockSurface
            | Step::LockerReport => {
                return Err("the game client cannot take a session lock".to_string());
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The locker end of a second test connection: just enough of a screen
/// locker to take and release the session lock around a pointer-lock test,
/// map the fullscreen lock surface pointer focus must land on, and report
/// what its own pointer was told -- so the tests can assert the lock
/// surface really received the `enter`, the motion and the click, rather
/// than inferring it from the game client's silence.
#[derive(Default)]
struct LockerClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    pointer: Option<wl_pointer::WlPointer>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    locked_seen: bool,
    finished_seen: bool,
    /// The size and serial the compositor configured the lock surface to.
    lock_configure: Option<(u32, u32, u32)>,
    enters: u32,
    motions: Vec<(f64, f64)>,
    buttons: u32,
    axis: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for LockerClient {
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
        if interface == ext_session_lock_manager_v1::ExtSessionLockManagerV1::interface().name {
            client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name {
            client.seat = Some(registry.bind(name, version.min(5), qh, ()));
        } else if interface == wl_output::WlOutput::interface().name {
            client.output = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => client.locked_seen = true,
            ext_session_lock_v1::Event::Finished => client.finished_seen = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(LockerClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(LockerClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(LockerClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(LockerClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(LockerClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(LockerClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(LockerClient: ignore wl_seat::WlSeat);

impl Dispatch<wl_pointer::WlPointer, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { .. } => client.enters += 1,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => client.motions.push((surface_x, surface_y)),
            wl_pointer::Event::Button { .. } => client.buttons += 1,
            wl_pointer::Event::Axis { .. } => client.axis += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            client.lock_configure = Some((serial, width, height));
        }
    }
}

/// A fullscreen `wl_buffer` of opaque pixels for the lock surface, over a
/// real memfd. The configure's size is an exact requirement, so this is
/// built at exactly that size by the caller.
fn locker_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<LockerClient>,
    width: i32,
    height: i32,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("flexwm-locker-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

fn run_locker(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LockerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    client
        .lock_manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    client.pointer = Some(seat.get_pointer(&qh, ()));

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::TakeSessionLock => {
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                // Fresh flags, not sticky ones: this connection takes (and
                // releases, and re-takes) the session lock several times per
                // test, and waiting on a `locked` left over from the
                // previous lock would ack a lock that was never confirmed --
                // whose `unlock_and_destroy` the compositor then rightly
                // refuses with `InvalidUnlock`, killing this client.
                client.locked_seen = false;
                client.finished_seen = false;
                let lock = manager.lock(&qh, ());
                client.lock = Some(lock);
                // Exactly one of the two must arrive, and the protocol says
                // so in as many words: "In response to the creation of this
                // object the compositor must send either the locked or
                // finished event." A `finished` here is a refusal, which
                // fails the test rather than hanging it.
                wait_for(&mut queue, &mut client, "locked or finished", |seen| {
                    (seen.locked_seen || seen.finished_seen).then_some(())
                })?;
                if !client.locked_seen {
                    return Err("the session lock was refused".to_string());
                }
                acks.send(Ack::SessionLocked).map_err(|e| e.to_string())?;
                continue;
            }
            Step::ReleaseSessionLock => {
                let lock = client.lock.take().ok_or("no session lock to release")?;
                lock.unlock_and_destroy();
                // Flush the request before acknowledging: wayland-client
                // buffers it until a roundtrip, and the ack below is sent
                // over a channel, not the wire -- without this the
                // compositor would only see the unlock when some later
                // step happens to roundtrip (the shape the old round-trip
                // test relied on implicitly via its re-lock proof).
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                // Event-silent by protocol (see `Ack::SessionReleased`):
                // the ack below means requested-and-flushed, and the test
                // proves the unlock by locking again.
                acks.send(Ack::SessionReleased).map_err(|e| e.to_string())?;
                continue;
            }
            Step::MapLockSurface => {
                let lock = client.lock.clone().ok_or("no session lock to surface")?;
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                let output = client.output.clone().ok_or("no wl_output")?;
                let surface = compositor.create_surface(&qh, ());
                let lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
                let (serial, width, height) =
                    wait_for(&mut queue, &mut client, "a lock configure", |seen| {
                        seen.lock_configure
                    })?;
                lock_surface.ack_configure(serial);
                let buffer = locker_buffer(&shm, &qh, width as i32, height as i32)?;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
                // Flush the commit before acknowledging, for the same
                // reason as `ReleaseSessionLock` below: without this the
                // map (and its pointer `enter`) only lands whenever some
                // later step roundtrips.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
                continue;
            }
            Step::LockerReport => {
                let report = Ack::LockerReport {
                    enters: client.enters,
                    motions: std::mem::take(&mut client.motions),
                    buttons: std::mem::take(&mut client.buttons),
                    axis: std::mem::take(&mut client.axis),
                };
                client.enters = 0;
                acks.send(report).map_err(|e| e.to_string())?;
                continue;
            }
            _ => return Err("the locker cannot map windows or lock pointers".to_string()),
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// The tests
// -------------------------------------------------------------------------

/// Moves the pointer by `(dx, dy)` from client `index`'s window point and
/// hands back what that client observed, so each motion test pins its own
/// numbers without re-deriving the aim.
fn move_by(fixture: &mut Fixture, index: usize, surface: u32, dx: f64, dy: f64) -> Report {
    let (x, y) = window_point(fixture, index, surface);
    fixture.state.pointer_move(x + dx, y + dy);
    let _ = fixture.state.display_handle.flush_clients();
    fixture.report(index)
}

#[test]
fn both_manager_globals_are_advertised() {
    let (_fixture, run) = Fixture::start();
    assert!(
        run.relative,
        "no zwp_relative_pointer_manager_v1 -- the global is missing"
    );
    assert!(
        run.constraints,
        "no zwp_pointer_constraints_v1 -- the global is missing"
    );
}

#[test]
fn a_teleport_onto_a_surface_reports_no_relative_motion() {
    // Nobody had pointer focus when the motion began, so there is no client
    // to credit: the `enter` arrives, but no `relative_motion` does. This is
    // the pre-move-focus rule (`super`), not a dropped event.
    let (mut fixture, run) = Fixture::start();
    let arrived = fixture.focus(0, run.surface);
    assert!(
        arrived.relatives.is_empty(),
        "a teleport onto a fresh surface reported motion to no focus: {:?}",
        arrived.relatives
    );
}

#[test]
fn a_focused_client_receives_exact_deltas_with_no_lock_involved() {
    // The gating correction, pinned: no constraint is ever created here, and
    // relative motion still flows. Delivery is about pointer focus, not lock
    // state (see `super`) -- a test that locked first would prove nothing
    // about the distinction.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let report = move_by(&mut fixture, 0, run.surface, 20.0, 20.0);
    assert_eq!(
        report.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "wrong relative vector for a (20, 20) absolute move"
    );
    assert!(
        !report.motions.is_empty(),
        "the absolute motion the relative one pairs with never arrived"
    );
}

#[test]
fn a_client_without_pointer_focus_receives_no_relative_motion() {
    // The other half of the gating: the second client holds a live relative
    // object for its own pointer, but focus is on the first client's window,
    // so every delta routes to the first client and none to the second.
    let (mut fixture, run_a) = Fixture::start();
    fixture.spawn(run_client);
    let Ack::Started { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported being mapped before its globals");
    };
    let Ack::Mapped { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported observations before being mapped");
    };
    fixture.focus(0, run_a.surface);
    let report_a = move_by(&mut fixture, 0, run_a.surface, 20.0, 20.0);
    assert_eq!(
        report_a.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "the focused client lost its own stream"
    );
    let report_b = fixture.report(1);
    assert!(
        report_b.relatives.is_empty(),
        "an unfocused client received relative motion: {:?}",
        report_b.relatives
    );
    assert_eq!(
        report_b.enters, 0,
        "the pointer entered a window it never moved over"
    );
}

#[test]
fn a_locked_pointer_reports_relative_motion_but_moves_nothing_absolute() {
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    // Two moves, two vectors: while locked the absolute position is held,
    // so the second delta is measured from the same held point, not from
    // where the first move asked to go -- (100, 100), not (30, 30).
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.state.pointer_move(x + 70.0, y + 70.0);
    fixture.state.pointer_move(x + 100.0, y + 100.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert_eq!(
        report.relatives,
        vec![(70.0, 70.0, 70.0, 70.0), (100.0, 100.0, 100.0, 100.0)],
        "a locked pointer lost or mangled its relative stream"
    );
    assert!(
        report.locked,
        "the lock was not active while those moves ran"
    );
    assert!(
        report.motions.is_empty(),
        "absolute motion leaked through an active lock: {:?}",
        report.motions
    );
    assert_eq!(
        fixture.pointer_at(),
        (x, y),
        "an active lock moved the absolute position"
    );
}

#[test]
fn a_lock_taken_before_focus_engages_on_arrival() {
    // A game arming its mouse mode at startup, ahead of any pointer motion:
    // creation-time activation has no focus to act on, so the lock sits
    // inactive until focus arrives, and arrival engages it (see
    // `engage_pending_constraint`). Without that second half this test
    // would wait for a `locked` event that never comes.
    let (mut fixture, run) = Fixture::start();
    let before = fixture.report(0);
    assert_eq!(
        before.enters, 0,
        "the pointer is already over the window; this test proves nothing about unfocused locking"
    );
    let Ack::Done = fixture.run_on(0, Step::Arm) else {
        panic!("arming the lock answered with something else");
    };
    let armed = fixture.report(0);
    assert!(
        !armed.locked,
        "an unfocused lock activated at creation time"
    );
    let arrival = fixture.focus(0, run.surface);
    assert!(
        arrival.locked,
        "arriving focus did not engage the waiting lock"
    );
    // And it holds from there: the absolute position is frozen while the
    // relative stream flows.
    let report = move_by(&mut fixture, 0, run.surface, 20.0, 20.0);
    assert_eq!(
        report.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "the engaged lock lost its relative stream"
    );
    assert!(
        report.motions.is_empty(),
        "absolute motion leaked through the engaged lock: {:?}",
        report.motions
    );
    let (x, y) = window_point(&fixture, 0, run.surface);
    assert_eq!(
        fixture.pointer_at(),
        (x, y),
        "the engaged lock did not hold the absolute position"
    );
}

#[test]
fn releasing_the_lock_resumes_absolute_motion_and_keeps_relative() {
    // Unlocking restores the ordinary focused behavior, not silence: the
    // absolute position moves again *and* the relative stream keeps flowing.
    // (Releasing a lock stops holding; it does not unfocus.)
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    let Ack::Done = fixture.run_on(0, Step::Unlock) else {
        panic!("unlocking answered with something else");
    };
    let report = move_by(&mut fixture, 0, run.surface, 5.0, 5.0);
    assert_eq!(
        report.relatives,
        vec![(5.0, 5.0, 5.0, 5.0)],
        "relative motion stopped after the lock was released"
    );
    // Destroying a `Persistent` lock is silent: Smithay removes the
    // constraint without an `unlocked` event (those come only from an
    // explicit deactivate, e.g. pointer-leave). Pinned as absence, not
    // assumed.
    assert!(
        !report.unlocked,
        "destroying a persistent lock sent an `unlocked` event"
    );
    assert!(
        !report.motions.is_empty(),
        "absolute motion did not resume after the lock was released"
    );
    let (x, y) = window_point(&fixture, 0, run.surface);
    assert_eq!(
        fixture.pointer_at(),
        (x + 5.0, y + 5.0),
        "the absolute position is not where the freed move put it"
    );
}

#[test]
fn a_confined_pointer_is_held_past_its_surface_and_clamped_inside_it() {
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let Ack::Confined = fixture.run_on(0, Step::Confine) else {
        panic!("confining answered with something other than `confined`");
    };
    let (x, y) = window_point(&fixture, 0, run.surface);
    // Off the surface entirely (a single window, so bare desktop): held,
    // with the full vector still reported.
    fixture.state.pointer_move(x + 500.0, y);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert_eq!(
        report.relatives,
        vec![(500.0, 0.0, 500.0, 0.0)],
        "confinement swallowed the relative vector it should only have held absolute"
    );
    assert!(
        report.confined,
        "the confinement was not active while those moves ran"
    );
    assert!(
        report.motions.is_empty(),
        "absolute motion escaped a confinement: {:?}",
        report.motions
    );
    assert_eq!(
        fixture.pointer_at(),
        (x, y),
        "a confinement let the pointer leave its surface"
    );
    // Back inside: the move lands, both halves flowing.
    let report = move_by(&mut fixture, 0, run.surface, 5.0, 5.0);
    assert_eq!(
        report.relatives,
        vec![(5.0, 5.0, 5.0, 5.0)],
        "relative motion stopped inside a confinement"
    );
    assert!(
        !report.motions.is_empty(),
        "a move inside the confinement never landed"
    );
    assert_eq!(
        fixture.pointer_at(),
        (x + 5.0, y + 5.0),
        "a move inside the confinement landed elsewhere"
    );
    // Released: free again (and, like the lock's destroy, silent -- no
    // `unconfined` event).
    let Ack::Done = fixture.run_on(0, Step::Unconfine) else {
        panic!("unconfining answered with something else");
    };
    fixture.state.pointer_move(x + 500.0, y);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert!(
        !report.unconfined,
        "destroying a persistent confinement sent an `unconfined` event"
    );
    assert_eq!(
        fixture.pointer_at(),
        (x + 500.0, y),
        "absolute motion did not resume after the confinement was released"
    );
}

#[test]
fn device_deltas_keep_their_accelerated_and_unaccelerated_pairs() {
    // The libinput path carries two pairs -- the accelerated delta the
    // absolute position moves by, and the pre-accel device delta -- and the
    // relative event must report each as its own (see `super`). The pairs
    // here are deliberately far apart (10 vs 3) so a compositor reporting
    // one for both fails loudly rather than within float noise.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.state.pointer_move_relative(10.0, 0.0, 3.0, 0.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert_eq!(
        report.relatives,
        vec![(10.0, 0.0, 3.0, 0.0)],
        "the relative event did not carry the device pairs through"
    );
    assert_eq!(
        fixture.pointer_at(),
        (x + 10.0, y),
        "the absolute position did not move by the accelerated delta"
    );
}

#[test]
fn relative_deltas_are_unclipped_where_absolute_motion_clamps() {
    // The spec's own example: motion clipped by the output edge still
    // reports the unclipped vector. Driven far past the 1600px edge from a
    // focused window, so the absolute position stops at 1599 while the
    // relative event carries the whole 5000.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (_, y) = window_point(&fixture, 0, run.surface);
    fixture
        .state
        .pointer_move_relative(5000.0, 0.0, 5000.0, 0.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert_eq!(
        report.relatives,
        vec![(5000.0, 0.0, 5000.0, 0.0)],
        "the edge clipped the relative vector it must not touch"
    );
    assert_eq!(
        fixture.pointer_at(),
        (1599.0, y),
        "the absolute position did not clamp at the output edge"
    );
}

#[test]
fn disconnecting_a_client_with_a_live_lock_and_relative_pointer_is_clean() {
    // The client goes away holding both objects; Smithay tears both down
    // from its own destruction hooks. What this pins is the compositor's
    // half: motion afterwards serves a new client correctly, with no panic
    // and no stale stream -- the old objects neither deliver nor block.
    let (mut fixture, run_a) = Fixture::start();
    fixture.focus(0, run_a.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    fixture.disconnect(0);
    fixture.state.pointer_move(100.0, 100.0);
    fixture.settle();
    fixture.spawn(run_client);
    let Ack::Started { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported being mapped before its globals");
    };
    let Ack::Mapped { surface: surface_b } = fixture.wait_for_ack(1) else {
        panic!("the second client reported observations before being mapped");
    };
    fixture.focus(1, surface_b);
    let report = move_by(&mut fixture, 1, surface_b, 20.0, 20.0);
    assert_eq!(
        report.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "a fresh client after a dirty disconnect lost its relative stream"
    );
}

#[test]
fn two_focused_clients_each_get_their_own_stream() {
    // Smithay routes by the focused surface's client (`same_client_as`):
    // each client sees exactly the motion delivered while focus is on its
    // own window, and nothing from the other's turn.
    let (mut fixture, run_a) = Fixture::start();
    fixture.spawn(run_client);
    let Ack::Started { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported being mapped before its globals");
    };
    let Ack::Mapped { surface: surface_b } = fixture.wait_for_ack(1) else {
        panic!("the second client reported observations before being mapped");
    };
    fixture.focus(0, run_a.surface);
    // Window points are layout, so they do not move under the pointer:
    // read both once and derive every vector from them.
    let (ax, ay) = window_point(&fixture, 0, run_a.surface);
    let (bx, by) = window_point(&fixture, 1, surface_b);
    fixture.state.pointer_move(ax + 20.0, ay + 20.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report_a = fixture.report(0);
    assert_eq!(
        report_a.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "the first client lost its own stream"
    );
    let quiet_b = fixture.report(1);
    assert!(
        quiet_b.relatives.is_empty(),
        "the second client heard the first client's motion: {:?}",
        quiet_b.relatives
    );
    fixture.focus(1, surface_b);
    // The focus switch itself is a teleport that began with focus on the
    // first client -- at (ax + 20, ay + 20), where the last move left the
    // pointer -- so it is credited there (pre-move focus -- see `super`),
    // with exactly the switch vector. Drained here, so the stream the next
    // assertion reads starts clean.
    let switch = fixture.report(0);
    assert_eq!(
        switch.relatives,
        vec![(
            bx - ax - 20.0,
            by - ay - 20.0,
            bx - ax - 20.0,
            by - ay - 20.0
        )],
        "a focus-changing teleport was not credited to the surface it left"
    );
    fixture.state.pointer_move(bx + 20.0, by + 20.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report_b = fixture.report(1);
    assert_eq!(
        report_b.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "the second client lost its own stream"
    );
    let quiet_a = fixture.report(0);
    assert!(
        quiet_a.relatives.is_empty(),
        "the first client heard the second client's motion: {:?}",
        quiet_a.relatives
    );
}

#[test]
fn confine_region_clamps_per_axis_against_a_sub_rectangle() {
    // An L of two rects through the focus row: the full-width strip the
    // pointer sits in, plus a bar down its right side. A move deep into the
    // bar's column but past the strip keeps its x (the x-step alone lands
    // in the strip) and loses its y (the y-step alone lands in neither) --
    // the per-axis shape, not an all-or-nothing clamp.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let (ox, oy) = surface_origin(&fixture, fx, fy);
    let (ox, oy) = (whole(ox, "surface origin x"), whole(oy, "surface origin y"));
    let (flx, fly) = (whole(fx, "focus x") - ox, whole(fy, "focus y") - oy);
    let fy0 = fly - 8;
    let fx1 = (flx + 18).min(48);
    let (tx, ty) = (fx1 + 8, fy0 + 40);
    assert!(
        fy0 >= 0 && tx < 64 && ty < 64 && flx < fx1,
        "focus at ({flx}, {fly}) does not fit the L shape; the layout moved"
    );
    let Ack::Confined = fixture.run_on(
        0,
        Step::ConfineRegion {
            rects: vec![(0, fy0, 64, 16), (fx1, fy0, 16, 48)],
        },
    ) else {
        panic!("confining with a region answered with something other than `confined`");
    };
    fixture
        .state
        .pointer_move(f64::from(ox + tx), f64::from(oy + ty));
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    assert_eq!(
        report.relatives,
        vec![(
            f64::from(tx - flx),
            f64::from(ty - fly),
            f64::from(tx - flx),
            f64::from(ty - fly)
        )],
        "the relative vector was clipped with the absolute position"
    );
    assert!(
        report.confined,
        "the confinement was not active while that move ran"
    );
    assert!(!report.motions.is_empty(), "the clamped move never landed");
    // x kept, y zeroed: the absolute position is the target's column on
    // the focus row.
    assert_eq!(
        fixture.pointer_at(),
        (f64::from(ox + tx), fy),
        "the move was not clamped per axis"
    );
}

#[test]
fn a_constraint_whose_region_misses_the_pointer_does_not_apply() {
    // The region gate (anvil's): a constraint whose region does not contain
    // the pointer's current position constrains nothing, even while active.
    // Creation-time activation itself ignores the region -- the client does
    // get `confined` -- so this pins active-but-not-applying, not inactive.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let Ack::Confined = fixture.run_on(
        0,
        Step::ConfineRegion {
            rects: vec![(0, 0, 10, 10)],
        },
    ) else {
        panic!("confining with a region answered with something other than `confined`");
    };
    let report = move_by(&mut fixture, 0, run.surface, 20.0, 20.0);
    assert!(
        report.confined,
        "the confinement was not active while that move ran"
    );
    assert_eq!(
        report.relatives,
        vec![(20.0, 20.0, 20.0, 20.0)],
        "an inapplicable confinement disturbed the relative stream"
    );
    assert!(
        !report.motions.is_empty(),
        "an inapplicable confinement held absolute motion"
    );
    let (x, y) = window_point(&fixture, 0, run.surface);
    assert_eq!(
        fixture.pointer_at(),
        (x + 20.0, y + 20.0),
        "an inapplicable confinement moved the pointer somewhere else"
    );
    // Leaving while gated out deactivates (the constraint is active, so
    // the leave tears it down to inactive-but-kept): the client sees
    // `unconfined`, and the entry persists for a later re-entry.
    fixture.state.pointer_move(1500.0, 900.0);
    let _ = fixture.state.display_handle.flush_clients();
    let left = fixture.report(0);
    assert!(
        left.unconfined,
        "leaving with an active-but-gated confinement sent no `unconfined`"
    );
    // Re-entering inside the region re-arms through `engage_pending_constraint`:
    // no new confine request in between.
    let (ox, oy) = surface_origin(&fixture, x, y);
    let (ox, oy) = (whole(ox, "surface origin x"), whole(oy, "surface origin y"));
    fixture
        .state
        .pointer_move(f64::from(ox + 5), f64::from(oy + 5));
    let _ = fixture.state.display_handle.flush_clients();
    let reentry = fixture.report(0);
    assert!(
        reentry.enters >= 1,
        "the pointer never re-entered the window"
    );
    assert!(
        reentry.confined,
        "re-entering inside the region did not re-arm the confinement"
    );
    // And the re-armed confinement holds: the full vector reports while
    // the absolute position stays put.
    fixture
        .state
        .pointer_move(f64::from(ox + 505), f64::from(oy + 5));
    let _ = fixture.state.display_handle.flush_clients();
    let held = fixture.report(0);
    assert_eq!(
        held.relatives,
        vec![(500.0, 0.0, 500.0, 0.0)],
        "the re-armed confinement lost its relative stream"
    );
    assert_eq!(
        fixture.pointer_at(),
        (f64::from(ox + 5), f64::from(oy + 5)),
        "the re-armed confinement let the pointer escape"
    );
}

#[test]
fn engage_respects_the_region_on_arrival_and_reentry() {
    // The engage gate in both directions: arriving outside the armed
    // region stays disarmed, and leaving and re-entering inside it arms.
    let (mut fixture, run) = Fixture::start();
    let before = fixture.report(0);
    assert_eq!(
        before.enters, 0,
        "the pointer is already over the window; this test proves nothing about unfocused arming"
    );
    let Ack::Done = fixture.run_on(
        0,
        Step::ArmRegion {
            rects: vec![(50, 50, 10, 10)],
        },
    ) else {
        panic!("arming a regional confinement answered with something else");
    };
    let armed = fixture.report(0);
    assert!(
        !armed.confined,
        "an unfocused confinement activated at creation time"
    );
    // Arrival outside the region: still disarmed.
    let arrival = fixture.focus(0, run.surface);
    assert!(
        !arrival.confined,
        "arrival outside the region engaged the confinement"
    );
    // A move within the surface to the region's doorstep: no enter, no
    // engage, and the move itself is free (the gate sees the pointer
    // outside the region).
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let (ox, oy) = surface_origin(&fixture, fx, fy);
    let (ox, oy) = (whole(ox, "surface origin x"), whole(oy, "surface origin y"));
    fixture
        .state
        .pointer_move(f64::from(ox + 55), f64::from(oy + 55));
    let _ = fixture.state.display_handle.flush_clients();
    let inside = fixture.report(0);
    assert!(
        !inside.confined,
        "a same-surface move engaged the confinement"
    );
    assert_eq!(
        fixture.pointer_at(),
        (f64::from(ox + 55), f64::from(oy + 55)),
        "a move outside an inapplicable region did not land"
    );
    // Leave and re-enter inside the region: engages.
    fixture.state.pointer_move(1500.0, 900.0);
    let _ = fixture.state.display_handle.flush_clients();
    assert!(
        fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .current_focus()
            .is_none(),
        "the pointer never left the surface; the re-entry proves nothing"
    );
    fixture.report(0);
    fixture
        .state
        .pointer_move(f64::from(ox + 55), f64::from(oy + 55));
    let _ = fixture.state.display_handle.flush_clients();
    let reentry = fixture.report(0);
    assert!(
        reentry.enters >= 1,
        "the pointer never re-entered the window"
    );
    assert!(
        reentry.confined,
        "re-entering inside the region did not engage the confinement"
    );
    // And it holds from there: the full vector still reports while the
    // absolute position stays put.
    fixture
        .state
        .pointer_move(f64::from(ox + 555), f64::from(oy + 55));
    let _ = fixture.state.display_handle.flush_clients();
    let held = fixture.report(0);
    assert_eq!(
        held.relatives,
        vec![(500.0, 0.0, 500.0, 0.0)],
        "the engaged regional confinement lost its relative stream"
    );
    assert_eq!(
        fixture.pointer_at(),
        (f64::from(ox + 55), f64::from(oy + 55)),
        "the engaged regional confinement let the pointer escape"
    );
}

#[test]
fn a_session_lock_deactivates_a_held_pointer_lock() {
    // BEHAVIOR CORRECTION. This replaces
    // `a_persistent_lock_survives_a_session_lock_round_trip`, which pinned
    // the buggy behavior as intended: no `unlocked` to the game, no `enter`
    // to the lock surface, the stream continuing across the lock. That
    // assertion was wrong, and its stated basis was false: the module doc
    // justified the freeze with "closing the offending window frees the
    // pointer with one chord", but while the session is locked every
    // keybinding except `ChangeVt` is forwarded to the locker
    // (`input.rs`), so the chord never executes and a VT switch was the
    // only recovery -- while deltas, buttons and axis kept streaming to
    // the game underneath, the password-leak class `session_lock.rs`
    // rejects for the keyboard. The fixed behavior: locking deactivates
    // the held constraint (the game sees `unlocked`), the lock
    // transition's focus refresh lands on the lock surface, and nothing
    // pointer-shaped reaches the game until unlock re-derives focus onto
    // it and the still-registered persistent lock re-arms through the
    // ordinary arrival path (the game sees `locked` with no new request --
    // the protocol's own leave/re-enter story, the same one the
    // regional-confinement test pins).
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    let locker = fixture.spawn(run_locker);
    fixture.send_step(locker, Step::TakeSessionLock);
    // The fresh lock confirms on its first blanked frame, which nothing
    // has drawn yet: render one explicitly rather than hoping the frame
    // timer fires mid-dispatch.
    fixture.render();
    let Ack::SessionLocked = fixture.wait_for_ack(locker) else {
        panic!("the session lock answered with something other than `locked`");
    };
    fixture.settle();
    // The lock surface the locked session's pointer focus must land on.
    fixture.run_on(locker, Step::MapLockSurface);
    // Locking deactivated the held constraint: the game saw `unlocked`,
    // and focus left it (a `leave`, so no new `enter` here).
    let during = fixture.report(0);
    assert!(
        during.unlocked,
        "the session lock left a held pointer lock active"
    );
    assert_eq!(
        during.enters, 0,
        "pointer focus re-entered the game surface under session lock"
    );
    // ...and landed on the lock surface at map-commit time.
    let locker_during = fixture.locker_report(locker);
    assert!(
        locker_during.enters >= 1,
        "the lock surface never received pointer focus under session lock"
    );
    // A held mouse while locked: motion, click and scroll. None of it may
    // reach the game; all of it reaches the lock surface.
    locked_input(&mut fixture, fx, fy);
    let silent = fixture.report(0);
    assert!(
        silent.relatives.is_empty(),
        "device deltas streamed to the game across the lock: {:?}",
        silent.relatives
    );
    assert!(
        silent.motions.is_empty(),
        "absolute motion reached the game across the lock: {:?}",
        silent.motions
    );
    assert_eq!(
        silent.buttons, 0,
        "buttons reached the game across the lock"
    );
    assert_eq!(silent.axis, 0, "axis reached the game across the lock");
    let heard = fixture.locker_report(locker);
    assert!(
        !heard.motions.is_empty(),
        "locked motion reached nobody: not the game, not the lock surface"
    );
    assert_eq!(
        heard.buttons, 2,
        "the locked click did not reach the lock surface"
    );
    assert!(
        heard.axis >= 1,
        "locked scroll did not reach the lock surface"
    );
    // Absolute motion while locked lands on the lock surface -- held off
    // the game, not frozen in place.
    assert_eq!(
        fixture.pointer_at(),
        (fx + 10.0, fy + 10.0),
        "locked motion did not land on the lock surface"
    );
    // Unlock: focus returns to the game surface, and the persistent lock
    // -- deactivated, never destroyed -- re-arms on arrival with no new
    // request from the client.
    fixture.send_step(locker, Step::ReleaseSessionLock);
    let Ack::SessionReleased = fixture.wait_for_ack(locker) else {
        panic!("the session unlock answered with something unexpected");
    };
    fixture.settle();
    let back = fixture.report(0);
    assert!(
        back.locked,
        "unlocking did not re-arm the game's persistent lock"
    );
    assert!(
        back.enters >= 1,
        "pointer focus did not return to the game surface on unlock"
    );
    // And the stream resumes from there, absolute still held: relative
    // flows, nothing leaks through.
    let (px, py) = fixture.pointer_at();
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.state.pointer_move(x + 20.0, y + 20.0);
    let _ = fixture.state.display_handle.flush_clients();
    let resumed = fixture.report(0);
    assert_eq!(
        resumed.relatives,
        vec![(x + 20.0 - px, y + 20.0 - py, x + 20.0 - px, y + 20.0 - py)],
        "the re-armed lock lost its relative stream"
    );
    assert!(
        resumed.motions.is_empty(),
        "absolute motion leaked through the re-armed lock: {:?}",
        resumed.motions
    );
    assert_eq!(
        fixture.pointer_at(),
        (px, py),
        "the re-armed lock did not hold the absolute position"
    );
}

/// Motion, click and scroll from where the pointer is, the way a hand on
/// the mouse (or an attacker holding it) behaves while the session is
/// locked. The caller drains both clients' reports before and after, so
/// this only moves the pointer and flushes.
fn locked_input(fixture: &mut Fixture, x: f64, y: f64) {
    fixture.state.pointer_move(x + 10.0, y + 10.0);
    let _ = fixture.state.display_handle.flush_clients();
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.state.scroll(0.0, 10.0);
    let _ = fixture.state.display_handle.flush_clients();
}

/// Locks the session around a mapped lock surface: the three steps every
/// session-lock test below repeats (take, confirm on a blanked frame,
/// map), returning the locker's client index.
fn lock_session_with_surface(fixture: &mut Fixture) -> usize {
    let locker = fixture.spawn(run_locker);
    fixture.send_step(locker, Step::TakeSessionLock);
    fixture.render();
    let Ack::SessionLocked = fixture.wait_for_ack(locker) else {
        panic!("the session lock answered with something other than `locked`");
    };
    fixture.settle();
    fixture.run_on(locker, Step::MapLockSurface);
    locker
}

/// Proves the unlock the way the old round-trip test did: the unlock event
/// is silent by protocol, so a fresh lock afterwards must confirm -- a
/// still-locked session would refuse it with `finished`.
fn unlock_session(fixture: &mut Fixture, locker: usize) {
    fixture.send_step(locker, Step::ReleaseSessionLock);
    let Ack::SessionReleased = fixture.wait_for_ack(locker) else {
        panic!("the session unlock answered with something unexpected");
    };
    fixture.settle();
}

#[test]
fn a_session_lock_deactivates_a_held_confinement() {
    // The ticket's confine question, answered NO and pinned: a held
    // confine does *not* freeze focus at lock, so it needs no new
    // treatment. The lock-time refresh is a zero-delta move whose origin
    // re-derivation runs under the locked hit test, which finds nothing
    // (no lock surface mapped yet) -- so `absolute_target` takes its
    // fail-open (`Free`) rather than `Held`, the refresh delivers a `leave`
    // to the game, and Smithay's own leave path deactivates the
    // confinement. The game therefore already sees `unconfined` and the
    // lock surface already takes focus; unlock re-arms through the
    // ordinary arrival path. This test pins that correct behavior so the
    // lock fix cannot regress it.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let Ack::Confined = fixture.run_on(0, Step::Confine) else {
        panic!("confining answered with something other than `confined`");
    };
    let locker = lock_session_with_surface(&mut fixture);
    let during = fixture.report(0);
    assert!(
        during.unconfined,
        "the session lock left a held confinement active"
    );
    assert!(
        fixture.locker_report(locker).enters >= 1,
        "the lock surface never received pointer focus under session lock"
    );
    locked_input(&mut fixture, fx, fy);
    let silent = fixture.report(0);
    assert!(
        silent.relatives.is_empty()
            && silent.motions.is_empty()
            && silent.buttons == 0
            && silent.axis == 0,
        "pointer input streamed to the game across the lock: {silent:?}"
    );
    unlock_session(&mut fixture, locker);
    let back = fixture.report(0);
    assert!(
        back.confined,
        "unlocking did not re-arm the game's persistent confinement"
    );
    assert!(
        back.enters >= 1,
        "pointer focus did not return to the game surface on unlock"
    );
    // And it confines from there: a move far off the surface reports the
    // full vector while absolute stays put.
    fixture.state.pointer_move(1500.0, 900.0);
    let _ = fixture.state.display_handle.flush_clients();
    let held = fixture.report(0);
    let (dx, dy) = (1500.0 - (fx + 10.0), 900.0 - (fy + 10.0));
    assert_eq!(
        held.relatives,
        vec![(dx, dy, dx, dy)],
        "the re-armed confinement lost its relative stream"
    );
    assert!(
        held.motions.is_empty(),
        "absolute motion escaped the re-armed confinement: {:?}",
        held.motions
    );
}

#[test]
fn locking_with_no_active_constraint_changes_nothing() {
    // The fix must not move what was never frozen: a focused game with no
    // lock or confinement takes the ordinary lock path -- focus to the
    // lock surface, silence for the game, focus back on unlock -- exactly
    // as before this change.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let locker = lock_session_with_surface(&mut fixture);
    let during = fixture.report(0);
    assert!(
        !during.unlocked && !during.unconfined,
        "locking deactivated a constraint that was never active"
    );
    assert_eq!(
        during.enters, 0,
        "pointer focus re-entered the game surface under session lock"
    );
    assert!(
        fixture.locker_report(locker).enters >= 1,
        "the lock surface never received pointer focus under session lock"
    );
    locked_input(&mut fixture, fx, fy);
    let silent = fixture.report(0);
    assert!(
        silent.relatives.is_empty()
            && silent.motions.is_empty()
            && silent.buttons == 0
            && silent.axis == 0,
        "pointer input reached the unfocused game under session lock: {silent:?}"
    );
    let heard = fixture.locker_report(locker);
    assert!(
        !heard.motions.is_empty() && heard.buttons == 2 && heard.axis >= 1,
        "locked input did not reach the lock surface: {heard:?}"
    );
    unlock_session(&mut fixture, locker);
    let back = fixture.report(0);
    assert!(
        back.enters >= 1,
        "pointer focus did not return to the game surface on unlock"
    );
    // No lock was ever taken, so motion after unlock moves absolute as
    // well as reporting relative.
    let (px, py) = fixture.pointer_at();
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.state.pointer_move(x + 20.0, y + 20.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    let (dx, dy) = (x + 20.0 - px, y + 20.0 - py);
    assert_eq!(
        report.relatives,
        vec![(dx, dy, dx, dy)],
        "relative motion stopped after an unconstrained lock round trip"
    );
    assert!(
        !report.motions.is_empty(),
        "absolute motion did not resume after an unconstrained lock round trip"
    );
}

#[test]
fn a_lock_requested_while_locked_stays_inactive_until_unlock() {
    // Lock-wins: between the lock request and the lock transition there is
    // no dispatch, so the only race is a client asking *after* the session
    // locked. Activation is focus-gated, and while locked nothing but a
    // lock surface can hold focus, so the request sits inactive -- no
    // `locked`, no stream to the game -- and the ordinary arrival path
    // engages it on unlock.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let (fx, fy) = window_point(&fixture, 0, run.surface);
    let locker = lock_session_with_surface(&mut fixture);
    let Ack::Done = fixture.run_on(0, Step::Arm) else {
        panic!("arming the lock answered with something else");
    };
    let armed = fixture.report(0);
    assert!(
        !armed.locked,
        "a lock requested under session lock activated"
    );
    locked_input(&mut fixture, fx, fy);
    let silent = fixture.report(0);
    assert!(
        silent.relatives.is_empty()
            && silent.motions.is_empty()
            && silent.buttons == 0
            && silent.axis == 0,
        "pointer input reached the game before its lock engaged: {silent:?}"
    );
    unlock_session(&mut fixture, locker);
    let back = fixture.report(0);
    assert!(
        back.locked,
        "unlocking did not engage the lock requested while locked"
    );
    assert!(
        back.enters >= 1,
        "pointer focus did not return to the game surface on unlock"
    );
    let (px, py) = fixture.pointer_at();
    let (x, y) = window_point(&fixture, 0, run.surface);
    fixture.state.pointer_move(x + 20.0, y + 20.0);
    let _ = fixture.state.display_handle.flush_clients();
    let report = fixture.report(0);
    let (dx, dy) = (x + 20.0 - px, y + 20.0 - py);
    assert!(
        report.motions.is_empty(),
        "absolute motion leaked through the engaged lock: {:?}",
        report.motions
    );
    assert_eq!(
        report.relatives,
        vec![(dx, dy, dx, dy)],
        "the engaged lock lost its relative stream"
    );
}

#[test]
fn unlocking_with_the_locked_game_client_gone_does_not_panic() {
    // The deactivation sends `unlocked` to a live client at lock time, but
    // nothing is owed afterwards: the game disconnects mid-lock, and the
    // unlock must neither send to the dead client nor wedge the session.
    let (mut fixture, run) = Fixture::start();
    fixture.focus(0, run.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    let locker = lock_session_with_surface(&mut fixture);
    assert!(
        fixture.report(0).unlocked,
        "the session lock left a held pointer lock active"
    );
    fixture.disconnect(0);
    fixture.settle();
    unlock_session(&mut fixture, locker);
    // The session is usable afterwards: a fresh lock confirms instead of
    // refusing with `finished`.
    fixture.send_step(locker, Step::TakeSessionLock);
    fixture.render();
    let Ack::SessionLocked = fixture.wait_for_ack(locker) else {
        panic!("re-locking after the unlock did not confirm it");
    };
    unlock_session(&mut fixture, locker);
}

#[test]
fn a_session_lock_deactivates_only_the_focused_constraint() {
    // Lock means nobody gets the pointer but the locker -- and it takes
    // exactly one deactivation to get there. Only a focused surface can
    // hold an *active* constraint (activation is focus-gated at creation
    // and on arrival; Smithay deactivates on leave), and the seat holds a
    // single focus, so the focused client's lock is the only active one.
    // The second client's armed-but-inactive lock must be left alone: no
    // `unlocked` for what never activated, no `locked` without an arrival.
    let (mut fixture, run_a) = Fixture::start();
    fixture.spawn(run_client);
    let Ack::Started { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported being mapped before its globals");
    };
    let Ack::Mapped { .. } = fixture.wait_for_ack(1) else {
        panic!("the second client reported observations before being mapped");
    };
    fixture.focus(0, run_a.surface);
    let (fx, fy) = window_point(&fixture, 0, run_a.surface);
    let Ack::Locked = fixture.run_on(0, Step::Lock) else {
        panic!("locking answered with something other than `locked`");
    };
    let Ack::Done = fixture.run_on(1, Step::Arm) else {
        panic!("arming the second lock answered with something else");
    };
    assert!(
        !fixture.report(1).locked,
        "an unfocused lock activated at creation time"
    );
    let locker = lock_session_with_surface(&mut fixture);
    assert!(
        fixture.report(0).unlocked,
        "the session lock left the focused lock active"
    );
    let other = fixture.report(1);
    assert!(
        !other.unlocked && !other.locked,
        "the session lock touched the unfocused client's inactive lock"
    );
    locked_input(&mut fixture, fx, fy);
    for (index, what) in [(0, "focused"), (1, "unfocused")] {
        let silent = fixture.report(index);
        assert!(
            silent.relatives.is_empty()
                && silent.motions.is_empty()
                && silent.buttons == 0
                && silent.axis == 0,
            "pointer input reached the {what} game under session lock: {silent:?}"
        );
    }
    unlock_session(&mut fixture, locker);
    assert!(
        fixture.report(0).locked,
        "unlocking did not re-arm the focused client's lock"
    );
    assert!(
        !fixture.report(1).locked,
        "unlocking engaged a lock whose surface was never entered"
    );
}
