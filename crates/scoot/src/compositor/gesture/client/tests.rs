//! What a real client's gesture objects receive, over a real socket: the
//! global's opt-in, pinch and hold forwarding, the begin-decides-and-end-
//! matches rule, and a focus change in the middle of a gesture.
//!
//! The libinput arms in `tty/mod.rs` are one-line calls into the `State`
//! methods driven here, and no libinput device exists in a test, so the
//! methods are the entry points.

use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
    zwp_pointer_gestures_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::init_client_gestures;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

const CANVAS: i32 = 400;

/// One gesture event as a client's object received it, in arrival order.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::compositor) enum Ev {
    SwipeBegin(u32),
    SwipeUpdate(f64, f64),
    SwipeEnd(bool),
    PinchBegin(u32),
    PinchUpdate(f64, f64, f64, f64),
    PinchEnd(bool),
    HoldBegin(u32),
    HoldEnd(bool),
}

pub(in crate::compositor) enum Step {
    /// Map a toplevel and paint it at the size the compositor configured.
    Map,
    /// Destroy the mapped toplevel and its surface.
    Unmap,
    Report,
    /// Watch the seat for `timeout_ms` of quiet (`ext_idle_notifier_v1`).
    Watch {
        timeout_ms: u32,
    },
}

#[derive(Debug, Default)]
pub(in crate::compositor) struct Ack {
    /// The registry listed `zwp_pointer_gestures_v1`.
    pub(in crate::compositor) has_global: bool,
    /// The version the client bound it at.
    pub(in crate::compositor) version: u32,
    /// The pointer is on this client's surface (enter without leave).
    pub(in crate::compositor) inside: bool,
    /// Gesture events since the last report.
    pub(in crate::compositor) events: Vec<Ev>,
    /// `wl_pointer.frame` events since the last report.
    pub(in crate::compositor) frames: u32,
    /// Cumulative idle-notify `idled` events.
    pub(in crate::compositor) idled: u32,
    /// Cumulative idle-notify `resumed` events.
    pub(in crate::compositor) resumed: u32,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    gestures: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    version: u32,
    /// The size the compositor's last `xdg_toplevel.configure` asked for.
    configured: (i32, i32),
    inside: bool,
    events: Vec<Ev>,
    frames: u32,
    idle_notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    idled: u32,
    resumed: u32,
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
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "zwp_pointer_gestures_v1" => {
                client.version = version;
                client.gestures = Some(registry.bind(name, version.min(3), qh, ()));
            }
            "ext_idle_notifier_v1" => {
                client.idle_notifier = Some(registry.bind(name, version.min(2), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => client.idled += 1,
            ext_idle_notification_v1::Event::Resumed => client.resumed += 1,
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
            wl_pointer::Event::Enter { .. } => client.inside = true,
            wl_pointer::Event::Leave { .. } => client.inside = false,
            wl_pointer::Event::Frame => client.frames += 1,
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_swipe_v1::Event;
        match event {
            Event::Begin { fingers, .. } => client.events.push(Ev::SwipeBegin(fingers)),
            Event::Update { dx, dy, .. } => client.events.push(Ev::SwipeUpdate(dx, dy)),
            Event::End { cancelled, .. } => client.events.push(Ev::SwipeEnd(cancelled != 0)),
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_pinch_v1::Event;
        match event {
            Event::Begin { fingers, .. } => client.events.push(Ev::PinchBegin(fingers)),
            Event::Update {
                dx,
                dy,
                scale,
                rotation,
                ..
            } => client.events.push(Ev::PinchUpdate(dx, dy, scale, rotation)),
            Event::End { cancelled, .. } => client.events.push(Ev::PinchEnd(cancelled != 0)),
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_hold_v1::Event;
        match event {
            Event::Begin { fingers, .. } => client.events.push(Ev::HoldBegin(fingers)),
            Event::End { cancelled, .. } => client.events.push(Ev::HoldEnd(cancelled != 0)),
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            client.configured = (width, height);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);
wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore zwp_pointer_gestures_v1::ZwpPointerGesturesV1);

/// A solid `width`x`height` buffer over a memfd-backed pool, so the window
/// has a size the pointer can land on.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
) -> wl_buffer::WlBuffer {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-gesture-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len]).expect("a filled pool");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
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
    // Twice: the second delivers the seat's capabilities, which is when the
    // pointer (and so the gesture objects) can be made.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let pointer = client.pointer.clone().ok_or("no wl_pointer")?;
    // Held for the whole run: dropping a proxy does not destroy the object,
    // but the client owns them like a toolkit would.
    let _objects = client.gestures.clone().map(|gestures| {
        (
            gestures.get_swipe_gesture(&pointer, &qh, ()),
            gestures.get_pinch_gesture(&pointer, &qh, ()),
            (client.version >= 3).then(|| gestures.get_hold_gesture(&pointer, &qh, ())),
        )
    });
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let mut watches = Vec::new();
    let mut mapped: Option<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )> = None;

    while let Ok(step) = steps.recv() {
        match step {
            Step::Map => {
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let (width, height) = match client.configured {
                    (w, h) if w > 0 && h > 0 => (w, h),
                    _ => (100, 100),
                };
                let buffer = solid_buffer(&shm, &qh, width, height);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width, height);
                surface.commit();
                mapped = Some((surface, xdg, toplevel));
            }
            Step::Unmap => {
                if let Some((surface, xdg, toplevel)) = mapped.take() {
                    toplevel.destroy();
                    xdg.destroy();
                    surface.destroy();
                }
            }
            Step::Report => {}
            Step::Watch { timeout_ms } => {
                let notifier = client
                    .idle_notifier
                    .clone()
                    .ok_or("no ext_idle_notifier_v1")?;
                let seat = client.seat.clone().ok_or("no wl_seat")?;
                // Held for the run: the watch ends with the object.
                watches.push(notifier.get_idle_notification(timeout_ms, &seat, &qh, ()));
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(Ack {
            has_global: client.gestures.is_some(),
            version: client.version,
            inside: client.inside,
            events: std::mem::take(&mut client.events),
            frames: std::mem::take(&mut client.frames),
            idled: client.idled,
            resumed: client.resumed,
        })
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

/// A compositor with the global on or off and `clients` mapped, painted
/// windows, the pointer parked on the first.
fn fixture(advertise: bool, clients: usize) -> Fixture {
    let mut fixture: Fixture = Harness::headless(Appearance::default(), CANVAS);
    init_client_gestures(&mut fixture.state, advertise);
    for index in 0..clients {
        assert_eq!(fixture.spawn(run_client), index);
        fixture.run_on(index, Step::Map);
    }
    if clients > 0 {
        point_at(&mut fixture, 0);
        assert!(
            fixture.run_on(0, Step::Report).inside,
            "premise: the pointer is on the first window"
        );
    }
    fixture
}

/// Moves the pointer to the middle of the `index`th window's frame.
fn point_at(fixture: &mut Fixture, index: usize) {
    let arrangement = fixture.state.world.arrange();
    let rect = arrangement
        .placements
        .get(index)
        .unwrap_or_else(|| panic!("no window {index} in {arrangement:?}"))
        .rect;
    fixture.state.pointer_move(
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    );
    fixture.settle();
}

/// Moves the pointer to a corner no window covers.
fn point_at_nothing(fixture: &mut Fixture) {
    fixture.state.pointer_move(f64::from(CANVAS - 1), 1.0);
    fixture.settle();
}

/// What `index` heard since its last report.
fn heard(fixture: &mut Fixture, index: usize) -> Vec<Ev> {
    fixture.run_on(index, Step::Report).events
}

#[test]
fn the_global_is_absent_by_default() {
    let mut fixture = fixture(false, 1);
    let ack = fixture.run(Step::Report);
    assert!(!ack.has_global, "zwp_pointer_gestures_v1 is on by default");
    assert!(fixture.state.client_gestures.is_none());
    // And pinch and hold are dropped before they do anything, as when these
    // arms were `_ => {}`: no cursor reshow timer, no idle activity.
    fixture.state.appearance.cursor_hide_after_ms = 1000;
    fixture.state.gesture_pinch_begin(2, 1000);
    fixture.state.gesture_pinch_end(false, 1010);
    fixture.state.gesture_hold_begin(2, 1020);
    fixture.state.gesture_hold_end(false, 1030);
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "a pinch or hold with no global counted as pointer activity"
    );
    fixture.state.gesture_swipe_begin(4, 1040);
    fixture.state.gesture_swipe_end(false, 1050);
    assert!(heard(&mut fixture, 0).is_empty());
}

#[test]
fn with_the_global_a_pinch_or_hold_is_pointer_activity() {
    let mut fixture = fixture(true, 1);
    fixture.state.appearance.cursor_hide_after_ms = 1000;
    assert!(fixture.state.cursor_hide_deadline.is_none());
    fixture.state.gesture_hold_begin(2, 1000);
    assert!(fixture.state.cursor_hide_deadline.is_some());
    fixture.state.cursor_hide_deadline = None;
    fixture.state.gesture_pinch_begin(2, 1010);
    assert!(fixture.state.cursor_hide_deadline.is_some());
}

/// Polls the client until its cumulative `(idled, resumed)` satisfy `done`,
/// failing at a deadline instead of hanging.
fn wait_for_idle_counts(fixture: &mut Fixture, done: impl Fn(u32, u32) -> bool) -> (u32, u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let ack = fixture.run_on(0, Step::Report);
        if done(ack.idled, ack.resumed) {
            return (ack.idled, ack.resumed);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out; saw idled={} resumed={}",
            ack.idled,
            ack.resumed
        );
    }
}

/// Lets the seat go idle under a watch, then runs `gesture`, which must
/// resume it. A removed `announce_activity()` leaves the seat idle for the
/// whole `QUIET` window, so no `resumed` arrives and this fails.
fn assert_gesture_resumes_an_idle_seat(gesture: impl FnOnce(&mut crate::compositor::State)) {
    const QUIET: std::time::Duration = std::time::Duration::from_millis(500);
    let mut fixture = fixture(true, 1);
    fixture.run_on(0, Step::Watch { timeout_ms: 50 });
    wait_for_idle_counts(&mut fixture, |idled, _| idled >= 1);
    let before = fixture.run_on(0, Step::Report).resumed;
    assert_eq!(before, 0, "premise: nothing resumed the seat yet");

    gesture(&mut fixture.state);

    let deadline = std::time::Instant::now() + QUIET;
    let mut resumed = 0;
    while resumed == 0 && std::time::Instant::now() < deadline {
        resumed = fixture.run_on(0, Step::Report).resumed;
    }
    assert!(resumed >= 1, "the gesture did not count as idle activity");
}

#[test]
fn a_pinch_begin_is_idle_activity() {
    assert_gesture_resumes_an_idle_seat(|state| state.gesture_pinch_begin(2, 1000));
}

#[test]
fn a_hold_begin_is_idle_activity() {
    assert_gesture_resumes_an_idle_seat(|state| state.gesture_hold_begin(2, 1000));
}

#[test]
fn the_global_is_advertised_at_version_3_when_enabled() {
    let mut fixture = fixture(true, 1);
    let ack = fixture.run(Step::Report);
    assert!(ack.has_global);
    assert_eq!(ack.version, 3, "hold needs version 3");
    assert!(fixture.state.client_gestures.is_some());
}

#[test]
fn a_pinch_reaches_the_focused_client_whole() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    state.gesture_pinch_begin(2, 1000);
    state.gesture_pinch_update((1.0, -2.0), 1.5, 10.0, 1010);
    state.gesture_pinch_update((0.5, 0.0), 2.0, -4.0, 1020);
    state.gesture_pinch_end(false, 1030);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![
            Ev::PinchBegin(2),
            Ev::PinchUpdate(1.0, -2.0, 1.5, 10.0),
            Ev::PinchUpdate(0.5, 0.0, 2.0, -4.0),
            Ev::PinchEnd(false),
        ]
    );
}

