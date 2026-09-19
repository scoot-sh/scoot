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
use std::sync::mpsc::{Receiver, Sender, channel};

use smithay::backend::input::InputTime;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Element;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::input::pointer::MotionEvent;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::utils::{Rectangle, SERIAL_COUNTER};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, pixel};

// -------------------------------------------------------------------------
// Pure arithmetic
// -------------------------------------------------------------------------

/// The default size and colors `Appearance::default()` resolves to, spelled
/// out as `generate_bitmap`'s own parameters -- they are config values now,
/// not compile-time constants, so the pure tests pass them explicitly and
/// `the_default_appearance_still_describes_the_original_shape` below is what
/// ties these literals back to the real defaults.
const DEFAULT_SIZE: i32 = 16;
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// What `Cursor::new` is given for its theme name when a test wants the
/// compositor's *own* drawn shapes rather than whatever the machine running
/// the suite happens to have installed.
///
/// A deliberately unresolvable name, not `None`: `None` means "follow
/// `$XCURSOR_THEME`, then `default`", which on a developer's own desktop (or
/// any container with an icon theme installed) would find a real theme and
/// make every pixel assertion below depend on which distro's cursors are
/// present. See `cursor/theme.rs`.
const NO_THEME: Option<&str> = Some("scoot-test-no-such-theme");

#[test]
fn bitmap_hotspot_pixel_is_the_outline_color() {
    let pixels = generate_bitmap(DEFAULT_SIZE, WHITE, BLACK);
    assert_eq!(pixel(&pixels, DEFAULT_SIZE, 0, 0), BLACK);
}

#[test]
fn bitmap_is_fully_transparent_above_the_diagonal() {
    let pixels = generate_bitmap(DEFAULT_SIZE, WHITE, BLACK);
    // (size - 1, 0): far right of the top row, well outside the triangle.
    assert_eq!(
        pixel(&pixels, DEFAULT_SIZE, DEFAULT_SIZE - 1, 0),
        TRANSPARENT
    );
}

#[test]
fn bitmap_interior_is_the_fill_color() {
    let pixels = generate_bitmap(DEFAULT_SIZE, WHITE, BLACK);
    // (1, 3): strictly inside the triangle (0 < x < y).
    assert_eq!(pixel(&pixels, DEFAULT_SIZE, 1, 3), WHITE);
}

/// The built-in defaults are what an unconfigured scoot draws, and they must
/// stay the 16x16 white-on-black triangle this module shipped with -- the
/// literals the three tests above use.
#[test]
fn the_default_appearance_still_describes_the_original_shape() {
    let defaults = Appearance::default();
    assert_eq!(defaults.cursor_size, DEFAULT_SIZE);
    assert_eq!(defaults.cursor_color.to_argb8888(), WHITE);
    // The outline `Cursor::new` derives from that fill color.
    assert_eq!(
        Color::new(0.0, 0.0, 0.0, defaults.cursor_color.a).to_argb8888(),
        BLACK
    );
}

/// A non-default size produces exactly that many pixels, and the shape scales
/// with it rather than staying 16x16 inside a bigger buffer. Runs the whole
/// configurable range, including both clamp bounds.
#[test]
fn bitmap_honors_the_requested_size() {
    for size in [
        Appearance::MIN_CURSOR_SIZE,
        DEFAULT_SIZE,
        48,
        Appearance::MAX_CURSOR_SIZE,
    ] {
        let pixels = generate_bitmap(size, WHITE, BLACK);
        assert_eq!(
            pixels.len(),
            (size * size * 4) as usize,
            "wrong buffer length for size {size}"
        );
        // The bottom-left corner is inside the triangle at every size >= 3,
        // which is what proves the shape grew with the buffer.
        assert_eq!(
            pixel(&pixels, size, 1, size - 1),
            WHITE,
            "size {size}'s bottom row should be filled"
        );
        // ...and the bottom-right corner is on the diagonal, i.e. outline.
        assert_eq!(
            pixel(&pixels, size, size - 1, size - 1),
            BLACK,
            "size {size}'s diagonal should reach the far corner"
        );
    }
}

