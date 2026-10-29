//! The panel on a headless scoot: open, type through `scoot msg type`
//! and launch; toggle; dismiss; and the per-shipped-look goldens.
//!
//! Each test starts its own headless scoot with a fixture applications
//! tree (so no test reads the machine's desktop files) and drives the
//! real `scootlaunch` binary. Skipped without a `scoot` binary beside
//! the test binary; `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure, so
//! CI cannot go green by skipping. A pick launches its stub (which
//! records the environment the launch gave it) and exits 0 with empty
//! stdout; a dismiss exits 1.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::process::{Command, Stdio};
use std::time::Duration;

use harness::{
    BROWSER_BLUE, Geom, MARK_WHITE, Panel, TERMINAL_ORANGE, desktop, desktop_with_icon,
    fixture_icon_theme, launch_bin, render_launch_fragment, rgb, session_with_apps,
    session_with_stubs, wait_for, wait_marker,
};

/// The default look's font size and radius (no look layer in the sandbox).
const DEFAULT_EM: f32 = 13.0;
const DEFAULT_RADIUS: u32 = 8;

fn refs<'a>(bodies: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    bodies
        .iter()
        .map(|(file, body)| (*file, body.as_str()))
        .collect()
}

fn apps() -> Vec<(&'static str, String)> {
    vec![
        ("firefox.desktop", desktop("Firefox", "/usr/bin/firefox")),
        ("files.desktop", desktop("Files", "/usr/bin/files")),
        ("terminal.desktop", desktop("Terminal", "/usr/bin/terminal")),
    ]
}

#[test]
fn open_type_select_launches() {
    let Some((session, data_home, markers)) =
        session_with_stubs("type-select", &["firefox", "files", "terminal"])
    else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    // The defaults (no look layer in the sandbox): surface #1e1e2e,
    // focus #f9e2af.
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    let g = Geom::new(DEFAULT_EM, DEFAULT_RADIUS);
    panel.type_text("term");
    std::thread::sleep(Duration::from_millis(500));
    panel.save("type-select");
    // The redraw reached the screen: only "terminal" matches, so the
    // second row's middle is panel fill, not a row.
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    assert_eq!(
        panel
            .shot()
            .at((x0 + x1) / 2, y0 + g.list_y() + g.row_h + g.row_h / 2),
        rgb("#1e1e2e"),
        "the filtered list drew"
    );
    panel.key("Return");
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(status.success(), "exit 0 on a launch: {}", panel.stderr());
    assert_eq!(out, "", "a launch prints nothing: {}", panel.stderr());
    // The filtered pick ran: only the terminal stub fired.
    let terminal = wait_marker(&markers[2]);
    assert!(terminal.contains("terminal.sh"), "{terminal}");
    assert!(
        fs::read_to_string(&markers[0])
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        fs::read_to_string(&markers[1])
            .unwrap_or_default()
            .is_empty()
    );
    // The launch is recorded for frecency.
    let history = panel
        .session
        .scratch
        .state_home()
        .join("scootlaunch")
        .join("history");
    let text = fs::read_to_string(&history).unwrap_or_default();
    assert!(
        text.contains("terminal.desktop"),
        "the launch is recorded: {text}"
    );
    panel.clean();
    assert!(
        !panel
            .session
            .scratch
            .0
            .join(format!(
                "scootlaunch-{}.sock",
                panel.session.wayland_display
            ))
            .exists(),
        "the socket is gone with the panel"
    );
}

#[test]
fn ctrl_n_moves_and_return_launches() {
    // The modifiers-mask Control path, end to end: Ctrl+N moves like
    // Down through a real compositor's modifiers events.
    let Some((session, data_home, markers)) =
        session_with_stubs("ctrl-n", &["firefox", "files", "terminal"])
    else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("ctrl+n");
    std::thread::sleep(Duration::from_millis(300));
    panel.save("ctrl-n");
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0 on a launch: {}", panel.stderr());
    assert_eq!(panel.stdout(), "", "a launch prints nothing");
    // Empty query orders by name (files, firefox, terminal); Ctrl+N
    // moves from files to firefox.
    let firefox = wait_marker(&markers[0]);
    assert!(firefox.contains("firefox.sh"), "Ctrl+N moved: {firefox}");
    panel.clean();
}

