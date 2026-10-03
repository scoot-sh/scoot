//! The resource ratchet's rows for tooltips, as a measurement and not a check
//! (`#[ignore]`: it asserts nothing about speed, it prints `BENCH key=value`
//! lines). Run it against the build that is being measured, release for the
//! published numbers:
//!
//! ```sh
//! cargo test -p scootbar --release --test tooltip_bench -- --ignored --nocapture idle
//! cargo test -p scootbar --release --test tooltip_bench -- --ignored --nocapture cycles
//! ```
//!
//! Under `cargo test`, not nextest: `idle` takes minutes and nextest ends a
//! test at its 120 s terminate-after (`.config/nextest.toml`).
//!
//! `idle` is the row every milestone repeats, for three bars (a clock alone,
//! which has no tooltip and must cost what it did; a `push` module with a
//! tooltip and the pointer resting off the bar; the same with the pointer
//! resting on the module with its tooltip shown): RSS, peak RSS, descriptors,
//! shm mappings and idle wakeups per minute. `cycles` is a tooltip's own cost:
//! the bar's CPU over many shows and hides, its memory while shown, and what a
//! cycle leaves behind. Neither is a test of anything but that the bar
//! survives them.

mod common;

use std::fs;
use std::time::Duration;

use common::{Reaper, Session, open_fds, rgb, shm_mappings, wakeups};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const HEIGHT: u32 = 40;
const DELAY_MS: u64 = 100;

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