/// Both colors are written through verbatim, with no channel reordering and
/// no fallback to the old hardcoded white/black. Uses colors whose four bytes
/// are all distinct, so a swap anywhere would change the assertion.
#[test]
fn bitmap_honors_the_requested_colors() {
    let fill = [0x11, 0x22, 0x33, 0xff];
    let outline = [0x44, 0x55, 0x66, 0x80];
    let pixels = generate_bitmap(8, fill, outline);
    assert_eq!(pixel(&pixels, 8, 1, 3), fill, "the interior");
    assert_eq!(pixel(&pixels, 8, 0, 0), outline, "the hotspot corner");
    assert_eq!(pixel(&pixels, 8, 0, 5), outline, "the left edge");
    assert_eq!(pixel(&pixels, 8, 5, 5), outline, "the diagonal");
    assert_eq!(
        pixel(&pixels, 8, 7, 0),
        TRANSPARENT,
        "outside the triangle stays transparent whatever the colors are"
    );
}

/// `Cursor::new` re-applies the size clamp at the allocation itself, so a
/// degenerate or absurd value that somehow reached it (neither is reachable
/// through a config file -- `Appearance::clamped` has already clamped and
/// warned -- but `Appearance`'s fields are public) can neither build a
/// zero-sized buffer nor an allocation whose own length overflows `i32`
/// (which `size * size * 4` does from `size = 23171` up). Needs only a
/// renderer, not a
/// whole compositor: the element's geometry is the built buffer's real size.
#[test]
fn cursor_new_clamps_a_degenerate_or_absurd_size() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    for size in [
        i32::MIN,
        -1,
        0,
        1,
        Appearance::MIN_CURSOR_SIZE,
        48,
        Appearance::MAX_CURSOR_SIZE,
        i32::MAX,
    ] {
        let cursor = Cursor::new(size, Color::new(1.0, 1.0, 1.0, 1.0), NO_THEME);
        let elements = cursor.element(&mut renderer, (0.0, 0.0).into(), 1.0);
        assert_eq!(
            elements.len(),
            1,
            "size {size} produced no fallback element"
        );
        let geometry = elements[0].geometry(1.0.into());
        let expected = Appearance::clamp_cursor_size(size);
        assert_eq!(
            (geometry.size.w, geometry.size.h),
            (expected, expected),
            "size {size} should have been clamped"
        );
    }
}

#[test]
fn a_zero_hotspot_leaves_the_pointer_location_unchanged() {
    let pointer = Point::<f64, Logical>::from((12.5, 30.0));
    let hotspot = Point::<i32, Logical>::from((0, 0));
    let location = element_location(pointer, hotspot, 1.0);
    assert_eq!((location.x, location.y), (12.5, 30.0));
}

#[test]
fn a_nonzero_hotspot_offsets_the_pointer_location() {
    let pointer = Point::<f64, Logical>::from((100.0, 100.0));
    let hotspot = Point::<i32, Logical>::from((4, 6));
    let location = element_location(pointer, hotspot, 1.0);
    assert_eq!((location.x, location.y), (96.0, 94.0));
}

/// At a scale other than 1 both the pointer and the hotspot are in logical
/// space, so both scale -- the hotspot offset must not stay at its logical
/// size while the pointer moves to physical coordinates, or the cursor's tip
/// drifts away from what it is pointing at by `hotspot * (scale - 1)`.
#[test]
fn a_nonzero_hotspot_scales_with_the_output() {
    let pointer = Point::<f64, Logical>::from((50.0, 50.0));
    let hotspot = Point::<i32, Logical>::from((4, 6));
    let location = element_location(pointer, hotspot, 2.0);
    assert_eq!((location.x, location.y), (92.0, 88.0));
}

