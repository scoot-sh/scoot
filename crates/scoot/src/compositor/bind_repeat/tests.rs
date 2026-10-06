//! Tests for bind repeat: timing on a synthetic clock, the cancel paths,
//! and the actions that never repeat.
//!
//! Repeat counts are observed through `State::spawned_children`: every
//! re-fire of a `spawn true` bind forks exactly one child, tracked there
//! until the reaper drains it. No test here settles the event loop between
//! counts, so nothing is reaped mid-test and the set only grows.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::time::{Duration, Instant};

use scoot_core::Action;
use scoot_ipc::KeyCombo;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keycode, Keysym};
use smithay::reexports::calloop::timer::TimeoutAction;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::input::keysym_named;
use crate::compositor::keybindings::{BindFlags, Bound, Keybindings, Modifiers};
use crate::compositor::test_support::Harness;

const VOLUME_UP: &str = "XF86AudioRaiseVolume";
const VOLUME_DOWN: &str = "XF86AudioLowerVolume";

/// A repeatable `spawn true` bind on a bare key: no modifiers to hold, so
/// the test holds exactly one keycode.
fn repeat_spawn(key_name: &str) -> (Modifiers, Keysym, Bound, BindFlags) {
    (
        Modifiers::default(),
        keysym_named(key_name).expect("the repeat key is a known keysym"),
        Bound::Action(Action::Spawn(vec!["true".into()])),
        BindFlags {
            repeat: true,
            allow_when_locked: false,
        },
    )
}

/// The keycode holding `key_name` fires, for a `repeat_spawn` bind: what a
/// test holds down across several `key` calls to prove a bind repeats (or
/// does not).
fn combo_code(harness: &mut Harness<(), ()>, key_name: &str) -> Keycode {
    harness
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: key_name.into(),
        })
        .expect("the repeat key resolves on the test keymap")
}

/// A live `State` with `bind` (on `key_name`) inserted and its key held
/// down: the press has fired exactly once on return.
fn held_harness(
    key_name: &str,
    bind: (Modifiers, Keysym, Bound, BindFlags),
) -> (Harness<(), ()>, Keycode) {
    let mut harness: Harness<(), ()> = Harness::headless(Appearance::default(), 120);
    let (modifiers, keysym, bound, flags) = bind;
    harness
        .state
        .keybindings
        .insert(modifiers, keysym, bound, flags);
    assert_eq!(
        harness.state.spawned_children.len(),
        0,
        "nothing has spawned before the press"
    );
    let code = combo_code(&mut harness, key_name);
    harness.state.key(code, KeyState::Pressed);
    (harness, code)
}

/// The repeat delay and interval are the seat keyboard's own: the same pair
/// `State::new` hands clients through `wl_keyboard.repeat_info`, so binds
/// step exactly the way client-side key repeat does.
#[test]
fn repeat_timing_is_the_seat_keyboards_own() {
    assert_eq!(KEYBOARD_REPEAT_DELAY_MS, 200);
    assert_eq!(KEYBOARD_REPEAT_RATE_PER_SECOND, 25);
    assert_eq!(repeat_delay(), Duration::from_millis(200));
    assert_eq!(repeat_interval(), Duration::from_millis(40));
}

/// Holding a `repeat` bind's key re-fires past the delay at the rate, and
/// the release stops it: the timer after that fires into nothing.
#[test]
fn holding_a_repeat_bind_re_fires_at_the_rate_and_release_stops_it() {
    let (mut harness, code) = held_harness(VOLUME_UP, repeat_spawn(VOLUME_UP));
    assert_eq!(
        harness.state.spawned_children.len(),
        1,
        "the press fired once"
    );
    assert!(
        harness.state.bind_repeat_timer_live,
        "the press armed the timer"
    );

    // Before the delay: nothing re-fires.
    let pressed_at = harness.state.bind_repeat.as_ref().expect("armed").next - repeat_delay();
    assert!(
        matches!(
            harness
                .state
                .note_bind_repeat_timeout(pressed_at + repeat_delay() - Duration::from_millis(1)),
            TimeoutAction::ToDuration(left) if left == Duration::from_millis(1),
        ),
        "an early firing re-arms for the remainder instead of firing"
    );
    assert_eq!(harness.state.spawned_children.len(), 1);

    // At the delay: the first re-fire, then stepping at the rate.
    harness
        .state
        .note_bind_repeat_timeout(pressed_at + repeat_delay());
    assert_eq!(
        harness.state.spawned_children.len(),
        2,
        "first re-fire at the delay"
    );
    harness
        .state
        .note_bind_repeat_timeout(pressed_at + repeat_delay() + repeat_interval());
    assert_eq!(
        harness.state.spawned_children.len(),
        3,
        "then stepping at the rate"
    );

    // The release stops it: the live timer fires once more into nothing.
    harness.state.key(code, KeyState::Released);
    assert!(
        harness.state.bind_repeat.is_none(),
        "release cancels the repeat"
    );
    assert!(
        matches!(
            harness
                .state
                .note_bind_repeat_timeout(pressed_at + repeat_delay() + repeat_interval() * 2),
            TimeoutAction::Drop,
        ),
        "the cancelled timer drops without firing"
    );
    assert_eq!(
        harness.state.spawned_children.len(),
        3,
        "nothing fired after release"
    );
    assert!(
        !harness.state.bind_repeat_timer_live,
        "the timer dropped itself"
    );
}

