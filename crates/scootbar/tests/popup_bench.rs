//! The resource ratchet's rows for popups, as a measurement and not a check
//! (`#[ignore]`: it asserts nothing about speed, it prints `BENCH key=value`
//! lines). Run it against the build that is being measured, release for the
//! published numbers:
//!
//! ```sh
//! cargo test -p scootbar --release --test popup_bench -- --ignored --nocapture idle
//! cargo test -p scootbar --release --test popup_bench -- --ignored --nocapture cycles
//! ```
//!
//! Both run under `cargo test`, not nextest: nextest ends a test at its 120 s
//! terminate-after (`.config/nextest.toml`), and `cycles` takes minutes with a
//! debug compositor (every step is checked by screenshot).
//!
//! `idle` is the row every milestone repeats: the volume module placed on a
//! headless scoot against a live PulseAudio-protocol server, RSS, peak RSS,
//! descriptors, shm mappings and idle wakeups with **no popup open**, so a
//! build with popups and one without are directly comparable (set
//! `SCOOTBAR_BENCH_NO_POPUP` to leave the popup binding out of the config, as
//! a build without the feature needs). `cycles` is the popup's own cost: the
//! bar's CPU over many open and close cycles, its memory while open, and what
//! a cycle leaves behind.

mod common;
mod pulse;

use std::fs;
use std::time::Duration;

use common::{Reaper, Session, open_fds, rgb, shm_mappings, wakeups};

const BAR: &str = "#102030";
const HEIGHT: u32 = 40;

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
/// the first field), exact where utime and stime are 10 ms ticks. The bar is
/// one thread (`idle_threads` says so), so this is all of it.
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

fn rig(tag: &str) -> Option<(Session, Reaper, pulse::Server)> {
    let session = Session::scoot(tag, 1, "")?;
    let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
    let click = if std::env::var_os("SCOOTBAR_BENCH_NO_POPUP").is_some() {
        ""
    } else {
        "on-click = \"popup\"\n"
    };
    // Tooltips off, so one never stands where a closed popup is looked for
    // (volume has a tooltip, and a pointer rests on it). `SCOOTBAR_BENCH_NO_TOOLTIP`
    // leaves the key out, for a build that predates it.
    let tooltips = if std::env::var_os("SCOOTBAR_BENCH_NO_TOOLTIP").is_some() {
        ""
    } else {
        "tooltip-delay = 0\n"
    };
    let file = session.runtime_dir().join("bar.toml");
    fs::write(
        &file,
        format!(
            "[bar]\nheight = {HEIGHT}\nfont-size = 20\n{tooltips}[colors]\nbackground = \"{BAR}\"\n\
             [volume]\n{click}"
        ),
    )
    .unwrap();
    let address = server.address();
    let mut bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap(), "--center", "volume"],
        &[("PULSE_SERVER", address.as_str())],
    ));
    // The module is on the bar once something other than the background is
    // in its middle.
    session.wait_for(&mut bar.0, "the module drawn", |session| {
        let shot = session.scoot_screenshot(1);
        (0..1600)
            .any(|x| shot.at(x, HEIGHT / 2) != rgb(BAR))
            .then_some(())
    });
    Some((session, bar, server))
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

#[test]
#[ignore = "a measurement: run it with --run-ignored only"]
fn idle() {
    let Some((_session, bar, _server)) = rig("bench-idle") else {
        return;
    };
    let pid = bar.0.id();
    let start = settle(pid);
    std::thread::sleep(Duration::from_secs(60));
    let woke = wakeups(pid) - start;
    println!("BENCH idle_rss_kb={}", status_kb(pid, "VmRSS:"));
    println!("BENCH idle_peak_rss_kb={}", status_kb(pid, "VmHWM:"));
    println!("BENCH idle_threads={}", threads(pid));
    println!("BENCH idle_fds={}", open_fds(pid));
    println!("BENCH idle_shm_mappings={}", shm_mappings(pid));
    println!("BENCH idle_wakeups_per_minute={woke}");
}

#[test]
#[ignore = "a measurement: run it with --run-ignored only"]
fn cycles() {
    let Some((session, mut bar, _server)) = rig("bench-cycles") else {
        return;
    };
    let pid = bar.0.id();
    let click = |session: &Session| {
        let reply = session.scoot_ipc(r#"{"type":"click","x":800,"y":20,"button":"left"}"#);
        assert_eq!(reply["type"], "ok");
    };
    let shown = |session: &Session| session.scoot_screenshot(1).at(800, HEIGHT + 2) == rgb(BAR);
    settle(pid);
    let (fds, maps, rss) = (open_fds(pid), shm_mappings(pid), status_kb(pid, "VmRSS:"));
    // One cycle looked at, so the rest are known to be opening.
    click(&session);
    session.wait_for(&mut bar.0, "the popup open", |s| shown(s).then_some(()));
    println!("BENCH open_rss_kb={}", status_kb(pid, "VmRSS:"));
    println!("BENCH open_fds={}", open_fds(pid));
    println!("BENCH open_shm_mappings={}", shm_mappings(pid));
    click(&session);
    std::thread::sleep(Duration::from_millis(200));
    let cycles = 100u64;
    let (ran, woke) = (cpu_ns(pid), wakeups(pid));
    let hwm = status_kb(pid, "VmHWM:");
    // Every cycle waits for the popup to be on screen and then gone, so what
    // is timed is a popup that mapped, was drawn and was destroyed, not one
    // closed before it got that far. The screenshots that look cost scoot,
    // not the process timed.
    for _ in 0..cycles {
        click(&session);
        session.wait_for(&mut bar.0, "open", |s| shown(s).then_some(()));
        click(&session);
        session.wait_for(&mut bar.0, "closed", |s| (!shown(s)).then_some(()));
    }
    std::thread::sleep(Duration::from_millis(300));
    let spent = cpu_ns(pid) - ran;
    println!("BENCH cycles={cycles}");
    println!("BENCH cycle_cpu_ns_total={spent}");
    println!(
        "BENCH cycle_cpu_us_per_open_close={:.1}",
        spent as f64 / 1000.0 / cycles as f64
    );
    println!(
        "BENCH cycle_wakeups_per_open_close={:.2}",
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
