//! What reaches the framebuffer and the pointer: a floating window over the
//! strip, its ring over the window beneath it, its popups fitted to its
//! output, nothing past its output, nothing over the lock screen.
//!
//! `Harness::pixels` reads the frame back through `Backend::capture`, the
//! same read-back IPC `screenshot` and `ext-image-copy-capture-v1` use, so a
//! floating window in these pixels is a floating window in a capture.

use scoot_core::OutputId;

use super::*;
use crate::compositor::headless;

/// Square, and rounded -- the painted-ring path, with the rounded clip on
/// the floating window -- which is what a session with `corner_radius` set
/// draws. The probes sit at a window's vertical centre, on the straight
/// part of the ring either way.
const RADII: [i32; 2] = [0, 10];

/// The ring of a floating window is drawn over the tiled window beneath it,
/// not under it: the pixel just outside the dialog, which lies inside its
/// parent's column, is ring-coloured.
#[test]
fn a_floating_window_s_ring_is_drawn_over_the_window_beneath_it() {
    for radius in RADII {
        ring_over_the_window_beneath(radius);
    }
}

fn ring_over_the_window_beneath(radius: i32) {
    let mut fixture = Fixture::new();
    fixture.state.appearance.corner_radius = radius;
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let column = fixture.placement(0).rect;
    let placed = fixture.placement(dialog).rect;
    let (cx, cy) = centre(placed);
    let outside = placed.x - 1;
    assert!(
        column.contains(scoot_core::Point::new(outside, cy)),
        "the probe must be over the column: {column:?} {placed:?}"
    );
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, cx, cy), DIALOG_BGRA);
    assert_eq!(
        pixel(&pixels, outside, cy),
        RING_BGRA,
        "ring under the column (radius {radius})"
    );
    assert_eq!(pixel(&pixels, placed.x - RING - 1, cy), TILED_BGRA);
    if radius > 0 {
        // The rounded clip cuts the dialog's own corner: its top-left pixel
        // is not the dialog's colour.
        assert_ne!(pixel(&pixels, placed.x, placed.y), DIALOG_BGRA);
    }
}

/// Two overlapping floating windows: the upper one's ring is drawn over the
/// lower one, which only drawing each ring directly under its own window
/// gets right (all floating rings under all floating windows would bury it).
#[test]
fn stacked_floating_windows_each_draw_their_ring_over_what_is_below() {
    for radius in RADII {
        stacked_rings(radius);
    }
}

fn stacked_rings(radius: i32) {
    let mut fixture = Fixture::new();
    fixture.state.appearance.corner_radius = radius;
    fixture.map(Spec::tiled());
    let lower = fixture.map(Spec {
        color: OTHER_BGRA,
        dialog: true,
        natural: (160, 120),
        ..Spec::tiled()
    });
    let upper = fixture.map(Spec {
        color: DIALOG_BGRA,
        dialog: true,
        ..Spec::tiled()
    });
    let big = fixture.placement(lower).rect;
    let small = fixture.placement(upper).rect;
    let (_, cy) = centre(small);
    assert!(big.contains(scoot_core::Point::new(small.x - RING - 1, cy)));
    let pixels = fixture.render();
    assert_eq!(
        pixel(&pixels, small.x - 1, cy),
        RING_BGRA,
        "radius {radius}"
    );
    assert_eq!(pixel(&pixels, small.x - RING - 1, cy), OTHER_BGRA);
}

/// The lock screen hides a floating window like everything else, and the
/// pointer cannot reach it through the lock.
#[test]
fn the_lock_screen_covers_a_floating_window() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let (cx, cy) = centre(fixture.placement(dialog).rect);
    assert!(test_support::contains(&fixture.render(), DIALOG_BGRA));
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    let pixels = fixture.render();
    assert!(
        !test_support::contains(&pixels, DIALOG_BGRA),
        "a floating window shows through the lock"
    );
    assert!(!test_support::contains(&pixels, RING_BGRA));
    let under = fixture
        .state
        .surface_under((f64::from(cx), f64::from(cy)).into());
    let dialog_surface = fixture
        .state
        .windows
        .get(&fixture.id(dialog))
        .and_then(smithay::desktop::Window::toplevel)
        .map(|toplevel| toplevel.wl_surface().clone());
    assert!(under.map(|(surface, _)| surface) != dialog_surface);
}

/// A floating window that draws itself larger than its output (and ignores
/// being asked to fit) is still drawn only on its own output.
#[test]
fn a_floating_window_is_clipped_to_its_output() {
    let mut fixture = Fixture::new();
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.settle();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    assert_eq!(fixture.placement(dialog).output, OutputId(1));
    fixture.done(Step::DrawSized {
        window: dialog,
        width: 2 * CANVAS,
        height: 2 * CANVAS,
    });
    let placed = fixture.placement(dialog);
    assert_eq!(placed.rect, Rect::new(0, 0, CANVAS, CANVAS), "{placed:?}");
    assert_eq!(
        placed.requested,
        Some(scoot_core::Size::new(CANVAS, CANVAS)),
        "asked to fit"
    );
    fixture.render();
    assert!(test_support::contains(
        &fixture.pixels_of(OutputId(1)),
        DIALOG_BGRA
    ));
    assert!(
        !test_support::contains(&fixture.pixels_of(OutputId(2)), DIALOG_BGRA),
        "the floating window bled onto the next output"
    );
}

/// A floating window's popup is fitted into its output's usable area like
/// any window's: anchored at the dialog's bottom-right corner and too big to
/// fit below and right of it, it is flipped or slid back on screen.
#[test]
fn a_floating_window_s_popup_is_fitted_into_its_output() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let placed = fixture.placement(dialog).rect;
    let (w, h) = (NATURAL.0, NATURAL.1);
    let popup = fixture.popup(dialog, (w - 1, h - 1, 1, 1), (150, 150));
    let global = Rect::new(placed.x + popup.x, placed.y + popup.y, popup.w, popup.h);
    let output = Rect::new(0, 0, CANVAS, CANVAS);
    assert_eq!(
        global.intersection(output),
        global,
        "the popup {global:?} runs off the output"
    );
}

/// A dialog appears where the user just clicked. The next click, without
/// the pointer moving, must go to the dialog -- `wl_pointer.button` goes to
/// the surface last entered, so the dialog appearing has to re-derive that.
#[test]
fn a_dialog_appearing_under_a_still_pointer_takes_the_pointer() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let (x, y) = centre(fixture.placement(0).rect);
    fixture.state.pointer_move(f64::from(x), f64::from(y));
    fixture.settle();
    assert_eq!(fixture.pointer(), Some(Entered::Window(0)));
    let dialog = fixture.map(Spec::dialog_of(0));
    assert!(
        fixture
            .placement(dialog)
            .rect
            .contains(scoot_core::Point::new(x, y))
    );
    assert_eq!(fixture.pointer(), Some(Entered::Window(dialog)));
}

/// The strip under a floating window is drawn exactly as without it, bar
/// the pixels the window and its ring cover.
#[test]
fn the_strip_draws_as_before_around_a_floating_window() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let before = fixture.render();
    let dialog = fixture.map(Spec::dialog_of(0));
    let covered = fixture.placement(dialog).rect.inset(-RING);
    let after = fixture.render();
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            if covered.contains(scoot_core::Point::new(x, y)) {
                continue;
            }
            let (was, now) = (pixel(&before, x, y), pixel(&after, x, y));
            // The column's ring goes inactive-coloured (it lost focus); the
            // test palette makes both ring colours the same.
            assert_eq!(was, now, "({x}, {y}) changed outside the dialog");
        }
    }
}
