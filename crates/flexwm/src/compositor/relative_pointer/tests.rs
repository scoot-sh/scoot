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
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

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
    },
    /// The `locked` event arrived.
    Locked,
    /// The `confined` event arrived.
    Confined,
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
#[derive(Default)]
struct Report {
    enters: u32,
    motions: Vec<(f64, f64)>,
    /// `(dx, dy, dx_unaccel, dy_unaccel)` per `relative_motion`.
    relatives: Vec<(f64, f64, f64, f64)>,
    locked: bool,
    unlocked: bool,
    confined: bool,
    unconfined: bool,
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
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);

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
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
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
