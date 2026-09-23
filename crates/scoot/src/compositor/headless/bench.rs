//! What one frame of [`State::render`](crate::compositor::State::render)
//! costs, printed for a human.
//!
//! CLAUDE.md asks for real before/after numbers whenever a change lands on a
//! hot path, and this is the hottest one. Asserts nothing -- a wall-clock
//! threshold in CI is a flake, not a guarantee -- so it is `#[ignore]`d like
//! `cursor/shapes/tests.rs`'s shape dump and `dmabuf/tests.rs`'s commit-sync
//! timing, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot render_frame_cost -- --ignored --nocapture
//! ```
//!
//! ## What the scenes measure, and what they deliberately do not
//!
//! Two scenes, both driving a real renderer into a real [`CANVAS`]-square
//! framebuffer -- `PixmanRenderer` by default, or `GlesRenderer` with
//! `SCOOT_TEST_RENDERER=gles` (see `test_support`), which is how the two are
//! compared on the same scenes rather than on two hand-written ones:
//!
//! - **empty desktop** -- no windows, no layer surfaces, no cursor: the
//!   damage tracker's clear plus `render()`'s own fixed per-frame work
//!   (output geometry, the layer-map lock, the frame-callback and cleanup
//!   pass). This is the *most sensitive* scene for a change to the frame
//!   loop's structure, because that fixed cost is the whole measurement
//!   rather than a sliver of it.
//! - **`RING_WINDOWS` windows** -- the same, plus a real `World::arrange`
//!   and the four focus-ring elements per placement that
//!   `Decorations::elements` builds from it, composited by pixman.
//!
//! The windows exist in [`scoot_core`] only: they are pushed straight into
//! the core with `WindowOpened` rather than mapped by a client, so the ring
//! is drawn for each of them while the window gather
//! (`render/elements.rs`'s `window_elements`) still finds no mapped window
//! to draw. That is deliberate -- a client surface's texture
//! import is *pixman's* cost, essentially constant against any change to how
//! the frame is assembled, and including it would bury exactly the fixed
//! overhead these numbers exist to watch. A change that claims to be free
//! has to be free here.
//!
//! [`resize_cost`] is the second measurement here, and it is about a
//! different path: [`State::resize_output`](crate::compositor::State), which
//! `--nested` reaches on every host configure that names a new size (a
//! browser window resized under webtop, a drag under a floating host). It
//! used to run once per process on that backend, so what it costs was not
//! worth knowing; now a drag can reach it per host frame, and it is.
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use scoot_core::{Event as CoreEvent, WindowId, WindowInfo};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::test_support::{Harness, test_renderer};

mod outputs;

/// The framebuffer each scene renders into. 800 square, matching the
/// headless-render benchmark recorded in
/// `docs/backlog/resolved/present-skip-eats-frame-damage-done.md` so the two
/// numbers are comparable.
const CANVAS: i32 = 800;

/// Frames per timed run.
const ROUNDS: u32 = 500;

/// Timed runs per scene.
const RUNS: u32 = 5;

/// Frames rendered before timing starts, so the first-frame costs (the
/// damage tracker's full-output redraw, the pixman image faulting in) do not
/// land in the average.
const WARMUP: u32 = 50;

/// How many windows the second scene arranges.
const RING_WINDOWS: u64 = 8;

/// Resizes per timed run in [`resize_cost`]. Far fewer than [`ROUNDS`]: one
/// resize rebuilds a whole render target, so this is milliseconds apiece
/// rather than microseconds.
const RESIZES: u32 = 40;

/// No client ever connects here, so the step/ack vocabulary is empty.
type Fixture = Harness<(), ()>;

/// A live compositor with a real headless backend and `windows` windows in
/// the core.
fn scene(windows: u64) -> Fixture {
    scene_outputs(windows, 1)
}

/// The same, with `outputs` side-by-side outputs. Windows open on the first
/// (see `shell.rs`'s `add_window`), so a two-output scene is one populated
/// strip plus empty ones -- the honest shape for the per-output scaling
/// question: what does each additional output's own frame cost when nothing
/// on it moves.
fn scene_outputs(windows: u64, outputs: i32) -> Fixture {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    for index in 2..=outputs {
        headless::add_output(
            &mut fixture.state,
            &format!("headless-{index}"),
            CANVAS,
            CANVAS,
        )
        .expect("another headless output");
    }
    for index in 0..windows {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(index + 1),
            info: WindowInfo {
                app_id: "bench".to_string(),
                title: "bench".to_string(),
                hints: Default::default(),
            },
            output: None,
            focus: true,
        });
    }
    fixture
}

