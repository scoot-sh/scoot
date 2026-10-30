//! The panel's shadow, motion and sliding highlight on a headless scoot:
//! where the shadow sits and what a press on it does, what the entrance
//! and the fade cost the exit, and that a panel at rest asks for nothing.
//!
//! Each test writes a look fragment (`launch.toml`) before the open, since
//! the panel reads its look at open, and drives the real `scootlaunch`.
//! Frame requests and commits are read off the client's `WAYLAND_DEBUG`
//! log: what is counted is what went on the wire.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::time::{Duration, Instant};

use harness::{Panel, desktop, rgb, session_with_apps, session_with_stubs, wait_for, wait_marker};

/// The default look's border (no color in the fragment), which finds the
/// panel in a screenshot.
const BORDER: &str = "#f9e2af";

/// A panel on a fresh session with `fragment` as its look layer.
fn start(tag: &str, fragment: &str) -> Option<Panel> {
    let bodies = [
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
        ("gamma.desktop", desktop("Gamma", "/usr/bin/gamma")),
    ];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let (session, data_home) = session_with_apps(tag, &refs)?;
    write_look(&session, fragment);
    Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
}

fn write_look(session: &scootui::test_support::Session, fragment: &str) {
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), fragment).unwrap();
}

/// How many requests of `name` (`commit`, `frame`) the client has logged.
fn requests(panel: &Panel, name: &str) -> usize {
    panel
        .stderr()
        .lines()
        .filter(|line| line.contains("-> wl_surface@") && line.contains(&format!(".{name}(")))
        .count()
}

/// Waits until the surface has been drawn: the first two commits (the
/// empty one, then the first frame).
fn wait_drawn(panel: &Panel) {
    wait_for("the first frame", Duration::from_secs(10), || {
        requests(panel, "commit") >= 2
    });
}

fn bounds(panel: &Panel) -> (u32, u32, u32, u32) {
    panel.find_panel(rgb(BORDER))
}

const SOFT: &str = "shadow = \"soft\"\n";
const GLOW: &str = "shadow = \"glow\"\n";
const MOVING: &str = "shadow = \"soft\"\nmotion = \"ease-out\"\nmotion-duration-ms = 300\n";

#[test]
fn a_shadow_leaves_the_panel_where_it_was() {
    let Some(mut plain) = start("fx-plain", "") else {
        return;
    };
    plain.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    let before = bounds(&plain);
    plain.key("Escape");
    plain.wait_exit();
    for (tag, fragment) in [("fx-soft", SOFT), ("fx-deep", "shadow = \"deep\"\n")] {
        let Some(mut shadowed) = start(tag, fragment) else {
            return;
        };
        shadowed.wait_shown(rgb(BORDER));
        std::thread::sleep(Duration::from_millis(300));
        shadowed.save(tag);
        assert_eq!(bounds(&shadowed), before, "{tag}: the panel did not move");
        shadowed.key("Escape");
        shadowed.wait_exit();
    }
}

#[test]
fn a_glow_paints_its_color_around_the_panel_and_not_on_it() {
    let Some(mut panel) = start("fx-glow", GLOW) else {
        return;
    };
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    panel.save("fx-glow");
    let (x0, y0, x1, y1) = bounds(&panel);
    let shot = panel.shot();
    let mid = (x0 + x1) / 2;
    // Just under the panel: the glow's tint (yellowish), not the black
    // output; far under it: nothing.
    let backdrop = shot.at(5, 5);
    let near = shot.at(mid, y1 + 4);
    assert!(
        near[0] > backdrop[0] + 20 && near[0] > near[2],
        "a warm glow under the panel: {near:?} over {backdrop:?}"
    );
    assert_eq!(shot.at(mid, y1 + 60), backdrop, "and none far away");
    assert_eq!(shot.at(x0 - 60, (y0 + y1) / 2), backdrop);
    // The glow does not change the panel's own border.
    assert_eq!(shot.at(mid, y1), rgb(BORDER));
    panel.key("Escape");
    panel.wait_exit();
}

#[test]
fn a_press_on_the_shadow_dismisses_and_a_press_on_the_panel_does_not() {
    let Some(mut panel) = start("fx-press", SOFT) else {
        return;
    };
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    let (x0, y0, _, y1) = bounds(&panel);
    // The panel's own padding: a press there is a press on the panel.
    panel.click(x0 + 20, y1 - 8);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "a press on the panel keeps it open"
    );
    // 6 px outside the border is inside the soft shadow's 12 px room.
    panel.click(x0 - 6, (y0 + y1) / 2);
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "a press on the shadow dismisses");
    assert_eq!(panel.stdout(), "", "and launches nothing");
    panel.clean();
}

