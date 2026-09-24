//! Wire tests for explicit sync (see `drm_syncobj.rs`).
//!
//! Every test drives a real `wayland-client` connection through a real
//! [`State`](crate::compositor::State) with the global enabled on the test
//! machine's own `/dev/dri/renderD128`, and a client that creates a real DRM
//! timeline syncobj on its own open of that node, imports it, attaches real
//! dma-bufs (udmabuf, the provenance pixman imports -- see
//! `dmabuf/tests.rs`) with real acquire/release points, and signals and
//! queries those points with the real ioctls. So "the commit waited for the
//! point", "the release point was signalled", "the flood was cut off" are
//! claims about the kernel objects and the wire, not about bookkeeping.
//!
//! Where the machine has no render node, the node fails Smithay's probe, or
//! `/dev/udmabuf` is missing, a test prints why and passes without asserting
//! (the same trade `dmabuf/tests.rs` documents; the dev VM has all three).
//! The one test that needs none of them -- the global is absent by default --
//! always runs.
//!
//! Pinned to pixman: the subject is the protocol, not the renderer, and GLES
//! on software EGL refuses udmabuf imports (`test_support::test_renderer`'s
//! known seven), which would turn every test here into a client kill.

use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::reexports::drm::control::{Device as ControlDevice, syncobj};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::utils::DeviceFd;
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::wp::linux_drm_syncobj::v1::client::{
    wp_linux_drm_syncobj_manager_v1, wp_linux_drm_syncobj_surface_v1,
    wp_linux_drm_syncobj_timeline_v1,
};

use super::{MAX_ACQUIRE_WAITS_PER_CLIENT, MAX_TIMELINES_PER_CLIENT};
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// The headless framebuffer. Nothing here reads a pixel.
const CANVAS: i32 = 32;

/// The square dma-buf every commit attaches, in pixels.
const SIZE: i32 = 4;

/// The render node both halves of every test open.
const RENDER_NODE: &str = "/dev/dri/renderD128";

/// `wp_linux_drm_syncobj_manager_v1.error.invalid_timeline`.
const INVALID_TIMELINE: u32 = 1;

/// `wp_linux_drm_syncobj_surface_v1.error.no_release_point`.
const NO_RELEASE_POINT: u32 = 5;

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

enum Step {
    /// Report every global's interface name.
    Globals,
    /// Open the render node, create a timeline syncobj and import it. Also
    /// checks `/dev/udmabuf` works, so a test can skip on a machine that
    /// cannot make a dma-buf.
    Setup,
    /// Create a `wl_surface` with a syncobj surface on it; answers its
    /// index and protocol id.
    NewSurface,
    /// Attach a fresh dma-buf to surface `surface` with acquire and release
    /// points on the timeline, and commit. Answers the buffer's index.
    Commit {
        surface: usize,
        acquire: u64,
        release: u64,
    },
    /// The same, `count` times over on one surface, each commit with its own
    /// never-signalled acquire point: the flood.
    CommitMany { surface: usize, count: u32 },
    /// Attach a fresh dma-buf with only an acquire point -- malformed.
    CommitAcquireOnly { surface: usize, acquire: u64 },
    /// Signal `point` on the client's timeline from the client's own fd.
    Signal { point: u64 },
    /// Whether `point` on the client's timeline has signalled.
    Signalled { point: u64 },
    /// Destroy surface `surface` (its syncobj surface first, then the
    /// `wl_surface`).
    DestroySurface { surface: usize },
    /// Destroy the `wl_buffer` at `buffer`.
    DestroyBuffer { buffer: usize },
    /// Import the client's syncobj `count` more times, keeping every
    /// timeline object (`keep`) or destroying each right after its import.
    ImportTimelines { count: u32, keep: bool },
    /// The review's shape, `surfaces` times over: a fresh surface with a
    /// syncobj surface, two freshly imported timelines, an acquire point on
    /// one and a release point on the other, then both timeline objects
    /// destroyed. With `commit`, the points are committed with one shared
    /// dma-buf (acquire point already signalled, so nothing waits);
    /// without, they stay pending. Either way every point outlives its
    /// timeline object, as the protocol says it must, and keeps its fd.
    HoardPoints { surfaces: u32, commit: bool },
    /// Destroy every surface made so far (syncobj surface first).
    DestroyAllSurfaces,
    /// Import a plain memfd as a timeline, which the device cannot import.
    ImportBogusTimeline,
}

enum Ack {
    Globals(Vec<String>),
    Ready,
    /// This machine cannot make a render-node syncobj or a udmabuf.
    Unsupported(String),
    Surface {
        index: usize,
        id: u32,
    },
    Committed {
        buffer: usize,
    },
    Done,
    Signalled(bool),
}

type Fixture = Harness<Step, Ack>;

/// The two process-wide resources these tests share with other suites
/// under `cargo test` (one process, many threads), held for a whole test:
///
/// - `dispatch/tests.rs`'s fd-flood lock: the bound tests here push a
///   client past the fd-pressure graces, where the verdict reads the
///   process's fd table -- a neighbouring ~512-fd flood would turn a kill at
///   65 into one at 17 -- and the floods here hold a few hundred fds
///   themselves.
/// - `dmabuf/tests.rs`'s mapping lock: every commit here imports a udmabuf
///   that pixman maps, and that suite counts the process's dma-buf
///   mappings.
///
/// Taken in that order, the only test anywhere that takes both. Free under
/// nextest, which gives every test its own process.
struct Serialised {
    _flood: std::sync::MutexGuard<'static, ()>,
    _mappings: std::sync::MutexGuard<'static, ()>,
}

