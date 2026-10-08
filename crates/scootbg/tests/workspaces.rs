//! One wallpaper per workspace end to end: `set --workspace` against a
//! real `scoot --headless`, switching workspaces through scoot's IPC,
//! checked by real pixels (scoot's own screenshot) and by `query`.
//!
//! Colors throughout (no image files, no decoding): the switching,
//! mapping and restore paths are what is under test, and colors draw
//! through the same reconcile as images do.
//!
//! Workspaces need windows: an output with no windows has a single
//! (empty) workspace, and there is nothing to switch to. Each test opens
//! one `foot` per output (skipped where `foot` is not installed) and
//! kills them at the end.

mod common;

use common::{PATIENCE, Session, wait_exit};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const BASE: &str = "#c03020";
const SECOND: &str = "#101014";
const SECOND_OTHER: &str = "#202020";

/// Every output listed and configured.
fn configured(session: &Session, count: usize) -> Vec<Value> {
    session.query_until("all configured", |o| {
        o.len() == count
            && o.iter()
                .all(|o| o["surface"]["state"] == "configured" && o["surface"]["size"].is_object())
    })
}

/// `scootbg ARGS`, asserting it succeeds silently.
fn ok(session: &Session, args: &[&str]) {
    let out = session.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "{args:?} printed {:?}", out.stdout);
}

/// The active workspace per output, by name, from `query`.
fn actives(session: &Session) -> Vec<(String, Value)> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["name"].as_str().unwrap().to_owned(),
                o["workspace"].clone(),
            )
        })
        .collect()
}

