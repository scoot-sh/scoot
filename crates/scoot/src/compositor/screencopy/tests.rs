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

use crate::cli::RendererKind;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::session_lock::LOCK_VBLANK_TIMEOUT;
use crate::compositor::test_support::{Harness, wait_for};

use super::MAX_FRAMES_PER_CLIENT;
use super::xrgb_needs_forcing;

mod cursor;
#[cfg(feature = "gpu-scanout")]
mod streaming;

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

/// The same palette as [`appearance`] but with an opaque background: the
/// shape that proves skipping the forcing pass changes no byte.
fn opaque_appearance() -> Appearance {
    Appearance {
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..appearance()
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
    /// Carve a buffer out of a pool and capture into it, with the three things
    /// [`solid_buffer`]'s one-buffer-per-pool shape cannot express:
    ///
    /// - `format` decides whether the opacity pass runs at all -- only
    ///   `Xrgb8888` takes it, which is why an `Argb8888` test of the offset and
    ///   stride terms says nothing about the *wide* pass that also uses them;
    /// - `sibling` puts another buffer ahead of the capture's, so `data.offset`
    ///   is non-zero and there is something adjacent that must stay untouched;
    /// - `pad` puts `pad` bytes between rows, so `data.stride` exceeds the
    ///   pixels and there is padding that must stay untouched too.
    CaptureFromPool {
        format: wl_shm::Format,
        sibling: bool,
        pad: i32,
    },
    /// The same, but return as soon as the request is on the wire -- for the
    /// cases where "the compositor has *not* answered yet" is the assertion.
    CaptureWithoutWaiting,
    /// Report the outstanding frame's state, waiting a little in case it is
    /// about to arrive.
    PollFrame,
    /// With a frame already outstanding, create and capture a second one on
    /// the same session and report *its* outcome.
    CaptureAgain,
    /// Create `count` frames on the session, deliberately never attaching,
    /// capturing or destroying any of them, and hold them all: the
    /// `create_frame`-loop leak shape. Never acknowledged -- a bounded
    /// compositor answers with `duplicate_frame`, which ends the client, so
    /// this step is only ever run via
    /// [`Harness::run_expecting_disconnect`](crate::compositor::test_support::Harness::run_expecting_disconnect).
    FloodFrames { count: u32 },
    /// Create `count` frames and keep them alive past this step, so a later
    /// `DestroySession` ends the session while its frames are still live --
    /// legal per the protocol, and what must not dangle the bookkeeping.
    HoldFrames { count: u32 },
    /// Create and destroy one frame per round, round-tripping both halves, so
    /// the bookkeeping has to drain as fast as it fills.
    CycleFrames { rounds: u32 },
    /// `ext_image_copy_capture_session_v1.destroy`.
    DestroySession,
    /// Lock the session and give it a solid [`LOCK_BGRA`] lock surface.
    Lock,
    /// The same, but return once the surface commit is on the wire without
    /// waiting for `locked` -- for the cases where the confirmation under
    /// test arrives later, through [`State::note_flip_completed`] or
    /// [`State::note_blank_timeout`], rather than with the render.
    LockNoWait,
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

    /// The same, but with an opaque background -- the configuration in which
    /// the forcing pass is skipped.
    fn start_opaque() -> Self {
        let mut fixture = Harness::headless(opaque_appearance(), CANVAS);
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
fn forcing_fires_below_opaque_and_only_there() {
    // The threshold `write_capture`'s conditional hangs off, pinned
    // directly: exact `< 1.0`, no epsilon. `254 / 255` is the nearest value
    // below opaque any `"#rrggbbaa"` config can spell, and the last line is
    // the largest `f32` below `1.0` -- both must still force, or the
    // guarantee dies for a background that reads back translucent.
    assert!(!xrgb_needs_forcing(1.0));
    assert!(xrgb_needs_forcing(0.5));
    assert!(xrgb_needs_forcing(0.0));
    assert!(xrgb_needs_forcing(254.0 / 255.0));
    assert!(xrgb_needs_forcing(f32::from_bits(0x3F7F_FFFF)));
}

#[test]
fn an_xrgb_capture_over_an_opaque_background_is_the_framebuffer_byte_for_byte() {
    // The byte-identity half of the conditional-forcing change: with an
    // opaque background the forcing pass is skipped, and the capture must be
    // exactly what a plain row copy hands over -- which is also exactly what
    // the old unconditional pass produced, since it OR'd `0xff` onto bytes
    // that already were `0xff`. A window is mapped so this pins content, not
    // just a blank background.
    let mut fixture = Fixture::start_opaque();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
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
        framebuffer.chunks_exact(4).all(|pixel| pixel[3] == 0xFF),
        "the premise this test pins: over an opaque background no framebuffer \
         pixel carries alpha -- windows composite source-over onto an opaque \
         destination, so only the clear color could have put any there"
    );
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "the window that was on screen has to be in the capture"
    );
    assert_eq!(
        captured, framebuffer,
        "skipping the forcing pass over an opaque background must change no \
         byte: an Xrgb8888 capture is the framebuffer as-is"
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

// ---------------------------------------------------------------------------
// A parked capture across a vblank-deferred lock confirmation (PR #84)
// ---------------------------------------------------------------------------
//
// PR #84 moved lock confirmation out of `render()` and onto the DRM vblank
// under `--tty`. The tick that rendered the blank then drops its timer -- a
// parked screencopy frame is none of `frame_tick`'s re-arm conditions -- and
// neither confirm path re-armed it, so a capture parked across the wait was
// never delivered unless some later commit happened to restart the ticker.
//
// These tests drive that shape without DRM hardware: the render target is
// taken away so the lock's own render draws nothing and confirms nothing
// (the headless stand-in for `--tty` deferring confirmation to the flip's
// vblank), the wait `render()` would have recorded is recorded by hand, and
// the blank tick consuming the dirty flag -- which drops the timer -- is a
// `needs_render = false` plus a real tick.
//
// Two things a headless harness cannot reproduce, and how these tests stay
// honest about both:
//
// - The blank is never drawn here (any successful locked render would
//   confirm headless-style and clear `pending`), so the parked frame is not
//   even due at confirm time and the backdrop stays stale. On `--tty` the
//   blank tick both paints the backdrop and bumps `frame_serial` (making the
//   parked frame due), and the re-armed tick delivers straight from that
//   framebuffer -- its `render()` early-outs on the consumed dirty flag, and
//   no post-confirm render is needed. Here the delivering render is
//   `refresh_lock_state`'s catch-up (`session_lock.rs`): the stale backdrop
//   re-arms on the first display dispatch after the restore, and that tick
//   draws the lock screen the capture is then served from.
// - Because of that second re-arm, delivery alone cannot discriminate the
//   fix headless: it happens pre-fix too. What fails pre-fix is the pin each
//   test takes first -- `timer_armed` immediately after the confirm call,
//   with zero dispatches in between, so nothing but the confirm path itself
//   could have armed it. That pin is the regression test; the delivery and
//   pixel assertions after it prove the re-armed ticker actually serves a
//   parked frame, with post-blank content, and then goes idle again.

/// Parks a capture, locks with confirmation deferred the way `--tty` defers
/// it, and leaves the compositor exactly where the blank tick left it: the
/// timer dropped, the wait recorded, the render target back in place.
///
/// Returns the `Instant` the wait was armed at, for the fallback-confirmation
/// test.
fn park_across_deferred_lock(fixture: &mut Fixture) -> Instant {
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    // One capture delivered, so the next one parks on a static screen: the
    // protocol lets the compositor wait for a change, and this one does.
    let (outcome, _) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);
    fixture.run(Step::CaptureWithoutWaiting);
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Waiting);

    // No render target: the lock's own render draws nothing and -- headless
    // confirming on render -- confirms nothing either. `pending` survives,
    // the way it survives a `--tty` render that defers to the vblank.
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.run(Step::LockNoWait);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the lock has to have taken for this test to mean anything"
    );
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "with no frame drawn, the blanked frame is still outstanding"
    );
    // What `--tty`'s render records once its blanked frame presents as flip
    // 7. The fallback timer the real path arms alongside is deliberately not
    // armed here: the test drives the deadline by hand.
    let t0 = Instant::now();
    let _ = fixture.state.session_lock.await_vblank(Some(7), t0);
    // The blank tick consuming the dirty flag: nothing in the re-arm set is
    // left, so a real tick drops the timer -- the mechanism under test.
    fixture.state.needs_render = false;
    fixture.tick(Duration::from_millis(50));
    assert!(
        !fixture.state.timer_armed,
        "the blank tick must have dropped the timer: a parked capture is \
         none of frame_tick's re-arm conditions"
    );
    fixture.state.put_primary_backend(backend);
    t0
}