#[test]
fn a_hold_reaches_the_focused_client_and_a_cancelled_one_says_so() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    state.gesture_hold_begin(3, 1000);
    state.gesture_hold_end(false, 1100);
    // libinput ends a hold with `cancelled` when the fingers moved on.
    state.gesture_hold_begin(2, 1200);
    state.gesture_hold_end(true, 1210);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![
            Ev::HoldBegin(3),
            Ev::HoldEnd(false),
            Ev::HoldBegin(2),
            Ev::HoldEnd(true),
        ]
    );
}

#[test]
fn a_swipe_that_is_not_scoots_reaches_the_client_and_one_that_is_does_not() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    // Four fingers: not `swipe_fingers` (3), so the client's.
    state.gesture_swipe_begin(4, 1000);
    state.gesture_swipe_update(3.0, -1.5, 1010);
    state.gesture_swipe_end(false, 1020);
    // Three fingers: scoot's, and the client hears none of it.
    state.gesture_swipe_begin(3, 2000);
    state.gesture_swipe_update(-30.0, 0.5, 2010);
    state.gesture_swipe_end(false, 2020);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![
            Ev::SwipeBegin(4),
            Ev::SwipeUpdate(3.0, -1.5),
            Ev::SwipeEnd(false)
        ]
    );
}

