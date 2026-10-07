//! Tests for the virtual pointer and keyboard (`virtual_input.rs`).
//!
//! These drive *real* `wayland-client` connections -- binding the managers
//! and sending exactly what wayvnc sends -- through a real [`State`] with a
//! real headless backend. What is under test is what the client observes
//! (pointer/keyboard events, protocol errors) and what the compositor did
//! (pointer location, seat focus, survival), not handler internals.
//!
//! The client runs on its own thread while the test pumps the compositor;
//! each test is a linear script of [`Step`]s reporting back [`Ack`]s. See
//! `selection/tests.rs` for the shared shape, and `test_support` for the
//! harness itself.
//!
//! Two cases have no test here, deliberately:
//!
//! - `create_virtual_pointer_with_output` naming an unknown output: outputs
//!   are unforgeable -- the only `wl_output` a client can name is the
//!   compositor's own, which always resolves -- so the union fallback is
//!   defence for an output that went away mid-drag, not a reachable path.
//! - Applying to real hardware: headless has no VT, so the VT switch-away
//!   release is exercised on the dev VM (see the PR description), not here.
//!
//! These need a writable `$XDG_RUNTIME_DIR` for the same reason the other
//! compositor tests do.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::Action;
use scoot_ipc::PointerButton;
use smithay::input::keyboard::{Keysym, xkb};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{Connection, Dispatch, DispatchError, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

use super::super::decorations::Appearance;
use super::super::keybindings::{BindFlags, Bound, Modifiers};
use super::super::test_support::{Harness, wait_for};

/// The framebuffer the headless backend renders into. Absolute motion maps
/// onto this many logical pixels per side.
const CANVAS: i32 = 200;

/// A live compositor with a real headless backend, virtual input enabled,
/// and connected clients scripted a step at a time.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A fixture with the virtual managers advertised, and one connected
    /// client running the test script.
    fn enabled() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        super::init(&mut fixture.state, true);
        fixture.spawn(run_client);
        fixture
    }

    /// A fixture with the managers advertised and `[virtual_input] binds`
    /// on: virtual keys run binds by translated seat keysym.
    fn with_binds() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.state.virtual_input_binds = true;
        super::init(&mut fixture.state, true);
        fixture.spawn(run_client);
        fixture
    }

    /// A fixture with virtual input off: the managers are never advertised.
    fn disabled() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// Everything client 0 has seen since the last call.
    fn take_log(&mut self) -> Vec<Seen> {
        match self.run(Step::TakeLog) {
            Ack::Log(log) => log,
            _ => panic!("the client answered a log request with nothing"),
        }
    }

    /// The globals client 0 was advertised, in registry order.
    fn advertised(&mut self) -> Vec<(String, u32)> {
        match self.run(Step::Globals) {
            Ack::Globals(globals) => globals,
            _ => panic!("the client answered a globals request with nothing"),
        }
    }

    /// The seat pointer's current location, the only unambiguous answer to
    /// "where did absolute motion put it".
    fn pointer_at(&self) -> (f64, f64) {
        let location = self
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .current_location();
        (location.x, location.y)
    }

    /// The surface the seat's keyboard focus is actually on, or `None` --
    /// the only unambiguous answer to "where do keystrokes go".
    fn keyboard_on(&self) -> Option<WlSurface> {
        use smithay::wayland::seat::WaylandFocus;
        self.state
            .seat
            .get_keyboard()
            .expect("a keyboard")
            .current_focus()
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()))
    }
}

/// One client instruction. Multi-word gestures (a scroll sequence, a
/// keymap upload plus a key) are several steps; the client acks each one,
/// so the compositor side can assert between them.
#[derive(Debug)]
enum Step {
    /// Bind everything the script needs and drain the initial burst.
    Bind,
    /// Hand back the globals the registry advertised.
    Globals,
    /// Hand back (and clear) what the client has seen so far.
    TakeLog,
    /// Map one `xdg_toplevel` with a real buffer, ack its configure: the
    /// setup every key/button/scroll test needs for focus to exist.
    MapWindow,
    /// Create a virtual pointer, optionally bound to the (only) output.
    CreatePointer {
        with_output: bool,
    },
    /// Absolute motion over `x_extent` by `y_extent`.
    MotionAbs {
        x: u32,
        y: u32,
        x_extent: u32,
        y_extent: u32,
    },
    /// Relative motion.
    MotionRel {
        dx: f64,
        dy: f64,
    },
    /// A button press or release by Linux code.
    Button {
        code: u32,
        pressed: bool,
    },
    /// One scroll-sequence event each; `Frame` closes the sequence.
    AxisSource {
        source: u32,
    },
    Axis {
        axis: u32,
        value: f64,
    },
    AxisDiscrete {
        axis: u32,
        value: f64,
        discrete: i32,
    },
    AxisStop {
        axis: u32,
    },
    Frame,
    /// Destroy the virtual pointer object (releases what it holds).
    DestroyPointer,
    /// Create a virtual keyboard.
    CreateKeyboard,
    /// Build an xkb keymap for `layout` in-process and upload it.
    UploadKeymap {
        layout: String,
    },
    /// Upload `bytes` as the keymap (for the malformed-upload case).
    UploadRaw {
        bytes: Vec<u8>,
    },
    /// Upload with format 99 (for the unknown-format case).
    UploadBadFormat {
        bytes: Vec<u8>,
    },
    /// One key press or release by evdev code.
    Key {
        code: u32,
        pressed: bool,
    },
    /// One key press or release that must kill the connection with a
    /// protocol error containing `contains`: the poison and its
    /// observation happen in the same step, because the error is only
    /// readable on the round trip before the teardown completes (see
    /// `expect_error`).
    KeyExpectingError {
        code: u32,
        contains: String,
    },
    /// One modifiers notification.
    Modifiers {
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    },
    /// One modifiers notification that must kill the connection with a
    /// protocol error containing `contains` (same shape as
    /// [`Step::KeyExpectingError`]).
    ModifiersExpectingError {
        contains: String,
    },
    /// Destroy the virtual keyboard object (releases what it holds).
    DestroyKeyboard,
    /// Take the session lock and park holding it (locker client only).
    TakeLock,
    /// Map the lock surface with a real buffer, so it takes both foci
    /// (locker client only).
    MapLockSurface,
    /// Hand back (and clear) what the locker client has seen (locker only).
    TakeLockerLog,
    /// Round-trip expecting the connection to die with a protocol error
    /// containing `contains` (for errors forged compositor-side, which no
    /// typed client step can send).
    ExpectError {
        contains: String,
    },
}

/// What the client answers a step with.
#[derive(Debug)]
enum Ack {
    Done,
    Log(Vec<Seen>),
    Globals(Vec<(String, u32)>),
}

