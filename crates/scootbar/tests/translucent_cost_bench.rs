//! The translucent-bar compositor-cost live run: a floating window moving
//! under the bar on a headless scoot (the CPU tier) at the five looks'
//! example opacities against opaque, as a measurement and not a check
//! (`#[ignore]`): it asserts nothing about speed, it prints `BENCH
//! key=value` lines. Run it against the build that is being measured,
//! release for the published numbers:
//!
//! ```sh
//! taskset -c 2 cargo test -p scootbar --release --test translucent_cost_bench -- --ignored --nocapture underlay
//! ```
//!
//! Under `cargo test`, not nextest: each opacity is a minute-scale session
//! and nextest ends a test at its 120 s terminate-after
//! (`.config/nextest.toml`).
//!
//! What this covers of the ticket (`bar-translucent-compositor-cost`): the
//! headless `scoot --renderer cpu` run with a floating window moving under
//! the bar, comparing frame time with the bar at alpha 1.0 and at each
//! example opacity (0.7, 0.78, 0.82, 0.88). The compositor has no frame
//! instrumentation over IPC, so this uses a controlled screenshot cadence:
//! one `screenshot` round trip per animation step, timed, with the window
//! moved via `scoot msg` actions between steps. The blend micro-bench (the
//! bar strip's own `OVER` vs `SRC` at 1x and 2x, through the real
//! `PixmanRenderer::render_texture_from_to` path) lives in-crate in scoot
//! (`compositor/headless/bench.rs` `translucent_strip_blend_cost`): no
//! in-process timer here can see the strip's blend, and no screenshot timer
//! there can see occlusion culling.
//!
//! The bar runs with `exclusive = false` (an overlay-style bar): the
//! shipped default (`exclusive = true`) keeps every tiled window out of the
//! bar's rectangle and a static wallpaper adds no per-frame cost, so there
//! is nothing to animate under it. With no zone the floating window can sit
//! with its top rows behind the bar's rectangle, which is exactly the
//! non-default configuration the ticket asks to put a number on (a floating
//! window dragged under the bar, an overlay over it). The opaque bar still
//! sets its whole-strip opaque region there while a translucent one sets
//! none (`daemon/canvas.rs` `set_opaque_region`), so the damage tracker's
//! occlusion culling is in play during the animation, not just the blend.

mod common;

use std::fs;
use std::time::{Duration, Instant};

use common::{Reaper, Session, foot_missing};

const BAR: &str = "#102030";
const HEIGHT: u32 = 40;

/// The example opacities the five shipped looks carry, against opaque.
const ALPHAS: &[f32] = &[1.0, 0.70, 0.78, 0.82, 0.88];

fn key(alpha: f32) -> String {
    format!("{alpha:.2}")
}

fn median_and_p5(samples: &mut [Duration]) -> (Duration, Duration) {
    samples.sort_unstable();
    (samples[samples.len() / 2], samples[samples.len() / 20])
}

/// A bar with `opacity` in `[bar]` and no exclusive zone, waiting until it
/// is drawn. `scoot_config` is the compositor's config (output scales live
/// there).
fn rig(tag: &str, alpha: f32, scoot_config: &str) -> Option<(Session, Reaper)> {
    let session = Session::scoot(&common::scootbar_bin(), tag, 1, scoot_config)?;
    let file = session.runtime_dir().join("bar.toml");
    fs::write(
        &file,
        format!(
            "[bar]\nheight = {HEIGHT}\nfont-size = 20\nopacity = {alpha}\nexclusive = false\n\
             tooltip-delay = 0\n[colors]\nbackground = \"{BAR}\"\n"
        ),
    )
    .unwrap();
    let mut bar = Reaper(session.bar_with_env(
        &[
            "--config",
            file.to_str().unwrap(),
            "--center=",
            "--right",
            "clock",
        ],
        &[],
    ));
    session.wait_for(&mut bar.0, "the bar drawn", |session| {
        let shot = session.scoot_screenshot(1);
        (0..1600)
            .any(|x| shot.at(x, HEIGHT / 2) != shot.at(x, 500))
            .then_some(())
    });
    Some((session, bar))
}

