//! Tests for output scaling: the config-independent helpers here, and a real
//! `wayland-client` connection that binds `wp_fractional_scale_v1`,
//! `wp_viewporter` and `wl_output` and asserts what it is actually told.
//!
//! The pure tests need no live compositor: `clamp_scale`, `smithay_scale` and
//! `logical_size` are all functions of their arguments (an `Output` is a plain
//! value, constructible without a backend). The client tests drive a real
//! [`State`] through a real socket pair and a real renderer (pixman, or GLES
//! under `SCOOT_TEST_RENDERER=gles`), the same
//! choice `layer_shell/tests.rs` made and for the same reason: "what did the
//! client actually receive" and "where did the surface actually land on
//! screen" are both invisible to a test that calls the handler directly.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use scoot_core::Config;
use scoot_ipc::{Request, Response};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::Transform;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;

fn output_at(physical: (i32, i32), scale: Scale) -> Output {
    let output = Output::new(
        "test".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: "test".into(),
            serial_number: "0".into(),
        },
    );
    let mode = Mode {
        size: physical.into(),
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), Some(Transform::Normal), Some(scale), None);
    output
}

// -- clamp_scale ---------------------------------------------------------

/// Both ends of the range, the endpoints themselves, a common fractional
/// value, and the two spellings a TOML file can use for "not a number".
#[test]
fn clamp_scale_brings_everything_into_range() {
    for (configured, expected) in [
        (f64::NAN, 1.0),
        (f64::INFINITY, 1.0),
        (f64::NEG_INFINITY, 1.0),
        (f64::MIN, MIN_SCALE),
        (-1.0, MIN_SCALE),
        (0.0, MIN_SCALE),
        (MIN_SCALE, MIN_SCALE),
        (0.75, 0.75),
        (1.0, 1.0),
        (1.25, 1.25),
        (1.5, 1.5),
        (2.0, 2.0),
        (MAX_SCALE, MAX_SCALE),
        (MAX_SCALE + 1.0, MAX_SCALE),
        (f64::MAX, MAX_SCALE),
    ] {
        assert_eq!(
            clamp_scale(configured),
            expected,
            "clamp_scale({configured}) was wrong"
        );
    }
}

/// The value TOML `nan`/`inf` resolve to must not reach
/// `Scale::fractional_scale()`: `physical / NaN` is NaN, and no layout is.
#[test]
fn a_non_finite_scale_is_never_returned() {
    for scale in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(clamp_scale(scale).is_finite(), "clamp_scale({scale})");
    }
}

// -- smithay_scale -------------------------------------------------------

/// Exactly 1.0 stays `Scale::Integer(1)`, so a session that never asked for
/// scaling is byte-identical to the one before this feature existed; anything
/// else is fractional, which is what lets `wl_output.scale` carry `ceil`.
#[test]
fn exactly_one_is_spelled_as_the_integer_variant() {
    assert!(matches!(smithay_scale(1.0), Scale::Integer(1)));
    assert!(matches!(
        smithay_scale(1.5),
        Scale::Fractional(f) if f == 1.5
    ));
    assert!(matches!(
        smithay_scale(0.5),
        Scale::Fractional(f) if f == 0.5
    ));
}

/// The integer `wl_surface.preferred_buffer_scale` is `ceil`, matching what
/// Smithay puts on `wl_output.scale` for the same `Scale`: `1.5` rounds up to
/// `2`, `2.0` stays `2`, `1.0` stays `1`, and both ends of the clamp are
/// integers already.
#[test]
fn integer_scale_rounds_up_to_match_wl_output_scale() {
    for (scale, expected) in [
        (MIN_SCALE, 1),
        (0.75, 1),
        (1.0, 1),
        (1.25, 2),
        (1.5, 2),
        (2.0, 2),
        (2.5, 3),
        (MAX_SCALE, 4),
    ] {
        assert_eq!(
            integer_scale(scale),
            expected,
            "integer_scale({scale}) was wrong"
        );
        // The value must agree with Smithay's own integer scale for the
        // `Scale` this configured value resolves to -- they are two views of
        // the same number and a client may see both.
        assert_eq!(
            integer_scale(scale),
            smithay_scale(scale).integer_scale(),
            "integer_scale({scale}) disagrees with Scale::integer_scale()"
        );
    }
}

// -- logical_size --------------------------------------------------------

