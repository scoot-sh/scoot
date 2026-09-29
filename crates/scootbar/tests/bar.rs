//! scootbar on a headless scoot: a bar on every output that reserves its
//! space (scoot's `outputs` reports the usable area it leaves), in its
//! color where it should be (scoot's own screenshots), at the output's real
//! device pixels at a fractional scale, idle with no wakeups, and gone with
//! the compositor.
//!
//! Skipped without a `scoot` binary (see `common`); `SCOOTBAR_REQUIRE_SCOOT`
//! makes that a failure. Outputs coming and going are `tests/hotplug.rs`,
//! on sway: scoot's headless backend cannot add or remove them at runtime.
#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use common::{Reaper, Session, Shot, rgb, wait_exit, wakeups};
use serde_json::Value;

const BAR: &str = "#c03020";
const WIDTH: u32 = 1600;
const HEIGHT: u32 = 1000;

/// Output `id`'s usable rectangle as `(x, y, width, height)`.
fn usable(outputs: &[Value], id: u64) -> Option<(i64, i64, i64, i64)> {
    let output = outputs.iter().find(|o| o["id"] == id)?;
    let u = &output["usable"];
    Some((
        u["x"].as_i64()?,
        u["y"].as_i64()?,
        u["width"].as_i64()?,
        u["height"].as_i64()?,
    ))
}

/// Waits until every one of `ids` has `expected` as its usable rectangle
/// (relative to the output's own origin).
fn wait_usable(session: &Session, bar: &mut Reaper, ids: &[u64], expected: (i64, i64, i64, i64)) {
    session.wait_for(&mut bar.0, "the zone reserved", |session| {
        let outputs = session.scoot_outputs();
        ids.iter()
            .all(|&id| {
                let origin = outputs
                    .iter()
                    .find(|o| o["id"] == id)
                    .and_then(|o| o["rect"]["x"].as_i64())
                    .unwrap_or(0);
                usable(&outputs, id).map(|(x, y, w, h)| (x - origin, y, w, h)) == Some(expected)
            })
            .then_some(())
    });
}

/// Waits until output `id`'s screenshot has the bar's color at `(x, y)`,
/// and returns that screenshot.
fn wait_drawn(session: &Session, bar: &mut Reaper, id: u64, x: u32, y: u32) -> Shot {
    session.wait_for(&mut bar.0, "the bar drawn", |session| {
        let shot = session.scoot_screenshot(id);
        (shot.at(x, y) == rgb(BAR)).then_some(shot)
    })
}

/// The rows `from..to` of `shot`, at columns `x0..x1`, are all `color`.
fn assert_rows(
    shot: &Shot,
    rows: std::ops::Range<u32>,
    cols: std::ops::Range<u32>,
    color: [u8; 3],
    what: &str,
) {
    for y in rows {
        for x in cols.clone() {
            assert_eq!(shot.at(x, y), color, "{what}: pixel ({x},{y})");
        }
    }
}

fn assert_not(shot: &Shot, x: u32, y: u32, what: &str) {
    assert_ne!(
        shot.at(x, y),
        rgb(BAR),
        "{what}: pixel ({x},{y}) is the bar's"
    );
}