#[test]
fn an_update_or_end_without_a_begin_reaches_nobody() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    // Nothing open: every one of these is dropped.
    state.gesture_pinch_update((1.0, 1.0), 1.2, 3.0, 1000);
    state.gesture_pinch_end(false, 1010);
    state.gesture_hold_end(false, 1020);
    state.gesture_swipe_update(1.0, 1.0, 1030);
    state.gesture_swipe_end(false, 1040);
    // Closed: the update after an end, and a second end, are dropped too.
    state.gesture_pinch_begin(2, 2000);
    state.gesture_pinch_end(false, 2010);
    state.gesture_pinch_update((1.0, 1.0), 1.2, 3.0, 2020);
    state.gesture_pinch_end(false, 2030);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![Ev::PinchBegin(2), Ev::PinchEnd(false)]
    );
}

#[test]
fn a_non_finite_pinch_update_is_dropped_not_sent() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    state.gesture_pinch_begin(2, 1000);
    state.gesture_pinch_update((f64::NAN, 0.0), 1.0, 0.0, 1010);
    state.gesture_pinch_update((0.0, 0.0), f64::INFINITY, 0.0, 1020);
    state.gesture_pinch_update((0.0, 0.0), 1.0, f64::NAN, 1030);
    state.gesture_pinch_end(false, 1040);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![Ev::PinchBegin(2), Ev::PinchEnd(false)]
    );
}