/// The exact rectangles a real panel produces, including the one that does
/// *not* divide evenly: `ceil` is what Smithay's own `output_geometry` uses,
/// so a 2560-wide panel at 1.5 is 1707 logical pixels and the core must be
/// told 1707, not 1706.
#[test]
fn logical_size_matches_smithays_rounding_in_both_directions() {
    for (physical, scale, expected) in [
        ((2880, 1800), 1.5, (1920, 1200)),
        ((2560, 1600), 1.5, (1707, 1067)),
        ((2880, 1800), 2.0, (1440, 900)),
        ((1920, 1080), 1.0, (1920, 1080)),
        ((1920, 1080), 0.5, (3840, 2160)),
        ((2880, 1800), 4.0, (720, 450)),
    ] {
        let output = output_at(physical, smithay_scale(scale));
        assert_eq!(
            logical_size(&output),
            expected,
            "{physical:?} at scale {scale}"
        );
    }
}

/// An output with no mode at all reads as `(0, 0)`, exactly as the
/// `unwrap_or((0, 0))` every caller used before this helper existed.
#[test]
fn logical_size_is_zero_without_a_mode() {
    let output = Output::new(
        "mode-less".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: "test".into(),
            serial_number: "0".into(),
        },
    );
    assert_eq!(logical_size(&output), (0, 0));
}

// -- the input clamp at a scale other than 1 ----------------------------

/// The off-desktop bug in its smallest form: `pointer_move_relative` used to
/// clamp against `current_mode().size` -- the *physical* framebuffer -- while
/// the pointer lives in logical coordinates. At scale 2 a 200px framebuffer is
/// only 100 logical pixels wide, so a large relative motion must stop at 99,
/// not at the 199 the physical extent would allow.
#[test]
fn relative_pointer_motion_is_clamped_to_the_logical_extent() {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let mut state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        crate::compositor::keybindings::Keybindings::default(),
        crate::compositor::decorations::Appearance::default(),
        2.0,
        crate::compositor::test_support::test_renderer(),
    )
    .expect("a compositor state with a wayland socket");
    crate::compositor::headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

    state.pointer_move_relative(1000.0, 1000.0, 1000.0, 1000.0);
    let pointer = state.seat.get_pointer().expect("a pointer");
    let location = pointer.current_location();
    assert_eq!(
        (location.x, location.y),
        (99.0, 99.0),
        "the pointer was clamped to the physical extent, not the logical one"
    );
}

/// Startup placement centres in *logical* coordinates too: at scale 2 the
/// 200px framebuffer is a 100x100 desktop, so the centre is (50, 50), not
/// the (100, 100) the physical size would give.
#[test]
fn startup_placement_centres_on_the_logical_extent() {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let mut state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        crate::compositor::keybindings::Keybindings::default(),
        crate::compositor::decorations::Appearance::default(),
        2.0,
        crate::compositor::test_support::test_renderer(),
    )
    .expect("a compositor state with a wayland socket");
    crate::compositor::headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

    let location = state
        .seat
        .get_pointer()
        .expect("a pointer")
        .current_location();
    assert_eq!(
        (location.x, location.y),
        (50.0, 50.0),
        "the pointer was centred on the physical extent, not the logical one"
    );
}

// -------------------------------------------------------------------------
// A live compositor, a live client, real pixels
// -------------------------------------------------------------------------

/// The physical framebuffer these tests render into. At scale 2 this is a
/// 100x100 logical desktop, which is plenty for one small window.
const CANVAS: i32 = 200;
/// The opaque BGRA bytes a test window commits as its buffer.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];

/// One instruction for the client thread.
enum Step {
    /// Create a surface, attach a `wp_fractional_scale_v1` to it, bind a v6
    /// `wl_compositor`, round trip, and report the preferred scales it was
    /// sent plus the `wl_output.scale`.
    Negotiate,
    /// Report the scale values the client has been sent so far, without
    /// creating anything -- what a reload's re-send is measured against.
    ReportScales,
    /// Map an `xdg_toplevel` whose buffer is `buffer`x`buffer` physical
    /// pixels, with an optional viewport destination of `destination` logical
    /// pixels (the shape a fractional-scale client draws at).
    MapWindow {
        buffer: i32,
        destination: Option<(i32, i32)>,
    },
    /// Like `MapWindow`, but with a `wp_fractional_scale_v1` object on the
    /// toplevel surface itself -- the shape a real fractional-scale client
    /// takes, and the surface a reload's re-send has to reach.
    MapScaledWindow {
        buffer: i32,
        destination: Option<(i32, i32)>,
    },
}

