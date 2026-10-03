//! What a client is told about maximized, and what reaches the
//! framebuffer: the `maximized` state bit with the usable-area size, the bar
//! kept, the ring kept, and fullscreen winning while both hold.
//!
//! The canvas is the same 200x200 with gap 12, so the usable area with no
//! bar is (12, 12, 176, 176), and with the 20px top bar (12, 32, 176, 156).

use super::*;

/// The usable area with no bar, minus the gap: what a maximized window
/// fills while it covers.
const USABLE: Rect = Rect::new(12, 12, 176, 176);
/// The usable area with the top bar, minus the gap.
const BAR_USABLE: Rect = Rect::new(12, 32, 176, 156);

/// Maps a window and maximizes it, drawing it at the size it was told.
fn maximized_window(fixture: &mut Fixture) {
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetMaximized { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
}

/// Maximized and tiled are exclusive, and travel in the same configure as
/// the size: entering sends `maximized` with no tiled state, leaving sends
/// the four tiled states back with the column's size.
#[test]
fn maximize_replaces_the_tiled_states_and_leaving_restores_them() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let entered = fixture.configured(Step::SetMaximized { window: 0 });
    assert!(!entered.fullscreen, "{entered:?}");
    assert!(entered.maximized && !entered.tiled, "{entered:?}");
    assert_eq!((entered.width, entered.height), (USABLE.w, USABLE.h));

    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.rect_of(0), USABLE);
    let left = fixture.configured(Step::UnsetMaximized { window: 0 });
    assert!(left.tiled && !left.maximized, "{left:?}");
    assert_ne!(
        (left.width, left.height),
        (USABLE.w, USABLE.h),
        "leaving hands back the column's size with the tiled states"
    );
}

#[test]
fn every_request_is_answered_with_a_configure_even_when_nothing_changes() {
    // xdg-shell: "the compositor will respond by emitting a configure event"
    // -- for both requests, whatever the answer.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let before = fixture.configures(0).len();
    fixture.configured(Step::SetMaximized { window: 0 });
    let first = fixture.configures(0).len();
    assert!(first > before);
    let again = fixture.configured(Step::SetMaximized { window: 0 });
    assert!(again.maximized);
    assert_eq!(
        fixture.configures(0).len(),
        first + 1,
        "a repeat went unanswered"
    );

    fixture.configured(Step::UnsetMaximized { window: 0 });
    let left = fixture.configures(0).len();
    let still = fixture.configured(Step::UnsetMaximized { window: 0 });
    assert!(!still.maximized);
    assert_eq!(fixture.configures(0).len(), left + 1);
}