#[test]
fn a_press_just_below_the_shadow_dismisses_too() {
    let Some(mut panel) = start("fx-below", SOFT) else {
        return;
    };
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    let (x0, _, x1, y1) = bounds(&panel);
    panel.click((x0 + x1) / 2, y1 + 10);
    assert_eq!(panel.wait_exit().code(), Some(1));
    panel.clean();
}

#[test]
fn a_panel_at_rest_asks_for_nothing_with_every_effect_on() {
    let fragment =
        "shadow = \"glow\"\nfill = \"gradient\"\nmotion = \"spring\"\nmotion-duration-ms = 200\n";
    let Some(mut panel) = start("fx-idle", fragment) else {
        return;
    };
    wait_drawn(&panel);
    // Past the entrance and the settling redraws.
    std::thread::sleep(Duration::from_millis(1500));
    let (commits, frames) = (requests(&panel, "commit"), requests(&panel, "frame"));
    assert!(frames > 0, "the entrance asked for frames");
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(requests(&panel, "commit"), commits, "commits at rest");
    assert_eq!(requests(&panel, "frame"), frames, "frame requests at rest");
    eprintln!("fx-idle: {commits} commits, {frames} frame requests to open, then none for 3 s");
    panel.key("Escape");
    panel.wait_exit();
}

#[test]
fn a_shadow_alone_never_asks_for_a_frame() {
    let Some(mut panel) = start("fx-shadow-frames", SOFT) else {
        return;
    };
    wait_drawn(&panel);
    std::thread::sleep(Duration::from_millis(800));
    panel.type_text("a");
    panel.key("Down");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(
        requests(&panel, "frame"),
        0,
        "no motion, no frame callbacks"
    );
    panel.key("Escape");
    panel.wait_exit();
}

#[test]
fn the_entrance_settles_on_the_frame_a_still_look_draws() {
    let shoot = |tag: &str, fragment: &str| -> Option<Vec<[u8; 3]>> {
        let mut panel = start(tag, fragment)?;
        wait_drawn(&panel);
        std::thread::sleep(Duration::from_millis(1200));
        let shot = panel.shot();
        let pixels = (0..shot.height)
            .flat_map(|y| (0..shot.width).map(move |x| (x, y)))
            .map(|(x, y)| shot.at(x, y))
            .collect();
        panel.key("Escape");
        panel.wait_exit();
        Some(pixels)
    };
    let Some(still) = shoot("fx-still", SOFT) else {
        return;
    };
    let Some(moved) = shoot("fx-moved", MOVING) else {
        return;
    };
    assert!(still == moved, "the settled entrance is the still frame");
}

#[test]
fn escape_fades_out_before_it_exits_and_an_outside_click_does_not_wait() {
    // A fade of the longest length the look allows, so "did not wait" is
    // far from "waited" on any machine.
    let fragment = "motion = \"ease-out\"\nmotion-duration-ms = 1000\n";
    let Some(mut panel) = start("fx-escape", fragment) else {
        return;
    };
    wait_drawn(&panel);
    std::thread::sleep(Duration::from_millis(1200));
    let sent = Instant::now();
    panel.key("Escape");
    let status = panel.wait_exit();
    let took = sent.elapsed();
    eprintln!("escape with a 1000 ms fade: exited after {took:?}");
    assert_eq!(status.code(), Some(1));
    assert!(
        took >= Duration::from_millis(800),
        "the fade plays before the exit: {took:?}"
    );
    assert!(
        took < Duration::from_millis(3000),
        "and is bounded: {took:?}"
    );
    panel.clean();

    // A click outside is the compositor closing the surface: no fade.
    let Some(mut panel) = start("fx-outside", fragment) else {
        return;
    };
    wait_drawn(&panel);
    std::thread::sleep(Duration::from_millis(1200));
    let sent = Instant::now();
    panel.click(10, 10);
    let status = panel.wait_exit();
    let took = sent.elapsed();
    eprintln!("outside click with a 1000 ms fade: exited after {took:?}");
    assert_eq!(status.code(), Some(1));
    assert!(
        took < Duration::from_millis(900),
        "an outside click does not wait for a fade: {took:?}"
    );
    panel.clean();
}

#[test]
fn a_launch_does_not_wait_for_the_fade() {
    let Some((session, data_home, markers)) = session_with_stubs("fx-launch", &["alpha"]) else {
        return;
    };
    write_look(
        &session,
        "motion = \"ease-out\"\nmotion-duration-ms = 1000\n",
    );
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    wait_drawn(&panel);
    std::thread::sleep(Duration::from_millis(1200));
    let sent = Instant::now();
    panel.key("Return");
    let launched = wait_marker(&markers[0]);
    assert!(launched.contains("alpha.sh"), "{launched}");
    // The stub has run, and the panel is still fading: it has not exited.
    // (The fade is the longest a look allows, so the stub is far ahead of
    // it on any machine.)
    eprintln!("the launch ran {:?} after the key", sent.elapsed());
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "the panel is still playing its fade when the launch has run"
    );
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0 on a launch: {}", panel.stderr());
    assert!(
        sent.elapsed() >= Duration::from_millis(800),
        "and the panel played its fade: {:?}",
        sent.elapsed()
    );
    panel.clean();
}

