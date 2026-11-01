//! The opacity gate's live bench: the bar on a headless scoot (the CPU
//! tier) at the five looks' example opacities against opaque, as a
//! measurement and not a check (`#[ignore]`): it asserts nothing about
//! speed, it prints `BENCH key=value` lines. Run it against the build
//! that is being measured, release for the published numbers:
//!
//! ```sh
//! taskset -c 2 cargo test -p scootbar --release --test opacity_bench -- --ignored --nocapture idle
//! taskset -c 2 cargo test -p scootbar --release --test opacity_bench -- --ignored --nocapture updates
//! taskset -c 2 cargo test -p scootbar --release --test opacity_bench -- --ignored --nocapture popup
//! taskset -c 2 cargo test -p scootbar --release --test opacity_bench -- --ignored --nocapture frames
//! ```
//!
//! Under `cargo test`, not nextest: `idle` takes minutes per opacity and
//! nextest ends a test at its 120 s terminate-after
//! (`.config/nextest.toml`). What each covers of the ticket's five
//! scenarios (the in-process repaint numbers are `render/tests/bench.rs`;
//! no in-process timer can see wakeups, popups or the compositor):
//!
//! - `idle`: a clock bar as shipped, per opacity, 60 s each: wakeups (the
//!   no-extra-wakeups budget) and the bar's CPU over the window (the
//!   per-minute tick's full cost: a 60 s window holds exactly one
//!   minute-aligned tick).
//! - `updates`: one `push` text change after another, the bar's CPU per
//!   update: the damage-limited span repaint behind the compositor.
//! - `popup`: the volume slider opened and closed, the bar's CPU per
//!   open/close: the bar with a popup. Needs the `popup` and `volume`
//!   features; without them it reports `skipped_missing_features`.
//! - `frames`: `screenshot` round trips at 1x and 2x, opaque against
//!   translucent: the compositor's frame cost behind the bar, plus PPM
//!   screenshots of both for the record.
//!
//! Why `scootbar` is benched through its binary and not an `examples/`
//! target: it is a binary crate, which an example cannot import; the
//! `#[ignore]`d integration test driving the real binary is the
//! `popup_bench`/`tooltip_bench` pattern, and the real in-process paint
//! path is timed in-crate by `render/tests/bench.rs` instead.

mod common;
mod pulse;

use std::fs;
use std::time::{Duration, Instant};

use common::{Reaper, Session, open_fds, rgb, shm_mappings, wakeups};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const HEIGHT: u32 = 40;

/// The example opacities the five pins wait on, as written in the looks'
/// comments, against opaque.
const ALPHAS: &[f32] = &[1.0, 0.70, 0.78, 0.82, 0.88];

fn key(alpha: f32) -> String {
    format!("{alpha:.2}")
}

fn status_kb(pid: u32, key: &str) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .trim()
                .strip_suffix("kB")?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// Nanoseconds the process has spent running on a CPU (`/proc/PID/schedstat`,
/// the first field), exact where utime and stime are 10 ms ticks.
fn cpu_ns(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/schedstat"))
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn settle(pid: u32) -> u64 {
    let mut last = wakeups(pid);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = wakeups(pid);
        if now == last {
            return now;
        }
        last = now;
    }
}

/// A bar with `opacity` in `[bar]`, `module` on it, waiting until the
/// module is drawn. `scoot_config` is the compositor's config (output
/// scales live there).
fn rig(tag: &str, alpha: f32, module: &str, scoot_config: &str) -> Option<(Session, Reaper)> {
    let session = Session::scoot(&common::scootbar_bin(), tag, 1, scoot_config)?;
    let file = session.runtime_dir().join("bar.toml");
    let module_config = match module {
        "clock" => String::new(),
        "tick" => "[push.tick]\nplaceholder = \"TICK\"\n".to_owned(),
        "volume" => "[volume]\non-click = \"popup\"\n".to_owned(),
        other => panic!("unknown bench module {other}"),
    };
    let lists = match module {
        "clock" => String::new(),
        "tick" => "right = [\"tick\"]\n".to_owned(),
        "volume" => String::new(),
        other => panic!("unknown bench module {other}"),
    };
    fs::write(
        &file,
        format!(
            "{lists}[bar]\nheight = {HEIGHT}\nfont-size = 20\nopacity = {alpha}\n\
             tooltip-delay = 0\n[colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\n{module_config}"
        ),
    )
    .unwrap();
    let mut args = vec!["--config", file.to_str().unwrap()];
    match module {
        "clock" => args.extend(["--center=", "--right", "clock"]),
        "tick" => {}
        "volume" => args.extend(["--center", "volume"]),
        other => panic!("unknown bench module {other}"),
    };
    let mut bar = Reaper(session.bar_with_env(&args, &[]));
    session.wait_for(&mut bar.0, "the module drawn", |session| {
        let shot = session.scoot_screenshot(1);
        (0..1600)
            .any(|x| shot.at(x, HEIGHT / 2) != rgb(BAR))
            .then_some(())
    });
    Some((session, bar))
}

