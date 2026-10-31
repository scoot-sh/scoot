//! Scroll binds on a live headless session with a real client: what the
//! client's `wl_pointer` receives, and what the table fires. The pure
//! accumulator is pinned in `scroll_binds/tests.rs`.
//!
//! The observable action is `cycle-column-width`: each fire moves the focused
//! column through its three widths, so one fire and two fires land on
//! different widths (three would wrap, so no test fires three).

use scoot_core::Action;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

use super::*;
use crate::compositor::keybindings::{BindFlags, Bound, Modifiers, ScrollDir, Trigger};

const SUPER: Modifiers = Modifiers {
    super_: true,
    shift: false,
    ctrl: false,
    alt: false,
};

impl ScrollFixture {
    /// Binds `dir` (with `mods` held) to `cycle-column-width`.
    fn bind(&mut self, mods: Modifiers, dir: ScrollDir) {
        self.fixture.state.keybindings.insert_trigger(
            mods,
            Trigger::Scroll(dir),
            Bound::Action(Action::CycleColumnWidth),
            BindFlags::default(),
        );
    }

    /// The Super key, resolved from the keymap rather than hard-coded.
    fn super_key(&mut self) -> Keycode {
        let combo: KeyCombo = "super+h".parse().expect("a parsable combination");
        let (_, held) = self
            .fixture
            .state
            .resolve_combo(&combo)
            .expect("`super+h` is pressable on a US layout");
        *held.as_slice().first().expect("Super is held for it")
    }

    fn press_super(&mut self) -> Keycode {
        let key = self.super_key();
        self.fixture.state.key(key, KeyState::Pressed);
        key
    }

    /// The focused window's current width: the observable of a fire.
    fn width(&mut self) -> i32 {
        let arrangement = self.fixture.state.world.arrange();
        let focused = arrangement.focused.expect("a focused window");
        arrangement
            .placements
            .iter()
            .find(|placement| placement.id == focused)
            .expect("the focused window is placed")
            .rect
            .w
    }

    /// One finger-scroll frame on the vertical axis. `Some(0.0)` is the
    /// terminating zero, which `scroll_frame` turns into the `axis_stop`.
    fn finger_vertical(&mut self, vertical: f64) {
        let time = InputTime::from_millis(self.fixture.state.millis());
        let frame = scroll_frame(
            time,
            Some(AxisSource::Finger),
            no_axis(),
            plain_axis(Some(vertical)),
        )
        .expect("finger motion is a scroll");
        self.fixture.state.emit_scroll(frame);
    }

    fn finger_stop(&mut self) {
        self.finger_vertical(0.0);
    }

    fn seen(&mut self) -> Scrolled {
        self.fixture.settle();
        self.scroll_report()
    }
}

fn nothing_arrived(seen: &Scrolled) {
    assert!(seen.axes.is_empty(), "axis events leaked: {seen:?}");
    assert!(seen.value120s.is_empty(), "value120 leaked: {seen:?}");
    assert!(seen.stops.is_empty(), "a stop leaked: {seen:?}");
    assert!(seen.sources.is_empty(), "a source leaked: {seen:?}");
}

#[test]
fn a_wheel_click_fires_once_and_the_client_hears_nothing() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.press_super();
    s.fixture.state.scroll(0.0, 15.0);
    let after = s.width();
    assert_ne!(before, after, "the bind did not fire");
    let seen = s.seen();
    nothing_arrived(&seen);
    // Two more clicks are two more fires, one each.
    s.fixture.state.scroll(0.0, 15.0);
    let second = s.width();
    assert_ne!(second, after);
}

#[test]
fn a_scroll_no_bind_matches_passes_through_whole() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    // Wrong modifier state: Super is not held.
    s.fixture.state.scroll(0.0, 15.0);
    assert_eq!(s.width(), before, "fired without its modifier");
    let seen = s.seen();
    assert_eq!(seen.sources, vec![wl_pointer::AxisSource::Wheel]);
    assert_eq!(seen.axes, vec![(wl_pointer::Axis::VerticalScroll, 15.0)]);
    assert_eq!(
        seen.value120s,
        vec![(wl_pointer::Axis::VerticalScroll, 120)]
    );
}

