//! A capture *stream* keeps the scanout tier's covered output composited
//! (`Screencopy::streaming`, read by `render::primary_direct`).
//!
//! The harness half drives a real capture client against a real covering
//! fullscreen window and asks the judgement `draw_frame_scanout` runs
//! (`State::primary_direct_now`); the window's boundary is pinned on the
//! pure `requested_recently`, since waiting out a real second per assertion
//! would prove nothing more.

use std::time::{Duration, Instant};

use scoot_core::Action;

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use super::*;
use crate::compositor::dmabuf::scanout::{FormatsKey, REVERT_HOLD, Steer};
use crate::compositor::render::PrimaryDirect;
use crate::compositor::screencopy::{STREAM_WINDOW, requested_recently};

/// A covering fullscreen window steered with a scanout feedback built for a
/// virtio-shaped plane (`dmabuf/scanout.rs`), as of `now`. The capture
/// client binds no dma-buf feedback, so what is pinned here is the steering
/// decision and who holds the scanout feedback; the wire is pinned in
/// `fullscreen/tests/scanout_feedback.rs`.
fn steered(now: Instant) -> (Fixture, WlSurface) {
    let mut fixture = Fixture::start_opaque();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    assert!(fixture.state.act(Action::ToggleFullscreen));
    fixture.settle();
    let plane: FormatSet = [Fourcc::Xrgb8888, Fourcc::Argb8888]
        .into_iter()
        .map(|code| Format {
            code,
            modifier: Modifier::Invalid,
        })
        .collect();
    fixture
        .state
        .install_scanout_feedback(&plane, 0xfeed, FormatsKey { planes: 0, lost: 0 });
    assert_eq!(fixture.state.steer_now(now), Steer::Sent);
    let surface = fixture
        .state
        .windows
        .values()
        .next()
        .and_then(smithay::desktop::Window::toplevel)
        .expect("the covering window")
        .wl_surface()
        .clone();
    assert!(
        fixture
            .state
            .scanout_feedback
            .for_new_surface(&surface)
            .is_some()
    );
    (fixture, surface)
}

#[test]
fn a_capture_stream_holds_the_scanout_feedback_then_reverts_it() {
    // A recorder streaming a fullscreen game: the output composites, and
    // once that has lasted the hold the window is told the default again,
    // so it can go back to its best layout for rendering.
    let start = Instant::now();
    let (mut fixture, surface) = steered(start);
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::CaptureWithoutWaiting);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Streaming);
    assert_eq!(fixture.state.steer_now(start), Steer::Holding);
    assert!(
        fixture
            .state
            .scanout_feedback
            .for_new_surface(&surface)
            .is_some()
    );
    assert_eq!(
        fixture.state.steer_now(start + REVERT_HOLD),
        Steer::Reverted
    );
    assert!(
        fixture
            .state
            .scanout_feedback
            .for_new_surface(&surface)
            .is_none()
    );
}

#[test]
fn a_one_shot_capture_does_not_flap_the_scanout_feedback() {
    // A shell refreshing a thumbnail: one request counts as a stream for
    // `STREAM_WINDOW`, which is shorter than the hold, so the window's
    // layout is never touched.
    assert!(STREAM_WINDOW < REVERT_HOLD, "the hold outlasts one request");
    let start = Instant::now();
    let (mut fixture, surface) = steered(start);
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::CaptureWithoutWaiting);
    assert_eq!(fixture.state.steer_now(start), Steer::Holding);
    fixture.run(Step::DestroySession);
    fixture.settle();
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
    assert_eq!(fixture.state.steer_now(start + STREAM_WINDOW), Steer::Kept);
    assert!(
        fixture
            .state
            .scanout_feedback
            .for_new_surface(&surface)
            .is_some()
    );
}

#[test]
fn an_idle_session_does_not_count_and_a_capture_does() {
    let mut fixture = Fixture::start_opaque();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    assert!(fixture.state.act(Action::ToggleFullscreen));
    fixture.settle();
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);

    // A session that exists but asks for nothing -- a thumbnail source
    // between refreshes -- leaves the output free to go direct.
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);

    // A request -- still parked, or already answered by the time this
    // asks -- is a stream: the frame that answers the next one must be a
    // composite, not a direct frame the capture then has to force.
    fixture.run(Step::CaptureWithoutWaiting);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Streaming);

    // Delivered, and not yet re-requested: still a stream for the window
    // after the request, which is what covers the gap before a recorder's
    // next request.
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Streaming);
}

#[test]
fn a_session_ending_ends_the_stream() {
    let mut fixture = Fixture::start_opaque();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    assert!(fixture.state.act(Action::ToggleFullscreen));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::CaptureWithoutWaiting);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Streaming);
    fixture.run(Step::DestroySession);
    fixture.settle();
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn a_frame_parked_past_the_window_still_counts() {
    // A recorder waiting on a still screen: its frame stays parked (nothing
    // is due until the pixels move) for longer than the window. It is still
    // a stream, so the frame that ends the pause is composited rather than
    // forced. Asked at a `now` well past the window rather than by sleeping.
    let mut fixture = Fixture::start_opaque();
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
    assert_eq!(outcome, Outcome::Ready, "the first frame is always due");
    // The second waits: the screen has not changed since the first.
    fixture.run(Step::CaptureWithoutWaiting);
    fixture.settle();
    assert_eq!(
        fixture.run(Step::PollFrame).frame().0,
        Outcome::Waiting,
        "the test's own premise: the frame is parked"
    );
    let output = fixture.state.outputs.primary_id().expect("an output");
    let later = Instant::now() + STREAM_WINDOW * 5;
    assert!(fixture.state.screencopy.streaming(output, later));
    fixture.run(Step::DestroySession);
    fixture.settle();
    assert!(!fixture.state.screencopy.streaming(output, later));
}

#[test]
fn a_stream_on_an_uncovered_output_changes_nothing() {
    // The stream rule only ever runs on a covered output: without a
    // fullscreen window the answer is `NotCovered` with or without a capture.
    let mut fixture = Fixture::start_opaque();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    fixture.run(Step::CaptureWithoutWaiting);
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NotCovered
    );
}

#[test]
fn the_window_is_strict_and_a_stamp_from_the_future_is_new() {
    let at = Instant::now();
    assert!(!requested_recently(None, at), "never asked");
    assert!(requested_recently(Some(at), at));
    let just_inside = STREAM_WINDOW - Duration::from_millis(1);
    assert!(requested_recently(Some(at), at + just_inside));
    assert!(
        !requested_recently(Some(at), at + STREAM_WINDOW),
        "strictly inside the window: at exactly one window it has lapsed"
    );
    assert!(!requested_recently(Some(at), at + STREAM_WINDOW * 10));
    // A stamp taken after `now` was read (the clock read first, the request
    // stamped later in the same tick) is brand new, not a panic or a wrap.
    assert!(requested_recently(Some(at + Duration::from_millis(5)), at));
}