#[test]
fn a_parked_capture_is_delivered_once_a_vblank_confirms_the_lock() {
    // The regression itself: pending capture + lock + vblank confirm, with
    // zero further client commits anywhere after the confirm.
    let mut fixture = Fixture::start();
    park_across_deferred_lock(&mut fixture);

    // The DRM vblank handler's own call, for flip 7's completion.
    fixture.state.note_flip_completed(Some(7));
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "the tracked flip's vblank confirms the lock"
    );
    // The pin that fails pre-fix: nothing has been dispatched since the
    // confirm call, so an armed ticker is the confirm path's own doing.
    // (`PollFrame` below cannot discriminate headless -- the stale backdrop
    // re-arms through `refresh_lock_state` on its display dispatch either
    // way, as the module doc above explains.)
    assert!(
        fixture.state.timer_armed,
        "confirming the lock must re-arm the frame ticker, or nothing runs \
         `service_captures` again and the parked frame is stuck"
    );
    let (outcome, captured) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Ready,
        "a vblank-confirmed lock must deliver the capture it parked"
    );
    assert_eq!(
        captured,
        fixture.pixels(),
        "the delivered frame is the framebuffer the re-armed tick drew"
    );
    assert!(
        captured
            .chunks_exact(4)
            .any(|pixel| pixel == LOCK_BGRA.as_slice()),
        "and that framebuffer is the lock screen, not the parked desktop"
    );
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "no pixel of the pre-blank desktop may reach a capture parked across the lock"
    );

    // Steady-state cost: the re-arm was one tick, not a hot timer.
    fixture.settle();
    assert!(
        !fixture.state.timer_armed,
        "after delivery the timer must be dropped again -- it fires only \
         when work exists"
    );
}