fn serialise() -> Serialised {
    Serialised {
        _flood: crate::compositor::dispatch::tests::hold_flood_lock(),
        _mappings: crate::compositor::dmabuf::tests::exclusive_mappings(),
    }
}

/// A compositor with the global enabled on the render node, or `None` (with
/// the reason printed) where this machine cannot offer it. The locks come
/// first in the tuple so a `let Some((_locks, fixture))` drops them *after*
/// the fixture, whose teardown is what unmaps the test's buffers.
fn start(test: &str) -> Option<(Serialised, Fixture)> {
    let locks = serialise();
    let Some(device) = open_render_node() else {
        skipped(test, "no usable render node");
        return None;
    };
    let device = DrmDeviceFd::new(DeviceFd::from(device));
    let mut fixture = Harness::headless_on(Appearance::default(), CANVAS, RendererKind::Pixman);
    let display = fixture.state.display_handle.clone();
    if !fixture
        .state
        .drm_syncobj
        .enable(&display, [("the render node".into(), Some(device))])
    {
        skipped(test, "the render node fails the syncobj-eventfd probe");
        return None;
    }
    fixture.spawn(run_client);
    match fixture.run(Step::Setup) {
        Ack::Ready => Some((locks, fixture)),
        Ack::Unsupported(why) => {
            skipped(test, &why);
            None
        }
        _ => panic!("unexpected answer to setup"),
    }
}