/// What the client reports back.
enum Ack {
    Negotiated {
        preferred_scale: Option<f64>,
        /// `wl_surface.preferred_buffer_scale` (v6), the integer companion to
        /// `preferred_scale`.
        preferred_buffer_scale: Option<i32>,
        /// How many times that event arrived across several commits.
        preferred_buffer_scale_events: u32,
        output_scale: Option<i32>,
    },
    Scales {
        preferred_scale: Option<f64>,
        preferred_buffer_scale: Option<i32>,
        output_scale: Option<i32>,
    },
    Done,
}

/// A surface's own index in creation order, so a configure can be matched back
/// to its `xdg_surface`.
struct SurfaceIndex(usize);

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// Bound and kept alive so `wl_output.scale` (and the rest of its events)
    /// keep arriving.
    output: Option<wl_output::WlOutput>,
    /// `wp_fractional_scale_v1.preferred_scale`, converted from the protocol's
    /// 1/120ths to a plain factor.
    preferred_scale: Option<f64>,
    /// `wl_surface.preferred_buffer_scale` (v6), the integer preference a
    /// client that opted into fractional scaling still expects.
    preferred_buffer_scale: Option<i32>,
    /// How many `preferred_buffer_scale` events that surface was sent. Proves
    /// Smithay's per-surface cache: committing repeatedly at a fixed scale
    /// emits once, not once per commit.
    preferred_buffer_scale_events: u32,
    /// `wl_output.scale`, the integer a client that doesn't speak
    /// fractional-scale is told.
    output_scale: Option<i32>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    serials: Vec<Option<u32>>,
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
                client.compositor = Some(registry.bind(name, version.min(6), qh, ()))
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            // 4, not 3: `scale` (v2) is the field this test needs; binding the
            // highest version the server offers also proves `name`/
            // `description` (v4) don't upset it.
            "wl_output" => client.output = Some(registry.bind(name, version.min(4), qh, ())),
            "wp_fractional_scale_manager_v1" => {
                client.fractional_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wp_viewporter" => {
                client.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            client.output_scale = Some(factor);
        }
    }
}

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        _: wp_fractional_scale_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            // The protocol carries the scale as 1/120ths.
            client.preferred_scale = Some(f64::from(scale) / 120.0);
        }
    }
}

impl Dispatch<wp_viewporter::WpViewporter, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wp_viewporter::WpViewporter,
        _: wp_viewporter::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The viewporter has no events.
    }
}

impl Dispatch<wp_viewport::WpViewport, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wp_viewport::WpViewport,
        _: wp_viewport::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The viewport has no events.
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