#[test]
fn a_launch_gets_a_fresh_token_not_the_panels() {
    // The environment rules: the panel's own token never leaks into
    // the child; the minted one does.
    let Some((session, data_home, markers)) = session_with_stubs("token-env", &["alpha"]) else {
        return;
    };
    let Some(mut panel) = Panel::start_on(
        session,
        data_home,
        &[],
        &[("XDG_ACTIVATION_TOKEN", "parent-token-xyz")],
    ) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0 on a launch: {}", panel.stderr());
    let line = wait_marker(&markers[0]);
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields.len(), 4, "{line}");
    assert!(fields[0].ends_with("alpha.sh"), "{line}");
    assert_ne!(
        fields[1], "parent-token-xyz",
        "the panel's token must not leak: {line}"
    );
    panel.clean();
}

#[test]
fn the_toggle_closes_a_running_instance() {
    let bodies = apps();
    let Some(mut first) = Panel::start("toggle-first", &refs(&bodies), &[]) else {
        return;
    };
    first.wait_shown(rgb("#f9e2af"));
    // The second invocation tells the first to close and exits itself.
    let log = first.session.scratch.0.join("panel2.log");
    let file = fs::File::create(&log).unwrap();
    let empty_dirs = first.session.scratch.0.join("empty-dirs");
    let mut second = Command::new(launch_bin());
    second
        .env("XDG_RUNTIME_DIR", &first.session.scratch.0)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env("WAYLAND_DISPLAY", &first.session.wayland_display)
        .env("XDG_DATA_HOME", &first.data_home)
        .env("XDG_DATA_DIRS", &empty_dirs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(file);
    first.session.scratch.sandbox(&mut second);
    second
        .env("XDG_DATA_HOME", &first.data_home)
        .env("XDG_DATA_DIRS", &empty_dirs);
    let mut second = second.spawn().unwrap();
    wait_for("the toggle to exit", Duration::from_secs(10), || {
        second.try_wait().unwrap().is_some()
    });
    assert!(
        second.try_wait().unwrap().unwrap().success(),
        "the toggle exits 0"
    );
    let status = first.wait_exit();
    assert_eq!(status.code(), Some(1), "the first panel was dismissed");
    first.clean();
}

#[test]
fn escape_dismisses_without_a_launch() {
    let bodies = apps();
    let Some(mut panel) = Panel::start("escape", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "dismiss exits 1");
    assert_eq!(panel.stdout(), "", "no launch prints");
    panel.clean();
}

#[test]
fn pointer_motion_outside_does_not_dismiss() {
    // Moving the pointer off the panel is not a dismiss; only a press is.
    // Escape still dismisses after.
    let bodies = apps();
    let Some(mut panel) = Panel::start("outside", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    // The panel is centered and far smaller than the output: the corner
    // is outside it.
    panel.hover(10, 10);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "motion alone keeps the panel open"
    );
    panel.wait_shown(rgb("#f9e2af"));
    panel.save("outside-stays-open");
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "Escape dismisses");
    panel.clean();
}

#[test]
fn a_click_outside_the_panel_dismisses_it() {
    // The compositor never hands an exclusive surface's keyboard to the
    // desktop, so scoot closes the panel on a press outside it: exit 1,
    // like Escape, and nothing launched.
    let Some((session, data_home, markers)) =
        session_with_stubs("outside-click", &["alpha", "beta", "gamma"])
    else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // The corner of a centered panel far smaller than the output.
    panel.click(10, 10);
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "an outside click exits 1");
    assert_eq!(panel.stdout(), "", "no launch prints");
    for marker in &markers {
        assert!(
            fs::read_to_string(marker).unwrap_or_default().is_empty(),
            "nothing launched"
        );
    }
    panel.clean();
}

#[test]
fn a_click_on_a_row_launches_it() {
    let Some((session, data_home, markers)) =
        session_with_stubs("row-click", &["alpha", "beta", "gamma"])
    else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // The second row's middle, from the border box.
    let g = Geom::new(DEFAULT_EM, DEFAULT_RADIUS);
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    panel.click((x0 + x1) / 2, y0 + g.list_y() + g.row_h + g.row_h / 2);
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(
        status.success(),
        "exit 0 on a mouse launch: {}",
        panel.stderr()
    );
    assert_eq!(out, "", "a launch prints nothing");
    let beta = wait_marker(&markers[1]);
    assert!(beta.contains("beta.sh"), "the row launched: {beta}");
    panel.clean();
}

#[test]
fn a_missing_target_shows_its_error_and_stays_open() {
    // The Exec target vanished between index and launch: the error
    // draws for 2 s, the panel stays open, no history is recorded.
    let bodies: Vec<(&str, String)> = vec![(
        "gone.desktop",
        desktop("Gone", "/nonexistent-scootlaunch-target"),
    )];
    let Some(mut panel) = Panel::start("gone-target", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    // Still open past the launch: the error draws, nothing exits.
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "the panel stays open on a spawn failure"
    );
    panel.save("gone-target-error");
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "Escape dismisses after the error");
    // A failed spawn records nothing.
    assert!(
        !panel
            .session
            .scratch
            .state_home()
            .join("scootlaunch")
            .join("history")
            .exists(),
        "a failed spawn records nothing"
    );
    panel.clean();
}

