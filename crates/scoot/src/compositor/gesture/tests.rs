//! Swipe gestures: the pure maths (rubber band, settle target, axis lock,
//! velocity) and the state machine on a live headless compositor with core
//! windows (no client surfaces: the ring is what draws, which is enough to
//! watch the arrangement and the reveal).

use std::time::Duration;

use scoot_core::{
    Action, Event as CoreEvent, Horizontal, OutputId, Placement, WindowId, WindowInfo,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

type Fixture = Harness<(), ()>;

const CANVAS: i32 = 400;

// ---- pure ----

#[test]
fn the_rubber_band_is_free_inside_and_bounded_outside() {
    assert_eq!(rubber(5.0, -10.0, 10.0, 60.0), 5.0);
    assert_eq!(rubber(10.0, -10.0, 10.0, 60.0), 10.0);
    // Past the edge: moves, but less than the finger, and never past `limit`.
    let one = rubber(110.0, -10.0, 10.0, 60.0);
    assert!(one > 10.0 && one < 110.0, "{one}");
    let far = rubber(1.0e9, -10.0, 10.0, 60.0);
    assert!(far < 10.0 + 60.0 && far > 10.0 + 59.0, "{far}");
    // Monotone, and symmetric on the other side.
    assert!(rubber(300.0, -10.0, 10.0, 60.0) > one);
    assert!(
        (rubber(-110.0, -10.0, 10.0, 60.0) - (-10.0 - (one - 10.0))).abs() < 1e-9,
        "the low side mirrors the high side"
    );
}

#[test]
fn the_rubber_band_survives_degenerate_ranges() {
    // A strip that fits (lo == hi) and an inverted range still bound the pull.
    assert_eq!(rubber(0.0, 0.0, 0.0, 50.0), 0.0);
    assert!(rubber(100.0, 0.0, 0.0, 50.0) <= 50.0);
    assert!(rubber(100.0, 7.0, -7.0, 50.0).is_finite());
    assert_eq!(rubber(100.0, 0.0, 0.0, 0.0), 0.0, "no limit, no pull");
    assert!(rubber(f64::MAX, 0.0, 0.0, 50.0) <= 50.0);
}

// Columns at rest: view at the strip's left, two 190 px columns visible
// then more beyond, gap 20, anchor (usable edge) at 10.
const XS: [i32; 4] = [10, 220, 430, 640];

#[test]
fn a_slow_release_goes_to_the_nearest_column() {
    // Content dragged left by 100: column 0 sits at -90, column 1 at 120.
    assert_eq!(settle_column(&XS, -100.0, 10, 0.0), 0);
    // By 120: column 1 is 90 from the edge, column 0 is 120.
    assert_eq!(settle_column(&XS, -120.0, 10, 0.0), 1);
    // Not moved: stay.
    assert_eq!(settle_column(&XS, 0.0, 10, 0.0), 0);
    // Dragged right (rubber past the first column): still column 0.
    assert_eq!(settle_column(&XS, 80.0, 10, 0.0), 0);
    // A tie goes to the earlier column.
    assert_eq!(settle_column(&XS, -105.0, 10, 0.0), 0);
}

#[test]
fn a_flick_goes_to_the_next_column_in_its_direction() {
    // A tiny drag left, fast: the next column, not the nearest.
    assert_eq!(settle_column(&XS, -20.0, 10, -900.0), 1);
    // Fast right from column 1's rest: the one before.
    assert_eq!(settle_column(&XS, -210.0 + 10.0, 10, 900.0), 0);
    // Below the threshold is not a flick.
    assert_eq!(settle_column(&XS, -20.0, 10, -(FLICK_VELOCITY - 1.0)), 0);
    // Nothing ahead: fall back to the nearest, never out of range.
    assert_eq!(settle_column(&XS, -630.0, 10, -900.0), 3);
    assert_eq!(settle_column(&XS, 50.0, 10, 900.0), 0);
    // A column resting exactly on the edge is not skipped by a flick that
    // starts there: the next one is chosen.
    assert_eq!(settle_column(&XS, 0.0, 10, -900.0), 1);
}

#[test]
fn settling_never_indexes_out_of_range() {
    assert_eq!(settle_column(&[], 5.0, 0, 900.0), 0);
    assert_eq!(settle_column(&[10], 1.0e9, 10, -900.0), 0);
    assert_eq!(settle_column(&[10], -1.0e9, 10, 900.0), 0);
    assert_eq!(settle_column(&XS, f64::NAN, 10, f64::NAN), 0);
    assert!(settle_column(&XS, f64::INFINITY, 10, f64::INFINITY) < XS.len());
    assert!(settle_column(&XS, -1.0e300, i32::MIN, -1.0e300) < XS.len());
}

#[test]
fn the_axis_is_picked_once_and_by_the_larger_motion() {
    assert_eq!(pick_axis((0.0, 0.0)), None);
    assert_eq!(pick_axis((LOCK_DISTANCE - 0.1, 3.0)), None);
    assert_eq!(pick_axis((LOCK_DISTANCE, 3.0)), Some(Axis::Horizontal));
    assert_eq!(pick_axis((-3.0, -LOCK_DISTANCE)), Some(Axis::Vertical));
    assert_eq!(
        pick_axis((-20.0, 20.0)),
        Some(Axis::Horizontal),
        "ties are horizontal"
    );
}

#[test]
fn velocity_follows_recent_motion_and_forgets_a_rest() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    // 8 px in 8 ms is 1000 px/s; blended with a standing start it is half.
    assert!(near(blend_velocity(0.0, 8.0, 8), 500.0));
    assert!(near(blend_velocity(1000.0, 8.0, 8), 1000.0));
    // The speed before a long rest is not the speed after it.
    assert!(near(
        blend_velocity(1000.0, 8.0, VELOCITY_STALE_MS + 1),
        0.5 * 8.0 / 0.101
    ));
    // Same-millisecond events cannot make an infinite speed.
    assert!(near(blend_velocity(0.0, 8.0, 0), 0.5 * 8.0 / 0.004));
}

