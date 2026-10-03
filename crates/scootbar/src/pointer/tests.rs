use super::*;
use crate::outputs::Outputs;

const LEFT: u32 = 0x110;
const RIGHT: u32 = 0x111;
const MIDDLE: u32 = 0x112;

fn ids() -> (OutputId, OutputId) {
    let mut outputs: Outputs<()> = Outputs::default();
    (outputs.add(1, |_| ()), outputs.add(2, |_| ()))
}

fn target(output: OutputId, member: usize) -> Target {
    Target { output, member }
}

fn focused() -> (Pointer, OutputId, OutputId) {
    let (a, b) = ids();
    let mut pointer = Pointer::default();
    pointer.enter(a, 10.0, 5.0);
    (pointer, a, b)
}

/// One wheel notch as a wheel of each protocol age sends it: a frame.
fn notch(pointer: &mut Pointer, down: bool) {
    pointer.axis(if down { 15.0 } else { -15.0 });
    pointer.axis_value120(if down { 120 } else { -120 });
    pointer.frame();
}

// ---- clicks ----

#[test]
fn a_press_and_release_on_the_same_module_is_a_click() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 2);
    pointer.press(LEFT, Some(here));
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );
}

#[test]
fn a_press_says_which_trigger_it_armed_and_none_when_it_armed_nothing() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    assert_eq!(pointer.press(LEFT, Some(here)), Some(Trigger::Click));
    // A second button while one is held: a chord, nothing armed.
    assert_eq!(pointer.press(RIGHT, Some(here)), None);
    pointer.release(LEFT, Some(here));
    pointer.release(RIGHT, Some(here));
    // Over no module, or a button the bar does not answer.
    assert_eq!(pointer.press(LEFT, None), None);
    pointer.release(LEFT, None);
    assert_eq!(pointer.press(0x113, Some(here)), None);
    assert_eq!(
        pointer.press(MIDDLE, Some(here)),
        Some(Trigger::MiddleClick)
    );
}

#[test]
fn each_button_is_its_own_trigger() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    for (code, trigger) in [
        (LEFT, Trigger::Click),
        (RIGHT, Trigger::RightClick),
        (MIDDLE, Trigger::MiddleClick),
    ] {
        pointer.press(code, Some(here));
        assert_eq!(pointer.release(code, Some(here)), Some((trigger, here)));
    }
}

#[test]
fn a_release_after_the_pointer_moved_to_another_module_is_nothing() {
    let (mut pointer, a, _) = focused();
    pointer.press(LEFT, Some(target(a, 0)));
    assert_eq!(pointer.release(LEFT, Some(target(a, 1))), None);
    // And nothing stays armed: the next release alone is nothing too.
    assert_eq!(pointer.release(LEFT, Some(target(a, 0))), None);
}

#[test]
fn a_release_off_every_module_is_nothing() {
    let (mut pointer, a, _) = focused();
    pointer.press(LEFT, Some(target(a, 0)));
    assert_eq!(pointer.release(LEFT, None), None);
}

#[test]
fn a_release_on_another_output_is_nothing() {
    let (mut pointer, a, b) = focused();
    pointer.press(LEFT, Some(target(a, 0)));
    assert_eq!(pointer.release(LEFT, Some(target(b, 0))), None);
}

#[test]
fn a_release_after_the_pointer_left_is_nothing() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.leave();
    assert_eq!(pointer.release(LEFT, Some(here)), None);
    assert!(pointer.focus().is_none());
}

#[test]
fn a_release_with_no_press_is_nothing() {
    // A press that began on another surface, or before the bar had the
    // pointer.
    let (mut pointer, a, _) = focused();
    assert_eq!(pointer.release(LEFT, Some(target(a, 0))), None);
}

#[test]
fn a_press_over_no_module_arms_nothing() {
    let (mut pointer, a, _) = focused();
    pointer.press(LEFT, None);
    assert_eq!(pointer.release(LEFT, Some(target(a, 0))), None);
}

#[test]
fn a_second_button_makes_a_chord_not_a_click() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.press(RIGHT, Some(here));
    assert_eq!(pointer.release(RIGHT, Some(here)), None);
    assert_eq!(pointer.release(LEFT, Some(here)), None);
}