/// Renders `rounds` frames back to back and hands back the total.
///
/// `request_render` before each one because `render()` returns immediately
/// on a clean screen -- which is the behaviour under measurement's own
/// fast path, not something being worked around: what is being timed is the
/// cost of a frame that really draws.
fn render_frames<S, A>(fixture: &mut Harness<S, A>, rounds: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..rounds {
        fixture.state.request_render();
        fixture.state.render();
    }
    started.elapsed()
}

/// What one [`State::resize_output`](crate::compositor::State) costs: a new
/// render target at the new size, a re-`arrange`d layer map, a re-published
/// output head and capture constraint set, and a full `apply()` of the core's
/// arrangement onto the windows.
///
/// Two sizes alternating, so every call really rebuilds rather than hitting
/// any same-size shortcut, and windows in the core so the `apply()` half is
/// not measured empty.
///
/// What it does **not** include, and cannot: the host-side `wl_shm` pool
/// `--nested` rebuilds alongside this (`nested::buffers::BufferPool` -- a
/// `memfd_create`, an `ftruncate`, an `mmap` and two `wl_buffer`s), which
/// needs a live host compositor to construct. Read this as the renderer-side
/// floor of a nested resize, not its total.
#[test]
#[ignore = "prints per-resize timings for a human; asserts nothing"]
fn resize_cost() {
    let renderer = test_renderer();
    let mut fixture = scene(RING_WINDOWS);
    // Two sizes, neither of them CANVAS, so the first timed call is no
    // cheaper or dearer than the rest.
    let sizes = [(CANVAS, CANVAS + 8), (CANVAS + 16, CANVAS)];
    let resize_rounds = |fixture: &mut Fixture, rounds: u32| {
        let started = Instant::now();
        for round in 0..rounds {
            let (width, height) = sizes[round as usize % sizes.len()];
            assert!(
                fixture.state.resize_output(width, height),
                "a resize to {width}x{height} failed"
            );
        }
        started.elapsed()
    };
    resize_rounds(&mut fixture, 4);
    let mut best = Duration::MAX;
    for run in 1..=RUNS {
        let total = resize_rounds(&mut fixture, RESIZES);
        best = best.min(total);
        println!(
            "resize [{renderer}], {RING_WINDOWS} windows: {total:?} total, {:?} per resize \
             ({RESIZES} resizes, ~{CANVAS}x{CANVAS}, run {run}/{RUNS})",
            total / RESIZES
        );
    }
    println!(
        "resize [{renderer}], {RING_WINDOWS} windows: BEST {:?} per resize ({RESIZES} \
         resizes, ~{CANVAS}x{CANVAS})",
        best / RESIZES
    );
}

#[test]
#[ignore = "prints per-frame render timings for a human; asserts nothing"]
fn render_frame_cost() {
    // Named on every line, because `SCOOT_TEST_RENDERER=gles` runs these
    // exact scenes through the other renderer and two sets of numbers that
    // don't say which is which are worse than none.
    let renderer = test_renderer();
    for (label, windows) in [("empty desktop", 0), ("8 windows", RING_WINDOWS)] {
        let mut fixture = scene(windows);
        render_frames(&mut fixture, WARMUP);
        // Five runs rather than one: this is a VM sharing a host's cores, so
        // a single number says nothing about whether a difference between
        // two builds is real. Compare the *minima* -- noise only ever adds
        // time, so the fastest run of each build is the one least polluted
        // by whatever else the host was doing.
        let mut best = Duration::MAX;
        for run in 1..=RUNS {
            let total = render_frames(&mut fixture, ROUNDS);
            best = best.min(total);
            println!(
                "render [{renderer}], {label}: {total:?} total, {:?} per frame ({ROUNDS} \
                 frames, {CANVAS}x{CANVAS}, run {run}/{RUNS})",
                total / ROUNDS
            );
        }
        println!(
            "render [{renderer}], {label}: BEST {:?} per frame ({ROUNDS} frames, \
         {CANVAS}x{CANVAS})",
            best / ROUNDS
        );
    }
}