/// One thing the client observed, in wire order.
#[derive(Debug, PartialEq)]
enum Seen {
    PointerButton {
        code: u32,
        pressed: bool,
    },
    PointerAxis {
        axis: u32,
        value: f64,
    },
    PointerAxisDiscrete {
        axis: u32,
        discrete: i32,
    },
    PointerAxisStop {
        axis: u32,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    /// A `wl_keyboard.keymap` event arrived (the seat keymap, counted).
    Keymap,
}

/// Everything the test client binds or observes.
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    /// The mapped window's surface. Never read: keeping the client-side
    /// proxy alive is what keeps the server-side surface (and the window)
    /// alive for the rest of the test.
    #[allow(dead_code)]
    output: Option<wl_surface::WlSurface>,
    wl_output: Option<wayland_client::protocol::wl_output::WlOutput>,
    globals: Vec<(String, u32)>,
    seen: Vec<Seen>,
    window_serial: Option<u32>,
    pointer_object: Option<zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1>,
    keyboard_object: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
}

impl TestClient {
    fn bind_managers(&mut self) -> Result<(), String> {
        if self.pointer_manager.is_none() {
            return Err("no zwlr_virtual_pointer_manager_v1 advertised".to_owned());
        }
        if self.keyboard_manager.is_none() {
            return Err("no zwp_virtual_keyboard_manager_v1 advertised".to_owned());
        }
        Ok(())
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
        client.globals.push((interface.clone(), version));
        match interface.as_str() {
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(6), qh, ()));
            }
            "wl_shm" => {
                client.shm = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_seat" => {
                client.seat = Some(registry.bind(name, version.min(7), qh, ()));
            }
            "xdg_wm_base" => {
                client.wm_base = Some(registry.bind(name, version.min(6), qh, ()));
            }
            "wl_output" => {
                client.wl_output = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "zwlr_virtual_pointer_manager_v1" => {
                client.pointer_manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
            "zwp_virtual_keyboard_manager_v1" => {
                client.keyboard_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
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

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { .. } => client.seen.push(Seen::Keymap),
            wl_keyboard::Event::Key { key, state, .. } => client.seen.push(Seen::Key {
                code: key,
                pressed: matches!(
                    state,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                ),
            }),
            _ => {}
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
            wl_pointer::Event::Button { button, state, .. } => {
                client.seen.push(Seen::PointerButton {
                    code: button,
                    pressed: matches!(
                        state,
                        wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed)
                    ),
                });
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                let axis = match axis {
                    wayland_client::WEnum::Value(wl_pointer::Axis::VerticalScroll) => 0,
                    wayland_client::WEnum::Value(wl_pointer::Axis::HorizontalScroll) => 1,
                    _ => 99,
                };
                client.seen.push(Seen::PointerAxis { axis, value });
            }
            wl_pointer::Event::AxisDiscrete { axis, discrete, .. } => {
                let axis = match axis {
                    wayland_client::WEnum::Value(wl_pointer::Axis::VerticalScroll) => 0,
                    wayland_client::WEnum::Value(wl_pointer::Axis::HorizontalScroll) => 1,
                    _ => 99,
                };
                client
                    .seen
                    .push(Seen::PointerAxisDiscrete { axis, discrete });
            }
            wl_pointer::Event::AxisStop { axis, .. } => {
                let axis = match axis {
                    wayland_client::WEnum::Value(wl_pointer::Axis::VerticalScroll) => 0,
                    wayland_client::WEnum::Value(wl_pointer::Axis::HorizontalScroll) => 1,
                    _ => 99,
                };
                client.seen.push(Seen::PointerAxisStop { axis });
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
        client: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            client.window_serial = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wayland_client::protocol::wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wayland_client::protocol::wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wayland_client::protocol::wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(TestClient: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

/// Builds an xkb keymap string for `layout` in-process: the test client
/// uploads real keymaps the way wayvnc does, compiled from rules rather
/// than pasted, so the positions asserted on are what xkb says they are.
fn test_keymap(layout: &str) -> String {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("a compiled test keymap");
    keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
}

/// The evdev code of the key carrying `keysym_name` at any level of
/// `layout`: what the test sends for "the key that types this".
fn evdev_for(layout: &str, keysym_name: &str) -> u32 {
    use smithay::input::keyboard::Keysym;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("a compiled test keymap");
    let keysym = xkb::keysym_from_name(keysym_name, xkb::KEYSYM_NO_FLAGS);
    assert_ne!(keysym, Keysym::NoSymbol, "unknown keysym {keysym_name}");
    for raw in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        let code = smithay::input::keyboard::Keycode::new(raw);
        if (0..keymap.num_levels_for_key(code, 0)).any(|level| {
            keymap
                .key_get_syms_by_level(code, 0, level)
                .contains(&keysym)
        }) {
            return raw - 8;
        }
    }
    panic!("no key for {keysym_name} in the {layout} test keymap");
}

/// The client script: one linear protocol conversation per test.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut client = TestClient {
        compositor: None,
        shm: None,
        seat: None,
        keyboard: None,
        pointer: None,
        wm_base: None,
        pointer_manager: None,
        keyboard_manager: None,
        output: None,
        wl_output: None,
        globals: Vec::new(),
        seen: Vec::new(),
        window_serial: None,
        pointer_object: None,
        keyboard_object: None,
    };
    /// Two client round trips: one is not enough and cannot be made enough,
    /// because a configure is sent when the compositor's layout says so, which
    /// may be a dispatch cycle or two after the request that provoked it.
    fn pump(
        queue: &mut wayland_client::EventQueue<TestClient>,
        client: &mut TestClient,
    ) -> Result<(), String> {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        Ok(())
    }
    while let Ok(step) = steps.recv() {
        match step {
            Step::Bind => {
                pump(&mut queue, &mut client)?;
                if client.keyboard.is_none() {
                    let seat = client.seat.clone().ok_or("no wl_seat")?;
                    client.keyboard = Some(seat.get_keyboard(&qh, ()));
                    client.pointer = Some(seat.get_pointer(&qh, ()));
                    pump(&mut queue, &mut client)?;
                }
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Globals => {
                pump(&mut queue, &mut client)?;
                acks.send(Ack::Globals(client.globals.clone()))
                    .map_err(|e| e.to_string())?;
            }
            Step::TakeLog => {
                pump(&mut queue, &mut client)?;
                let seen = std::mem::take(&mut client.seen);
                acks.send(Ack::Log(seen)).map_err(|e| e.to_string())?;
            }
            Step::MapWindow => {
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let _toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                wait_for(&mut queue, &mut client, "a toplevel configure", |client| {
                    client.window_serial
                })?;
                // A real buffer, so the window maps and takes focus.
                let mut file = tempfile::tempfile().map_err(|e| format!("a shm tempfile: {e}"))?;
                file.write_all(&[0xff; 64 * 64 * 4])
                    .map_err(|e| e.to_string())?;
                use std::os::fd::AsFd;
                let pool = shm.create_pool(file.as_fd(), 64 * 64 * 4, &qh, ());
                let buffer =
                    pool.create_buffer(0, 64, 64, 64 * 4, wl_shm::Format::Argb8888, &qh, ());
                surface.attach(Some(&buffer), 0, 0);
                surface.commit();
                wait_for(&mut queue, &mut client, "keyboard focus", |client| {
                    client
                        .seen
                        .iter()
                        .any(|seen| matches!(seen, Seen::Keymap))
                        .then_some(())
                })?;
                client.output = Some(surface);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::CreatePointer { with_output } => {
                client.bind_managers()?;
                let manager = client.pointer_manager.clone().ok_or("no pointer manager")?;
                let seat = client.seat.clone().ok_or("no wl_seat")?;
                let pointer = if with_output {
                    let output = client.wl_output.clone().ok_or("no wl_output")?;
                    manager.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ())
                } else {
                    manager.create_virtual_pointer(Some(&seat), &qh, ())
                };
                client.pointer_object = Some(pointer);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::MotionAbs {
                x,
                y,
                x_extent,
                y_extent,
            } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                pointer.motion_absolute(0, x, y, x_extent, y_extent);
                pointer.frame();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::MotionRel { dx, dy } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                pointer.motion(0, dx, dy);
                pointer.frame();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Button { code, pressed } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                pointer.button(
                    0,
                    code,
                    if pressed {
                        wl_pointer::ButtonState::Pressed
                    } else {
                        wl_pointer::ButtonState::Released
                    },
                );
                pointer.frame();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::AxisSource { source } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                let source = match source {
                    0 => wl_pointer::AxisSource::Wheel,
                    1 => wl_pointer::AxisSource::Finger,
                    2 => wl_pointer::AxisSource::Continuous,
                    3 => wl_pointer::AxisSource::WheelTilt,
                    _ => return Err(format!("unknown test axis source {source}")),
                };
                pointer.axis_source(source);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Axis { axis, value } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                let axis = match axis {
                    0 => wl_pointer::Axis::VerticalScroll,
                    1 => wl_pointer::Axis::HorizontalScroll,
                    _ => return Err(format!("unknown test axis {axis}")),
                };
                pointer.axis(0, axis, value);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::AxisDiscrete {
                axis,
                value,
                discrete,
            } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                let axis = match axis {
                    0 => wl_pointer::Axis::VerticalScroll,
                    1 => wl_pointer::Axis::HorizontalScroll,
                    _ => return Err(format!("unknown test axis {axis}")),
                };
                pointer.axis_discrete(0, axis, value, discrete);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::AxisStop { axis } => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                let axis = match axis {
                    0 => wl_pointer::Axis::VerticalScroll,
                    1 => wl_pointer::Axis::HorizontalScroll,
                    _ => return Err(format!("unknown test axis {axis}")),
                };
                pointer.axis_stop(0, axis);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Frame => {
                let pointer = client.pointer_object.clone().ok_or("no virtual pointer")?;
                pointer.frame();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::DestroyPointer => {
                let pointer = client.pointer_object.take().ok_or("no virtual pointer")?;
                pointer.destroy();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::CreateKeyboard => {
                client.bind_managers()?;
                let manager = client
                    .keyboard_manager
                    .clone()
                    .ok_or("no keyboard manager")?;
                let seat = client.seat.clone().ok_or("no wl_seat")?;
                let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
                client.keyboard_object = Some(keyboard);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::UploadKeymap { layout } => {
                let file = upload_keymap_step(&mut client, test_keymap(&layout).as_bytes(), 1)?;
                queue.flush().map_err(|e| e.to_string())?;
                drop(file);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::UploadRaw { bytes } => {
                let file = upload_keymap_step(&mut client, &bytes, 1)?;
                queue.flush().map_err(|e| e.to_string())?;
                drop(file);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::UploadBadFormat { bytes } => {
                let file = upload_keymap_step(&mut client, &bytes, 99)?;
                queue.flush().map_err(|e| e.to_string())?;
                drop(file);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Key { code, pressed } => {
                let keyboard = client
                    .keyboard_object
                    .clone()
                    .ok_or("no virtual keyboard")?;
                // The typed client API takes the raw state value, like the
                // wire: 1 presses, anything else releases.
                keyboard.key(0, code, u32::from(pressed));
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::KeyExpectingError { code, contains } => {
                let keyboard = client
                    .keyboard_object
                    .clone()
                    .ok_or("no virtual keyboard")?;
                // No separate flush: the poison rides `expect_error`'s own
                // round trip, so the write that provokes the disconnect and
                // the read that observes it are one step. A flush here plus
                // a later round trip loses the posted error to EPIPE when
                // the server closes between the two.
                keyboard.key(0, code, 1);
                expect_error(&conn, &mut queue, &mut client, &contains)?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
                return Ok(());
            }
            Step::Modifiers {
                depressed,
                latched,
                locked,
                group,
            } => {
                let keyboard = client
                    .keyboard_object
                    .clone()
                    .ok_or("no virtual keyboard")?;
                keyboard.modifiers(depressed, latched, locked, group);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::ModifiersExpectingError { contains } => {
                let keyboard = client
                    .keyboard_object
                    .clone()
                    .ok_or("no virtual keyboard")?;
                // No separate flush, as above: the poison rides the round
                // trip that observes the disconnect.
                keyboard.modifiers(0, 0, 0, 0);
                expect_error(&conn, &mut queue, &mut client, &contains)?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
                return Ok(());
            }
            Step::DestroyKeyboard => {
                let keyboard = client.keyboard_object.take().ok_or("no virtual keyboard")?;
                keyboard.destroy();
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::TakeLock => {
                return Err("TakeLock is for the locker client".to_owned());
            }
            Step::MapLockSurface => {
                return Err("MapLockSurface is for the locker client".to_owned());
            }
            Step::TakeLockerLog => {
                return Err("TakeLockerLog is for the locker client".to_owned());
            }
            Step::ExpectError { contains } => {
                // A protocol error kills the connection: round-trip until
                // the error (or a deadline), then answer with what it said.
                // The error may already be posted (forged compositor-side
                // before this step arrived), so the round trip's own write
                // can hit a closed socket: `expect_error` drains the posted
                // error before deciding.
                expect_error(&conn, &mut queue, &mut client, &contains)?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Round-trips until the connection dies with a protocol error containing
/// `contains`: the poison rides the round trip's own flush, so the error is
/// still on the wire rather than behind a teardown the harness already
/// settled.
///
/// A write after the server closes fails with EPIPE while the error the
/// server posted is still sitting in the socket buffer, and the round trip
/// reports the write failure instead of the posted error. So on an I/O
/// error -- the disconnect without its explanation -- this drains read-side
/// only (`dispatch_pending`, one `prepare_read`/`read`, `dispatch_pending`,
/// the same shape as `client_fds`/`drm_syncobj`'s `sync`) and reads what
/// was actually posted from `conn.protocol_error()` before deciding.
fn expect_error(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    contains: &str,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match queue.roundtrip(client) {
            Err(error) => {
                let message = error.to_string();
                if message.contains(contains) {
                    return Ok(());
                }
                if matches!(
                    error,
                    DispatchError::Backend(wayland_client::backend::WaylandError::Io(_))
                ) {
                    let _ = queue.dispatch_pending(client);
                    if let Some(guard) = conn.prepare_read() {
                        let _ = guard.read();
                    }
                    let _ = queue.dispatch_pending(client);
                    if let Some(posted) = conn.protocol_error() {
                        if posted.message.contains(contains) {
                            return Ok(());
                        }
                        return Err(format!(
                            "expected a protocol error containing {contains:?}, got: {}",
                            posted.message
                        ));
                    }
                }
                return Err(format!(
                    "expected a protocol error containing {contains:?}, got: {message}"
                ));
            }
            Ok(_) => {
                if std::time::Instant::now() >= deadline {
                    return Err(format!(
                        "the compositor never sent a protocol error containing {contains:?}"
                    ));
                }
            }
        }
    }
}
/// Uploads `bytes` as the virtual keyboard's keymap with `format`,
/// handing back the file so the caller can flush first: the request
/// carries a borrowed fd, and the compositor's copy is the duplicate the
/// flush sends, so the file must outlive it.
fn upload_keymap_step(
    client: &mut TestClient,
    bytes: &[u8],
    format: u32,
) -> Result<std::fs::File, String> {
    use std::os::fd::AsFd;
    let keyboard = client
        .keyboard_object
        .clone()
        .ok_or("no virtual keyboard")?;
    let mut file = tempfile::tempfile().map_err(|e| format!("a keymap tempfile: {e}"))?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    let size = bytes.len() as u32;
    keyboard.keymap(format, file.as_fd(), size);
    Ok(file)
}

/// A minimal session-lock client: takes the lock, maps a lock surface
/// that draws (so it takes keyboard and pointer focus like a real lock
/// screen), and records what it is sent. Abandoned, never unlocked, at the
/// end -- an abandoned lock stays locked by design.
struct LockerClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    output: Option<wayland_client::protocol::wl_output::WlOutput>,
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    /// The acked configure: serial plus the size the surface must draw at.
    configured: Option<(u32, u32, u32)>,
    seen: Vec<Seen>,
    _surface: Option<wl_surface::WlSurface>,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    client.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    client.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_seat" => {
                    client.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                "wl_output" => {
                    if client.output.is_none() {
                        client.output = Some(registry.bind(name, version.min(4), qh, ()));
                    }
                }
                "ext_session_lock_manager_v1" => {
                    client.manager = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for LockerClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Keyboard and pointer up front: the lock surface takes both foci
        // when it maps, and this client must already be listening.
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && client.keyboard.is_none() {
                client.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && client.pointer.is_none() {
                client.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key { key, state, .. } = event {
            client.seen.push(Seen::Key {
                code: key,
                pressed: matches!(
                    state,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                ),
            });
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button { button, state, .. } = event {
            client.seen.push(Seen::PointerButton {
                code: button,
                pressed: matches!(
                    state,
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed)
                ),
            });
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        _: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Ack the lock-surface configure on arrival: the surface may only
        // draw after the ack, and the ack is what lets it map and take
        // focus.
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            client.configured = Some((serial, width, height));
        }
    }
}

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(LockerClient: ignore wayland_client::protocol::wl_output::WlOutput);
wayland_client::delegate_noop!(LockerClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(LockerClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(LockerClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(LockerClient: ignore wayland_client::protocol::wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(LockerClient: ignore wayland_client::protocol::wl_buffer::WlBuffer);

fn run_locker(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut client = LockerClient {
        compositor: None,
        shm: None,
        seat: None,
        keyboard: None,
        pointer: None,
        output: None,
        manager: None,
        lock: None,
        configured: None,
        seen: Vec::new(),
        _surface: None,
    };
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    while let Ok(step) = steps.recv() {
        match step {
            Step::TakeLock => {
                let manager = client.manager.clone().ok_or("no session lock manager")?;
                let lock = manager.lock(&qh, ());
                client.lock = Some(lock);
                queue.flush().map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::MapLockSurface => {
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let shm = client.shm.clone().ok_or("no wl_shm")?;
                let output = client.output.clone().ok_or("no wl_output")?;
                let lock = client.lock.clone().ok_or("no lock")?;
                let surface = compositor.create_surface(&qh, ());
                let _lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
                wait_for(&mut queue, &mut client, "a lock configure", |client| {
                    client.configured
                })?;
                // A real buffer at the configured size, so the surface
                // maps and takes both foci.
                let (_, width, height) = client.configured.expect("a lock configure");
                let stride = width as i32 * 4;
                let mut file = tempfile::tempfile().map_err(|e| format!("a lock tempfile: {e}"))?;
                file.write_all(&vec![0xff; (stride * height as i32) as usize])
                    .map_err(|e| e.to_string())?;
                use std::os::fd::AsFd;
                let pool = shm.create_pool(file.as_fd(), stride * height as i32, &qh, ());
                let buffer = pool.create_buffer(
                    0,
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                    &qh,
                    (),
                );
                surface.attach(Some(&buffer), 0, 0);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                client._surface = Some(surface);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::TakeLockerLog => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let seen = std::mem::take(&mut client.seen);
                acks.send(Ack::Log(seen)).map_err(|e| e.to_string())?;
            }
            _ => return Err("the locker client only takes locks".to_owned()),
        }
    }
    Ok(())
}

/// Locks the session through a second client and maps its lock surface:
/// the precondition every lock-gated test shares, so a later failure can
/// only be about the gate, not about the lock. The lock surface takes both
/// foci like a real lock screen, which is what makes these tests strong: a
/// neutered gate would deliver straight to it.
fn lock_session(fixture: &mut Fixture) {
    fixture.spawn(run_locker);
    fixture.run_on(1, Step::TakeLock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the locker client should hold the session locked"
    );
    fixture.run_on(1, Step::MapLockSurface);
}

/// What the locker client has seen since the last call.
fn take_locker_log(fixture: &mut Fixture) -> Vec<Seen> {
    match fixture.run_on(1, Step::TakeLockerLog) {
        Ack::Log(log) => log,
        _ => panic!("the locker answered a log request with nothing"),
    }
}

/// Maps one window and asserts it holds both foci: the setup every
/// delivery test needs, so a later failure can only be about the virtual
/// device, not about focus.
fn map_focused_window(fixture: &mut Fixture) {
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    assert!(
        fixture.state.focus.is_some(),
        "the mapped window should have window focus"
    );
    assert!(
        fixture.keyboard_on().is_some(),
        "the mapped window should have keyboard focus"
    );
    fixture.take_log();
}

/// A point inside the focused window's drawn area: the client draws its
/// 64x64 buffer at the slot's top-left, so just inside that corner always
/// lands on the window whatever the layout says.
fn window_point(fixture: &Fixture) -> (f64, f64) {
    let id = fixture.state.focus.expect("a focused window");
    let arranged = fixture.state.world.arrange();
    let placement = arranged
        .placements
        .iter()
        .find(|placement| placement.id == id)
        .expect("a placement for the focused window");
    (
        f64::from(placement.rect.x) + 5.0,
        f64::from(placement.rect.y) + 5.0,
    )
}

/// Absolute motion to a logical point, over 1000-extents.
fn move_to(fixture: &mut Fixture, x: f64, y: f64) {
    fixture.run(Step::MotionAbs {
        x: (x / f64::from(CANVAS) * 1000.0) as u32,
        y: (y / f64::from(CANVAS) * 1000.0) as u32,
        x_extent: 1000,
        y_extent: 1000,
    });
}

/// The evdev code the seat layout gives `keysym_name`: the independent
/// expectation a translated key must arrive with.
fn seat_evdev(fixture: &Fixture, keysym_name: &str) -> u32 {
    let keyboard = fixture.state.seat.get_keyboard().expect("a keyboard");
    let keysym = xkb::keysym_from_name(keysym_name, xkb::KEYSYM_NO_FLAGS);
    assert_ne!(keysym.raw(), 0, "unknown keysym {keysym_name}");
    let code = keyboard
        .keycode_for_keysym(keysym)
        .unwrap_or_else(|| panic!("no seat key for {keysym_name}"));
    code.raw() - 8
}

#[test]
fn managers_advertised_when_enabled() {
    let mut fixture = Fixture::enabled();
    let globals = fixture.advertised();
    let pointer = globals
        .iter()
        .find(|(name, _)| name == "zwlr_virtual_pointer_manager_v1")
        .expect("a virtual pointer manager");
    assert_eq!(pointer.1, 2, "the pointer manager is version 2");
    let keyboard = globals
        .iter()
        .find(|(name, _)| name == "zwp_virtual_keyboard_manager_v1")
        .expect("a virtual keyboard manager");
    assert_eq!(keyboard.1, 1, "the keyboard manager is version 1");
}

#[test]
fn no_managers_when_disabled() {
    let mut fixture = Fixture::disabled();
    let globals = fixture.advertised();
    assert!(
        globals
            .iter()
            .all(|(name, _)| name != "zwlr_virtual_pointer_manager_v1"),
        "a disabled session must not advertise the pointer manager"
    );
    assert!(
        globals
            .iter()
            .all(|(name, _)| name != "zwp_virtual_keyboard_manager_v1"),
        "a disabled session must not advertise the keyboard manager"
    );
}

#[test]
fn absolute_motion_maps_onto_output() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    // Half of a 1000-extent lands mid-output on the 200px canvas.
    fixture.run(Step::MotionAbs {
        x: 500,
        y: 250,
        x_extent: 1000,
        y_extent: 1000,
    });
    assert_eq!(fixture.pointer_at(), (100.0, 50.0));
    // wayvnc's own shape: full INT32_MAX extents, normalized positions.
    fixture.run(Step::MotionAbs {
        x: u32::MAX / 2,
        y: u32::MAX / 4,
        x_extent: u32::MAX,
        y_extent: u32::MAX,
    });
    let (x, y) = fixture.pointer_at();
    assert!(
        (x - 100.0).abs() < 1.0,
        "wayvnc-shaped x maps mid-output: {x}"
    );
    assert!(
        (y - 50.0).abs() < 1.0,
        "wayvnc-shaped y maps quarter-output: {y}"
    );
}

#[test]
fn absolute_motion_with_output_maps_onto_it() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: true });
    fixture.run(Step::MotionAbs {
        x: 1000,
        y: 1000,
        x_extent: 1000,
        y_extent: 1000,
    });
    // The far corner clamps onto the output's last pixel.
    assert_eq!(fixture.pointer_at(), (199.0, 199.0));
}

#[test]
fn absolute_motion_clamps_and_zero_extent_maps_origin() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    // Past the extent clamps onto the edge rather than leaving the output.
    fixture.run(Step::MotionAbs {
        x: 5000,
        y: 5000,
        x_extent: 1000,
        y_extent: 1000,
    });
    assert_eq!(fixture.pointer_at(), (199.0, 199.0));
    // A zero extent names only position zero: the origin, not a division
    // by zero.
    fixture.run(Step::MotionAbs {
        x: 0,
        y: 0,
        x_extent: 0,
        y_extent: 0,
    });
    assert_eq!(fixture.pointer_at(), (0.0, 0.0));
}

#[test]
fn relative_motion_moves() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    fixture.run(Step::MotionAbs {
        x: 0,
        y: 0,
        x_extent: 1000,
        y_extent: 1000,
    });
    assert_eq!(fixture.pointer_at(), (0.0, 0.0));
    fixture.run(Step::MotionRel { dx: 30.0, dy: 40.0 });
    assert_eq!(fixture.pointer_at(), (30.0, 40.0));
}

#[test]
fn button_press_reaches_focused_client() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    fixture.run(Step::Button {
        code: 0x110,
        pressed: true,
    });
    fixture.run(Step::Button {
        code: 0x110,
        pressed: false,
    });
    assert_eq!(
        fixture.take_log(),
        [
            Seen::PointerButton {
                code: 0x110,
                pressed: true
            },
            Seen::PointerButton {
                code: 0x110,
                pressed: false
            },
        ]
    );
}

#[test]
fn back_button_reaches_focused_client() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    fixture.run(Step::Button {
        code: 0x115,
        pressed: true,
    });
    fixture.run(Step::Button {
        code: 0x115,
        pressed: false,
    });
    assert_eq!(
        fixture.take_log(),
        [
            Seen::PointerButton {
                code: 0x115,
                pressed: true
            },
            Seen::PointerButton {
                code: 0x115,
                pressed: false
            },
        ]
    );
    // And the compositor names the new buttons on its own paths too.
    assert_eq!(super::super::input::button_code(PointerButton::Back), 0x115);
    assert_eq!(
        super::super::input::button_code(PointerButton::Forward),
        0x116
    );
}

#[test]
fn scroll_discrete_reaches_client_as_one_frame() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    // wayvnc's own shape: wheel source, a continuous nudge plus one
    // discrete click down, framed -- one frame out, values summed.
    fixture.run(Step::AxisSource { source: 0 });
    fixture.run(Step::Axis {
        axis: 0,
        value: 5.0,
    });
    fixture.run(Step::AxisDiscrete {
        axis: 0,
        value: 15.0,
        discrete: 1,
    });
    fixture.run(Step::Frame);
    // Smithay emits the discrete event before the continuous one.
    assert_eq!(
        fixture.take_log(),
        [
            Seen::PointerAxisDiscrete {
                axis: 0,
                discrete: 1
            },
            Seen::PointerAxis {
                axis: 0,
                value: 20.0
            },
        ]
    );
    // A stop on its own frame: the stop event, no motion.
    fixture.run(Step::AxisStop { axis: 0 });
    fixture.run(Step::Frame);
    assert_eq!(fixture.take_log(), [Seen::PointerAxisStop { axis: 0 }]);
}

#[test]
fn invalid_axis_disconnects() {
    use smithay::reexports::wayland_server::WEnum;

    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    // Forged compositor-side: no typed client can send an out-of-range
    // axis, so the request goes straight to the handler with one.
    let resource = fixture
        .state
        .virtual_input
        .pointers
        .keys()
        .next()
        .expect("a virtual pointer")
        .clone();
    super::pointer_axis(&mut fixture.state, &resource, WEnum::Unknown(99), 1.0);
    fixture.settle();
    fixture.run(Step::ExpectError {
        contains: "axis is not a wl_pointer.axis value".to_owned(),
    });
}

#[test]
fn invalid_axis_source_disconnects() {
    use smithay::reexports::wayland_server::WEnum;

    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    let resource = fixture
        .state
        .virtual_input
        .pointers
        .keys()
        .next()
        .expect("a virtual pointer")
        .clone();
    super::pointer_axis_source(&mut fixture.state, &resource, WEnum::Unknown(99));
    fixture.settle();
    fixture.run(Step::ExpectError {
        contains: "axis_source is not a wl_pointer.axis_source value".to_owned(),
    });
}

#[test]
fn axis_sources_accepted() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreatePointer { with_output: false });
    // Every source the protocol names is accepted: a source-only frame
    // carries no motion, so nothing is delivered and the client survives.
    // (An out-of-range source cannot be forged through the typed client
    // API; the `invalid_axis_source` branch shares its shape with the
    // `invalid_axis` one pinned by `invalid_axis_disconnects`.)
    for source in 0..4 {
        fixture.run(Step::AxisSource { source });
        fixture.run(Step::Frame);
    }
}

#[test]
fn destroyed_pointer_releases_held_button() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    fixture.run(Step::Button {
        code: 0x110,
        pressed: true,
    });
    fixture.take_log();
    // Destroy with the button still held: the release must still arrive.
    fixture.run(Step::DestroyPointer);
    assert_eq!(
        fixture.take_log(),
        [Seen::PointerButton {
            code: 0x110,
            pressed: false
        }]
    );
}

#[test]
fn locked_session_drops_pointer_input() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    lock_session(&mut fixture);
    // Positive control: a physical click reaches the lock surface, so the
    // emptiness asserted below is the gate, not missing focus.
    fixture.state.pointer_move(100.0, 100.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    assert_eq!(
        take_locker_log(&mut fixture),
        [
            Seen::PointerButton {
                code: 0x110,
                pressed: true
            },
            Seen::PointerButton {
                code: 0x110,
                pressed: false
            },
        ]
    );
    // Motion, buttons and scroll while locked: nothing moves, and nothing
    // reaches either the window behind the lock or the lock surface.
    fixture.run(Step::MotionAbs {
        x: 200,
        y: 200,
        x_extent: 1000,
        y_extent: 1000,
    });
    fixture.run(Step::Button {
        code: 0x110,
        pressed: true,
    });
    fixture.run(Step::AxisDiscrete {
        axis: 0,
        value: 15.0,
        discrete: 1,
    });
    fixture.run(Step::Frame);
    assert_eq!(fixture.pointer_at(), (100.0, 100.0));
    assert_eq!(fixture.take_log(), []);
    assert_eq!(take_locker_log(&mut fixture), []);
}

#[test]
fn key_before_keymap_is_no_keymap() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::KeyExpectingError {
        code: 30,
        contains: "before any keymap".to_owned(),
    });
}

#[test]
fn modifiers_before_keymap_is_no_keymap() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::ModifiersExpectingError {
        contains: "before any keymap".to_owned(),
    });
}

#[test]
fn us_key_reaches_focused_client() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    let code = evdev_for("us", "a");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Key {
        code,
        pressed: false,
    });
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: false
            },
        ]
    );
}