#[test]
fn a_third_button_after_a_chord_arms_nothing_while_one_is_held() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.press(RIGHT, Some(here));
    // Left comes up, right is still down: a middle press is part of the
    // same chord, not a click of its own.
    assert_eq!(pointer.release(LEFT, Some(here)), None);
    pointer.press(MIDDLE, Some(here));
    assert_eq!(pointer.release(MIDDLE, Some(here)), None);
    assert_eq!(pointer.release(RIGHT, Some(here)), None);
    // Everything up: the next press is a click again.
    pointer.press(MIDDLE, Some(here));
    assert_eq!(
        pointer.release(MIDDLE, Some(here)),
        Some((Trigger::MiddleClick, here))
    );
}

#[test]
fn the_wrong_buttons_release_does_not_fire_or_disarm() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    assert_eq!(pointer.release(RIGHT, Some(here)), None);
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );
}

#[test]
fn other_buttons_are_ignored() {
    // BTN_SIDE, BTN_EXTRA, BTN_TOUCH and a nonsense code.
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    for code in [0x113, 0x114, 0x14a, 0, u32::MAX] {
        pointer.press(code, Some(here));
        assert_eq!(pointer.release(code, Some(here)), None, "code {code:#x}");
    }
    // They did not disturb a real press.
    pointer.press(LEFT, Some(here));
    pointer.press(0x113, Some(here));
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );
}

#[test]
fn leaving_and_entering_forgets_a_press() {
    let (mut pointer, a, b) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.enter(b, 1.0, 1.0);
    assert_eq!(pointer.release(LEFT, Some(here)), None);
}

#[test]
fn an_output_going_away_clears_only_its_own_pointer() {
    let (mut pointer, a, b) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.forget(b);
    assert_eq!(pointer.focus().map(|focus| focus.output), Some(a));
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );

    pointer.press(LEFT, Some(here));
    pointer.forget(a);
    assert!(pointer.focus().is_none());
    assert_eq!(pointer.release(LEFT, Some(here)), None);
}

#[test]
fn motion_moves_the_focus_and_does_nothing_without_one() {
    let (mut pointer, a, _) = focused();
    pointer.motion(40.5, 3.0);
    assert_eq!(
        pointer.focus(),
        Some(Focus {
            output: a,
            x: 40.5,
            y: 3.0
        })
    );
    pointer.leave();
    pointer.motion(1.0, 1.0);
    assert!(pointer.focus().is_none());
}

#[test]
fn a_reload_disarms_a_press_but_the_pointer_stays_where_it_is() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    notch(&mut pointer, true);
    pointer.disarm();
    assert_eq!(pointer.focus().map(|focus| focus.output), Some(a));
    assert_eq!(pointer.release(LEFT, Some(here)), None);
    assert!(!pointer.scroll_waiting());
    // And it works on, with no new `enter`.
    pointer.press(LEFT, Some(here));
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );
}

#[test]
fn a_reload_with_a_button_held_keeps_the_chord_detection() {
    let (mut pointer, a, _) = focused();
    let here = target(a, 0);
    pointer.press(LEFT, Some(here));
    pointer.disarm();
    // The left button is still down: a right press now is a second button
    // held, a chord, and neither releases is a click.
    pointer.press(RIGHT, Some(here));
    assert_eq!(pointer.release(RIGHT, Some(here)), None);
    assert_eq!(pointer.release(LEFT, Some(here)), None);
    // Both are up: a press is a plain click again.
    pointer.press(LEFT, Some(here));
    assert_eq!(
        pointer.release(LEFT, Some(here)),
        Some((Trigger::Click, here))
    );
}

// ---- touch ----

#[test]
fn touch_and_the_keyboard_never_get_a_pointer() {
    assert!(!wants_pointer(Capability::Touch));
    assert!(!wants_pointer(Capability::Keyboard));
    assert!(!wants_pointer(Capability::Touch | Capability::Keyboard));
    assert!(!wants_pointer(Capability::empty()));
    assert!(wants_pointer(Capability::Pointer));
    assert!(wants_pointer(Capability::Pointer | Capability::Touch));
}

// ---- scroll ----

#[test]
fn one_notch_is_one_step_even_with_both_value_kinds() {
    // A wheel sends the continuous value and the discrete one for the same
    // movement: counted once.
    let (mut pointer, _, _) = focused();
    let now = Instant::now();
    notch(&mut pointer, true);
    assert_eq!(pointer.take_scroll(now), Some((Trigger::ScrollDown, 1)));
}