fn open_render_node() -> Option<OwnedFd> {
    rustix::fs::open(
        RENDER_NODE,
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()
}

/// Records that a test asserted nothing on this machine. Visible under
/// `--nocapture`; see `dmabuf/tests.rs`'s `skipped` for what that costs.
fn skipped(test: &str, why: &str) {
    eprintln!("{test}: skipped -- {why}");
}

impl Fixture {
    fn new_surface(&mut self) -> (usize, u32) {
        match self.run(Step::NewSurface) {
            Ack::Surface { index, id } => (index, id),
            _ => panic!("expected a surface"),
        }
    }

    fn commit(&mut self, surface: usize, acquire: u64, release: u64) -> usize {
        match self.run(Step::Commit {
            surface,
            acquire,
            release,
        }) {
            Ack::Committed { buffer } => buffer,
            _ => panic!("expected a commit"),
        }
    }

    fn done(&mut self, step: Step) {
        match self.run(step) {
            Ack::Done => {}
            _ => panic!("expected done"),
        }
    }

    fn signalled(&mut self, point: u64) -> bool {
        match self.run(Step::Signalled { point }) {
            Ack::Signalled(signalled) => signalled,
            _ => panic!("expected a signalled answer"),
        }
    }

    /// The server-side surface for a protocol id client 0 reported.
    fn server_surface(&self, id: u32) -> ServerSurface {
        self.client(0)
            .object_from_protocol_id(&self.state.display_handle, id)
            .expect("the surface exists server-side")
    }

    /// Whether the server-side surface has a buffer applied.
    fn has_buffer(&self, id: u32) -> bool {
        with_renderer_surface_state(&self.server_surface(id), |state| state.buffer().is_some())
            .unwrap_or(false)
    }

    fn waits_in_flight(&self) -> u32 {
        self.state.drm_syncobj.waits.in_flight()
    }
}

// ---------------------------------------------------------------------------
// Where the global exists
// ---------------------------------------------------------------------------

#[test]
fn the_global_is_not_offered_unless_enabled() {
    // Every session but the GPU scanout tier on a device that passes the
    // probe: headless here, and the same `DrmSyncobj::default()` nested and
    // dumb `--tty` start from.
    let mut fixture = Harness::headless_on(Appearance::default(), CANVAS, RendererKind::Pixman);
    fixture.spawn(run_client);
    let Ack::Globals(globals) = fixture.run(Step::Globals) else {
        panic!("expected globals");
    };
    assert!(globals.iter().any(|g| g == "wl_compositor"), "{globals:?}");
    assert!(
        !globals
            .iter()
            .any(|g| g == "wp_linux_drm_syncobj_manager_v1"),
        "explicit sync must not be offered where it is not honoured: {globals:?}"
    );
    assert!(!fixture.state.drm_syncobj.active());
}

#[test]
fn the_global_is_offered_once_enabled() {
    let Some((_locks, mut fixture)) = start("the_global_is_offered_once_enabled") else {
        return;
    };
    let Ack::Globals(globals) = fixture.run(Step::Globals) else {
        panic!("expected globals");
    };
    assert_eq!(
        globals
            .iter()
            .filter(|g| *g == "wp_linux_drm_syncobj_manager_v1")
            .count(),
        1,
        "{globals:?}"
    );
}

#[test]
fn enabling_twice_keeps_one_global() {
    let Some((_locks, mut fixture)) = start("enabling_twice_keeps_one_global") else {
        return;
    };
    let device = DrmDeviceFd::new(DeviceFd::from(open_render_node().expect("a render node")));
    let display = fixture.state.display_handle.clone();
    assert!(
        fixture
            .state
            .drm_syncobj
            .enable(&display, [("the render node".into(), Some(device))])
    );
    let Ack::Globals(globals) = fixture.run(Step::Globals) else {
        panic!("expected globals");
    };
    assert_eq!(
        globals
            .iter()
            .filter(|g| *g == "wp_linux_drm_syncobj_manager_v1")
            .count(),
        1
    );
}

#[test]
fn candidates_that_fail_or_cannot_open_fall_through_to_one_that_passes() {
    // The split render/display shape: the first device cannot import
    // timelines (here a non-DRM fd, which answers the probe's ioctl with
    // `ENOTTY` rather than `ENOENT`), the second could not be opened at all,
    // the third passes.
    let Some(device) = open_render_node() else {
        skipped(
            "candidates_that_fail_or_cannot_open_fall_through_to_one_that_passes",
            "no usable render node",
        );
        return;
    };
    let not_drm = rustix::fs::open(
        "/dev/null",
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .expect("/dev/null");
    let mut fixture: Fixture =
        Harness::headless_on(Appearance::default(), CANVAS, RendererKind::Pixman);
    let display = fixture.state.display_handle.clone();
    let offered = fixture.state.drm_syncobj.enable(
        &display,
        [
            (
                "not a drm device".into(),
                Some(DrmDeviceFd::new(DeviceFd::from(not_drm))),
            ),
            ("unopenable".into(), None),
            (
                "the render node".into(),
                Some(DrmDeviceFd::new(DeviceFd::from(device))),
            ),
        ],
    );
    if !offered {
        skipped(
            "candidates_that_fail_or_cannot_open_fall_through_to_one_that_passes",
            "the render node fails the syncobj-eventfd probe",
        );
        return;
    }
    assert!(fixture.state.drm_syncobj.active());
}

#[test]
fn no_passing_candidate_offers_nothing() {
    let not_drm = rustix::fs::open(
        "/dev/null",
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .expect("/dev/null");
    let mut fixture = Harness::headless_on(Appearance::default(), CANVAS, RendererKind::Pixman);
    let display = fixture.state.display_handle.clone();
    assert!(!fixture.state.drm_syncobj.enable(
        &display,
        [
            (
                "not a drm device".into(),
                Some(DrmDeviceFd::new(DeviceFd::from(not_drm)))
            ),
            ("unopenable".into(), None),
        ],
    ));
    assert!(!fixture.state.drm_syncobj.active());
    fixture.spawn(run_client);
    let Ack::Globals(globals) = fixture.run(Step::Globals) else {
        panic!("expected globals");
    };
    assert!(
        !globals
            .iter()
            .any(|g| g == "wp_linux_drm_syncobj_manager_v1")
    );
}

// ---------------------------------------------------------------------------
// Acquire points
// ---------------------------------------------------------------------------

#[test]
fn a_commit_waits_for_its_acquire_point() {
    let Some((_locks, mut fixture)) = start("a_commit_waits_for_its_acquire_point") else {
        return;
    };
    let (surface, id) = fixture.new_surface();
    fixture.commit(surface, 1, 2);
    fixture.settle();
    assert!(
        !fixture.has_buffer(id),
        "the buffer must not be applied before its acquire point signals"
    );
    assert_eq!(fixture.waits_in_flight(), 1);

    fixture.done(Step::Signal { point: 1 });
    fixture.settle();
    assert!(
        fixture.has_buffer(id),
        "the commit applies once the point signals"
    );
    assert_eq!(fixture.waits_in_flight(), 0);
    assert_eq!(
        fixture.state.drm_syncobj.explicit().len(),
        1,
        "the buffer is classified explicit for the release hold"
    );
}

#[test]
fn an_already_signalled_acquire_point_is_not_waited_on() {
    let Some((_locks, mut fixture)) = start("an_already_signalled_acquire_point_is_not_waited_on")
    else {
        return;
    };
    let (surface, id) = fixture.new_surface();
    fixture.done(Step::Signal { point: 5 });
    fixture.commit(surface, 5, 6);
    // No dispatch beyond the commit's own round trip: the commit applied
    // inside the request, with no eventfd and no wait record at all.
    assert!(fixture.has_buffer(id));
    assert_eq!(fixture.waits_in_flight(), 0);
    assert_eq!(
        fixture.state.drm_syncobj.waits.surfaces_tracked(),
        0,
        "the fast path creates no per-surface record"
    );
}

#[test]
fn later_commits_queue_behind_a_waiting_one_and_apply_in_order() {
    let Some((_locks, mut fixture)) =
        start("later_commits_queue_behind_a_waiting_one_and_apply_in_order")
    else {
        return;
    };
    // One timeline, so signalling a point signals every point below it: the
    // second commit's acquire point is the *lower* one, so it can signal
    // while the first's is still pending. Release points sit far above both,
    // and apart, so each can be read on its own.
    let (surface, id) = fixture.new_surface();
    let first = fixture.commit(surface, 5, 100);
    let second = fixture.commit(surface, 3, 200);
    fixture.settle();
    assert!(!fixture.has_buffer(id));
    assert_eq!(fixture.waits_in_flight(), 2);
    // The second commit's point signalling releases its wait but applies
    // nothing: its transaction is ordered behind the first on this surface.
    fixture.done(Step::Signal { point: 3 });
    fixture.settle();
    assert!(!fixture.has_buffer(id));
    assert_eq!(fixture.waits_in_flight(), 1);
    fixture.done(Step::Signal { point: 5 });
    fixture.settle();
    assert!(fixture.has_buffer(id));
    assert_eq!(fixture.waits_in_flight(), 0);
    // Both applied, in order: the first buffer was replaced by the second,
    // so its release point is signalled and the second's is not.
    assert!(fixture.signalled(100), "buffer {first} was replaced");
    assert!(
        !fixture.signalled(200),
        "buffer {second} is still on screen"
    );
}

#[test]
fn a_never_signalled_point_stalls_only_its_own_surface() {
    let Some((_locks, mut fixture)) = start("a_never_signalled_point_stalls_only_its_own_surface")
    else {
        return;
    };
    let (stuck, stuck_id) = fixture.new_surface();
    let (free, free_id) = fixture.new_surface();
    fixture.commit(stuck, 1000, 1001);
    fixture.done(Step::Signal { point: 7 });
    fixture.commit(free, 7, 8);
    fixture.settle();
    assert!(!fixture.has_buffer(stuck_id), "the stuck surface waits");
    assert!(
        fixture.has_buffer(free_id),
        "a sibling surface of the same client is not held behind it"
    );

    // A second client is served normally while the first is stuck.
    let other = fixture.spawn(run_client);
    let Ack::Globals(globals) = fixture.run_on(other, Step::Globals) else {
        panic!("expected globals");
    };
    assert!(!globals.is_empty());
    // And the compositor keeps rendering frames.
    fixture.render();
    assert_eq!(fixture.waits_in_flight(), 1);
}

#[test]
fn destroying_a_surface_mid_wait_removes_its_eventfd_source() {
    let Some((_locks, mut fixture)) =
        start("destroying_a_surface_mid_wait_removes_its_eventfd_source")
    else {
        return;
    };
    let (surface, id) = fixture.new_surface();
    fixture.commit(surface, 1, 2);
    fixture.settle();
    let server = fixture.server_surface(id);
    let tokens = fixture.state.drm_syncobj.waits.tokens_for(&server);
    assert_eq!(tokens.len(), 1);
    assert!(
        fixture.state.loop_handle.update(&tokens[0]).is_ok(),
        "the wait's source is registered"
    );

    fixture.done(Step::DestroySurface { surface });
    fixture.settle();
    assert_eq!(fixture.waits_in_flight(), 0);
    assert_eq!(fixture.state.drm_syncobj.waits.surfaces_tracked(), 0);
    assert!(
        fixture.state.loop_handle.update(&tokens[0]).is_err(),
        "the source (and its eventfd) went with the surface, not when the point signals"
    );
    // A point signalling afterwards finds nothing to wake and breaks nothing.
    fixture.done(Step::Signal { point: 1 });
    fixture.settle();
    let (again, again_id) = fixture.new_surface();
    fixture.done(Step::Signal { point: 9 });
    fixture.commit(again, 9, 10);
    assert!(fixture.has_buffer(again_id), "the client carries on");
}

#[test]
fn a_disconnect_mid_wait_removes_every_eventfd_source() {
    let Some((_locks, mut fixture)) = start("a_disconnect_mid_wait_removes_every_eventfd_source")
    else {
        return;
    };
    let (first, first_id) = fixture.new_surface();
    let (second, second_id) = fixture.new_surface();
    fixture.commit(first, 1, 2);
    fixture.commit(second, 3, 4);
    fixture.commit(second, 5, 6);
    fixture.settle();
    let mut tokens = fixture
        .state
        .drm_syncobj
        .waits
        .tokens_for(&fixture.server_surface(first_id));
    tokens.extend(
        fixture
            .state
            .drm_syncobj
            .waits
            .tokens_for(&fixture.server_surface(second_id)),
    );
    assert_eq!(tokens.len(), 3);
    assert_eq!(fixture.waits_in_flight(), 3);

    fixture.disconnect(0);
    assert_eq!(fixture.waits_in_flight(), 0);
    assert_eq!(fixture.state.drm_syncobj.waits.surfaces_tracked(), 0);
    for token in &tokens {
        assert!(fixture.state.loop_handle.update(token).is_err());
    }
}

#[test]
fn outstanding_waits_are_bounded_per_client() {
    let Some((_locks, mut fixture)) = start("outstanding_waits_are_bounded_per_client") else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    let error = fixture.run_expecting_disconnect(Step::CommitMany {
        surface,
        count: MAX_ACQUIRE_WAITS_PER_CLIENT + 1,
    });
    assert!(
        error.contains(&format!("code {NO_MEMORY}")) && error.contains("wl_display"),
        "the flood is cut off with wl_display.no_memory: {error}"
    );
    assert_eq!(
        fixture.waits_in_flight(),
        0,
        "the killed client's waits are all gone"
    );
    assert_eq!(fixture.state.drm_syncobj.waits.surfaces_tracked(), 0);

    // Everyone else is still served.
    let other = fixture.spawn(run_client);
    let Ack::Globals(globals) = fixture.run_on(other, Step::Globals) else {
        panic!("expected globals");
    };
    assert!(!globals.is_empty());
}

#[test]
fn exactly_the_bound_is_allowed() {
    let Some((_locks, mut fixture)) = start("exactly_the_bound_is_allowed") else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    match fixture.run(Step::CommitMany {
        surface,
        count: MAX_ACQUIRE_WAITS_PER_CLIENT,
    }) {
        Ack::Done => {}
        _ => panic!("expected done"),
    }
    fixture.settle();
    assert_eq!(fixture.waits_in_flight(), MAX_ACQUIRE_WAITS_PER_CLIENT);
}

#[test]
fn a_malformed_commit_gets_smithays_error_and_no_wait() {
    let Some((_locks, mut fixture)) = start("a_malformed_commit_gets_smithays_error_and_no_wait")
    else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    let error = fixture.run_expecting_disconnect(Step::CommitAcquireOnly {
        surface,
        acquire: 1,
    });
    assert!(
        error.contains(&format!("code {NO_RELEASE_POINT}"))
            && error.contains("wp_linux_drm_syncobj_surface_v1"),
        "{error}"
    );
    assert_eq!(
        fixture.waits_in_flight(),
        0,
        "no eventfd for a commit that is refused"
    );
}

// ---------------------------------------------------------------------------
// Release points
// ---------------------------------------------------------------------------

#[test]
fn a_release_point_is_signalled_when_its_buffer_is_replaced() {
    let Some((_locks, mut fixture)) =
        start("a_release_point_is_signalled_when_its_buffer_is_replaced")
    else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    fixture.done(Step::Signal { point: 1 });
    fixture.commit(surface, 1, 2);
    fixture.settle();
    assert!(!fixture.signalled(2), "not while the buffer is on screen");
    fixture.done(Step::Signal { point: 3 });
    fixture.commit(surface, 3, 4);
    fixture.settle();
    assert!(fixture.signalled(2), "signalled once replaced");
    assert!(!fixture.signalled(4));
}

#[test]
fn a_release_point_is_signalled_when_its_surface_is_destroyed() {
    let Some((_locks, mut fixture)) =
        start("a_release_point_is_signalled_when_its_surface_is_destroyed")
    else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    fixture.done(Step::Signal { point: 1 });
    fixture.commit(surface, 1, 2);
    fixture.settle();
    fixture.done(Step::DestroySurface { surface });
    fixture.settle();
    assert!(fixture.signalled(2));
}

#[test]
fn a_destroyed_buffer_leaves_the_explicit_set() {
    let Some((_locks, mut fixture)) = start("a_destroyed_buffer_leaves_the_explicit_set") else {
        return;
    };
    let (surface, _) = fixture.new_surface();
    fixture.done(Step::Signal { point: 1 });
    let first = fixture.commit(surface, 1, 2);
    fixture.done(Step::Signal { point: 3 });
    fixture.commit(surface, 3, 4);
    assert_eq!(fixture.state.drm_syncobj.explicit().len(), 2);
    fixture.done(Step::DestroyBuffer { buffer: first });
    fixture.settle();
    assert_eq!(fixture.state.drm_syncobj.explicit().len(), 1);
}

// ---------------------------------------------------------------------------
// Timelines
// ---------------------------------------------------------------------------

#[test]
fn live_timelines_are_bounded_per_client() {
    let Some((_locks, mut fixture)) = start("live_timelines_are_bounded_per_client") else {
        return;
    };
    // Setup imported one already.
    let error = fixture.run_expecting_disconnect(Step::ImportTimelines {
        count: MAX_TIMELINES_PER_CLIENT,
        keep: true,
    });
    assert!(
        error.contains(&format!("code {INVALID_TIMELINE}"))
            && error.contains("wp_linux_drm_syncobj_manager_v1"),
        "{error}"
    );
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        0,
        "the killed client's timelines are all released"
    );
}

/// The review's shape, pending: points set on a fresh surface outlive the
/// timeline objects they are on, and keep their fds open here. Counting
/// objects released all of them on destroy, and 440 surfaces held 927 fds
/// with nothing counted. The fds are what is counted now, so the import
/// that would hold the 129th is refused. Setup holds one, and each surface
/// two, so that is the second import of surface 64.
#[test]
fn pending_points_on_destroyed_timelines_count_against_the_bound() {
    let Some((_locks, mut fixture)) =
        start("pending_points_on_destroyed_timelines_count_against_the_bound")
    else {
        return;
    };
    let error = fixture.run_expecting_disconnect(Step::HoardPoints {
        surfaces: MAX_TIMELINES_PER_CLIENT,
        commit: false,
    });
    assert!(
        error.contains("after 64 surfaces")
            && error.contains(&format!("code {INVALID_TIMELINE}"))
            && error.contains("wp_linux_drm_syncobj_manager_v1")
            && error.contains("destroyed ones its sync points still reference"),
        "{error}"
    );
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        0,
        "the killed client's surfaces went, and every timeline fd with them"
    );
}

/// The same with the points committed: they ride in the surface's current
/// state and then in the renderer's `Buffer`, still after the timeline
/// objects are gone, and still count.
#[test]
fn committed_points_on_destroyed_timelines_count_against_the_bound() {
    let Some((_locks, mut fixture)) =
        start("committed_points_on_destroyed_timelines_count_against_the_bound")
    else {
        return;
    };
    let error = fixture.run_expecting_disconnect(Step::HoardPoints {
        surfaces: MAX_TIMELINES_PER_CLIENT,
        commit: true,
    });
    assert!(
        error.contains("after 64 surfaces") && error.contains(&format!("code {INVALID_TIMELINE}")),
        "{error}"
    );
    assert_eq!(fixture.state.client_fds.timelines_in_flight(), 0);
}

/// Timelines held only by points stop counting once the points go: here by
/// destroying the surfaces, after which the fds close and the next sweep
/// finds them closed. So a client that did this once can import a whole
/// cap's worth again.
#[test]
fn timelines_held_by_points_stop_counting_when_the_points_go() {
    let Some((_locks, mut fixture)) =
        start("timelines_held_by_points_stop_counting_when_the_points_go")
    else {
        return;
    };
    fixture.done(Step::HoardPoints {
        surfaces: 40,
        commit: false,
    });
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        81,
        "setup's, and two per surface: every one still open here"
    );
    fixture.done(Step::DestroyAllSurfaces);
    fixture.settle();
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        1,
        "just setup's"
    );
    fixture.done(Step::ImportTimelines {
        count: MAX_TIMELINES_PER_CLIENT - 1,
        keep: true,
    });
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        MAX_TIMELINES_PER_CLIENT
    );
}

