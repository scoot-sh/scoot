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

/// What drawing X at an integer scale costs at a fractional output scale
/// -- the decision `scale.rs`'s module doc records. One floating X window,
/// 400x300 logical, on a 1200-square output at `[output] scale = 1.5`,
/// with the X side at client scale 1 (the X server drawing at the logical
/// size and the renderer scaling it up: every session before `scale.rs`),
/// 1.5 (the exact scale, drawn one to one) and 2 (`ceil`, what scoot does:
/// twice the logical size, scaled down). Each frame nudges the window one logical pixel
/// in the `Space` and back, so the whole window is re-composited every
/// frame -- the worst case, a window scrolling or animating under the
/// pointer -- with no X traffic in the timed span. Printed with the
/// buffer's own size: the X server's pixmap and the shared-memory buffer
/// each hold that many pixels, four bytes each.
///
/// ```text
/// cargo test --release -p scoot --features xwayland --bin scoot x11_scaled_composite_cost -- --ignored --nocapture --test-threads=1
/// ```
#[test]
#[ignore = "prints per-frame timings for a human; asserts nothing"]
fn x11_scaled_composite_cost() {
    use smithay::backend::renderer::utils::with_renderer_surface_state;
    use smithay::wayland::seat::WaylandFocus;

    use super::live::{Shape, live_shaped};

    const CANVAS: i32 = 1200;
    const SCENE_FRAMES: u32 = 200;
    let mut rows = Vec::new();
    for client_scale in [1.0, 1.5, 2.0] {
        let Some(mut live) = live_shaped(
            "x11_scaled_composite_cost",
            Shape {
                canvas: CANVAS,
                scale: 1.5,
                client_scale: Some(client_scale),
                ..Shape::default()
            },
        ) else {
            return;
        };
        let mut props = Props::new(RED);
        props.dialog = true;
        let xid = live.x.map(&props);
        let id = live.managed(xid);
        let _ = live
            .fixture
            .state
            .world
            .handle_action(scoot_core::Action::ResizeFloating {
                id,
                size: scoot_core::Size::new(400, 300),
                edges: scoot_core::Edges::BOTTOM_RIGHT,
            });
        let _ = live
            .fixture
            .state
            .world
            .handle_action(scoot_core::Action::MoveFloating { id, x: 50, y: 50 });
        live.fixture.state.apply();
        super::x11::eventually(&mut live.fixture, "the window at 400x300", |fixture| {
            fixture
                .state
                .window(id)
                .and_then(smithay::desktop::Window::x11_surface)
                .is_some_and(|x11| {
                    x11.wl_surface().is_some()
                        && x11.last_configure().size == (400, 300).into()
                        && x11.bbox().size == x11.last_configure().size
                })
        });
        live.drain();
        let window = live.fixture.state.windows[&id].clone();
        let logical = window.x11_surface().expect("an X window").bbox().size;
        let buffer = window
            .wl_surface()
            .and_then(|surface| {
                with_renderer_surface_state(&surface, |state| state.buffer_size()).flatten()
            })
            .expect("a committed buffer");
        let frame = time(&mut live, "frame", SCENE_FRAMES, |live| {
            for i in 0..SCENE_FRAMES {
                let x = 50 + i32::try_from(i % 2).expect("0 or 1");
                live.fixture
                    .state
                    .space
                    .map_element(window.clone(), (x, 50), false);
                live.fixture.state.request_render();
                live.fixture.state.render();
            }
        });
        rows.push(format!(
            "X at client scale {client_scale}: {}x{} logical, buffer {}x{} ({} KiB); {frame}",
            logical.w,
            logical.h,
            buffer.w,
            buffer.h,
            buffer.w * buffer.h * 4 / 1024
        ));
    }
    println!("one floating X window at [output] scale = 1.5, re-composited every frame:");
    for row in rows {
        println!("  {row}");
    }
}
