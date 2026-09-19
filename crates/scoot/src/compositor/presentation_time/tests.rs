//! Tests for `wp_presentation` (presentation-time).
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the
//! presentation global, maps a painted toplevel, requests feedback on its
//! surface, and the test asserts on the `presented`/`discarded` events that
//! actually arrive on the wire -- pinned to sane field values, not just
//! event presence. The timestamps a test asserts are the compositor's own
//! `CLOCK_MONOTONIC` readings at the frame handoff (see `super` for what
//! each backend's handoff is), so a compositor that stamped fiction would
//! fail the monotonicity and refresh assertions rather than merely emitting
//! nothing.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

use super::presented_frame;

/// The framebuffer these tests render into. Small on purpose: nothing here
/// reads pixels except the disconnect test's liveness check.
const CANVAS: i32 = 320;

/// The side, in pixels, of the buffer each toplevel paints.
const SURFACE: i32 = 64;

/// One instruction for the client thread.
enum Step {
    /// `wp_presentation.feedback` on the mapped window, then attach, damage
    /// and commit so the request reaches the committed state Smithay takes
    /// feedback from. The feedback object is held client-side under the next
    /// index.
    RequestFeedback,
    /// Two [`Step::RequestFeedback`]s back to back in a single step, with no
    /// compositor settle between the two commits: the frame timer the first
    /// commit arms must not fire before the second commit supersedes it, or
    /// the first feedback would be presented instead of discarded.
    RequestFeedbackPair,
    /// Drain every feedback's wire events into a report, clearing the
    /// per-feedback buffers (each feedback object is one-shot by protocol).
    ReportFeedback,
    /// A snapshot of every feedback object so far, without advancing the
    /// report cursor: for re-reading an already-reported object whose
    /// outcome arrived later (the lock test's unlock presenting the same
    /// object the locked frame reported as pending).
    ReportAll,
    /// `wp_presentation.feedback` on a bare `wl_surface` with no shell role
    /// -- never mapped, never in the space -- then attach and commit.
    FeedbackOnBareSurface,
    /// `ext_session_lock_manager_v1.lock`, waiting for `locked`. Only the
    /// locker script answers this (see `run_locker`).
    TakeSessionLock,
    /// `unlock_and_destroy` on the session lock. Only the locker answers.
    ReleaseSessionLock,
}

/// What a client answers a [`Step`] with.
enum Ack {
    /// Sent once at startup, before mapping: whether the registry round trip
    /// found `wp_presentation`, and the `clk_id` it reported (`None` when
    /// the global is missing and there was nothing to bind).
    Started {
        presentation: bool,
        clock_id: Option<u32>,
    },
    /// The window is mapped and the client is parked holding it.
    Mapped,
    /// The drained feedback observations (see [`Step::ReportFeedback`]).
    Feedback { events: Vec<FeedbackEvent> },
    /// The session `locked` event arrived (locker client only).
    SessionLocked,
    /// The session unlock was requested and flushed (locker client only).
    SessionReleased,
    /// Anything else a step answers when there is nothing to count.
    Done,
}

/// One feedback object's drained wire events.
struct FeedbackEvent {
    /// How many `sync_output` events preceded the outcome.
    sync_outputs: u32,
    presented: Option<PresentedFields>,
    discarded: bool,
}

/// The fields of one `presented` event, combined to host integers.
struct PresentedFields {
    sec: u64,
    nsec: u32,
    refresh: u32,
    seq: u64,
    /// The raw `kind` bits, so the test pins the exact flags per backend.
    flags: u32,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A live compositor with one output and one connected client whose
    /// window is mapped.
    fn start() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        let Ack::Started {
            presentation,
            clock_id,
        } = fixture.wait_for_ack(0)
        else {
            panic!("the client reported being mapped before it reported its globals");
        };
        assert!(presentation, "no wp_presentation -- the global is missing");
        assert_eq!(
            clock_id,
            Some(CLOCK_MONOTONIC_ID),
            "presentation clock is not CLOCK_MONOTONIC (clk_id 1): {clock_id:?}"
        );
        let Ack::Mapped = fixture.wait_for_ack(0) else {
            panic!("the client reported feedback before it reported being mapped");
        };
        fixture
    }

    /// Requests feedback, renders one frame, and hands back what the client
    /// observed -- so each presented-frame test pins its own numbers.
    fn present_once(&mut self) -> Vec<FeedbackEvent> {
        let Ack::Done = self.run_on(0, Step::RequestFeedback) else {
            panic!("a feedback request answered with something else");
        };
        self.render();
        let Ack::Feedback { events } = self.run_on(0, Step::ReportFeedback) else {
            panic!("a feedback report answered with something else");
        };
        events
    }

    /// The single presented event in `events`, asserting there is exactly
    /// one feedback and it was presented rather than discarded.
    fn only_presented(events: &[FeedbackEvent]) -> &PresentedFields {
        assert_eq!(
            events.len(),
            1,
            "expected one feedback object, saw {}",
            events.len()
        );
        let event = &events[0];
        assert!(
            !event.discarded,
            "the feedback was discarded instead of presented"
        );
        event
            .presented
            .as_ref()
            .expect("no presented event arrived")
    }
}