/// An import Smithay refuses (the fd is no syncobj) was recorded before
/// delegation; it kills the client, and the record is left on a closed
/// number that no sweep counts.
#[test]
fn an_import_smithay_refuses_leaves_nothing_counted() {
    let Some((_locks, mut fixture)) = start("an_import_smithay_refuses_leaves_nothing_counted")
    else {
        return;
    };
    let error = fixture.run_expecting_disconnect(Step::ImportBogusTimeline);
    assert!(
        error.contains(&format!("code {INVALID_TIMELINE}"))
            && error.contains("failed to import syncobj timeline"),
        "{error}"
    );
    assert_eq!(fixture.state.client_fds.timelines_in_flight(), 0);
}

/// Destroyed timelines with no points on them close at once, so however many
/// a client imports and destroys, the sweep at the cap finds them gone.
#[test]
fn destroyed_timelines_do_not_count_against_the_bound() {
    let Some((_locks, mut fixture)) = start("destroyed_timelines_do_not_count_against_the_bound")
    else {
        return;
    };
    fixture.done(Step::ImportTimelines {
        count: MAX_TIMELINES_PER_CLIENT * 2,
        keep: false,
    });
    fixture.settle();
    assert_eq!(
        fixture.state.client_fds.timelines_in_flight(),
        1,
        "just setup's"
    );
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/// The client's own open of the render node, for the syncobj ioctls.
struct Node(OwnedFd);

impl AsFd for Node {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl smithay::reexports::drm::Device for Node {}
impl ControlDevice for Node {}

#[derive(Default)]
struct TestClient {
    /// Set by the `wl_callback` a [`sync`] is waiting on.
    synced: bool,
    globals: Vec<(String, u32, u32)>,
    compositor: Option<wl_compositor::WlCompositor>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    manager: Option<wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1>,
    node: Option<Node>,
    syncobj: Option<syncobj::Handle>,
    timeline: Option<wp_linux_drm_syncobj_timeline_v1::WpLinuxDrmSyncobjTimelineV1>,
    extra_timelines: Vec<wp_linux_drm_syncobj_timeline_v1::WpLinuxDrmSyncobjTimelineV1>,
    surfaces: Vec<(
        wl_surface::WlSurface,
        Option<wp_linux_drm_syncobj_surface_v1::WpLinuxDrmSyncobjSurfaceV1>,
    )>,
    /// Every buffer made, with the plane fd behind it.
    buffers: Vec<Option<(wl_buffer::WlBuffer, OwnedFd)>>,
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    for (name, interface, version) in client
        .globals
        .iter()
        .map(|(i, n, v)| (*n, i.as_str(), *v))
        .collect::<Vec<_>>()
    {
        match interface {
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), &qh, ()))
            }
            "zwp_linux_dmabuf_v1" => {
                client.dmabuf = Some(registry.bind(name, version.min(3), &qh, ()));
            }
            "wp_linux_drm_syncobj_manager_v1" => {
                client.manager = Some(registry.bind(name, 1, &qh, ()));
            }
            _ => {}
        }
    }
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::Globals => {
                Ack::Globals(client.globals.iter().map(|(i, _, _)| i.clone()).collect())
            }
            Step::Setup => setup(&mut client, &qh),
            Step::NewSurface => {
                let compositor = client.compositor.as_ref().ok_or("no wl_compositor")?;
                let manager = client.manager.as_ref().ok_or("no syncobj manager")?;
                let surface = compositor.create_surface(&qh, ());
                let sync = manager.get_surface(&surface, &qh, ());
                let id = surface.id().protocol_id();
                client.surfaces.push((surface, Some(sync)));
                Ack::Surface {
                    index: client.surfaces.len() - 1,
                    id,
                }
            }
            Step::Commit {
                surface,
                acquire,
                release,
            } => {
                let buffer = commit(&mut client, &qh, surface, Some(acquire), Some(release))?;
                Ack::Committed { buffer }
            }
            Step::CommitMany { surface, count } => {
                for n in 0..u64::from(count) {
                    let acquire = 1_000_000 + 2 * n;
                    commit(&mut client, &qh, surface, Some(acquire), Some(acquire + 1))?;
                    // A round trip per commit, not a flood of unread
                    // requests: a kill that lands while requests are still
                    // unread in the compositor's socket resets the
                    // connection, and the client never gets to read the
                    // error the test asserts on.
                    sync(&conn, &mut queue, &mut client)
                        .map_err(|error| format!("after {} commits: {error}", n + 1))?;
                }
                Ack::Done
            }
            Step::CommitAcquireOnly { surface, acquire } => {
                commit(&mut client, &qh, surface, Some(acquire), None)?;
                Ack::Done
            }
            Step::Signal { point } => {
                let node = client.node.as_ref().ok_or("no node")?;
                let handle = client.syncobj.ok_or("no syncobj")?;
                node.syncobj_timeline_signal(&[handle], &[point])
                    .map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Signalled { point } => {
                let node = client.node.as_ref().ok_or("no node")?;
                let handle = client.syncobj.ok_or("no syncobj")?;
                let mut points = [0];
                node.syncobj_timeline_query(&[handle], &mut points, false)
                    .map_err(|e| e.to_string())?;
                Ack::Signalled(points[0] >= point)
            }
            Step::DestroySurface { surface } => {
                let (wl, sync) = &mut client.surfaces[surface];
                if let Some(sync) = sync.take() {
                    sync.destroy();
                }
                wl.destroy();
                Ack::Done
            }
            Step::DestroyBuffer { buffer } => {
                if let Some((wl, _)) = client.buffers[buffer].take() {
                    wl.destroy();
                }
                Ack::Done
            }
            Step::ImportTimelines { count, keep } => {
                let handle = client.syncobj.ok_or("no syncobj")?;
                let manager = client.manager.clone().ok_or("no syncobj manager")?;
                for _ in 0..count {
                    let fd = client
                        .node
                        .as_ref()
                        .ok_or("no node")?
                        .syncobj_to_fd(handle, false)
                        .map_err(|e| e.to_string())?;
                    let timeline = manager.import_timeline(fd.as_fd(), &qh, ());
                    if keep {
                        client.extra_timelines.push(timeline);
                    } else {
                        timeline.destroy();
                    }
                    sync(&conn, &mut queue, &mut client)?;
                }
                Ack::Done
            }
            Step::HoardPoints { surfaces, commit } => {
                hoard_points(&conn, &mut queue, &mut client, surfaces, commit)?;
                Ack::Done
            }
            Step::ImportBogusTimeline => {
                let manager = client.manager.clone().ok_or("no syncobj manager")?;
                let memfd = rustix::fs::memfd_create(
                    "scoot-bogus-timeline",
                    rustix::fs::MemfdFlags::CLOEXEC,
                )
                .map_err(|e| e.to_string())?;
                client
                    .extra_timelines
                    .push(manager.import_timeline(memfd.as_fd(), &qh, ()));
                sync(&conn, &mut queue, &mut client)?;
                Ack::Done
            }
            Step::DestroyAllSurfaces => {
                for (wl, sync) in &mut client.surfaces {
                    if let Some(sync) = sync.take() {
                        sync.destroy();
                    }
                    wl.destroy();
                }
                client.surfaces.clear();
                Ack::Done
            }
        };
        sync(&conn, &mut queue, &mut client)?;
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// [`Step::HoardPoints`]: see its doc. A round trip per surface, for the
/// reason `Step::CommitMany` gives, and the error says how far it got.
fn hoard_points(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    surfaces: u32,
    commit: bool,
) -> Result<(), String> {
    let qh = queue.handle();
    let handle = client.syncobj.ok_or("no syncobj")?;
    let manager = client.manager.clone().ok_or("no syncobj manager")?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let node = client.node.as_ref().ok_or("no node")?;
    // Every timeline below is this one syncobj under a fresh fd, so point 1
    // is signalled for all of them at once.
    node.syncobj_timeline_signal(&[handle], &[1])
        .map_err(|e| e.to_string())?;
    let shared = if commit {
        let fd = dmabuf_fd().ok_or("no udmabuf")?;
        let params = client
            .dmabuf
            .as_ref()
            .ok_or("no dmabuf global")?
            .create_params(&qh, ());
        params.add(fd.as_fd(), 0, 0, SIZE as u32 * 4, 0, 0);
        let buffer = params.create_immed(
            SIZE,
            SIZE,
            u32::from_ne_bytes(*b"AR24"),
            zwp_linux_buffer_params_v1::Flags::empty(),
            &qh,
            (),
        );
        params.destroy();
        client.buffers.push(Some((buffer.clone(), fd)));
        Some(buffer)
    } else {
        None
    };
    for n in 0..surfaces {
        let import = |client: &TestClient| -> Result<_, String> {
            let fd = client
                .node
                .as_ref()
                .ok_or("no node")?
                .syncobj_to_fd(handle, false)
                .map_err(|e| e.to_string())?;
            Ok(manager.import_timeline(fd.as_fd(), &qh, ()))
        };
        let surface = compositor.create_surface(&qh, ());
        let syncobj_surface = manager.get_surface(&surface, &qh, ());
        let acquire = import(client)?;
        let release = import(client)?;
        syncobj_surface.set_acquire_point(&acquire, 0, 1);
        syncobj_surface.set_release_point(&release, 0, 2);
        if let Some(buffer) = &shared {
            surface.attach(Some(buffer), 0, 0);
            surface.damage_buffer(0, 0, SIZE, SIZE);
            surface.commit();
        }
        acquire.destroy();
        release.destroy();
        client.surfaces.push((surface, Some(syncobj_surface)));
        sync(conn, queue, client).map_err(|error| format!("after {} surfaces: {error}", n + 1))?;
    }
    Ok(())
}

