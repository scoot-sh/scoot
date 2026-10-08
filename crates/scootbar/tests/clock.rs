//! The clock on real compositors: the time read back off screenshots, on
//! every output, in the zone `TZ` names, ticking on its boundary with only
//! its own span damaged; idle between minutes without a single wakeup; and
//! the refusals when there is no usable font.
//!
//! The bar draws with the seven-segment test font (`src/testfont.rs`,
//! passed as `--font` by the harness), 50 pixels to the em on a 60-pixel
//! bar, so every segment is 5 whole pixels and the text is read back by
//! sampling them ([`common::testfont::decode`]).

// Every test here places the clock, so this file exists only where it
// does (dev/research/scootbar-testing.md: the feature matrix).
#![cfg(feature = "clock")]

mod common;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{Reaper, Session, Shot, rgb, testfont, wakeups};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const HEIGHT: u32 = 60;
const EM: u32 = 50;
/// A 50-pixel line centered in 60 rows: 5 above, baseline at 45.
const BASELINE: i64 = 45;
/// India's offset as a POSIX string: the clock must show it, not UTC.
const TZ: &str = "IST-5:30";
const OFFSET: i64 = 5 * 3600 + 1800;

/// The clock's flags: `format` (`None`: the default), in `section`.
fn clock_args<'a>(format: Option<&'a str>, section: &'a str) -> Vec<&'a str> {
    let mut args = vec![
        "--background",
        BAR,
        "--foreground",
        FG,
        "--height",
        "60",
        "--font-size",
        "50",
        section,
        "clock",
    ];
    if let Some(format) = format {
        args.extend(["--clock-format", format]);
    }
    args
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The bar's text read back from the top `HEIGHT` rows of `shot`, spaces
/// left out; empty when there is no ink yet.
fn read(shot: &Shot) -> String {
    let background = rgb(BAR);
    let ink = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as u32) < shot.width
            && (y as u32) < HEIGHT
            && shot.at(x as u32, y as u32) != background
    };
    let columns: Vec<i64> = (0..i64::from(shot.width))
        .filter(|&x| (0..i64::from(HEIGHT)).any(|y| ink(x, y)))
        .collect();
    let (Some(&left), Some(&right)) = (columns.first(), columns.last()) else {
        return String::new();
    };
    testfont::decode(ink, left, right, BASELINE, f64::from(EM))
}

/// `(hour, minute, second)` of the Unix time `t` shifted by `offset`.
fn wall(t: i64, offset: i64) -> (i64, i64, i64) {
    let s = (t + offset).rem_euclid(86_400);
    (s / 3600, s / 60 % 60, s % 60)
}

/// The 12-hour default as the test font reads it back: `3:07pm`.
fn twelve_hour(t: i64) -> String {
    let (h, m, _) = wall(t, OFFSET);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{m:02}{}", if h >= 12 { "pm" } else { "am" })
}

fn twenty_four(t: i64) -> String {
    let (h, m, _) = wall(t, OFFSET);
    format!("{h:02}:{m:02}")
}

/// Waits for output `id` to show one of the times `expected` gives for
/// the seconds around now (a minute may turn between the shot and the
/// check).
fn wait_time(
    session: &Session,
    bar: &mut Reaper,
    id: u64,
    expected: impl Fn(i64) -> String,
) -> Shot {
    session.wait_for(&mut bar.0, "the time shown", |session| {
        let shot = session.scoot_screenshot(id);
        let text = read(&shot);
        let t = now();
        (text == expected(t) || text == expected(t - 1) || text == expected(t - 60)).then_some(shot)
    })
}

