//! What one virtual input event costs, end to end: the wayvnc path, from
//! the client's socket through dispatch to delivery.
//!
//! The scene is a real headless compositor with virtual input enabled, one
//! mapped client window holding both foci, and one connected client with
//! live virtual devices (a pointer and a `us`-keymapped keyboard, the way
//! wayvnc holds them). The client floods `EVENTS` requests, flushing in
//! batches so neither side's socket fills, while the compositor dispatches
//! until the flood is answered. What is timed is the whole pipeline --
//! socket reads, the per-request dispatch, the mapping/translation, and
//! delivery to the focused client -- which is the honest answer to "what
//! rate can a remote client inject at", not just the handler's own
//! nanoseconds. Run by hand:
//!
//! ```text
//! cargo test -p scoot --bin scoot virtual_input_cost -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Asserts nothing -- a wall-clock threshold in CI is a flake, not a
//! guarantee. Compare against `pointer_motion_cost` (the shared absolute
//! core this path reaches) and `key_dispatch_cost` (the shared seat path
//! virtual keys translate into): the virtual framing -- one hash lookup,
//! a few multiplies, a keysym round trip -- should read as noise beside
//! the socket and delivery work both paths already pay.
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// Flooded requests per timed run.
const EVENTS: u32 = 20_000;

/// Requests flushed per batch: small enough that neither socket fills mid-run.
const BATCH: u32 = 500;

/// Flushes, yielding to the server when its socket is momentarily full: a
/// flood outruns a compositor dispatching in slices, and a single flush
/// that errors `WouldBlock` is backpressure, not failure. Bounded, so a
/// dead server still fails loudly instead of spinning.
fn flush_paced(
    queue: &mut wayland_client::EventQueue<FloodClient>,
    client: &mut FloodClient,
) -> Result<(), String> {
    use std::io::ErrorKind;
    for _ in 0..100_000 {
        match queue.flush() {
            Ok(()) => return Ok(()),
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == ErrorKind::WouldBlock =>
            {
                queue
                    .dispatch_pending(client)
                    .map_err(|e| format!("flood drain: {e}"))?;
            }
            Err(error) => return Err(format!("flood flush: {error}")),
        }
    }
    Err("the server stopped draining mid-flood".to_owned())
}

type BenchFixture = Harness<(), Flooded>;

/// The flood's own acknowledgement: how many requests went out.
#[derive(Debug)]
struct Flooded {
    events: u32,
}

fn scene() -> BenchFixture {
    let mut fixture = Harness::headless(Appearance::default(), 200);
    crate::compositor::virtual_input::init(&mut fixture.state, true);
    fixture
}

/// Spawns the flooding client: `keys` selects the flood both benches share
/// below through one script shape.
fn spawn_flood(fixture: &mut BenchFixture, keys: bool) {
    if keys {
        fixture.spawn(|stream, steps, acks| flood_client_inner(stream, steps, acks, true));
    } else {
        fixture.spawn(|stream, steps, acks| flood_client_inner(stream, steps, acks, false));
    }
}