/// A fractional scale keeps the subtraction exact rather than rounding the
/// pointer and the hotspot separately: subtracting first is what makes the
/// cursor tip land on the same physical subpixel the pointer is over.
#[test]
fn a_fractional_scale_keeps_the_hotspot_offset_exact() {
    let pointer = Point::<f64, Logical>::from((10.0, 20.0));
    let hotspot = Point::<i32, Logical>::from((1, 2));
    let location = element_location(pointer, hotspot, 1.5);
    assert_eq!((location.x, location.y), (13.5, 27.0));
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
    /// `wl_surface.offset` on the cursor surface, followed by a commit --
    /// the request `wayland.xml` says decrements the cursor hotspot.
    OffsetCursorSurface { x: i32, y: i32 },
    /// `wl_surface.offset` on the focus (non-cursor) surface, followed by a
    /// commit -- the negative control proving only the active cursor
    /// surface's offset moves the hotspot.
    OffsetFocusSurface { x: i32, y: i32 },
    /// Destroy the cursor surface without setting any replacement.
    DestroyCursorSurface,
    /// `wp_cursor_shape_device_v1.set_shape`: name a shape and let the
    /// compositor draw it, with no cursor surface anywhere in sight.
    SetShape {
        shape: wp_cursor_shape_device_v1::Shape,
    },
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
    /// `wp_cursor_shape_manager_v1`, if the compositor advertised it. `None`
    /// here is what a test asserting the global exists actually fails on.
    cursor_shape: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
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
                // Version 5, not 4: `wl_surface.offset` is a version-5
                // request, and the cursor-offset tests below need it.
                // Smithay advertises version 5 for this global.
                client.compositor = Some(registry.bind(name, version.min(5), qh, ()));
            }
            "wl_subcompositor" => {
                client.subcompositor = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "wp_cursor_shape_manager_v1" => {
                client.cursor_shape = Some(registry.bind(name, version.min(1), qh, ()));
            }
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
wayland_client::delegate_noop!(TestClient: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(TestClient: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

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
    let fd = rustix::fs::memfd_create("scoot-cursor-test", rustix::fs::MemfdFlags::CLOEXEC)
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
    let focus_surface = focus.clone();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    ids.send(focus.id().protocol_id())
        .map_err(|e| e.to_string())?;

    // Reported back before any step runs, so a test can assert the global is
    // advertised at all rather than only that `set_shape` had an effect.
    ids.send(u32::from(client.cursor_shape.is_some()))
        .map_err(|e| e.to_string())?;

    let mut cursor: Option<wl_surface::WlSurface> = None;
    // Created once and reused: the protocol allows one device per pointer,
    // and re-creating it per step would be a second object for the same
    // pointer rather than a fresh start.
    let mut shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1> = None;
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
            Step::OffsetCursorSurface { x, y } => {
                let cursor = cursor.as_ref().ok_or("no cursor surface")?;
                cursor.offset(x, y);
                cursor.commit();
            }
            Step::OffsetFocusSurface { x, y } => {
                focus_surface.offset(x, y);
                focus_surface.commit();
            }
            Step::DestroyCursorSurface => {
                let surface = cursor.take().ok_or("no cursor surface to destroy")?;
                surface.destroy();
            }
            Step::SetShape { shape } => {
                let serial = client.enter_serial.ok_or("the pointer never entered")?;
                let device = match &shape_device {
                    Some(device) => device,
                    None => {
                        let manager = client
                            .cursor_shape
                            .clone()
                            .ok_or("no wp_cursor_shape_manager_v1")?;
                        shape_device = Some(manager.get_pointer(&pointer, &qh, ()));
                        shape_device.as_ref().expect("the device just created")
                    }
                };
                device.set_shape(serial, shape);
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
/// See [`crate::compositor::test_support`] for everything that is not specific
/// to the cursor.
///
/// The client keeps running (and its surfaces keep existing) until this is
/// dropped, so every assertion a test makes must happen while its `Fixture` is
/// still alive -- dropping it disconnects the client, which destroys its
/// surfaces and resets the very cursor state under test.
type Fixture = Harness<Step, ()>;

impl Fixture {
    fn new() -> Self {
        Self::with_appearance(Appearance::default())
    }

    /// Like [`Fixture::new`], but with the `[appearance]` values a config file
    /// would have resolved to -- which is all `Cursor::new` ever sees, since
    /// the fallback bitmap is built once in `State::new` and never rebuilt.
    fn with_appearance(appearance: Appearance) -> Self {
        Self::start(appearance).0
    }

    /// [`Fixture::new`], plus whether the client found
    /// `wp_cursor_shape_manager_v1` in its registry.
    ///
    /// Only the client's own registry roundtrip can answer that, and only one
    /// test asks, so it is handed back here rather than kept as fixture state.
    fn reporting_globals() -> (Self, bool) {
        Self::start(Appearance::default())
    }

    fn start(mut appearance: Appearance) -> (Self, bool) {
        // Force the drawn shapes, whatever the machine running the suite has
        // installed. Every pixel assertion below describes `shapes.rs`'s own
        // output; with `Appearance::default()`'s `cursor_theme: None` these
        // would instead assert on whichever xcursor theme happens to be
        // present, and would pass on a bare container while failing on a
        // developer's desktop. The themed path is covered hermetically in
        // `cursor/theme/tests.rs`.
        appearance.cursor_theme = NO_THEME.map(str::to_owned);
        // No backend: nothing here renders through the compositor's own
        // pipeline. [`Fixture::frame`] draws the cursor elements offscreen.
        let mut fixture = Harness::bare(appearance);
        let (id_tx, id_rx) = channel();
        fixture.spawn(move |stream, steps, acks| run_client(stream, id_tx, steps, acks));

        // Give the pointer a focus inside this client, which is what makes
        // its later `set_cursor` calls legal (Smithay checks the serial
        // against the last `wl_pointer.enter` it sent).
        let focus_id = fixture.wait_for(0, &id_rx, "the client's focus surface id");
        // Sent straight after the id, from the same registry roundtrip.
        let cursor_shape_advertised =
            fixture.wait_for(0, &id_rx, "the client's cursor-shape report") != 0;
        let focus: ServerSurface = fixture
            .client(0)
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
        (fixture, cursor_shape_advertised)
    }

    /// Renders this frame's cursor elements into a [`CANVAS`]-square
    /// framebuffer and hands back the raw BGRA pixels, exactly the layout
    /// `render/pixman.rs` composites into.
    fn frame(&self) -> Canvas {
        let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
        let mut image = renderer
            .create_buffer(Fourcc::Argb8888, (CANVAS, CANVAS).into())
            .expect("an offscreen buffer");
        let elements =
            self.state
                .cursor
                .element(&mut renderer, POINTER.into(), self.state.output_scale);
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

/// One rendered frame: its raw BGRA pixels and how many cursor elements went
/// into it.
struct Canvas {
    pixels: Vec<u8>,
    count: usize,
}

impl Canvas {
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        pixel(&self.pixels, CANVAS, x, y)
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

    let canvas = fixture.frame();
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
    assert_eq!(fixture.frame().at(46, 44), CLIENT_BGRA);

    // Same surface, new hotspot. `CursorImageStatus::Surface` carries only
    // the surface, so this second status compares equal to the first -- the
    // hotspot has to be re-read from the surface at render time or this
    // change is invisible.
    fixture.run(Step::SetCursor { hotspot: (0, 0) });
    let canvas = fixture.frame();
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
    fixture.frame().assert_blank();
}

#[test]
fn destroying_the_cursor_surface_falls_back_to_the_builtin_shape() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(50, 50), CLIENT_BGRA);

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

    let canvas = fixture.frame();
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
        canvas.at(50 + DEFAULT_SIZE, 50),
        CLEAR_BGRA,
        "just past the 16x16 fallback"
    );
}

/// The whole point of the `[appearance]` override, end to end: a configured
/// size and fill color reach the pixels that actually get drawn.
///
/// This is the test that can catch a byte-order mistake in
/// `Color::to_argb8888`, which no pure test of the bitmap can: white and
/// black are symmetric under a B<->R swap, so the colors here deliberately
/// are not. It also proves premultiplication survives a real pixman import
/// rather than only the unit conversion.
#[test]
fn a_configured_cursor_size_and_color_reach_the_rendered_pixels() {
    const SIZE: i32 = 48;
    // #ff8040: R=255, G=128, B=64 -- distinct in all three channels, so the
    // BGRA bytes below could not come out right under any reordering. Also
    // deliberately *not* `#ff8000`, whose BGRA bytes reversed would be
    // `CLEAR_BGRA` exactly: a swap would then draw the cursor in the
    // background's own color, and the failure could not tell "wrong color"
    // from "nothing drawn".
    let fill = Color::parse("#ff8040").expect("a valid color");
    let fill_bgra = [0x40, 0x80, 0xff, 0xff];

    let fixture = Fixture::with_appearance(Appearance {
        cursor_size: SIZE,
        cursor_color: fill,
        ..Appearance::default()
    });
    // No client cursor is ever set, so this is the fallback shape: the status
    // `State::new` starts with is `default_named()`.
    let canvas = fixture.frame();
    assert_eq!(canvas.count, 1, "the fallback element");

    let (px, py) = (POINTER.0 as i32, POINTER.1 as i32);
    assert_eq!(canvas.at(px, py), BLACK, "the hotspot is still the outline");
    assert_eq!(canvas.at(px, py + 10), BLACK, "the left edge");
    assert_eq!(canvas.at(px + 10, py + 10), BLACK, "the diagonal");
    assert_eq!(canvas.at(px + 2, py + 20), fill_bgra, "the configured fill");
    assert_eq!(
        canvas.at(px + 40, py + 2),
        CLEAR_BGRA,
        "outside the triangle, still the clear color"
    );
    // The size took: the last row of a 48px shape is filled, the row after it
    // is off the bitmap entirely. This is what a still-16x16 buffer (or a
    // 48px buffer with a 16px shape in it) would fail.
    assert_eq!(
        canvas.at(px + 1, py + SIZE - 1),
        fill_bgra,
        "the bottom row of the configured size"
    );
    assert_eq!(
        canvas.at(px + 1, py + SIZE),
        CLEAR_BGRA,
        "one row past the configured size"
    );
}

/// A translucent `cursor_color` composites over what is behind it instead of
/// replacing it -- the case premultiplied alpha exists for, and the reason
/// `Cursor::new` gives the outline the fill's alpha rather than full opacity.
#[test]
fn a_translucent_cursor_color_blends_with_the_background() {
    // 50% white over the clear color: pixman's OVER on premultiplied
    // components gives dst * (1 - a) + src, i.e. (255 + 128) / 2 rounded for
    // each channel of CLEAR_BGRA = [255, 128, 0, 255].
    let fixture = Fixture::with_appearance(Appearance {
        cursor_size: 16,
        cursor_color: Color::parse("#ffffff80").expect("a valid color"),
        ..Appearance::default()
    });
    let canvas = fixture.frame();
    let (px, py) = (POINTER.0 as i32, POINTER.1 as i32);

    let blended = canvas.at(px + 1, py + 3);
    assert_ne!(blended, WHITE, "a translucent fill must not draw as opaque");
    assert_ne!(blended, CLEAR_BGRA, "...but must still draw something");
    // Half of the fill plus half of the background, within one unit of
    // rounding on each channel.
    for (channel, (got, (src, dst))) in blended
        .iter()
        .zip([255u8, 255, 255, 255].iter().zip(CLEAR_BGRA.iter()))
        .enumerate()
    {
        let expected = (i32::from(*src) + i32::from(*dst)) / 2;
        assert!(
            (i32::from(*got) - expected).abs() <= 1,
            "channel {channel}: {got} is not about halfway between {src} and {dst}"
        );
    }
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
        assert_eq!(fixture.frame().at(50, 50), CLIENT_BGRA);
        fixture.run(Step::HideCursor);
        fixture.frame().assert_blank();
    }

    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(46, 44), CLIENT_BGRA);
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
        let canvas = fixture.frame();
        // The element must really have been built and handed to the damage
        // tracker at that coordinate -- an early bail-out would make the
        // "nothing drawn" assertion below prove nothing.
        assert_eq!(canvas.count, 1, "the element was skipped, not placed");
        canvas.assert_nothing_drawn();
    }

    // ...and a sane hotspot afterwards still works, i.e. nothing was left
    // wedged.
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(50, 50), CLIENT_BGRA);
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
    let canvas = fixture.frame();
    assert_eq!(canvas.count, 2, "a parent and its subsurface");
    assert_eq!(canvas.at(50, 50), CLIENT_BGRA, "the parent at the hotspot");
    assert_eq!(canvas.at(75, 51), CHILD_BGRA, "the child, offset by 24");
    assert_eq!(canvas.at(82, 51), CLEAR_BGRA, "past the child's 8x8");
}

