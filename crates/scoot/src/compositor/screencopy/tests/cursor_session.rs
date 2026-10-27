//! Pointer cursor sessions (`create_pointer_cursor_session`,
//! `screencopy/cursor_session.rs`): the cursor image in a buffer of its own,
//! as wayvnc asks for it.
//!
//! Each test drives a real client over the wire. The image is checked
//! against an **oracle** -- the same cursor drawn into a whole screenshot
//! (`capture_pixels_for` with the pointer in and out) -- so "the image is
//! right" means "it is exactly the pixels the pointer adds to the screen,
//! at the hotspot the events named".

use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::FailureReason;

use super::*;

/// A theme name nothing resolves, so the cursor is this compositor's own
/// drawn arrow whatever the machine running the suite has installed.
const NO_THEME: &str = "scoot-test-no-such-theme";

const ARGB8888: u32 = wl_shm::Format::Argb8888 as u32;
const BUFFER_CONSTRAINTS: u32 = FailureReason::BufferConstraints as u32;

fn start() -> Fixture {
    let appearance = Appearance {
        cursor_theme: Some(NO_THEME.to_owned()),
        ..opaque_appearance()
    };
    let mut fixture = Harness::headless(appearance, CANVAS);
    fixture.spawn(run_client);
    fixture
}

fn primary(fixture: &Fixture) -> scoot_core::OutputId {
    fixture.state.outputs.primary_id().expect("an output")
}

/// A whole-screen capture of the primary output with the pointer in or out.
fn screen(fixture: &mut Fixture, cursor: bool) -> Vec<u8> {
    let id = primary(fixture);
    fixture
        .state
        .capture_pixels_for(Some(id), cursor)
        .expect("a capture")
        .bgra
}

/// The pointer moved by the compositor's own input path, with the frame
/// tick left to the test.
fn move_pointer(fixture: &mut Fixture, x: f64, y: f64) {
    fixture.state.pointer_move(x, y);
}

/// Starts a cursor session and returns the constraint batch it was told.
fn open(fixture: &mut Fixture) -> Constraints {
    fixture.run(Step::StartCursorSession).constraints()
}

/// Captures one cursor frame sized as the session was told.
fn take(fixture: &mut Fixture, told: &Constraints) -> (Outcome, Vec<u8>) {
    fixture
        .run(Step::Capture {
            width: told.width as i32,
            height: told.height as i32,
            format: wl_shm::Format::Argb8888,
        })
        .frame()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
fn a_cursor_session_is_told_the_images_size_and_argb_only() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert!(!told.stopped, "the session is served, not refused");
    assert_eq!(told.dones, 1);
    let edge = crate::compositor::cursor::physical_size(fixture.state.cursor.logical_size(), 1.0);
    assert_eq!((told.width, told.height), (edge as u32, edge as u32));
    assert_eq!(
        told.formats,
        vec![ARGB8888],
        "a cursor needs its alpha: no Xrgb8888 offered"
    );
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (1, 1));
}

#[test]
fn the_first_frame_is_the_pointer_exactly_as_the_screen_shows_it() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    fixture.render();
    let told = open(&mut fixture);
    let (outcome, image) = take(&mut fixture, &told);
    assert_eq!(outcome, Outcome::Ready, "a first frame never waits");

    let events = fixture.run(Step::ReadCursorEvents).cursor_events();
    let [
        CursorEvent::Enter,
        CursorEvent::Hotspot(hx, hy),
        CursorEvent::Position(px, py),
    ] = events[..]
    else {
        panic!("enter, hotspot, position in that order, got {events:?}");
    };
    assert_eq!((px, py), (20, 20), "the hotspot's place on the output");

    let with = screen(&mut fixture, true);
    let without = screen(&mut fixture, false);
    let (ox, oy) = (px - hx, py - hy);
    let mut opaque = 0;
    for y in 0..told.height {
        for x in 0..told.width {
            let cursor = pixel_at(&image, told.width, x, y);
            let (sx, sy) = (ox as u32 + x, oy as u32 + y);
            if cursor[3] == 0xff {
                opaque += 1;
                assert_eq!(cursor, pixel_at(&with, CANVAS as u32, sx, sy), "({x},{y})");
            } else if cursor[3] == 0 {
                assert_eq!(
                    pixel_at(&with, CANVAS as u32, sx, sy),
                    pixel_at(&without, CANVAS as u32, sx, sy),
                    "a transparent cursor pixel changes nothing on screen ({x},{y})"
                );
            }
        }
    }
    assert!(opaque > 10, "the arrow has opaque pixels, got {opaque}");
}