/// What each additional output costs per frame: the same scenes as
/// [`render_frame_cost`] with a second (empty) output beside the first.
///
/// One populated strip plus one empty one (see [`scene_outputs`]): the
/// honest scaling shape for a compositor whose extra screens usually show
/// their own windows -- an empty second output is the floor, not the
/// ceiling, and a second populated one can only cost more compositing, never
/// more frame-loop structure. Alternates one- and two-output scenes within
/// each run so drift lands on both alike.
#[test]
#[ignore = "prints per-frame render timings for a human; asserts nothing"]
fn render_frame_cost_multi_output() {
    let renderer = test_renderer();
    for (label, windows) in [("empty desktop", 0), ("8 windows", RING_WINDOWS)] {
        let mut single = scene_outputs(windows, 1);
        let mut double = scene_outputs(windows, 2);
        render_frames(&mut single, WARMUP);
        render_frames(&mut double, WARMUP);
        let mut best_single = Duration::MAX;
        let mut best_double = Duration::MAX;
        for run in 1..=RUNS {
            let one = render_frames(&mut single, ROUNDS);
            let two = render_frames(&mut double, ROUNDS);
            best_single = best_single.min(one);
            best_double = best_double.min(two);
            println!(
                "render [{renderer}], {label}: one output {one:?} total, {:?} per frame; \
                 two outputs {two:?} total, {:?} per frame \
                 ({ROUNDS} frames, {CANVAS}x{CANVAS}, run {run}/{RUNS})",
                one / ROUNDS,
                two / ROUNDS,
            );
        }
        println!(
            "render [{renderer}], {label}: BEST one {:?} vs two {:?} per frame",
            best_single / ROUNDS,
            best_double / ROUNDS,
        );
    }
}

// ---------------------------------------------------------------------------
// Rounded corners: what `corner_radius` costs per frame
// ---------------------------------------------------------------------------

/// Frames per timed run in [`rounded_corners_cost`]. Fewer than [`ROUNDS`]:
/// every frame here composites real client textures at full damage, so one
/// frame costs milliseconds, not microseconds.
const ROUNDED_ROUNDS: u32 = 200;

/// Timed runs per tier. Tiers alternate within a run (square, rounded,
/// square, rounded, ...) so a thermal or noisy-neighbor drift lands on both
/// tiers alike instead of flattering whichever ran first.
const ROUNDED_RUNS: u32 = 12;

/// Frames after a radius switch before timing starts: the switch repaints
/// every ring buffer once, and that one-off must not land in the average.
const ROUNDED_WARMUP: u32 = 30;

/// The radius under test: a value people actually configure.
const ROUNDED_RADIUS: i32 = 12;

/// What the bench client can be told to do.
#[derive(Debug)]
enum RoundedStep {
    /// Map a toplevel (no buffer yet) and report its index.
    Map,
    /// Attach a `w` x `h` solid-`color` buffer to the `index`th surface and
    /// commit, so the drawn window is exactly that size at its placement.
    Attach {
        index: usize,
        w: i32,
        h: i32,
        color: [u8; 4],
    },
}

#[derive(Debug)]
enum RoundedAck {
    Done,
}

