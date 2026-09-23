//! The pointer in a capture is the request's, not the tier's
//! (`render::capture_cursor`).
//!
//! Every test compares a capture against an **oracle**: the same scene
//! rendered as a whole frame by the same renderer with the cursor
//! composited into it, or without it. The harness has no `--tty`, so the
//! two frame shapes are chosen with the `frame_cursor_for_test` seam:
//! `Some(true)` is the dumb tier's frame (cursor composited in), `None` is
//! headless's own (never drawn) -- and, for the capture, the same shape as a
//! scanout frame whose cursor rode a plane (the record differs only in
//! `off_frame`, which `patch_region`'s unit tests cover).
//!
//! "Equals the oracle" is the whole claim: a capture that asked for the
//! pointer is byte-for-byte what a composite with the pointer would have
//! drawn, and one that did not is byte-for-byte a composite without it --
//! whichever the frame it was read from held.

use smithay::input::pointer::{CursorIcon, CursorImageStatus};

use super::*;
use crate::compositor::render::CursorInFrame;

/// A theme name nothing resolves, so the cursor is this compositor's own
/// drawn arrow whatever the machine running the suite has installed (see
/// `cursor/tests.rs`'s `NO_THEME`).
const NO_THEME: &str = "scoot-test-no-such-theme";

fn cursor_appearance(corner_radius: i32) -> Appearance {
    Appearance {
        cursor_theme: Some(NO_THEME.to_owned()),
        corner_radius,
        ..opaque_appearance()
    }
}

fn start_with(appearance: Appearance, scale: f64) -> Fixture {
    let mut fixture = Harness::headless_scaled(appearance, CANVAS, scale);
    fixture.spawn(run_client);
    fixture
}

fn start() -> Fixture {
    start_with(cursor_appearance(0), 1.0)
}

fn primary(fixture: &Fixture) -> scoot_core::OutputId {
    fixture.state.outputs.primary_id().expect("an output")
}

/// The whole-frame oracle: the current scene drawn with the cursor
/// composited in (`with`) or not, by the harness's own renderer. Leaves the
/// frame seam at `restore` and the framebuffer drawn in that shape again, so
/// the test goes on reading the frame it set up.
fn oracle(fixture: &mut Fixture, with: bool, restore: Option<bool>) -> Vec<u8> {
    fixture.state.frame_cursor_for_test = Some(with);
    let pixels = fixture.render();
    fixture.state.frame_cursor_for_test = restore;
    fixture.render();
    pixels
}

/// An IPC capture of the primary output, `cursor` as asked.
fn shot(fixture: &mut Fixture, cursor: bool) -> Vec<u8> {
    let id = primary(fixture);
    fixture
        .state
        .capture_pixels_for(Some(id), cursor)
        .expect("a capture")
        .bgra
}

/// How many pixels differ between two frames.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

/// Asserts `captured` is the oracle byte for byte, with a readable message
/// (a 14 KiB byte-slice diff is not one).
fn assert_is(captured: &[u8], oracle: &[u8], what: &str) {
    assert_eq!(captured.len(), oracle.len(), "{what}: size");
    let off = differing(captured, oracle);
    assert_eq!(off, 0, "{what}: {off} pixels differ from the oracle");
}

/// The cursor really is on screen in the with-oracle: otherwise every
/// "equals the oracle" below would hold for a capture with no cursor at all.
fn assert_cursor_shows(with: &[u8], without: &[u8]) {
    assert!(
        differing(with, without) > 0,
        "the scene must show the cursor somewhere, or these tests prove nothing"
    );
}

#[test]
fn a_headless_screenshot_draws_the_pointer_the_frame_never_did() {
    let mut fixture = start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.state.pointer_move(20.0, 20.0);
    fixture.render();
    let framebuffer = fixture.pixels();
    let with = oracle(&mut fixture, true, None);
    let without = oracle(&mut fixture, false, None);
    assert_cursor_shows(&with, &without);
    assert_is(
        &framebuffer,
        &without,
        "a headless frame never holds the cursor",
    );

    assert_is(&shot(&mut fixture, true), &with, "asked for the pointer");
    assert_is(
        &shot(&mut fixture, false),
        &without,
        "asked to leave it out",
    );
    assert_is(
        &fixture.pixels(),
        &framebuffer,
        "a capture must not touch the framebuffer it reads",
    );
}