/// The bar exited before drawing (a build without the module or key this
/// rig needs): report the skip instead of failing the wait.
fn exited(bar: &mut Reaper) -> bool {
    std::thread::sleep(Duration::from_secs(2));
    bar.0.try_wait().unwrap().is_some()
}

fn median_and_p5(samples: &mut [Duration]) -> (Duration, Duration) {
    samples.sort_unstable();
    (samples[samples.len() / 2], samples[samples.len() / 20])
}

/// A `Shot` as a binary PPM, for the record.
fn save_ppm(session: &Session, id: u64, path: &std::path::Path) {
    let shot = session.scoot_screenshot(id);
    let mut out = Vec::with_capacity(shot.width as usize * shot.height as usize * 3 + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", shot.width, shot.height).as_bytes());
    for y in 0..shot.height {
        for x in 0..shot.width {
            out.extend_from_slice(&shot.at(x, y));
        }
    }
    fs::write(path, out).unwrap();
}

#[test]
#[ignore = "a measurement: run it with --ignored only"]
fn idle() {
    let window = Duration::from_secs(60);
    // Rotate the order per run so thermal and neighbor noise cannot favor
    // whichever opacity ran first.
    for run in 0..3 {
        let mut order = ALPHAS.to_vec();
        let mid = run % order.len();
        order.rotate_left(mid);
        for &alpha in &order {
            let tag = format!("obench-idle-{run}-{alpha:.2}");
            let Some((session, bar)) = rig(&tag, alpha, "clock", "") else {
                return;
            };
            let pid = bar.0.id();
            let label = format!("idle_run{run}_opacity{}", key(alpha));
            println!(
                "BENCH {label}_rss_kb={} fds={} shm={}",
                status_kb(pid, "VmRSS:"),
                open_fds(pid),
                shm_mappings(pid),
            );
            let start_wakeups = settle(pid);
            let start_cpu = cpu_ns(pid);
            std::thread::sleep(window);
            println!(
                "BENCH {label}_wakeups_per_60s={}",
                wakeups(pid) - start_wakeups
            );
            println!(
                "BENCH {label}_cpu_us_per_60s={:.0}",
                (cpu_ns(pid) - start_cpu) as f64 / 1000.0
            );
            drop((session, bar));
        }
    }
}

#[test]
#[ignore = "a measurement: run it with --ignored only"]
fn updates() {
    for run in 0..3 {
        let mut order = ALPHAS.to_vec();
        let mid = run % order.len();
        order.rotate_left(mid);
        for &alpha in &order {
            let tag = format!("obench-updates-{run}-{alpha:.2}");
            let Some((session, mut bar)) = rig(&tag, alpha, "tick", "") else {
                return;
            };
            let pid = bar.0.id();
            let label = format!("updates_run{run}_opacity{}", key(alpha));
            let set = |n: u32| {
                let out = session
                    .scootbar()
                    .arg("msg")
                    .args(["set", "tick", &format!(r#"{{"text":"TICK {n:04}"}}"#)])
                    .output()
                    .unwrap();
                assert!(out.status.success());
            };
            set(0);
            session.wait_for(&mut bar.0, "the first text drawn", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600)
                    .any(|x| shot.at(x, HEIGHT / 2) == rgb(FG))
                    .then_some(())
            });
            settle(pid);
            let cycles = 100u64;
            let (ran, woke) = (cpu_ns(pid), wakeups(pid));
            for n in 1..=cycles {
                set(n as u32);
            }
            session.wait_for(&mut bar.0, "the last text drawn", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600)
                    .any(|x| shot.at(x, HEIGHT / 2) == rgb(FG))
                    .then_some(())
            });
            let spent = cpu_ns(pid) - ran;
            println!("BENCH {label}_cycles={cycles}");
            println!(
                "BENCH {label}_cpu_us_per_update={:.1}",
                spent as f64 / 1000.0 / cycles as f64
            );
            println!(
                "BENCH {label}_wakeups_per_update={:.2}",
                (wakeups(pid) - woke) as f64 / cycles as f64
            );
            drop((session, bar));
        }
    }
}