impl Dispatch<xdg_surface::XdgSurface, SurfaceIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &SurfaceIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(slot) = client.serials.get_mut(index.0)
        {
            *slot = Some(serial);
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // v6's integer preference. Captured for every surface; the tests read
        // the value the one surface under test was sent.
        if let wl_surface::Event::PreferredBufferScale { factor } = event {
            client.preferred_buffer_scale = Some(factor);
            client.preferred_buffer_scale_events += 1;
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);

/// A `width`x`height` `wl_buffer` filled with `color`, over a real memfd --
/// the same path any toolkit takes.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> wl_buffer::WlBuffer {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-scale-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Round-trips until the `index`-th toplevel's configure serial has arrived.
fn wait_for_serial(
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    index: usize,
) -> Result<u32, String> {
    for _ in 0..50 {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        if let Some(serial) = client.serials[index] {
            return Ok(serial);
        }
    }
    Err("the compositor never configured the toplevel".into())
}

/// A surface and its role objects, held for the test's duration so wayland-
/// client doesn't destroy a role the compositor still has mapped.
type KeptSurface = (
    wl_surface::WlSurface,
    Option<xdg_surface::XdgSurface>,
    Option<xdg_toplevel::XdgToplevel>,
    Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
);

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let manager = client
        .fractional_manager
        .clone()
        .ok_or("no wp_fractional_scale_manager_v1 -- the global is missing")?;
    let viewporter = client
        .viewporter
        .clone()
        .ok_or("no wp_viewporter -- the global is missing")?;

    // Keep every surface and its xdg role objects alive for the test's
    // duration: dropping an `xdg_surface`/`xdg_toplevel` destroys the role.
    let mut keep_alive: Vec<KeptSurface> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match &step {
            Step::Negotiate => {
                let surface = compositor.create_surface(&qh, ());
                let fractional = manager.get_fractional_scale(&surface, &qh, ());
                // Bare commits (no buffer, no role) so the compositor's
                // per-commit path -- where the integer scale is also sent --
                // actually runs. Three of them: Smithay's cache means the
                // client must still see exactly one event.
                surface.commit();
                surface.commit();
                surface.commit();
                // Wait for *both* events. They are independent protocol
                // objects (the integer travels on the surface, the fractional
                // on its own object), so neither implies the other.
                for _ in 0..50 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    if client.preferred_scale.is_some() && client.preferred_buffer_scale.is_some() {
                        break;
                    }
                }
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                outcome = Ack::Negotiated {
                    preferred_scale: client.preferred_scale,
                    preferred_buffer_scale: client.preferred_buffer_scale,
                    preferred_buffer_scale_events: client.preferred_buffer_scale_events,
                    output_scale: client.output_scale,
                };
                keep_alive.push((surface, None, None, Some(fractional)));
            }
            Step::ReportScales => {
                outcome = Ack::Scales {
                    preferred_scale: client.preferred_scale,
                    preferred_buffer_scale: client.preferred_buffer_scale,
                    output_scale: client.output_scale,
                };
            }
            Step::MapWindow {
                buffer,
                destination,
            } => {
                let (buffer, destination) = (*buffer, *destination);
                let surface = compositor.create_surface(&qh, ());
                let index = client.serials.len();
                client.serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceIndex(index));
                let toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                let serial = wait_for_serial(&mut queue, &mut client, index)?;
                xdg.ack_configure(serial);
                if let Some((width, height)) = destination {
                    let viewport = viewporter.get_viewport(&surface, &qh, ());
                    viewport.set_destination(width, height);
                }
                let buf = solid_buffer(&shm, &qh, buffer, buffer, WINDOW_BGRA);
                surface.attach(Some(&buf), 0, 0);
                surface.damage(0, 0, buffer, buffer);
                surface.commit();
                keep_alive.push((surface, Some(xdg), Some(toplevel), None));
            }
            Step::MapScaledWindow {
                buffer,
                destination,
            } => {
                let (buffer, destination) = (*buffer, *destination);
                let surface = compositor.create_surface(&qh, ());
                // The fractional object first, so it exists before the
                // commits below -- the same order a real client takes.
                let fractional = manager.get_fractional_scale(&surface, &qh, ());
                let index = client.serials.len();
                client.serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceIndex(index));
                let toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                let serial = wait_for_serial(&mut queue, &mut client, index)?;
                xdg.ack_configure(serial);
                if let Some((width, height)) = destination {
                    let viewport = viewporter.get_viewport(&surface, &qh, ());
                    viewport.set_destination(width, height);
                }
                let buf = solid_buffer(&shm, &qh, buffer, buffer, WINDOW_BGRA);
                surface.attach(Some(&buf), 0, 0);
                surface.damage(0, 0, buffer, buffer);
                surface.commit();
                keep_alive.push((surface, Some(xdg), Some(toplevel), Some(fractional)));
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend at `scale`, and one
/// connected client.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<Ack>,
    client: Option<JoinHandle<Result<(), String>>>,
    /// Held so the config file `install_config` writes stays alive while
    /// the session reloads from it.
    _config_dir: Option<tempfile::TempDir>,
}

impl Fixture {
    fn new(scale: f64) -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            crate::compositor::keybindings::Keybindings::default(),
            crate::compositor::decorations::Appearance::default(),
            scale,
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        crate::compositor::headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        state
            .display_handle
            .insert_client(
                server_end,
                Arc::new(crate::compositor::state::ClientState::default()),
            )
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, step_rx, ack_tx));

        Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
            _config_dir: None,
        }
    }

    /// Points the session's reload path at a temp file holding `contents` --
    /// the shape `reload/tests.rs` builds its `State` in, so this fixture's
    /// live client can watch a real `Request::Reload` land on the wire.
    fn install_config(&mut self, contents: &str) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).expect("a config file");
        self.state.config_path = Some(path);
        self._config_dir = Some(dir);
    }

    /// Rewrites the installed config file and serves a real `Request::Reload`
    /// against the live session, then settles so the client hears every
    /// re-sent event before its next step runs.
    fn reload_with(&mut self, contents: &str) -> Response {
        let path = self
            .state
            .config_path
            .clone()
            .expect("install_config ran first");
        std::fs::write(&path, contents).expect("a rewritten config file");
        let response = self.state.handle_request(Request::Reload);
        self.settle();
        response
    }

    fn run(&mut self, step: Step) -> Ack {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        let ack = self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
        self.settle();
        ack
    }

    fn wait_for<T>(&mut self, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self
                        .client
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("the client stopped while waiting for {what}: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// Renders a frame and hands back its raw BGRA pixels.
    fn render(&mut self) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        let output = self.state.outputs.primary_id().expect("an output");
        let backend = self.state.backends.get_mut(&output).expect("a backend");
        backend
            .capture(<[u8]>::to_vec)
            .expect("a framebuffer readback")
    }

    /// Where the core says the first window goes, in logical pixels.
    fn window_rect(&self) -> scoot_core::Rect {
        self.state
            .world
            .arrange()
            .placements
            .first()
            .expect("a placed window")
            .rect
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.steps.take());
        if let Some(handle) = self.client.take()
            && !std::thread::panicking()
        {
            let _ = handle.join();
        }
    }
}