#[derive(Default)]
struct RoundedClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    serial: Option<u32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for RoundedClient {
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
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for RoundedClient {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for RoundedClient {
    fn event(
        client: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            client.serial = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(RoundedClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(RoundedClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(RoundedClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(RoundedClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(RoundedClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(RoundedClient: ignore xdg_toplevel::XdgToplevel);

fn run_rounded_client(
    stream: UnixStream,
    steps: Receiver<RoundedStep>,
    acks: Sender<RoundedAck>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = RoundedClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let mut surfaces: Vec<wl_surface::WlSurface> = Vec::new();

    while let Ok(step) = steps.recv() {
        match step {
            RoundedStep::Map => {
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_title("rounded-bench".into());
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                surfaces.push(surface);
                acks.send(RoundedAck::Done).map_err(|e| e.to_string())?;
            }
            RoundedStep::Attach { index, w, h, color } => {
                let stride = w * 4;
                let len = (stride * h) as usize;
                let fd = rustix::fs::memfd_create(
                    "scoot-rounded-bench",
                    rustix::fs::MemfdFlags::CLOEXEC,
                )
                .expect("a memfd");
                let mut file = std::fs::File::from(fd);
                let bytes: Vec<u8> = color.iter().copied().cycle().take(len).collect();
                file.write_all(&bytes).expect("a filled pool file");
                let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
                let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, &qh, ());
                pool.destroy();
                let surface = &surfaces[index];
                surface.attach(Some(&buffer), 0, 0);
                surface.damage_buffer(0, 0, w, h);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(RoundedAck::Done).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

type RoundedFixture = Harness<RoundedStep, RoundedAck>;

/// Maps `windows` clients and sizes each buffer to its live placement (or a
/// multiple of it for the overhang scene), so the drawn content and the
/// rounding clip agree exactly as they do in a production session.
fn rounded_scene(windows: usize, overhang: i32) -> RoundedFixture {
    let mut fixture = RoundedFixture::headless(Appearance::default(), CANVAS);
    fixture.spawn(run_rounded_client);
    for _ in 0..windows {
        fixture.run(RoundedStep::Map);
    }
    let arrangement = fixture.state.world.arrange();
    assert_eq!(
        arrangement.placements.len(),
        windows,
        "every mapped client must have a placement"
    );
    // Distinct opaque colors per window, so occlusion (or its absence) is
    // what the pixels say, not an assumption.
    let colors: [[u8; 4]; 3] = [
        [0x00, 0x00, 0xFF, 0xFF],
        [0x00, 0xFF, 0x00, 0xFF],
        [0xFF, 0x00, 0x00, 0xFF],
    ];
    for (index, placement) in arrangement.placements.iter().enumerate() {
        fixture.run(RoundedStep::Attach {
            index,
            w: placement.rect.w * overhang,
            h: placement.rect.h * overhang,
            color: colors[index % colors.len()],
        });
    }
    fixture
}

/// Times `rounds` frames at `radius`, after a warmup that absorbs the ring
/// repaints the switch caused.
fn time_rounded_tier(fixture: &mut RoundedFixture, radius: i32, rounds: u32) -> Duration {
    fixture.state.appearance.corner_radius = radius;
    render_frames(fixture, ROUNDED_WARMUP);
    render_frames(fixture, rounds)
}

/// What one frame costs with square vs rounded windows, on scenes with real
/// client content:
///
/// - **single**: one window. Nothing is behind it, so this isolates the
///   clip-filter overhead (damage rects minus the corner staircases, plus
///   the extra composite ops) from any opacity effect.
/// - **tiled3**: three windows tiling the output. The realistic session:
///   adjacent windows, gaps, one painted ring each.
/// - **overhang3**: the same three windows with double-size buffers, the
///   shape a shrink still in flight has (content bleeding past its
///   placement into the neighbor). Square, the bleed draws; rounded, the
///   clip cuts it to the placement -- so this scene also shows whether the
///   clip's removal of overlap outweighs its own cost.
///
/// Tiers alternate per run and the table reports min/median/max per tier:
/// this is a VM sharing a host's cores (see `render_frame_cost`), so a
/// single number per tier says nothing about whether a delta is real.
#[test]
#[ignore = "prints per-frame render timings for a human; asserts nothing"]
fn rounded_corners_cost() {
    let renderer = test_renderer();
    for (label, windows, overhang) in [("single", 1, 1), ("tiled3", 3, 1), ("overhang3", 3, 2)] {
        let mut fixture = rounded_scene(windows, overhang);
        let mut square: Vec<Duration> = Vec::with_capacity(ROUNDED_RUNS as usize);
        let mut rounded: Vec<Duration> = Vec::with_capacity(ROUNDED_RUNS as usize);
        // Paired deltas: each run's rounded batch minus its own square
        // batch, so a drift between runs cannot flatter either tier. The
        // median of these is the number to compare against the medians'
        // delta below; when they disagree, the paired one is honest.
        let mut paired: Vec<f64> = Vec::with_capacity(ROUNDED_RUNS as usize);
        for _ in 0..ROUNDED_RUNS {
            let square_total = time_rounded_tier(&mut fixture, 0, ROUNDED_ROUNDS);
            let rounded_total = time_rounded_tier(&mut fixture, ROUNDED_RADIUS, ROUNDED_ROUNDS);
            paired.push(rounded_total.as_nanos() as f64 / square_total.as_nanos() as f64 - 1.0);
            square.push(square_total);
            rounded.push(rounded_total);
        }
        let summarize = |samples: &mut Vec<Duration>| {
            samples.sort();
            let median = samples[samples.len() / 2] / ROUNDED_ROUNDS;
            (
                samples[0] / ROUNDED_ROUNDS,
                median,
                samples[samples.len() - 1] / ROUNDED_ROUNDS,
            )
        };
        let (square_min, square_med, square_max) = summarize(&mut square);
        let (round_min, round_med, round_max) = summarize(&mut rounded);
        let delta = round_med.as_nanos() as f64 / square_med.as_nanos() as f64 - 1.0;
        paired.sort_by(|a, b| a.total_cmp(b));
        let paired_med = paired[paired.len() / 2];
        let paired_min = paired[0];
        let paired_max = paired[paired.len() - 1];
        println!(
            "rounded [{renderer}], {label}: square min/median/max {square_min:?}/{square_med:?}/{square_max:?} \
             vs radius {ROUNDED_RADIUS} {round_min:?}/{round_med:?}/{round_max:?} \
             per frame ({ROUNDED_ROUNDS} frames x {ROUNDED_RUNS} alternating runs, {CANVAS}x{CANVAS}); \
             median delta {:+.1}%, paired min/median/max {:+.1}%/{:+.1}%/{:+.1}%",
            delta * 100.0,
            paired_min * 100.0,
            paired_med * 100.0,
            paired_max * 100.0
        );
    }
}