// -------------------------------------------------------------------------
// `wp-cursor-shape-v1`
// -------------------------------------------------------------------------

/// The BGRA bytes the default `[appearance]` draws a shape's fill and outline
/// in, as they land on the canvas.
const FILL_BGRA: [u8; 4] = [255, 255, 255, 255];
const OUTLINE_BGRA: [u8; 4] = [0, 0, 0, 255];

#[test]
fn the_cursor_shape_manager_is_advertised() {
    // The global itself, not its effect: a client that cannot find it never
    // gets as far as `set_shape`, and every test below would then be
    // asserting on the default status rather than on anything the protocol
    // did. `foot` prints "compositor does not implement server-side cursors"
    // on exactly this.
    let (_fixture, advertised) = Fixture::reporting_globals();
    assert!(
        advertised,
        "wp_cursor_shape_manager_v1 is not in the registry"
    );
}

#[test]
fn a_named_shape_reaches_the_rendered_pixels() {
    let mut fixture = Fixture::new();
    // The default status is already `Named(Default)`, so asking for the
    // arrow would prove nothing. `Text` is the shape a terminal asks for the
    // moment the pointer is over its grid, and it is drawn as an I-beam --
    // which has *nothing* at the pointer's own position minus the hotspot,
    // where the arrow has its point.
    fixture.run(Step::SetShape {
        shape: wp_cursor_shape_device_v1::Shape::Text,
    });

    assert!(
        matches!(
            fixture.state.cursor.status,
            CursorImageStatus::Named(CursorIcon::Text)
        ),
        "set_shape(text) did not reach the cursor's status"
    );

    let canvas = fixture.frame();
    assert_eq!(canvas.count, 1, "one element for a named shape");
    // The I-beam is centred on the pointer (see `Shape::hotspot`), so its
    // 16x16 bitmap occupies x 42..58, y 42..58 -- and the arrow's own
    // top-left point, at the pointer itself, is nowhere in it.
    assert_eq!(
        canvas.at(50, 50),
        FILL_BGRA,
        "the I-beam's stem runs through the pointer"
    );
    assert_eq!(
        canvas.at(41, 50),
        CLEAR_BGRA,
        "one pixel left of the centred 16x16 bitmap"
    );
    assert_eq!(
        canvas.at(58, 50),
        CLEAR_BGRA,
        "one pixel right of the centred 16x16 bitmap"
    );
    // The arrow would have filled the quadrant below-right of the pointer;
    // the I-beam does not, which is what distinguishes "drew the shape asked
    // for" from "drew the default and happened to cover the middle".
    assert_eq!(
        canvas.at(55, 55),
        CLEAR_BGRA,
        "the arrow's interior is empty for an I-beam"
    );
}

