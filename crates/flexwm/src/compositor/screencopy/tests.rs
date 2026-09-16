//! Tests for `ext-image-copy-capture-v1` / `ext-image-capture-source-v1`.
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`] with a real `headless` backend: the client binds the two
//! globals, allocates a real `wl_shm` buffer over a real memfd, sends a real
//! `capture`, and then **reads its own buffer back out of that memfd** and
//! compares it byte for byte with the framebuffer the compositor drew.
//!
//! That last part is the point. The claim under test is "the client was handed
//! the pixels that were on screen" -- and, for the locked case, "the client was
//! *not* handed the pixels that were on screen before the lock". Both are
//! claims about bytes in a client's buffer, and a test that asserted on a
//! compositor-side field would pass just as happily against a version that
//! copied the wrong frame.
//!
//! Reading the buffer back through `File::read_at` rather than mmapping it is
//! not a shortcut: a `wl_shm` pool *is* the memfd, so a read of the fd sees
//! exactly the bytes the compositor's shared mapping wrote, with no `unsafe`
//! in the test.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1,
    ext_image_copy_capture_session_v1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer these tests render into. Small on purpose: every capture
/// here is a whole-framebuffer copy that a test then compares byte for byte.
const CANVAS: i32 = 60;
/// A window's buffer, smaller than the canvas so a capture that picked up the
/// wrong frame differs in more than one pixel.
const WINDOW_BUFFER: i32 = 24;

/// What every client buffer here is pre-filled with before a capture.
///
/// Load-bearing in the offset and padding tests, which assert on the bytes the
/// compositor must *not* have written: a fill of zeroes would be
/// indistinguishable from a wrong write that happened to land on black. Picked
/// so no pixel this compositor draws in these tests can equal it -- the
/// background, the ring, the window and the lock surface are all listed above.
const SENTINEL: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

// Colors as the BGRA bytes a pixman `Argb8888` buffer holds them in.
const WINDOW_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const SECOND_WINDOW_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const LOCK_BGRA: [u8; 4] = [0xE0, 0x20, 0xE0, 0xFF];

/// A palette nothing else defaults to, with a **translucent** background on
/// purpose: it is what makes the `Xrgb8888` test able to tell "the alpha byte
/// was forced opaque" from "the framebuffer happened to be opaque anyway".
fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 2,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(1.0, 0.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 0.5),
        ..Appearance::default()
    }
}

/// What a frame ended as, from the client's side of the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Outcome {
    /// Neither `ready` nor `failed` has arrived -- the compositor is waiting.
    #[default]
    Waiting,
    Ready,
    Failed(u32),
}

/// The constraint batch a session announced, as the client saw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Constraints {
    width: u32,
    height: u32,
    /// Every `shm_format`, in the order it arrived -- the order is part of
    /// what this compositor promises (see the module doc's "Buffer formats").
    formats: Vec<u32>,
    /// How many `done` events have closed a batch. A re-advertisement after a
    /// resize is a second one.
    dones: u32,
    stopped: bool,
}

/// One instruction for the client thread.
enum Step {
    /// Map an `xdg_toplevel` with a solid buffer of this colour.
    MapWindow([u8; 4]),
    /// Create a capture source for the one output and a session on it, then
    /// round-trip until the constraint batch is closed by `done`.
    StartSession { paint_cursors: bool },
    /// Report the newest constraint batch without doing anything else.
    ReadConstraints,
    /// Allocate a `width`x`height` buffer in `format`, attach it, damage it,
    /// `capture`, and wait for `ready`/`failed`.
    Capture {
        width: i32,
        height: i32,
        format: wl_shm::Format,
    },
    /// Capture into the **second** of two buffers carved out of one pool, so
    /// the write has a non-zero `offset` within that pool.
    CaptureAtOffset,
    /// Capture into a buffer whose rows are `pad` bytes further apart than the
    /// pixels need, i.e. `stride > width * 4`.
    CaptureWithPaddedRows { pad: i32 },
    /// The same, but return as soon as the request is on the wire -- for the
    /// cases where "the compositor has *not* answered yet" is the assertion.
    CaptureWithoutWaiting,
    /// Report the outstanding frame's state, waiting a little in case it is
    /// about to arrive.
    PollFrame,
    /// With a frame already outstanding, create and capture a second one on
    /// the same session and report *its* outcome.
    CaptureAgain,
    /// `ext_image_copy_capture_session_v1.destroy`.
    DestroySession,
    /// Lock the session and give it a solid [`LOCK_BGRA`] lock surface.
    Lock,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Done,
    Constraints(Constraints),
    /// A frame's outcome, plus the client's own buffer read straight back out
    /// of its memfd.
    Frame(Outcome, Vec<u8>),
}

impl Ack {
    fn constraints(self) -> Constraints {
        match self {
            Ack::Constraints(constraints) => constraints,
            _ => panic!("expected a constraint batch"),
        }
    }

    fn frame(self) -> (Outcome, Vec<u8>) {
        match self {
            Ack::Frame(outcome, pixels) => (outcome, pixels),
            _ => panic!("expected a frame outcome"),
        }
    }
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn start() -> Self {
        let mut fixture = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }
}

/// How long a `PollFrame` gives the compositor before reporting
/// [`Outcome::Waiting`].
///
/// Long enough to be evidence rather than a race: at
/// [`FRAME_INTERVAL`](crate::compositor::headless::FRAME_INTERVAL) (16ms) this
/// is ~18 frame ticks, so "still waiting" means the compositor decided not to
/// answer, not that it had no chance to.
const POLL_PATIENCE: Duration = Duration::from_millis(300);

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn a_session_is_told_the_framebuffer_size_and_both_formats() {
    let mut fixture = Fixture::start();
    let constraints = fixture
        .run(Step::StartSession {
            paint_cursors: false,
        })
        .constraints();

    assert_eq!(
        (constraints.width, constraints.height),
        (CANVAS as u32, CANVAS as u32),
        "the buffer size must be the framebuffer a capture is read back out of"
    );
    assert_eq!(
        constraints.formats,
        vec![
            wl_shm::Format::Xrgb8888 as u32,
            wl_shm::Format::Argb8888 as u32
        ],
        "both formats, opaque one first -- see the module doc"
    );
    assert_eq!(constraints.dones, 1, "exactly one `done` closes a batch");
    assert!(!constraints.stopped, "the output source must be accepted");
}

#[test]
fn a_capture_hands_the_client_the_frame_the_compositor_drew() {
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();

    assert_eq!(outcome, Outcome::Ready, "the capture must succeed");
    let framebuffer = fixture.pixels();
    assert_eq!(
        captured, framebuffer,
        "an Argb8888 capture is the framebuffer byte for byte"
    );
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "the window that was on screen has to be in the capture"
    );
}

#[test]
fn an_xrgb_capture_is_opaque_even_over_a_translucent_background() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Xrgb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);

    let framebuffer = fixture.pixels();
    assert!(
        framebuffer.chunks_exact(4).any(|pixel| pixel[3] != 0xFF),
        "this test is only meaningful if the framebuffer really is translucent \
         somewhere -- `appearance()` gives the background an alpha below 1.0"
    );
    assert!(
        captured.chunks_exact(4).all(|pixel| pixel[3] == 0xFF),
        "every alpha byte of an Xrgb8888 capture must be forced opaque"
    );
    assert!(
        captured
            .chunks_exact(4)
            .zip(framebuffer.chunks_exact(4))
            .all(|(captured, drawn)| captured[..3] == drawn[..3]),
        "only the fourth byte may differ from the framebuffer"
    );
}

#[test]
fn an_xrgb_capture_is_opaque_at_a_width_the_wide_step_cannot_divide() {
    // `write_capture` forces the X byte four pixels at a time, so a width that
    // is not a multiple of four leaves a tail the wide step cannot reach. Every
    // other test here runs at `CANVAS` (60 = 15 whole steps, no tail), so
    // without this the tail loop could be deleted and nothing would notice --
    // and a `--tty` session takes its width from the connector, not from a
    // number this compositor gets to pick.
    let mut fixture = Fixture::start();
    let odd = CANVAS + 1;
    assert_eq!(odd % 4, 1, "the point of this test is a width with a tail");
    assert!(fixture.state.resize_output(odd, CANVAS));
    fixture.settle();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });

    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: odd,
            height: CANVAS,
            format: wl_shm::Format::Xrgb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);
    let opaque = captured
        .chunks_exact(4)
        .enumerate()
        .find(|(_, pixel)| pixel[3] != 0xFF);
    assert_eq!(
        opaque,
        None,
        "every pixel of an Xrgb8888 capture must be opaque, including the \
         last {} of each row, which the wide step cannot cover",
        odd % 4
    );
}

#[test]
fn a_capture_while_locked_shows_the_lock_screen_and_not_the_windows() {
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::Lock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the lock has to have taken for this test to mean anything"
    );

    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();

    assert_eq!(outcome, Outcome::Ready, "a locked session still captures");
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == LOCK_BGRA.as_slice()),
        "the lock surface has to be what the capture shows"
    );
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "no pixel of the window behind the lock screen may reach a capture"
    );
}

#[test]
fn a_capture_parked_before_a_lock_is_served_from_the_locked_frame() {
    // The race the lock guarantee actually has to survive: a capture requested
    // while the desktop is up, still parked when the session locks, and served
    // afterwards. Nothing here is special-cased for it -- it works because the
    // copy happens after `render()` has already decided what to draw -- so the
    // test exists to keep that reuse from being refactored away.
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, _) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);

    // Parked: the screen has not changed, so nothing serves it yet.
    fixture.run(Step::CaptureWithoutWaiting);
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Waiting);

    // Locking is a change, so the parked frame is served -- from the lock
    // screen's framebuffer, which is the only one that exists by then.
    fixture.run(Step::Lock);
    let (outcome, captured) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Ready);
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "a capture requested before the lock must still not carry the desktop"
    );
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == LOCK_BGRA.as_slice()),
        "and it must carry the lock screen that replaced it"
    );
}

#[test]
fn a_later_capture_waits_for_the_screen_to_change() {
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, _) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready, "the first capture never waits");

    // A second capture with nothing moving on screen: the protocol lets the
    // compositor wait indefinitely, and this one does.
    fixture.run(Step::CaptureWithoutWaiting);
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Waiting,
        "a repeat capture of an unchanged screen must not be copied again"
    );

    // ...and the moment something actually changes, it is served.
    fixture.run(Step::MapWindow(SECOND_WINDOW_BGRA));
    let (outcome, captured) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Ready,
        "the parked capture must be served once the screen changes"
    );
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == SECOND_WINDOW_BGRA.as_slice()),
        "and it must carry the *new* frame, not the one it waited on"
    );
}

#[test]
fn a_capture_lands_at_its_buffers_offset_inside_a_shared_pool() {
    // Every other test here allocates one buffer per pool at offset 0, which is
    // exactly the shape that cannot tell "honours `data.offset`" from "writes
    // at the start of the pool". A toolkit sub-allocating several buffers from
    // one pool is the ordinary case; drop the `data.offset` term from
    // `write_capture` and this is the only thing in the suite that notices.
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, pool) = fixture.run(Step::CaptureAtOffset).frame();
    assert_eq!(outcome, Outcome::Ready);

    let bytes = (CANVAS * CANVAS * 4) as usize;
    assert_eq!(pool.len(), bytes * 2, "the whole pool is read back");
    let (first, second) = pool.split_at(bytes);
    let framebuffer = fixture.pixels();

    assert_eq!(
        second, framebuffer,
        "the capture must land in the buffer it was attached to, at its own \
         offset in the pool"
    );
    assert!(
        first.chunks_exact(4).all(|pixel| pixel == SENTINEL),
        "the sibling buffer at offset 0 must be byte-for-byte untouched -- a \
         capture written at the pool's start instead of the buffer's would \
         have clobbered another of the client's own buffers"
    );
}

#[test]
fn a_capture_honours_a_buffer_whose_rows_are_padded() {
    // The other half of the same gap: every other buffer here has
    // `stride == width * 4`, so nothing notices if `write_capture` walked rows
    // by the pixel width instead of by the client's stride. `wl_shm` allows any
    // stride at least that wide.
    const PAD: i32 = 16;
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, pool) = fixture
        .run(Step::CaptureWithPaddedRows { pad: PAD })
        .frame();
    assert_eq!(outcome, Outcome::Ready);

    let row = (CANVAS * 4) as usize;
    let stride = row + PAD as usize;
    assert_eq!(pool.len(), stride * CANVAS as usize);
    let framebuffer = fixture.pixels();

    for y in 0..CANVAS as usize {
        let line = &pool[y * stride..][..stride];
        assert_eq!(
            &line[..row],
            &framebuffer[y * row..][..row],
            "row {y} has to land at its own stride, not packed against the last one"
        );
        assert!(
            line[row..].chunks_exact(4).all(|pixel| pixel == SENTINEL),
            "row {y}'s padding is not part of the image and must be left alone"
        );
    }
}

#[test]
fn a_buffer_smaller_than_the_output_is_refused() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, _) = fixture
        .run(Step::Capture {
            width: CANVAS / 2,
            height: CANVAS / 2,
            format: wl_shm::Format::Argb8888,
        })
        .frame();

    assert_eq!(
        outcome,
        Outcome::Failed(ext_image_copy_capture_frame_v1::FailureReason::BufferConstraints as u32),
        "a client whose buffer does not match has to be told to re-allocate"
    );
}

#[test]
fn a_resized_output_re_advertises_its_buffer_size() {
    let mut fixture = Fixture::start();
    let before = fixture
        .run(Step::StartSession {
            paint_cursors: false,
        })
        .constraints();
    assert_eq!(before.width, CANVAS as u32);

    assert!(
        fixture.state.resize_output(CANVAS * 2, CANVAS + 8),
        "the render target has to have been rebuilt"
    );
    fixture.settle();

    let after = fixture.run(Step::ReadConstraints).constraints();
    assert_eq!(
        (after.width, after.height),
        ((CANVAS * 2) as u32, (CANVAS + 8) as u32),
        "a session has to hear the new framebuffer size"
    );
    assert_eq!(
        after.dones, 2,
        "an update is a whole batch closed by its own `done`"
    );
}

#[test]
fn a_second_outstanding_capture_on_one_session_is_refused() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    // One successful capture first, so the *next* one is guaranteed to still
    // be parked when the second arrives: a session's first frame is served on
    // the next tick whatever the screen is doing, and a later one waits for a
    // change that nothing here makes. Without this the race decides the
    // result.
    let (outcome, _) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);

    fixture.run(Step::CaptureWithoutWaiting);
    let (outcome, _) = fixture.run(Step::CaptureAgain).frame();

    assert_eq!(
        outcome,
        Outcome::Failed(ext_image_copy_capture_frame_v1::FailureReason::Unknown as u32),
        "only one frame may be outstanding on a session at a time -- the pinned \
         Smithay rev never raises `duplicate_frame`, so this compositor bounds it"
    );
}

#[test]
fn a_destroyed_session_leaves_the_compositors_list() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (1, 1),
        "the session has to be tracked on both sides"
    );

    fixture.run(Step::DestroySession);
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (0, 0),
        "nothing upstream sweeps Smithay's own session list -- this compositor \
         has to, or every session a client ever made leaks"
    );
}

#[test]
fn a_client_that_disconnects_with_a_capture_outstanding_leaves_nothing_behind() {
    let mut fixture = Fixture::start();
    let other = fixture.spawn(run_client);

    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::CaptureWithoutWaiting);
    fixture.run_on(
        other,
        Step::StartSession {
            paint_cursors: false,
        },
    );
    assert_eq!(fixture.state.screencopy.session_count(), (2, 2));

    fixture.disconnect(0);
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (1, 1),
        "the departed client's session has to be gone from both lists"
    );

    // The survivor is still served, which is the actual claim: one client
    // leaving mid-capture must not disturb another's.
    let (outcome, captured) = fixture
        .run_on(
            other,
            Step::Capture {
                width: CANVAS,
                height: CANVAS,
                format: wl_shm::Format::Argb8888,
            },
        )
        .frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(captured, fixture.pixels());
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/// A client's own buffer, kept alongside the memfd it lives in so the test can
/// read back whatever the compositor wrote into it.
struct CaptureBuffer {
    buffer: wl_buffer::WlBuffer,
    file: std::fs::File,
    /// The whole **pool**'s length, which is what [`read_back`] reads: for the
    /// offset and padding tests, what the compositor left alone is as much the
    /// assertion as what it wrote.
    pool_len: usize,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    output: Option<wl_output::WlOutput>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    sources:
        Option<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1>,
    capture: Option<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,

    /// The newest constraint batch, and whether a `done` has closed it yet.
    constraints: Constraints,
    /// Set only between `buffer_size`/`shm_format` and the `done` that closes
    /// the batch, so a test never reads half an update.
    incoming: Constraints,
    /// The newest `xdg_surface.configure` serial *per window*, in the order
    /// the script mapped them.
    ///
    /// Per window and not one shared slot, which is not fussiness: mapping a
    /// second window re-lays-out the first, so the first's fresh configure
    /// would land in a shared slot and the second would then ack a serial that
    /// is not its own -- `xdg_wm_base.wrong_configure_serial`, i.e. a killed
    /// client, intermittently.
    window_serials: Vec<Option<u32>>,
    /// The outstanding frame's outcome, and -- separately -- the outcome of
    /// the extra frame [`Step::CaptureAgain`] creates while the first is still
    /// outstanding. Two fields rather than one so neither step can read the
    /// other's answer.
    frame: Outcome,
    second_frame: Outcome,
    locked: bool,
    lock_configure: Option<(u32, u32, u32)>,
}

/// Which of the two frame slots a `ext_image_copy_capture_frame_v1` reports
/// into.
#[derive(Clone, Copy, Debug)]
struct FrameSlot {
    extra: bool,
}

/// Which window an `xdg_surface.configure` belongs to, by the order the script
/// mapped them.
#[derive(Clone, Copy, Debug)]
struct SurfaceIndex(usize);

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let output = client.output.clone().ok_or("no wl_output")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let sources = client
        .sources
        .clone()
        .ok_or("no ext_output_image_capture_source_manager_v1 -- the global is missing")?;
    let capture = client
        .capture
        .clone()
        .ok_or("no ext_image_copy_capture_manager_v1 -- the global is missing")?;

    let mut session: Option<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1> = None;
    let mut frame: Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1> = None;
    let mut held: Option<CaptureBuffer> = None;
    let mut windows: Vec<wl_surface::WlSurface> = Vec::new();
    let mut locks: Vec<ext_session_lock_v1::ExtSessionLockV1> = Vec::new();
    // Buffers that share a pool with a capture buffer and are never attached to
    // anything -- the "sibling the compositor must not have touched" in
    // `Step::CaptureAtOffset`. Held so the `wl_buffer` objects stay alive for
    // the whole run, i.e. so the pool really does have two live buffers in it.
    let mut neighbours: Vec<CaptureBuffer> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let outcome = match step {
            Step::MapWindow(color) => {
                let surface = compositor.create_surface(&qh, ());
                let index = SurfaceIndex(client.window_serials.len());
                client.window_serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, index);
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_title("capture".into());
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "an xdg configure", |client| {
                    client.window_serials[index.0]
                })?;
                xdg.ack_configure(serial);
                let buffer = solid_buffer(
                    &shm,
                    &qh,
                    WINDOW_BUFFER,
                    WINDOW_BUFFER,
                    color,
                    wl_shm::Format::Argb8888,
                );
                surface.attach(Some(buffer.as_ref()), 0, 0);
                surface.damage_buffer(0, 0, WINDOW_BUFFER, WINDOW_BUFFER);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                windows.push(surface);
                Ack::Done
            }
            Step::StartSession { paint_cursors } => {
                let source = sources.create_source(&output, &qh, ());
                let options = if paint_cursors {
                    ext_image_copy_capture_manager_v1::Options::PaintCursors
                } else {
                    ext_image_copy_capture_manager_v1::Options::empty()
                };
                client.constraints = Constraints::default();
                client.incoming = Constraints::default();
                let new_session = capture.create_session(&source, options, &qh, ());
                // The source object is not needed again: the session holds the
                // compositor's own handle on it. Destroying it here is also a
                // small check that a session outlives its source.
                source.destroy();
                wait_for(&mut queue, &mut client, "a constraint batch", |client| {
                    (client.constraints.dones > 0 || client.constraints.stopped).then_some(())
                })?;
                session = Some(new_session);
                Ack::Constraints(client.constraints.clone())
            }
            Step::ReadConstraints => Ack::Constraints(client.constraints.clone()),
            Step::Capture {
                width,
                height,
                format,
            } => {
                let session = session.as_ref().ok_or("no session")?;
                start_frame(
                    &shm,
                    &qh,
                    session,
                    &mut frame,
                    &mut held,
                    &mut client,
                    width,
                    height,
                    format,
                )?;
                wait_for(&mut queue, &mut client, "a frame outcome", |client| {
                    (client.frame != Outcome::Waiting).then_some(())
                })?;
                Ack::Frame(client.frame, read_back(&held))
            }
            Step::CaptureAtOffset => {
                let session = session.as_ref().ok_or("no session")?;
                let bytes = (CANVAS * CANVAS * 4) as usize;
                let mut pool = Pool::new(&shm, &qh, bytes * 2, SENTINEL);
                // Two buffers, the capture going into the *second* -- what a
                // toolkit double-buffering out of one pool has.
                let first = pool.buffer(0, CANVAS, CANVAS, CANVAS * 4, wl_shm::Format::Argb8888);
                let second = pool.buffer(
                    bytes as i32,
                    CANVAS,
                    CANVAS,
                    CANVAS * 4,
                    wl_shm::Format::Argb8888,
                );
                pool.finish();
                neighbours.push(first);
                capture_into(
                    session,
                    &qh,
                    &mut frame,
                    &mut held,
                    &mut client,
                    second,
                    CANVAS,
                    CANVAS,
                );
                wait_for(&mut queue, &mut client, "a frame outcome", |client| {
                    (client.frame != Outcome::Waiting).then_some(())
                })?;
                Ack::Frame(client.frame, read_back(&held))
            }
            Step::CaptureWithPaddedRows { pad } => {
                let session = session.as_ref().ok_or("no session")?;
                let stride = CANVAS * 4 + pad;
                let mut pool = Pool::new(&shm, &qh, (stride * CANVAS) as usize, SENTINEL);
                let buffer = pool.buffer(0, CANVAS, CANVAS, stride, wl_shm::Format::Argb8888);
                pool.finish();
                capture_into(
                    session,
                    &qh,
                    &mut frame,
                    &mut held,
                    &mut client,
                    buffer,
                    CANVAS,
                    CANVAS,
                );
                wait_for(&mut queue, &mut client, "a frame outcome", |client| {
                    (client.frame != Outcome::Waiting).then_some(())
                })?;
                Ack::Frame(client.frame, read_back(&held))
            }
            Step::CaptureWithoutWaiting => {
                let session = session.as_ref().ok_or("no session")?;
                start_frame(
                    &shm,
                    &qh,
                    session,
                    &mut frame,
                    &mut held,
                    &mut client,
                    CANVAS,
                    CANVAS,
                    wl_shm::Format::Argb8888,
                )?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::CaptureAgain => {
                let session = session.as_ref().ok_or("no session")?;
                // Deliberately does *not* destroy the outstanding frame first
                // -- that is the thing under test.
                let second = session.create_frame(&qh, FrameSlot { extra: true });
                let buffer = solid_buffer(
                    &shm,
                    &qh,
                    CANVAS,
                    CANVAS,
                    [0, 0, 0, 0],
                    wl_shm::Format::Argb8888,
                );
                client.second_frame = Outcome::Waiting;
                second.attach_buffer(buffer.as_ref());
                second.damage_buffer(0, 0, CANVAS, CANVAS);
                second.capture();
                wait_for(
                    &mut queue,
                    &mut client,
                    "the second frame's outcome",
                    |client| (client.second_frame != Outcome::Waiting).then_some(()),
                )?;
                Ack::Frame(client.second_frame, Vec::new())
            }
            Step::PollFrame => {
                let deadline = Instant::now() + POLL_PATIENCE;
                while client.frame == Outcome::Waiting && Instant::now() < deadline {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    thread::sleep(Duration::from_millis(2));
                }
                Ack::Frame(client.frame, read_back(&held))
            }
            Step::DestroySession => {
                if let Some(session) = session.take() {
                    session.destroy();
                }
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Lock => {
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                let lock = manager.lock(&qh, ());
                let surface = compositor.create_surface(&qh, ());
                client.lock_configure = None;
                // No commit before the ack: `ext-session-lock-v1` sends the
                // first `configure` off `get_lock_surface` itself, and a
                // commit ahead of acking it is `commit_before_first_ack`.
                let role = lock.get_lock_surface(&surface, &output, &qh, ());
                let (serial, width, height) =
                    wait_for(&mut queue, &mut client, "a lock configure", |client| {
                        client.lock_configure
                    })?;
                role.ack_configure(serial);
                let buffer = solid_buffer(
                    &shm,
                    &qh,
                    width as i32,
                    height as i32,
                    LOCK_BGRA,
                    wl_shm::Format::Argb8888,
                );
                surface.attach(Some(buffer.as_ref()), 0, 0);
                surface.damage_buffer(0, 0, width as i32, height as i32);
                surface.commit();
                wait_for(&mut queue, &mut client, "the locked event", |client| {
                    client.locked.then_some(())
                })?;
                locks.push(lock);
                Ack::Done
            }
        };
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Allocates a buffer, attaches it to a fresh frame, and sends `capture`.
#[allow(clippy::too_many_arguments)]
fn start_frame(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    session: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    frame: &mut Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1>,
    held: &mut Option<CaptureBuffer>,
    client: &mut TestClient,
    width: i32,
    height: i32,
    format: wl_shm::Format,
) -> Result<(), String> {
    // A recognisable fill, so a capture that wrote nothing is not mistaken for
    // one that wrote black.
    let buffer = solid_buffer(shm, qh, width, height, SENTINEL, format);
    capture_into(session, qh, frame, held, client, buffer, width, height);
    Ok(())
}

/// Attaches an already-allocated buffer to a fresh frame and sends `capture`.
///
/// Split out of [`start_frame`] for the steps that need a buffer this file's
/// one-buffer-per-pool helper cannot make: one at a non-zero offset in its
/// pool, or one with padded rows.
#[allow(clippy::too_many_arguments)]
fn capture_into(
    session: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    qh: &QueueHandle<TestClient>,
    frame: &mut Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1>,
    held: &mut Option<CaptureBuffer>,
    client: &mut TestClient,
    buffer: CaptureBuffer,
    width: i32,
    height: i32,
) {
    if let Some(previous) = frame.take() {
        previous.destroy();
    }
    let new_frame = session.create_frame(qh, FrameSlot { extra: false });
    client.frame = Outcome::Waiting;
    new_frame.attach_buffer(buffer.as_ref());
    new_frame.damage_buffer(0, 0, width, height);
    new_frame.capture();
    *frame = Some(new_frame);
    *held = Some(buffer);
}

/// The client's **whole pool**, read straight back out of the memfd behind it.
///
/// The pool and not just the buffer, which matters for the two tests that put
/// more than the capture's own bytes in one: what they assert on is as much
/// what the compositor did *not* touch (a sibling buffer, a row's padding) as
/// what it did. For a pool holding exactly one tightly-strided buffer -- every
/// other test here -- the two are the same bytes.
fn read_back(held: &Option<CaptureBuffer>) -> Vec<u8> {
    let Some(held) = held else {
        return Vec::new();
    };
    let mut pixels = vec![0u8; held.pool_len];
    held.file
        .read_exact_at(&mut pixels, 0)
        .expect("the client's own pool is readable");
    pixels
}

/// A `width`x`height` `wl_buffer` filled with `color`, alone in its pool at
/// offset 0 with rows exactly `width * 4` bytes apart -- over a real memfd,
/// the same path any toolkit takes, and readable afterwards.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
    format: wl_shm::Format,
) -> CaptureBuffer {
    let stride = width * 4;
    let mut pool = Pool::new(shm, qh, (stride * height) as usize, color);
    let buffer = pool.buffer(0, width, height, stride, format);
    pool.finish();
    buffer
}

/// A `wl_shm` pool a test can carve more than one buffer out of, or carve one
/// padded buffer out of.
///
/// Exists because `solid_buffer`'s shape -- one buffer, offset 0, stride
/// exactly `width * 4` -- is precisely the shape that cannot exercise the two
/// client-supplied numbers `write_capture` has to honour. A toolkit
/// sub-allocating several buffers from one pool is the ordinary case, not an
/// exotic one.
struct Pool {
    pool: wl_shm_pool::WlShmPool,
    qh: QueueHandle<TestClient>,
    file: std::fs::File,
    len: usize,
}

impl Pool {
    /// A pool of `len` bytes, pre-filled with `fill` repeated, so anything the
    /// compositor writes is distinguishable from what was already there.
    fn new(shm: &wl_shm::WlShm, qh: &QueueHandle<TestClient>, len: usize, fill: [u8; 4]) -> Self {
        let fd = rustix::fs::memfd_create("flexwm-capture-test", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("a memfd");
        let mut file = std::fs::File::from(fd);
        let bytes: Vec<u8> = fill.iter().copied().cycle().take(len).collect();
        file.write_all(&bytes).expect("a filled pool file");
        let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
        Self {
            pool,
            qh: qh.clone(),
            file,
            len,
        }
    }

    /// One buffer inside it, at `offset` bytes from the pool's start.
    fn buffer(
        &mut self,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: wl_shm::Format,
    ) -> CaptureBuffer {
        CaptureBuffer {
            buffer: self
                .pool
                .create_buffer(offset, width, height, stride, format, &self.qh, ()),
            file: self.file.try_clone().expect("a second handle on the pool"),
            pool_len: self.len,
        }
    }

    /// The `wl_shm_pool` is no longer needed once its buffers exist -- the
    /// mapping outlives it, which is what every toolkit relies on.
    fn finish(self) {
        self.pool.destroy();
    }
}

impl CaptureBuffer {
    /// The `wl_buffer`, for the requests that take one.
    fn as_ref(&self) -> &wl_buffer::WlBuffer {
        &self.buffer
    }
}

// ---------------------------------------------------------------------------
// Client-side dispatch
// ---------------------------------------------------------------------------

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
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "wl_output" => client.output = Some(registry.bind(name, version.min(3), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "ext_output_image_capture_source_manager_v1" => {
                client.sources = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "ext_image_copy_capture_manager_v1" => {
                client.capture = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                client.incoming.width = width;
                client.incoming.height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                client.incoming.formats.push(match format {
                    WEnum::Value(format) => format as u32,
                    WEnum::Unknown(raw) => raw,
                });
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                let dones = client.constraints.dones + 1;
                client.constraints = std::mem::take(&mut client.incoming);
                client.constraints.dones = dones;
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                client.constraints.stopped = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, FrameSlot>
    for TestClient
{
    fn event(
        client: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        slot: &FrameSlot,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let outcome = match event {
            ext_image_copy_capture_frame_v1::Event::Ready => Outcome::Ready,
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                Outcome::Failed(match reason {
                    WEnum::Value(reason) => reason as u32,
                    WEnum::Unknown(raw) => raw,
                })
            }
            _ => return,
        };
        if slot.extra {
            client.second_frame = outcome;
        } else {
            client.frame = outcome;
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
            && let Some(slot) = client.window_serials.get_mut(index.0)
        {
            *slot = Some(serial);
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

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            client.locked = true;
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            client.lock_configure = Some((serial, width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(
    TestClient: ignore ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1
);
wayland_client::delegate_noop!(
    TestClient: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1
);
wayland_client::delegate_noop!(
    TestClient: ignore ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1
);
wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);