#[test]
#[ignore = "a measurement: run it with --ignored only"]
fn popup() {
    for run in 0..3 {
        let mut order = ALPHAS.to_vec();
        let mid = run % order.len();
        order.rotate_left(mid);
        for &alpha in &order {
            let tag = format!("obench-popup-{run}-{alpha:.2}");
            let session = Session::scoot(&common::scootbar_bin(), &tag, 1, "");
            let Some(session) = session else {
                return;
            };
            let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
            let file = session.runtime_dir().join("bar.toml");
            fs::write(
                &file,
                format!(
                    "[bar]\nheight = {HEIGHT}\nfont-size = 20\nopacity = {alpha}\n\
                     tooltip-delay = 0\n[colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\n\
                     [volume]\non-click = \"popup\"\n"
                ),
            )
            .unwrap();
            let address = server.address();
            let mut bar = Reaper(session.bar_with_env(
                &["--config", file.to_str().unwrap(), "--center", "volume"],
                &[("PULSE_SERVER", address.as_str())],
            ));
            if exited(&mut bar) {
                println!("BENCH skipped_missing_features=1 (volume/popup)");
                return;
            }
            session.wait_for(&mut bar.0, "the module drawn", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600)
                    .any(|x| shot.at(x, HEIGHT / 2) != rgb(BAR))
                    .then_some(())
            });
            let pid = bar.0.id();
            let label = format!("popup_run{run}_opacity{}", key(alpha));
            let click = |session: &Session| {
                let reply = session.scoot_ipc(r#"{"type":"click","x":800,"y":20,"button":"left"}"#);
                assert_eq!(reply["type"], "ok");
            };
            let shown =
                |session: &Session| session.scoot_screenshot(1).at(800, HEIGHT + 2) == rgb(BAR);
            settle(pid);
            click(&session);
            session.wait_for(&mut bar.0, "the popup open", |s| shown(s).then_some(()));
            click(&session);
            std::thread::sleep(Duration::from_millis(200));
            let cycles = 50u64;
            let (ran, woke) = (cpu_ns(pid), wakeups(pid));
            for _ in 0..cycles {
                click(&session);
                session.wait_for(&mut bar.0, "open", |s| shown(s).then_some(()));
                click(&session);
                session.wait_for(&mut bar.0, "closed", |s| (!shown(s)).then_some(()));
            }
            std::thread::sleep(Duration::from_millis(300));
            let spent = cpu_ns(pid) - ran;
            println!("BENCH {label}_cycles={cycles}");
            println!(
                "BENCH {label}_cpu_us_per_open_close={:.1}",
                spent as f64 / 1000.0 / cycles as f64
            );
            println!(
                "BENCH {label}_wakeups_per_open_close={:.2}",
                (wakeups(pid) - woke) as f64 / cycles as f64
            );
            drop((session, bar));
        }
    }
}

#[test]
#[ignore = "a measurement: run it with --ignored only"]
fn frames() {
    let scales = [
        ("", "1x"),
        ("[[outputs]]\nname = \"headless-1\"\nscale = 2\n", "2x"),
    ];
    // Any translucent alpha takes the compositor's blend path; opaque can
    // skip it. 0.70 stands for the translucent side, with 0.88 as the
    // spot check that intermediate alphas cost the same path.
    let alphas = [1.0f32, 0.70, 0.88];
    for run in 0..3 {
        let mut order = alphas.to_vec();
        let mid = run % order.len();
        order.rotate_left(mid);
        for &(scoot_config, stag) in &scales {
            for &alpha in &order {
                let tag = format!("obench-frames-{run}-{stag}-{alpha:.2}");
                let Some((session, bar)) = rig(&tag, alpha, "tick", scoot_config) else {
                    return;
                };
                let pid = bar.0.id();
                let label = format!("frames_run{run}_{stag}_opacity{}", key(alpha));
                settle(pid);
                // Warm shots first: the first screenshot maps buffers.
                for _ in 0..5 {
                    session.scoot_screenshot(1);
                }
                let iters = 30usize;
                let mut samples = Vec::with_capacity(iters);
                for _ in 0..iters {
                    let start = Instant::now();
                    session.scoot_screenshot(1);
                    samples.push(start.elapsed());
                }
                let (median, p5) = median_and_p5(&mut samples);
                println!(
                    "BENCH {label}_shot_ms_median={:.2} p5={:.2}",
                    median.as_secs_f64() * 1e3,
                    p5.as_secs_f64() * 1e3,
                );
                if run == 0 {
                    let dir = std::path::PathBuf::from(format!(
                        "/dev/shm/bo/shots-{stag}-{:02}.ppm",
                        (alpha * 100.0) as u32
                    ));
                    save_ppm(&session, 1, &dir);
                    println!("BENCH {label}_shot_saved={}", dir.display());
                }
                drop((session, bar));
            }
        }
    }
}