#[test]
fn a_begin_over_an_open_gesture_closes_the_old_one_cancelled() {
    let mut fixture = fixture(true, 1);
    let state = &mut fixture.state;
    state.gesture_pinch_begin(2, 1000);
    state.gesture_pinch_begin(3, 1010);
    state.gesture_pinch_end(false, 1020);
    state.gesture_hold_begin(2, 2000);
    state.gesture_hold_begin(3, 2010);
    state.gesture_hold_end(false, 2020);
    state.gesture_swipe_begin(4, 3000);
    state.gesture_swipe_begin(4, 3010);
    state.gesture_swipe_end(false, 3020);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![
            Ev::PinchBegin(2),
            Ev::PinchEnd(true),
            Ev::PinchBegin(3),
            Ev::PinchEnd(false),
            Ev::HoldBegin(2),
            Ev::HoldEnd(true),
            Ev::HoldBegin(3),
            Ev::HoldEnd(false),
            Ev::SwipeBegin(4),
            Ev::SwipeEnd(true),
            Ev::SwipeBegin(4),
            Ev::SwipeEnd(false),
        ]
    );
}

#[test]
fn a_gesture_sends_no_pointer_frame() {
    let mut fixture = fixture(true, 1);
    let before = fixture.run(Step::Report).frames;
    assert_eq!(before, 0, "the report drained the enter's frames");
    let state = &mut fixture.state;
    for i in 0..50 {
        state.gesture_pinch_begin(2, 1000 + 100 * i);
        state.gesture_pinch_update((1.0, 1.0), 1.1, 1.0, 1010 + 100 * i);
        state.gesture_pinch_end(false, 1020 + 100 * i);
    }
    fixture.settle();
    let ack = fixture.run(Step::Report);
    assert_eq!(ack.frames, 0, "a gesture woke the client with a frame");
    assert_eq!(ack.events.len(), 150);
}

/// Item 3 of the ticket: the pointer leaves the surface a swipe began on.
/// Smithay's `leave` ends it there, cancelled, exactly once; the surface the
/// pointer lands on never saw the `begin` and hears nothing.
#[test]
fn a_focus_change_mid_swipe_cancels_it_on_the_old_client_and_tells_the_new_nothing() {
    let mut fixture = fixture(true, 2);
    point_at(&mut fixture, 0);
    assert!(fixture.run_on(0, Step::Report).inside);
    let _ = heard(&mut fixture, 1);
    fixture.state.gesture_swipe_begin(4, 1000);
    fixture.state.gesture_swipe_update(2.0, 0.0, 1010);
    point_at(&mut fixture, 1);
    assert!(fixture.run_on(1, Step::Report).inside, "premise: moved");
    fixture.state.gesture_swipe_update(2.0, 0.0, 1020);
    fixture.state.gesture_swipe_end(false, 1030);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![
            Ev::SwipeBegin(4),
            Ev::SwipeUpdate(2.0, 0.0),
            Ev::SwipeEnd(true)
        ],
        "the old focus gets exactly one cancelled end, and nothing after it"
    );
    assert_eq!(
        heard(&mut fixture, 1),
        vec![],
        "the new focus never saw the begin"
    );
}