#[test]
fn leaving_the_pointer_out_takes_it_out_of_a_frame_that_composited_it() {
    // The dumb tier's shape: every frame holds the cursor.
    let mut fixture = start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.state.pointer_move(28.0, 22.0);
    fixture.render();
    let framebuffer = fixture.pixels();
    let with = oracle(&mut fixture, true, Some(true));
    let without = oracle(&mut fixture, false, Some(true));
    assert_cursor_shows(&with, &without);
    assert_is(&framebuffer, &with, "the frame holds the cursor");

    assert_is(&shot(&mut fixture, false), &without, "taken out");
    assert_is(
        &shot(&mut fixture, true),
        &with,
        "already there, left alone",
    );
}

#[test]
fn a_frame_already_matching_the_request_costs_no_region() {
    // The two cheap cases, pinned on the decision itself rather than on
    // timing: nothing is rendered when the frame already is the answer.
    let mut fixture = start();
    fixture.render();
    let id = primary(&fixture);
    let mut backend = fixture.state.take_backend(id).expect("a backend");
    assert!(
        fixture
            .state
            .capture_cursor_patch(&mut backend, id, false)
            .is_none(),
        "a cursorless frame asked for no cursor"
    );
    fixture.state.put_backend(id, backend);

    fixture.state.frame_cursor_for_test = Some(true);
    fixture.render();
    let mut backend = fixture.state.take_backend(id).expect("a backend");
    assert!(
        fixture
            .state
            .capture_cursor_patch(&mut backend, id, true)
            .is_none(),
        "a frame holding the cursor where it is, asked for the cursor"
    );
    fixture.state.put_backend(id, backend);
}

#[test]
fn a_pointer_that_moved_since_the_frame_leaves_exactly_one_cursor() {
    // The recorded frame holds the cursor at one place and the pointer is
    // now somewhere else, with no frame in between: the region re-rendered
    // is both places, so the capture shows the pointer once, where it is.
    let mut fixture = start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.state.pointer_move(8.0, 8.0);
    fixture.render();
    fixture.state.pointer_move(40.0, 36.0);
    // No render: read the stale frame and patch it by hand, which is what
    // both capture paths do after their own (here skipped) render.
    let id = primary(&fixture);
    let mut backend = fixture.state.take_backend(id).expect("a backend");
    let mut captured = backend.capture(<[u8]>::to_vec).expect("a read-back");
    let patch = fixture
        .state
        .capture_cursor_patch(&mut backend, id, true)
        .expect("the pointer moved, so the frame does not match");
    patch.apply(&mut captured, CANVAS, CANVAS);
    fixture.state.put_backend(id, backend);

    let with = oracle(&mut fixture, true, Some(true));
    assert_is(&captured, &with, "one cursor, at the pointer");
}

#[test]
fn the_region_keeps_a_rounded_windows_corner_cut() {
    // `Rounded` cuts its corners against a clip in output coordinates; the
    // region is re-rendered relocated to its own origin, which only lands
    // the cut in the right place because `Rounded::relocate` moves the clip
    // with the element. Pointer just outside the window's top-left corner,
    // so the arrow (which grows right and down from its hotspot) covers the
    // cut.
    let mut fixture = start_with(cursor_appearance(10), 1.0);
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.settle();
    let window = fixture
        .state
        .world
        .arrange()
        .placements
        .first()
        .expect("the window's placement")
        .rect;
    fixture
        .state
        .pointer_move(f64::from(window.x - 1), f64::from(window.y - 1));
    fixture.render();
    let with = oracle(&mut fixture, true, None);
    let without = oracle(&mut fixture, false, None);
    assert_cursor_shows(&with, &without);
    assert_is(
        &shot(&mut fixture, true),
        &with,
        "the corner under the cursor",
    );
}

#[test]
fn the_region_matches_at_a_fractional_scale() {
    let mut fixture = start_with(cursor_appearance(0), 1.5);
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.state.pointer_move(13.3, 17.7);
    fixture.render();
    let with = oracle(&mut fixture, true, None);
    let without = oracle(&mut fixture, false, None);
    assert_cursor_shows(&with, &without);
    assert_is(&shot(&mut fixture, true), &with, "drawn in at 1.5");

    fixture.state.frame_cursor_for_test = Some(true);
    fixture.render();
    assert_is(&shot(&mut fixture, false), &without, "taken out at 1.5");
}