#[derive(Default)]
struct FloodClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    wl_output: Option<wayland_client::protocol::wl_output::WlOutput>,
    window_serial: Option<u32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for FloodClient {
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

impl Dispatch<wl_seat::WlSeat, ()> for FloodClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { .. } = event {
            // Both up front: the flood needs listening objects, and the
            // window needs them bound before it maps into focus.
            if client.keyboard.is_none() {
                client.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if client.pointer.is_none() {
                client.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for FloodClient {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for FloodClient {
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

wayland_client::delegate_noop!(FloodClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(FloodClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(FloodClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(FloodClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(FloodClient: ignore wayland_client::protocol::wl_output::WlOutput);
wayland_client::delegate_noop!(FloodClient: ignore wl_keyboard::WlKeyboard);
wayland_client::delegate_noop!(FloodClient: ignore wl_pointer::WlPointer);
wayland_client::delegate_noop!(FloodClient: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(FloodClient: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
wayland_client::delegate_noop!(FloodClient: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(FloodClient: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(FloodClient: ignore wayland_client::protocol::wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(FloodClient: ignore wayland_client::protocol::wl_buffer::WlBuffer);

/// Maps one window (both foci land on it) and builds the virtual devices
/// the benchmark floods through, then waits for the bench's go signal and
/// floods. `keys` selects the flood: motion or typing. Setup and flood are
/// two phases so the timed run holds only the flood, not the mapping.
fn flood_client_inner(
    stream: UnixStream,
    steps: Receiver<()>,
    acks: Sender<Flooded>,
    keys: bool,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut client = FloodClient::default();
    queue
        .roundtrip(&mut client)
        .map_err(|e| format!("setup registry 1: {e}"))?;
    queue
        .roundtrip(&mut client)
        .map_err(|e| format!("setup registry 2: {e}"))?;

    // One mapped window, so both foci exist and delivery has somewhere to go.
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let _toplevel = xdg.get_toplevel(&qh, ());
    surface.commit();
    crate::compositor::test_support::wait_for(
        &mut queue,
        &mut client,
        "a toplevel configure",
        |client| client.window_serial,
    )?;
    let mut file = tempfile::tempfile().map_err(|e| format!("shm tempfile: {e}"))?;
    file.write_all(&[0xff; 64 * 64 * 4])
        .map_err(|e| e.to_string())?;
    use std::os::fd::AsFd;
    let pool = shm.create_pool(file.as_fd(), 64 * 64 * 4, &qh, ());
    let buffer = pool.create_buffer(0, 64, 64, 64 * 4, wl_shm::Format::Argb8888, &qh, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    // Setup done: the bench times only the flood below, not the mapping.
    acks.send(Flooded { events: 0 })
        .map_err(|e| e.to_string())?;
    steps.recv().map_err(|e| e.to_string())?;

    if keys {
        let manager = client
            .keyboard_manager
            .clone()
            .ok_or("no keyboard manager")?;
        let seat = client.seat.clone().ok_or("no wl_seat")?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        // The stock US keymap, compiled in-process like the tests'.
        let context = smithay::input::keyboard::xkb::Context::new(
            smithay::input::keyboard::xkb::CONTEXT_NO_FLAGS,
        );
        let map = smithay::input::keyboard::xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            None,
            smithay::input::keyboard::xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("a keymap");
        let text = map.get_as_string(smithay::input::keyboard::xkb::KEYMAP_FORMAT_TEXT_V1);
        let mut keymap_file = tempfile::tempfile().map_err(|e| format!("keymap tempfile: {e}"))?;
        keymap_file
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        keymap_file.flush().map_err(|e| e.to_string())?;
        keyboard.keymap(1, keymap_file.as_fd(), text.len() as u32);
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        // `KEY_A` in evdev: press and release alternately, like typing.
        for i in 0..EVENTS {
            keyboard.key(0, 30, u32::from(i % 2 == 0));
            if i % BATCH == 0 {
                flush_paced(&mut queue, &mut client)?;
            }
        }
        flush_paced(&mut queue, &mut client)?;
    } else {
        let manager = client.pointer_manager.clone().ok_or("no pointer manager")?;
        let seat = client.seat.clone().ok_or("no wl_seat")?;
        let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        // wayvnc's own shape: normalized INT32_MAX extents, alternating
        // two adjacent positions so focus never moves mid-run.
        for i in 0..EVENTS {
            let x = u32::MAX / 2 + (i % 2);
            pointer.motion_absolute(0, x, u32::MAX / 2, u32::MAX, u32::MAX);
            pointer.frame();
            if i % BATCH == 0 {
                flush_paced(&mut queue, &mut client)?;
            }
        }
        flush_paced(&mut queue, &mut client)?;
    }
    acks.send(Flooded { events: EVENTS })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn report(what: &str, elapsed: Duration, events: u32) {
    let per = elapsed / events;
    println!("virtual input, {what}: {per:?} per event ({events} events in {elapsed:?})");
}

/// One absolute motion plus its frame, end to end, at the device's rate.
#[test]
#[ignore = "prints per-event timings for a human; asserts nothing"]
fn virtual_motion_cost() {
    let mut fixture = scene();
    spawn_flood(&mut fixture, false);
    let _ = fixture.wait_for_ack(0);
    fixture.settle();
    let started = Instant::now();
    fixture.send_step(0, ());
    let ack = fixture.wait_for_ack(0);
    let elapsed = started.elapsed();
    report("motion_absolute + frame", elapsed, ack.events);
}

/// One key press or release, translated and delivered, at typing's rate and
/// far past it.
#[test]
#[ignore = "prints per-event timings for a human; asserts nothing"]
fn virtual_key_cost() {
    let mut fixture = scene();
    spawn_flood(&mut fixture, true);
    let _ = fixture.wait_for_ack(0);
    fixture.settle();
    let started = Instant::now();
    fixture.send_step(0, ());
    let ack = fixture.wait_for_ack(0);
    let elapsed = started.elapsed();
    report("key press/release", elapsed, ack.events);
}