#[test]
fn different_named_shapes_draw_different_pixels() {
    // The reason the protocol is worth having: one compositor-drawn shape per
    // name, not one shape for every name. Compared as whole frames so this
    // cannot pass on a single lucky pixel.
    let mut fixture = Fixture::new();
    let mut frames: Vec<(wp_cursor_shape_device_v1::Shape, Vec<u8>)> = Vec::new();
    for shape in [
        wp_cursor_shape_device_v1::Shape::Default,
        wp_cursor_shape_device_v1::Shape::Text,
        wp_cursor_shape_device_v1::Shape::Crosshair,
        wp_cursor_shape_device_v1::Shape::EwResize,
        wp_cursor_shape_device_v1::Shape::NsResize,
        wp_cursor_shape_device_v1::Shape::NotAllowed,
        wp_cursor_shape_device_v1::Shape::Move,
    ] {
        fixture.run(Step::SetShape { shape });
        frames.push((shape, fixture.frame().pixels));
    }
    for (i, (shape, pixels)) in frames.iter().enumerate() {
        for (other, other_pixels) in &frames[i + 1..] {
            assert_ne!(
                pixels, other_pixels,
                "{shape:?} and {other:?} rendered identically"
            );
        }
    }
}

#[test]
fn an_unmapped_shape_name_still_draws_the_arrow() {
    // Every name this compositor does not draw something specific for falls
    // back to the arrow rather than to nothing -- see `Shape::for_icon`. A
    // pointer that vanishes because a client named `wait` would be a worse
    // bug than the wrong shape.
    let mut fixture = Fixture::new();
    fixture.run(Step::SetShape {
        shape: wp_cursor_shape_device_v1::Shape::Wait,
    });
    let canvas = fixture.frame();
    assert_eq!(canvas.count, 1, "the arrow element");
    assert_eq!(
        canvas.at(50, 50),
        OUTLINE_BGRA,
        "the arrow's own point, at the pointer"
    );
    assert_eq!(canvas.at(51, 53), FILL_BGRA, "its white interior");
}