/// A bind without `repeat` fires exactly once no matter how long its key is
/// held -- and arms no timer at all.
#[test]
fn a_bind_without_repeat_fires_once_and_arms_nothing() {
    let (modifiers, keysym, bound, _) = repeat_spawn(VOLUME_UP);
    let plain = (
        modifiers,
        keysym,
        bound,
        BindFlags {
            repeat: false,
            allow_when_locked: false,
        },
    );
    let (mut harness, code) = held_harness(VOLUME_UP, plain);
    assert_eq!(
        harness.state.spawned_children.len(),
        1,
        "the press fired once"
    );
    assert!(harness.state.bind_repeat.is_none(), "no repeat armed");
    assert!(
        !harness.state.bind_repeat_timer_live,
        "no timer source without a held repeatable key"
    );

    // Driving the (nonexistent) timer past the delay fires nothing.
    assert!(
        matches!(
            harness
                .state
                .note_bind_repeat_timeout(Instant::now() + repeat_delay() * 2),
            TimeoutAction::Drop,
        ),
        "no repeat armed, so the timer drops"
    );
    assert_eq!(harness.state.spawned_children.len(), 1);
    harness.state.key(code, KeyState::Released);
}

/// `quit`, `close` and `show-keymap` never repeat, even when flagged:
///
/// holding quit must never end the session, holding close must never work
/// through every window, and holding the keymap key must never stack
/// terminals. `config.rs` clears the flag at load with a warning; this is
/// the backstop for a table built any other way.
#[test]
fn quit_close_and_show_keymap_are_never_repeatable() {
    assert!(!bind_action_repeats(&Action::Quit));
    assert!(!bind_action_repeats(&Action::CloseFocused));
    assert!(!bind_action_repeats(&Action::ShowKeymap));
    assert!(bind_action_repeats(&Action::Spawn(vec!["true".into()])));
    assert!(bind_action_repeats(&Action::FocusColumn(
        scoot_core::Horizontal::Left
    )));
}

/// Two repeatable keys held at once: the latest press wins, and releasing
/// it stops the repeat even while the first key is still down.
#[test]
fn a_second_repeatable_press_replaces_the_first() {
    let mut harness: Harness<(), ()> = Harness::headless(Appearance::default(), 120);
    let (first_mods, first_keysym, first_bound, first_flags) = repeat_spawn(VOLUME_UP);
    harness
        .state
        .keybindings
        .insert(first_mods, first_keysym, first_bound, first_flags);
    let (second_mods, second_keysym, second_bound, second_flags) = repeat_spawn(VOLUME_DOWN);
    harness
        .state
        .keybindings
        .insert(second_mods, second_keysym, second_bound, second_flags);

    let first = combo_code(&mut harness, VOLUME_UP);
    let second = combo_code(&mut harness, VOLUME_DOWN);
    assert_ne!(first, second, "two keys, two keycodes");

    let t0 = Instant::now();
    harness.state.key(first, KeyState::Pressed);
    assert_eq!(
        harness.state.bind_repeat.as_ref().expect("armed").keycode,
        first
    );
    harness.state.key(second, KeyState::Pressed);
    assert_eq!(
        harness
            .state
            .bind_repeat
            .as_ref()
            .expect("still armed")
            .keycode,
        second,
        "the latest press wins"
    );
    assert_eq!(
        harness.state.spawned_children.len(),
        2,
        "one fire per press"
    );

    // Releasing the first key -- its repeat was replaced -- changes
    // nothing; releasing the second stops it.
    harness.state.key(first, KeyState::Released);
    assert!(
        harness.state.bind_repeat.is_some(),
        "the second key still repeats"
    );
    harness
        .state
        .note_bind_repeat_timeout(t0 + repeat_delay() * 2);
    assert_eq!(harness.state.spawned_children.len(), 3);
    harness.state.key(second, KeyState::Released);
    assert!(harness.state.bind_repeat.is_none());
}

