//! Windows mode (`scootlaunch --windows`) on a headless scoot and on a
//! headless sway: three windows are listed, and the chosen one is focused.
//!
//! Both compositors serve `wlr-foreign-toplevel-management`, the protocol
//! the mode lists and activates through. The windows are mapped by a toy
//! `xdg_toplevel` client ([`toy`]); on scoot keys go through the IPC `key`
//! request and focus is read from `windows`, on sway they go through a
//! virtual keyboard ([`vkeyboard`]) and focus is read from `get_tree`.
//! Skipped without a `scoot` (or `sway`) on the test machine;
//! `SCOOTLAUNCH_REQUIRE_SCOOT` and `SCOOTLAUNCH_REQUIRE_SWAY` make that a
//! failure, so CI cannot go green by skipping.

#[path = "harness.rs"]
mod harness;
#[path = "toy/mod.rs"]
mod toy;
#[path = "vkeyboard/mod.rs"]
mod vkeyboard;

use std::fs;
use std::time::Duration;

use harness::{
    BROWSER_BLUE, Geom, Panel, TERMINAL_ORANGE, desktop_with_icon, fixture_icon_theme, launch_bin,
    render_launch_fragment, rgb, session_with_apps, wait_for,
};
use scootui::test_support::Session;
use toy::Toy;

/// The default look's focus color: the panel's border with no look layer.
const BORDER: &str = "#f9e2af";

/// (app_id, title) of the three windows, in the order they are mapped.
/// Titles sort alphabetically, which is the empty-query order.
const WINDOWS: [(&str, &str); 3] = [
    ("alpha", "Alpha Doc"),
    ("Beta-Class", "Beta Doc"),
    ("gamma", "Gamma Doc"),
];

fn scoot_windows(session: &Session) -> Vec<serde_json::Value> {
    session.scoot_ipc(r#"{"type":"windows"}"#)["windows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The title of the window scoot reports focused, if any.
fn scoot_focused(session: &Session) -> Option<String> {
    scoot_windows(session)
        .iter()
        .find(|window| window["focused"] == true)
        .map(|window| window["title"].as_str().unwrap_or("").to_string())
}

/// Maps the three windows one at a time, so the last one mapped (Gamma)
/// is the focused one.
fn map_three(session: &Session, mut count: impl FnMut(&Session) -> usize) -> Vec<Toy> {
    let mut toys = Vec::new();
    for (index, (app_id, title)) in WINDOWS.iter().enumerate() {
        toys.push(Toy::open(session.wayland_socket(), app_id, title));
        wait_for("a window to map", Duration::from_secs(10), || {
            count(session) > index
        });
    }
    toys
}

/// The harness panel, pulled apart so the session can serve the next run.
fn into_session(panel: Panel) -> (Session, std::path::PathBuf) {
    let Panel {
        session, data_home, ..
    } = panel;
    (session, data_home)
}

#[test]
fn each_of_three_windows_is_listed_and_the_chosen_one_is_focused_on_scoot() {
    let Some((mut session, mut data_home)) = session_with_apps("win-scoot", &[]) else {
        return;
    };
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    assert_eq!(scoot_focused(&session).as_deref(), Some("Gamma Doc"));
    // Down N rows from the top picks that window: Beta (1), Gamma (2),
    // Alpha (0). Each run starts from a different focus, so a pick that
    // did nothing could not pass.
    for (downs, expect) in [(1, "Beta Doc"), (2, "Gamma Doc"), (0, "Alpha Doc")] {
        let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
        panel.wait_shown(rgb(BORDER));
        std::thread::sleep(Duration::from_millis(300));
        assert_ne!(scoot_focused(&panel.session).as_deref(), Some(expect));
        for _ in 0..downs {
            panel.key("Down");
        }
        panel.key("Return");
        let status = panel.wait_exit();
        assert_eq!(status.code(), Some(0), "{}", panel.stderr());
        assert_eq!(panel.stdout(), "");
        wait_for(
            &format!("{expect} to be focused"),
            Duration::from_secs(10),
            || scoot_focused(&panel.session).as_deref() == Some(expect),
        );
        panel.clean();
        (session, data_home) = into_session(panel);
    }
}

#[test]
fn typing_filters_by_title_and_by_app_id() {
    let Some((mut session, mut data_home)) = session_with_apps("win-filter", &[]) else {
        return;
    };
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    // The app id is searchable: "Beta-Class" is nowhere in the title.
    // Then the title: "alpha d" matches only Alpha Doc.
    for (query, expect) in [("beta-class", "Beta Doc"), ("alpha d", "Alpha Doc")] {
        let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
        panel.wait_shown(rgb(BORDER));
        std::thread::sleep(Duration::from_millis(300));
        panel.type_text(query);
        std::thread::sleep(Duration::from_millis(300));
        panel.key("Return");
        assert_eq!(panel.wait_exit().code(), Some(0), "{}", panel.stderr());
        wait_for(
            &format!("{expect} to be focused"),
            Duration::from_secs(10),
            || scoot_focused(&panel.session).as_deref() == Some(expect),
        );
        (session, data_home) = into_session(panel);
    }
}

#[test]
fn with_a_motion_set_the_window_is_activated_after_the_fade() {
    let Some((session, data_home)) = session_with_apps("win-motion", &[]) else {
        return;
    };
    // A spring on the panel (the user's own file): the pick fades the
    // panel out first, then activates.
    let config = session.scratch.config_home().join("scoot");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("launch.toml"),
        "[effects]\nmotion = \"spring\"\nmotion-duration-ms = 200\n",
    )
    .unwrap();
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(500));
    panel.key("Down");
    let picked = std::time::Instant::now();
    panel.key("Return");
    assert_eq!(panel.wait_exit().code(), Some(0), "{}", panel.stderr());
    // The fade ran before the exit: it takes the 200 ms the file asks for
    // (frames arrive at the output's rate), where no motion exits at once.
    assert!(
        picked.elapsed() >= Duration::from_millis(150),
        "the panel exited after {:?}, before its fade could have run",
        picked.elapsed()
    );
    wait_for("Beta Doc to be focused", Duration::from_secs(10), || {
        scoot_focused(&panel.session).as_deref() == Some("Beta Doc")
    });
    panel.clean();
}

