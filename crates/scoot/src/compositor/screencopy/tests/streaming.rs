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

use super::*;
use crate::compositor::render::PrimaryDirect;
use crate::compositor::screencopy::{STREAM_WINDOW, requested_recently};

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

    // A parked request is a stream: the frame that answers it must be a
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