#[test]
fn every_output_gets_a_bar_that_reserves_its_height() {
    let Some(session) = Session::scoot("every", 2, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    // The default height, 28, off the top of both outputs.
    wait_usable(&session, &mut bar, &[1, 2], (0, 28, 1600, 972));
    for id in [1, 2] {
        let shot = wait_drawn(&session, &mut bar, id, 0, 0);
        assert_eq!((shot.width, shot.height), (WIDTH, HEIGHT));
        assert_rows(&shot, 0..28, 0..WIDTH, rgb(BAR), "the bar");
        for x in [0, WIDTH / 2, WIDTH - 1] {
            assert_not(&shot, x, 28, "below the bar");
            assert_not(&shot, x, HEIGHT - 1, "the bottom");
        }
    }
}

#[test]
fn a_margin_is_reserved_on_the_anchored_edge_and_the_surface_is_the_bar() {
    let Some(session) = Session::scoot("margin", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR, "--height", "20", "--margin", "8,4"]));
    // The zone is the bar plus the margin on its edge: scoot adds the
    // margin (Smithay's `arrange`), and the bar sets only its height.
    wait_usable(&session, &mut bar, &[1], (0, 28, 1600, 972));
    let shot = wait_drawn(&session, &mut bar, 1, 4, 8);
    // Exactly the bar: 20 rows from y 8, 4 in from each side, and nothing
    // of its color in the margin around it (no transparent border drawn,
    // and none needed).
    assert_rows(&shot, 8..28, 4..WIDTH - 4, rgb(BAR), "the bar");
    for x in [0, 3, WIDTH / 2, WIDTH - 4, WIDTH - 1] {
        assert_not(&shot, x, 7, "the top margin");
        assert_not(&shot, x, 28, "below the bar");
    }
    for y in [8, 17, 27] {
        assert_not(&shot, 3, y, "the left margin");
        assert_not(&shot, WIDTH - 4, y, "the right margin");
    }

    // A window goes beside it, inside the usable area.
    if common_foot_missing() {
        return;
    }
    let reply = session.scoot_ipc(r#"{"type":"action","action":"spawn","command":["foot"]}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    let rect = session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"]
            .as_array()?
            .first()
            .map(|w| w["rect"].clone())
    });
    let y = rect["y"].as_i64().unwrap();
    let height = rect["height"].as_i64().unwrap();
    assert!(
        y >= 28,
        "the window starts at y {y}, over the bar or its margin: {rect}"
    );
    assert!(y + height <= i64::from(HEIGHT), "{rect}");
    // Close it rather than leave it to outlive the session (the harness
    // also kills scoot's children, for a test that fails before here).
    let reply = session.scoot_ipc(r#"{"type":"action","action":"close_focused"}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the window closed", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"].as_array()?.is_empty().then_some(())
    });
}

/// `foot` is the one client the tests use to place a window; it is in the
/// dev shell. Without it the placement check is skipped, unless
/// `SCOOTBAR_REQUIRE_SCOOT` asks for everything.
fn common_foot_missing() -> bool {
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("foot").is_file()));
    if !found {
        assert!(
            std::env::var_os("SCOOTBAR_REQUIRE_SCOOT").is_none(),
            "SCOOTBAR_REQUIRE_SCOOT is set but there is no foot on PATH"
        );
        eprintln!("skipped the window check -- no foot on PATH");
    }
    !found
}

#[test]
fn a_bottom_bar_reserves_the_bottom() {
    let Some(session) = Session::scoot("bottom", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR, "--edge", "bottom", "--height", "30"]));
    wait_usable(&session, &mut bar, &[1], (0, 0, 1600, 970));
    let shot = wait_drawn(&session, &mut bar, 1, 0, HEIGHT - 1);
    assert_rows(&shot, HEIGHT - 30..HEIGHT, 0..WIDTH, rgb(BAR), "the bar");
    assert_not(&shot, 0, HEIGHT - 31, "above the bar");
    assert_not(&shot, 0, 0, "the top");
}

/// The bar's height in device pixels on `shot`: the rows from the top, at
/// every sampled column, in its color.
fn device_height(shot: &Shot) -> u32 {
    let columns = [0, shot.width / 2, shot.width - 1];
    let mut rows = 0;
    while rows < shot.height && columns.iter().all(|&x| shot.at(x, rows) == rgb(BAR)) {
        rows += 1;
    }
    rows
}

