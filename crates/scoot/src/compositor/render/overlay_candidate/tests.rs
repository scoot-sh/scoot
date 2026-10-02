//! [`OverlayPick::marking`]'s gates, over hand-built arrangements: which
//! window the next frame marks, and which current-frame gate suppresses it.
//! The post-gather refresh (the shared refusals, the front-to-back pick) is
//! pinned through a real `State` in `fullscreen/tests/overlay_pick.rs`;
//! what is pinned here needs no client, no renderer and no elements.

use scoot_core::{Arrangement, OutputId, Placement, Rect, WindowId};

use super::*;

const OUTPUT: OutputId = OutputId(1);
const OTHER_OUTPUT: OutputId = OutputId(2);
const WINDOW: WindowId = WindowId(7);

fn placed(id: WindowId, output: OutputId, visible: bool, fullscreen: bool) -> Placement {
    Placement {
        id,
        output,
        rect: Rect::new(0, 0, 100, 100),
        visible,
        fullscreen,
        floating: false,
        requested: None,
    }
}

fn arrangement(placements: Vec<Placement>) -> Arrangement {
    Arrangement {
        placements,
        focused: None,
        focused_output: None,
    }
}

/// The pick, with every gate open: the stored window, still placed on this
/// output, visible and tiled.
fn marking(pick: &OverlayPick, arrangement: Option<&Arrangement>) -> Option<WindowId> {
    pick.marking(false, false, false, false, 1, arrangement, OUTPUT)
}

fn picked() -> OverlayPick {
    OverlayPick {
        window: Some(WINDOW),
    }
}

fn placed_window() -> Arrangement {
    arrangement(vec![placed(WINDOW, OUTPUT, true, false)])
}

#[test]
fn nothing_picked_marks_nothing() {
    assert_eq!(
        marking(&OverlayPick::default(), Some(&placed_window())),
        None
    );
}

#[test]
fn the_stored_pick_marks_when_every_gate_is_open() {
    assert_eq!(marking(&picked(), Some(&placed_window())), Some(WINDOW));
}

#[test]
fn each_current_frame_gate_suppresses_the_mark() {
    let pick = picked();
    let arrangement = placed_window();
    // Locked, streamed, forced, covered, or no overlay plane: no frame a
    // capture depends on marks anything.
    assert_eq!(
        pick.marking(true, false, false, false, 1, Some(&arrangement), OUTPUT),
        None,
        "locked"
    );
    assert_eq!(
        pick.marking(false, true, false, false, 1, Some(&arrangement), OUTPUT),
        None,
        "streaming"
    );
    assert_eq!(
        pick.marking(false, false, true, false, 1, Some(&arrangement), OUTPUT),
        None,
        "forced"
    );
    assert_eq!(
        pick.marking(false, false, false, true, 1, Some(&arrangement), OUTPUT),
        None,
        "covered"
    );
    assert_eq!(
        pick.marking(false, false, false, false, 0, Some(&arrangement), OUTPUT),
        None,
        "no overlay plane"
    );
    assert_eq!(
        pick.marking(false, false, false, false, 1, None, OUTPUT),
        None
    );
}

#[test]
fn a_window_that_left_does_not_mark() {
    // Closed: nowhere in the arrangement.
    assert_eq!(marking(&picked(), Some(&arrangement(vec![]))), None);
    // Moved to the other output: this output marks nothing for it.
    assert_eq!(
        marking(
            &picked(),
            Some(&arrangement(vec![placed(
                WINDOW,
                OTHER_OUTPUT,
                true,
                false
            )]))
        ),
        None
    );
    // Fullscreened since: the primary plane's domain, never the overlay's.
    assert_eq!(
        marking(
            &picked(),
            Some(&arrangement(vec![placed(WINDOW, OUTPUT, true, true)]))
        ),
        None
    );
    // Scrolled out of view: `window_elements` draws nothing for it, so
    // there is nothing to mark.
    assert_eq!(
        marking(
            &picked(),
            Some(&arrangement(vec![placed(WINDOW, OUTPUT, false, false)]))
        ),
        None
    );
}

#[test]
fn two_overlays_still_mark_at_most_one_window() {
    // The hardware may have two overlay planes per CRTC; the pick is one
    // window, and the mark is one window, whatever the count.
    let pick = picked();
    let arrangement = placed_window();
    for planes in [1, 2, 8] {
        assert_eq!(
            pick.marking(
                false,
                false,
                false,
                false,
                planes,
                Some(&arrangement),
                OUTPUT
            ),
            Some(WINDOW),
            "{planes} overlay planes"
        );
    }
}
