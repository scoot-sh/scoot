//! Pixel tests for rounded window corners (`[appearance] corner_radius`).
//!
//! Every test here drives a real `wayland-client` connection through a real
//! [`State`](crate::compositor::State) with a real headless backend, maps
//! real `wl_shm` windows, renders, and asserts on framebuffer bytes -- never
//! on which enum variant a path chose. Colors are exact-byte values (pure
//! red/green client buffers), except the ring and background, which are
//! sampled from the frame itself: pixman truncates where GLES rounds, so the
//! two renderers disagree by 1 LSB on colors derived from floats (see
//! `docs/configuration.md`), and hard-coding those bytes would fail one of
//! the two `SCOOT_TEST_RENDERER` runs.
//!
//! Window placement is read from the live arrangement, never hard-coded, so
//! these survive layout default changes that preserve the tiling contract.
//!
//! Like every other live-`State` suite, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::Rect;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::{Appearance, ring_rects};
use crate::compositor::rounded::{clip_rect, cut_width, physical_radius, ring_layout};
use crate::compositor::test_support::{Harness, assert_pixel, find_color, pixel, wait_for};

mod committed;

/// The framebuffer these tests render into.
const CANVAS: i32 = 400;

/// Pure red, opaque -- what every test window draws.
const WINDOW_BGRA: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];
/// Pure green, opaque -- the popup color.
const POPUP_BGRA: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
/// Purple, opaque -- the wallpaper color. Distinct from the window, the
/// popup, the ring and the background on purpose.
const WALLPAPER_BGRA: [u8; 4] = [0x80, 0x00, 0x80, 0xFF];

type Fixture = Harness<Step, Ack>;

/// What a test tells its client to do.
#[derive(Debug)]
enum Step {
    /// Map a toplevel drawing a solid `color` buffer sized to whatever the
    /// compositor configures (like a real toolkit).
    Window { color: [u8; 4] },
    /// Map a toplevel that draws `shrink` logical pixels short of what it
    /// was configured to, per axis -- a terminal rounding down to whole
    /// character cells (`foot`'s default `resize-by-cells`), a fixed-size
    /// dialog, an older client.
    ShortWindow { color: [u8; 4], shrink: (i32, i32) },
    /// Redraw the `window`-th toplevel (by creation order) `shrink` short
    /// of the size it last acked, with no new configure: a client whose
    /// next frame changes how much of its slot it fills.
    Redraw { window: usize, shrink: (i32, i32) },
    /// Map a no-grab popup parented to the first window, sized `w` x `h`,
    /// anchored to grow down-right from the parent's origin -- over the
    /// parent's top-left rounded corner.
    Popup { color: [u8; 4], w: i32, h: i32 },
    /// Map a fullscreen background-layer wallpaper drawing a solid `color`.
    Wallpaper { color: [u8; 4] },
}

#[derive(Debug)]
enum Ack {
    Done,
}