#[test]
fn the_order_of_the_two_value_kinds_does_not_matter() {
    let (mut pointer, _, _) = focused();
    pointer.axis_value120(-120);
    pointer.axis(-15.0);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(Instant::now()),
        Some((Trigger::ScrollUp, 1))
    );
}

#[test]
fn discrete_notches_count_whole() {
    let (mut pointer, _, _) = focused();
    pointer.axis_discrete(3);
    pointer.axis(45.0);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(Instant::now()),
        Some((Trigger::ScrollDown, 3))
    );
}

#[test]
fn a_smooth_scroll_adds_up_to_steps() {
    // A touchpad: only continuous values, a few pixels a frame.
    let (mut pointer, _, _) = focused();
    let now = Instant::now();
    let mut taken = 0;
    for _ in 0..30 {
        pointer.axis(2.5);
        pointer.frame();
        // 30 frames of 2.5 px: 75 px, 5 steps, with no remainder lost.
        if let Some((trigger, steps)) = pointer.take_scroll(now + SCROLL_FRAME * taken) {
            assert_eq!(trigger, Trigger::ScrollDown);
            taken += steps;
        }
    }
    let tail = pointer.take_scroll(now + SCROLL_FRAME * 1000);
    assert_eq!(taken + tail.map_or(0, |(_, steps)| steps), 5);
}

#[test]
fn half_notches_add_up() {
    let (mut pointer, _, _) = focused();
    pointer.axis_value120(60);
    pointer.frame();
    assert_eq!(pointer.take_scroll(Instant::now()), None);
    pointer.axis_value120(60);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(Instant::now()),
        Some((Trigger::ScrollDown, 1))
    );
}

#[test]
fn reversing_direction_drops_the_remainder() {
    let (mut pointer, _, _) = focused();
    pointer.axis_value120(100);
    pointer.frame();
    // Not 100 - 50: the scroll turned around, and starts from nothing.
    pointer.axis_value120(-50);
    pointer.frame();
    pointer.axis_value120(-50);
    pointer.frame();
    assert_eq!(pointer.take_scroll(Instant::now()), None);
    pointer.axis_value120(-20);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(Instant::now()),
        Some((Trigger::ScrollUp, 1))
    );
}

#[test]
fn axis_stop_drops_the_remainder() {
    let (mut pointer, _, _) = focused();
    pointer.axis_value120(100);
    pointer.frame();
    pointer.axis_stop();
    pointer.axis_value120(100);
    pointer.frame();
    assert_eq!(pointer.take_scroll(Instant::now()), None);
}

#[test]
fn opposite_steps_in_one_window_cancel() {
    let (mut pointer, _, _) = focused();
    notch(&mut pointer, true);
    notch(&mut pointer, false);
    assert_eq!(pointer.take_scroll(Instant::now()), None);
    assert!(!pointer.scroll_waiting());
}

#[test]
fn a_flood_is_one_action_per_frame() {
    // A free-spinning wheel or a touchpad fling: thousands of events. The
    // counter is bounded, the first take carries it, and a second take in
    // the same instant, or before a frame has passed, carries nothing.
    let (mut pointer, _, _) = focused();
    let t0 = Instant::now();
    for _ in 0..10_000 {
        notch(&mut pointer, true);
    }
    assert!(pointer.scroll_waiting());
    assert_eq!(
        pointer.take_scroll(t0),
        Some((Trigger::ScrollDown, MAX_STEPS))
    );
    assert_eq!(pointer.take_scroll(t0), None);

    // Events keep coming, all inside one frame of the last action: held.
    for _ in 0..1000 {
        notch(&mut pointer, true);
    }
    assert_eq!(pointer.take_scroll(t0 + SCROLL_FRAME / 2), None);
    assert_eq!(
        pointer.scroll_wait(t0 + SCROLL_FRAME / 2),
        Some(SCROLL_FRAME / 2)
    );
    // The frame passes: exactly one more action, carrying what piled up.
    assert_eq!(
        pointer.take_scroll(t0 + SCROLL_FRAME),
        Some((Trigger::ScrollDown, MAX_STEPS))
    );
    assert_eq!(pointer.take_scroll(t0 + SCROLL_FRAME), None);
    assert_eq!(pointer.scroll_wait(t0 + SCROLL_FRAME), None);
}