#[test]
fn a_parked_capture_is_delivered_once_the_fallback_confirms_the_lock() {
    // The same re-arm through the other confirm path: the vblank never
    // arrives (switched away, discarded flip, silent driver) and the
    // one-second fallback confirms instead.
    let mut fixture = Fixture::start();
    let t0 = park_across_deferred_lock(&mut fixture);

    fixture.state.note_blank_timeout(t0 + LOCK_VBLANK_TIMEOUT);
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "the fallback confirms the lock without any vblank"
    );
    // The same pin through the other confirm path (see the vblank test for
    // why the pin, not the delivery, discriminates headless).
    assert!(
        fixture.state.timer_armed,
        "a fallback confirm must re-arm the frame ticker too"
    );
    let (outcome, captured) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Ready,
        "a fallback-confirmed lock must deliver the capture it parked"
    );
    assert_eq!(
        captured,
        fixture.pixels(),
        "the delivered frame is the framebuffer the re-armed tick drew"
    );
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "no pixel of the pre-blank desktop may reach a capture parked across the lock"
    );

    fixture.settle();
    assert!(
        !fixture.state.timer_armed,
        "after delivery the timer must be dropped again -- it fires only \
         when work exists"
    );
}

#[test]
fn a_confirm_with_no_parked_capture_costs_one_tick() {
    // The re-arm buys exactly one tick, even with nothing waiting on it: a
    // confirm with no parked capture must not keep the timer alive.
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));

    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.run(Step::LockNoWait);
    assert!(fixture.state.session_lock.awaiting_blank());
    let _ = fixture
        .state
        .session_lock
        .await_vblank(Some(7), Instant::now());
    fixture.state.needs_render = false;
    fixture.tick(Duration::from_millis(50));
    assert!(!fixture.state.timer_armed);
    fixture.state.put_primary_backend(backend);

    fixture.state.note_flip_completed(Some(7));
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "the tracked flip's vblank confirms the lock"
    );
    // The one re-armed tick runs -- a render nobody asked for -- and then
    // the timer is dropped again rather than ticking on idle. That single
    // wakeup per lock is the whole steady-state cost of the fix.
    assert!(
        fixture.state.timer_armed,
        "the confirm path re-arms unconditionally; the re-arm set decides \
         whether the tick after it keeps the timer"
    );
    fixture.tick(Duration::from_millis(50));
    assert!(
        !fixture.state.timer_armed,
        "a confirm with nothing parked must not keep the timer alive"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "the session is still locked; only the wait was taken"
    );
}