/// Which `xdg_surface` a configure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurfaceKind {
    Window(usize),
    Popup,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// `wp_fractional_scale_v1.preferred_scale`, converted from the
    /// protocol's 1/120ths to a plain factor. Fixed for the session, so one
    /// slot is enough no matter how many windows map.
    preferred_scale: Option<f64>,
    popup_serial: Option<u32>,
    /// Per-window configure state by creation order: mapping a second
    /// window re-layouts (and reconfigures) the first, so a single shared
    /// slot would let one window's configure overwrite the other's before
    /// it is acked -- acking that serial on the wrong object is a protocol
    /// error.
    window_serials: Vec<Option<u32>>,
    window_sizes: Vec<Option<(i32, i32)>>,
    layer_size: Option<(u32, u32)>,
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
        if interface == wl_compositor::WlCompositor::interface().name {
            // Version 4: `surface.attach`/`commit` are v1, but
            // `damage_buffer` needs v4.
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name {
            client.layer_shell = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface
            == wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1::interface().name
        {
            client.fractional_manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wp_viewporter::WpViewporter::interface().name {
            client.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
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

impl Dispatch<xdg_surface::XdgSurface, SurfaceKind> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        kind: &SurfaceKind,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            match kind {
                SurfaceKind::Window(index) => {
                    if let Some(slot) = client.window_serials.get_mut(*index) {
                        *slot = Some(serial);
                    }
                }
                SurfaceKind::Popup => client.popup_serial = Some(serial),
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, SurfaceKind> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        kind: &SurfaceKind,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            if width > 0 && height > 0 {
                if let SurfaceKind::Window(index) = kind {
                    if let Some(slot) = client.window_sizes.get_mut(*index) {
                        *slot = Some((width, height));
                    }
                }
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer.ack_configure(serial);
            if width > 0 && height > 0 {
                client.layer_size = Some((width, height));
            }
        }
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

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(TestClient: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(TestClient: ignore wp_viewport::WpViewport);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore xdg_popup::XdgPopup);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A `w` x `h` solid-`color` `wl_buffer` over a real memfd.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    w: i32,
    h: i32,
    color: [u8; 4],
) -> wl_buffer::WlBuffer {
    let stride = w * 4;
    let len = (stride * h) as usize;
    let fd = rustix::fs::memfd_create("scoot-rounded-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let bytes: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&bytes).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Commits one solid `color` frame `shrink` logical pixels short of `size`
/// per axis, the way a fractional-aware toolkit draws: the viewport
/// destination at the logical size, the buffer at that size times the
/// preferred scale.
#[allow(clippy::too_many_arguments)]
fn draw_short(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    surface: &wl_surface::WlSurface,
    viewport: &wp_viewport::WpViewport,
    preferred: f64,
    size: (i32, i32),
    shrink: (i32, i32),
    color: [u8; 4],
) {
    let (w, h) = (size.0 - shrink.0, size.1 - shrink.1);
    viewport.set_destination(w, h);
    let (bw, bh) = (
        (f64::from(w) * preferred).round() as i32,
        (f64::from(h) * preferred).round() as i32,
    );
    let buffer = solid_buffer(shm, qh, bw, bh, color);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, bw, bh);
    surface.commit();
}

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
    let layer_shell = client.layer_shell.clone();
    let fractional_manager = client
        .fractional_manager
        .clone()
        .ok_or("no wp_fractional_scale_manager_v1")?;
    let viewporter = client.viewporter.clone().ok_or("no wp_viewporter")?;
    // Held so every mapped surface stays alive for the run.
    let mut surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    // Per toplevel, by creation order: what `Step::Redraw` needs to commit
    // a new frame -- the surface, its viewport and the size it acked.
    let mut toplevels: Vec<(wl_surface::WlSurface, wp_viewport::WpViewport, (i32, i32))> =
        Vec::new();
    let mut roles: Vec<xdg_surface::XdgSurface> = Vec::new();
    // The fractional-scale objects and viewports: dropping either could
    // release surface state the compositor still reads, so they live as
    // long as their surface does.
    let mut fractional_scales: Vec<wp_fractional_scale_v1::WpFractionalScaleV1> = Vec::new();
    let mut viewports: Vec<wp_viewport::WpViewport> = Vec::new();
    let mut parent: Option<xdg_surface::XdgSurface> = None;

    while let Ok(step) = steps.recv() {
        match step {
            Step::Window { color } | Step::ShortWindow { color, .. } => {
                let shrink = match step {
                    Step::ShortWindow { shrink, .. } => shrink,
                    _ => (0, 0),
                };
                let index = client.window_serials.len();
                client.window_serials.push(None);
                client.window_sizes.push(None);
                let surface = compositor.create_surface(&qh, ());
                // A modern toolkit speaks fractional scale whenever the
                // globals exist: learn the session's scale first, then size
                // the buffer from it (the `foot`/GTK shape), with the
                // viewport destination at the configured logical size.
                let fractional = fractional_manager.get_fractional_scale(&surface, &qh, ());
                let preferred = wait_for(&mut queue, &mut client, "a preferred scale", |client| {
                    client.preferred_scale
                })?;
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceKind::Window(index));
                let toplevel = xdg.get_toplevel(&qh, SurfaceKind::Window(index));
                toplevel.set_title("rounded".into());
                surface.commit();
                // A real toolkit sizes its buffer to the configure: wait for
                // one carrying a size, so the drawn window matches the
                // placement the assertions are derived from.
                let (w, h) = wait_for(&mut queue, &mut client, "a sized configure", |client| {
                    client.window_sizes[index]
                })?;
                let serial = wait_for(&mut queue, &mut client, "an xdg serial", |client| {
                    client.window_serials[index]
                })?;
                xdg.ack_configure(serial);
                let viewport = viewporter.get_viewport(&surface, &qh, ());
                draw_short(
                    &shm,
                    &qh,
                    &surface,
                    &viewport,
                    preferred,
                    (w, h),
                    shrink,
                    color,
                );
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                toplevels.push((surface.clone(), viewport.clone(), (w, h)));
                parent = Some(xdg.clone());
                roles.push(xdg);
                surfaces.push(surface);
                fractional_scales.push(fractional);
                viewports.push(viewport);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Redraw { window, shrink } => {
                let (surface, viewport, size) = toplevels.get(window).ok_or("no such toplevel")?;
                let preferred = client.preferred_scale.ok_or("no preferred scale")?;
                draw_short(
                    &shm,
                    &qh,
                    surface,
                    viewport,
                    preferred,
                    *size,
                    shrink,
                    WINDOW_BGRA,
                );
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Popup { color, w, h } => {
                let parent = parent.clone().ok_or("no parent window")?;
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, SurfaceKind::Popup);
                let positioner = wm_base.create_positioner(&qh, ());
                positioner.set_size(w, h);
                positioner.set_anchor_rect(0, 0, 10, 10);
                positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
                positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
                let popup = xdg.get_popup(Some(&parent), &positioner, &qh, ());
                drop(positioner);
                drop(popup);
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "a popup configure", |client| {
                    client.popup_serial
                })?;
                xdg.ack_configure(serial);
                let buffer = solid_buffer(&shm, &qh, w, h, color);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage_buffer(0, 0, w, h);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                roles.push(xdg);
                surfaces.push(surface);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::Wallpaper { color } => {
                let layer_shell = layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?;
                let surface = compositor.create_surface(&qh, ());
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    zwlr_layer_shell_v1::Layer::Background,
                    "rounded-wallpaper".into(),
                    &qh,
                    (),
                );
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Bottom
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer.set_size(CANVAS as u32, CANVAS as u32);
                layer.set_exclusive_zone(0);
                surface.commit();
                let (w, h) = wait_for(&mut queue, &mut client, "a layer configure", |client| {
                    client.layer_size
                })?;
                let buffer = solid_buffer(&shm, &qh, w as i32, h as i32, color);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage_buffer(0, 0, w as i32, h as i32);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                surfaces.push(surface);
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

impl Fixture {
    fn with_radius(radius: i32) -> Self {
        let mut fixture = Harness::headless(
            Appearance {
                corner_radius: radius,
                ..Appearance::default()
            },
            CANVAS,
        );
        fixture.spawn(run_client);
        fixture
    }

    /// The same at an output scale other than 1.0, with an explicit ring
    /// thickness: the ticket's `corner_radius = 10, focus_ring_width = 4`
    /// shape for the fractional-scale separation test below.
    fn with_radius_at_scale(radius: i32, thickness: i32, scale: f64) -> Self {
        let mut fixture = Harness::headless_scaled(
            Appearance {
                corner_radius: radius,
                focus_ring_width: thickness,
                ..Appearance::default()
            },
            CANVAS,
            scale,
        );
        fixture.spawn(run_client);
        fixture
    }

    /// The live arrangement's first (and here only) placement.
    fn placement(&mut self) -> Rect {
        let placements = self.placements();
        assert_eq!(placements.len(), 1, "these tests map exactly one window");
        placements[0]
    }

    /// Every live placement in arrangement order.
    fn placements(&mut self) -> Vec<Rect> {
        self.settle();
        self.state
            .world
            .arrange()
            .placements
            .iter()
            .map(|placement| placement.rect)
            .collect()
    }
}

/// Full-frame color census: how many pixels of each byte value the frame
/// holds.
fn census(pixels: &[u8]) -> HashMap<[u8; 4], usize> {
    let mut counts = HashMap::new();
    for pixel in pixels.chunks_exact(4) {
        *counts
            .entry(pixel.try_into().expect("4 bytes"))
            .or_insert(0) += 1;
    }
    counts
}

// ---------------------------------------------------------------------------
// radius 0: the byte-identical baseline
// ---------------------------------------------------------------------------

/// `corner_radius = 0` renders exactly what the square path always has: one
/// window's-worth of red, four ring bars' worth of ring color, background
/// everywhere else -- pinned as exact counts, so any future change to the
/// default path moves a number here. Rendering twice is byte-identical, so
/// the census pins a stable frame rather than a lucky one.
#[test]
fn radius_zero_renders_the_square_baseline_exactly() {
    let mut fixture = Fixture::with_radius(0);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();

    let first = fixture.render();
    let second = fixture.render();
    assert_eq!(first, second, "two renders of a settled frame must agree");

    let bounds = Rect::new(0, 0, CANVAS, CANVAS);
    let bars = ring_rects(placement, Appearance::default().focus_ring_width, bounds);
    let bar_area: i32 = [bars.top, bars.bottom, bars.left, bars.right]
        .into_iter()
        .flatten()
        .map(|rect| rect.w * rect.h)
        .sum();
    // The top bar's middle pixel is ring color for sure (it never touches a
    // window corner or an output edge on this scene).
    let ring_sample = [
        placement.x + placement.w / 2,
        placement.y - Appearance::default().focus_ring_width / 2 - 1,
    ];
    let counts = census(&first);
    let ring_color: [u8; 4] = first[(ring_sample[1] * CANVAS + ring_sample[0]) as usize * 4..][..4]
        .try_into()
        .expect("in bounds");
    assert_eq!(
        counts.get(&WINDOW_BGRA).copied().unwrap_or(0),
        (placement.w * placement.h) as usize,
        "every window pixel must be red"
    );
    assert_eq!(
        counts.get(&ring_color).copied().unwrap_or(0),
        bar_area as usize,
        "every ring-bar pixel must be ring color"
    );
    assert_eq!(
        counts.values().sum::<usize>(),
        (CANVAS * CANVAS) as usize,
        "the census must cover the whole frame"
    );
    assert_eq!(
        counts.len(),
        3,
        "exactly three colors on a square frame: window, ring, background"
    );
}

// ---------------------------------------------------------------------------
// rounding: corners reveal what is below, the ring follows
// ---------------------------------------------------------------------------

/// The headline: with `corner_radius = 12`, the window's corner pixels are
/// background, the first pixel inside the staircase is window, and the ring
/// hugs the rounded shape -- no square remnants on the diagonal, ring on the
/// band. Checked on all four corners; the top-left is worked in detail.
#[test]
fn rounded_corners_reveal_the_background_and_the_ring_follows() {
    const RADIUS: i32 = 12;
    let mut fixture = Fixture::with_radius(RADIUS);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();

    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let (x, y, w, h) = (placement.x, placement.y, placement.w, placement.h);
    // The frame corner is background on this scene (the layout insets every
    // window by the gap); everything below measures against it.
    assert_ne!(
        bg, WINDOW_BGRA,
        "the test needs a non-window background sample"
    );

    // Red census: the full rect minus the four staircases -- the draw clip
    // tied to `cut_width` end to end.
    let cut: i32 = (0..RADIUS).map(|row| cut_width(RADIUS, row)).sum();
    let counts = census(&pixels);
    assert_eq!(
        counts.get(&WINDOW_BGRA).copied().unwrap_or(0),
        (w * h - 4 * cut) as usize,
        "the window must draw its rect minus exactly the four staircases"
    );

    // The ring color, sampled from the top straight run (it never touches a
    // corner or an output edge on this scene).
    let thickness = Appearance::default().focus_ring_width;
    let ring_sample = [x + w / 2, y - thickness / 2 - 1];
    let ring: [u8; 4] = pixels[(ring_sample[1] * CANVAS + ring_sample[0]) as usize * 4..][..4]
        .try_into()
        .expect("in bounds");
    assert_ne!(
        ring, WINDOW_BGRA,
        "the sampled ring pixel must not be window"
    );
    assert_ne!(ring, bg, "the sampled ring pixel must not be background");

    // Top-left in detail. Row 0: the window cuts `cut0` pixels; the ring
    // band hugs the staircase, so the cut zone is background outside the
    // outer arc and ring inside it -- and the first kept pixel is window.
    let cut0 = cut_width(RADIUS, 0);
    let cut1 = cut_width(RADIUS, 1);
    assert_pixel(&pixels, CANVAS, x, y, bg, "the extreme corner is cut");
    assert_pixel(
        &pixels,
        CANVAS,
        x + 1,
        y,
        bg,
        "row 0, outside the outer arc",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + cut0 - 1,
        y,
        ring,
        "row 0, the ring hugs the window staircase",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + cut0,
        y,
        WINDOW_BGRA,
        "row 0, first kept pixel",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x,
        y + 1,
        bg,
        "row 1, outside the outer arc",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + cut1 - 1,
        y + 1,
        ring,
        "row 1, the ring hugs the window staircase",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + cut1,
        y + 1,
        WINDOW_BGRA,
        "row 1, first kept pixel",
    );
    // The other three corners: extreme pixel cut, first inner pixel kept.
    assert_pixel(&pixels, CANVAS, x + w - 1, y, bg, "top-right corner");
    assert_pixel(
        &pixels,
        CANVAS,
        x + w - 1 - cut0,
        y,
        WINDOW_BGRA,
        "top-right kept",
    );
    assert_pixel(&pixels, CANVAS, x, y + h - 1, bg, "bottom-left corner");
    assert_pixel(
        &pixels,
        CANVAS,
        x + cut0,
        y + h - 1,
        WINDOW_BGRA,
        "bottom-left kept",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + w - 1,
        y + h - 1,
        bg,
        "bottom-right corner",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + w - 1 - cut0,
        y + h - 1,
        WINDOW_BGRA,
        "bottom-right kept",
    );
    // Center still window.
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        "the window middle",
    );

    // The ring's straight runs paint, the diagonal just outside the outer
    // corner is background (a square ring would paint there), and the band
    // itself sits exactly between the window staircase and the outer arc.
    assert_pixel(
        &pixels,
        CANVAS,
        ring_sample[0],
        ring_sample[1],
        ring,
        "top straight run",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x - 1,
        y - 1,
        bg,
        "no square corner remnant",
    );
    // Diagonal through the top-left corner's center (the two circles share
    // it: the window's radius 12 around (x + 12, y + 12), the ring's outer
    // radius 15 around the same point). k steps out along the diagonal:
    // k=8 lands inside the window, k=9 on the band, k=13 past the ring.
    let (cx, cy) = (x + RADIUS, y + RADIUS);
    assert_pixel(&pixels, CANVAS, cx - 9, cy - 9, ring, "on the band");
    assert_pixel(
        &pixels,
        CANVAS,
        cx - 8,
        cy - 8,
        WINDOW_BGRA,
        "inside the window",
    );
    assert_pixel(&pixels, CANVAS, cx - 13, cy - 13, bg, "outside the ring");
}

/// `corner_radius = 1` cuts nothing: the corner pixel's center is still
/// inside the unit circle, so the frame is the square baseline. Pins the
/// pixel-center rule end to end rather than just in `cut_width`.
#[test]
fn radius_one_leaves_the_corner_square() {
    let mut fixture = Fixture::with_radius(1);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS,
        placement.x,
        placement.y,
        WINDOW_BGRA,
        "a radius of 1 must not cut the corner pixel",
    );
}

/// A radius past half the window's smaller dimension clamps to a stadium:
/// no panic, no wrap, fully round ends. The extreme corners are background
/// and the middle of each edge is window.
#[test]
fn a_huge_radius_clamps_to_a_stadium() {
    let mut fixture = Fixture::with_radius(10_000);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let (x, y, w, h) = (placement.x, placement.y, placement.w, placement.h);
    let effective = (w.min(h) / 2).max(0);
    assert!(effective > 0, "the test needs a non-degenerate window");
    for (px, py, what) in [
        (x, y, "top-left corner of a stadium"),
        (x + w - 1, y, "top-right corner of a stadium"),
        (x, y + h - 1, "bottom-left corner of a stadium"),
        (x + w - 1, y + h - 1, "bottom-right corner of a stadium"),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, bg, what);
    }
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y,
        WINDOW_BGRA,
        "top edge middle survives the clamp",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        "the middle survives the clamp",
    );
    // And the physical radius the frame used really is the clamped one.
    assert_eq!(
        physical_radius(10_000, clip_rect(placement, 1.0), 1.0),
        effective,
        "physical_radius must clamp before anything draws"
    );
}

// ---------------------------------------------------------------------------
// gh #205: ring vs content alignment across output scales (separation test)
// ---------------------------------------------------------------------------

/// The separation test for gh #205 ("ring and window content corners do not
/// line up at fractional output scale"): the same fractional-aware client
/// (preferred scale + viewport destination, the `foot`/GTK shape) at scale
/// 1.0, 2.0 and 1.5 with the ticket's `corner_radius = 10,
/// focus_ring_width = 4`. The ticket's hypothesis says the integer scales
/// line up and only 1.5 is wrong (the rounding-route split); if an integer
/// leg fails too, the hypothesis is falsified and the failure -- not a
/// rounding fix -- is what needs explaining.
///
/// What "lines up" means, pinned per scale: the window draws its clip rect
/// minus exactly the four staircases (red census), every corner row's first
/// and last red pixel sits exactly on the staircase the ring's inner edge is
/// painted from, and the pixel just outside each is ring color.
#[test]
fn ring_and_content_align_at_scale_one() {
    check_ring_content_alignment(1.0, 10, 4);
}

#[test]
fn ring_and_content_align_at_scale_two() {
    check_ring_content_alignment(2.0, 10, 4);
}

#[test]
fn ring_and_content_align_at_scale_one_point_five() {
    check_ring_content_alignment(1.5, 10, 4);
}

/// The drift ticket's brute-force shape applied to this fix: the same
/// alignment at the other common fractional scales, not just the ticket's
/// 1.5. One test with a fresh fixture per scale rather than three tests,
/// since these are one assertion at different render targets.
#[test]
fn ring_and_content_align_across_fractional_scales() {
    for scale in [1.25, 1.75, 4.0 / 3.0] {
        check_ring_content_alignment(scale, 10, 4);
    }
}

/// `corner_radius = 1` at scale 1.5 is a physical radius of 2, which cuts
/// exactly the corner pixel per the pixel-center rule -- the scale-1.0 "cuts
/// nothing" pin does not transfer, and this pins what replaces it: the same
/// clip-minus-staircase census and row transitions at a tiny radius.
#[test]
fn radius_one_at_fractional_scale_cuts_one_pixel() {
    check_ring_content_alignment(1.5, 1, 4);
}

/// A radius past half the window's smaller dimension clamps to a stadium at
/// fractional scale too: no panic, no wrap, extreme corners background and
/// edge middles window.
#[test]
fn huge_radius_clamps_to_a_stadium_at_fractional_scale() {
    const SCALE: f64 = 1.5;
    let mut fixture = Fixture::with_radius_at_scale(10_000, 4, SCALE);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();
    let clip = clip_rect(placement, SCALE);
    let (x, y, w, h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);
    let effective = physical_radius(10_000, clip, SCALE);
    assert_eq!(
        effective,
        clip.size.w.min(clip.size.h) / 2,
        "the frame must use the clamped radius"
    );
    assert!(
        cut_width(effective, 0) > 0,
        "the test needs a radius that actually cuts the extreme corner"
    );
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    for (px, py, what) in [
        (x, y, "top-left corner of a stadium"),
        (x + w - 1, y, "top-right corner of a stadium"),
        (x, y + h - 1, "bottom-left corner of a stadium"),
        (x + w - 1, y + h - 1, "bottom-right corner of a stadium"),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, bg, what);
    }
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y,
        WINDOW_BGRA,
        "top edge middle survives the clamp",
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        "the middle survives the clamp",
    );
}

fn check_ring_content_alignment(scale: f64, configured: i32, thickness: i32) {
    let mut fixture = Fixture::with_radius_at_scale(configured, thickness, scale);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let pixels = fixture.render();
    assert_ring_hugs_content(&pixels, placement, scale, configured, thickness);
}

/// Everything "the ring and the content line up" means for one window whose
/// drawn rect (in logical pixels) is `drawn`, on a frame with nothing else in
/// it: the red census, every inner corner row on the staircase with ring
/// just outside, and the ring's *outer* edge a concentric arc -- radius plus
/// ring thickness, around the same corner -- in every row of the outer
/// corner band, the rows beside the window's own edge included.
fn assert_ring_hugs_content(
    pixels: &[u8],
    drawn: Rect,
    scale: f64,
    configured: i32,
    thickness: i32,
) {
    let clip = clip_rect(drawn, scale);
    let radius = physical_radius(configured, clip, scale);
    assert!(
        radius > 1,
        "scale {scale}: the test needs a radius that cuts"
    );
    let thickness_phys = (f64::from(thickness) * scale).round() as i32;
    assert!(
        thickness_phys > 0,
        "scale {scale}: the test needs a visible ring band"
    );
    let (x, y, w, h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);

    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    assert_ne!(
        bg, WINDOW_BGRA,
        "scale {scale}: the test needs a non-window background sample"
    );

    // Red census: the clip rect minus the four staircases. Position-free,
    // so a shifted or resized content rect fails here no matter which
    // corner it hides in.
    let cut: i32 = (0..radius).map(|row| cut_width(radius, row)).sum();
    let counts = census(pixels);
    assert_eq!(
        counts.get(&WINDOW_BGRA).copied().unwrap_or(0),
        (w * h - 4 * cut) as usize,
        "scale {scale}: the window must draw its clip rect minus exactly the four staircases"
    );

    // The ring color, sampled from the top straight run (it never touches a
    // corner on this scene).
    let ring_sample = [x + w / 2, y - thickness_phys / 2 - 1];
    let ring: [u8; 4] = pixels[(ring_sample[1] * CANVAS + ring_sample[0]) as usize * 4..][..4]
        .try_into()
        .expect("in bounds");
    assert_ne!(
        ring, WINDOW_BGRA,
        "scale {scale}: the sampled ring pixel must not be window"
    );
    assert_ne!(
        ring, bg,
        "scale {scale}: the sampled ring pixel must not be background"
    );
    assert_pixel(
        pixels,
        CANVAS,
        ring_sample[0],
        ring_sample[1],
        ring,
        &format!("scale {scale}: top straight run"),
    );

    // Every corner row: the first and last red pixels sit exactly on the
    // staircase, ring just outside. Top and bottom halves mirror.
    for row in 0..radius {
        let cut = cut_width(radius, row);
        for (py, what) in [(y + row, "top"), (y + h - 1 - row, "bottom")] {
            if cut > 0 {
                assert_pixel(
                    pixels,
                    CANVAS,
                    x + cut - 1,
                    py,
                    ring,
                    &format!("scale {scale}: {what} row {row}, ring hugs the staircase"),
                );
                assert_pixel(
                    pixels,
                    CANVAS,
                    x + w - cut,
                    py,
                    ring,
                    &format!("scale {scale}: {what} row {row}, ring hugs the staircase (right)"),
                );
            }
            assert_pixel(
                pixels,
                CANVAS,
                x + cut,
                py,
                WINDOW_BGRA,
                &format!("scale {scale}: {what} row {row}, first kept pixel"),
            );
            assert_pixel(
                pixels,
                CANVAS,
                x + w - 1 - cut,
                py,
                WINDOW_BGRA,
                &format!("scale {scale}: {what} row {row}, last kept pixel"),
            );
        }
    }
    // Center still window; the diagonal just outside the outer corner is
    // background (a square ring would paint there).
    assert_pixel(
        pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        &format!("scale {scale}: the window middle"),
    );
    // The diagonal just outside the outer corner: background once the
    // diagonal clears the outer arc (a square ring would paint there), ring
    // while it is still inside it (a tiny radius with a thick ring). The
    // gate is arithmetic on the two circles sharing the corner's center:
    // `(radius + 1) * sqrt(2) > radius + thickness_phys`, which
    // `radius > thickness_phys * 2` implies for every thickness this helper
    // runs with (all `<= 8`; the slack runs out past that, and a new caller
    // past it must re-derive rather than widen the gate).
    if radius > thickness_phys * 2 {
        assert_pixel(
            pixels,
            CANVAS,
            x - 1,
            y - 1,
            bg,
            &format!("scale {scale}: no square corner remnant"),
        );
    } else {
        assert_pixel(
            pixels,
            CANVAS,
            x - 1,
            y - 1,
            ring,
            &format!("scale {scale}: the outer arc still covers the diagonal"),
        );
    }

    // The outer edge: the ring's own outer rect (the painted canvas, where
    // the element draws it) rounded by `radius + thickness_phys` -- the
    // circle concentric with the window's corner. Every row of that corner
    // band, top and bottom, left and right: the cut pixels are background
    // and the first kept one is ring. The rows level with the window's own
    // edge are the ones a full-height side bar would square off.
    let (ring_loc, _, canvas) = ring_layout(drawn, thickness, scale);
    let origin: Point<i32, Physical> = ring_loc.to_i32_round();
    let radius_outer = radius + thickness_phys;
    assert!(
        radius_outer * 2 <= canvas.h.min(canvas.w),
        "scale {scale}: the test needs a ring whose outer corners do not meet"
    );
    for row in 0..radius_outer {
        let cut = cut_width(radius_outer, row);
        for (py, what) in [
            (origin.y + row, "top"),
            (origin.y + canvas.h - 1 - row, "bottom"),
        ] {
            let (left, right) = (origin.x + cut, origin.x + canvas.w - 1 - cut);
            if cut > 0 {
                assert_pixel(
                    pixels,
                    CANVAS,
                    left - 1,
                    py,
                    bg,
                    &format!("scale {scale}: {what} outer row {row}, cut on the left"),
                );
                assert_pixel(
                    pixels,
                    CANVAS,
                    right + 1,
                    py,
                    bg,
                    &format!("scale {scale}: {what} outer row {row}, cut on the right"),
                );
            }
            assert_pixel(
                pixels,
                CANVAS,
                left,
                py,
                ring,
                &format!("scale {scale}: {what} outer row {row}, first ring pixel on the left"),
            );
            assert_pixel(
                pixels,
                CANVAS,
                right,
                py,
                ring,
                &format!("scale {scale}: {what} outer row {row}, first ring pixel on the right"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// popups stay square
// ---------------------------------------------------------------------------

/// A no-grab popup over the parent's rounded corner keeps its own square
/// corners: every corner of the popup's bbox is popup color, while the
/// parent's uncovered rounded corner is background. Locates the popup by
/// color rather than hard-coding the positioner's arithmetic.
#[test]
fn popups_stay_square_over_rounded_corners() {
    const RADIUS: i32 = 12;
    const POPUP_W: i32 = 80;
    const POPUP_H: i32 = 60;
    let mut fixture = Fixture::with_radius(RADIUS);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture.run(Step::Popup {
        color: POPUP_BGRA,
        w: POPUP_W,
        h: POPUP_H,
    });
    let placement = fixture.placement();
    let pixels = fixture.render();

    // The popup's bbox, from its own pixels.
    let mut min_x = CANVAS;
    let mut max_x = 0;
    let mut min_y = CANVAS;
    let mut max_y = 0;
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let base = (y * CANVAS + x) as usize * 4;
            if pixels[base..base + 4] == POPUP_BGRA {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    assert!(
        find_color(&pixels, CANVAS, POPUP_BGRA).is_some(),
        "the popup must have drawn"
    );
    assert_eq!(max_x - min_x + 1, POPUP_W, "the popup draws its full width");
    assert_eq!(
        max_y - min_y + 1,
        POPUP_H,
        "the popup draws its full height"
    );
    // It really does cover the parent's top-left corner (else the squareness
    // assertions below would be vacuous).
    assert!(
        min_x <= placement.x && min_y <= placement.y,
        "the popup must reach the parent's top-left corner, at ({min_x}, {min_y})"
    );
    // Square: all four popup corners are popup color -- a clipped popup
    // would show background at its own corners.
    for (px, py, what) in [
        (min_x, min_y, "popup top-left stays square"),
        (max_x, min_y, "popup top-right stays square"),
        (min_x, max_y, "popup bottom-left stays square"),
        (max_x, max_y, "popup bottom-right stays square"),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, POPUP_BGRA, what);
    }
    // And the parent's own far corner still rounds.
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    assert_pixel(
        &pixels,
        CANVAS,
        placement.x + placement.w - 1,
        placement.y,
        bg,
        "the parent's uncovered corner still rounds under a popup",
    );
}

/// A background-layer wallpaper behind a rounded window shows through the
/// cut corners: the reachable below-window case in a settled layout (only
/// the background layer or the clear color -- tiling keeps settled windows
/// apart, but a surface can transiently overhang its placement mid-move, as
/// the bench's own overhang scene shows, so this relies on no
/// placements-never-overlap claim: the shrunk opaque region composites
/// whatever is genuinely below, repainted through damage, which self-heals
/// rather than going stale). The ring still draws over the wallpaper.
#[test]
fn a_wallpaper_shows_through_rounded_corners() {
    const RADIUS: i32 = 12;
    let mut fixture = Fixture::with_radius(RADIUS);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture.run(Step::Wallpaper {
        color: WALLPAPER_BGRA,
    });
    let placement = fixture.placement();
    let pixels = fixture.render();

    let (x, y, w, h) = (placement.x, placement.y, placement.w, placement.h);
    // The wallpaper covers the clear color everywhere it shows...
    assert_pixel(
        &pixels,
        CANVAS,
        0,
        0,
        WALLPAPER_BGRA,
        "the frame corner is wallpaper",
    );
    // ...including the window's cut corners.
    for (px, py, what) in [
        (x, y, "wallpaper through the top-left cut"),
        (x + w - 1, y, "wallpaper through the top-right cut"),
        (x, y + h - 1, "wallpaper through the bottom-left cut"),
        (
            x + w - 1,
            y + h - 1,
            "wallpaper through the bottom-right cut",
        ),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, WALLPAPER_BGRA, what);
    }
    // ...but the ring still draws on top of it.
    let thickness = Appearance::default().focus_ring_width;
    let ring: [u8; 4] = pixels[((y - thickness / 2 - 1) * CANVAS + x + w / 2) as usize * 4..][..4]
        .try_into()
        .expect("in bounds");
    assert_ne!(
        ring, WALLPAPER_BGRA,
        "the ring must draw over the wallpaper"
    );
    assert_ne!(
        ring, WINDOW_BGRA,
        "the sampled pixel must be ring, not window"
    );
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y - thickness / 2 - 1,
        ring,
        "the top straight run over wallpaper",
    );
    // And the window middle is untouched by any of it.
    assert_pixel(
        &pixels,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        "the window middle",
    );
}

/// Corners track what is below across frames: render the window alone (its
/// cut corners show the clear color), map a wallpaper behind it, re-render.
/// The corners must now be wallpaper, and the window middle still window.
///
/// Under `--headless`'s always-full redraw this passes even with a stale
/// opaque claim -- pixels come out right when every frame repaints
/// everything -- so this pins the end-to-end behavior while the unit test
/// below pins the opaque half. On a damage-tracked backend this exact
/// sequence is the stale-corner artifact (the tracker trusting a full-rect
/// opaque claim never repaints what is below the cuts), so both stay.
#[test]
fn corners_track_what_is_below_across_frames() {
    const RADIUS: i32 = 12;
    let mut fixture = Fixture::with_radius(RADIUS);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let placement = fixture.placement();
    let (x, y, w, h) = (placement.x, placement.y, placement.w, placement.h);

    let first = fixture.render();
    let bg: [u8; 4] = first[0..4].try_into().expect("canvas corner");
    assert_pixel(
        &first,
        CANVAS,
        x,
        y,
        bg,
        "the cut corner over the clear color, first frame",
    );

    fixture.run(Step::Wallpaper {
        color: WALLPAPER_BGRA,
    });
    let second = fixture.render();
    for (px, py, what) in [
        (x, y, "wallpaper through the top-left cut, second frame"),
        (
            x + w - 1,
            y,
            "wallpaper through the top-right cut, second frame",
        ),
        (
            x,
            y + h - 1,
            "wallpaper through the bottom-left cut, second frame",
        ),
        (
            x + w - 1,
            y + h - 1,
            "wallpaper through the bottom-right cut, second frame",
        ),
    ] {
        assert_pixel(&second, CANVAS, px, py, WALLPAPER_BGRA, what);
    }
    assert_pixel(
        &second,
        CANVAS,
        x + w / 2,
        y + h / 2,
        WINDOW_BGRA,
        "the window middle, second frame",
    );
}

/// Moving a painted window moves its ring: the strips are cached by
/// size/shape/color, and a reorder keeps all three fixed while the origins
/// move -- so the second render below must reuse the cached buffers at new
/// origins. Pre-fix it reuses the stored origins too, and the top/bottom
/// strips draw stale while the solid side bars (rebuilt from the live rect
/// every frame) move correctly.
#[test]
fn moving_a_painted_window_moves_its_ring() {
    const RADIUS: i32 = 12;
    let mut fixture = Fixture::with_radius(RADIUS);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture.run(Step::Window { color: WINDOW_BGRA });
    let thickness = Appearance::default().focus_ring_width;

    let before = fixture.state.world.arrange();
    assert_eq!(before.placements.len(), 2, "two windows, two columns");
    assert_eq!(
        (before.placements[0].rect.w, before.placements[0].rect.h),
        (before.placements[1].rect.w, before.placements[1].rect.h),
        "the swap must keep both windows' sizes fixed, so the ring cache hits"
    );
    let before_rects: Vec<(scoot_core::WindowId, Rect)> = before
        .placements
        .iter()
        .map(|placement| (placement.id, placement.rect))
        .collect();
    fixture.render(); // paints (and caches) both rings

    // Move the focused window toward the other column. Focus follows the
    // moved column, so its color -- and both windows' sizes -- stay fixed.
    let focused = before.focused.expect("a focused window");
    let focused_rect = before
        .placements
        .iter()
        .find(|placement| placement.id == focused)
        .expect("the focused placement")
        .rect;
    let other_rect = before
        .placements
        .iter()
        .find(|placement| placement.id != focused)
        .expect("the other placement")
        .rect;
    let direction = if focused_rect.x < other_rect.x {
        scoot_ipc::Horizontal::Right
    } else {
        scoot_ipc::Horizontal::Left
    };
    let response =
        fixture
            .state
            .handle_request(scoot_ipc::Request::Action(scoot_ipc::Action::MoveColumn {
                direction,
            }));
    assert!(
        matches!(response, scoot_ipc::Response::Ok { .. }),
        "move-column was not served"
    );
    fixture.settle();

    let after = fixture.state.world.arrange();
    assert_eq!(
        after.focused,
        Some(focused),
        "focus follows the moved column"
    );

    let pixels = fixture.render();
    // Every window that moved with its size fixed: its ring color, sampled
    // from its own left side bar (solid bars rebuild from the live rect
    // every frame, so this sample is correct with or without the bug), must
    // match its top straight run (a cached strip). Pre-fix the strips still
    // draw at the old origins, so the run pixel is whatever was already
    // there -- the other window's stale strip, or background -- and this
    // fails.
    let mut moved_any = false;
    for placement in &after.placements {
        let (rect, old) = (
            placement.rect,
            before_rects
                .iter()
                .find(|(id, _)| *id == placement.id)
                .expect("the same window from before the move")
                .1,
        );
        assert_eq!(
            (rect.w, rect.h),
            (old.w, old.h),
            "sizes stay fixed: the ring cache must hit"
        );
        if rect.x == old.x && rect.y == old.y {
            continue;
        }
        moved_any = true;
        let ring = pixel(
            &pixels,
            CANVAS,
            rect.x - thickness / 2 - 1,
            rect.y + rect.h / 2,
        );
        assert_pixel(
            &pixels,
            CANVAS,
            rect.x + rect.w / 2,
            rect.y - thickness / 2 - 1,
            ring,
            "the top run follows its window after a move",
        );
    }
    assert!(moved_any, "the move must actually relocate a window");
}

// ---------------------------------------------------------------------------
// Pure geometry: radius, staircase, clip and paint arithmetic
// ---------------------------------------------------------------------------

use super::*;
use smithay::utils::Rectangle;
use smithay::utils::{Logical, Physical, Point, Scale, Size};

// -- effective_radius ---------------------------------------------------

#[test]
fn a_zero_config_stays_zero() {
    assert_eq!(effective_radius(0, 800, 600), 0);
}

#[test]
fn a_negative_config_becomes_zero() {
    assert_eq!(effective_radius(-12, 800, 600), 0);
}

#[test]
fn a_small_radius_passes_through() {
    assert_eq!(effective_radius(12, 800, 600), 12);
}

#[test]
fn a_radius_past_half_the_height_clamps_to_it() {
    assert_eq!(effective_radius(400, 800, 600), 300);
}

#[test]
fn a_radius_past_half_the_width_clamps_to_it() {
    assert_eq!(effective_radius(500, 800, 1200), 400);
}

#[test]
fn an_absurd_radius_clamps_rather_than_overflowing() {
    assert_eq!(effective_radius(i32::MAX, 800, 600), 300);
}

#[test]
fn a_zero_size_window_rounds_nothing() {
    assert_eq!(effective_radius(12, 0, 600), 0);
    assert_eq!(effective_radius(12, 800, 0), 0);
    assert_eq!(effective_radius(12, 0, 0), 0);
}

// -- cut_width ----------------------------------------------------------

/// The independent oracle: pixel `(px, py)` of the top-left `radius`
/// square draws iff its center is inside the quarter circle around
/// `(radius, radius)`.
fn covers(px: i32, py: i32, radius: i32) -> bool {
    let r = f64::from(radius);
    let dx = f64::from(px) + 0.5 - r;
    let dy = f64::from(py) + 0.5 - r;
    dx * dx + dy * dy <= r * r
}

/// `cut_width` is exactly the per-pixel rule, on every radius that fits
/// in the test's time plus spot larges. There are no on-circle ties to
/// disagree on (see `cut_width`'s doc), so any mismatch here is a formula
/// bug, not float noise.
#[test]
fn cut_width_matches_the_pixel_rule_on_every_small_radius() {
    for radius in 0..=64 {
        for row in 0..radius {
            let cut = cut_width(radius, row);
            let expected = (0..radius)
                .take_while(|px| !covers(*px, row, radius))
                .count() as i32;
            assert_eq!(
                cut, expected,
                "radius {radius} row {row}: formula says {cut}, pixels say {expected}"
            );
            assert!(
                (0..=radius).contains(&cut),
                "radius {radius} row {row}: cut {cut} outside 0..=radius"
            );
        }
    }
}

#[test]
fn cut_width_matches_the_pixel_rule_on_large_radii() {
    for radius in [100, 256, 1000, 4096, 16383] {
        for row in [0, 1, radius / 2, radius - 1] {
            let cut = cut_width(radius, row);
            let expected = (0..radius)
                .take_while(|px| !covers(*px, row, radius))
                .count() as i32;
            assert_eq!(
                cut, expected,
                "radius {radius} row {row}: formula says {cut}, pixels say {expected}"
            );
        }
    }
}

#[test]
fn radius_one_cuts_nothing() {
    assert_eq!(cut_width(1, 0), 0);
}

#[test]
fn radius_two_cuts_one_pixel_off_the_first_row_only() {
    assert_eq!(cut_width(2, 0), 1);
    assert_eq!(cut_width(2, 1), 0);
}

// -- row_cut ------------------------------------------------------------

#[test]
fn middle_rows_cut_nothing() {
    assert_eq!(row_cut(12, 12, 100), 0);
    assert_eq!(row_cut(12, 50, 100), 0);
    assert_eq!(row_cut(12, 87, 100), 0);
}

#[test]
fn top_and_bottom_rows_mirror() {
    for radius in [2, 8, 12, 30] {
        let h = 4 * radius + 10;
        for row in 0..radius {
            assert_eq!(
                row_cut(radius, row, h),
                cut_width(radius, row),
                "top row {row} at radius {radius}"
            );
            assert_eq!(
                row_cut(radius, h - 1 - row, h),
                cut_width(radius, row),
                "bottom row {row} at radius {radius}"
            );
        }
    }
}

#[test]
fn a_zero_radius_cuts_no_row() {
    assert_eq!(row_cut(0, 0, 100), 0);
}

// -- clip_rect ----------------------------------------------------------

#[test]
fn clip_rect_is_the_identity_at_scale_one() {
    let placement = Rect::new(100, 80, 800, 600);
    assert_eq!(
        clip_rect(placement, 1.0),
        Rectangle::new((100, 80).into(), (800, 600).into())
    );
}

#[test]
fn clip_rect_doubles_cleanly_at_scale_two() {
    let placement = Rect::new(100, 80, 800, 600);
    assert_eq!(
        clip_rect(placement, 2.0),
        Rectangle::new((200, 160).into(), (1600, 1200).into())
    );
}

// -- physical_radius ----------------------------------------------------

#[test]
fn physical_radius_is_the_identity_at_scale_one() {
    let clip = Rectangle::new((0, 0).into(), (800, 600).into());
    assert_eq!(physical_radius(12, clip, 1.0), 12);
}

#[test]
fn physical_radius_scales_and_clamps_to_the_clip() {
    let clip = Rectangle::new((0, 0).into(), (800, 600).into());
    assert_eq!(physical_radius(12, clip, 2.0), 24);
    // 400 logical px at scale 2 is 800 physical -- past half the 600px
    // height, so the clip wins.
    assert_eq!(physical_radius(400, clip, 2.0), 300);
}

#[test]
fn physical_radius_saturates_rather_than_overflowing() {
    let clip = Rectangle::new((0, 0).into(), (800, 600).into());
    assert_eq!(physical_radius(i32::MAX, clip, 4.0), 300);
}

// -- Rounded::new clamping ----------------------------------------------

struct Stub;

#[test]
fn the_constructor_clamps_an_oversize_radius_to_the_clip() {
    let clip = Rectangle::new((0, 0).into(), (800, 600).into());
    let wrapped = Rounded::new(Stub, clip, 10_000);
    assert_eq!(wrapped.radius(), 300);
}

#[test]
fn the_constructor_keeps_a_fitting_radius() {
    let clip = Rectangle::new((0, 0).into(), (800, 600).into());
    let wrapped = Rounded::new(Stub, clip, 12);
    assert_eq!(wrapped.radius(), 12);
}

// -- ring_layout ----------------------------------------------------------

#[test]
fn ring_layout_is_exact_at_scale_one() {
    let (loc, logical, canvas) = ring_layout(Rect::new(100, 80, 200, 150), 4, 1.0);
    assert_eq!(loc, Point::<f64, Physical>::from((96.0, 76.0)));
    assert_eq!(logical, Size::<i32, Logical>::from((208, 158)));
    assert_eq!(canvas, Size::<i32, Physical>::from((208, 158)));
}

#[test]
fn ring_layout_doubles_cleanly_at_scale_two() {
    let (loc, logical, canvas) = ring_layout(Rect::new(100, 80, 200, 150), 4, 2.0);
    assert_eq!(loc, Point::<f64, Physical>::from((192.0, 152.0)));
    assert_eq!(logical, Size::<i32, Logical>::from((208, 158)));
    assert_eq!(canvas, Size::<i32, Physical>::from((416, 316)));
}

#[test]
fn ring_layout_at_a_fractional_scale_stays_sane() {
    // Best-effort territory (see the module doc): what matters is no
    // panic, positive canvas, and the canvas covering the scaled size.
    let (loc, logical, canvas) = ring_layout(Rect::new(100, 80, 200, 150), 4, 1.5);
    assert_eq!(logical, Size::<i32, Logical>::from((208, 158)));
    assert!(
        canvas.w > 0 && canvas.h > 0,
        "a degenerate canvas paints nothing"
    );
    assert_eq!((loc.x, loc.y), (144.0, 114.0));
    assert!(
        canvas.w >= (208.0f64 * 1.5) as i32 - 1 && canvas.w <= (208.0f64 * 1.5) as i32 + 1,
        "canvas {canvas:?} disagrees with the scaled size by more than rounding"
    );
}

// -- paint_ring -------------------------------------------------------------

const RING: [u8; 4] = [0x61, 0x59, 0x59, 0xff];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn painted(
    canvas: (i32, i32),
    inner: Rectangle<i32, Physical>,
    radius_inner: i32,
    radius_outer: i32,
) -> Vec<u8> {
    let mut pixels = vec![0; canvas.0 as usize * canvas.1 as usize * 4];
    let outer = Rectangle::new((0, 0).into(), (canvas.0, canvas.1).into());
    paint_ring(
        &mut pixels,
        &RingPaint {
            canvas: (canvas.0, canvas.1).into(),
            outer,
            radius_outer,
            inner,
            radius_inner,
        },
        RING,
    );
    pixels
}

fn at(pixels: &[u8], canvas_w: i32, x: i32, y: i32) -> [u8; 4] {
    let base = (y * canvas_w + x) as usize * 4;
    pixels[base..base + 4].try_into().expect("in bounds")
}

/// A square ring paints exactly the four bars: top/bottom span the full
/// width, left/right only the window height -- the same decomposition
/// `ring_rects` uses, so a zero inner radius agrees with the rect path.
#[test]
fn a_square_ring_paints_the_four_bars() {
    // Window 200x150 at offset (4, 4) in a 208x158 canvas, thickness 4.
    let inner = Rectangle::new((4, 4).into(), (200, 150).into());
    let pixels = painted((208, 158), inner, 0, 4);
    // Top bar, bottom bar, left bar, right bar.
    assert_eq!(at(&pixels, 208, 100, 0), RING);
    assert_eq!(at(&pixels, 208, 100, 157), RING);
    assert_eq!(at(&pixels, 208, 0, 80), RING);
    assert_eq!(at(&pixels, 208, 207, 80), RING);
    // Just inside the window: transparent. Just outside the ring: canvas
    // edge, but the bars cover the full width here.
    assert_eq!(at(&pixels, 208, 100, 80), CLEAR);
    assert_eq!(at(&pixels, 208, 100, 3), RING);
    assert_eq!(at(&pixels, 208, 3, 80), RING);
    // The window's own corners are transparent (square: the corner pixel
    // of the inner rect is window, not ring).
    assert_eq!(at(&pixels, 208, 4, 4), CLEAR);
}

/// A rounded ring cuts the outer corners and bites the inner ones: the
/// extreme corner pixel stays transparent, the straight runs paint.
#[test]
fn a_rounded_ring_cuts_both_corner_sets() {
    let inner = Rectangle::new((12, 12).into(), (200, 150).into());
    // thickness 12, inner radius 8, outer radius 20.
    let pixels = painted((224, 174), inner, 8, 20);
    // Straight runs still paint.
    assert_eq!(at(&pixels, 224, 112, 0), RING, "top straight run");
    assert_eq!(at(&pixels, 224, 0, 87), RING, "left straight run");
    // Extreme outer corner pixel: outside the outer circle.
    assert_eq!(at(&pixels, 224, 0, 0), CLEAR, "outer corner");
    // The cut is a staircase, not a square notch: row 0 cuts
    // `cut_width(20, 0)` pixels, so the first painted pixel is exactly
    // there rather than at the square's edge.
    let cut = cut_width(20, 0);
    assert_eq!(at(&pixels, 224, cut - 1, 0), CLEAR);
    assert_eq!(at(&pixels, 224, cut, 0), RING);
    // The window's own corner square is cut away, and that cut zone is
    // ring, not background: (12, 12) is window-relative (0, 0), outside
    // the window's staircase but inside the outer span.
    assert_eq!(at(&pixels, 224, 12, 12), RING, "cut zone paints");
    // ...but the ring hugs the staircase: the last pixel before the
    // window's cut paints, the first window pixel does not.
    let inner_cut = cut_width(8, 0);
    assert_eq!(at(&pixels, 224, 12 + inner_cut - 1, 12), RING);
    assert_eq!(at(&pixels, 224, 12 + inner_cut, 12), CLEAR);
}

/// The ring's inner edge is the window clip's outer edge: every painted
/// pixel adjacent to the inner rect sits exactly outside the window's own
/// staircase, so no gap and no overlap. This walks the whole inner
/// boundary rather than spot-checking corners.
#[test]
fn the_ring_inner_edge_matches_the_window_staircase() {
    let inner = Rectangle::new((12, 12).into(), (200, 150).into());
    let pixels = painted((224, 174), inner, 8, 20);
    // For each canvas row crossing the window, the leftmost painted pixel
    // must be exactly the window's cut on that row.
    for y in 12..162 {
        let yi = y - 12;
        let cut = row_cut(8, yi, 150);
        let first_ring = 12 + cut - 1;
        // Everything left of the window's cut on this row is ring (until
        // the outer cut, which is further out here).
        assert_eq!(
            at(&pixels, 224, first_ring, y),
            RING,
            "row {y}: pixel just outside the window staircase must be ring"
        );
        if cut < 200 {
            assert_eq!(
                at(&pixels, 224, 12 + cut, y),
                CLEAR,
                "row {y}: pixel just inside the window staircase must not be ring"
            );
        }
    }
}

// -- Rounded::opaque_regions --------------------------------------------------

/// A window element claiming its whole rect opaque -- what a client surface
/// reports before [`Rounded`] cuts it.
struct OpaqueStub {
    id: Id,
    geom: Rectangle<i32, Physical>,
}

impl Element for OpaqueStub {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, BufferSpace> {
        Rectangle::from_size((f64::from(self.geom.size.w), f64::from(self.geom.size.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geom
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::from_slice(&[Rectangle::new((0, 0).into(), self.geom.size)])
    }
}

/// The atomicity half the pixel tests cannot see: [`Rounded`] shrinks the
/// opaque region in the same construction that clips the draw, so the two
/// cannot drift apart. A draw-clip-only version (opaque untouched) renders
/// identical pixels under `--headless`'s always-full redraw -- every pixel
/// test above passes either way -- while on a damage-tracked backend the
/// tracker trusts the full-rect claim and never repaints what is below the
/// cut corners: persistent stale pixels. This test fails against that
/// neuter (the region stays the full rect); the two-frame pixel test above
/// pins the end-to-end behavior both share.
#[test]
fn the_opaque_region_shrinks_with_the_draw_clip() {
    const RADIUS: i32 = 12;
    const W: i32 = 200;
    const H: i32 = 150;
    let clip = Rectangle::<i32, Physical>::new((40, 30).into(), (W, H).into());
    let wrapped = Rounded::new(
        OpaqueStub {
            id: Id::new(),
            geom: clip,
        },
        clip,
        RADIUS,
    );
    let opaque: Vec<Rectangle<i32, Physical>> =
        wrapped.opaque_regions(1.0.into()).into_iter().collect();
    // Whole corner squares subtracted (the conservative superset of the
    // staircase -- see `corner_squares`), nothing else.
    let area: i32 = opaque.iter().map(|rect| rect.size.w * rect.size.h).sum();
    assert_eq!(
        area,
        W * H - 4 * RADIUS * RADIUS,
        "the opaque region must lose exactly the four corner squares"
    );
    let covers = |x: i32, y: i32| {
        opaque.iter().any(|rect| {
            x >= rect.loc.x
                && x < rect.loc.x + rect.size.w
                && y >= rect.loc.y
                && y < rect.loc.y + rect.size.h
        })
    };
    // Element-relative: the stub sits exactly on the clip, so the offset is
    // zero and these are output coordinates minus (40, 30).
    assert!(!covers(0, 0), "the cut corner must not be opaque");
    assert!(!covers(W - 1, 0), "the cut corner must not be opaque");
    assert!(!covers(0, H - 1), "the cut corner must not be opaque");
    assert!(!covers(W - 1, H - 1), "the cut corner must not be opaque");
    assert!(covers(W / 2, 0), "the top edge middle stays opaque");
    assert!(covers(W / 2, H / 2), "the window middle stays opaque");
}