#[test]
fn typing_works_from_the_first_frame_of_the_entrance() {
    let Some((session, data_home, markers)) = session_with_stubs("fx-type", &["alpha", "beta"])
    else {
        return;
    };
    write_look(
        &session,
        "motion = \"ease-out\"\nmotion-duration-ms = 1000\n",
    );
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    let opened = Instant::now();
    wait_drawn(&panel);
    // The entrance has barely begun (it runs a full second): type and pick
    // straight away.
    panel.type_text("bet");
    panel.key("Return");
    let launched = wait_marker(&markers[1]);
    assert!(launched.contains("beta.sh"), "{launched}");
    eprintln!("typed and launched {:?} after the open", opened.elapsed());
    assert!(
        opened.elapsed() < Duration::from_millis(900),
        "the pick did not wait for the entrance: {:?}",
        opened.elapsed()
    );
    panel.wait_exit();
    panel.clean();
}

#[test]
fn moving_the_selection_asks_for_frames_until_the_highlight_lands() {
    let Some(mut panel) = start("fx-slide", MOVING) else {
        return;
    };
    wait_drawn(&panel);
    std::thread::sleep(Duration::from_millis(1200));
    let before = requests(&panel, "frame");
    panel.key("Down");
    std::thread::sleep(Duration::from_millis(1000));
    let after = requests(&panel, "frame");
    assert!(after > before, "the slide asked for frames");
    let commits = requests(&panel, "commit");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(requests(&panel, "frame"), after, "none once it landed");
    assert_eq!(requests(&panel, "commit"), commits, "and no commits");
    panel.key("Escape");
    panel.wait_exit();
}

#[test]
fn a_toggle_during_the_fade_opens_a_fresh_panel() {
    let fragment = "motion = \"ease-out\"\nmotion-duration-ms = 900\n";
    let bodies = [("alpha.desktop", desktop("Alpha", "/usr/bin/alpha"))];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) = session_with_apps("fx-toggle", &refs) else {
        return;
    };
    write_look(&session, fragment);
    let Some(mut first) = Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    wait_drawn(&first);
    std::thread::sleep(Duration::from_millis(1200));
    first.key("Escape");
    // The fade has begun (its claim is gone): a second invocation opens
    // a panel of its own instead of signalling the leaving one.
    std::thread::sleep(Duration::from_millis(250));
    let mut command = std::process::Command::new(harness::launch_bin());
    command
        .env("XDG_RUNTIME_DIR", &first.session.scratch.0)
        .env("WAYLAND_DISPLAY", &first.session.wayland_display)
        .env_remove("WAYLAND_DEBUG")
        .arg("--font")
        .arg(first.session.font())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    first.session.scratch.sandbox(&mut command);
    command
        .env("XDG_DATA_HOME", &first.data_home)
        .env("XDG_DATA_DIRS", first.session.scratch.0.join("empty-dirs"));
    let mut second = command.spawn().unwrap();
    first.wait_exit();
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        second.try_wait().unwrap().is_none(),
        "the second panel opened and stays open"
    );
    second.kill().ok();
    second.wait().ok();
}

#[test]
fn a_click_on_a_row_still_lands_on_that_row_inside_a_shadow() {
    // The pointer arrives in surface coordinates, the shadow's room
    // included: the row under it is found from the panel's own origin.
    let Some((session, data_home, markers)) =
        session_with_stubs("fx-row-click", &["alpha", "beta", "gamma"])
    else {
        return;
    };
    write_look(&session, SOFT);
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    panel.wait_shown(rgb(BORDER));
    std::thread::sleep(Duration::from_millis(300));
    // The default look: 13 px text, radius 8.
    let g = harness::Geom::new(13.0, 8);
    let (x0, y0, x1, _) = bounds(&panel);
    panel.click((x0 + x1) / 2, y0 + g.list_y() + g.row_h + g.row_h / 2);
    let status = panel.wait_exit();
    assert!(status.success(), "{}", panel.stderr());
    let launched = wait_marker(&markers[1]);
    assert!(launched.contains("beta.sh"), "the second row: {launched}");
    panel.clean();
}

#[test]
fn escape_during_the_entrance_fades_from_where_it_is_and_exits() {
    let fragment = "motion = \"ease-out\"\nmotion-duration-ms = 900\n";
    let Some(mut panel) = start("fx-early-escape", fragment) else {
        return;
    };
    wait_drawn(&panel);
    // The first frame is on the wire: the entrance has barely begun.
    let sent = Instant::now();
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1));
    assert!(
        sent.elapsed() < Duration::from_millis(2500),
        "bounded: {:?}",
        sent.elapsed()
    );
    panel.clean();
}