#[test]
fn german_z_types_z() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    // The key in the German `z` position (where US has `y`): the client
    // must hear the seat's `z`, not the position's US reading.
    let code = evdev_for("de", "z");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Key {
        code,
        pressed: false,
    });
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "z"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "z"),
                pressed: false
            },
        ]
    );
}

#[test]
fn untranslatable_key_is_dropped() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.take_log();
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    fixture.take_log();
    // `ß` lives in the German map but in no US-seat position: dropped,
    // not mistyped. (Absence is deterministic here, not a timeout: the
    // ack only arrives after the compositor dispatched the press and the
    // log drains everything the client was sent.)
    let code = evdev_for("de", "ssharp");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Key {
        code,
        pressed: false,
    });
    assert_eq!(fixture.take_log(), []);
}

#[test]
fn malformed_keymap_keeps_working_keyboard() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    // Garbage over a working keymap: ignored, and the old map still
    // translates.
    fixture.run(Step::UploadRaw {
        bytes: b"this is not an xkb keymap".to_vec(),
    });
    let code = evdev_for("us", "a");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Key {
        code,
        pressed: false,
    });
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: false
            },
        ]
    );
}

#[test]
fn unknown_keymap_format_keeps_old_map() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::CreateKeyboard);
    // Format 99 over no keymap: ignored (not a protocol error), so the
    // key that follows still answers `no_keymap` for the missing map.
    fixture.run(Step::UploadBadFormat {
        bytes: test_keymap("us").into_bytes(),
    });
    fixture.run(Step::KeyExpectingError {
        code: 30,
        contains: "before any keymap".to_owned(),
    });
}