#[test]
fn escape_activates_nothing() {
    let Some((session, data_home)) = session_with_apps("win-escape", &[]) else {
        return;
    };
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Down");
    panel.key("Escape");
    assert_eq!(panel.wait_exit().code(), Some(1), "{}", panel.stderr());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(scoot_focused(&panel.session).as_deref(), Some("Gamma Doc"));
}

#[test]
fn a_click_on_a_row_activates_that_window() {
    let Some((session, data_home)) = session_with_apps("win-click", &[]) else {
        return;
    };
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    let (x0, y0, x1, _) = panel.find_panel(rgb(BORDER));
    let g = Geom::new(13.0, 8);
    // The first row is Alpha Doc.
    panel.click((x0 + x1) / 2, y0 + g.list_y() + g.row_h / 2);
    assert_eq!(panel.wait_exit().code(), Some(0), "{}", panel.stderr());
    wait_for("Alpha Doc to be focused", Duration::from_secs(10), || {
        scoot_focused(&panel.session).as_deref() == Some("Alpha Doc")
    });
}

#[test]
fn a_window_that_closed_while_the_panel_was_open_is_refused_not_activated() {
    let Some((session, data_home)) = session_with_apps("win-closed", &[]) else {
        return;
    };
    let mut toys = map_three(&session, |session| scoot_windows(session).len());
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    // Beta (the middle toy) goes away under the open panel.
    toys.remove(1).close();
    wait_for("Beta to close", Duration::from_secs(10), || {
        scoot_windows(&panel.session).len() == 2
    });
    panel.key("Down");
    panel.key("Return");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "{}", panel.stderr());
    let log = panel.stderr();
    assert!(log.contains("closed"), "{log}");
    assert!(!log.contains("protocol"), "{log}");
    // Nothing was activated: Gamma still has focus.
    assert_eq!(scoot_focused(&panel.session).as_deref(), Some("Gamma Doc"));
}

#[test]
fn with_no_windows_the_panel_lists_nothing_and_enter_does_nothing() {
    let Some((session, data_home)) = session_with_apps("win-none", &[]) else {
        return;
    };
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "Enter on an empty list closed the panel: {}",
        panel.stderr()
    );
    panel.key("Escape");
    assert_eq!(panel.wait_exit().code(), Some(1));
}