/// The per-shipped-look goldens: the panel draws what fuzzel draws for
/// the look (the design's table), read back off a real screenshot. The
/// look layer is the rendered `launch.toml` for the shipped look, as
/// `scoot look apply` would write it.
#[test]
fn goldens_per_shipped_look() {
    const SHIPPED: &[&str] = &[
        "ginger-night",
        "moonrise",
        "music-desk",
        "radial-burst",
        "vinyl-sunset",
    ];
    for name in SHIPPED {
        golden_one(name);
    }
}

fn golden_one(name: &str) {
    let bodies: Vec<(&str, String)> = vec![
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
        ("gamma.desktop", desktop("Gamma", "/usr/bin/gamma")),
    ];
    let refs = refs(&bodies);
    // The rendered fragment for the shipped look, as the state link
    // would carry it after `scoot look apply`. Written before the open:
    // the panel reads the layer at open, never after.
    let fragment = render_launch_fragment(name);
    let (session, data_home) = match session_with_apps(&format!("golden-{name}"), &refs) {
        Some(session) => session,
        None => return,
    };
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), &fragment).unwrap();
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    let table: toml::Table = fragment.parse().expect("launch.toml parses");
    let colors = table["colors"].as_table().expect("colors").clone();
    let color = |key: &str| rgb(colors[key].as_str().unwrap());
    let surface = color("surface");
    let focus = color("focus");
    let selection = color("selection");
    let em = table["font-size"].as_integer().expect("font-size") as f32;
    let opacity = table["opacity"].as_float().unwrap_or(1.0);
    panel.wait_shown(focus);
    panel.save(&format!("golden-{name}"));
    let shot = panel.shot();
    // Finds the panel through its opaque border ring, whatever the
    // compositor's placement: the border's bounding box is the panel.
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for x in (0..1600).step_by(2) {
        for y in (0..1000).step_by(2) {
            if shot.at(x, y) == focus {
                xs.push(x);
                ys.push(y);
            }
        }
    }
    assert!(!xs.is_empty(), "{name}: no border ring on screen");
    let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let (y0, y1) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
    let width = 40.0 * em;
    let radius = table["radius"].as_integer().expect("radius") as u32;
    let g = Geom::new(em, radius);
    let height = g.height(10) as f32;
    assert!(
        (x1 - x0) as f32 >= width - 6.0 && (x1 - x0) as f32 <= width + 2.0,
        "{name}: panel width {} for {width}",
        x1 - x0
    );
    assert!(
        (y1 - y0) as f32 >= height - 6.0 && (y1 - y0) as f32 <= height + 2.0,
        "{name}: panel height {} for {height}",
        y1 - y0
    );
    // The pad under the last row is surface fill (blended at the look's
    // opacity over the black output); the first row is the selection.
    // The probe sits at the bottom-center: clear of the rounded corners
    // at any shipped radius.
    let expected_fill = blend_over_black(surface, opacity);
    let fill = shot.at((x0 + x1) / 2, y1 - 6);
    assert!(
        near(fill, expected_fill, 4),
        "{name}: fill {fill:?} for {expected_fill:?} at opacity {opacity}"
    );
    let row_center_y = y0 + g.list_y() + g.row_h / 2;
    let selected = shot.at((x0 + x1) / 2, row_center_y);
    assert_eq!(
        selected, selection,
        "{name}: first row {selected:?} for selection {selection:?}"
    );
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

/// The icon column draws, per shipped look: a headless panel over the
/// fixture icon theme (a PNG icon, an SVG icon, a tile fallback), a
/// query typed, and the icon column's pixels counted over the rows.
/// The terminal PNG mark, the browser SVG disc and the accent tile must
/// each show; neutering `draw_icon` (returning early) leaves the column
/// at row background and fails all three counts, so the icon draw path
/// has pixel coverage per decoder.
///
/// The same open saves `scootlaunch-{name}.png` when
/// `SCOOTLAUNCH_SHOTS` is set: those saves are the docs screenshots
/// (`docs/assets/` and `site/src/assets/`), reproducible with
/// `SCOOTLAUNCH_SHOTS=<dir> cargo nextest run -p scootlaunch icons_docs`.
#[test]
fn icons_docs_moonrise() {
    icons_docs_one("moonrise");
}