#[test]
fn a_focus_change_mid_pinch_or_hold_is_cancelled_the_same_way() {
    let mut fixture = fixture(true, 2);
    point_at(&mut fixture, 0);
    let _ = heard(&mut fixture, 1);
    fixture.state.gesture_pinch_begin(2, 1000);
    fixture.state.gesture_hold_begin(2, 1001);
    point_at(&mut fixture, 1);
    fixture
        .state
        .gesture_pinch_update((0.0, 0.0), 1.3, 0.0, 1010);
    fixture.state.gesture_pinch_end(false, 1020);
    fixture.state.gesture_hold_end(false, 1021);
    fixture.settle();
    let mut old = heard(&mut fixture, 0);
    // The pinch and the hold are separate objects: only each one's own order
    // is promised.
    let pick = |events: &[Ev], keep: fn(&Ev) -> bool| -> Vec<Ev> {
        events.iter().filter(|ev| keep(ev)).cloned().collect()
    };
    assert_eq!(
        pick(&old, |ev| matches!(
            ev,
            Ev::PinchBegin(_) | Ev::PinchUpdate(..) | Ev::PinchEnd(_)
        )),
        vec![Ev::PinchBegin(2), Ev::PinchEnd(true)]
    );
    assert_eq!(
        pick(&old, |ev| matches!(ev, Ev::HoldBegin(_) | Ev::HoldEnd(_))),
        vec![Ev::HoldBegin(2), Ev::HoldEnd(true)]
    );
    old.clear();
    assert_eq!(heard(&mut fixture, 1), vec![]);
}

#[test]
fn leaving_every_window_mid_gesture_cancels_it_once() {
    let mut fixture = fixture(true, 1);
    fixture.state.gesture_swipe_begin(4, 1000);
    point_at_nothing(&mut fixture);
    fixture.state.gesture_swipe_update(2.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(false, 1020);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 0),
        vec![Ev::SwipeBegin(4), Ev::SwipeEnd(true)]
    );
}

#[test]
fn entering_a_window_mid_gesture_gets_no_update_or_end() {
    let mut fixture = fixture(true, 1);
    point_at_nothing(&mut fixture);
    let _ = heard(&mut fixture, 0);
    // The begin went nowhere (no pointer focus); the fingers then carry the
    // pointer onto the window and lift there.
    fixture.state.gesture_swipe_begin(4, 1000);
    fixture.state.gesture_pinch_begin(2, 1001);
    point_at(&mut fixture, 0);
    fixture.state.gesture_swipe_update(2.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(false, 1020);
    fixture
        .state
        .gesture_pinch_update((0.0, 0.0), 1.2, 0.0, 1011);
    fixture.state.gesture_pinch_end(false, 1021);
    fixture.settle();
    assert_eq!(heard(&mut fixture, 0), vec![]);
}

#[test]
fn a_window_closing_mid_gesture_leaves_the_compositor_and_the_next_gesture_whole() {
    let mut fixture = fixture(true, 2);
    point_at(&mut fixture, 0);
    let _ = heard(&mut fixture, 1);
    fixture.state.gesture_pinch_begin(2, 1000);
    fixture.run_on(0, Step::Unmap);
    fixture
        .state
        .gesture_pinch_update((0.0, 0.0), 1.2, 0.0, 1010);
    fixture.state.gesture_pinch_end(false, 1020);
    fixture.settle();
    // Whatever client 0 heard, client 1 was never part of that pinch.
    assert_eq!(heard(&mut fixture, 1), vec![]);
    // A whole gesture afterwards reaches the window the pointer is over.
    point_at(&mut fixture, 0);
    assert!(
        fixture.run_on(1, Step::Report).inside,
        "premise: client 1 is under it"
    );
    fixture.state.gesture_pinch_begin(2, 2000);
    fixture.state.gesture_pinch_end(false, 2010);
    fixture.settle();
    assert_eq!(
        heard(&mut fixture, 1),
        vec![Ev::PinchBegin(2), Ev::PinchEnd(false)]
    );
}