#[test]
fn pointer_motion_alone_serves_no_frame_but_moves_the_position() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);
    fixture.run(Step::ReadCursorEvents);

    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    assert_eq!(
        fixture.run(Step::PollFrame).frame().0,
        Outcome::Waiting,
        "nothing changed yet"
    );

    for step in 0..50 {
        move_pointer(&mut fixture, 21.0 + f64::from(step % 30), 25.0);
    }
    move_pointer(&mut fixture, 40.0, 30.0);
    assert_eq!(
        fixture.run(Step::PollFrame).frame().0,
        Outcome::Waiting,
        "fifty motion events are not a new image"
    );
    assert_eq!(
        fixture.run(Step::ReadCursorEvents).cursor_events(),
        vec![CursorEvent::Position(40, 30)],
        "the position event is coalesced to the tick: only the last one"
    );
}

#[test]
fn a_shape_change_serves_the_parked_frame_with_the_new_image() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    let (_, arrow) = take(&mut fixture, &told);
    fixture.run(Step::ReadCursorEvents);
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });

    fixture
        .state
        .cursor
        .set_status(CursorImageStatus::Named(CursorIcon::Text));
    fixture.state.cursor_image_changed();
    let (outcome, text) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_ne!(text, arrow, "the I-beam is not the arrow");
    assert!(
        text.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0xff),
        "and it has opaque pixels"
    );
    let events = fixture.run(Step::ReadCursorEvents).cursor_events();
    assert!(
        matches!(events[..], [CursorEvent::Hotspot(..)]),
        "the new hotspot; the position did not move, so none is sent: {events:?}"
    );
}

#[test]
fn a_hidden_cursor_is_a_transparent_frame_not_a_failure_and_leaves() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    let (_, arrow) = take(&mut fixture, &told);
    fixture.run(Step::ReadCursorEvents);

    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    fixture.state.cursor.set_status(CursorImageStatus::Hidden);
    fixture.state.cursor_image_changed();
    let (outcome, hidden) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Ready,
        "wayvnc ends cursor capture for any failure but buffer_constraints"
    );
    assert!(hidden.iter().all(|byte| *byte == 0), "fully transparent");
    assert_eq!(
        fixture.run(Step::ReadCursorEvents).cursor_events(),
        vec![CursorEvent::Leave]
    );
    assert_eq!(
        fixture.run(Step::ReadConstraints).constraints().dones,
        1,
        "hiding keeps the size: no new constraint batch"
    );

    // And back: the arrow, entered again.
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    fixture
        .state
        .cursor
        .set_status(CursorImageStatus::default_named());
    fixture.state.cursor_image_changed();
    let (outcome, shown) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(shown, arrow);
    let events = fixture.run(Step::ReadCursorEvents).cursor_events();
    assert!(matches!(
        events[..],
        [
            CursorEvent::Enter,
            CursorEvent::Hotspot(..),
            CursorEvent::Position(20, 20)
        ]
    ));
}

#[test]
fn an_image_of_another_size_is_announced_and_the_old_buffer_refused() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);

    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    // A bigger cursor, as a reload with another `cursor_size` makes.
    let bigger = (told.width as i32 + 8).min(32);
    let color = fixture.state.appearance.cursor_color;
    fixture.state.cursor.rebuild(bigger, color, Some(NO_THEME));
    fixture.state.cursor_image_changed();
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(
        outcome,
        Outcome::Failed(BUFFER_CONSTRAINTS),
        "a buffer sized for the old image is refused, never written past"
    );
    let now = fixture.run(Step::ReadConstraints).constraints();
    assert_eq!(now.dones, 2, "the new size is announced");
    assert_eq!((now.width, now.height), (bigger as u32, bigger as u32));
    assert_eq!(now.formats, vec![ARGB8888]);

    let (outcome, image) = take(&mut fixture, &now);
    assert_eq!(outcome, Outcome::Ready, "and the retry is served at once");
    assert_eq!(image.len(), (bigger * bigger * 4) as usize);
}