#[test]
fn a_vertical_swipe_needs_its_distance_and_maps_to_the_configured_action() {
    use scoot_core::Vertical::{Down, Up};
    let short = VERTICAL_DISTANCE - 1.0;
    assert_eq!(vertical_action(VerticalMode::FocusWorkspace, short), None);
    assert_eq!(vertical_action(VerticalMode::FocusWorkspace, -short), None);
    // Content moving up (negative) uncovers what is below it.
    assert_eq!(
        vertical_action(VerticalMode::FocusWorkspace, -150.0),
        Some(Action::FocusWorkspace(Down))
    );
    assert_eq!(
        vertical_action(VerticalMode::FocusWindow, 150.0),
        Some(Action::FocusWindow(Up))
    );
    assert_eq!(vertical_action(VerticalMode::None, 500.0), None);
    assert_eq!(vertical_action(VerticalMode::FocusWindow, f64::NAN), None);
}

#[test]
fn natural_and_sensitivity_scale_the_motion() {
    let mut config = GestureConfig::default();
    assert_eq!(config.scale(), 1.0);
    config.natural = false;
    config.sensitivity = 2.0;
    assert_eq!(config.scale(), -2.0);
    assert!(config.enabled());
    config.horizontal = HorizontalMode::None;
    config.vertical = VerticalMode::None;
    assert!(!config.enabled());
}

// ---- on a live compositor ----

fn open_windows(fixture: &mut Fixture, count: u64) {
    for id in 1..=count {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(id),
            info: WindowInfo {
                app_id: "gesture".to_owned(),
                title: "gesture".to_owned(),
                hints: Default::default(),
                parent: None,
            },
            output: None,
            focus: true,
        });
    }
    // Back to the first column.
    for _ in 1..count {
        fixture
            .state
            .world
            .handle_action(Action::FocusColumn(Horizontal::Left));
    }
    fixture.state.apply();
}

fn fixture(windows: u64) -> Fixture {
    let mut fixture = Fixture::headless(Appearance::default(), CANVAS);
    open_windows(&mut fixture, windows);
    // Snap unless a test asks for the settle.
    fixture.state.animation.enabled = false;
    fixture
}

fn placements(fixture: &Fixture) -> Vec<Placement> {
    fixture.state.world.arrange().placements
}

fn focused(fixture: &Fixture) -> Option<WindowId> {
    fixture.state.world.focused_window()
}

fn x_of(fixture: &Fixture, id: u64) -> i32 {
    fixture
        .state
        .world
        .arrange()
        .get(WindowId(id))
        .expect("the window is placed")
        .rect
        .x
}

/// One swipe: begin with `fingers`, an update per entry (10 ms apart unless
/// `dt` says), then end. Returns nothing; tests read the state.
fn swipe(fixture: &mut Fixture, fingers: u32, moves: &[(f64, f64)], dt: u32) {
    let mut time = 1000;
    fixture.state.gesture_swipe_begin(fingers, time);
    for &(dx, dy) in moves {
        time += dt;
        fixture.state.gesture_swipe_update(dx, dy, time);
    }
    fixture.state.gesture_swipe_end(false, time + 1);
}

#[test]
fn the_premise_two_visible_columns_and_a_strip_beyond() {
    let fixture = fixture(4);
    let all = placements(&fixture);
    assert_eq!(all.len(), 4);
    assert!(all[0].visible && all[1].visible, "{all:?}");
    assert!(
        !all[3].visible,
        "the last column starts scrolled out: {all:?}"
    );
    assert_eq!(focused(&fixture), Some(WindowId(1)));
}

#[test]
fn a_swipe_with_other_than_the_configured_fingers_is_forwarded_whole() {
    let mut fixture = fixture(4);
    for fingers in [2, 4, 5] {
        fixture.state.gesture_swipe_begin(fingers, 10);
        assert_eq!(fixture.state.gestures.route, Route::Forwarded);
        assert!(fixture.state.gestures.claim.is_none());
        assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
        fixture.state.gesture_swipe_update(-300.0, 0.0, 20);
        assert_eq!(fixture.state.gestures.route, Route::Forwarded);
        fixture.state.gesture_swipe_end(false, 30);
        assert_eq!(fixture.state.gestures.route, Route::Idle);
        assert_eq!(
            focused(&fixture),
            Some(WindowId(1)),
            "{fingers} fingers moved focus"
        );
    }
}

