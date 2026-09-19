//! `wl_data_device.start_drag` serial validation: which serials buy a drag,
//! and which are refused.
//!
//! These drive *real* `wayland-client` connections -- binding the data-device
//! manager and calling `start_drag` exactly as a toolkit does -- through a
//! real [`State`] with a real headless backend. What is under test is what
//! the *client* was told (`wl_data_source.cancelled`, `wl_data_device.enter`),
//! because "who holds the pointer" is a claim about the wire.
//!
//! The choreography needs the compositor to press and release a real button
//! around the client's request (a drag requires a live implicit grab), so
//! each test runs its client script on a thread while the test body pumps
//! the compositor and injects pointer input between stages, rendezvousing
//! over channels. `Mapped` → the test moves the pointer onto the window and
//! presses; `ButtonSeen` → the test tells the script to attempt the drag;
//! `DragAttempted` → the test releases and finishes. The button stays held
//! for the whole attempt, which is what makes the refusal timing airtight: a
//! refused source is cancelled *while held*, an accepted one only at release
//! (a drop no target accepted ends in `cancelled` too).
//!
//! Like `super::tests`, these need a writable `$XDG_RUNTIME_DIR` for the
//! same reason (`State::new` binds a real listening socket either way).

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use scoot_core::Config;
use scoot_ipc::PointerButton;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Logical, Point};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// What the drag tests offer. Distinctive on purpose, matching `super::tests`.
const MIME: &str = "text/plain";

/// How long a client script may take before the compositor counts as not
/// answering. Generous: a debug build on a VM.
const PATIENCE: Duration = Duration::from_secs(20);

/// Size of the headless output the harness builds, in logical pixels.
const CANVAS: f64 = 200.0;

/// A live compositor with a real headless backend, serving client threads
/// that each run one script to completion.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
}

impl Harness {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, 200, 200).expect("a headless backend");
        Self { event_loop, state }
    }

    /// Connects one client over a socket pair (no dependence on the real
    /// listening socket's name) and runs `script` on its thread.
    fn run_client(
        &mut self,
        script: impl FnOnce(ClientConn) -> Result<String, String> + Send + 'static,
    ) -> JoinHandle<Result<String, String>> {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");
        std::thread::spawn(move || {
            let conn = ClientConn::new(client_end)?;
            script(conn)
        })
    }

    /// Pumps the compositor once.
    fn pump(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut self.state)
            .expect("a compositor dispatch");
    }
}

/// The client end of one connection: the socket, queue and dispatch state.
struct ClientConn {
    queue: wayland_client::EventQueue<Client>,
    client: Client,
}