#[test]
fn destroyed_keyboard_releases_held_key() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    let code = evdev_for("us", "a");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.take_log();
    // Destroy with the key still held: the release must still arrive, on
    // the seat code the press went out with.
    fixture.run(Step::DestroyKeyboard);
    assert_eq!(
        fixture.take_log(),
        [Seen::Key {
            code: seat_evdev(&fixture, "a"),
            pressed: false
        }]
    );
}

#[test]
fn locked_session_drops_keyboard_input() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    lock_session(&mut fixture);
    // Positive control: physical typing reaches the lock surface, so the
    // emptiness asserted below is the gate, not missing focus.
    fixture.state.type_text("a").expect("typed text");
    fixture.settle();
    assert_eq!(
        take_locker_log(&mut fixture),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "a"),
                pressed: false
            },
        ]
    );
    // Keys and modifiers while locked: nothing reaches either the window
    // behind the lock or the lock surface. The keymap upload is harmless
    // state and still accepted, so typing works again after unlock
    // without re-uploading.
    let code = evdev_for("us", "a");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Modifiers {
        depressed: 1,
        latched: 0,
        locked: 0,
        group: 0,
    });
    assert_eq!(fixture.take_log(), []);
    assert_eq!(take_locker_log(&mut fixture), []);
}

#[test]
fn lock_releases_held_virtual_key() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    let code = evdev_for("us", "a");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.take_log();
    // Lock with the key still held: the release must arrive on the window
    // (which still holds focus at that point, before the refresh) and
    // nothing may reach the lock surface after it maps.
    lock_session(&mut fixture);
    assert_eq!(
        fixture.take_log(),
        [Seen::Key {
            code: seat_evdev(&fixture, "a"),
            pressed: false
        }]
    );
    assert_eq!(take_locker_log(&mut fixture), []);
}