#[test]
fn the_other_direction_passes_through() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.press_super();
    s.fixture.state.scroll(0.0, -15.0);
    assert_eq!(s.width(), before);
    let seen = s.seen();
    assert_eq!(seen.axes, vec![(wl_pointer::Axis::VerticalScroll, -15.0)]);
}

#[test]
fn horizontal_directions_bind_too() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Left);
    s.bind(SUPER, ScrollDir::Right);
    let before = s.width();
    s.press_super();
    s.fixture.state.scroll(-15.0, 0.0);
    let left = s.width();
    assert_ne!(left, before);
    s.fixture.state.scroll(15.0, 0.0);
    assert_ne!(s.width(), left);
    nothing_arrived(&s.seen());
}

#[test]
fn a_diagonal_scroll_is_taken_per_axis() {
    let mut s = ScrollFixture::new();
    // Only the vertical half is bound: the horizontal half must survive.
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.press_super();
    s.fixture.state.scroll(15.0, 15.0);
    assert_ne!(s.width(), before);
    let seen = s.seen();
    assert_eq!(seen.axes, vec![(wl_pointer::Axis::HorizontalScroll, 15.0)]);
    assert_eq!(
        seen.value120s,
        vec![(wl_pointer::Axis::HorizontalScroll, 120)]
    );
}

#[test]
fn a_finger_swipe_fires_per_step_and_the_client_never_sees_it() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.press_super();
    // The default step is 60 units: 40 alone fires nothing, +40 fires one.
    s.finger_vertical(40.0);
    assert_eq!(s.width(), before, "40 of 60 units fired early");
    s.finger_vertical(40.0);
    let fired = s.width();
    assert_ne!(fired, before);
    s.finger_stop();
    nothing_arrived(&s.seen());
}

#[test]
fn a_swipe_that_started_before_super_stays_the_clients() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.finger_vertical(10.0);
    s.press_super();
    s.finger_vertical(200.0);
    s.finger_stop();
    assert_eq!(s.width(), before, "claimed a sequence the client had begun");
    let seen = s.seen();
    assert_eq!(
        seen.axes,
        vec![
            (wl_pointer::Axis::VerticalScroll, 10.0),
            (wl_pointer::Axis::VerticalScroll, 200.0)
        ]
    );
    assert_eq!(seen.stops, vec![wl_pointer::Axis::VerticalScroll]);
}

#[test]
fn a_claimed_swipe_stays_claimed_after_super_is_released() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let key = s.press_super();
    s.finger_vertical(70.0);
    let fired = s.width();
    s.fixture.state.key(key, KeyState::Released);
    // The rest of the swipe, and its end, belong to nobody: the client never
    // saw the start, so it must not see the middle or the stop.
    s.finger_vertical(300.0);
    s.finger_stop();
    assert_eq!(s.width(), fired, "fired without the modifier");
    nothing_arrived(&s.seen());
    // And the next swipe is the client's again.
    s.finger_vertical(25.0);
    s.finger_stop();
    let seen = s.seen();
    assert_eq!(seen.axes, vec![(wl_pointer::Axis::VerticalScroll, 25.0)]);
    assert_eq!(seen.stops, vec![wl_pointer::Axis::VerticalScroll]);
}

#[test]
fn a_reversal_inside_a_claimed_swipe_is_swallowed() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    s.press_super();
    s.finger_vertical(70.0);
    // Up is not bound, but this sequence is already ours.
    s.finger_vertical(-30.0);
    s.finger_stop();
    nothing_arrived(&s.seen());
}