#[test]
fn a_fractional_scale_is_drawn_at_device_pixels_and_follows_a_change() {
    let Some(session) = Session::scoot("scale", 1, "[output]\nscale = 1.5\n") else {
        return;
    };
    let mut bar = Reaper(session.bar_with_env(&["--background", BAR], &[("WAYLAND_DEBUG", "1")]));
    // Each step: the scale, the buffer the bar must be drawn in (device
    // pixels: scoot's logical width times the scale, rounded as the
    // protocol says; a solid color cannot show a buffer drawn larger and
    // scaled down, so the trace is what proves it), and the device rows
    // the screenshot must show.
    //
    // - 1.5: 1600 / 1.5 is 1067 logical (scoot rounds up), drawn 1601
    //   wide, as scootbg's `density` explains; 28 × 1.5 = 42.
    // - 1.25: 1280 logical, 1600 wide; 28 × 1.25 = 35.
    // - 2: 800 logical, 1600 wide; 56.
    for (step, (scale, buffer, rows)) in [
        ("1.5", (1601, 42), 42),
        ("1.25", (1600, 35), 35),
        ("2", (1600, 56), 56),
    ]
    .into_iter()
    .enumerate()
    {
        if step > 0 {
            std::fs::write(
                session.runtime_dir().join("config.toml"),
                format!("[output]\nscale = {scale}\n"),
            )
            .unwrap();
            let reply = session.scoot_ipc(r#"{"type":"reload"}"#);
            assert_eq!(reply["type"], "reloaded", "{reply}");
        }
        let shot = session.wait_for(&mut bar.0, &format!("{rows} rows at {scale}"), |session| {
            let shot = session.scoot_screenshot(1);
            (device_height(&shot) == rows).then_some(shot)
        });
        assert_rows(&shot, 0..rows, 0..shot.width, rgb(BAR), scale);
        assert_not(&shot, shot.width / 2, rows, scale);
        let buffers = common::created_buffers(&session.bar_stderr());
        assert_eq!(
            buffers.last(),
            Some(&buffer),
            "at {scale}, the buffers made so far: {buffers:?}"
        );
    }
}

#[test]
fn without_a_viewporter_it_draws_at_the_integer_scale() {
    // The knob exists in debug builds only (`daemon::wayland`).
    if !cfg!(debug_assertions) {
        eprintln!("skipped -- SCOOTBAR_DEBUG_NO_VIEWPORTER needs a debug build");
        return;
    }
    let Some(session) = Session::scoot("noviewport", 1, "[output]\nscale = 1.5\n") else {
        return;
    };
    let mut bar = Reaper(session.bar_with_env(
        &["--background", BAR],
        &[
            ("WAYLAND_DEBUG", "1"),
            ("SCOOTBAR_DEBUG_NO_VIEWPORTER", "1"),
        ],
    ));
    // Drawn at 2 (1.5 rounded up), 1067 × 28 logical as 2134 × 56, with
    // `set_buffer_scale(2)`; scoot scales it down to the same 42 rows.
    let shot = session.wait_for(&mut bar.0, "42 rows", |session| {
        let shot = session.scoot_screenshot(1);
        (device_height(&shot) == 42).then_some(shot)
    });
    assert_rows(&shot, 0..42, 0..shot.width, rgb(BAR), "the bar");
    let trace = session.bar_stderr();
    assert_eq!(
        common::created_buffers(&trace).last(),
        Some(&(2134, 56)),
        "{trace}"
    );
    assert!(trace.contains("set_buffer_scale(2)"), "{trace}");
    // Advertised in the registry dump, but never bound: no object.
    assert!(!trace.contains("wp_viewporter@"), "{trace}");
    assert!(!trace.contains("wp_viewport@"), "{trace}");
    assert!(
        trace.contains("the compositor has no wp_viewporter"),
        "{trace}"
    );
}

#[test]
fn idle_it_makes_no_wakeups() {
    let Some(session) = Session::scoot("idle", 2, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    wait_usable(&session, &mut bar, &[1, 2], (0, 28, 1600, 972));
    for id in [1, 2] {
        wait_drawn(&session, &mut bar, id, 0, 0);
    }
    // Settled once the count stops moving; then a window with no change.
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
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(wakeups(pid), last, "scootbar woke up while idle");
}

#[test]
fn it_exits_with_an_error_when_the_compositor_goes() {
    let Some(mut session) = Session::scoot("gone", 1, "") else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    wait_usable(&session, &mut bar, &[1], (0, 28, 1600, 972));
    session.kill_compositor();
    let status = wait_exit(&mut bar.0);
    assert_eq!(status.code(), Some(1), "{}", session.bar_stderr());
    let stderr = session.bar_stderr();
    assert!(
        stderr.contains("lost the connection to the compositor"),
        "{stderr}"
    );
}

#[test]
fn no_compositor_is_an_error_not_a_hang() {
    // No compositor needed: a display that does not exist. The test font,
    // like every other test's: the daemon loads a font before it connects,
    // so without one this would fail on a machine with no system fonts
    // before it ever reached the connection it is here to test.
    let scratch = common::Scratch::new("none");
    let output = std::process::Command::new(common::scootbar_bin())
        .arg("daemon")
        .arg("--font")
        .arg(scratch.font())
        .env("XDG_RUNTIME_DIR", &scratch.0)
        .env("WAYLAND_DISPLAY", "wayland-none")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot connect to the Wayland compositor"),
        "{stderr}"
    );
}

#[test]
fn a_usage_error_is_status_2_and_starts_nothing() {
    let output = std::process::Command::new(common::scootbar_bin())
        .args(["daemon", "--height", "0"])
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--height"), "{stderr}");
}