#[test]
fn both_axes_off_forwards_even_the_configured_fingers() {
    let mut fixture = fixture(4);
    fixture.state.gestures.config.horizontal = HorizontalMode::None;
    fixture.state.gestures.config.vertical = VerticalMode::None;
    swipe(&mut fixture, 3, &[(-300.0, 0.0)], 10);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    fixture.state.gesture_swipe_begin(3, 10);
    assert_eq!(fixture.state.gestures.route, Route::Forwarded);
    fixture.state.gesture_swipe_end(false, 20);
}

#[test]
fn a_claimed_swipe_reveals_the_columns_scrolled_out_of_view_once_it_locks_horizontal() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 10);
    assert_eq!(fixture.state.gestures.route, Route::Claimed);
    assert_eq!(fixture.state.gestures.reveal.len(), 4);
    // Not yet: nothing is mapped before the axis is known.
    assert!(!fixture.state.gestures.revealing);
    assert!(
        fixture
            .state
            .frame_arrangement()
            .placements
            .iter()
            .any(|p| !p.visible)
    );
    fixture.state.gesture_swipe_update(-5.0, 0.0, 20);
    assert!(!fixture.state.gestures.revealing);
    fixture.state.gesture_swipe_update(-30.0, 0.0, 30);
    assert!(fixture.state.gestures.revealing);
    let all = placements(&fixture);
    let hidden: Vec<_> = all.iter().filter(|p| !p.visible).collect();
    assert!(!hidden.is_empty(), "the premise needs a column out of view");
    for placement in hidden {
        assert!(
            fixture.state.gestures.reveals(placement),
            "{placement:?} is not revealed"
        );
    }
    let arrangement = fixture.state.frame_arrangement();
    assert!(arrangement.placements.iter().all(|p| p.visible));
    // Only the frame's copy changes: the core still reports the truth.
    assert!(placements(&fixture).iter().any(|p| !p.visible));
    fixture.state.gesture_swipe_end(true, 40);
    assert!(
        fixture.state.gestures.reveal.is_empty(),
        "revealed windows leaked past the swipe"
    );
    assert!(!fixture.state.gestures.revealing);
    assert!(
        fixture
            .state
            .frame_arrangement()
            .placements
            .iter()
            .any(|p| !p.visible)
    );
}

#[test]
fn a_vertical_or_inert_swipe_never_maps_the_hidden_columns() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 10);
    fixture.state.gesture_swipe_update(0.0, -40.0, 20);
    fixture.state.gesture_swipe_update(0.0, -40.0, 30);
    assert!(!fixture.state.gestures.revealing);
    assert!(
        placements(&fixture)
            .iter()
            .any(|p| !p.visible && !fixture.state.gestures.reveals(p))
    );
    fixture.state.gesture_swipe_end(true, 40);
    // Horizontal off: a sideways swipe is swallowed but reveals nothing.
    fixture.state.gestures.config.horizontal = HorizontalMode::None;
    fixture.state.gesture_swipe_begin(3, 100);
    fixture.state.gesture_swipe_update(-80.0, 0.0, 110);
    assert!(!fixture.state.gestures.revealing);
    fixture.state.gesture_swipe_end(false, 120);
}

#[test]
fn the_strip_follows_the_fingers_after_the_axis_locks() {
    let mut fixture = fixture(4);
    let at_rest = x_of(&fixture, 1);
    fixture.state.gesture_swipe_begin(3, 1000);
    // Under the lock distance: nothing moves yet.
    fixture.state.gesture_swipe_update(-5.0, 1.0, 1010);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    fixture.state.gesture_swipe_update(-30.0, 2.0, 1020);
    let shift = fixture.state.gesture_shift(Some(OutputId(1)));
    assert_eq!(shift, -35, "motion since the start, once locked");
    // Another output is not shifted.
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(2))), 0);
    assert_eq!(fixture.state.gesture_shift(None), 0);
    fixture.state.gesture_swipe_update(-40.0, 0.0, 1030);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), -75);
    // The targets did not move: the shift is only drawn.
    assert_eq!(x_of(&fixture, 1), at_rest);
    fixture.state.gesture_swipe_end(true, 1040);
}

#[test]
fn a_swipe_past_the_first_column_is_rubber_banded() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(500.0, 0.0, 1010);
    let shift = fixture.state.gesture_shift(Some(OutputId(1)));
    assert!(shift > 0 && shift < 500, "{shift}");
    assert!(
        f64::from(shift) <= OVERSCROLL * f64::from(CANVAS),
        "pulled past the limit: {shift}"
    );
    fixture.state.gesture_swipe_end(true, 1020);
}

#[test]
fn releasing_past_half_a_column_focuses_the_next_one() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let drag = -(f64::from(step) * 0.7);
    // Slow, in ten moves: no flick.
    let moves = [(drag / 10.0, 0.0); 10];
    swipe(&mut fixture, 3, &moves, 120);
    assert_eq!(focused(&fixture), Some(WindowId(2)));
    // Animation is off: it snapped, and the extra columns are unmapped again.
    assert_eq!(fixture.state.gestures.output, None);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    assert_eq!(fixture.state.gestures.route, Route::Idle);
}

