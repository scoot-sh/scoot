//! What the X half of the hot paths costs, against the Wayland half on the
//! same session: three windows of one kind or the other, then a pointer
//! motion over one, a full `apply()`, a frame, and a key event to the
//! focused one. Printed for a human; asserts nothing -- run by hand in an
//! `xwayland` build with `Xwayland` on `PATH`:
//!
//! ```text
//! cargo test -p scoot --features xwayland --bin scoot x11_hot_path_cost -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The per-X-window work these add is the state lock and reference-count
//! bump each `X11Surface` accessor takes (`id_of` on the commit path, the
//! hit test, the configure diff in `apply()`, the surface lookup in the
//! frame), and -- for a key -- Smithay's X keyboard target, which forwards
//! to the associated surface. Two rows are not like-for-like, and read as
//! upper bounds on the X overhead: XWayland binds a pointer and a keyboard,
//! so motion over an X window and a key to one are written to its socket,
//! while the Wayland peer binds neither and costs no write.

use std::time::{Duration, Instant};

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

use super::live::{Live, RED, live};
use super::x11::Props;

const MOTIONS: u32 = 20_000;
const APPLIES: u32 = 2_000;
const FRAMES: u32 = 300;
const KEYS: u32 = 20_000;
const RUNS: usize = 5;
const KEY_A: u32 = 30 + 8;

fn median(mut runs: Vec<Duration>, per: u32) -> (Duration, Duration, Duration) {
    runs.sort();
    (
        runs[0] / per,
        runs[runs.len() / 2] / per,
        runs[runs.len() - 1] / per,
    )
}

fn time(live: &mut Live, what: &str, per: u32, mut body: impl FnMut(&mut Live)) -> String {
    body(live);
    let runs = (0..RUNS)
        .map(|_| {
            let started = Instant::now();
            body(live);
            started.elapsed()
        })
        .collect();
    let (min, med, max) = median(runs, per);
    format!("{what}: median {med:?} (min {min:?}, max {max:?}; {RUNS} runs x {per})")
}

fn measure(live: &mut Live, label: &str) {
    let focused = live.fixture.state.focus.expect("a focused window");
    let rect = live.placement(focused).rect;
    let (x, y) = (
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    );
    let motion = time(live, "pointer motion", MOTIONS, |live| {
        for i in 0..MOTIONS {
            let offset = f64::from(i % 2);
            live.fixture.state.pointer_move(x + offset, y + offset);
        }
    });
    let apply = time(live, "apply()", APPLIES, |live| {
        for _ in 0..APPLIES {
            live.fixture.state.apply();
        }
    });
    let frame = time(live, "frame", FRAMES, |live| {
        for _ in 0..FRAMES {
            live.fixture.state.request_render();
            live.fixture.state.render();
        }
    });
    let key = time(live, "key event", KEYS, |live| {
        for i in 0..KEYS {
            let state = if i % 2 == 0 {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            live.fixture.state.key(Keycode::new(KEY_A), state);
            // Now and then, so XWayland's replies do not queue up
            // unread; rare enough to stay out of the per-key figure.
            if i % 5_000 == 0 {
                live.fixture.settle();
            }
        }
    });
    println!("{label}:\n  {motion}\n  {apply}\n  {frame}\n  {key}");
}

#[test]
#[ignore = "prints per-event timings for a human; asserts nothing"]
fn x11_hot_path_cost() {
    let Some(mut wayland) = live("x11_hot_path_cost (wayland)") else {
        return;
    };
    for title in ["one", "two", "three"] {
        wayland.map_peer(title);
    }
    measure(&mut wayland, "three Wayland windows");
    drop(wayland);

    let Some(mut x) = live("x11_hot_path_cost (x11)") else {
        return;
    };
    for _ in 0..3 {
        let xid = x.x.map(&Props::new(RED));
        x.managed(xid);
    }
    measure(&mut x, "three X windows");
}