#[test]
fn parked_captures_on_two_sessions_are_all_delivered_by_one_confirm() {
    // One confirm re-arms one tick, and one tick's `service_captures` serves
    // every session that is due -- not just the first.
    let mut fixture = Fixture::start();
    let other = fixture.spawn(run_client);
    for client in [0, other] {
        fixture.run_on(
            client,
            Step::StartSession {
                paint_cursors: false,
            },
        );
        let (outcome, _) = fixture
            .run_on(
                client,
                Step::Capture {
                    width: CANVAS,
                    height: CANVAS,
                    format: wl_shm::Format::Argb8888,
                },
            )
            .frame();
        assert_eq!(outcome, Outcome::Ready);
        fixture.run_on(client, Step::CaptureWithoutWaiting);
    }
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    // Re-park after the window mapped: the mapping was the change that would
    // otherwise have served them.
    //
    // Wait for the screen to go quiescent first: a stable `frame_serial`
    // across a settle plus several ticks means no commit is still working
    // its way through the frame timer. Both clients are parked on their
    // step channels and no timer is armed at idle, so once the serial holds
    // still there is nothing left that could advance it.
    let mut serial = fixture.state.frame_serial;
    for _ in 0..10 {
        fixture.settle();
        fixture.tick(Duration::from_millis(50));
        let next = fixture.state.frame_serial;
        if next == serial {
            break;
        }
        serial = next;
    }
    assert_eq!(
        fixture.state.frame_serial, serial,
        "the screen must be quiescent before re-parking: a still-advancing \
         frame_serial would serve the parked capture this test asserts \
         `Waiting` on"
    );
    for client in [0, other] {
        // Synchronize the session to the current screen, then assert it
        // parks. A re-parked capture is due -- and correctly served --
        // whenever its session's last delivery predates the current
        // `frame_serial`: under full-suite parallel load the pre-map parked
        // frame can be consumed by an earlier tick than the map's own,
        // leaving `delivered` behind the serial the re-park observes. The
        // measured shape is `Ready` at an unmoving serial (park, settle and
        // poll all read the same value), *not* an advance between park and
        // poll -- serving that due capture is correct production behavior,
        // so the test re-syncs instead of asserting against it. The first
        // poll drains the lag, which re-syncs `delivered` to the current
        // serial; the re-park off it then waits deterministically, because
        // nothing after quiescence can advance the serial again. One retry
        // is the proven max; the loop is bounded at three so a genuinely
        // advancing screen fails loudly instead of polling forever. The
        // delivered-frames assertions after the confirm stay exact -- only
        // this intermediate poll learns to re-sync.
        let mut outcome = Outcome::Waiting;
        for attempt in 0..3 {
            fixture.run_on(client, Step::CaptureWithoutWaiting);
            (outcome, _) = fixture.run_on(client, Step::PollFrame).frame();
            if outcome == Outcome::Waiting {
                break;
            }
            eprintln!(
                "client {client} attempt {attempt}: a re-parked capture came \
                 back {outcome:?} at serial {} -- draining the lag and \
                 re-parking",
                fixture.state.frame_serial
            );
        }
        assert_eq!(
            outcome,
            Outcome::Waiting,
            "client {client}: a capture parked on a quiescent, synchronized \
             screen must wait"
        );
    }

    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.run(Step::LockNoWait);
    assert!(fixture.state.session_lock.awaiting_blank());
    let _ = fixture
        .state
        .session_lock
        .await_vblank(Some(7), Instant::now());
    fixture.state.needs_render = false;
    fixture.tick(Duration::from_millis(50));
    assert!(!fixture.state.timer_armed);
    fixture.state.put_primary_backend(backend);

    fixture.state.note_flip_completed(Some(7));
    assert!(
        fixture.state.timer_armed,
        "the confirm must re-arm the ticker whatever is parked"
    );
    for client in [0, other] {
        let (outcome, captured) = fixture.run_on(client, Step::PollFrame).frame();
        assert_eq!(
            outcome,
            Outcome::Ready,
            "one confirm must deliver every parked session, not just the first"
        );
        assert!(
            !captured
                .chunks_exact(4)
                .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
            "client {client}: no pixel of the pre-blank desktop"
        );
    }

    fixture.settle();
    assert!(!fixture.state.timer_armed);
}