/// One live feedback object and what the wire has said about it so far.
struct FeedbackRec {
    /// Held alive, not read: without a destructor request in the protocol
    /// the server never learns of a client-side drop, so releasing this
    /// would let the id be reused under a live server object.
    #[allow(dead_code)]
    feedback: wp_presentation_feedback::WpPresentationFeedback,
    sync_outputs: u32,
    presented: Option<PresentedFields>,
    discarded: bool,
}

/// The client end of one test connection: enough of a toolkit to map a
/// painted toplevel and request presentation feedback on it.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    output: Option<wl_output::WlOutput>,
    presentation: Option<wp_presentation::WpPresentation>,
    clock_id: Option<u32>,
    surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    feedbacks: Vec<FeedbackRec>,
    /// How many leading `feedbacks` a report has already covered. Reports
    /// never remove entries: a feedback object the server still holds stays
    /// addressable under its creation index, so a `presented` arriving for
    /// an already-reported object must still find it (see the lock test,
    /// where the unlock presents the same object the locked frame reported
    /// as pending).
    reported: usize,
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
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_output::WlOutput::interface().name {
            client.output = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wp_presentation::WpPresentation::interface().name {
            // Version 2 is what the compositor advertises; it carries the
            // variable-refresh semantics, which behave identically for the
            // fixed refresh this compositor reports.
            client.presentation = Some(registry.bind(name, version.min(2), qh, ()));
        }
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wp_presentation::WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            client.clock_id = Some(clk_id);
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, usize> for TestClient {
    fn event(
        client: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let rec = &mut client.feedbacks[*index];
        match event {
            wp_presentation_feedback::Event::SyncOutput { .. } => rec.sync_outputs += 1,
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                flags,
            } => {
                let flags = match flags {
                    WEnum::Value(kind) => kind.bits(),
                    WEnum::Unknown(bits) => bits,
                };
                rec.presented = Some(PresentedFields {
                    sec: (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo),
                    nsec: tv_nsec,
                    refresh,
                    seq: (u64::from(seq_hi) << 32) | u64::from(seq_lo),
                    flags,
                });
            }
            wp_presentation_feedback::Event::Discarded => rec.discarded = true,
            _ => {}
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

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
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

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque pixels, over a real memfd --
/// the same path any toolkit takes (the shape `activation/tests.rs` uses).
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("scoot-presentation-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        SURFACE,
        SURFACE,
        stride,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    // A second round trip for the `clock_id` the bind handshake sends: it
    // is queued by the bind above but not yet dispatched.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;

    // Mapped in two commits, the way the protocol asks: the role-only commit
    // first, then pixels once the compositor's configure has been acked (the
    // `xdg_surface` handler above does that as the event arrives).
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title("presentation".to_string());
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let buffer = solid_buffer(&shm, &qh)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, SURFACE, SURFACE);
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    client.surface = Some(surface.clone());
    client.buffer = Some(buffer);

    let presentation = client.presentation.is_some();
    let clock_id = client.clock_id;
    acks.send(Ack::Started {
        presentation,
        clock_id,
    })
    .map_err(|e| e.to_string())?;
    acks.send(Ack::Mapped).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        // Drain anything the compositor sent since the last step -- in
        // particular the `presented`/`discarded` the test just caused --
        // before acting, so a report answers what happened rather than what
        // is still in flight.
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::RequestFeedback => {
                request_feedback(&mut client, &qh, &surface)?;
            }
            Step::RequestFeedbackPair => {
                request_feedback(&mut client, &qh, &surface)?;
                request_feedback(&mut client, &qh, &surface)?;
            }
            Step::FeedbackOnBareSurface => {
                let bare = compositor.create_surface(&qh, ());
                request_feedback(&mut client, &qh, &bare)?;
            }
            Step::ReportFeedback | Step::ReportAll => {
                let snapshot = matches!(step, Step::ReportAll);
                let events = client
                    .feedbacks
                    .iter()
                    .skip(if snapshot { 0 } else { client.reported })
                    .map(|rec| FeedbackEvent {
                        sync_outputs: rec.sync_outputs,
                        presented: rec.presented.as_ref().map(|fields| PresentedFields {
                            sec: fields.sec,
                            nsec: fields.nsec,
                            refresh: fields.refresh,
                            seq: fields.seq,
                            flags: fields.flags,
                        }),
                        discarded: rec.discarded,
                    })
                    .collect();
                if !snapshot {
                    client.reported = client.feedbacks.len();
                }
                acks.send(Ack::Feedback { events })
                    .map_err(|e| e.to_string())?;
                continue;
            }
            Step::TakeSessionLock | Step::ReleaseSessionLock => {
                return Err("the presentation client cannot take a session lock".to_string());
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One `feedback` request plus the damaged commit that moves it from
/// Smithay's pending state into the committed state the frame takes from.
/// Without the commit the request would sit in pending forever and no frame
/// could ever present it.
fn request_feedback(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    surface: &wl_surface::WlSurface,
) -> Result<(), String> {
    let presentation = client.presentation.clone().ok_or("no wp_presentation")?;
    let buffer = client.buffer.clone().ok_or("no buffer to commit")?;
    let index = client.feedbacks.len();
    let feedback = presentation.feedback(surface, qh, index);
    client.feedbacks.push(FeedbackRec {
        feedback,
        sync_outputs: 0,
        presented: None,
        discarded: false,
    });
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, SURFACE, SURFACE);
    surface.commit();
    Ok(())
}

/// The locker end of a second test connection: just enough of a screen
/// locker to take and release the session lock around a feedback test.
/// Maps no surfaces at all, like the relative-pointer round-trip test.
#[derive(Default)]
struct LockerClient {
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    locked_seen: bool,
    finished_seen: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for LockerClient {
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
        if interface == ext_session_lock_manager_v1::ExtSessionLockManagerV1::interface().name {
            client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => client.locked_seen = true,
            ext_session_lock_v1::Event::Finished => client.finished_seen = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

fn run_locker(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LockerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    client
        .lock_manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1")?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::TakeSessionLock => {
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                let lock = manager.lock(&qh, ());
                client.lock = Some(lock);
                wait_for(&mut queue, &mut client, "locked or finished", |seen| {
                    (seen.locked_seen || seen.finished_seen).then_some(())
                })?;
                if !client.locked_seen {
                    return Err("the session lock was refused".to_string());
                }
                acks.send(Ack::SessionLocked).map_err(|e| e.to_string())?;
                continue;
            }
            Step::ReleaseSessionLock => {
                let lock = client.lock.take().ok_or("no session lock to release")?;
                lock.unlock_and_destroy();
                // Flush the unlock before acknowledging: the ack travels
                // over the step channel, not the Wayland socket, so without
                // this the compositor can ack, settle and render while the
                // unlock is still sitting in this client's unsent buffer --
                // and the test would render a still-locked session.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::SessionReleased).map_err(|e| e.to_string())?;
                continue;
            }
            _ => return Err("the locker cannot map windows or request feedback".to_string()),
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// `presented_frame`: which backend stamps what (unit level)
// -------------------------------------------------------------------------

// A committed nested frame stamps zero: nested output is self-refreshing
// with no queryable count, so the protocol requires zero.
#[test]
fn a_committed_nested_frame_stamps_zero() {
    assert_eq!(
        presented_frame(true, true, false, None, true),
        Some(0),
        "a frame the host accepted must stamp seq zero, not a counter"
    );
}

// A nested frame the host never accepted stamps nothing at all -- dropped
// for lack of a free buffer, a size mismatch, or a flush a dead host
// refused (see `Host::present`): a time nothing was shown at must not go
// out. A live host connection is unconstructible in-harness (its surface,
// shm and buffers are registry-bound against a real host compositor), so
// this arm -- the bool plumbing `Host::present`'s return feeds -- is the
// unit-level pin for that path, verified in code at its single call site.
#[test]
fn a_dropped_nested_frame_stamps_nothing() {
    assert_eq!(
        presented_frame(true, false, false, None, true),
        None,
        "a frame the host never accepted must leave feedback queued"
    );
}

// A tty flip stamps its issued number: the per-scanout counter the backend
// has in hand.
#[test]
fn an_issued_tty_flip_stamps_its_number() {
    assert_eq!(
        presented_frame(false, false, true, Some(7), true),
        Some(7),
        "a tty flip must stamp its issued number, not a frame counter"
    );
}

// A tty frame that issued no flip stamps nothing -- busy CRTC, paused
// session, size mismatch, failed commit.
#[test]
fn an_unflipped_tty_frame_stamps_nothing() {
    assert_eq!(
        presented_frame(false, false, true, None, true),
        None,
        "a tty frame with no flip must leave feedback queued"
    );
}

// Plain headless stamps zero for a drawn frame (no retrace to count) and
// nothing when the draw itself failed.
#[test]
fn headless_stamps_zero_for_a_drawn_frame_and_nothing_otherwise() {
    assert_eq!(
        presented_frame(false, false, false, None, true),
        Some(0),
        "a drawn headless frame must stamp seq zero"
    );
    assert_eq!(
        presented_frame(false, false, false, None, false),
        None,
        "an undrawn headless frame must leave feedback queued"
    );
}

// -------------------------------------------------------------------------
// The tests
// -------------------------------------------------------------------------

/// The compositor's `CLOCK_MONOTONIC` id, which is what the bind handshake
/// must report: the timestamps below are only interpretable in that clock.
const CLOCK_MONOTONIC_ID: u32 = 1;

/// Roughly 60 Hz, in nanoseconds: the output mode's own refresh (see
/// `set_mode`), which is what a fixed-refresh `presented` event carries.
const REFRESH_60HZ_MIN: u32 = 15_000_000;
const REFRESH_60HZ_MAX: u32 = 17_500_000;

#[test]
fn the_presentation_global_is_advertised_with_the_monotonic_clock() {
    // The advertisement and clock assertions live in `Fixture::start`,
    // which fails here when either is wrong -- this test exists so the
    // failure has a name pointing at the global rather than at a step.
    let _fixture = Fixture::start();
}

#[test]
fn a_committed_surface_is_presented_with_sane_fields() {
    let mut fixture = Fixture::start();
    let events = fixture.present_once();
    let presented = Fixture::only_presented(&events);
    assert!(
        presented.sec != 0 || presented.nsec != 0,
        "the presented timestamp is zeroed: it measures no clock"
    );
    assert!(
        presented.nsec < 1_000_000_000,
        "tv_nsec {} is outside [0, 999999999], violating the protocol",
        presented.nsec
    );
    assert!(
        (REFRESH_60HZ_MIN..=REFRESH_60HZ_MAX).contains(&presented.refresh),
        "refresh {} ns is not the output mode's ~60 Hz",
        presented.refresh
    );
    assert_eq!(
        presented.flags, 0,
        "headless reports no vsync/hw flags: there is no retrace and no zero-copy path"
    );
    assert_eq!(
        events[0].sync_outputs, 1,
        "one bound wl_output means one sync_output before presented"
    );
}

#[test]
fn timestamps_increase_and_seq_is_zero_on_headless() {
    // Each `present_once` owns its report. `--headless` has no vertical
    // retrace and no refresh cycle of its own, so the protocol requires
    // `seq` zero ("If the output does not have a concept of vertical
    // retrace or a refresh cycle ... then seq_hi/seq_lo MUST be zero") --
    // while the timestamps must still order the two frames.
    let mut fixture = Fixture::start();
    let a = fixture.present_once();
    let b = fixture.present_once();
    let pa = Fixture::only_presented(&a);
    let pb = Fixture::only_presented(&b);
    let (ta, tb) = ((pa.sec, pa.nsec), (pb.sec, pb.nsec));
    assert!(
        tb > ta,
        "the second presented timestamp {tb:?} is not after {ta:?}"
    );
    for (which, presented) in [("first", pa), ("second", pb)] {
        assert_eq!(
            presented.seq, 0,
            "the {which} headless presented seq is {}: headless has no retrace, so seq MUST be zero",
            presented.seq
        );
    }
}

#[test]
fn an_explicit_seq_reaches_the_wire_verbatim() {
    // Pins that the caller's seq -- the tty flip number, or zero elsewhere
    // per `presented_frame` -- is what the `presented` event carries, rather
    // than anything the take path invents (it used to be the damaged-frame
    // counter). Calls `present_feedback` directly instead of rendering so
    // the asserted number can be one no frame would produce.
    //
    // Sequenced deterministically against the frame timer: the first
    // feedback is consumed and reported (slate clean), then the harness
    // idles past the timer's own deadline so it fires and drops itself --
    // after that no auto-render can interleave before the direct call,
    // which runs with no settle between the ack and the take.
    use std::time::Duration;

    let mut fixture = Fixture::start();
    let first = fixture.present_once();
    assert!(
        Fixture::only_presented(&first).seq == 0,
        "slate not clean: the first frame did not present with seq zero"
    );
    fixture.tick(Duration::from_millis(30));
    fixture.send_step(0, Step::RequestFeedback);
    let Ack::Done = fixture.wait_for_ack(0) else {
        panic!("a feedback request answered with something else");
    };
    let output = fixture.state.output.clone().expect("an output");
    fixture.state.present_feedback(&output, false, None, 42);
    let Ack::Feedback { events } = fixture.run_on(0, Step::ReportFeedback) else {
        panic!("a feedback report answered with something else");
    };
    assert_eq!(
        Fixture::only_presented(&events).seq,
        42,
        "the presented seq is not the number the frame handed over"
    );
}

#[test]
fn a_superseded_commit_is_discarded_not_presented() {
    let mut fixture = Fixture::start();
    // Two feedbacks on two consecutive commits with no frame between: the
    // first content update was never shown, so its feedback must be
    // discarded while the second is presented.
    let Ack::Done = fixture.run_on(0, Step::RequestFeedbackPair) else {
        panic!("a feedback request answered with something else");
    };
    fixture.render();
    let Ack::Feedback { events } = fixture.run_on(0, Step::ReportFeedback) else {
        panic!("a feedback report answered with something else");
    };
    assert_eq!(
        events.len(),
        2,
        "expected two feedback objects, saw {}",
        events.len()
    );
    assert!(
        events[0].discarded && events[0].presented.is_none(),
        "the superseded first commit was not discarded"
    );
    assert!(
        !events[1].discarded && events[1].presented.is_some(),
        "the shown second commit was not presented"
    );
}

#[test]
fn an_unmapped_surface_gets_no_feedback() {
    // A surface with no shell role is never in the space and never drawn:
    // presenting feedback for it would describe a frame that showed
    // nothing. Its feedback stays queued -- neither presented nor
    // discarded.
    let mut fixture = Fixture::start();
    let Ack::Done = fixture.run_on(0, Step::FeedbackOnBareSurface) else {
        panic!("a feedback request answered with something else");
    };
    fixture.render();
    fixture.settle();
    let Ack::Feedback { events } = fixture.run_on(0, Step::ReportFeedback) else {
        panic!("a feedback report answered with something else");
    };
    assert_eq!(
        events.len(),
        1,
        "expected one feedback object, saw {}",
        events.len()
    );
    assert!(
        !events[0].discarded && events[0].presented.is_none(),
        "an undisplayed surface got feedback for a frame that never showed it"
    );
}

#[test]
fn disconnecting_with_pending_feedback_is_clean() {
    // A client that asked for feedback, committed, and vanished before any
    // frame: Smithay's own teardown owns the queued callback, and the next
    // renders must run as if it was never there.
    let mut fixture = Fixture::start();
    let Ack::Done = fixture.run_on(0, Step::RequestFeedback) else {
        panic!("a feedback request answered with something else");
    };
    fixture.disconnect(0);
    fixture.render();
    fixture.render();
    let pixels = fixture.pixels();
    assert_eq!(
        pixels.len(),
        (CANVAS * CANVAS * 4) as usize,
        "the framebuffer did not survive a disconnect with pending feedback"
    );
}

#[test]
fn a_locked_window_gets_no_feedback_until_unlock() {
    // While locked, windows are gathered into no frame: their feedback must
    // reflect what actually presented (nothing), not what rendered (the
    // blanked frame). Unlocking presents the queued update on the next
    // frame. The feedback is requested *after* the lock is established, so
    // no earlier frame can have consumed it.
    let mut fixture = Fixture::start();
    fixture.spawn(run_locker);
    let Ack::SessionLocked = fixture.run_on(1, Step::TakeSessionLock) else {
        panic!("the locker answered a lock with something else");
    };
    let Ack::Done = fixture.run_on(0, Step::RequestFeedback) else {
        panic!("a feedback request answered with something else");
    };
    fixture.render();
    let Ack::Feedback { events } = fixture.run_on(0, Step::ReportFeedback) else {
        panic!("a feedback report answered with something else");
    };
    assert_eq!(
        events.len(),
        1,
        "expected one feedback object, saw {}",
        events.len()
    );
    assert!(
        !events[0].discarded && events[0].presented.is_none(),
        "a locked-out window got feedback for a frame that never showed it"
    );
    let Ack::SessionReleased = fixture.run_on(1, Step::ReleaseSessionLock) else {
        panic!("the locker answered an unlock with something else");
    };
    fixture.render();
    // A snapshot, not a delta report: the unlock presents the *same* object
    // the locked frame already reported as pending.
    let Ack::Feedback { events } = fixture.run_on(0, Step::ReportAll) else {
        panic!("a feedback report answered with something else");
    };
    let presented = Fixture::only_presented(&events);
    assert!(
        presented.sec != 0 || presented.nsec != 0,
        "the post-unlock presented timestamp is zeroed"
    );
}