fn bgra_at(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    let index = ((y * CANVAS + x) * 4) as usize;
    pixels[index..index + 4].try_into().expect("four bytes")
}

/// A real client gets all of the scale story: `preferred_scale` from
/// `wp_fractional_scale_v1` (the exact fractional value), the integer
/// `wl_surface.preferred_buffer_scale` that backs it up (v6), and
/// `wl_output.scale` (the integer a client that doesn't speak fractional-scale
/// is told). At `1.5` the two integers are both `2` while the fractional value
/// is `1.5`; at `2.0` all three coincide.
///
/// At `1.0` the client receives *no* `preferred_buffer_scale`, and that is
/// correct rather than a gap: Smithay's `send_surface_state` caches a default
/// of `scale: 1` (`compositor/tree.rs`'s `SuggestedSurfaceState::default`) and
/// only emits on a change, so a scale-1 session sends nothing and the client
/// keeps the implicit integer default of 1 -- byte-identical to the behavior
/// before this event existed. The effective integer is 1 either way, which is
/// what the `output_scale`/implicit assertions below pin.
///
/// This is the regression test for the Ghostty failure: remove the
/// `send_surface_state` call and the `1.5`/`2.0` cases fail, because the
/// client then never receives `preferred_buffer_scale` at all.
#[test]
fn a_client_is_told_the_fractional_and_integer_scales() {
    // `expected_buffer_scale` is `None` exactly where Smithay's own cache makes
    // the event redundant (scale 1); `integer` is the effective integer either
    // way.
    for (scale, preferred, expected_buffer_scale, integer) in [
        (1.5, 1.5, Some(2), 2),
        (2.0, 2.0, Some(2), 2),
        (1.0, 1.0, None, 1),
    ] {
        let mut fixture = Fixture::new(scale);
        let Ack::Negotiated {
            preferred_scale,
            preferred_buffer_scale,
            preferred_buffer_scale_events,
            output_scale,
        } = fixture.run(Step::Negotiate)
        else {
            panic!("the negotiate step must report what it saw");
        };
        assert_eq!(
            preferred_scale,
            Some(preferred),
            "wp_fractional_scale_v1.preferred_scale at scale {scale}"
        );
        assert_eq!(
            preferred_buffer_scale, expected_buffer_scale,
            "wl_surface.preferred_buffer_scale at scale {scale}"
        );
        // Three commits were sent; Smithay's per-surface cache means at most
        // one event, so the per-commit path adds no repeated traffic.
        assert_eq!(
            preferred_buffer_scale_events,
            u32::from(expected_buffer_scale.is_some()),
            "preferred_buffer_scale events across three commits at scale {scale}"
        );
        // Whatever was (or wasn't) sent explicitly, the integer the client acts
        // on is `ceil(scale)`.
        assert_eq!(
            preferred_buffer_scale.unwrap_or(1),
            integer,
            "effective wl_surface.preferred_buffer_scale at scale {scale}"
        );
        assert_eq!(
            output_scale,
            Some(integer),
            "wl_output.scale at scale {scale}"
        );
    }
}