#[test]
fn releasing_onto_a_column_aligns_the_view_to_its_left_edge() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let drag = -(f64::from(step) * 0.7);
    swipe(&mut fixture, 3, &[(drag / 10.0, 0.0); 10], 120);
    assert_eq!(focused(&fixture), Some(WindowId(2)));
    // The ticket's gap: plain focus would leave a fully visible neighbour
    // where it is. The aligned action lines it up with the usable edge
    // instead: the same x a direct `scroll-view-to-column` lands on.
    let usable_x = fixture
        .state
        .world
        .usable_area(OutputId(1))
        .expect("an output")
        .inset(fixture.state.world.config().gap)
        .x;
    assert_eq!(x_of(&fixture, 2), usable_x);
    let mut direct = self::fixture(4);
    direct
        .state
        .world
        .handle_action(Action::ScrollViewToColumn(1));
    assert_eq!(x_of(&direct, 2), usable_x);
    assert_eq!(x_of(&fixture, 2), x_of(&direct, 2));
}

#[test]
fn releasing_under_half_a_column_stays_put() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let drag = -(f64::from(step) * 0.3);
    swipe(&mut fixture, 3, &[(drag / 10.0, 0.0); 10], 120);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
}

#[test]
fn a_short_fast_flick_goes_one_column() {
    let mut fixture = fixture(4);
    // 60 px in 6 ms-spaced updates is 10 000 px/s.
    swipe(
        &mut fixture,
        3,
        &[(-20.0, 0.0), (-20.0, 0.0), (-20.0, 0.0)],
        6,
    );
    assert_eq!(focused(&fixture), Some(WindowId(2)));
}

#[test]
fn a_fast_swipe_that_rests_before_the_release_is_not_a_flick() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    for (i, dx) in [-20.0, -20.0, -20.0].into_iter().enumerate() {
        fixture
            .state
            .gesture_swipe_update(dx, 0.0, 1006 + 6 * i as u32);
    }
    // The fingers sat still for half a second before lifting.
    fixture.state.gesture_swipe_end(false, 1600);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
}

#[test]
fn a_slow_swipe_just_past_the_lock_is_not_a_flick() {
    // The motion before the axis locks lands in one event. It must not read as
    // a burst of speed: a deliberate drag a little past the lock distance and
    // a release must stay on the nearest column.
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-6.0, 0.0, 1008);
    fixture.state.gesture_swipe_update(-9.0, 0.0, 1016);
    // Lifted straight after the lock event, so any speed it made is what
    // the release sees.
    fixture.state.gesture_swipe_end(false, 1020);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
}

#[test]
fn a_window_opening_mid_swipe_does_not_misdirect_the_release() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture
        .state
        .gesture_swipe_update(-f64::from(step) * 0.8, 0.0, 1100);
    // A new window opens, takes the focus and shifts the strip.
    fixture.state.world.handle_event(CoreEvent::WindowOpened {
        id: WindowId(99),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    fixture.state.apply();
    fixture.state.gesture_swipe_end(false, 1700);
    // Counted against the layout at release: a column of the strip, never a
    // step past its end.
    assert!(focused(&fixture).is_some());
    assert_eq!(fixture.state.gestures.route, Route::Idle);
    assert_eq!(fixture.state.gestures.output, None);
}

#[test]
fn a_swipe_right_goes_back_and_natural_false_reverses_it() {
    let mut fixture = fixture(4);
    fixture
        .state
        .world
        .handle_action(Action::FocusColumn(Horizontal::Right));
    fixture.state.apply();
    assert_eq!(focused(&fixture), Some(WindowId(2)));
    swipe(&mut fixture, 3, &[(20.0, 0.0), (20.0, 0.0), (20.0, 0.0)], 6);
    assert_eq!(
        focused(&fixture),
        Some(WindowId(1)),
        "content right, focus left"
    );
    fixture.state.gestures.config.natural = false;
    swipe(&mut fixture, 3, &[(20.0, 0.0), (20.0, 0.0), (20.0, 0.0)], 6);
    assert_eq!(focused(&fixture), Some(WindowId(2)), "reversed");
}

#[test]
fn a_swipe_can_cross_several_columns() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let drag = -(f64::from(step) * 2.1);
    swipe(&mut fixture, 3, &[(drag / 20.0, 0.0); 20], 100);
    assert_eq!(focused(&fixture), Some(WindowId(3)));
}

#[test]
fn a_cancelled_end_moves_nothing() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-400.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(true, 1020);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    assert_eq!(fixture.state.gestures.output, None);
}

#[test]
fn a_vertical_swipe_fires_one_action_on_release() {
    let mut fixture = fixture(4);
    // Content moving up: the workspace below. There is only the empty
    // trailing one, so the active index tells.
    let before = fixture
        .state
        .world
        .workspaces(OutputId(1))
        .expect("an output")
        .active;
    swipe(
        &mut fixture,
        3,
        &[(0.0, -60.0), (0.0, -60.0), (0.0, -60.0)],
        10,
    );
    let after = fixture
        .state
        .world
        .workspaces(OutputId(1))
        .expect("an output")
        .active;
    assert_eq!(after, before + 1);
    // And the strip never moved sideways.
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
}

