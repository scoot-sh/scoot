//! Tests for the cursor module.
//!
//! The pure arithmetic ([`generate_bitmap`], [`element_location`]) is unit
//! tested directly. Everything about a *client-supplied* cursor surface
//! needs a real `wl_surface` with a real committed buffer, so those tests
//! drive an actual `wayland-client` connection through an actual [`State`]
//! -- the same approach `dispatch/tests.rs` established -- and then render
//! the result with a real [`PixmanRenderer`] and read the pixels back. That
//! is what distinguishes "drew the client's image" from "drew the fallback
//! triangle"; asserting on the element enum's variant alone would pass even
//! if the element were positioned or imported wrongly.
//!
//! Deliberately *not* covered here, and verified on real `--tty` hardware
//! instead: the frame callbacks `headless.rs::render` sends to a cursor
//! surface, which only run when `State::tty` is `Some` and there is no way
//! to construct a `Tty` without a real DRM device and seat.
//!
//! These need a writable `$XDG_RUNTIME_DIR`, since [`State::new`] binds a
//! real listening socket -- same requirement, and same reasoning, as
//! `dispatch/tests.rs`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::backend::input::InputTime;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::input::pointer::MotionEvent;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::reexports::wayland_server::{Client, Display};
use smithay::utils::{Rectangle, SERIAL_COUNTER};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

use super::*;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

// -------------------------------------------------------------------------
// Pure arithmetic
// -------------------------------------------------------------------------

#[test]
fn bitmap_hotspot_pixel_is_opaque_black() {
    let pixels = generate_bitmap();
    assert_eq!(&pixels[0..4], &[0, 0, 0, 255]);
}

#[test]
fn bitmap_is_fully_transparent_above_the_diagonal() {
    let pixels = generate_bitmap();
    // (SIZE - 1, 0): far right of the top row, well outside the triangle.
    let idx = ((SIZE - 1) * 4) as usize;
    assert_eq!(&pixels[idx..idx + 4], &[0, 0, 0, 0]);
}

#[test]
fn bitmap_interior_is_opaque_white() {
    let pixels = generate_bitmap();
    // (1, 3): strictly inside the triangle (0 < x < y).
    let idx = ((3 * SIZE + 1) * 4) as usize;
    assert_eq!(&pixels[idx..idx + 4], &[255, 255, 255, 255]);
}

#[test]
fn a_zero_hotspot_leaves_the_pointer_location_unchanged() {
    let pointer = Point::<f64, Logical>::from((12.5, 30.0));
    let hotspot = Point::<i32, Logical>::from((0, 0));
    let location = element_location(pointer, hotspot);
    assert_eq!((location.x, location.y), (12.5, 30.0));
}

#[test]
fn a_nonzero_hotspot_offsets_the_pointer_location() {
    let pointer = Point::<f64, Logical>::from((100.0, 100.0));
    let hotspot = Point::<i32, Logical>::from((4, 6));
    let location = element_location(pointer, hotspot);
    assert_eq!((location.x, location.y), (96.0, 94.0));
}

// -------------------------------------------------------------------------
// A live compositor, a live client, a live renderer
// -------------------------------------------------------------------------

/// Where the pointer sits in every live test below, well inside the
/// framebuffer so a cursor of any size used here fits around it.
const POINTER: (f64, f64) = (50.0, 50.0);
/// Side of the square framebuffer the live tests render into.
const CANVAS: i32 = 100;
/// The clear color those renders start from, and the BGRA bytes it lands as.
/// Deliberately not black: the fallback triangle's own hotspot pixel is
/// opaque black, so a black canvas could not tell "drew the fallback" from
/// "drew nothing".
const CLEAR: [f32; 4] = [0.0, 0.5019608, 1.0, 1.0];
const CLEAR_BGRA: [u8; 4] = [255, 128, 0, 255];
/// The opaque BGRA color a test client commits to its cursor surface.
const CLIENT_BGRA: [u8; 4] = [0x20, 0x40, 0xE0, 0xFF];
/// ...and to a subsurface of it.
const CHILD_BGRA: [u8; 4] = [0xE0, 0x20, 0x40, 0xFF];

/// One instruction for the client thread. The client half of these tests has
/// to interleave with the compositor half -- a `set_cursor` is only honored
/// after the pointer entered one of this client's surfaces, which only
/// happens once the compositor dispatches -- so the two run as real threads
/// and the test script is shipped over a channel one step at a time.
enum Step {
    /// Create the cursor surface and commit a `size`x`size` buffer of
    /// `color` to it.
    CommitCursorBuffer { size: i32, color: [u8; 4] },
    /// Create the cursor surface and commit *nothing* to it.
    CreateBareCursorSurface,
    /// `wl_pointer.set_cursor` with the cursor surface and this hotspot.
    SetCursor { hotspot: (i32, i32) },
    /// `wl_pointer.set_cursor(NULL)`, i.e. hide the cursor.
    HideCursor,
    /// Destroy the cursor surface without setting any replacement.
    DestroyCursorSurface,
    /// Give the cursor surface a child subsurface with its own buffer.
    AddSubsurface {
        size: i32,
        color: [u8; 4],
        offset: (i32, i32),
    },
}