#[test]
fn a_scroll_emitted_with_binds_off_passes_a_bound_scroll_through() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    let before = s.width();
    s.press_super();
    let time = InputTime::from_millis(s.fixture.state.millis());
    let frame = scroll_frame(
        time,
        Some(AxisSource::Wheel),
        no_axis(),
        ScrollAxis {
            value: Some(15.0),
            v120: Some(120),
            stop: false,
            direction: AxisRelativeDirection::Identical,
        },
    )
    .expect("a wheel scroll");
    s.fixture.state.emit_scroll_with(frame, false);
    assert_eq!(s.width(), before, "binds ran for a gated source");
    let seen = s.seen();
    assert_eq!(seen.axes, vec![(wl_pointer::Axis::VerticalScroll, 15.0)]);
}

#[test]
fn an_absurd_injected_delta_returns_promptly() {
    let mut s = ScrollFixture::new();
    s.bind(SUPER, ScrollDir::Down);
    s.press_super();
    let started = Instant::now();
    s.fixture.state.scroll(0.0, 1e300);
    s.fixture.state.scroll(0.0, f64::MAX);
    s.fixture.state.scroll(f64::NAN, f64::INFINITY);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a huge delta stalled the compositor"
    );
}

#[test]
fn ipc_scroll_runs_the_binds_like_the_wheel() {
    let mut s = ScrollFixture::new();
    // A bare scroll bind: no modifier needed, so the request alone drives it.
    s.bind(Modifiers::default(), ScrollDir::Down);
    let before = s.width();
    let reply = s
        .fixture
        .state
        .handle_request(Request::Scroll { dx: 0.0, dy: 15.0 });
    assert!(matches!(reply, Response::Ok { .. }), "{reply:?}");
    assert_ne!(s.width(), before);
    nothing_arrived(&s.seen());
}

#[test]
fn with_no_scroll_binds_every_scroll_is_untouched() {
    let mut s = ScrollFixture::new();
    s.press_super();
    s.fixture.state.scroll(0.0, 15.0);
    s.finger_vertical(12.0);
    s.finger_stop();
    let seen = s.seen();
    assert_eq!(
        seen.axes,
        vec![
            (wl_pointer::Axis::VerticalScroll, 15.0),
            (wl_pointer::Axis::VerticalScroll, 12.0)
        ]
    );
    assert_eq!(seen.stops, vec![wl_pointer::Axis::VerticalScroll]);
}

/// Per-event cost of the scroll filter, the number the PR quotes. Ignored:
/// run with `cargo test -p scoot --bin scoot bench_scroll_filter -- --ignored
/// --nocapture`. Times `filter_scroll` itself, not delivery, so the three
/// cases are the cost this change adds to `emit_scroll`:
///
/// 1. no scroll bind in the table (the default session): two integer tests;
/// 2. binds exist but this event matches none (plain scrolling in a session
///    that has a `super+scroll-*` bind): a keyboard-state read and a table
///    scan;
/// 3. a bind matches and the event stays under one step (an accumulating
///    swipe).
#[test]
#[ignore = "timing, not correctness"]
fn bench_scroll_filter() {
    const ROUNDS: u32 = 1_000_000;
    let mut s = ScrollFixture::new();
    let frame = scroll_frame(
        InputTime::from_millis(1),
        Some(AxisSource::Finger),
        no_axis(),
        plain_axis(Some(1e-6)),
    )
    .expect("finger motion");
    let time = |label: &str, s: &mut ScrollFixture| {
        let started = Instant::now();
        let mut kept = 0u32;
        for _ in 0..ROUNDS {
            kept += u32::from(
                s.fixture
                    .state
                    .filter_scroll(std::hint::black_box(frame))
                    .is_some(),
            );
        }
        let per = started.elapsed().as_nanos() as f64 / f64::from(ROUNDS);
        println!("{label}: {per:.1} ns/event ({kept} of {ROUNDS} forwarded)");
    };
    time("no scroll binds        ", &mut s);
    s.bind(SUPER, ScrollDir::Down);
    time("binds, event unmatched ", &mut s);
    s.press_super();
    // 1e-6 units per event, a million events: one unit against a 60-unit step, so it accumulates and never fires.
    time("bind matches, sub-step ", &mut s);
}