#[test]
fn a_flood_counts_actions_not_events() {
    // 1,000 events a second for one second: at most one action per frame.
    let (mut pointer, _, _) = focused();
    let t0 = Instant::now();
    let mut actions = 0;
    let mut steps = 0;
    for ms in 0..1000u32 {
        notch(&mut pointer, true);
        if let Some((_, n)) = pointer.take_scroll(t0 + Duration::from_millis(ms.into())) {
            actions += 1;
            steps += n;
        }
    }
    assert!(actions <= 1000 / 16 + 1, "{actions} actions");
    assert!(actions >= 50, "{actions} actions: the flood was swallowed");
    // Nothing is lost short of the cap: every notch is a step somewhere.
    let rest = pointer
        .take_scroll(t0 + Duration::from_secs(2))
        .map_or(0, |(_, n)| n);
    assert_eq!(steps + rest, 1000);
}

#[test]
fn nothing_waits_until_a_frame_closes() {
    let (mut pointer, _, _) = focused();
    pointer.axis_value120(120);
    assert!(!pointer.scroll_waiting());
    assert_eq!(pointer.take_scroll(Instant::now()), None);
    pointer.frame();
    assert!(pointer.scroll_waiting());
}

#[test]
fn an_idle_pointer_asks_for_no_wakeup() {
    let (pointer, _, _) = focused();
    let now = Instant::now();
    assert!(!pointer.scroll_waiting());
    assert_eq!(pointer.scroll_wait(now), None);
}

#[test]
fn leaving_drops_waiting_steps_and_the_remainder() {
    let (mut pointer, a, _) = focused();
    notch(&mut pointer, true);
    pointer.axis_value120(60);
    pointer.frame();
    pointer.leave();
    assert!(!pointer.scroll_waiting());
    pointer.enter(a, 0.0, 0.0);
    pointer.axis_value120(60);
    pointer.frame();
    assert_eq!(pointer.take_scroll(Instant::now()), None);
}

#[test]
fn scroll_with_no_focus_is_ignored() {
    let mut pointer = Pointer::default();
    pointer.axis(100.0);
    pointer.axis_value120(120);
    pointer.frame();
    assert!(!pointer.scroll_waiting());
}

#[test]
fn hostile_values_do_not_overflow_or_wrap() {
    let (mut pointer, _, _) = focused();
    let now = Instant::now();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        pointer.axis(value);
        pointer.frame();
    }
    assert!(!pointer.scroll_waiting());
    pointer.axis(f64::MAX);
    pointer.axis_discrete(i32::MAX);
    pointer.axis_value120(i32::MAX);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(now),
        Some((Trigger::ScrollDown, MAX_STEPS))
    );
    pointer.axis(f64::MIN);
    pointer.axis_discrete(i32::MIN);
    pointer.axis_value120(i32::MIN);
    pointer.frame();
    assert_eq!(
        pointer.take_scroll(now + SCROLL_FRAME),
        Some((Trigger::ScrollUp, MAX_STEPS))
    );
}

#[test]
fn a_clock_that_runs_backwards_does_not_panic() {
    // `take_scroll` with an earlier `now` than the last action: still held
    // back, never an underflow.
    let (mut pointer, _, _) = focused();
    let later = Instant::now() + Duration::from_secs(5);
    notch(&mut pointer, true);
    assert!(pointer.take_scroll(later).is_some());
    notch(&mut pointer, true);
    assert_eq!(pointer.take_scroll(later - Duration::from_secs(1)), None);
    assert!(
        pointer
            .scroll_wait(later - Duration::from_secs(1))
            .is_some()
    );
}

#[test]
fn the_warm_pointer_allocates_nothing() {
    // Input is the hot path: every event, from a mouse at 1 kHz or a
    // touchpad at 120 Hz, goes through here.
    let (mut pointer, a, _) = focused();
    let here = target(a, 1);
    let t0 = Instant::now();
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for round in 0..200u32 {
            let x = f64::from(round);
            pointer.enter(a, x, 5.0);
            pointer.motion(x + 1.0, 6.0);
            pointer.press(LEFT, Some(here));
            let _ = pointer.release(LEFT, Some(here));
            pointer.axis(15.0);
            pointer.axis_discrete(1);
            pointer.axis_value120(-120);
            pointer.axis_stop();
            pointer.frame();
            let _ = pointer.scroll_waiting();
            let _ = pointer.take_scroll(t0 + SCROLL_FRAME * round);
            let _ = pointer.scroll_wait(t0);
            pointer.disarm();
            pointer.forget(a);
            let _ = pointer.focus();
            pointer.leave();
        }
    });
    assert_eq!(allocations, 0);
}