#[test]
fn unmapping_discards_maximized_and_the_remap_is_not_maximized() {
    // xdg-shell: an unmapped toplevel "returns to the state it had right
    // after xdg_surface.get_toplevel" -- which was not maximized.
    let mut fixture = Fixture::new();
    maximized_window(&mut fixture);
    assert!(fixture.state.world.is_maximized(fixture.id(0)));

    fixture.done(Step::Unmap { window: 0 });
    assert!(
        !fixture.state.world.is_maximized(fixture.id(0)),
        "maximized survived an unmap"
    );
    let used = fixture.configured(Step::Remap {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert!(!used.maximized, "{used:?}");
    assert!(!used.fullscreen, "{used:?}");
    assert!(!fixture.state.world.is_maximized(fixture.id(0)));
}

#[test]
fn a_maximized_window_fills_the_usable_area_keeping_bar_and_ring() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.map(WINDOW_BGRA);
    // Tiled first: the bar reserves the top, the window sits below it.
    assert_eq!(fixture.rect_of(0).y, BAR_HEIGHT as i32 + GAP);

    fixture.configured(Step::SetMaximized { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    // The usable area, not the output: the bar's strip stays outside.
    assert_eq!(fixture.rect_of(0), BAR_USABLE);
    let frame = fixture.render();
    assert_eq!(pixel(&frame, 5, 5), BAR_BGRA, "the bar was drawn over");
    assert_eq!(
        pixel(&frame, CANVAS / 2, CANVAS / 2),
        WINDOW_BGRA,
        "the window does not fill the usable area"
    );
    assert_eq!(
        pixel(&frame, BAR_USABLE.x, BAR_USABLE.y),
        WINDOW_BGRA,
        "the window does not reach the usable corner"
    );
    assert!(
        test_support::contains(&frame, RING_BGRA),
        "a maximized window lost its ring"
    );
}

#[test]
fn a_maximized_window_behind_no_bar_reaches_the_gap() {
    let mut fixture = Fixture::new();
    maximized_window(&mut fixture);
    assert_eq!(fixture.rect_of(0), USABLE);
    let frame = fixture.render();
    assert_eq!(pixel(&frame, 0, 0), BACKGROUND_BGRA);
    assert_eq!(pixel(&frame, USABLE.x, USABLE.y), WINDOW_BGRA);
    assert!(test_support::contains(&frame, RING_BGRA));
}

#[test]
fn fullscreen_wins_while_set_and_leaving_it_returns_to_maximized() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetMaximized { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.rect_of(0), BAR_USABLE);

    // Fullscreen on top: fullscreen only on the wire, the output's size.
    let covered = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    assert!(covered.fullscreen && !covered.maximized, "{covered:?}");
    assert_eq!((covered.width, covered.height), (CANVAS, CANVAS));
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.rect_of(0), OUTPUT);

    // Leaving fullscreen returns to maximized, not to the plain strip.
    let back = fixture.configured(Step::UnsetFullscreen { window: 0 });
    assert!(!back.fullscreen && back.maximized, "{back:?}");
    assert_eq!((back.width, back.height), (BAR_USABLE.w, BAR_USABLE.h));
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.rect_of(0), BAR_USABLE);
}

#[test]
fn a_late_maximized_frame_after_leaving_does_not_widen_the_column() {
    // The race a video player hits on the way out: the tiled configure goes
    // out while its next frame is still sized for the usable area. That
    // frame answers the *maximized* configure (it has not acked the new
    // one), so it must not read as a window refusing to shrink.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let tiled = fixture.rect_of(1);

    fixture.configured(Step::SetMaximized { window: 1 });
    fixture.done(Step::Draw {
        window: 1,
        color: OTHER_BGRA,
    });
    fixture.configured(Step::UnsetMaximized { window: 1 });
    fixture.done(Step::DrawUnacked {
        window: 1,
        width: USABLE.w,
        height: USABLE.h,
    });
    // ...and then it catches up.
    fixture.done(Step::Draw {
        window: 1,
        color: OTHER_BGRA,
    });
    assert_eq!(fixture.rect_of(1), tiled, "the column did not come back");
    // Nothing learned either: cycling width still lands on the presets.
    let configures = fixture.configures(1);
    let last = configures.last().unwrap();
    assert_eq!((last.width, last.height), (tiled.w, tiled.h));
}

#[test]
fn the_ipc_snapshot_reports_maximized() {
    // What an agent reads: `scoot msg windows` carries the bit from the
    // same placement the configure was built from.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let snapshots = fixture.state.window_snapshots();
    assert!(snapshots.iter().all(|s| !s.maximized));

    fixture.state.act(scoot_core::Action::SetMaximized {
        id: fixture.id(1),
        maximized: true,
    });
    fixture.settle();
    let snapshots = fixture.state.window_snapshots();
    assert!(snapshots[1].maximized, "{snapshots:?}");
    assert!(!snapshots[0].maximized, "{snapshots:?}");
    assert!(!snapshots[1].fullscreen, "{snapshots:?}");
}

#[test]
fn toggle_maximize_over_ipc_reaches_the_focused_window() {
    // The `toggle-maximize` spelling through `State::act`, the path an IPC
    // `action` request takes: the focused window is maximized, and its
    // configure carries the bit.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    fixture.state.act(scoot_core::Action::ToggleMaximize);
    fixture.settle();
    assert!(fixture.state.world.is_maximized(fixture.id(1)));
    let last = fixture.configures(1).last().copied().expect("a configure");
    assert!(last.maximized && !last.tiled, "{last:?}");
}