#[test]
fn a_short_vertical_swipe_does_nothing() {
    let mut fixture = fixture(4);
    swipe(&mut fixture, 3, &[(0.0, -20.0), (0.0, -20.0)], 10);
    assert_eq!(
        fixture
            .state
            .world
            .workspaces(OutputId(1))
            .expect("an output")
            .active,
        0
    );
}

#[test]
fn vertical_can_step_windows_instead() {
    let mut fixture = fixture(1);
    fixture.state.world.handle_event(CoreEvent::WindowOpened {
        id: WindowId(50),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    fixture.state.gestures.config.vertical = VerticalMode::FocusWindow;
    // Stack window 50 under window 1 in one column; the focus stays on 50.
    fixture
        .state
        .world
        .handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    fixture.state.apply();
    assert_eq!(focused(&fixture), Some(WindowId(50)));
    // Content moving down: the window above.
    swipe(&mut fixture, 3, &[(0.0, 80.0), (0.0, 80.0)], 10);
    assert_eq!(
        focused(&fixture),
        Some(WindowId(1)),
        "focus-window up did not fire"
    );
}

#[test]
fn a_swipe_locked_to_a_disabled_axis_is_swallowed_and_does_nothing() {
    let mut fixture = fixture(4);
    fixture.state.gestures.config.vertical = VerticalMode::None;
    // Vertical motion with only the horizontal axis on: claimed (the
    // client must not see half of it), inert.
    fixture.state.gesture_swipe_begin(3, 1000);
    assert_eq!(fixture.state.gestures.route, Route::Claimed);
    fixture.state.gesture_swipe_update(0.0, -300.0, 1010);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    fixture.state.gesture_swipe_end(false, 1020);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    assert_eq!(
        fixture
            .state
            .world
            .workspaces(OutputId(1))
            .expect("an output")
            .active,
        0
    );
}

#[test]
fn zero_and_one_window_consume_the_swipe_and_move_nothing() {
    let mut fixture = fixture(0);
    swipe(&mut fixture, 3, &[(-300.0, 0.0), (-300.0, 0.0)], 10);
    assert_eq!(focused(&fixture), None);
    assert_eq!(fixture.state.gestures.route, Route::Idle);
    let mut fixture = self::fixture(1);
    fixture.state.gesture_swipe_begin(3, 1000);
    assert_eq!(fixture.state.gestures.route, Route::Claimed);
    fixture.state.gesture_swipe_update(-300.0, 0.0, 1010);
    let shift = fixture.state.gesture_shift(Some(OutputId(1)));
    assert!(
        shift <= 0 && shift > -100,
        "a lone column barely gives: {shift}"
    );
    fixture.state.gesture_swipe_end(false, 1020);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
}

#[test]
fn hostile_deltas_are_ignored_or_bounded() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(f64::NAN, 1.0, 1010);
    fixture.state.gesture_swipe_update(1.0, f64::INFINITY, 1020);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    for time in [1030, 1030, 1020, u32::MAX, 0] {
        fixture
            .state
            .gesture_swipe_update(f64::MAX, -f64::MAX, time);
        fixture.state.gesture_swipe_update(-1.0e300, 1.0e300, time);
    }
    let shift = fixture.state.gesture_shift(Some(OutputId(1)));
    assert!(
        f64::from(shift.abs()) <= OVERSCROLL * f64::from(CANVAS) + 1.0 + 1.0e7,
        "{shift}"
    );
    fixture.state.gesture_swipe_end(false, 5);
    assert!(fixture.state.gestures.route == Route::Idle);
    assert!(focused(&fixture).is_some());
}

#[test]
fn an_update_or_end_with_no_begin_is_ignored() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_update(-300.0, 0.0, 10);
    fixture.state.gesture_swipe_end(false, 20);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    assert_eq!(fixture.state.gestures.route, Route::Idle);
}

#[test]
fn a_new_begin_closes_the_swipe_still_open() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-300.0, 0.0, 1010);
    // libinput never does this; if it did, the old swipe is dropped without
    // acting and the new one starts clean.
    fixture.state.gesture_swipe_begin(3, 1100);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    fixture.state.gesture_swipe_begin(2, 1200);
    assert_eq!(fixture.state.gestures.route, Route::Forwarded);
    assert!(fixture.state.gestures.claim.is_none());
    fixture.state.gesture_swipe_begin(3, 1300);
    assert_eq!(fixture.state.gestures.route, Route::Claimed);
    fixture.state.gesture_swipe_end(true, 1400);
}

