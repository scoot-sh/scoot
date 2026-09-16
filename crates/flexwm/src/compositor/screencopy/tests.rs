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
    len: usize,
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
    if let Some(previous) = frame.take() {
        previous.destroy();
    }
    // A recognisable fill, so a capture that wrote nothing is not mistaken for
    // one that wrote black.
    let buffer = solid_buffer(shm, qh, width, height, [0x11, 0x22, 0x33, 0x44], format);
    let new_frame = session.create_frame(qh, FrameSlot { extra: false });
    client.frame = Outcome::Waiting;
    new_frame.attach_buffer(buffer.as_ref());
    new_frame.damage_buffer(0, 0, width, height);
    new_frame.capture();
    *frame = Some(new_frame);
    *held = Some(buffer);
    Ok(())
}

/// The client's own buffer, read straight back out of the memfd behind it.
fn read_back(held: &Option<CaptureBuffer>) -> Vec<u8> {
    let Some(held) = held else {
        return Vec::new();
    };
    let mut pixels = vec![0u8; held.len];
    held.file
        .read_exact_at(&mut pixels, 0)
        .expect("the client's own pool is readable");
    pixels
}

/// A `width`x`height` `wl_buffer` filled with `color`, over a real memfd --
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
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("flexwm-capture-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, format, qh, ());
    pool.destroy();
    CaptureBuffer { buffer, file, len }
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