#[test]
fn a_named_shape_replaces_a_client_cursor_surface() {
    // The two sources of cursor pixels are one status, so a client that
    // uploaded a surface and then named a shape must stop showing the
    // surface -- otherwise its own stale image outlives the request that
    // replaced it, and (worse) keeps its `SurfaceData` pinned.
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(50, 50), CLIENT_BGRA);

    fixture.run(Step::SetShape {
        shape: wp_cursor_shape_device_v1::Shape::Crosshair,
    });
    assert!(
        matches!(
            fixture.state.cursor.status,
            CursorImageStatus::Named(CursorIcon::Crosshair)
        ),
        "the client's surface is still the active cursor image"
    );
    let canvas = fixture.frame();
    assert_eq!(canvas.count, 1, "one element, and it is not the surface");
    assert_ne!(
        canvas.at(50, 50),
        CLIENT_BGRA,
        "the client's own pixels are still being drawn"
    );
}

// -------------------------------------------------------------------------
// `wl_surface.offset` on the cursor surface
// -------------------------------------------------------------------------

/// `wayland.xml` (`wl_pointer.set_cursor`): "On wl_surface.offset requests
/// to the pointer surface, hotspot_x and hotspot_y are decremented by the x
/// and y parameters passed to the request. The offset must be applied by
/// wl_surface.commit as usual." The image content shifts with the offset
/// while the hotspot follows it, so the hotspot pixel stays exactly on the
/// pointer -- without the adjustment the image moves but the click point
/// does not, and every assertion below fails on the stale position.
#[test]
fn a_surface_offset_moves_the_hotspot_with_the_image() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(46, 44), CLIENT_BGRA);

    // hotspot (4, 6) - (4, 6) = (0, 0): the 24x24 image moves from
    // x 46..70, y 44..68 to x 50..74, y 50..74.
    fixture.run(Step::OffsetCursorSurface { x: 4, y: 6 });
    let canvas = fixture.frame();
    // Hotspot (0, 0) means the image's top-left is the hotspot pixel: both
    // land exactly on the pointer.
    assert_eq!(canvas.at(46, 44), CLEAR_BGRA, "the old top-left is clear");
    assert_eq!(
        canvas.at(50, 50),
        CLIENT_BGRA,
        "hotspot and top-left on the pointer"
    );
    assert_eq!(
        canvas.at(73, 73),
        CLIENT_BGRA,
        "the image's new bottom-right"
    );
    assert_eq!(
        canvas.at(74, 73),
        CLEAR_BGRA,
        "one pixel right of the image"
    );
    assert_eq!(canvas.at(49, 50), CLEAR_BGRA, "one pixel left of the image");
    assert_eq!(canvas.at(50, 49), CLEAR_BGRA, "one pixel above the image");

    // Offsets accumulate across commits: hotspot (0, 0) - (2, 3) = (-2, -3),
    // so the image moves to x 52..76, y 53..77. A negative hotspot is legal
    // -- the pointer then aims past the image's top-left, so the pointer's
    // own pixel is clear while the image still sits exactly at
    // pointer - hotspot.
    fixture.run(Step::OffsetCursorSurface { x: 2, y: 3 });
    let canvas = fixture.frame();
    assert_eq!(
        canvas.at(50, 50),
        CLEAR_BGRA,
        "the pointer aims 2px left / 3px above the image"
    );
    assert_eq!(canvas.at(52, 53), CLIENT_BGRA, "the image's top-left");
    assert_eq!(canvas.at(75, 76), CLIENT_BGRA, "the image's bottom-right");
    assert_eq!(canvas.at(51, 53), CLEAR_BGRA, "one pixel left of the image");
    assert_eq!(canvas.at(52, 52), CLEAR_BGRA, "one pixel above the image");
}