#[test]
fn a_reload_mid_swipe_does_not_change_that_swipe() {
    let mut fixture = fixture(4);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    fixture.state.gesture_swipe_begin(3, 1000);
    // The file now turns everything off, and the finger count away.
    fixture.state.gestures.config.horizontal = HorizontalMode::None;
    fixture.state.gestures.config.vertical = VerticalMode::None;
    fixture.state.gestures.config.fingers = 4;
    fixture
        .state
        .gesture_swipe_update(-f64::from(step) * 0.8, 0.0, 1100);
    assert_eq!(fixture.state.gestures.route, Route::Claimed);
    fixture.state.gesture_swipe_end(false, 1700);
    assert_eq!(
        focused(&fixture),
        Some(WindowId(2)),
        "the swipe kept its own config"
    );
    // A forwarded swipe stays forwarded through a reload that would claim it.
    fixture.state.gestures.config = GestureConfig::default();
    fixture.state.gesture_swipe_begin(2, 2000);
    assert_eq!(fixture.state.gestures.route, Route::Forwarded);
    fixture.state.gesture_swipe_update(-300.0, 0.0, 2010);
    assert_eq!(fixture.state.gestures.route, Route::Forwarded);
    fixture.state.gesture_swipe_end(false, 2020);
}

#[test]
fn a_swipe_ending_nowhere_near_an_output_drops_cleanly() {
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-100.0, 0.0, 1010);
    // The output goes away mid-swipe.
    fixture
        .state
        .world
        .handle_event(CoreEvent::OutputRemoved { id: OutputId(1) });
    fixture.state.gesture_swipe_update(-100.0, 0.0, 1020);
    assert_eq!(
        fixture.state.gestures.output, None,
        "the claim outlived its output"
    );
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
    fixture.state.gesture_swipe_end(false, 1030);
    assert_eq!(fixture.state.gestures.route, Route::Idle);
}

// ---- the settle ----

fn animate(fixture: &mut Fixture, ms: u64) {
    fixture.state.animation.enabled = true;
    fixture.state.animation.duration = Duration::from_millis(ms);
}

/// The drawn offset at the moment the settle began: time-free, so a slow or
/// preempted test run cannot move it.
fn settle_offset_at_start(fixture: &Fixture) -> f64 {
    let settle = fixture.state.gestures.settle.expect("a settle in flight");
    fixture.state.gestures.offset(settle.start)
}

/// Pretends the settle began long enough ago to be over, without sleeping.
fn age_settle(fixture: &mut Fixture) {
    let settle = fixture
        .state
        .gestures
        .settle
        .as_mut()
        .expect("a settle in flight");
    settle.start = settle
        .start
        .checked_sub(settle.duration + Duration::from_millis(5))
        .expect("an instant that far back");
}

#[test]
fn with_animation_the_offset_eases_out_and_the_swipe_lands() {
    let mut fixture = fixture(4);
    animate(&mut fixture, 600_000);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let drag = -(f64::from(step) * 0.7);
    swipe(&mut fixture, 3, &[(drag / 10.0, 0.0); 10], 120);
    assert_eq!(focused(&fixture), Some(WindowId(2)));
    // The targets are already the new ones; the old offset is still drawn,
    // re-based onto them, and the revealed columns stay mapped meanwhile.
    assert!(fixture.state.gestures.settle.is_some());
    assert_eq!(fixture.state.gestures.output, Some(OutputId(1)));
    assert!(!fixture.state.gestures.reveal.is_empty());
    assert_ne!(
        settle_offset_at_start(&fixture),
        0.0,
        "the settle starts from where the fingers left off"
    );
    assert!(
        fixture.state.gesture_tick(),
        "frames are needed while it eases"
    );
    age_settle(&mut fixture);
    assert!(!fixture.state.gesture_tick(), "and not once it landed");
    assert_eq!(fixture.state.gestures.output, None);
    assert!(fixture.state.gestures.reveal.is_empty());
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
}

#[test]
fn the_settle_starts_where_the_fingers_left_off() {
    // The re-base: the drawn position just before and just after the focus
    // action are the same, whatever the layout did to the targets.
    let mut fixture = fixture(4);
    animate(&mut fixture, 600_000);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    let first_before = x_of(&fixture, 1);
    let drag = -(f64::from(step) * 0.7);
    fixture.state.gesture_swipe_begin(3, 1000);
    for i in 0..10 {
        fixture
            .state
            .gesture_swipe_update(drag / 10.0, 0.0, 1120 + 120 * i);
    }
    let drawn_before = first_before + fixture.state.gesture_shift(Some(OutputId(1)));
    fixture.state.gesture_swipe_end(false, 2400);
    let drawn_after = x_of(&fixture, 1) + settle_offset_at_start(&fixture).round() as i32;
    assert_eq!(
        drawn_before, drawn_after,
        "the strip jumped on release: {drawn_before} -> {drawn_after}"
    );
}

#[test]
fn a_cancelled_swipe_eases_back_without_acting() {
    let mut fixture = fixture(4);
    animate(&mut fixture, 600_000);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-150.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(true, 1020);
    assert_eq!(focused(&fixture), Some(WindowId(1)));
    assert_eq!(settle_offset_at_start(&fixture), -150.0);
    age_settle(&mut fixture);
    assert!(!fixture.state.gesture_tick());
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
}

#[test]
fn a_new_swipe_catches_a_settling_strip_where_it_is() {
    let mut fixture = fixture(4);
    animate(&mut fixture, 600_000);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-150.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(true, 1020);
    let drawn = fixture.state.gesture_shift(Some(OutputId(1)));
    assert!(drawn < 0);
    fixture.state.gesture_swipe_begin(3, 1100);
    let caught = fixture.state.gesture_shift(Some(OutputId(1)));
    // Ten minutes of settle: a few milliseconds of test time move it well
    // under a pixel.
    assert!((caught - drawn).abs() <= 1, "{drawn} -> {caught}");
    fixture.state.gesture_swipe_end(true, 1200);
}