#[test]
fn ctrl_enter_runs_nothing() {
    let Some((session, data_home)) = session_with_apps("win-ctrl", &[]) else {
        return;
    };
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    let marker = session.scratch.0.join("ran");
    let mut panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    // In apps mode this would run the typed text as a command.
    panel.type_text(&format!("touch {}", marker.display()));
    panel.key("ctrl+Return");
    std::thread::sleep(Duration::from_millis(600));
    assert!(!marker.exists(), "Ctrl+Enter ran the typed text");
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "the panel closed: {}",
        panel.stderr()
    );
    panel.key("Escape");
    assert_eq!(panel.wait_exit().code(), Some(1));
}

#[test]
fn rows_draw_the_icon_of_the_desktop_entry_their_app_id_names() {
    let bodies = [
        (
            "alpha.desktop",
            desktop_with_icon("Alpha App", "/usr/bin/alpha", "alpha"),
        ),
        (
            "betaapp.desktop",
            format!(
                "{}StartupWMClass=Beta-Class\n",
                desktop_with_icon("Beta App", "/usr/bin/beta", "beta")
            ),
        ),
    ];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) = session_with_apps("win-icons", &refs) else {
        return;
    };
    let theme = fixture_icon_theme(&data_home);
    let fragment = render_launch_fragment("moonrise");
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
    let _toys = map_three(&session, |session| scoot_windows(session).len());
    let panel = Panel::start_on(session, data_home, &["--windows"], &[]).unwrap();
    let table: toml::Table = fragment.parse().unwrap();
    let colors = table["colors"].as_table().unwrap();
    let color = |key: &str| rgb(colors[key].as_str().unwrap());
    let (focus, accent) = (color("focus"), color("accent"));
    let em = table["font-size"].as_integer().unwrap() as f32;
    panel.wait_shown(focus);
    std::thread::sleep(Duration::from_millis(500));
    let shot = panel.shot();
    let (x0, y0, _, _) = panel.find_panel(focus);
    let radius = table["radius"].as_integer().unwrap() as u32;
    let g = Geom::new(em, radius);
    let icon_x = x0 + g.inner + g.lead;
    let list_y = y0 + g.list_y();
    // Row 0 Alpha Doc: its app id `alpha` is a desktop id, with the
    // terminal PNG. Row 1 Beta Doc: `Beta-Class` is only a
    // StartupWMClass, with the browser SVG. Row 2 Gamma Doc: no entry,
    // and the theme has no `gamma`, so the letter tile in the accent.
    let count = |row: u32, wanted: &dyn Fn([u8; 3]) -> bool| {
        let y = list_y + row * g.row_h + g.row_h.saturating_sub(g.side) / 2;
        let mut n = 0u32;
        for x in icon_x.saturating_sub(1)..icon_x + g.side + 1 {
            for yy in y.saturating_sub(1)..y + g.side + 1 {
                if wanted(shot.at(x, yy)) {
                    n += 1;
                }
            }
        }
        n
    };
    let terminal = count(0, &|p| p == TERMINAL_ORANGE);
    let browser = count(1, &|p| p == BROWSER_BLUE);
    let tile = count(2, &|p| p == accent);
    assert!(
        terminal >= g.side * g.side / 2,
        "{terminal} terminal PNG pixels in the Alpha row"
    );
    assert!(
        browser >= g.side * g.side / 8,
        "{browser} browser SVG pixels in the Beta row"
    );
    assert!(
        tile >= g.side * g.side / 2,
        "{tile} accent tile pixels in the Gamma row"
    );
    // And the rows do not share icons: Beta's row has no terminal
    // orange, Alpha's no browser blue.
    assert_eq!(count(1, &|p| p == TERMINAL_ORANGE), 0);
    assert_eq!(count(0, &|p| p == BROWSER_BLUE), 0);
    panel.key("Escape");
}