/// A window's buffer lands at the *physical* rectangle its logical placement
/// corresponds to, not at the logical coordinates the core arranged it in.
///
/// This is the shape a real fractional-scale client draws at: a buffer sized
/// `logical * scale` (40px for a 20-logical-pixel surface), with the viewport
/// destination set to the logical size so the compositor scales it down.
#[test]
fn a_scaled_surface_lands_at_the_physical_rectangle() {
    let mut fixture = Fixture::new(2.0);
    fixture.run(Step::MapWindow {
        buffer: 40,
        destination: Some((20, 20)),
    });
    let pixels = fixture.render();

    let rect = fixture.window_rect();
    let origin = (rect.x * 2, rect.y * 2);
    assert_eq!(
        bgra_at(&pixels, origin.0 + 10, origin.1 + 10),
        WINDOW_BGRA,
        "the window's pixels are not where logical {:?} maps to at scale 2",
        rect
    );

    // And *not* at the logical coordinates: the old, unscaled behavior would
    // have put the buffer at (rect.x, rect.y). The focus ring is drawn there
    // instead, so the assertion is only "not the window's color".
    assert_ne!(
        bgra_at(&pixels, rect.x + 10, rect.y + 10),
        WINDOW_BGRA,
        "the window's pixels are still at its logical coordinates; the output \
         scale was not applied"
    );

    // A point clear of both the window and its ring is the background.
    assert_ne!(
        bgra_at(&pixels, 2, 2),
        WINDOW_BGRA,
        "nothing should be drawn at the top-left corner"
    );
}

/// A config reload re-sends the whole scale story on the wire: `wl_output`'s
/// integer, the fractional `preferred_scale`, and its integer companion --
/// to a mapped window's surface that negotiated all three *before* the
/// reload, which the bind-time tests above never exercise. The window
/// relayouts into the halved desktop and still draws, so this is also the
/// reload-with-windows-mapped proof: buffers rescale through the new
/// arrangement rather than sticking at the old one.
#[test]
fn a_reload_rescales_what_a_live_client_sees() {
    let mut fixture = Fixture::new(1.0);
    fixture.install_config("");

    // One mapped window with a fractional-scale object on its own surface --
    // the shape a real fractional-scale client takes -- drawing real pixels.
    fixture.run(Step::MapScaledWindow {
        buffer: 20,
        destination: None,
    });
    let Ack::Scales {
        preferred_scale,
        preferred_buffer_scale,
        output_scale,
    } = fixture.run(Step::ReportScales)
    else {
        panic!("the report step must send the cached scales");
    };
    assert_eq!(preferred_scale, Some(1.0));
    assert_eq!(
        preferred_buffer_scale, None,
        "scale 1 sends no integer event"
    );
    assert_eq!(output_scale, Some(1));
    let before = fixture.render();
    let placed = fixture.window_rect();
    assert_eq!(
        bgra_at(&before, placed.x + 5, placed.y + 5),
        WINDOW_BGRA,
        "the mapped window should draw before the reload"
    );

    // The reload itself, through the real request path.
    let response = fixture.reload_with("[output]\nscale = 2.0\n");
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid scale reload should report, not error: {response:?}");
    };
    assert_eq!(applied, &["output.scale".to_owned()]);
    assert!(
        refused.is_empty(),
        "nothing here should refuse: {refused:?}"
    );
    assert_eq!(fixture.state.output_scale, 2.0);
    assert_eq!(fixture.state.integer_scale, 2);

    // The pre-existing surface hears the new scale without re-binding: the
    // fractional value, the integer companion (whose cache moved off its
    // scale-1 default), and `wl_output.scale` together.
    let Ack::Scales {
        preferred_scale,
        preferred_buffer_scale,
        output_scale,
    } = fixture.run(Step::ReportScales)
    else {
        panic!("the report step must send the cached scales");
    };
    assert_eq!(
        preferred_scale,
        Some(2.0),
        "the fractional re-send never reached the pre-existing surface"
    );
    assert_eq!(
        preferred_buffer_scale,
        Some(2),
        "the integer companion re-send never reached the pre-existing surface"
    );
    assert_eq!(
        output_scale,
        Some(2),
        "the wl_output re-advertise never reached the bound client"
    );

    // The desktop halved (200px canvas, scale 2), and the mapped window
    // drew again inside it -- at its new physical rectangle.
    let output = fixture
        .state
        .outputs
        .primary()
        .expect("the headless output")
        .clone();
    let geometry = fixture
        .state
        .space
        .output_geometry(&output)
        .expect("the mapped output's geometry");
    assert_eq!(
        (geometry.size.w, geometry.size.h),
        (CANVAS / 2, CANVAS / 2),
        "the logical desktop should halve at scale 2"
    );
    let after = fixture.render();
    let moved = fixture.window_rect();
    assert!(
        moved.w < placed.w,
        "the window should relayout into the smaller desktop: {} -> {}",
        placed.w,
        moved.w
    );
    assert_eq!(
        bgra_at(&after, moved.x * 2 + 5, moved.y * 2 + 5),
        WINDOW_BGRA,
        "the mapped window's pixels are not at its rescaled rectangle {moved:?}"
    );
}