impl ClientConn {
    fn new(stream: UnixStream) -> Result<Self, String> {
        let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = Client::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        Ok(Self { queue, client })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.client)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Everything a drag client binds or observes.
#[derive(Default)]
struct Client {
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Serial of the last button *press* the client was sent, if any.
    button_serial: Option<u32>,
    keyboard_focus: Option<wl_surface::WlSurface>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    dd_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    /// Whether this client's data source has been cancelled.
    cancelled: bool,
    /// Whether this client's data source saw `dnd_finished`.
    finished: bool,
    /// Mime types offered to this client's device, in order.
    offered_mimes: Vec<String>,
    /// Drag `enter` events this client's device was sent.
    enters: u32,
    xdg_serial: Option<u32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
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
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(7), qh, ())),
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_data_device_manager" => {
                client.dd_manager = Some(registry.bind(name, version.min(3), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { .. } = event {
            if client.pointer.is_none() {
                client.pointer = Some(seat.get_pointer(qh, ()));
            }
            if client.keyboard.is_none() {
                client.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button { serial, state, .. } = event
            && state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed)
        {
            client.button_serial = Some(serial);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { surface, .. } = event {
            client.keyboard_focus = Some(surface);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            client.xdg_serial = Some(serial);
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_data_device_manager::WlDataDeviceManager,
        _: wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { .. } => {}
            wl_data_device::Event::Enter { .. } => client.enters += 1,
            _ => {}
        }
    }

    /// The `data_offer` event carries the offer as a server-created `new_id`,
    /// so the client has to say what user data it gets. Opcode 0 is
    /// `data_offer`, the only child-creating event on this interface.
    fn event_created_child(
        opcode: u16,
        qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        assert_eq!(opcode, 0, "the only child here is data_offer");
        qh.make_data::<wl_data_offer::WlDataOffer, ()>(())
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            client.offered_mimes.push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Cancelled => client.cancelled = true,
            wl_data_source::Event::DndFinished => client.finished = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

/// A `size`x`size` `wl_buffer`, over a real memfd -- the same path any
/// toolkit takes to map a window.
fn solid_buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<Client>, size: i32) -> wl_buffer::WlBuffer {
    use rustix::fs::{MemfdFlags, memfd_create};

    let stride = size * 4;
    let len = (stride * size) as usize;
    let fd = memfd_create("scoot-dnd-test", MemfdFlags::CLOEXEC).expect("a memfd");
    let mut file = std::fs::File::from(fd);
    use std::io::Write;
    file.write_all(&vec![0u8; len]).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, size, size, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Maps one window the toolkit way and waits for keyboard focus, returning
/// the surface (the drag origin). Every drag script starts here: without a
/// mapped, focused surface no press is ever delivered to it.
fn map_window(conn: &mut ClientConn) -> Result<wl_surface::WlSurface, String> {
    let qh = conn.queue.handle();
    let compositor = conn.client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = conn.client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = conn.client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let _toplevel = xdg.get_toplevel(&qh, ());
    surface.commit();
    wait_for_event(conn, "an xdg configure", |client| {
        client.xdg_serial.is_some()
    })?;
    xdg.ack_configure(conn.client.xdg_serial.expect("the serial"));
    let buffer = solid_buffer(&shm, &qh, 40);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, 40, 40);
    surface.commit();
    wait_for_event(conn, "keyboard focus", |client| {
        client.keyboard_focus.is_some()
    })?;
    Ok(surface)
}

/// Rounds the client's queue until `ready` sees what the script is waiting
/// for, or errors when the compositor never sends it. Each round trip is one
/// full client↔compositor exchange, and the test body is pumping
/// concurrently, so a handful is plenty; the bound is against hanging the
/// test suite, not against normal latency.
fn wait_for_event(
    conn: &mut ClientConn,
    what: &str,
    mut ready: impl FnMut(&Client) -> bool,
) -> Result<(), String> {
    for _ in 0..200 {
        conn.roundtrip()?;
        if ready(&conn.client) {
            return Ok(());
        }
    }
    Err(format!("the compositor never sent {what}"))
}

/// Script → test: where the drag script has got to.
#[derive(Debug)]
enum ToTest {
    /// Window mapped and focused; the test should move the pointer on and
    /// press.
    Mapped,
    /// The client was sent a button press carrying this serial.
    ButtonSeen(u32),
    /// `start_drag` sent and settled; whether the source has been cancelled
    /// *while the button is still held*.
    DragAttempted { cancelled: bool },
    /// Final report after release: cancelled yet, and how many drag enters
    /// the device was sent.
    Done { cancelled: bool, enters: u32 },
}

/// Test → script: what to do next.
enum ToClient {
    /// Attempt the drag with the given serial (the test names it so the
    /// refusal cases can pass a serial the client never received).
    StartDrag(u32),
    /// Do final round trips and report.
    Finish,
}

/// Points over the mapped window: a grid scan for surfaces the window map
/// knows, rather than hard-coded coordinates -- the layout decides where a
/// single window lands (a niri-like column, not the canvas middle), and a
/// layout change must not silently turn these tests into clicks on bare
/// desktop.
fn window_points(harness: &mut Harness) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    let mut y = 4.0;
    while y < CANVAS {
        let mut x = 4.0;
        while x < CANVAS {
            if let Some((surface, _)) = harness
                .state
                .surface_under(Point::<f64, Logical>::from((x, y)))
                && harness.state.id_of(&surface).is_some()
            {
                points.push((x, y));
            }
            x += 4.0;
        }
        y += 4.0;
    }
    points
}

/// Moves the pointer onto the window and presses the left button, returning
/// the press point and a second, distinct window point to move to mid-drag.
/// A layout change that leaves no window under the pointer fails fast here
/// instead of timing out in the script.
fn press_on_window(harness: &mut Harness) -> ((f64, f64), (f64, f64)) {
    let points = window_points(harness);
    assert!(
        !points.is_empty(),
        "some point should be over the mapped window"
    );
    let press = points[0];
    // A second point well away from the press, so the mid-drag motion is a
    // real move across the window rather than a zero-delta event; falling
    // back to the press point itself (the drag still enters on any motion,
    // since its focus starts unset).
    let moved = points
        .iter()
        .find(|point| (point.0 - press.0).hypot(point.1 - press.1) >= 12.0)
        .copied()
        .unwrap_or(press);
    harness.state.pointer_move(press.0, press.1);
    for _ in 0..10 {
        harness.pump();
    }
    harness.state.pointer_button(PointerButton::Left, true);
    (press, moved)
}

/// Pumps until the script reports its next stage, failing on timeout. The
/// script runs each stage to completion on its own thread; this only moves
/// the compositor while waiting.
fn next_stage(rx: &Receiver<ToTest>, harness: &mut Harness) -> ToTest {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(stage) = rx.try_recv() {
            return stage;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the client script; the compositor stopped serving"
        );
        harness.pump();
    }
}

/// A drag with the button-press serial just delivered is accepted: no
/// cancellation while held, the pointer routes into the drag (the device
/// sees `enter`), and release ends it.
///
/// This passes with and without the serial gate -- it pins that the gate
/// does not refuse legitimate drags, the counterpart to the refusal below.
#[test]
fn a_drag_with_the_press_serial_is_accepted() {
    let mut harness = Harness::new();
    let (to_test_tx, to_test_rx) = channel::<ToTest>();
    let (to_client_tx, to_client_rx) = channel::<ToClient>();
    let handle = harness.run_client(move |mut conn| {
        let origin = map_window(&mut conn)?;
        to_test_tx.send(ToTest::Mapped).map_err(|e| e.to_string())?;
        wait_for_event(&mut conn, "a button press", |client| {
            client.button_serial.is_some()
        })?;
        let serial = conn.client.button_serial.expect("the press serial");
        to_test_tx
            .send(ToTest::ButtonSeen(serial))
            .map_err(|e| e.to_string())?;
        let ToClient::StartDrag(drag_serial) = to_client_rx.recv().map_err(|e| e.to_string())?
        else {
            return Err("expected StartDrag".into());
        };
        let qh = conn.queue.handle();
        let manager = conn
            .client
            .dd_manager
            .clone()
            .ok_or("no wl_data_device_manager")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        let device = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        source.offer(MIME.to_string());
        device.start_drag(Some(&source), &origin, None, drag_serial);
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        to_test_tx
            .send(ToTest::DragAttempted {
                cancelled: conn.client.cancelled,
            })
            .map_err(|e| e.to_string())?;
        let ToClient::Finish = to_client_rx.recv().map_err(|e| e.to_string())? else {
            return Err("expected Finish".into());
        };
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        to_test_tx
            .send(ToTest::Done {
                cancelled: conn.client.cancelled,
                enters: conn.client.enters,
            })
            .map_err(|e| e.to_string())?;
        Ok("drag script ran".into())
    });

    let ToTest::Mapped = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should map first");
    };
    let (_, moved) = press_on_window(&mut harness);
    let ToTest::ButtonSeen(serial) = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report the press serial");
    };
    to_client_tx
        .send(ToClient::StartDrag(serial))
        .expect("the script to take the drag command");
    let ToTest::DragAttempted { cancelled } = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report the attempt");
    };
    assert!(
        !cancelled,
        "a drag with the press serial just delivered should not be cancelled while held"
    );
    // Still holding: move within the window. A live drag routes the pointer
    // into its targets, so the device must see `enter`; a refused (or never
    // started) drag sends nothing.
    harness.state.pointer_move(moved.0, moved.1);
    for _ in 0..10 {
        harness.pump();
    }
    harness.state.pointer_button(PointerButton::Left, false);
    for _ in 0..10 {
        harness.pump();
    }
    assert!(
        !harness
            .state
            .seat
            .get_pointer()
            .is_some_and(|pointer| pointer.is_grabbed()),
        "release should end the drag, leaving no grab behind"
    );
    to_client_tx
        .send(ToClient::Finish)
        .expect("the script to take the finish command");
    let ToTest::Done { cancelled, enters } = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report its final state");
    };
    assert!(
        enters >= 1,
        "the live drag should have routed at least one enter to the device, saw none"
    );
    assert!(
        cancelled,
        "a drop no target accepted should cancel the source at release"
    );
    assert_eq!(
        handle.join().expect("the client thread").as_deref(),
        Ok("drag script ran"),
    );
}