/// An offset committed after a *new* `set_cursor` applies to the new
/// hotspot, not the one the surface was first given: `set_cursor` rewrites
/// the hotspot outright, while offset only decrements whatever is current.
#[test]
fn an_offset_after_a_new_hotspot_applies_to_the_new_hotspot() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    fixture.run(Step::SetCursor { hotspot: (10, 12) });
    assert_eq!(fixture.frame().at(40, 38), CLIENT_BGRA);

    // hotspot (10, 12) - (2, 2) = (8, 10): the image moves from x 40..64,
    // y 38..62 to x 42..66, y 40..64.
    fixture.run(Step::OffsetCursorSurface { x: 2, y: 2 });
    let canvas = fixture.frame();
    assert_eq!(
        canvas.at(50, 50),
        CLIENT_BGRA,
        "the hotspot stays on the pointer"
    );
    assert_eq!(canvas.at(42, 40), CLIENT_BGRA, "the image's new top-left");
    assert_eq!(
        canvas.at(65, 63),
        CLIENT_BGRA,
        "the image's new bottom-right"
    );
    assert_eq!(
        canvas.at(40, 38),
        CLEAR_BGRA,
        "the pre-offset top-left is clear"
    );
    assert_eq!(canvas.at(41, 40), CLEAR_BGRA, "one pixel left of the image");
}