#[test]
fn the_default_clock_shows_the_local_time_on_every_output() {
    let Some(session) = Session::scoot("clock", 2, "") else {
        return;
    };
    let mut bar = Reaper(session.bar_with_env(&clock_args(None, "--center"), &[("TZ", TZ)]));
    for id in [1, 2] {
        let shot = wait_time(&session, &mut bar, id, twelve_hour);
        // Centered: the ink's middle is within a glyph of the output's.
        let background = rgb(BAR);
        let columns: Vec<u32> = (0..shot.width)
            .filter(|&x| (0..HEIGHT).any(|y| shot.at(x, y) != background))
            .collect();
        let middle = (columns[0] + columns[columns.len() - 1]) / 2;
        assert!(middle.abs_diff(shot.width / 2) <= 30, "middle {middle}");
        // Nothing drawn below the bar.
        assert_ne!(shot.at(0, HEIGHT), background);
    }
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

#[test]
fn twenty_four_hours_is_one_flag_away() {
    let Some(session) = Session::scoot("clock24", 1, "") else {
        return;
    };
    let mut bar =
        Reaper(session.bar_with_env(&clock_args(Some("%H:%M"), "--center"), &[("TZ", TZ)]));
    wait_time(&session, &mut bar, 1, twenty_four);
}

/// A clock showing seconds changes on screen within about a second, and
/// each change damages only the clock's span, not the whole bar.
#[test]
fn a_seconds_clock_ticks_and_damages_only_its_span() {
    let Some(session) = Session::scoot("tick", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar_with_env(
        &clock_args(Some("%M:%S"), "--right"),
        &[("TZ", TZ), ("WAYLAND_DEBUG", "1")],
    ));
    let first = session.wait_for(&mut bar.0, "the clock drawn", |session| {
        let text = read(&session.scoot_screenshot(1));
        (text.len() == 5).then_some(text)
    });
    let next = session.wait_for(&mut bar.0, "the clock ticked", |session| {
        let text = read(&session.scoot_screenshot(1));
        (text.len() == 5 && text != first).then_some(text)
    });
    let t = now();
    let recent: Vec<String> = (0..3)
        .map(|back| {
            let (_, m, s) = wall(t - back, OFFSET);
            format!("{m:02}:{s:02}")
        })
        .collect();
    assert!(
        recent.contains(&next),
        "shows {next}, not one of {recent:?}"
    );
    // The first frame damages the whole buffer, the ticks only a span that
    // ends at the bar's right edge and is narrower than the bar.
    let trace = session.bar_stderr();
    let damages: Vec<(i64, i64)> = trace
        .lines()
        .filter_map(|line| {
            let args = line.split(".damage_buffer(").nth(1)?;
            let mut fields = args.trim_end_matches(')').split(", ");
            let x = fields.next()?.parse().ok()?;
            let _y: i64 = fields.next()?.parse().ok()?;
            let width = fields.next()?.parse().ok()?;
            Some((x, width))
        })
        .collect();
    assert!(damages.len() >= 2, "{damages:?}");
    assert_eq!(damages[0], (0, 1600), "{damages:?}");
    for &(x, width) in &damages[1..] {
        assert!(
            x > 0 && width < 1600,
            "a tick damaged {x} + {width}: {damages:?}"
        );
        assert_eq!(x + width, 1600, "{damages:?}");
    }
}

/// Between minute boundaries, a minute clock makes no system call at all:
/// its one wakeup a minute is its timer (the benchmark in the PR measures
/// the rate over five minutes).
#[test]
fn a_minute_clock_is_idle_between_minutes() {
    let Some(session) = Session::scoot("idle", 1, "") else {
        return;
    };
    let mut bar =
        Reaper(session.bar_with_env(&clock_args(Some("%H:%M"), "--center"), &[("TZ", TZ)]));
    wait_time(&session, &mut bar, 1, twenty_four);
    // A window with no minute boundary in it: if one is due within 12 s,
    // wait for it to pass first.
    let second = now().rem_euclid(60);
    if second > 45 {
        std::thread::sleep(Duration::from_secs((62 - second) as u64));
    }
    let pid = bar.0.id();
    let mut last = wakeups(pid);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = wakeups(pid);
        if now == last {
            break;
        }
        last = now;
    }
    let second = now().rem_euclid(60);
    assert!(second <= 52, "the settle ran to second {second}");
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(wakeups(pid), last, "the clock woke between minutes");
}

/// A window beside the clock bar: the clock keeps its place and time, and
/// the window (a real `foot`) is closed and gone at the end, not leaked.
#[test]
fn a_window_beside_the_clock_is_placed_and_cleaned_up() {
    let Some(session) = Session::scoot("window", 1, "") else {
        return;
    };
    let foot = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("foot").is_file()));
    if !foot {
        assert!(
            std::env::var_os("SCOOTBAR_REQUIRE_SCOOT").is_none(),
            "SCOOTBAR_REQUIRE_SCOOT is set but there is no foot on PATH"
        );
        eprintln!("skipped -- no foot on PATH");
        return;
    }
    let mut bar =
        Reaper(session.bar_with_env(&clock_args(Some("%H:%M"), "--center"), &[("TZ", TZ)]));
    wait_time(&session, &mut bar, 1, twenty_four);
    let reply = session.scoot_ipc(r#"{"type":"action","action":"spawn","command":["foot"]}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    let rect = session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"]
            .as_array()?
            .first()
            .map(|w| w["rect"].clone())
    });
    assert!(rect["y"].as_i64().unwrap() >= i64::from(HEIGHT), "{rect}");
    wait_time(&session, &mut bar, 1, twenty_four);
    let reply = session.scoot_ipc(r#"{"type":"action","action":"close_focused"}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the window closed", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"].as_array()?.is_empty().then_some(())
    });
    // And the client process with it: no `foot` left under this session.
    let dir = session.runtime_dir().to_owned();
    session.wait_for(&mut bar.0, "foot gone", |_| {
        (!foot_running_in(&dir)).then_some(())
    });
}