#[test]
fn lock_releases_held_button() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    fixture.run(Step::CreatePointer { with_output: false });
    let (x, y) = window_point(&fixture);
    move_to(&mut fixture, x, y);
    fixture.run(Step::Button {
        code: 0x110,
        pressed: true,
    });
    fixture.take_log();
    // Lock with the button still held: the release must arrive on the
    // window (which still holds focus at that point, before the refresh)
    // and nothing may reach the lock surface after it maps.
    lock_session(&mut fixture);
    assert_eq!(
        fixture.take_log(),
        [Seen::PointerButton {
            code: 0x110,
            pressed: false
        }]
    );
    assert_eq!(take_locker_log(&mut fixture), []);
}

#[test]
fn seat_keymap_survives_remote_typing() {
    let mut fixture = Fixture::enabled();
    map_focused_window(&mut fixture);
    // One seat keymap at keyboard creation; the German upload and the
    // keys after it must never send another.
    fixture.take_log();
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    let code = evdev_for("de", "z");
    fixture.run(Step::Key {
        code,
        pressed: true,
    });
    fixture.run(Step::Key {
        code,
        pressed: false,
    });
    assert!(
        !fixture.take_log().contains(&Seen::Keymap),
        "remote typing must not re-send the seat keymap"
    );
}