/// A round trip that cannot lose a protocol error to `EPIPE`: the `sync`
/// goes out in the same write as whatever the step queued, rather than in a
/// second write after `EventQueue::roundtrip`'s own flush -- a kill landing
/// between the two closes the socket under the second write, and the error
/// waiting in the receive buffer is never read (the race
/// `popup_parent/tests` documents). A protocol error is reported through
/// [`why`], as its code and interface.
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => {
                // The socket is gone, but the error that closed it may be
                // sitting unread in the receive buffer: read it before
                // reporting the write failure.
                let _ = queue.dispatch_pending(client);
                if let Some(guard) = conn.prepare_read() {
                    let _ = guard.read();
                }
                let _ = queue.dispatch_pending(client);
                return Err(why(conn, error));
            }
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| why(conn, e))?;
    }
    Ok(())
}

/// Describes why the connection stopped: the protocol error that killed it
/// (code and interface, the part the kill tests assert on) when there is one,
/// whatever surfaced first otherwise -- a write after the kill fails with
/// `EPIPE` before the error event is read.
fn why(conn: &Connection, error: impl std::fmt::Display) -> String {
    match conn.protocol_error() {
        Some(error) => format!(
            "protocol error code {} on {}@{}: {}",
            error.code, error.object_interface, error.object_id, error.message
        ),
        None => error.to_string(),
    }
}