/// Whether a `foot` process has `dir` as its `XDG_RUNTIME_DIR`.
fn foot_running_in(dir: &std::path::Path) -> bool {
    let wanted = format!("XDG_RUNTIME_DIR={}", dir.display());
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        let comm = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
        comm.trim() == "foot"
            && std::fs::read(path.join("environ"))
                .unwrap_or_default()
                .split(|&b| b == 0)
                .any(|var| var == wanted.as_bytes())
    })
}

#[test]
fn an_unusable_font_is_a_refusal_that_names_it() {
    let Some(session) = Session::scoot("nofont", 1, "") else {
        return;
    };
    for (font, says) in [
        ("/dev/null", "not a regular file"),
        ("/nonexistent/font.ttf", "No such file"),
    ] {
        let output = session
            .scootbar()
            .args(["daemon", "--font", font, "--center", "clock"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{font}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(font) && stderr.contains(says), "{stderr}");
    }
    // With no module placed no font is needed, so none is looked for.
    let mut bar = Reaper(
        session
            .scootbar()
            .args(["daemon", "--font", "/dev/null", "--center="])
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "a bar with no modules exited"
    );
}

/// On sway: the clock on an output plugged in while it runs.
#[test]
fn the_clock_follows_a_hotplugged_output_on_sway() {
    let Some(session) = Session::sway("swayclock", 1) else {
        return;
    };
    let mut bar =
        Reaper(session.bar_with_env(&clock_args(Some("%H:%M"), "--center"), &[("TZ", TZ)]));
    let shows = |name: &str| {
        let text = read(&session.screencopy(name));
        let t = now();
        text == twenty_four(t) || text == twenty_four(t - 60)
    };
    let first = session.wait_for(&mut bar.0, "outputs", |session| {
        let names = session.swaymsg(&["-t", "get_outputs", "-r"]);
        let value: serde_json::Value = serde_json::from_str(&names).unwrap();
        value.as_array()?.first()?["name"]
            .as_str()
            .map(str::to_owned)
    });
    session.wait_for(&mut bar.0, "the clock on the first output", |_| {
        shows(&first).then_some(())
    });
    session.swaymsg(&["create_output"]);
    let second = session.wait_for(&mut bar.0, "a second output", |session| {
        let names = session.swaymsg(&["-t", "get_outputs", "-r"]);
        let value: serde_json::Value = serde_json::from_str(&names).unwrap();
        let list = value.as_array()?;
        (list.len() == 2).then(|| list[1]["name"].as_str().map(str::to_owned))?
    });
    session.wait_for(&mut bar.0, "the clock on the new output", |_| {
        shows(&second).then_some(())
    });
}

/// A clock step: what it is, the time to set, what must show at once
/// (empty: the new time), and what must show after the next boundary with
/// the seconds to wait for it.
type Step = (&'static str, i64, &'static str, Option<(&'static str, u64)>);

/// Sets `CLOCK_REALTIME` back from `CLOCK_MONOTONIC` when dropped, so a
/// test that steps the clock restores it on any exit, a failure included.
struct ClockRestore {
    /// Realtime minus monotonic when the test started, in nanoseconds.
    offset: i128,
}

impl ClockRestore {
    fn new() -> Self {
        let rt = timespec_ns(rustix::time::clock_gettime(rustix::time::ClockId::Realtime));
        let mono = timespec_ns(rustix::time::clock_gettime(
            rustix::time::ClockId::Monotonic,
        ));
        Self { offset: rt - mono }
    }

    /// Sets the wall clock to `t` Unix seconds (plus `nanos`).
    fn set(&self, t: i64, nanos: i64) {
        let spec = rustix::time::Timespec {
            tv_sec: t,
            tv_nsec: nanos,
        };
        rustix::time::clock_settime(rustix::time::ClockId::Realtime, spec)
            .expect("clock_settime (needs CAP_SYS_TIME)");
    }
}

impl Drop for ClockRestore {
    fn drop(&mut self) {
        let mono = timespec_ns(rustix::time::clock_gettime(
            rustix::time::ClockId::Monotonic,
        ));
        let back = mono + self.offset;
        let spec = rustix::time::Timespec {
            tv_sec: (back / 1_000_000_000) as i64,
            tv_nsec: (back % 1_000_000_000) as i64,
        };
        let _ = rustix::time::clock_settime(rustix::time::ClockId::Realtime, spec);
    }
}

fn timespec_ns(ts: rustix::time::Timespec) -> i128 {
    i128::from(ts.tv_sec) * 1_000_000_000 + i128::from(ts.tv_nsec)
}

/// **Sets the system clock**, so it runs only when
/// `SCOOTBAR_TEST_SET_CLOCK` is set, as root on a disposable machine (never
/// a workstation; CI does not set it). The clock is put back from
/// `CLOCK_MONOTONIC` at the end, failure or not.
///
/// A step of the wall clock shows at once, not at the next minute
/// (`TFD_TIMER_CANCEL_ON_SET`, the same path a resume from suspend takes),
/// and summer time starts and ends on the minute it should, in New York's
/// zone read from a fixture file through `TZ`.
#[test]
fn clock_steps_and_summer_time_show_on_time() {
    if std::env::var_os("SCOOTBAR_TEST_SET_CLOCK").is_none() {
        eprintln!("skipped -- sets the system clock; SCOOTBAR_TEST_SET_CLOCK=1 runs it");
        return;
    }
    let Some(session) = Session::scoot("step", 1, "") else {
        return;
    };
    let zone = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/modules/clock/fixtures/America_New_York.fat.tzif"
    );
    let tz = format!(":{zone}");
    let mut bar = Reaper(session.bar_with_env(
        &clock_args(Some("%H:%M"), "--center"),
        &[("TZ", tz.as_str())],
    ));
    let clock = ClockRestore::new();
    let shows = |session: &Session, text: &str| read(&session.scoot_screenshot(1)) == text;
    let hhmm = |t: i64, offset: i64| {
        let (h, m, _) = wall(t, offset);
        format!("{h:02}:{m:02}")
    };
    // 2026-09-29 is in New York's summer: -4:00.
    let summer = -4 * 3600;
    session.wait_for(&mut bar.0, "the time shown", |session| {
        let t = now();
        (shows(session, &hhmm(t, summer)) || shows(session, &hhmm(t - 60, summer))).then_some(())
    });

    let mut report = Vec::new();
    // Each step: where to set the clock, what must show at once, and what
    // must show after the next boundary (seconds to wait for it).
    let steps: [Step; 3] = [
        ("forward an hour", now() + 3600, "", None),
        // 2026-11-01 05:59:50Z is 01:59:50 EDT; 06:00Z is 01:00 EST.
        (
            "to 10 s before New York's summer time ends",
            1_793_512_790,
            "01:59",
            Some(("01:00", 12)),
        ),
        // 2026-03-08 06:59:50Z is 01:59:50 EST; 07:00Z is 03:00 EDT.
        (
            "to 10 s before New York's summer time starts",
            1_772_953_190,
            "01:59",
            Some(("03:00", 12)),
        ),
    ];
    for (what, to, at_once, later) in steps {
        let at_once = if at_once.is_empty() {
            hhmm(to, summer)
        } else {
            at_once.to_owned()
        };
        let set_at = std::time::Instant::now();
        clock.set(to, 0);
        session.wait_for(&mut bar.0, what, |session| {
            shows(session, &at_once).then_some(())
        });
        let shown = set_at.elapsed();
        assert!(
            shown < Duration::from_secs(5),
            "{what}: took {shown:?}, so it waited for a tick"
        );
        report.push(format!(
            "{what}: {at_once} shown {} ms after the step",
            shown.as_millis()
        ));
        if let Some((next, wait)) = later {
            session.wait_for(&mut bar.0, what, |session| {
                shows(session, next).then_some(())
            });
            let tick = set_at.elapsed();
            assert!(tick < Duration::from_secs(wait), "{what}: {tick:?}");
            report.push(format!(
                "  then {next} {} ms after the step",
                tick.as_millis()
            ));
        }
    }
    drop(clock);
    eprintln!("{}", report.join("\n"));
}