/// scoot's outputs as `(id, name)`, for screenshots and switches by name.
fn scoot_ids(session: &Session) -> Vec<(u64, String)> {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["id"].as_u64().unwrap(),
                o["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Every pixel of the screenshot of `output` is `hex`, within
/// [`PATIENCE`]: a switch commits asynchronously, so the first frames may
/// still show the old wallpaper.
fn assert_screenshot(session: &Session, output: u64, hex: &str) {
    let want = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
    let (r, g, b) = (
        ((want >> 16) & 0xff) as u8,
        ((want >> 8) & 0xff) as u8,
        (want & 0xff) as u8,
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.scoot_screenshot(output);
        if !shot.pixels().is_empty() && shot.pixels().iter().all(|p| *p == [r, g, b]) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "output {output} never became all {hex}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Switches every output to workspace `index` (0-based) until scootbg
/// reports every output there. Per output: workspaces are per-output
/// lists on scoot, and switching one never disturbs another's. A switch
/// to a workspace that does not exist yet (a spawned window still
/// mapping) does nothing, and the next try follows. Fails after
/// [`PATIENCE`].
fn switch_to(session: &Session, ids: &[(u64, String)], index: usize, name: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        for (id, _) in ids {
            session.scoot_msg(&[
                "action",
                "focus-workspace-index",
                &index.to_string(),
                "--output",
                &id.to_string(),
            ]);
        }
        // Only the targeted outputs: the others keep whatever they have.
        let wanted: Vec<&String> = ids.iter().map(|(_, name)| name).collect();
        let current = actives(session);
        if current
            .iter()
            .filter(|(name, _)| wanted.contains(&name))
            .all(|(_, w)| w == &json!(name))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never {wanted:?} all on {name:?}; last: {current:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One window on each of the first `count` outputs, kept open: each
/// makes its workspace non-empty, so a trailing empty workspace appears
/// beside it to switch to. Returns the children, killed at the end
/// (closing the last window collapses the workspaces again). `None`
/// where `foot` is not installed: the test skips.
fn open_windows(session: &Session, count: usize) -> Option<Vec<std::process::Child>> {
    use std::process::{Command, Stdio};
    if Command::new("foot")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skipped -- no foot to open windows with");
        return None;
    }
    let display = session.wayland_display.clone();
    let runtime = session.runtime_dir().to_owned();
    let mut children = Vec::new();
    for (index, _) in scoot_ids(session).iter().take(count).enumerate() {
        session.scoot_msg(&["action", "focus-output-index", &index.to_string()]);
        children.push(
            Command::new("foot")
                .env("WAYLAND_DISPLAY", &display)
                .env("XDG_RUNTIME_DIR", &runtime)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("foot starts"),
        );
    }
    Some(children)
}

/// Kills the opened windows.
fn close_windows(children: &mut Vec<std::process::Child>) {
    for child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// What each output shows, by name, from `query`.
fn shows(session: &Session) -> Vec<(String, Value)> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["name"].as_str().unwrap().to_owned(), o["shows"].clone()))
        .collect()
}

/// A wallpaper per workspace follows the active one, per output, on
/// headless scoot.
///
/// The windows stay on workspace 1 throughout, so workspace 2 is empty:
/// full-screen pixels are asserted there (the switched-to wallpaper),
/// and `query`'s `shows` on workspace 1 (a window covers parts of it).
#[test]
fn workspace_wallpapers_follow_the_active_workspace_on_scoot() {
    let Some(session) = Session::start_with("perws", 2, "") else {
        return;
    };
    let Some(mut windows) = open_windows(&session, 1) else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    let ids = scoot_ids(&session);
    assert_eq!(ids.len(), 2);

    // The base everywhere, then one workspace mapped everywhere and a
    // different one for the second output alone.
    ok(&session, &["set", BASE]);
    ok(&session, &["set", SECOND, "--workspace", "2"]);
    let second = &ids[1].1;
    ok(
        &session,
        &["set", SECOND_OTHER, "--workspace", "2", "--output", second],
    );

    // `query` lists the mappings, and every output starts on workspace 1.
    let reply = session.query();
    let mappings = reply["workspaces"].as_array().unwrap();
    assert_eq!(mappings.len(), 2, "{reply}");
    switch_to(&session, &ids[..1], 0, "1");
    for (name, shown) in shows(&session) {
        assert_eq!(shown, json!({"color": BASE}), "{name}");
    }

    // To workspace 2 on the first output: it shows the global mapping.
    // The second output has a single workspace and never leaves it: it
    // keeps the base, undisturbed, on real pixels.
    switch_to(&session, &ids[..1], 1, "2");
    assert_screenshot(&session, ids[0].0, SECOND);
    assert_screenshot(&session, ids[1].0, BASE);
    assert_eq!(actives(&session)[1].1, json!("1"));

    // Back to 1: the base again, on a real switch, not a re-set.
    switch_to(&session, &ids[..1], 0, "1");
    for (name, shown) in shows(&session) {
        assert_eq!(shown, json!({"color": BASE}), "{name}");
    }

    // Clearing the global mapping falls back to the base.
    ok(&session, &["clear", "--workspace", "2"]);
    switch_to(&session, &ids[..1], 1, "2");
    assert_screenshot(&session, ids[0].0, BASE);

    // Clearing what is not mapped is an error that changes nothing.
    let out = session.run(&["clear", "--workspace", "2"]);
    assert!(!out.status.success());

    // The per-output mapping, made effective: back to workspace 1
    // (the window visible and focused), move it to the second output
    // (giving that output a second workspace), switch it there, and the
    // mapping shows on real pixels. The first output is windowless then,
    // and shows the base.
    switch_to(&session, &ids[..1], 0, "1");
    session.scoot_msg(&["action", "move-window-to-output", &ids[1].0.to_string()]);
    switch_to(&session, &ids[1..], 1, "2");
    assert_screenshot(&session, ids[1].0, SECOND_OTHER);
    assert_screenshot(&session, ids[0].0, BASE);

    close_windows(&mut windows);
    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}

/// Mappings survive a daemon restart: they are saved like any choice.
#[test]
fn workspace_mappings_restore() {
    let Some(session) = Session::start_with("perws-restore", 1, "") else {
        return;
    };
    let Some(mut windows) = open_windows(&session, 1) else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 1);
    ok(&session, &["set", BASE]);
    ok(&session, &["set", SECOND, "--workspace", "2"]);
    switch_to(&session, &scoot_ids(&session), 0, "1");

    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
    let mut daemon = session.daemon();
    configured(&session, 1);

    // The mapping is back, listed and effective: switching shows it.
    let reply = session.query();
    assert_eq!(reply["workspaces"].as_array().unwrap().len(), 1);
    let ids = scoot_ids(&session);
    switch_to(&session, &ids, 1, "2");
    assert_screenshot(&session, ids[0].0, SECOND);

    close_windows(&mut windows);
    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}