#[test]
fn a_parked_capture_outlives_a_locker_that_dies_mid_wait() {
    // "Unlock before confirm" has no protocol shape -- Smithay routes
    // `unlock_and_destroy` only once `locked` has been sent, which is what
    // clears `pending` (see `SessionLockHandler::unlock`) -- so the reachable
    // form of a lock ending mid-wait is the locker dying: the session stays
    // locked and reads as abandoned. The parked capture is still owed the
    // lock framebuffer, not a failure and not the desktop.
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
    fixture.run(Step::CaptureWithoutWaiting);

    let locker = fixture.spawn(run_client);
    let backend = fixture.state.take_primary_backend().expect("a backend");
    fixture.run_on(locker, Step::LockNoWait);
    assert!(fixture.state.session_lock.awaiting_blank());
    let _ = fixture
        .state
        .session_lock
        .await_vblank(Some(7), Instant::now());
    fixture.disconnect(locker);
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker leaves the session locked, not unlocked"
    );
    fixture.state.needs_render = false;
    fixture.tick(Duration::from_millis(50));
    assert!(!fixture.state.timer_armed);
    fixture.state.put_primary_backend(backend);

    fixture.state.note_flip_completed(Some(7));
    assert!(
        !fixture.state.session_lock.awaiting_blank(),
        "a dead locker's vblank still takes the wait"
    );
    assert!(
        fixture.state.timer_armed,
        "the confirm must re-arm the ticker for a dead locker's capture too"
    );
    let (outcome, captured) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Ready,
        "a dead locker's vblank still confirms, and the parked capture is \
         still owed the locked framebuffer"
    );
    assert_eq!(
        captured,
        fixture.pixels(),
        "the delivered frame is the framebuffer the re-armed tick drew"
    );
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "no pixel of the pre-blank desktop may reach a capture parked across the lock"
    );

    fixture.settle();
    assert!(!fixture.state.timer_armed);
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
    let (outcome, pool) = fixture
        .run(Step::CaptureFromPool {
            format: wl_shm::Format::Argb8888,
            sibling: true,
            pad: 0,
        })
        .frame();
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
        .run(Step::CaptureFromPool {
            format: wl_shm::Format::Argb8888,
            sibling: false,
            pad: PAD,
        })
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
fn an_opaque_capture_honours_the_offset_and_the_stride_as_well() {
    // The two tests above cover `data.offset` and `data.stride` for the
    // `Argb8888` path -- which is a plain row `memcpy` and never enters the
    // opacity pass at all. So the *wide* `Xrgb8888` pass, which walks the same
    // two client numbers a second time with its own arithmetic, was covered
    // only at `offset == 0` and `stride == width * 4`: exactly the gap those
    // two tests exist to close, one code path later.
    //
    // All three at once, because that is the shape that separates them: a
    // client double-buffering out of one pool, asking for the opaque format,
    // with padded rows. An opacity pass that ignored `data.offset` would stamp
    // `0xff` over every fourth byte of the *sibling* buffer -- which the client
    // may have attached to a visible surface -- while the capture it returned
    // looked perfectly correct.
    const PAD: i32 = 16;
    let mut fixture = Fixture::start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let (outcome, pool) = fixture
        .run(Step::CaptureFromPool {
            format: wl_shm::Format::Xrgb8888,
            sibling: true,
            pad: PAD,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);

    let row = (CANVAS * 4) as usize;
    let stride = row + PAD as usize;
    let bytes = stride * CANVAS as usize;
    assert_eq!(pool.len(), bytes * 2, "the whole pool is read back");
    let (sibling, target) = pool.split_at(bytes);
    let framebuffer = fixture.pixels();

    assert!(
        sibling.chunks_exact(4).all(|pixel| pixel == SENTINEL),
        "the sibling buffer must be byte-for-byte untouched -- neither the row \
         copy nor the opacity pass may write outside the buffer it was given"
    );
    for y in 0..CANVAS as usize {
        let line = &target[y * stride..][..stride];
        let drawn = &framebuffer[y * row..][..row];
        for (x, (captured, drawn)) in line[..row]
            .chunks_exact(4)
            .zip(drawn.chunks_exact(4))
            .enumerate()
        {
            assert_eq!(
                &captured[..3],
                &drawn[..3],
                "row {y} pixel {x}: the colour has to be the frame's, at its own stride"
            );
            assert_eq!(
                captured[3], 0xFF,
                "row {y} pixel {x}: the X byte has to be forced opaque, at its own stride"
            );
        }
        assert!(
            line[row..].chunks_exact(4).all(|pixel| pixel == SENTINEL),
            "row {y}'s padding is not part of the image -- the opacity pass must \
             stop at the pixels, not run to the stride"
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
fn a_create_frame_flood_is_refused_with_duplicate_frame() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });

    // More frames than any well-behaved client holds at once, none of them
    // ever captured or destroyed: the `create_frame`-loop leak shape. The
    // compositor must refuse with the protocol's own `duplicate_frame`
    // error, which ends this client -- and nothing else.
    let error = fixture.run_expecting_disconnect(Step::FloodFrames {
        count: MAX_FRAMES_PER_CLIENT + 5,
    });
    assert!(
        error.contains("duplicate_frame"),
        "a second live frame on crowded state must be the protocol's own \
         `duplicate_frame` error, got: {error}"
    );
    assert_eq!(
        fixture.state.screencopy.frames_in_flight(),
        0,
        "killing the flooding client has to drain the bookkeeping with it"
    );
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (0, 0),
        "the dead client's session must leave both lists, like any disconnect"
    );
}