#[test]
fn virtual_keys_do_not_run_binds() {
    let mut fixture = Fixture::enabled();
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    // Two windows: the second has focus. Super+h focuses the column to
    // the left when it runs as a bind; as a forwarded keystroke it moves
    // nothing.
    let focused = fixture.state.focus;
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    for (name, pressed) in [
        ("Super_L", true),
        ("h", true),
        ("h", false),
        ("Super_L", false),
    ] {
        let code = evdev_for("us", name);
        fixture.run(Step::Key { code, pressed });
    }
    assert_eq!(
        fixture.state.focus, focused,
        "a virtual Super+h must not run the focus bind"
    );
}

/// Sends one virtual chord: each `(name, pressed)` in order, translated
/// through `layout`'s positions the way wayvnc sends them.
fn press_chord(fixture: &mut Fixture, layout: &str, chord: &[(&str, bool)]) {
    for (name, pressed) in chord {
        let code = evdev_for(layout, name);
        fixture.run(Step::Key {
            code,
            pressed: *pressed,
        });
    }
}

/// Binds `combo` (modifiers + unshifted keysym) to `action` in the live
/// table: what a config file's `[binds]` entry becomes after load.
fn bind(fixture: &mut Fixture, mods: Modifiers, keysym: Keysym, action: Action, flags: BindFlags) {
    fixture
        .state
        .keybindings
        .insert(mods, keysym, Bound::Action(action), flags);
}