/// Nanoseconds the process has spent running on a CPU (`/proc/PID/schedstat`).
fn cpu_ns(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/schedstat"))
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn threads(pid: u32) -> usize {
    fs::read_dir(format!("/proc/{pid}/task")).unwrap().count()
}

/// A bar with `module` on the right (a `push` one has a tooltip, the `clock`
/// has none), and where its middle is. The tooltips are only in a build with
/// the `popup` feature: for a build without it, `SCOOTBAR_BENCH_NO_TOOLTIP`
/// leaves the key out of the config, which such a build would refuse.
fn rig(tag: &str, module: &str) -> Option<(Session, Reaper, u32)> {
    let session = Session::scoot(tag, 1, "")?;
    let file = session.runtime_dir().join("bar.toml");
    let delay = if std::env::var_os("SCOOTBAR_BENCH_NO_TOOLTIP").is_some() {
        String::new()
    } else {
        format!("tooltip-delay = {DELAY_MS}\n")
    };
    let (lists, push) = if module == "tip" {
        ("right = [\"tip\"]\n", "[push.tip]\nplaceholder = \"TIP\"\n")
    } else {
        ("", "")
    };
    fs::write(
        &file,
        format!(
            "{lists}[bar]\nheight = {HEIGHT}\nfont-size = 20\n{delay}\
             [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\n{push}"
        ),
    )
    .unwrap();
    let mut args = vec!["--config", file.to_str().unwrap()];
    if module == "clock" {
        args.extend(["--center=", "--right", "clock"]);
    }
    let mut bar = Reaper(session.bar_with_env(&args, &[]));
    session.wait_for(&mut bar.0, "the module drawn", |session| {
        let shot = session.scoot_screenshot(1);
        (1000..1600)
            .any(|x| shot.at(x, HEIGHT / 2) == rgb(FG))
            .then_some(())
    });
    let shot = session.scoot_screenshot(1);
    let xs: Vec<u32> = (1000..1600)
        .filter(|&x| shot.at(x, HEIGHT / 2) == rgb(FG))
        .collect();
    let x = (xs[0] + xs[xs.len() - 1]) / 2;
    Some((session, bar, x))
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

fn ipc(session: &Session, request: &str) {
    assert_eq!(session.scoot_ipc(request)["type"], "ok", "{request}");
}

fn move_to(session: &Session, x: u32, y: u32) {
    ipc(
        session,
        &format!(r#"{{"type":"pointer_move","x":{x},"y":{y}}}"#),
    );
}

fn report(label: &str, pid: u32, minute: Duration) {
    let start = settle(pid);
    std::thread::sleep(minute);
    let woke = wakeups(pid) - start;
    println!("BENCH {label}_rss_kb={}", status_kb(pid, "VmRSS:"));
    println!("BENCH {label}_peak_rss_kb={}", status_kb(pid, "VmHWM:"));
    println!("BENCH {label}_threads={}", threads(pid));
    println!("BENCH {label}_fds={}", open_fds(pid));
    println!("BENCH {label}_shm_mappings={}", shm_mappings(pid));
    println!("BENCH {label}_wakeups_per_{}s={woke}", minute.as_secs());
}

#[test]
#[ignore = "a measurement: run it with --run-ignored only"]
fn idle() {
    let window = Duration::from_secs(60);
    // A clock alone: no tooltip, so nothing of this feature is in play.
    let Some((session, bar, x)) = rig("tbench-idle-clock", "clock") else {
        return;
    };
    let pid = bar.0.id();
    report("clock_only", pid, window);
    // The pointer crossing and resting on a module that has none: no timer.
    move_to(&session, x, 20);
    report("clock_only_pointer_resting", pid, window);
    drop((session, bar));

    // A push module with a tooltip, the pointer away.
    let Some((session, bar, x)) = rig("tbench-idle-tip", "tip") else {
        return;
    };
    let pid = bar.0.id();
    let set = session
        .scootbar()
        .arg("msg")
        .args([
            "set",
            "tip",
            r#"{"text":"TIP","tooltip":"the tooltip text"}"#,
        ])
        .output()
        .unwrap();
    assert!(set.status.success());
    report("tip_pointer_away", pid, window);
    // The pointer resting on it, the tooltip shown.
    move_to(&session, x, 20);
    let mut bar = bar;
    session.wait_for(&mut bar.0, "the tooltip shown", |session| {
        (session.scoot_screenshot(1).at(x, HEIGHT + 2) == rgb(BAR)).then_some(())
    });
    report("tip_tooltip_shown", pid, window);
}

#[test]
#[ignore = "a measurement: run it with --run-ignored only"]
fn cycles() {
    let Some((session, mut bar, x)) = rig("tbench-cycles", "tip") else {
        return;
    };
    let pid = bar.0.id();
    let set = session
        .scootbar()
        .arg("msg")
        .args([
            "set",
            "tip",
            r#"{"text":"TIP","tooltip":"the tooltip text"}"#,
        ])
        .output()
        .unwrap();
    assert!(set.status.success());
    let shown = |session: &Session| session.scoot_screenshot(1).at(x, HEIGHT + 2) == rgb(BAR);
    settle(pid);
    let (fds, maps, rss) = (open_fds(pid), shm_mappings(pid), status_kb(pid, "VmRSS:"));
    move_to(&session, x, 20);
    session.wait_for(&mut bar.0, "the tooltip shown", |s| shown(s).then_some(()));
    println!("BENCH shown_rss_kb={}", status_kb(pid, "VmRSS:"));
    println!("BENCH shown_fds={}", open_fds(pid));
    println!("BENCH shown_shm_mappings={}", shm_mappings(pid));
    move_to(&session, 200, 600);
    session.wait_for(&mut bar.0, "hidden", |s| (!shown(s)).then_some(()));
    std::thread::sleep(Duration::from_millis(200));
    let cycles = 100u64;
    let (ran, woke) = (cpu_ns(pid), wakeups(pid));
    let hwm = status_kb(pid, "VmHWM:");
    // Every cycle waits for the tooltip on screen and then gone, so what is
    // timed is a tooltip that mapped, was drawn and was destroyed. The
    // screenshots that look cost scoot, not the process timed.
    for _ in 0..cycles {
        move_to(&session, x, 20);
        session.wait_for(&mut bar.0, "shown", |s| shown(s).then_some(()));
        move_to(&session, 200, 600);
        session.wait_for(&mut bar.0, "hidden", |s| (!shown(s)).then_some(()));
    }
    std::thread::sleep(Duration::from_millis(300));
    let spent = cpu_ns(pid) - ran;
    println!("BENCH cycles={cycles}");
    println!("BENCH cycle_cpu_ns_total={spent}");
    println!(
        "BENCH cycle_cpu_us_per_show_hide={:.1}",
        spent as f64 / 1000.0 / cycles as f64
    );
    println!(
        "BENCH cycle_wakeups_per_show_hide={:.2}",
        (wakeups(pid) - woke) as f64 / cycles as f64
    );
    println!(
        "BENCH after_rss_kb={} (before {rss})",
        status_kb(pid, "VmRSS:")
    );
    println!(
        "BENCH after_peak_rss_kb={} (before {hwm})",
        status_kb(pid, "VmHWM:")
    );
    println!("BENCH after_fds={} (before {fds})", open_fds(pid));
    println!(
        "BENCH after_shm_mappings={} (before {maps})",
        shm_mappings(pid)
    );
}