/// Opens the render node, makes the timeline syncobj, imports it, and checks
/// a udmabuf can be made.
fn setup(client: &mut TestClient, qh: &QueueHandle<TestClient>) -> Ack {
    let Some(fd) = open_render_node() else {
        return Ack::Unsupported("the client cannot open the render node".into());
    };
    let node = Node(fd);
    let handle = match node.create_syncobj(false) {
        Ok(handle) => handle,
        Err(error) => return Ack::Unsupported(format!("cannot create a syncobj: {error}")),
    };
    let exported = match node.syncobj_to_fd(handle, false) {
        Ok(fd) => fd,
        Err(error) => return Ack::Unsupported(format!("cannot export a syncobj: {error}")),
    };
    if dmabuf_fd().is_none() {
        return Ack::Unsupported("no usable /dev/udmabuf".into());
    }
    let (Some(manager), Some(_), Some(_)) = (&client.manager, &client.dmabuf, &client.compositor)
    else {
        return Ack::Unsupported("a global this suite needs is missing".into());
    };
    client.timeline = Some(manager.import_timeline(exported.as_fd(), qh, ()));
    client.node = Some(node);
    client.syncobj = Some(handle);
    Ack::Ready
}

/// Makes a fresh dma-buf `wl_buffer`, attaches it to `surface` with the
/// given points, and commits. Answers the buffer's index.
fn commit(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    surface: usize,
    acquire: Option<u64>,
    release: Option<u64>,
) -> Result<usize, String> {
    let fd = dmabuf_fd().ok_or("no udmabuf")?;
    let dmabuf = client.dmabuf.as_ref().ok_or("no dmabuf global")?;
    let params = dmabuf.create_params(qh, ());
    params.add(fd.as_fd(), 0, 0, SIZE as u32 * 4, 0, 0);
    let buffer = params.create_immed(
        SIZE,
        SIZE,
        u32::from_ne_bytes(*b"AR24"),
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    params.destroy();
    let timeline = client.timeline.as_ref().ok_or("no timeline")?;
    let (wl, sync) = &client.surfaces[surface];
    let sync = sync.as_ref().ok_or("the syncobj surface is gone")?;
    wl.attach(Some(&buffer), 0, 0);
    wl.damage_buffer(0, 0, SIZE, SIZE);
    if let Some(point) = acquire {
        sync.set_acquire_point(timeline, (point >> 32) as u32, point as u32);
    }
    if let Some(point) = release {
        sync.set_release_point(timeline, (point >> 32) as u32, point as u32);
    }
    wl.commit();
    client.buffers.push(Some((buffer, fd)));
    Ok(client.buffers.len() - 1)
}

/// One page of real dma-buf over `/dev/udmabuf`, or `None` where this
/// machine cannot make one. The same recipe as `dmabuf/tests.rs`.
fn dmabuf_fd() -> Option<OwnedFd> {
    /// `_IOW('u', 0x42, struct udmabuf_create)`.
    const UDMABUF_CREATE: u32 = 0x4018_7542;
    #[repr(C)]
    struct UdmabufCreate {
        memfd: u32,
        flags: u32,
        offset: u64,
        size: u64,
    }
    let memfd = rustix::fs::memfd_create(
        "scoot-syncobj-test",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .ok()?;
    let page = rustix::param::page_size() as u64;
    rustix::fs::ftruncate(&memfd, page).ok()?;
    rustix::fs::fcntl_add_seals(&memfd, rustix::fs::SealFlags::SHRINK).ok()?;
    let device = rustix::fs::open(
        "/dev/udmabuf",
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let create = UdmabufCreate {
        memfd: memfd.as_raw_fd() as u32,
        flags: 0x01,
        offset: 0,
        size: page,
    };
    // SAFETY: `device` is open for the whole call and `create` is a
    // correctly-shaped `struct udmabuf_create` the driver only reads. A
    // non-negative return is a fresh owned fd.
    let fd = unsafe {
        libc::ioctl(
            device.as_raw_fd(),
            UDMABUF_CREATE as _,
            std::ptr::addr_of!(create),
        )
    };
    // SAFETY: see above -- `fd` is a fresh fd nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            client.globals.push((interface, name, version));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        client.synced = true;
    }
}

/// Every other interface: no event this suite reads.
macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for TestClient {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_buffer::WlBuffer,
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
    wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1,
    wp_linux_drm_syncobj_surface_v1::WpLinuxDrmSyncobjSurfaceV1,
    wp_linux_drm_syncobj_timeline_v1::WpLinuxDrmSyncobjTimelineV1,
);
