//! The launcher draws at the compositor's scale: on a HiDPI output every
//! mode (apps, run, dmenu) attaches a buffer of exactly the surface's
//! logical size times the scale, not a 1x buffer the compositor upscales.
//!
//! Each test starts a headless scoot whose output has a scale (`[output]
//! scale` in its config, or `output-scale` live) and reads the client's own
//! `WAYLAND_DEBUG` trace: the `preferred_scale` the compositor sent, the
//! `wp_viewport.set_destination` (the surface's logical size) and every
//! `wl_shm_pool.create_buffer`. Skipped without a `scoot` binary beside the
//! test binary; `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::time::Duration;

use harness::{
    Panel, calls, created_buffers, desktop, rgb, scaled, session_with_apps_config, wait_for,
};

const BORDER: &str = "#f9e2af";

/// `preferred_scale`s the compositor sent, in 120ths, in order.
fn preferred(panel: &Panel) -> Vec<u32> {
    calls(&panel.stderr(), "preferred_scale")
        .iter()
        .filter_map(|n| n.first().copied())
        .collect()
}

/// The surface's logical size: the last viewport destination.
fn destination(panel: &Panel) -> Option<(u32, u32)> {
    let last = calls(&panel.stderr(), "set_destination").pop()?;
    Some((*last.first()?, *last.get(1)?))
}

fn apps_fixture() -> Vec<(&'static str, String)> {
    vec![
        ("firefox.desktop", desktop("Firefox", "/usr/bin/firefox")),
        ("files.desktop", desktop("Files", "/usr/bin/files")),
        ("terminal.desktop", desktop("Terminal", "/usr/bin/terminal")),
    ]
}

/// An apps (or `--run`) panel on an output at `scale`, with the wire log.
fn start(tag: &str, scale: &str, args: &[&str]) -> Option<Panel> {
    let bodies = apps_fixture();
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let (session, data_home) =
        session_with_apps_config(tag, &refs, &format!("[output]\nscale = {scale}\n"))?;
    Panel::start_on(session, data_home, args, &[("WAYLAND_DEBUG", "1")])
}

/// The panel is up and its draws have landed.
fn settle(panel: &Panel) {
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(400));
}

/// Every buffer is the surface's logical size times the compositor's
/// scale, rounded as the protocol says, from the first draw on.
fn assert_scaled(panel: &Panel, what: &str) {
    let want = preferred(panel).last().copied().unwrap();
    let (width, height) = destination(panel).expect("a viewport destination");
    let expect = (scaled(width, want), scaled(height, want));
    let buffers = created_buffers(&panel.stderr());
    assert!(!buffers.is_empty(), "{what}: no buffer created");
    for buffer in &buffers {
        assert_eq!(
            *buffer, expect,
            "{what}: a buffer for a {width}x{height} surface at {want}/120"
        );
    }
    eprintln!("{what}: {width}x{height} logical at {want}/120 -> buffers {buffers:?}");
}

/// The ticket's measurement: at `output-scale 2` the surface said 240/120
/// and the old launcher still attached a 520x282 buffer under a 520x282
/// destination. Now the buffer is 1040x564.
#[test]
fn an_apps_panel_attaches_a_2x_buffer_at_output_scale_2() {
    let Some(mut panel) = start("scale-apps-2", "2", &[]) else {
        return;
    };
    settle(&panel);
    assert_eq!(preferred(&panel).last(), Some(&240));
    assert_scaled(&panel, "apps at 2x");
    let (width, height) = destination(&panel).unwrap();
    let buffers = created_buffers(&panel.stderr());
    assert!(
        buffers.iter().all(|b| *b == (width * 2, height * 2)),
        "exactly twice the surface: {buffers:?}"
    );
    // On screen the panel is twice its logical size too: the screenshot is
    // in device pixels.
    let (x0, y0, x1, y1) = panel.find_panel(rgb(BORDER));
    assert!(x1 - x0 + 1 >= 2 * 380, "{x0}..{x1} wide for a 2x panel");
    assert!(y1 - y0 + 1 >= 2 * 200, "{y0}..{y1} tall for a 2x panel");
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1));
    panel.clean();
}