/// See [`icons_docs_moonrise`]: the light look draws the same column
/// from the same roles.
#[test]
fn icons_docs_music_desk() {
    icons_docs_one("music-desk");
}

fn icons_docs_one(name: &str) {
    let bodies: Vec<(&str, String)> = vec![
        (
            "alpha.desktop",
            desktop_with_icon("Alpha", "/usr/bin/alpha", "alpha"),
        ),
        (
            "beta.desktop",
            desktop_with_icon("Beta", "/usr/bin/beta", "beta"),
        ),
        ("gamma.desktop", desktop("Gamma", "/usr/bin/gamma")),
    ];
    let refs = refs(&bodies);
    let (session, data_home) = match session_with_apps(&format!("icons-docs-{name}"), &refs) {
        Some(session) => session,
        None => return,
    };
    let theme = fixture_icon_theme(&data_home);
    // The rendered fragment for the shipped look, pointed at the
    // fixture theme instead of the machine's icon theme.
    let fragment = render_launch_fragment(name);
    let mut table: toml::Table = fragment.parse().expect("launch.toml parses");
    table
        .get_mut("icons")
        .and_then(|icons| icons.as_table_mut())
        .expect("icons table")
        .insert("theme".to_string(), toml::Value::String(theme));
    let fragment = table.to_string();
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), &fragment).unwrap();
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    let table: toml::Table = fragment.parse().expect("launch.toml parses");
    let colors = table["colors"].as_table().expect("colors").clone();
    let color = |key: &str| rgb(colors[key].as_str().unwrap());
    let focus = color("focus");
    let accent = color("accent");
    let em = table["font-size"].as_integer().expect("font-size") as f32;
    panel.wait_shown(focus);
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("a");
    std::thread::sleep(Duration::from_millis(500));
    panel.save(&format!("scootlaunch-{name}"));
    let shot = panel.shot();
    let (x0, y0, _, _) = panel.find_panel(focus);
    // The icon column, as `paint_panel` derives it at scale 1: one
    // `lead` inside the row, which is inside the highlight.
    let radius = table["radius"].as_integer().expect("radius") as u32;
    let g = Geom::new(em, radius);
    let (row_h, side) = (g.row_h, g.side);
    let icon_x = x0 + g.inner + g.lead;
    let list_y = y0 + g.list_y();
    // All three rows match "a": Alpha (terminal PNG mark), Beta
    // (browser SVG disc), Gamma (letter tile). Count each decoder's
    // colors over all three rows — the rank order among equal matches
    // is not positional — while only icons paint these colors. Each
    // rect grows 1 px per side: `find_panel` steps by 2, so its box can
    // sit 1 px off the true border.
    // The marks are two-tone (glyph white over the shape color, shared
    // white counted for both). The opaque PNG square reads back mostly
    // exact, so its bound is half the square; the SVG disc is drawn over
    // transparency and its ring is thin after resampling, so its bound
    // is an eighth — both well above the zero a neutered `draw_icon`
    // leaves.
    let mut terminal = 0u32;
    let mut browser = 0u32;
    let mut tile = 0u32;
    for row in 0..3 {
        let y = list_y + row * row_h + row_h.saturating_sub(side) / 2;
        for x in icon_x.saturating_sub(1)..icon_x + side + 1 {
            for yy in y.saturating_sub(1)..y + side + 1 {
                let pixel = shot.at(x, yy);
                if pixel == TERMINAL_ORANGE || pixel == MARK_WHITE {
                    terminal += 1;
                }
                if pixel == BROWSER_BLUE || pixel == MARK_WHITE {
                    browser += 1;
                }
                // The first row is selected, its fill the accent already:
                // only the rows below count the tile's own fill.
                if row > 0 && pixel == accent {
                    tile += 1;
                }
            }
        }
    }
    assert!(
        terminal >= side * side / 2,
        "{name}: {terminal} terminal PNG icon pixels for one {side}px image row"
    );
    assert!(
        browser >= side * side / 8,
        "{name}: {browser} browser SVG icon pixels for one {side}px image row"
    );
    assert!(
        tile >= side * side / 2,
        "{name}: {tile} accent tile pixels for one {side}px tile row"
    );
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

/// `color` at `alpha` over the empty headless output (black): the same
/// premultiplication `Color::argb8888` writes, so an exact match is
/// expected (the test keeps a small tolerance for the compositor's own
/// rounding).
fn blend_over_black(color: [u8; 3], alpha: f64) -> [u8; 3] {
    let byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    color.map(|channel| ((u32::from(channel) * byte + 127) / 255).min(255) as u8)
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: i32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (*x as i32 - *y as i32).abs() <= tolerance)
}