#[test]
fn a_vertical_swipe_that_catches_a_settling_strip_still_locks_vertical() {
    // The caught offset is the strip's position, not finger motion: it must
    // not decide the axis.
    let mut fixture = fixture(4);
    animate(&mut fixture, 600_000);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-200.0, 0.0, 1010);
    fixture.state.gesture_swipe_end(true, 1020);
    assert!(fixture.state.gestures.settle.is_some());
    let before = fixture
        .state
        .world
        .workspaces(OutputId(1))
        .expect("an output")
        .active;
    swipe(
        &mut fixture,
        3,
        &[(0.0, -60.0), (0.0, -60.0), (0.0, -60.0)],
        10,
    );
    assert_eq!(
        fixture
            .state
            .world
            .workspaces(OutputId(1))
            .expect("an output")
            .active,
        before + 1,
        "the settling offset locked the swipe horizontal"
    );
}

#[test]
fn focus_moving_to_another_output_mid_swipe_takes_no_action_on_either() {
    use scoot_core::Rect;
    let mut fixture = fixture(4);
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-300.0, 0.0, 1010);
    // A second output with a strip of its own appears and takes the focus
    // under the swipe.
    fixture.state.world.handle_event(CoreEvent::OutputAdded {
        id: OutputId(2),
        area: Rect::new(CANVAS, 0, CANVAS, CANVAS),
    });
    for id in 20..23 {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(id),
            info: WindowInfo::default(),
            output: Some(OutputId(2)),
            focus: true,
        });
    }
    for _ in 0..2 {
        fixture
            .state
            .world
            .handle_action(Action::FocusColumn(Horizontal::Left));
    }
    fixture.state.apply();
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(2)),
        "premise: the focus moved"
    );
    assert_eq!(focused(&fixture), Some(WindowId(20)));
    fixture.state.gesture_swipe_end(false, 1700);
    assert_eq!(fixture.state.gestures.output, None);
    assert_eq!(fixture.state.gestures.route, Route::Idle);
    assert_eq!(
        focused(&fixture),
        Some(WindowId(20)),
        "the swipe dragged output 1's strip but stepped output 2's"
    );
}

#[test]
fn idle_costs_no_frames() {
    let mut fixture = fixture(4);
    assert!(!fixture.state.gesture_tick());
    fixture.state.needs_render = false;
    assert!(!fixture.state.gesture_tick());
    assert!(!fixture.state.needs_render);
    assert_eq!(fixture.state.gesture_shift(Some(OutputId(1))), 0);
}

#[test]
fn a_swiping_output_does_not_double_animate_with_slides() {
    let mut fixture = fixture(4);
    animate(&mut fixture, 5000);
    let step = x_of(&fixture, 2) - x_of(&fixture, 1);
    swipe(
        &mut fixture,
        3,
        &[(-f64::from(step) * 0.7 / 10.0, 0.0); 10],
        120,
    );
    assert_eq!(focused(&fixture), Some(WindowId(2)));
    assert!(
        fixture.state.slides.is_empty(),
        "per-window slides on top of the swipe's own settle: {:?}",
        fixture.state.slides
    );
}

// ---- drawing ----

