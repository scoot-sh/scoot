//! Layers, edges, the exclusive zone, and `scootbar msg hide|show|toggle`
//! on a headless scoot: every layer and edge draws at its edge and reserves
//! (or not) its zone, a hidden bar holds no shm buffer and releases its
//! zone, and a hide and show round trip leaves a window where it was
//! without it jumping between.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

mod common;

use std::time::{Duration, Instant};

use common::{Reaper, Session, Shot, rgb, shm_mappings, wait_exit};
use serde_json::Value;

const BAR: &str = "#c03020";
const WIDTH: u32 = 1600;
const HEIGHT: u32 = 1000;
const BAR_HEIGHT: u32 = 30;

fn msg(session: &Session, command: &str) -> Value {
    let output = session.scootbar().arg("msg").arg(command).output().unwrap();
    assert!(
        output.status.success(),
        "msg {command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    serde_json::from_str(text.trim_end()).unwrap()
}

/// Output 1's usable rectangle as `(y, height)`.
fn usable(session: &Session) -> (i64, i64) {
    let outputs = session.scoot_outputs();
    let u = &outputs[0]["usable"];
    (u["y"].as_i64().unwrap(), u["height"].as_i64().unwrap())
}

fn wait_usable(session: &Session, bar: &mut Reaper, expected: (i64, i64), what: &str) {
    session.wait_for(&mut bar.0, what, |session| {
        (usable(session) == expected).then_some(())
    });
}

fn wait_shot(session: &Session, bar: &mut Reaper, what: &str, ok: impl Fn(&Shot) -> bool) -> Shot {
    session.wait_for(&mut bar.0, what, |session| {
        let shot = session.scoot_screenshot(1);
        ok(&shot).then_some(shot)
    })
}

/// Every layer on every edge, with a zone and without: drawn along its
/// edge, and reserving its height (or nothing) there.
#[test]
fn every_layer_and_edge_is_drawn_and_reserves_or_floats() {
    let Some(session) = Session::scoot("layers", 1, "") else {
        return;
    };
    let height = BAR_HEIGHT.to_string();
    for layer in ["bottom", "top", "overlay"] {
        for edge in ["top", "bottom"] {
            for exclusive in ["true", "false"] {
                let what = format!("{layer}/{edge}/exclusive={exclusive}");
                let mut bar = Reaper(session.bar(&[
                    "--background",
                    BAR,
                    "--height",
                    &height,
                    "--layer",
                    layer,
                    "--edge",
                    edge,
                    "--exclusive",
                    exclusive,
                ]));
                let expected = match (exclusive, edge) {
                    ("false", _) => (0, i64::from(HEIGHT)),
                    (_, "top") => (i64::from(BAR_HEIGHT), i64::from(HEIGHT - BAR_HEIGHT)),
                    _ => (0, i64::from(HEIGHT - BAR_HEIGHT)),
                };
                wait_usable(&session, &mut bar, expected, &format!("{what}: zone"));
                let (bar_row, other_row) = if edge == "top" {
                    (0, HEIGHT - 1)
                } else {
                    (HEIGHT - 1, 0)
                };
                let shot = wait_shot(&session, &mut bar, &format!("{what}: drawn"), |shot| {
                    shot.at(WIDTH / 2, bar_row) == rgb(BAR)
                });
                let rows = if edge == "top" {
                    0..BAR_HEIGHT
                } else {
                    HEIGHT - BAR_HEIGHT..HEIGHT
                };
                for y in rows {
                    assert_eq!(shot.at(0, y), rgb(BAR), "{what}: row {y}");
                    assert_eq!(shot.at(WIDTH - 1, y), rgb(BAR), "{what}: row {y}");
                }
                assert_ne!(shot.at(WIDTH / 2, other_row), rgb(BAR), "{what}");
                let reply = msg(&session, "hide");
                assert_eq!(reply["visible"], false, "{what}");
                wait_usable(&session, &mut bar, (0, 1000), &format!("{what}: released"));
                let mut kill = session.scootbar();
                assert!(kill.arg("msg").arg("kill").status().unwrap().success());
                wait_exit(&mut bar.0);
            }
        }
    }
}

#[test]
fn a_hidden_bar_holds_no_buffer_and_no_zone() {
    let Some(session) = Session::scoot("hidden", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    let pid = bar.0.id();
    wait_usable(&session, &mut bar, (28, 972), "the zone reserved");
    wait_shot(&session, &mut bar, "drawn", |s| s.at(0, 0) == rgb(BAR));
    assert!(shm_mappings(pid) > 0, "a shown bar holds a buffer");

    assert_eq!(
        msg(&session, "hide"),
        serde_json::json!({"type": "bar", "visible": false})
    );
    wait_usable(&session, &mut bar, (0, 1000), "the zone released");
    session.wait_for(&mut bar.0, "the buffers released", |_| {
        (shm_mappings(pid) == 0).then_some(())
    });
    wait_shot(&session, &mut bar, "the bar gone", |s| {
        s.at(0, 0) != rgb(BAR)
    });
    // Idempotent, and the daemon still answers.
    assert_eq!(msg(&session, "hide")["visible"], false);
    assert_eq!(msg(&session, "query")["type"], "modules");

    assert_eq!(msg(&session, "show")["visible"], true);
    wait_usable(&session, &mut bar, (28, 972), "the zone back");
    wait_shot(&session, &mut bar, "drawn again", |s| {
        s.at(0, 0) == rgb(BAR)
    });
    assert!(shm_mappings(pid) > 0);
    assert_eq!(msg(&session, "show")["visible"], true);
    assert_eq!(msg(&session, "toggle")["visible"], false);
    assert_eq!(msg(&session, "toggle")["visible"], true);
    wait_usable(&session, &mut bar, (28, 972), "the zone back again");
}

/// A burst of toggles is one change of the net result, and never leaves a
/// half state: an even burst ends shown, an odd one hidden.
#[test]
fn a_burst_of_toggles_settles_on_the_net_result() {
    let Some(session) = Session::scoot("burst", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    let pid = bar.0.id();
    wait_usable(&session, &mut bar, (28, 972), "the zone reserved");
    // Many clients at once, not one after another.
    let mut children: Vec<_> = (0..40)
        .map(|_| {
            session
                .scootbar()
                .args(["msg", "toggle"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    // 40 toggles: shown again.
    wait_usable(&session, &mut bar, (28, 972), "shown after an even burst");
    wait_shot(&session, &mut bar, "drawn", |s| s.at(0, 0) == rgb(BAR));
    assert!(shm_mappings(pid) > 0);
    assert_eq!(msg(&session, "toggle")["visible"], false);
    wait_usable(&session, &mut bar, (0, 1000), "hidden after an odd one");
    assert_eq!(shm_mappings(pid), 0);
}

/// Hidden is runtime state: a reload keeps it, and a bar restarted is
/// shown.
#[test]
fn a_reload_keeps_the_bar_hidden() {
    let Some(session) = Session::scoot("reload", 1, "") else {
        return;
    };
    let path = session.runtime_dir().join("bar.toml");
    std::fs::write(&path, "[bar]\nlayer = \"top\"\n").unwrap();
    let mut bar = Reaper(
        session
            .scootbar()
            .args(["daemon", "--center=", "--config"])
            .arg(&path)
            .arg("--font")
            .arg(session.font())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(session.bar_log()).unwrap())
            .spawn()
            .unwrap(),
    );
    wait_usable(&session, &mut bar, (28, 972), "the zone reserved");
    msg(&session, "hide");
    wait_usable(&session, &mut bar, (0, 1000), "hidden");
    // A geometry change while hidden: nothing is made.
    std::fs::write(&path, "[bar]\nlayer = \"overlay\"\nexclusive = false\n").unwrap();
    msg(&session, "reload");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(usable(&session), (0, 1000));
    assert_eq!(shm_mappings(bar.0.id()), 0);
    // Shown with the new settings: floating, so no zone.
    msg(&session, "show");
    wait_shot(&session, &mut bar, "drawn", |s| {
        s.at(0, 0) == rgb("#1e1e2e")
    });
    assert_eq!(usable(&session), (0, 1000));
    assert!(shm_mappings(bar.0.id()) > 0);
}

fn window_rect(session: &Session) -> Option<(i64, i64, i64, i64)> {
    let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
    let r = &reply["windows"].as_array()?.first()?["rect"];
    Some((
        r["x"].as_i64()?,
        r["y"].as_i64()?,
        r["width"].as_i64()?,
        r["height"].as_i64()?,
    ))
}

/// A window moves once when the bar goes and once when it returns, and
/// is where it was after the round trip.
#[test]
fn a_round_trip_does_not_make_windows_jump() {
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("foot").is_file()));
    if !found {
        assert!(
            std::env::var_os("SCOOTBAR_REQUIRE_SCOOT").is_none(),
            "SCOOTBAR_REQUIRE_SCOOT is set but there is no foot on PATH"
        );
        eprintln!("skipped -- no foot on PATH");
        return;
    }
    let Some(session) = Session::scoot("jump", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    wait_usable(&session, &mut bar, (28, 972), "the zone reserved");
    let reply = session.scoot_ipc(r#"{"type":"action","action":"spawn","command":["foot"]}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    let with_bar = session.wait_for(&mut bar.0, "a window mapped", window_rect);
    assert!(with_bar.1 >= 28, "{with_bar:?}");
    // Let the window settle at its slot.
    std::thread::sleep(Duration::from_millis(300));
    let with_bar = window_rect(&session).unwrap();

    // Every rect the window has, sampled every few ms across the
    // transition, in order and without repeats.
    let trace = |session: &Session, until: (i64, i64, i64, i64)| {
        let mut seen = vec![];
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut settled = None;
        while Instant::now() < deadline {
            let rect = window_rect(session).unwrap();
            if seen.last() != Some(&rect) {
                seen.push(rect);
            }
            if rect == until {
                let at = *settled.get_or_insert_with(Instant::now);
                if at.elapsed() > Duration::from_millis(400) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        seen
    };

    msg(&session, "hide");
    session.wait_for(&mut bar.0, "the bar gone", |session| {
        (usable(session) == (0, 1000)).then_some(())
    });
    // The window reclaims the bar's 28 rows, where its slot began.
    let hidden = (with_bar.0, with_bar.1 - 28, with_bar.2, with_bar.3 + 28);
    session.wait_for(&mut bar.0, "the window reclaimed the zone", |session| {
        (window_rect(session) == Some(hidden)).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));

    msg(&session, "show");
    let seen = trace(&session, with_bar);
    assert_eq!(seen.last(), Some(&with_bar), "{seen:?}");
    // From the reclaimed rect straight to the bar's: nothing between.
    assert!(seen.len() <= 2, "the window jumped: {seen:?}");
    assert!(
        seen.iter().all(|r| *r == hidden || *r == with_bar),
        "{seen:?}"
    );

    let reply = session.scoot_ipc(r#"{"type":"action","action":"close_focused"}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the window closed", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"].as_array()?.is_empty().then_some(())
    });
}

#[test]
fn a_bad_layer_or_exclusive_flag_starts_nothing() {
    let Some(session) = Session::scoot("badflag", 1, "") else {
        return;
    };
    for args in [
        ["--layer", "background"],
        ["--exclusive", "maybe"],
        ["--layer", ""],
    ] {
        let output = session
            .scootbar()
            .arg("daemon")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

/// The layer and zone requests as the protocol trace has them: the pixels
/// alone cannot tell `bottom` from `top` on an empty desktop.
#[test]
fn the_requests_carry_the_layer_and_the_zone() {
    let Some(session) = Session::scoot("trace", 1, "") else {
        return;
    };
    // wlr-layer-shell's `layer` enum: 1 bottom, 2 top, 3 overlay.
    for (layer, number, exclusive, zone) in [
        ("bottom", "1", "true", "28"),
        ("top", "2", "true", "28"),
        ("overlay", "3", "false", "-1"),
    ] {
        let mut bar = Reaper(session.bar_with_env(
            &["--layer", layer, "--exclusive", exclusive],
            &[("WAYLAND_DEBUG", "1")],
        ));
        session.wait_for(&mut bar.0, "the zone requested", |session| {
            let log = session.bar_stderr();
            log.contains(&format!(".set_exclusive_zone({zone})"))
                .then_some(log)
        });
        let log = session.bar_stderr();
        let request = log
            .lines()
            .find(|line| line.contains(".get_layer_surface("))
            .unwrap_or_else(|| panic!("no get_layer_surface: {log}"));
        assert!(
            request.contains(&format!(", {number}, Some(\"scootbar\"))")),
            "{layer}: {request}"
        );
        assert!(
            session
                .scootbar()
                .args(["msg", "kill"])
                .status()
                .unwrap()
                .success()
        );
        wait_exit(&mut bar.0);
    }
}