/// Run mode draws the same way.
#[test]
fn a_run_panel_attaches_a_2x_buffer_at_output_scale_2() {
    let Some(mut panel) = start("scale-run-2", "2", &["--run"]) else {
        return;
    };
    settle(&panel);
    assert_scaled(&panel, "run at 2x");
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

/// The fractional scales laptops ship with, and 3: the buffer is the
/// surface times the scale to the pixel, which is what keeps the compositor
/// from resampling it.
#[test]
fn the_buffer_is_the_surface_times_the_scale_at_every_scale() {
    for (scale, v120) in [
        ("1.25", 150),
        ("1.5", 180),
        ("1.75", 210),
        ("2", 240),
        ("3", 360),
    ] {
        let Some(mut panel) = start(&format!("scale-every-{scale}"), scale, &[]) else {
            return;
        };
        settle(&panel);
        assert_eq!(preferred(&panel).last(), Some(&v120), "{scale}");
        assert_scaled(&panel, &format!("apps at {scale}x"));
        panel.key("Escape");
        panel.wait_exit();
        panel.clean();
    }
}

/// At 1x nothing changes: the buffer is the surface.
#[test]
fn at_scale_1_the_buffer_is_the_surface() {
    let Some(mut panel) = start("scale-1", "1", &[]) else {
        return;
    };
    settle(&panel);
    let (width, height) = destination(&panel).unwrap();
    for buffer in created_buffers(&panel.stderr()) {
        assert_eq!(buffer, (width, height));
    }
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

/// A scale change while the panel is open redraws it at the new scale,
/// keeps the surface's logical size, and keeps answering the keyboard.
#[test]
fn a_scale_change_while_open_redraws_at_the_new_scale() {
    let Some(mut panel) = start("scale-live", "1", &[]) else {
        return;
    };
    settle(&panel);
    let (width, height) = destination(&panel).unwrap();
    let (x0, _, x1, _) = panel.find_panel(rgb(BORDER));
    let before = x1 - x0 + 1;
    assert_eq!(
        created_buffers(&panel.stderr()).last(),
        Some(&(width, height))
    );

    for (setting, v120) in [("2.0", 240u32), ("1.5", 180), ("1.0", 120)] {
        panel.ipc(&format!(
            r#"{{"type":"output_scale","output":1,"scale":{setting}}}"#
        ));
        wait_for(
            &format!("a buffer at {v120}/120"),
            Duration::from_secs(10),
            || {
                created_buffers(&panel.stderr()).last()
                    == Some(&(scaled(width, v120), scaled(height, v120)))
            },
        );
        // The surface is the same logical size; the output's pixels follow.
        assert_eq!(destination(&panel), Some((width, height)));
        std::thread::sleep(Duration::from_millis(300));
        let (x0, _, x1, _) = panel.find_panel(rgb(BORDER));
        let drawn = x1 - x0 + 1;
        let want = i64::from(before) * i64::from(v120) / 120;
        eprintln!("at {v120}/120: the panel is {drawn} px wide on screen (1x: {before})");
        assert!(
            (i64::from(drawn) - want).abs() <= 4,
            "{drawn} wide at {v120}/120 against {before} at 1x"
        );
    }
    // Still alive and typing.
    panel.type_text("term");
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1));
    panel.clean();
}

/// An apps panel under a soft shadow on a 1.5x output: the surface is the
/// panel plus the shadow's room, the buffer is that times 1.5, a press on
/// the shadow dismisses (the input region is the panel's, in logical
/// pixels) and the panel stays where it was.
#[test]
fn an_apps_panel_with_a_shadow_at_1_5x_keeps_its_room_and_its_input_region() {
    let bodies = apps_fixture();
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) =
        session_with_apps_config("scale-shadow", &refs, "[output]\nscale = 1.5\n")
    else {
        return;
    };
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), "shadow = \"soft\"\n").unwrap();
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    settle(&panel);
    assert_eq!(preferred(&panel).last(), Some(&180));
    assert_scaled(&panel, "apps with a shadow at 1.5x");
    // The surface is bigger than the 520x282 panel: the shadow's room.
    let (width, height) = destination(&panel).unwrap();
    assert!(width > 520 && height > 282, "{width}x{height}");
    let (x0, y0, _, y1) = panel.find_panel(rgb(BORDER));
    panel.click((x0 as f64 / 1.5) as u32 - 6, (y0 + y1) / 3);
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "a press on the shadow dismisses");
    panel.clean();
}