fn pixel(frame: &[u8], x: i32, y: i32) -> [u8; 4] {
    let at = ((y * CANVAS + x) * 4) as usize;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

#[test]
fn a_column_scrolled_out_of_view_is_drawn_while_the_strip_is_pulled_to_it() {
    let mut fixture = fixture(4);
    let hidden = placements(&fixture)
        .into_iter()
        .find(|p| !p.visible)
        .expect("a hidden column");
    // Pull the strip left until the hidden column's left edge is on screen.
    let want = CANVAS - 60;
    let pull = f64::from(want - hidden.rect.x);
    fixture.state.gesture_swipe_begin(3, 1000);
    for i in 0..10 {
        fixture
            .state
            .gesture_swipe_update(pull / 10.0, 0.0, 1010 + 10 * i);
    }
    let shift = fixture.state.gesture_shift(Some(OutputId(1)));
    let edge = hidden.rect.x + shift;
    assert!(edge > 0 && edge < CANVAS - 4, "premise: edge {edge}");
    fixture.state.request_render();
    fixture.state.render();
    let during = fixture.pixels_of(OutputId(1));
    let background = pixel(&during, 3, CANVAS - 3);
    // The ring hugs the window's edge from outside: look a few pixels either
    // side of it.
    let near_edge =
        |frame: &[u8]| (edge - 8..=edge + 2).any(|x| pixel(frame, x, CANVAS / 2) != background);
    assert!(
        near_edge(&during),
        "the hidden column's ring is not drawn near x={edge}"
    );
    // After the swipe, the same pixels are background again.
    fixture.state.gesture_swipe_end(true, 2000);
    fixture.state.request_render();
    fixture.state.render();
    let after = fixture.pixels_of(OutputId(1));
    assert!(!near_edge(&after), "the ring stayed behind after the swipe");
}

/// Per-event cost of the gesture arms and the idle frame-side checks, the
/// numbers the PR quotes. Ignored: `cargo test --release -p scoot --bin scoot
/// bench_gesture -- --ignored --nocapture`.
///
/// 1. `idle shift`: what every frame pays per output with no swipe;
/// 2. `idle frame_arrangement - arrange`: what the reveal hook adds to the
///    per-tick arrangement when idle;
/// 3. `update`: one claimed horizontal update (accumulate, rubber band,
///    velocity, render request) -- at most a few hundred a second;
/// 4. `update, forwarded`: a swipe that is not scoot's, through the pointer.
#[test]
#[ignore = "timing, not correctness"]
fn bench_gesture() {
    const ROUNDS: u32 = 1_000_000;
    let mut fixture = fixture(4);
    let per = |label: &str, started: std::time::Instant, rounds: u32| {
        let ns = started.elapsed().as_nanos() as f64 / f64::from(rounds);
        println!("{label}: {ns:.1} ns");
    };

    let started = std::time::Instant::now();
    let mut sum = 0i64;
    for _ in 0..ROUNDS {
        sum += i64::from(
            fixture
                .state
                .gesture_shift(std::hint::black_box(Some(OutputId(1)))),
        );
    }
    std::hint::black_box(sum);
    per("idle shift                       ", started, ROUNDS);

    const ARRANGES: u32 = 20_000;
    let started = std::time::Instant::now();
    for _ in 0..ARRANGES {
        std::hint::black_box(fixture.state.world.arrange());
    }
    per("arrange (4 windows)              ", started, ARRANGES);
    let started = std::time::Instant::now();
    for _ in 0..ARRANGES {
        std::hint::black_box(fixture.state.frame_arrangement());
    }
    per("frame_arrangement, idle          ", started, ARRANGES);

    fixture.state.gesture_swipe_begin(3, 1000);
    let mut time = 1000;
    let started = std::time::Instant::now();
    for i in 0..ROUNDS {
        time += 8;
        let dx = if i % 2 == 0 { 3.0 } else { -3.0 };
        fixture.state.gesture_swipe_update(dx, 0.5, time);
    }
    per("update, claimed horizontal       ", started, ROUNDS);
    fixture.state.gesture_swipe_end(true, time + 1);

    fixture.state.gesture_swipe_begin(2, 1000);
    let started = std::time::Instant::now();
    for i in 0..ROUNDS {
        time += 8;
        let dx = if i % 2 == 0 { 3.0 } else { -3.0 };
        fixture.state.gesture_swipe_update(dx, 0.5, time);
    }
    per("update, forwarded                ", started, ROUNDS);
    fixture.state.gesture_swipe_end(false, time + 1);
}

#[test]
fn a_floating_window_and_its_ring_stay_put_while_the_strip_is_pulled() {
    use scoot_core::Size;
    let mut fixture = fixture(1);
    fixture.state.world.handle_event(CoreEvent::WindowOpened {
        id: WindowId(2),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    fixture
        .state
        .world
        .handle_event(CoreEvent::FloatingRequested {
            id: WindowId(2),
            floating: true,
            size: Some(Size::new(90, 90)),
        });
    fixture.state.apply();
    let all = placements(&fixture);
    let float = all
        .iter()
        .find(|p| p.floating)
        .expect("a floating placement");
    assert!(
        float.visible,
        "premise: the floating window is placed: {float:?}"
    );
    let strip = all.iter().find(|p| !p.floating).expect("a strip window");
    // The floating window's right ring, well clear of the strip's own edges
    // both at rest and after a pull of 60 (the strip's right edge is at 194 -> 134).
    let edge = float.rect.x + float.rect.w;
    let mid = float.rect.y + float.rect.h / 2;
    assert!(
        (edge - (strip.rect.x + strip.rect.w)).abs() > 40 && (edge - strip.rect.x).abs() > 40,
        "premise: the strip's edges stay clear of x={edge}: {strip:?} {float:?}"
    );
    let band = |frame: &[u8]| -> Vec<[u8; 4]> {
        (edge - 8..=edge + 2)
            .map(|x| pixel(frame, x, mid))
            .collect()
    };
    fixture.state.request_render();
    fixture.state.render();
    let rest = band(&fixture.pixels_of(OutputId(1)));
    let background = pixel(&fixture.pixels_of(OutputId(1)), 3, CANVAS - 3);
    assert!(
        rest.iter().any(|p| *p != background),
        "premise: the ring is drawn at rest"
    );
    fixture.state.gesture_swipe_begin(3, 1000);
    fixture.state.gesture_swipe_update(-60.0, 0.0, 1010);
    assert_ne!(
        fixture.state.gesture_shift(Some(OutputId(1))),
        0,
        "premise: pulled"
    );
    fixture.state.request_render();
    fixture.state.render();
    let during = band(&fixture.pixels_of(OutputId(1)));
    assert_eq!(
        rest, during,
        "the floating window's ring moved with the strip"
    );
    fixture.state.gesture_swipe_end(true, 1020);
}