#[test]
fn a_frame_flood_from_one_client_does_not_deny_another() {
    let mut fixture = Fixture::start();
    let other = fixture.spawn(run_client);
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run_on(
        other,
        Step::StartSession {
            paint_cursors: false,
        },
    );

    // Client 0 floods itself into a protocol error. Client 1, which never
    // misbehaved, must still capture afterwards -- the anti-`connection-cap-
    // denies-the-same-user` property, pinned for this protocol: the bound is
    // per client, so one client's greed can only ever kill that client.
    let error = fixture.run_expecting_disconnect(Step::FloodFrames {
        count: MAX_FRAMES_PER_CLIENT + 5,
    });
    assert!(
        error.contains("duplicate_frame"),
        "the flood must end in `duplicate_frame`, got: {error}"
    );
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
    assert_eq!(
        outcome,
        Outcome::Ready,
        "the innocent client must still be served after another was refused"
    );
    assert_eq!(captured, fixture.pixels());
}

#[test]
fn rapid_create_destroy_cycling_leaves_no_frame_bookkeeping() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });

    fixture.run(Step::CycleFrames { rounds: 50 });
    assert_eq!(
        fixture.state.screencopy.frames_in_flight(),
        0,
        "every destroyed frame has to release its bookkeeping slot"
    );

    // And the session is still fully usable afterwards: cycling must not
    // wedge it.
    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Argb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(captured, fixture.pixels());
}