/// Cancelling with nothing armed is a silent no-op -- every cancel path
/// calls this unconditionally.
#[test]
fn cancelling_with_nothing_armed_is_a_no_op() {
    let mut harness: Harness<(), ()> = Harness::headless(Appearance::default(), 120);
    harness.state.cancel_bind_repeat();
    harness.state.cancel_bind_repeat_for(Keycode::new(9));
    assert!(
        matches!(
            harness.state.note_bind_repeat_timeout(Instant::now()),
            TimeoutAction::Drop,
        ),
        "nothing armed, so the timer drops"
    );
    assert!(!harness.state.bind_repeat_timer_live);
}

/// Releasing a key that is not the repeated one leaves the repeat alone: a
/// modifier let go mid-hold must not stop the stepping.
#[test]
fn releasing_another_key_leaves_the_repeat_armed() {
    let (mut harness, code) = held_harness(VOLUME_UP, repeat_spawn(VOLUME_UP));
    let other = combo_code(&mut harness, VOLUME_DOWN);
    assert_ne!(other, code);
    // A press with no bind still releases through here: it must not cancel.
    harness.state.key(other, KeyState::Pressed);
    harness.state.key(other, KeyState::Released);
    assert!(
        harness.state.bind_repeat.is_some(),
        "another key's release is not a cancel"
    );
    assert_eq!(harness.state.spawned_children.len(), 1);
    harness.state.key(code, KeyState::Released);
}

/// A kernel repeat -- a second press of the already-held key -- never
/// reaches the bind filter (Smithay absorbs it before the filter, the
/// pinned fork's `key_input` "don't double-run the filter"), so it neither
/// double-fires the bind nor re-arms the repeat: the timer is the only
/// re-fire source.
#[test]
fn a_repeat_press_of_the_held_key_fires_nothing_extra() {
    let (mut harness, code) = held_harness(VOLUME_UP, repeat_spawn(VOLUME_UP));
    assert_eq!(harness.state.spawned_children.len(), 1);
    harness.state.key(code, KeyState::Pressed);
    assert_eq!(
        harness.state.spawned_children.len(),
        1,
        "the absorbed re-press must not fire again"
    );
    assert!(
        harness.state.bind_repeat.is_some(),
        "still armed exactly once"
    );
    harness.state.key(code, KeyState::Released);
    assert!(harness.state.bind_repeat.is_none());
}

/// A VT switch attempt ends an in-flight repeat even before any pause
/// lands: the key is gone with the session. No tty here, so the switch
/// itself is ignored -- the cancel still runs.
#[test]
fn a_vt_switch_attempt_cancels_an_in_flight_repeat() {
    let (mut harness, code) = held_harness(VOLUME_UP, repeat_spawn(VOLUME_UP));
    assert!(harness.state.bind_repeat.is_some());
    let _ = harness.state.change_vt(3);
    assert!(
        harness.state.bind_repeat.is_none(),
        "the switch attempt cancelled the repeat"
    );
    harness.state.key(code, KeyState::Released);
}

/// Removing an output ends an in-flight repeat: the focus and arrangement
/// the removal re-derives may retire the context the press happened in.
#[test]
fn removing_an_output_cancels_an_in_flight_repeat() {
    let (mut harness, code) = held_harness(VOLUME_UP, repeat_spawn(VOLUME_UP));
    assert!(harness.state.bind_repeat.is_some());
    let second =
        crate::compositor::headless::add_output(&mut harness.state, "headless-2", 120, 120)
            .expect("a second headless output");
    assert!(harness.state.remove_output(second));
    assert!(
        harness.state.bind_repeat.is_none(),
        "the output removal cancelled the repeat"
    );
    harness.state.key(code, KeyState::Released);
}

/// A `Keybindings` with no flags behaves exactly as before: `match_key`
/// reports the action and empty flags.
#[test]
fn a_table_without_flags_matches_like_a_plain_string_bind() {
    let mut table = Keybindings::default();
    let (modifiers, keysym, bound, _) = repeat_spawn(VOLUME_UP);
    table.insert(modifiers, keysym, bound, BindFlags::default());
    assert_eq!(
        table.match_key(keysym, modifiers),
        Some((
            Bound::Action(Action::Spawn(vec!["true".into()])),
            BindFlags::default()
        )),
    );
}