#[test]
fn a_cursor_on_the_screen_edge_keeps_its_whole_image() {
    // The pointer clamps at the edge, but the arrow hangs over it: the image
    // is the whole arrow, not what fits on the output.
    let mut fixture = start();
    let edge = f64::from(CANVAS) - 0.5;
    move_pointer(&mut fixture, edge, edge);
    let told = open(&mut fixture);
    let (outcome, image) = take(&mut fixture, &told);
    assert_eq!(outcome, Outcome::Ready);
    assert!(
        image
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] == 0xff)
    );
    let edge_events = fixture.run(Step::ReadCursorEvents).cursor_events();
    assert!(
        matches!(edge_events[0], CursorEvent::Enter),
        "{edge_events:?}"
    );
}

#[test]
fn destroying_the_session_or_disconnecting_leaves_nothing_behind() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);
    // A frame parked when the session goes away.
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    fixture.run(Step::DestroyCursorSession);
    fixture.settle();
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (0, 0));
    assert_eq!(
        fixture.state.screencopy.frames_in_flight(),
        1,
        "the frame object lives on"
    );

    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (1, 1));
    fixture.disconnect(0);
    fixture.settle();
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (0, 0));
    assert_eq!(fixture.state.screencopy.frames_in_flight(), 0);
}

#[test]
fn a_removed_output_stops_its_cursor_sessions() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    let id = primary(&fixture);
    fixture.state.stop_captures_on(id);
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Failed(FailureReason::Stopped as u32));
    assert!(fixture.run(Step::ReadConstraints).constraints().stopped);
    assert_eq!(fixture.state.screencopy.cursor_session_count().0, 0);
}

#[test]
fn a_frame_flood_on_a_cursor_session_is_refused_like_any_other() {
    let mut fixture = start();
    open(&mut fixture);
    let error = fixture.run_expecting_disconnect(Step::FloodFrames {
        count: MAX_FRAMES_PER_CLIENT + 5,
    });
    assert!(error.contains("duplicate_frame"), "got: {error}");
    fixture.settle();
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (0, 0));
    assert_eq!(fixture.state.screencopy.frames_in_flight(), 0);
}

#[test]
fn a_motion_event_costs_nothing_with_no_cursor_session_and_arms_the_tick_with_one() {
    let mut fixture = start();
    fixture.settle();
    fixture.state.timer_armed = false;
    move_pointer(&mut fixture, 30.0, 30.0);
    assert!(
        !fixture.state.timer_armed,
        "no cursor session: a motion arms nothing on a backend that draws no cursor"
    );

    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    fixture.settle();
    fixture.tick(Duration::from_millis(60));
    assert!(
        !fixture.state.timer_armed,
        "a parked frame keeps no timer alive"
    );
    move_pointer(&mut fixture, 31.0, 30.0);
    assert!(
        fixture.state.timer_armed,
        "a parked cursor frame wants the tick"
    );
}

#[test]
fn a_pointer_that_leaves_the_output_and_returns_enters_again_with_the_next_frame() {
    let mut fixture = start();
    move_pointer(&mut fixture, 20.0, 20.0);
    let told = open(&mut fixture);
    assert_eq!(take(&mut fixture, &told).0, Outcome::Ready);
    fixture.run(Step::ReadCursorEvents);

    // The session is on the first output; a second one takes the pointer.
    crate::compositor::headless::add_output(&mut fixture.state, "second", CANVAS, CANVAS)
        .expect("a second output");
    move_pointer(&mut fixture, f64::from(CANVAS) + 20.0, 20.0);
    fixture.state.ensure_ticking();
    fixture.tick(Duration::from_millis(60));
    assert_eq!(
        fixture.run(Step::ReadCursorEvents).cursor_events(),
        vec![CursorEvent::Leave],
        "the pointer is over the other output"
    );

    // Back, with the same image, and nothing parked while it was gone.
    move_pointer(&mut fixture, 20.0, 20.0);
    fixture.run(Step::CaptureSizedWithoutWaiting {
        width: told.width as i32,
        height: told.height as i32,
    });
    let (outcome, _) = fixture.run(Step::PollFrame).frame();
    assert_eq!(outcome, Outcome::Ready, "the pointer returning is news");
    let events = fixture.run(Step::ReadCursorEvents).cursor_events();
    assert!(
        matches!(
            events[..],
            [
                CursorEvent::Enter,
                CursorEvent::Hotspot(..),
                CursorEvent::Position(20, 20)
            ]
        ),
        "{events:?}"
    );
}