/// The client end of the one test connection.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    /// The serial of the most recent `wl_pointer.enter`. Smithay refuses a
    /// `set_cursor` whose serial doesn't match it.
    enter_serial: Option<u32>,
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
            "wl_subcompositor" => {
                client.subcompositor = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
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
        if let wl_pointer::Event::Enter { serial, .. } = event {
            client.enter_serial = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_seat::WlSeat);

/// A `size`x`size` `wl_buffer` filled with `color`, as a real shm pool over
/// a real memfd -- the same path any toolkit takes.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    size: i32,
    color: [u8; 4],
) -> wl_buffer::WlBuffer {
    let stride = size * 4;
    let len = (stride * size) as usize;
    let fd = rustix::fs::memfd_create("flexwm-cursor-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, size, size, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Runs the client half: binds the globals, makes a surface for the pointer
/// to enter, reports both surfaces' protocol ids back, then executes
/// whatever steps the test sends, acknowledging each one.
fn run_client(
    stream: UnixStream,
    ids: Sender<u32>,
    steps: Receiver<Step>,
    acks: Sender<()>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let pointer = seat.get_pointer(&qh, ());
    // A plain surface for the pointer to be over. It needs no buffer: the
    // compositor half hands it to `PointerHandle::motion` directly rather
    // than going through the layout, so nothing here depends on it mapping.
    let focus = compositor.create_surface(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    ids.send(focus.id().protocol_id())
        .map_err(|e| e.to_string())?;

    let mut cursor: Option<wl_surface::WlSurface> = None;
    while let Ok(step) = steps.recv() {
        // Drain anything the compositor sent since the last step -- in
        // particular the `wl_pointer.enter` whose serial `SetCursor` needs.
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::CommitCursorBuffer { size, color } => {
                let surface = compositor.create_surface(&qh, ());
                let buffer = solid_buffer(&shm, &qh, size, color);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, size, size);
                surface.commit();
                cursor = Some(surface);
            }
            Step::CreateBareCursorSurface => {
                cursor = Some(compositor.create_surface(&qh, ()));
            }
            Step::SetCursor { hotspot } => {
                let serial = client.enter_serial.ok_or("the pointer never entered")?;
                pointer.set_cursor(serial, cursor.as_ref(), hotspot.0, hotspot.1);
            }
            Step::HideCursor => {
                let serial = client.enter_serial.ok_or("the pointer never entered")?;
                pointer.set_cursor(serial, None, 0, 0);
            }
            Step::DestroyCursorSurface => {
                let surface = cursor.take().ok_or("no cursor surface to destroy")?;
                surface.destroy();
            }
            Step::AddSubsurface {
                size,
                color,
                offset,
            } => {
                let subcompositor = client.subcompositor.clone().ok_or("no wl_subcompositor")?;
                let parent = cursor.as_ref().ok_or("no cursor surface")?;
                let child = compositor.create_surface(&qh, ());
                let sub = subcompositor.get_subsurface(&child, parent, &qh, ());
                sub.set_position(offset.0, offset.1);
                sub.set_desync();
                let buffer = solid_buffer(&shm, &qh, size, color);
                child.attach(Some(&buffer), 0, 0);
                child.damage(0, 0, size, size);
                child.commit();
                parent.commit();
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with one connected client, scripted a step at a time.
///
/// The client keeps running (and its surfaces keep existing) until this is
/// dropped, so every assertion a test makes must happen while its `Fixture`
/// is still alive -- dropping it disconnects the client, which destroys its
/// surfaces and resets the very cursor state under test.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<()>,
    client: Option<JoinHandle<Result<(), String>>>,
}

impl Fixture {
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
        )
        .expect("a compositor state with a wayland socket");

        // A socket pair rather than the listening socket: identical
        // per-client dispatch, no dependence on which socket name it got.
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        let client: Client = state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (id_tx, id_rx) = channel();
        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, id_tx, step_rx, ack_tx));

        let mut fixture = Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
        };

        // Give the pointer a focus inside this client, which is what makes
        // its later `set_cursor` calls legal (Smithay checks the serial
        // against the last `wl_pointer.enter` it sent).
        let focus_id = fixture.wait_for(&id_rx, "the client's focus surface id");
        let focus: ServerSurface = client
            .object_from_protocol_id(&fixture.state.display_handle, focus_id)
            .expect("the client's focus surface");
        let pointer = fixture.state.seat.get_pointer().expect("a pointer");
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            &mut fixture.state,
            Some((focus, (0.0, 0.0).into())),
            &MotionEvent {
                location: POINTER.into(),
                serial,
                time: InputTime::from_millis(0),
            },
        );
        pointer.frame(&mut fixture.state);
        let _ = fixture.state.display_handle.flush_clients();
        fixture
    }

    /// Dispatches the compositor until `channel` produces a value. The
    /// deadline only exists so a regression fails in seconds instead of
    /// hanging the suite forever.
    fn wait_for<T>(&mut self, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(value) = channel.try_recv() {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the client thread stopped or the compositor did"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// Runs one client step to completion.
    fn run(&mut self, step: Step) {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
    }

    /// Renders this frame's cursor elements into a [`CANVAS`]-square
    /// framebuffer and hands back the raw BGRA pixels, exactly the layout
    /// `headless.rs` renders into.
    fn render(&self) -> Canvas {
        let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
        let mut image = renderer
            .create_buffer(Fourcc::Argb8888, (CANVAS, CANVAS).into())
            .expect("an offscreen buffer");
        let elements = self.state.cursor.element(&mut renderer, POINTER.into());
        let count = elements.len();
        let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
        let mut damage = OutputDamageTracker::new((CANVAS, CANVAS), 1.0, Transform::Normal);
        damage
            .render_output(&mut renderer, &mut framebuffer, 0, &elements, CLEAR)
            .expect("a rendered frame");
        let region = Rectangle::new((0, 0).into(), (CANVAS, CANVAS).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
            .expect("a framebuffer readback");
        let pixels = renderer
            .map_texture(&mapping)
            .expect("mapped pixels")
            .to_vec();
        Canvas { pixels, count }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Closing the step channel is what ends `run_client`'s loop.
        self.steps = None;
        if let Some(handle) = self.client.take() {
            // The client may still be mid-roundtrip, which only completes
            // while the compositor dispatches -- so keep dispatching until
            // it's actually done rather than blocking on the join.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !handle.is_finished() && Instant::now() < deadline {
                let _ = self
                    .event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state);
            }
            if let Ok(Err(error)) = handle.join() {
                // Not an assert: a panicking `Drop` while another assertion
                // is already unwinding aborts the process and hides it.
                eprintln!("the test client failed: {error}");
            }
        }
    }
}