#[test]
fn frames_outliving_their_session_leave_no_frame_bookkeeping() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });

    // Three live frames, then the session goes away under them -- legal per
    // the protocol ("this request doesn't affect ... frame ... objects
    // created by this object"). The frames still count until *they* die;
    // what must not happen is the session's entry dangling afterwards.
    fixture.run(Step::HoldFrames { count: 3 });
    assert_eq!(fixture.state.screencopy.frames_in_flight(), 3);
    fixture.run(Step::DestroySession);
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (0, 0),
        "the session must leave both lists even with frames outstanding"
    );
    assert_eq!(
        fixture.state.screencopy.frames_in_flight(),
        3,
        "live frames still count after their session is gone"
    );

    fixture.disconnect(0);
    assert_eq!(
        fixture.state.screencopy.frames_in_flight(),
        0,
        "the frames dying with their client has to drain the bookkeeping"
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

/// `grim`'s shape on a screen nothing redraws, on the GLES renderer: a
/// fresh session per capture (so none of them waits for damage), with and
/// without the pointer painted in. Each one used to leave its read-back's
/// pixel-pack buffer, a whole frame, queued in Smithay's cleanup queue
/// until the next frame drew; see `render/tests/capture_release.rs`, which
/// pins the mechanism and counts the same way.
#[test]
fn captures_of_a_static_screen_on_gles_leave_no_gl_objects_behind() {
    let mut fixture: Fixture = Harness::headless_on(appearance(), CANVAS, RendererKind::Gles);
    fixture.spawn(run_client);
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    let id = fixture.state.outputs.primary_id().expect("an output");
    fixture.state.pointer_move(10.0, 10.0);
    let grab = |fixture: &mut Fixture, paint_cursors: bool| {
        fixture.run(Step::StartSession { paint_cursors });
        let (outcome, captured) = fixture
            .run(Step::Capture {
                width: CANVAS,
                height: CANVAS,
                format: wl_shm::Format::Argb8888,
            })
            .frame();
        assert_eq!(outcome, Outcome::Ready, "a fresh session never waits");
        fixture.run(Step::DestroySession);
        captured
    };
    let live = |fixture: &mut Fixture| {
        fixture
            .state
            .backends
            .get_mut(&id)
            .expect("a backend")
            .gles_live_objects_for_test()
            .expect("a GLES backend")
    };
    for paint_cursors in [false, true] {
        let first = grab(&mut fixture, paint_cursors);
        let settled = live(&mut fixture);
        for _ in 0..8 {
            assert_eq!(
                grab(&mut fixture, paint_cursors),
                first,
                "cursors {paint_cursors}: nothing moved"
            );
        }
        assert_eq!(
            live(&mut fixture),
            settled,
            "cursors {paint_cursors}: captures of a static screen must not leave \
             GL objects queued"
        );
        let backend = fixture.state.backends.get_mut(&id).expect("a backend");
        backend.cleanup_texture_cache().expect("a drain");
        assert_eq!(
            live(&mut fixture),
            settled,
            "cursors {paint_cursors}: nothing was left queued for a drain to free"
        );
    }
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
    // What `Step::LockNoWait` mapped and must stay mapped: unlike `Step::Lock`
    // -- whose delivery happens before the next client roundtrip flushes the
    // arm-end destroys -- a deferred confirmation delivers an earthly delay
    // later, so dropping these at the arm's end would unmap the lock screen
    // before the capture under test is served. A real locker keeps its
    // surface mapped the same way.
    let mut lock_surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    let mut lock_roles: Vec<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1> = Vec::new();
    let mut lock_buffers: Vec<CaptureBuffer> = Vec::new();
    // Frames created by [`Step::HoldFrames`] and deliberately never destroyed,
    // so they outlive whatever the script does next. Held here rather than in
    // `frame` so no later step reaps them: dropping a client-side proxy sends
    // `destroy`, which would drain exactly the leak shape under test.
    let mut leaked: Vec<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1> = Vec::new();
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
            Step::CaptureFromPool {
                format,
                sibling,
                pad,
            } => {
                let session = session.as_ref().ok_or("no session")?;
                let stride = CANVAS * 4 + pad;
                let bytes = (stride * CANVAS) as usize;
                let mut pool = Pool::new(&shm, &qh, bytes * if sibling { 2 } else { 1 }, SENTINEL);
                let offset = if sibling {
                    // A buffer ahead of the capture's -- what a toolkit
                    // double-buffering out of one pool has, and what a write
                    // that ignored `data.offset` would land in. Kept alive for
                    // the run so the pool really does hold two live buffers.
                    neighbours.push(pool.buffer(0, CANVAS, CANVAS, stride, format));
                    bytes as i32
                } else {
                    0
                };
                let target = pool.buffer(offset, CANVAS, CANVAS, stride, format);
                pool.finish();
                capture_into(
                    session,
                    &qh,
                    &mut frame,
                    &mut held,
                    &mut client,
                    target,
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
            Step::FloodFrames { count } => {
                let session = session.as_ref().ok_or("no session")?;
                // Deliberately never attached, captured or destroyed -- the
                // leak shape. Held locally so the proxies stay alive until
                // the refusal lands: dropping one would send `destroy`.
                let mut flood = Vec::new();
                for _ in 0..count {
                    flood.push(session.create_frame(&qh, FrameSlot { extra: true }));
                }
                // The refusal arrives as a protocol error on a round trip,
                // not necessarily the first: the requests flush together and
                // the compositor answers while this loop keeps reading.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match queue.roundtrip(&mut client) {
                        Err(error) => return Err(error.to_string()),
                        Ok(_) => {
                            if Instant::now() >= deadline {
                                return Err(format!(
                                    "the compositor accepted {count} live frames \
                                     with no refusal"
                                ));
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
            }
            Step::HoldFrames { count } => {
                let session = session.as_ref().ok_or("no session")?;
                for _ in 0..count {
                    leaked.push(session.create_frame(&qh, FrameSlot { extra: true }));
                }
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::CycleFrames { rounds } => {
                let session = session.as_ref().ok_or("no session")?;
                for _ in 0..rounds {
                    let live = session.create_frame(&qh, FrameSlot { extra: true });
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    live.destroy();
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
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
            Step::LockNoWait => {
                // `Step::Lock` up to the surface commit, without the wait for
                // `locked`: the commit is on the wire (and a round trip has
                // flushed it), but confirmation is whatever the test drives
                // next by hand.
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
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                locks.push(lock);
                // Kept mapped (see the declaration): the capture this step
                // sets up is served after a deferred confirmation, not in
                // this step's own settle.
                lock_surfaces.push(surface);
                lock_roles.push(role);
                lock_buffers.push(buffer);
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
        let fd = rustix::fs::memfd_create("scoot-capture-test", rustix::fs::MemfdFlags::CLOEXEC)
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