#[test]
fn a_pointer_at_the_output_edge_is_clamped_not_overrun() {
    // Most of the arrow lies past the right and bottom edges: the region is
    // clamped to the output, and the part on screen still matches.
    let mut fixture = start();
    let edge = f64::from(CANVAS) - 3.0;
    fixture.state.pointer_move(edge, edge);
    fixture.render();
    let with = oracle(&mut fixture, true, None);
    let without = oracle(&mut fixture, false, None);
    assert_cursor_shows(&with, &without);
    assert_is(&shot(&mut fixture, true), &with, "clamped at the edge");
}

#[test]
fn a_hidden_cursor_is_in_no_capture() {
    let mut fixture = start();
    fixture.state.pointer_move(20.0, 20.0);
    fixture.state.cursor.set_status(CursorImageStatus::Hidden);
    fixture.state.cursor_changed();
    fixture.render();
    let framebuffer = fixture.pixels();
    assert_is(&shot(&mut fixture, true), &framebuffer, "headless, asked");
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.render();
    assert_is(
        &shot(&mut fixture, false),
        &framebuffer,
        "composited, not asked",
    );
    assert_is(&shot(&mut fixture, true), &framebuffer, "composited, asked");
}

#[test]
fn a_named_shape_is_drawn_as_the_frame_would_draw_it() {
    // `wp-cursor-shape-v1` and `set_cursor(NULL)`-with-a-name both land here:
    // the capture draws the shape the frame would, not the arrow.
    let mut fixture = start();
    fixture.state.pointer_move(20.0, 20.0);
    fixture.render();
    let arrow = shot(&mut fixture, true);
    fixture
        .state
        .cursor
        .set_status(CursorImageStatus::Named(CursorIcon::Text));
    fixture.state.cursor_changed();
    fixture.render();
    let with = oracle(&mut fixture, true, None);
    let captured = shot(&mut fixture, true);
    assert_is(&captured, &with, "the I-beam");
    assert!(differing(&captured, &arrow) > 0, "and not the arrow");
}

#[test]
fn a_locked_capture_shows_the_pointer_over_the_lock_screen_and_no_window() {
    let mut fixture = start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.run(Step::Lock);
    fixture.state.pointer_move(20.0, 20.0);
    fixture.render();
    let with = oracle(&mut fixture, true, None);
    let without = oracle(&mut fixture, false, None);
    assert_cursor_shows(&with, &without);
    let captured = shot(&mut fixture, true);
    assert_is(&captured, &with, "the lock screen with the pointer");
    assert!(
        !captured
            .chunks_exact(4)
            .any(|pixel| pixel == WINDOW_BGRA.as_slice()),
        "the region is gathered locked: no window pixel may reach it"
    );
}

#[test]
fn ext_capture_honours_paint_cursors_both_ways_on_both_frame_shapes() {
    for composited in [None, Some(true)] {
        for paint_cursors in [true, false] {
            let mut fixture = start();
            fixture.run(Step::MapWindow(WINDOW_BGRA));
            fixture.state.frame_cursor_for_test = composited;
            fixture.state.pointer_move(18.0, 26.0);
            fixture.render();
            let expected = oracle(&mut fixture, paint_cursors, composited);
            fixture.run(Step::StartSession { paint_cursors });
            let (outcome, captured) = fixture
                .run(Step::Capture {
                    width: CANVAS,
                    height: CANVAS,
                    format: wl_shm::Format::Argb8888,
                })
                .frame();
            assert_eq!(outcome, Outcome::Ready);
            assert_is(
                &captured,
                &expected,
                &format!("paint_cursors={paint_cursors}, frame composited={composited:?}"),
            );
        }
    }
}

