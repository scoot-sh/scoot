//! What one key event costs: [`State::key`](crate::compositor::State), the
//! path every keyboard source reaches (libinput under `--tty`, the host
//! under `--nested`, IPC `key`/`type`), with a real client window holding
//! the keyboard.
//!
//! The scene is the rounded bench's three client windows; the client binds
//! no `wl_seat`, so no key event is written to its socket and what is timed
//! is the compositor's own per-key work: the recipient lookup
//! (`current_focus`, which clones the seat's focus), the held-key and
//! keybinding filter, and Smithay's delivery walk. That is exactly the part
//! a change to the keyboard focus type can move. Run by hand:
//!
//! ```text
//! cargo test -p scoot --bin scoot key_dispatch_cost -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::time::{Duration, Instant};

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

use super::{RUNS, RoundedFixture, rounded_scene};

/// Key events (a press and a release each count one) per timed run.
const KEYS: u32 = 20_000;

/// `KEY_A` as an xkb keycode: a plain letter, bound to nothing.
const KEY_A: u32 = 30 + 8;

fn time_keys(fixture: &mut RoundedFixture, keys: u32) -> Duration {
    let started = Instant::now();
    for i in 0..keys {
        let state = if i % 2 == 0 {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        fixture.state.key(Keycode::new(KEY_A), state);
    }
    started.elapsed()
}

#[test]
#[ignore = "prints per-key timings for a human; asserts nothing"]
fn key_dispatch_cost() {
    let mut fixture = rounded_scene(3, 1);
    assert!(
        fixture
            .state
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some(),
        "the scene must have a window holding the keyboard"
    );
    time_keys(&mut fixture, KEYS / 10);
    let mut runs: Vec<Duration> = (0..RUNS).map(|_| time_keys(&mut fixture, KEYS)).collect();
    runs.sort();
    let per = |d: Duration| d / KEYS;
    println!(
        "key dispatch, a focused client window: median {:?} per key (min {:?}, max {:?}; {} runs x {KEYS})",
        per(runs[runs.len() / 2]),
        per(runs[0]),
        per(runs[runs.len() - 1]),
        runs.len(),
    );
}