/// One rendered frame: its raw BGRA pixels and how many cursor elements went
/// into it.
struct Canvas {
    pixels: Vec<u8>,
    count: usize,
}

impl Canvas {
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let idx = ((y * CANVAS + x) * 4) as usize;
        self.pixels[idx..idx + 4].try_into().expect("four bytes")
    }

    /// Asserts no cursor element existed at all *and* nothing was drawn.
    fn assert_blank(&self) {
        assert_eq!(self.count, 0, "expected no cursor elements");
        self.assert_nothing_drawn();
    }

    /// Asserts every pixel is the clear color. Unlike [`Canvas::assert_blank`]
    /// this tolerates an element existing -- one positioned entirely off the
    /// canvas draws nothing while still being in the list.
    fn assert_nothing_drawn(&self) {
        for chunk in self.pixels.chunks_exact(4) {
            assert_eq!(chunk, CLEAR_BGRA, "something was drawn on a blank frame");
        }
    }
}

#[test]
fn a_client_cursor_surface_is_drawn_instead_of_the_fallback() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });

    let canvas = fixture.render();
    assert_eq!(canvas.count, 1, "one element for a cursor with no children");
    // The hotspot must land exactly on the pointer: the client asked for
    // (4, 6) within a 24x24 image, so the image occupies x 46..70, y 44..68.
    assert_eq!(canvas.at(50, 50), CLIENT_BGRA, "the hotspot pixel");
    assert_eq!(canvas.at(46, 44), CLIENT_BGRA, "the image's top-left");
    assert_eq!(canvas.at(69, 67), CLIENT_BGRA, "the image's bottom-right");
    assert_eq!(canvas.at(45, 44), CLEAR_BGRA, "one pixel left of the image");
    assert_eq!(
        canvas.at(70, 67),
        CLEAR_BGRA,
        "one pixel right of the image"
    );
    assert_eq!(canvas.at(46, 43), CLEAR_BGRA, "one pixel above the image");
    assert_eq!(canvas.at(46, 68), CLEAR_BGRA, "one pixel below the image");
}