/// Whether the focused window is fullscreen right now: the observable half
/// of a `ToggleFullscreen` bind firing, no client cooperation needed (a
/// `CloseFocused` bind only *asks* -- `send_close` -- and the test client
/// never answers, so closes are invisible here).
fn is_fullscreen(fixture: &Fixture) -> bool {
    let id = fixture.state.focus.expect("a focused window");
    fixture
        .state
        .world
        .arrange()
        .placements
        .iter()
        .find(|placement| placement.id == id)
        .expect("a placement for the focused window")
        .fullscreen
}

const ALT: Modifiers = Modifiers {
    super_: false,
    shift: false,
    ctrl: false,
    alt: true,
};

const SUPER: Modifiers = Modifiers {
    super_: true,
    shift: false,
    ctrl: false,
    alt: false,
};

#[test]
fn virtual_bind_fires_when_binds_on() {
    // The mirror of `virtual_keys_do_not_run_binds`: the same Super+h
    // chord, but with the flag on -- focus moves left, the `h` press and
    // its release are intercepted (the client never sees them), and the
    // bare Super press and release still forward.
    let mut fixture = Fixture::with_binds();
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    let focused = fixture.state.focus;
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "us",
        &[
            ("Super_L", true),
            ("h", true),
            ("h", false),
            ("Super_L", false),
        ],
    );
    assert_ne!(
        fixture.state.focus, focused,
        "a virtual Super+h must run the focus bind with binds on"
    );
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: false
            },
        ],
        "the intercepted h press+release must not reach the client"
    );
    assert!(
        fixture.state.virtual_suppressed.is_empty(),
        "the intercepted release must clear the suppression entry"
    );
}

#[test]
fn virtual_bind_off_forwards_the_chord_whole() {
    // Default-off is inert as well as safe: the same chord as above is
    // forwarded whole -- focus never moves and the client hears every key.
    let mut fixture = Fixture::enabled();
    assert!(
        !fixture.state.virtual_input_binds,
        "the harness default must match the config default (off)"
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    let focused = fixture.state.focus;
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "us",
        &[
            ("Super_L", true),
            ("h", true),
            ("h", false),
            ("Super_L", false),
        ],
    );
    assert_eq!(
        fixture.state.focus, focused,
        "a virtual Super+h must not move focus with binds off"
    );
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "h"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "h"),
                pressed: false
            },
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: false
            },
        ]
    );
    assert!(
        fixture.state.virtual_suppressed.is_empty(),
        "nothing intercepted means nothing suppressed"
    );
}

#[test]
fn virtual_alt_return_fires_through_a_mismatched_remote_keymap() {
    // The brief's chord (`Alt+Return`) against a German remote on a US
    // seat: bind matching runs on the translated seat keysym, so `Return`
    // -- the same keysym at every position carrying it -- still fires even
    // though the positions disagree. The held Alt is what proves the
    // seat-side modifier state: the bind sees Alt held because the
    // translated Alt press updated it, not because the remote mask leaked.
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        ALT,
        Keysym::Return,
        Action::ToggleFullscreen,
        BindFlags::default(),
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "de",
        &[
            ("Alt_L", true),
            ("Return", true),
            ("Return", false),
            ("Alt_L", false),
        ],
    );
    assert!(
        is_fullscreen(&fixture),
        "a virtual Alt+Return must run the fullscreen bind with binds on"
    );
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "Alt_L"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "Alt_L"),
                pressed: false
            },
        ],
        "the intercepted Return must not reach the client"
    );
}