/// Both operands are client-controlled `i32` (`set_cursor` takes the hotspot
/// raw, `wl_surface.offset` takes the delta raw), so `i32::MIN - 1` is one
/// malicious commit away -- and a plain `-=` panics a debug build, taking
/// every client's unsaved state down with the compositor. The adjustment
/// saturates instead, which keeps the image off-canvas in either direction
/// rather than at a wrapped-around coordinate.
#[test]
fn an_extreme_offset_saturates_instead_of_panicking() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor {
        hotspot: (i32::MIN, i32::MIN),
    });
    fixture.frame().assert_nothing_drawn();

    fixture.run(Step::OffsetCursorSurface { x: 1, y: 1 });
    let canvas = fixture.frame();
    assert_eq!(canvas.count, 1, "the element was skipped, not placed");
    canvas.assert_nothing_drawn();

    // ...and a sane hotspot afterwards still works, i.e. nothing was left
    // wedged.
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(50, 50), CLIENT_BGRA);
}

/// Only the active cursor surface's own offset moves the hotspot: an offset
/// committed on any other surface -- here the focus surface the pointer is
/// over -- must leave the cursor exactly where it was.
#[test]
fn an_offset_on_another_surface_leaves_the_cursor_alone() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CommitCursorBuffer {
        size: 24,
        color: CLIENT_BGRA,
    });
    fixture.run(Step::SetCursor { hotspot: (4, 6) });
    assert_eq!(fixture.frame().at(46, 44), CLIENT_BGRA);

    fixture.run(Step::OffsetFocusSurface { x: 10, y: 10 });
    let canvas = fixture.frame();
    assert_eq!(
        canvas.at(50, 50),
        CLIENT_BGRA,
        "the hotspot stays on the pointer"
    );
    assert_eq!(
        canvas.at(46, 44),
        CLIENT_BGRA,
        "the image's top-left is unmoved"
    );
    assert_eq!(
        canvas.at(69, 67),
        CLIENT_BGRA,
        "the image's bottom-right is unmoved"
    );
}

/// The fallback path's one-element `Vec` is already the minimal allocation
/// (see `docs/backlog/resolved/cursor-element-per-frame-alloc-done.md`):
/// exactly one element at exactly capacity one -- a single 448-byte
/// `CursorElement<PixmanRenderer>` allocation per dirty frame at this
/// writing, measured ~107ns net per call on the dev VM (release), firing at
/// most once per 16ms frame tick under `--tty` and never at idle. This pins
/// that shape so a future refactor can't silently grow it: pushing into a
/// fresh caller-owned `Vec` -- the ticket's named cheaper shape -- allocates
/// four elements' worth (1792 bytes, measured) by Rust's minimum non-zero
/// capacity rule, so only a persistent reused buffer would beat this line,
/// and that ripple is filed in the record as costing more than it saves.
#[test]
fn fallback_element_is_one_exactly_sized_allocation() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let cursor = Cursor::new(DEFAULT_SIZE, Color::new(1.0, 1.0, 1.0, 1.0), NO_THEME);
    let elements = cursor.element(&mut renderer, (0.0, 0.0).into(), 1.0);
    assert_eq!(elements.len(), 1, "one element for the fallback cursor");
    assert_eq!(elements.capacity(), 1, "exactly sized, with no slack");
}

/// The hidden path allocates nothing at all (`Vec::new` with no push): a
/// hidden cursor costs zero bytes on the render path no matter the frame
/// rate, which is the other half of the ticket's rate analysis.
#[test]
fn hidden_cursor_allocates_nothing() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let mut cursor = Cursor::new(DEFAULT_SIZE, Color::new(1.0, 1.0, 1.0, 1.0), NO_THEME);
    cursor.set_status(CursorImageStatus::Hidden);
    let elements = cursor.element(&mut renderer, (0.0, 0.0).into(), 1.0);
    assert!(elements.is_empty(), "a hidden cursor draws nothing");
    assert_eq!(elements.capacity(), 0, "and allocates nothing to say so");
}