fn windows(session: &Session) -> serde_json::Value {
    session.scoot_ipc(r#"{"type":"windows"}"#)
}

/// The first window's id and rect, or `None` while no window is mapped.
fn first_window(session: &Session) -> Option<(u64, i64, i64, i64, i64)> {
    let reply = windows(session);
    let w = reply["windows"].as_array()?.first()?;
    Some((
        w["id"].as_u64()?,
        w["rect"]["x"].as_i64()?,
        w["rect"]["y"].as_i64()?,
        w["rect"]["width"].as_i64()?,
        w["rect"]["height"].as_i64()?,
    ))
}

#[test]
#[ignore = "a measurement: run it with --ignored only"]
fn underlay() {
    if foot_missing() {
        println!("BENCH skipped_no_foot=1");
        return;
    }
    let scales = [
        ("", "1x"),
        ("[[outputs]]\nname = \"headless-1\"\nscale = 2\n", "2x"),
    ];
    // Steps of the window's top edge from above the bar's rect to below
    // it: the middle steps composite moving client pixels behind the
    // translucent bar's rectangle, the first ones only approach it.
    const STEPS: usize = 24;
    for run in 0..2 {
        let mut order = ALPHAS.to_vec();
        let mid = run % order.len();
        order.rotate_left(mid);
        for &(scoot_config, stag) in &scales {
            for &alpha in &order {
                let tag = format!("tobench-underlay-{run}-{stag}-{alpha:.2}");
                let Some((session, mut bar)) = rig(&tag, alpha, scoot_config) else {
                    return;
                };
                let label = format!("underlay_run{run}_{stag}_opacity{}", key(alpha));
                // A client under the bar: spawned tiled, then floated.
                let reply =
                    session.scoot_ipc(r#"{"type":"action","action":"spawn","command":["foot"]}"#);
                assert_eq!(reply["type"], "ok", "{reply}");
                let (id, _, _, _, _) =
                    session.wait_for(&mut bar.0, "a window mapped", first_window);
                let reply = session.scoot_ipc(&format!(
                    r#"{{"type":"action","action":"set_floating","id":{id},"floating":true}}"#
                ));
                assert_eq!(reply["type"], "ok", "{reply}");
                // Let the float settle before stepping.
                std::thread::sleep(Duration::from_millis(500));
                // Warm screenshots first: the first ones map buffers.
                for _ in 0..5 {
                    session.scoot_screenshot(1);
                }
                let mut samples = Vec::with_capacity(STEPS);
                for step in 0..STEPS {
                    // Slide the window's top edge from y=-60 (above the
                    // bar) through the bar's 40 rows to y=+20 past it.
                    let y = -60 + step as i32 * 4;
                    let reply = session.scoot_ipc(&format!(
                        r#"{{"type":"action","action":"move_floating","id":{id},"x":400,"y":{y}}}"#
                    ));
                    assert_eq!(reply["type"], "ok", "{reply}");
                    let start = Instant::now();
                    let _shot = session.scoot_screenshot(1);
                    samples.push(start.elapsed());
                }
                let (median, p5) = median_and_p5(&mut samples);
                println!(
                    "BENCH {label}_steps={STEPS} shot_ms_median={:.2} p5={:.2}",
                    median.as_secs_f64() * 1e3,
                    p5.as_secs_f64() * 1e3,
                );
                // The compositor-side truth that the animation really put
                // pixels behind the bar: the floating window's placed rect
                // after the last step (top edge at y=32, inside the bar's
                // 40 rows). A clamp that kept it out would fail here
                // instead of silently measuring the wrong scene.
                let (_, _, ry, _, rh) = first_window(&session).expect("the window still mapped");
                println!("BENCH {label}_window_y={ry} h={rh}");
                assert!(
                    ry < HEIGHT as i64,
                    "the window never reached under the bar (y={ry})"
                );
                let reply = session.scoot_ipc(r#"{"type":"action","action":"close_focused"}"#);
                assert_eq!(reply["type"], "ok", "{reply}");
                drop((session, bar));
            }
        }
    }
}