/// Another client's press serial buys no drag: the check is on the (serial,
/// recipient) pair, which is what makes it a check on interaction rather
/// than on arithmetic -- from this client's side the serial is fabricated,
/// never delivered to it, however right the number is.
///
/// Fail-first: without the gate the drag installs (the live implicit grab
/// carries exactly this serial, which is all Smithay's dispatch checks), so
/// no `cancelled` arrives while held and this fails.
#[test]
fn another_clients_press_serial_starts_no_drag() {
    let mut harness = Harness::new();
    let (a_tx, a_rx) = channel::<ToTest>();
    let (a_cmd_tx, a_cmd_rx) = channel::<ToClient>();
    let a_handle = harness.run_client(move |mut conn| {
        let _origin = map_window(&mut conn)?;
        a_tx.send(ToTest::Mapped).map_err(|e| e.to_string())?;
        wait_for_event(&mut conn, "a button press", |client| {
            client.button_serial.is_some()
        })?;
        let serial = conn.client.button_serial.expect("the press serial");
        a_tx.send(ToTest::ButtonSeen(serial))
            .map_err(|e| e.to_string())?;
        let ToClient::Finish = a_cmd_rx.recv().map_err(|e| e.to_string())? else {
            return Err("expected Finish".into());
        };
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        // Reported through the channel, not just returned: the test body
        // keeps pumping while waiting for this, while it cannot pump while
        // joining -- round trips above would deadlock a bare join.
        a_tx.send(ToTest::Done {
            cancelled: conn.client.cancelled,
            enters: conn.client.enters,
        })
        .map_err(|e| e.to_string())?;
        Ok("holder script ran".into())
    });

    let ToTest::Mapped = next_stage(&a_rx, &mut harness) else {
        panic!("the holder should map first");
    };
    press_on_window(&mut harness);
    let ToTest::ButtonSeen(serial) = next_stage(&a_rx, &mut harness) else {
        panic!("the holder should report the press serial");
    };

    // A second client that received no input at all spends the first
    // client's press serial. Its origin surface is never mapped: the
    // request only needs an origin object, and mapping nothing proves the
    // attempt never earned any interaction of its own.
    let (b_tx, b_rx) = channel::<ToTest>();
    let (b_cmd_tx, b_cmd_rx) = channel::<ToClient>();
    let b_handle = harness.run_client(move |mut conn| {
        let qh = conn.queue.handle();
        for _ in 0..5 {
            conn.roundtrip()?;
        }
        let ToClient::StartDrag(drag_serial) = b_cmd_rx.recv().map_err(|e| e.to_string())? else {
            return Err("expected StartDrag".into());
        };
        let manager = conn
            .client
            .dd_manager
            .clone()
            .ok_or("no wl_data_device_manager")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        let compositor = conn.client.compositor.clone().ok_or("no wl_compositor")?;
        let origin = compositor.create_surface(&qh, ());
        let device = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        source.offer(MIME.to_string());
        device.start_drag(Some(&source), &origin, None, drag_serial);
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        b_tx.send(ToTest::DragAttempted {
            cancelled: conn.client.cancelled,
        })
        .map_err(|e| e.to_string())?;
        Ok("attacker script ran".into())
    });
    // Let the second client bind before attempting.
    for _ in 0..10 {
        harness.pump();
    }
    b_cmd_tx
        .send(ToClient::StartDrag(serial))
        .expect("the attacker to take the drag command");
    let ToTest::DragAttempted { cancelled } = next_stage(&b_rx, &mut harness) else {
        panic!("the attacker should report the attempt");
    };
    assert!(
        cancelled,
        "a drag with another client's press serial should be cancelled while held"
    );

    harness.state.pointer_button(PointerButton::Left, false);
    for _ in 0..10 {
        harness.pump();
    }
    assert!(
        !harness
            .state
            .seat
            .get_pointer()
            .is_some_and(|pointer| pointer.is_grabbed()),
        "release should leave no drag behind"
    );
    a_cmd_tx
        .send(ToClient::Finish)
        .expect("the holder to take the finish command");
    let ToTest::Done { cancelled, enters } = next_stage(&a_rx, &mut harness) else {
        panic!("the holder should report its final state");
    };
    assert!(
        !cancelled,
        "the holder never dragged, so its source should never be cancelled"
    );
    assert_eq!(
        enters, 0,
        "the refused drag should have routed nothing anywhere, saw {enters} enters"
    );
    assert_eq!(
        b_handle.join().expect("the attacker thread").as_deref(),
        Ok("attacker script ran"),
    );
    assert_eq!(
        a_handle.join().expect("the holder thread").as_deref(),
        Ok("holder script ran"),
    );
}