#[test]
fn an_xrgb_capture_forces_the_regions_alpha_too() {
    // Over a translucent background the region's own clear leaves alpha
    // below 0xff exactly like the frame's: the opacity pass has to run over
    // the patched pixels as well.
    let mut fixture = Harness::headless(
        Appearance {
            cursor_theme: Some(NO_THEME.to_owned()),
            ..appearance()
        },
        CANVAS,
    );
    fixture.spawn(run_client);
    fixture.state.pointer_move(20.0, 20.0);
    fixture.render();
    fixture.run(Step::StartSession {
        paint_cursors: true,
    });
    let (outcome, captured) = fixture
        .run(Step::Capture {
            width: CANVAS,
            height: CANVAS,
            format: wl_shm::Format::Xrgb8888,
        })
        .frame();
    assert_eq!(outcome, Outcome::Ready);
    assert!(
        captured.chunks_exact(4).all(|pixel| pixel[3] == 0xFF),
        "every alpha byte, the region's included, forced opaque"
    );
}

#[test]
fn a_pointer_move_serves_a_parked_cursor_capture_and_only_that_one() {
    // Headless draws no cursor, so a pointer move redraws nothing and moves
    // no frame serial -- yet a session that asked for the pointer has a new
    // picture to be handed. One that did not, has not.
    for paint_cursors in [true, false] {
        let mut fixture = start();
        fixture.state.pointer_move(10.0, 10.0);
        fixture.render();
        fixture.run(Step::StartSession { paint_cursors });
        let (outcome, _) = fixture
            .run(Step::Capture {
                width: CANVAS,
                height: CANVAS,
                format: wl_shm::Format::Argb8888,
            })
            .frame();
        assert_eq!(outcome, Outcome::Ready, "the first capture never waits");
        fixture.run(Step::CaptureWithoutWaiting);
        let (outcome, _) = fixture.run(Step::PollFrame).frame();
        assert_eq!(outcome, Outcome::Waiting, "nothing changed yet");

        let serial = fixture.state.frame_serial;
        fixture.state.pointer_move(40.0, 30.0);
        assert!(
            !fixture.state.needs_render,
            "headless: a pointer move must not cost a render"
        );
        let (outcome, captured) = fixture.run(Step::PollFrame).frame();
        assert_eq!(fixture.state.frame_serial, serial, "no frame was drawn");
        if paint_cursors {
            assert_eq!(outcome, Outcome::Ready, "the pointer is its content");
            let with = oracle(&mut fixture, true, None);
            assert_is(&captured, &with, "served with the pointer where it is now");
        } else {
            assert_eq!(
                outcome,
                Outcome::Waiting,
                "a cursorless session's content did not change"
            );
        }
    }
}

#[test]
fn repeated_cursor_captures_of_a_still_screen_draw_no_frame() {
    // The region render must stay out of the frame lifecycle: no dirty
    // flag, no serial -- or a cursor-painting client on a still screen
    // would keep the compositor rendering forever.
    let mut fixture = start();
    fixture.run(Step::MapWindow(WINDOW_BGRA));
    fixture.state.pointer_move(20.0, 20.0);
    fixture.settle();
    fixture.render();
    let serial = fixture.state.frame_serial;
    fixture.run(Step::StartSession {
        paint_cursors: true,
    });
    for _ in 0..3 {
        let _ = shot(&mut fixture, true);
    }
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
    assert_eq!(outcome, Outcome::Waiting, "a still screen is not re-served");
    assert!(!fixture.state.needs_render, "nothing asked for a frame");
    assert_eq!(fixture.state.frame_serial, serial, "no frame was drawn");
    fixture.settle();
    assert!(
        !fixture.state.timer_armed,
        "and the frame timer went idle with a cursor capture parked"
    );
}

#[test]
fn the_frame_records_what_it_composited_of_the_cursor() {
    // The source of truth the capture decision reads, per frame: nothing on
    // headless, the cursor's footprint when the frame draws it.
    let mut fixture = start();
    fixture.state.pointer_move(20.0, 20.0);
    fixture.render();
    let id = primary(&fixture);
    assert_eq!(
        fixture
            .state
            .backends
            .get(&id)
            .expect("a backend")
            .cursor_in_frame(),
        CursorInFrame::default()
    );
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.render();
    let record = fixture
        .state
        .backends
        .get(&id)
        .expect("a backend")
        .cursor_in_frame();
    let footprint = record.composited.expect("the frame drew the cursor");
    assert_eq!(
        (footprint.loc.x, footprint.loc.y),
        (20, 20),
        "the arrow's hotspot is its tip"
    );
    assert!(!record.off_frame);
}