#[test]
fn a_new_hotspot_on_the_same_surface_moves_the_cursor() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.render().at(46, 44), CLIENT_BGRA);

    // Same surface, new hotspot. `CursorImageStatus::Surface` carries only
    // the surface, so this second status compares equal to the first -- the
    // hotspot has to be re-read from the surface at render time or this
    // change is invisible.
    fixture.run(Step::SetCursor { hotspot: (0, 0) });
    let canvas = fixture.render();
    assert_eq!(canvas.at(50, 50), CLIENT_BGRA, "the new hotspot pixel");
    assert_eq!(canvas.at(73, 73), CLIENT_BGRA, "the image's bottom-right");
    assert_eq!(canvas.at(46, 44), CLEAR_BGRA, "the old position is clear");
}

#[test]
fn a_cursor_surface_with_no_buffer_draws_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateBareCursorSurface);
    fixture.run(Step::SetCursor { hotspot: (0, 0) });
    // Not the fallback triangle: the client did supply a cursor, it just has
    // no content yet, and inventing a shape for it would flash the wrong
    // image between `set_cursor` and the client's first commit.
    fixture.render().assert_blank();
}

#[test]
fn destroying_the_cursor_surface_falls_back_to_the_builtin_shape() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.render().at(50, 50), CLIENT_BGRA);

    // No replacement cursor is set: the surface simply goes away while it is
    // still the active image. Nothing upstream resets the status for us, so
    // this is the case that would otherwise render (or walk) a dead surface.
    fixture.run(Step::DestroyCursorSurface);
    // `CompositorHandler::destroyed` must actually drop the reference, not
    // merely have it skipped at render time: holding a destroyed surface
    // pins its whole `SurfaceData` (and last imported texture) alive for as
    // long as the pointer image is never set again.
    assert!(
        matches!(fixture.state.cursor.status, CursorImageStatus::Named(_)),
        "the destroyed cursor surface is still the active status"
    );

    let canvas = fixture.render();
    assert_eq!(canvas.count, 1, "the fallback element");
    assert_eq!(
        canvas.at(50, 50),
        [0, 0, 0, 255],
        "the triangle's own point"
    );
    assert_eq!(
        canvas.at(51, 53),
        [255, 255, 255, 255],
        "its white interior"
    );
    assert_eq!(
        canvas.at(50 + SIZE, 50),
        CLEAR_BGRA,
        "just past the 16x16 fallback"
    );
}

#[test]
fn hiding_and_restoring_a_cursor_leaves_no_stale_state() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });

    // Hover-driven cursor changes alternate at whatever rate the pointer
    // crosses widget boundaries; nothing may leak or wedge across them.
    for _ in 0..8 {
        fixture.run(Step::SetCursor { hotspot: (4, 6) });
        assert_eq!(fixture.render().at(50, 50), CLIENT_BGRA);
        fixture.run(Step::HideCursor);
        fixture.render().assert_blank();
    }

    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.render().at(46, 44), CLIENT_BGRA);
}

#[test]
fn an_extreme_hotspot_neither_panics_nor_draws() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });

    // The hotspot is the one client-controlled `i32` that flows straight into
    // element geometry, and `wl_pointer.set_cursor` takes it raw -- nothing
    // in the protocol bounds it. `pointer - hotspot` is computed in `f64`
    // (so it cannot wrap) and the cast back saturates, which puts the
    // element far off-canvas in either direction rather than at a wrapped-
    // around coordinate. What this test is really guarding is everything
    // downstream of that: the damage tracker's own `loc + size` arithmetic
    // on a saturated coordinate, in a debug build where a plain `+` would
    // panic on overflow.
    for hotspot in [(i32::MIN, i32::MIN), (i32::MAX, i32::MAX)] {
        fixture.run(Step::SetCursor { hotspot });
        let canvas = fixture.render();
        // The element must really have been built and handed to the damage
        // tracker at that coordinate -- an early bail-out would make the
        // "nothing drawn" assertion below prove nothing.
        assert_eq!(canvas.count, 1, "the element was skipped, not placed");
        canvas.assert_nothing_drawn();
    }

    // ...and a sane hotspot afterwards still works, i.e. nothing was left
    // wedged.
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.render().at(50, 50), CLIENT_BGRA);
}

#[test]
fn a_cursor_surface_with_a_subsurface_draws_both() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::AddSubsurface {
        size: 8,
        color: CHILD_BGRA,
        offset: (24, 0),
    });
    fixture.run(Step::SetCursor { hotspot: (0, 0) });

    // Spec-legal and rare, but nothing here may assume a cursor is exactly
    // one element: the child sits to the right of the parent's 24x24.
    let canvas = fixture.render();
    assert_eq!(canvas.count, 2, "a parent and its subsurface");
    assert_eq!(canvas.at(50, 50), CLIENT_BGRA, "the parent at the hotspot");
    assert_eq!(canvas.at(75, 51), CHILD_BGRA, "the child, offset by 24");
    assert_eq!(canvas.at(82, 51), CLEAR_BGRA, "past the child's 8x8");
}