/// A serial nothing was pressed with starts no drag: Smithay's dispatch
/// denies it before the handler ever runs (`has_grab` fails), silently --
/// no `cancelled`, no drag. This pins that floor, which is what makes the
/// test above the one that exercises the compositor's own gate rather than
/// the dispatch check.
#[test]
fn a_serial_nothing_was_pressed_with_starts_no_drag() {
    let mut harness = Harness::new();
    let (to_test_tx, to_test_rx) = channel::<ToTest>();
    let (to_client_tx, to_client_rx) = channel::<ToClient>();
    let handle = harness.run_client(move |mut conn| {
        let origin = map_window(&mut conn)?;
        to_test_tx.send(ToTest::Mapped).map_err(|e| e.to_string())?;
        wait_for_event(&mut conn, "a button press", |client| {
            client.button_serial.is_some()
        })?;
        let serial = conn.client.button_serial.expect("the press serial");
        to_test_tx
            .send(ToTest::ButtonSeen(serial))
            .map_err(|e| e.to_string())?;
        let ToClient::StartDrag(drag_serial) = to_client_rx.recv().map_err(|e| e.to_string())?
        else {
            return Err("expected StartDrag".into());
        };
        let qh = conn.queue.handle();
        let manager = conn
            .client
            .dd_manager
            .clone()
            .ok_or("no wl_data_device_manager")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        let device = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        source.offer(MIME.to_string());
        device.start_drag(Some(&source), &origin, None, drag_serial);
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        to_test_tx
            .send(ToTest::DragAttempted {
                cancelled: conn.client.cancelled,
            })
            .map_err(|e| e.to_string())?;
        let ToClient::Finish = to_client_rx.recv().map_err(|e| e.to_string())? else {
            return Err("expected Finish".into());
        };
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        to_test_tx
            .send(ToTest::Done {
                cancelled: conn.client.cancelled,
                enters: conn.client.enters,
            })
            .map_err(|e| e.to_string())?;
        Ok("drag script ran".into())
    });

    let ToTest::Mapped = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should map first");
    };
    let (_, moved) = press_on_window(&mut harness);
    let ToTest::ButtonSeen(serial) = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report the press serial");
    };
    // Nowhere near the live grab's serial: the dispatch denies this before
    // any compositor gate runs.
    to_client_tx
        .send(ToClient::StartDrag(serial.wrapping_add(100_000)))
        .expect("the script to take the drag command");
    let ToTest::DragAttempted { cancelled } = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report the attempt");
    };
    assert!(
        !cancelled,
        "a dispatch-denied drag cancels nothing; the refusal is silent"
    );
    harness.state.pointer_move(moved.0, moved.1);
    for _ in 0..10 {
        harness.pump();
    }
    harness.state.pointer_button(PointerButton::Left, false);
    for _ in 0..10 {
        harness.pump();
    }
    assert!(
        !harness
            .state
            .seat
            .get_pointer()
            .is_some_and(|pointer| pointer.is_grabbed()),
        "release should leave no drag behind"
    );
    to_client_tx
        .send(ToClient::Finish)
        .expect("the script to take the finish command");
    let ToTest::Done { cancelled, enters } = next_stage(&to_test_rx, &mut harness) else {
        panic!("the script should report its final state");
    };
    assert_eq!(enters, 0, "no drag means no drag enters, saw {enters}");
    assert!(
        !cancelled,
        "with no drag started, release still cancels nothing"
    );
    assert_eq!(
        handle.join().expect("the client thread").as_deref(),
        Ok("drag script ran"),
    );
}