/// The other modes never bind the toplevel manager, so a launcher opened
/// to pick an app is sent no window event at all: in the protocol trace
/// of the wire (`WAYLAND_DEBUG`) a request (`->`) names it in windows mode
/// and only there. (The compositor's `global` advertisement, `<-`, names
/// it in every mode.)
#[test]
fn only_windows_mode_binds_the_toplevel_manager() {
    let trace = |args: &[&str], tag: &str| {
        let (session, data_home) = session_with_apps(tag, &[])?;
        let _toys = map_three(&session, |session| scoot_windows(session).len());
        let mut panel =
            Panel::start_on(session, data_home, args, &[("WAYLAND_DEBUG", "1")]).unwrap();
        panel.wait_shown(rgb(BORDER));
        std::thread::sleep(Duration::from_millis(300));
        panel.key("Escape");
        assert_eq!(panel.wait_exit().code(), Some(1));
        Some(panel.stderr())
    };
    let sent = |log: &str| {
        log.lines()
            .filter(|line| line.contains("-> ") && line.contains("foreign_toplevel"))
            .count()
    };
    let (Some(windows), Some(apps), Some(run)) = (
        trace(&["--windows"], "win-trace-windows"),
        trace(&[], "win-trace-apps"),
        trace(&["--run"], "win-trace-run"),
    ) else {
        return;
    };
    assert!(
        sent(&windows) > 0,
        "windows mode never bound the manager:\n{windows}"
    );
    for (mode, log) in [("apps", &apps), ("run", &run)] {
        assert!(
            log.contains("zwlr_layer_shell_v1"),
            "{mode}: the trace shows no panel at all:\n{log}"
        );
        assert_eq!(
            sent(log),
            0,
            "{mode} mode touched the toplevel protocol:\n{log}"
        );
    }
}

// --- sway ---------------------------------------------------------------

/// The sway tree's leaf windows as `(app_id, focused)`, in tree order.
fn sway_windows(session: &Session) -> Vec<(String, bool)> {
    fn walk(node: &serde_json::Value, out: &mut Vec<(String, bool)>) {
        if let Some(app_id) = node["app_id"].as_str() {
            out.push((app_id.to_string(), node["focused"] == true));
        }
        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = node[key].as_array() {
                for child in children {
                    walk(child, out);
                }
            }
        }
    }
    let tree: serde_json::Value =
        serde_json::from_str(&session.swaymsg(&["-t", "get_tree", "-r"])).unwrap();
    let mut out = Vec::new();
    walk(&tree, &mut out);
    out
}

fn sway_focused(session: &Session) -> Option<String> {
    sway_windows(session)
        .into_iter()
        .find(|(_, focused)| *focused)
        .map(|(app_id, _)| app_id)
}

#[test]
fn each_of_three_windows_is_listed_and_the_chosen_one_is_focused_on_sway() {
    let Some(session) =
        Session::sway_requiring(&launch_bin(), "win-sway", 1, "SCOOTLAUNCH_REQUIRE_SWAY")
    else {
        return;
    };
    let output = {
        let outputs: serde_json::Value =
            serde_json::from_str(&session.swaymsg(&["-t", "get_outputs", "-r"])).unwrap();
        outputs[0]["name"].as_str().unwrap().to_string()
    };
    // Give the seat a keyboard before anything maps.
    let mut keyboard = vkeyboard::VirtualKeyboard::new(&session.wayland_socket());
    let _toys = map_three(&session, |session| sway_windows(session).len());
    assert_eq!(sway_focused(&session).as_deref(), Some("gamma"));
    let data_home = session.scratch.0.join("data-home");
    fs::create_dir_all(data_home.join("applications")).unwrap();
    fs::create_dir_all(session.scratch.0.join("empty-dirs")).unwrap();
    let mut session = session;
    for (downs, expect) in [(1, "Beta-Class"), (2, "gamma"), (0, "alpha")] {
        let mut panel = Panel::start_on(
            session,
            data_home.clone(),
            &["--windows", "--border-color", "ff00ffff"],
            &[],
        )
        .unwrap();
        // The panel is up when its border is on the output.
        wait_for("the panel shown", Duration::from_secs(10), || {
            let shot = panel.session.screencopy(&output);
            (0..shot.width).step_by(2).any(|x| {
                (0..shot.height)
                    .step_by(2)
                    .any(|y| shot.at(x, y) == [255, 0, 255])
            })
        });
        std::thread::sleep(Duration::from_millis(300));
        assert_ne!(sway_focused(&panel.session).as_deref(), Some(expect));
        for _ in 0..downs {
            keyboard.tap(vkeyboard::DOWN);
        }
        keyboard.tap(vkeyboard::RETURN);
        let status = panel.wait_exit();
        assert_eq!(status.code(), Some(0), "{}", panel.stderr());
        wait_for(
            &format!("{expect} to be focused"),
            Duration::from_secs(10),
            || sway_focused(&panel.session).as_deref() == Some(expect),
        );
        panel.clean();
        session = panel.session;
    }
}
