//! What one pointer motion costs: [`State::pointer_move`], the path every
//! motion source reaches (IPC, `--nested`, and `--tty`'s libinput through
//! `pointer_move_relative`), over a floating window -- and while that window
//! is being dragged, which is the per-event path `floating/grab.rs` adds.
//!
//! The scene is three real client windows with the last floated (the
//! rounded bench's client, so the hit test walks real surfaces). Motion
//! alternates between two points a pixel apart inside the floating window,
//! so no motion changes pointer focus -- the steady state a hand on a mouse
//! produces at the device's rate. Run by hand:
//!
//! ```text
//! cargo test -p scoot --bin scoot pointer_motion -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.
//!
//! [`State::pointer_move`]: crate::compositor::State::pointer_move

use std::time::{Duration, Instant};

use scoot_core::{Action, Rect};
use scoot_ipc::PointerButton;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

use super::{RUNS, RoundedFixture, RoundedStep, rounded_scene};

/// Motions per timed run.
const MOTIONS: u32 = 20_000;

/// Motions per timed batch of the resize drag, between which the client
/// acks what it was sent (a real client answers each configure; one that
/// never did would fill its socket).
const RESIZE_BATCH: u32 = 250;

/// `KEY_LEFTMETA` as an xkb keycode: Super, the default floating modifier.
const SUPER: u32 = 125 + 8;

/// Three windows, the last floated at the size it drew; answers the
/// floating window's rect.
fn floating_scene() -> (RoundedFixture, Rect) {
    let mut fixture = rounded_scene(3, 1);
    fixture.state.act(Action::ToggleFloating);
    fixture.settle();
    let rect = fixture
        .state
        .world
        .arrange()
        .placements
        .iter()
        .find(|p| p.floating && p.visible)
        .map(|p| p.rect)
        .expect("the scene has a visible floating window");
    (fixture, rect)
}

/// Two points a pixel apart around `rect`'s middle.
fn wiggle(rect: Rect) -> [(f64, f64); 2] {
    let (x, y) = (
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    );
    [(x, y), (x + 1.0, y + 1.0)]
}

fn time_motions(fixture: &mut RoundedFixture, points: [(f64, f64); 2], motions: u32) -> Duration {
    let started = Instant::now();
    for i in 0..motions {
        let (x, y) = points[(i % 2) as usize];
        fixture.state.pointer_move(x, y);
    }
    started.elapsed()
}

fn report(label: &str, runs: &mut [Duration], motions: u32) {
    runs.sort();
    let per = |d: Duration| d / motions;
    println!(
        "pointer motion, {label}: median {:?} per motion (min {:?}, max {:?}; {} runs x {motions})",
        per(runs[runs.len() / 2]),
        per(runs[0]),
        per(runs[runs.len() - 1]),
        runs.len(),
    );
}

/// Motion over a floating window with nothing grabbed: what every motion
/// costs. Runs in trees before the pointer drag existed, as the baseline.
#[test]
#[ignore = "prints per-motion timings for a human; asserts nothing"]
fn pointer_motion_cost() {
    let (mut fixture, rect) = floating_scene();
    let points = wiggle(rect);
    time_motions(&mut fixture, points, MOTIONS / 10);
    let mut runs: Vec<Duration> = (0..RUNS)
        .map(|_| time_motions(&mut fixture, points, MOTIONS))
        .collect();
    report("no drag, over a floating window", &mut runs, MOTIONS);
}

/// Motion while the floating window is dragged: every motion moves it (a
/// move drag), or asks it for a new size (a resize drag, whose configure is
/// the one allocation, Smithay's, per new size).
#[test]
#[ignore = "prints per-motion timings for a human; asserts nothing"]
fn pointer_motion_drag_cost() {
    for (label, button) in [
        ("move drag", PointerButton::Left),
        ("resize drag", PointerButton::Right),
    ] {
        let (mut fixture, rect) = floating_scene();
        let points = wiggle(rect);
        let (x, y) = points[0];
        fixture.state.pointer_move(x, y);
        fixture.state.key(Keycode::new(SUPER), KeyState::Pressed);
        fixture.state.pointer_button(button, true);
        assert!(
            fixture.state.floating_grab_window().is_some(),
            "the {label} did not begin"
        );
        let mut runs = Vec::new();
        for _ in 0..RUNS {
            let mut total = Duration::ZERO;
            let mut done = 0;
            while done < MOTIONS {
                total += time_motions(&mut fixture, points, RESIZE_BATCH);
                done += RESIZE_BATCH;
                if button == PointerButton::Right {
                    // Untimed: the client reads and acks its configures.
                    fixture.run(RoundedStep::Attach {
                        index: 2,
                        w: rect.w,
                        h: rect.h,
                        color: [0xFF, 0x00, 0x00, 0xFF],
                    });
                }
            }
            assert!(
                fixture.state.floating_grab_window().is_some(),
                "the {label} ended mid-run"
            );
            runs.push(total);
        }
        report(label, &mut runs, MOTIONS);
        fixture.state.pointer_button(button, false);
        fixture.state.key(Keycode::new(SUPER), KeyState::Released);
    }
}

/// An allocation probe for `valgrind --tool=dhat`, not a timing: the scene,
/// then `SCOOT_PROBE_MOTIONS` motions (1000 by default) with
/// `SCOOT_PROBE_DRAG` = `move`, `resize` or `none` (the default). Two runs
/// at different motion counts, diffed, give what one motion allocates: the
/// setup is the same in both. The resize probe acks nothing (its client is
/// not driven), which only matters to Smithay's per-configure bookkeeping,
/// the allocation it exists to count.
#[test]
#[ignore = "an allocation probe for dhat; asserts nothing"]
fn pointer_motion_allocations() {
    let motions: u32 = std::env::var("SCOOT_PROBE_MOTIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);
    let button = match std::env::var("SCOOT_PROBE_DRAG").as_deref() {
        Ok("move") => Some(PointerButton::Left),
        Ok("resize") => Some(PointerButton::Right),
        _ => None,
    };
    let (mut fixture, rect) = floating_scene();
    let points = wiggle(rect);
    let (x, y) = points[0];
    fixture.state.pointer_move(x, y);
    if let Some(button) = button {
        fixture.state.key(Keycode::new(SUPER), KeyState::Pressed);
        fixture.state.pointer_button(button, true);
        assert!(fixture.state.floating_grab_window().is_some());
    }
    time_motions(&mut fixture, points, motions);
    if let Some(button) = button {
        assert!(fixture.state.floating_grab_window().is_some());
        fixture.state.pointer_button(button, false);
        fixture.state.key(Keycode::new(SUPER), KeyState::Released);
    }
}