#[test]
fn virtual_german_z_matches_the_seat_z_bind() {
    // `z` sits where US has `y` on a German remote: translation by keysym
    // lands on the seat's `z` key, so a `Super+z` bind fires -- and the
    // position's US reading (`y`) must not.
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        SUPER,
        Keysym::z,
        Action::ToggleFullscreen,
        BindFlags::default(),
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "de",
        &[
            ("Super_L", true),
            ("z", true),
            ("z", false),
            ("Super_L", false),
        ],
    );
    assert!(
        is_fullscreen(&fixture),
        "a German-remote Super+z must run the seat Super+z bind"
    );
}

#[test]
fn virtual_german_y_position_does_not_fire_the_z_bind() {
    // The control for the test above: the German `y` position (US `z`) --
    // keysym `y`, which no bind names -- forwards whole and closes
    // nothing.
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        SUPER,
        Keysym::z,
        Action::ToggleFullscreen,
        BindFlags::default(),
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "de".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "de",
        &[
            ("Super_L", true),
            ("y", true),
            ("y", false),
            ("Super_L", false),
        ],
    );
    assert!(
        fixture.state.focus.is_some(),
        "a German-remote Super+y must not run the Super+z bind"
    );
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "y"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "y"),
                pressed: false
            },
            Seen::Key {
                code: seat_evdev(&fixture, "Super_L"),
                pressed: false
            },
        ]
    );
}

#[test]
fn locked_virtual_bind_fires_neither() {
    // The lock gate stays absolute with binds on: a bound chord while
    // locked neither runs the bind (the window survives) nor reaches any
    // client (neither the window behind the lock nor the lock surface) --
    // not even an `allow_when_locked` spawn.
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        ALT,
        Keysym::Return,
        Action::ToggleFullscreen,
        BindFlags::default(),
    );
    bind(
        &mut fixture,
        ALT,
        Keysym::m,
        Action::ToggleMaximize,
        BindFlags {
            repeat: false,
            allow_when_locked: true,
        },
    );
    map_focused_window(&mut fixture);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    lock_session(&mut fixture);
    press_chord(
        &mut fixture,
        "us",
        &[
            ("Alt_L", true),
            ("Return", true),
            ("Return", false),
            ("Alt_L", false),
        ],
    );
    press_chord(
        &mut fixture,
        "us",
        &[("Alt_L", true), ("m", true), ("m", false), ("Alt_L", false)],
    );
    assert!(
        !fixture
            .state
            .world
            .arrange()
            .placements
            .iter()
            .any(|placement| placement.fullscreen || placement.maximized),
        "no virtual bind may run while locked -- not even an allow_when_locked one"
    );
    assert_eq!(
        fixture.take_log(),
        [],
        "locked virtual keys must not reach the window"
    );
    assert_eq!(
        take_locker_log(&mut fixture),
        [],
        "locked virtual keys must not reach the lock surface either"
    );
}

#[test]
fn destroyed_virtual_keyboard_swallows_the_intercepted_release() {
    // Press Super (forwarded) and `h` (intercepted: focus moves), then
    // destroy with both still held. The synthesized releases must route
    // like the live ones: Super's forwards to the still-focused window,
    // `h`'s is swallowed (the client never saw its press), and no
    // suppression entry strands.
    let mut fixture = Fixture::with_binds();
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    let focused = fixture.state.focus;
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(&mut fixture, "us", &[("Super_L", true), ("h", true)]);
    assert_ne!(
        fixture.state.focus, focused,
        "the bind must have fired before the destroy"
    );
    fixture.take_log();
    fixture.run(Step::DestroyKeyboard);
    assert_eq!(
        fixture.take_log(),
        [Seen::Key {
            code: seat_evdev(&fixture, "Super_L"),
            pressed: false
        }],
        "only the forwarded Super release may arrive; the intercepted h release is swallowed"
    );
    assert!(
        fixture.state.virtual_suppressed.is_empty(),
        "the destroy sweep must not strand suppression entries"
    );
}

#[test]
fn lock_sweep_swallows_the_intercepted_release() {
    // The same routing through the lock path: hold Super+h across the
    // lock, and the window behind it must hear Super's release but never
    // `h`'s -- and the lock surface must hear nothing at all.
    let mut fixture = Fixture::with_binds();
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(&mut fixture, "us", &[("Super_L", true), ("h", true)]);
    fixture.take_log();
    lock_session(&mut fixture);
    assert_eq!(
        fixture.take_log(),
        [Seen::Key {
            code: seat_evdev(&fixture, "Super_L"),
            pressed: false
        }],
        "the lock sweep releases what the device held, swallowing the intercepted key"
    );
    assert_eq!(take_locker_log(&mut fixture), []);
    assert!(
        fixture.state.virtual_suppressed.is_empty(),
        "the lock sweep must not strand suppression entries"
    );
}

#[test]
fn a_held_virtual_bind_fires_exactly_once() {
    // Two presses, one release pair: the second press is absorbed (the
    // source already holds the key) before the filter, so a toggle bind
    // ends on -- fired once, never twice, and never forwarded as a
    // duplicate. The log proves interception (no Return events at all);
    // the toggle proves the count (twice would flip it back off).
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        ALT,
        Keysym::Return,
        Action::ToggleFullscreen,
        BindFlags::default(),
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(
        &mut fixture,
        "us",
        &[
            ("Alt_L", true),
            ("Return", true),
            ("Return", true),
            ("Return", false),
            ("Return", false),
            ("Alt_L", false),
        ],
    );
    assert!(
        is_fullscreen(&fixture),
        "the held bind must fire exactly once"
    );
    assert_eq!(
        fixture.take_log(),
        [
            Seen::Key {
                code: seat_evdev(&fixture, "Alt_L"),
                pressed: true
            },
            Seen::Key {
                code: seat_evdev(&fixture, "Alt_L"),
                pressed: false
            },
        ],
        "neither Return press nor either release may reach the client"
    );
}

#[test]
fn virtual_bind_with_repeat_flag_fires_once_and_arms_nothing() {
    // A flagged bind re-fires while a physical key is held; a virtual one
    // must not: the hold has no repeat lifecycle the compositor owns, so
    // the press fires once and arms no timer -- there is nothing whose
    // release could strand a re-fire after the device is gone.
    let mut fixture = Fixture::with_binds();
    bind(
        &mut fixture,
        ALT,
        Keysym::Return,
        Action::ToggleFullscreen,
        BindFlags {
            repeat: true,
            allow_when_locked: false,
        },
    );
    fixture.run(Step::Bind);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    fixture.take_log();
    press_chord(&mut fixture, "us", &[("Alt_L", true), ("Return", true)]);
    assert!(
        is_fullscreen(&fixture),
        "the flagged virtual bind fires its once"
    );
    assert!(
        fixture.state.bind_repeat.is_none(),
        "a virtual bind must never arm the repeat timer"
    );
    press_chord(&mut fixture, "us", &[("Return", false), ("Alt_L", false)]);
    assert!(
        is_fullscreen(&fixture),
        "nothing re-fires after the release either"
    );
}
