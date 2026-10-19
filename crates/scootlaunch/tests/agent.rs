//! The agent interface end to end: `--list`, `--query`, `--run-id`,
//! then the window in `scoot msg windows`; plus the hostile
//! ids/actions (unknown, huge, control characters, unknown action)
//! pinned to an error object and a nonzero exit, never a panic or a
//! launch of the wrong entry.
//!
//! The window leg needs `foot` on `PATH` (the dev shell carries it;
//! skipped without it, like the bar's window checks). The run legs
//! drive the real `scootlaunch` binary; the hostile legs are
//! compositor-free. Skipped without a `scoot` binary beside the test
//! binary; `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use harness::{launch_bin, session_with_apps, wait_for};

fn foot_present() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("foot").is_file()))
}

fn foot_entry() -> (&'static str, String) {
    (
        "org.example.AgentFoot.desktop",
        "[Desktop Entry]\nType=Application\nName=AgentFoot\nExec=foot\n".to_string(),
    )
}

/// One headless run of the real binary against the fixture index.
/// Stdout is the only JSON, stderr carries the notes.
///
/// Stdout and stderr go to files, not pipes: a launched app inherits
/// them, and a pipe would stay open past the launcher's exit (the
/// launcher exits milliseconds after spawning; the app outlives it),
/// hanging the wait. Answers the status plus both texts.
fn run_headless(
    session_scratch: &std::path::Path,
    data_home: &std::path::Path,
    tag: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> (std::process::ExitStatus, String, String) {
    let empty_dirs = session_scratch.join("empty-dirs");
    let home = session_scratch.join("home");
    for dir in [
        home.clone(),
        session_scratch.join("xdg-config"),
        session_scratch.join("xdg-state"),
        session_scratch.join("xdg-cache"),
    ] {
        fs::create_dir_all(&dir).unwrap();
    }
    let stdout_log = session_scratch.join(format!("{tag}.stdout"));
    let stderr_log = session_scratch.join(format!("{tag}.stderr"));
    let stdout_file = fs::File::create(&stdout_log).unwrap();
    let stderr_file = fs::File::create(&stderr_log).unwrap();
    let mut command = Command::new(launch_bin());
    command
        .env("XDG_RUNTIME_DIR", session_scratch)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", session_scratch.join("xdg-config"))
        .env("XDG_STATE_HOME", session_scratch.join("xdg-state"))
        .env("XDG_CACHE_HOME", session_scratch.join("xdg-cache"))
        .env("XDG_DATA_HOME", data_home)
        .env("XDG_DATA_DIRS", &empty_dirs)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout_file)
        .stderr(stderr_file);
    // The caller's overrides last (a dead bus, a display for the run).
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let status = command.status().expect("the binary runs");
    let stdout = fs::read_to_string(&stdout_log).unwrap_or_default();
    let stderr = fs::read_to_string(&stderr_log).unwrap_or_default();
    (status, stdout, stderr)
}

fn stdout_json(stdout: &str) -> serde_json::Value {
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout is exactly one JSON object, nothing else: {stdout:?}"
    );
    serde_json::from_str(stdout.lines().next().unwrap_or("")).expect("stdout parses as JSON")
}

/// The agent loop: list, query, run, then the window. Mirrors what an
/// agent in a VM does over SSH with `WAYLAND_DISPLAY` set.
#[test]
fn agent_lists_queries_runs_then_finds_the_window() {
    if !foot_present() {
        eprintln!("skipped -- no foot on PATH");
        return;
    }
    let bodies = [foot_entry()];
    let refs: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(file, body)| (*file, body.as_str()))
        .collect();
    let Some((session, data_home)) = session_with_apps("agent-loop", &refs) else {
        return;
    };
    let scratch = &session.scratch.0;
    // List: the entry is there with its Exec argv.
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-list",
        &["--list", "--json"],
        &[],
    );
    assert!(status.success(), "list exits 0: {status:?} {stderr:?}");
    let listed = stdout_json(&stdout);
    let entries = listed.as_array().expect("list is an array");
    let found = entries
        .iter()
        .find(|entry| entry["id"] == "org.example.AgentFoot.desktop")
        .expect("the fixture is listed");
    assert_eq!(found["name"], "AgentFoot");
    assert_eq!(found["exec"], serde_json::json!(["foot"]));
    // Query: ranked first, as the UI would rank it.
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-query",
        &["--query", "agentfoot", "--json"],
        &[],
    );
    assert!(status.success(), "query exits 0: {status:?} {stderr:?}");
    let ranked = stdout_json(&stdout);
    let first = ranked
        .as_array()
        .and_then(|entries| entries.first())
        .expect("the query matches");
    assert_eq!(first["id"], "org.example.AgentFoot.desktop");
    // Run: headless, through the panel's spawn path with no surface.
    // The child inherits the session's display, so its window maps.
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-run",
        &["--run-id", "org.example.AgentFoot.desktop"],
        &[
            ("WAYLAND_DISPLAY", &session.wayland_display),
            ("XDG_RUNTIME_DIR", scratch.to_str().unwrap()),
        ],
    );
    assert!(status.success(), "run exits 0: {status:?} {stderr:?}");
    let report = stdout_json(&stdout);
    assert_eq!(report["ok"], true, "{report}");
    // The window maps: fresh spawns take focus on map.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        let windows = reply["windows"].as_array().cloned().unwrap_or_default();
        if windows.iter().any(|window| window["app_id"] == "foot") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the launched foot never appeared in windows: {}",
            session.scoot_ipc(r#"{"type":"windows"}"#)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // Close the window again: the test leaves no client behind.
    session.scoot_ipc(r#"{"type":"action","action":"close_focused"}"#);
    wait_for("the window to close", Duration::from_secs(10), || {
        session.scoot_ipc(r#"{"type":"windows"}"#)["windows"]
            .as_array()
            .is_some_and(|windows| windows.is_empty())
    });
}

/// `--run-id` with `--action` launches the action's argv: the stub
/// records which script ran.
#[test]
fn run_id_with_action_launches_the_action() {
    let scratch_dir =
        tempfile::TempDir::with_prefix("scootlaunch-agent-action-").expect("scratch makes");
    let scratch = scratch_dir.path();
    let data_home = scratch.join("data-home");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).unwrap();
    let bins = scratch.join("bins");
    fs::create_dir_all(&bins).unwrap();
    let main_marker = scratch.join("main.launched");
    let action_marker = scratch.join("action.launched");
    let main_script = bins.join("main.sh");
    let action_script = bins.join("action.sh");
    fs::write(
        &main_script,
        format!("#!/bin/sh\necho main >> '{}'\n", main_marker.display()),
    )
    .unwrap();
    fs::write(
        &action_script,
        format!("#!/bin/sh\necho action >> '{}'\n", action_marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&main_script, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&action_script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        applications.join("act.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Act\nExec=\"{}\"\nActions=newthing;\n\n[Desktop Action newthing]\nName=New Thing\nExec=\"{}\"\n",
            main_script.display(),
            action_script.display()
        ),
    )
    .unwrap();
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-action",
        &["--run-id", "act.desktop", "--action", "newthing"],
        &[(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent-scootlaunch-bus",
        )],
    );
    assert!(
        status.success(),
        "the action launches: {status:?} {stderr:?}"
    );
    let report = stdout_json(&stdout);
    assert_eq!(report["ok"], true, "{report}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !action_marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        fs::read_to_string(&action_marker)
            .unwrap_or_default()
            .contains("action"),
        "the action's argv ran"
    );
    assert!(
        !main_marker.exists(),
        "the entry's own argv did not run instead"
    );
}

/// Hostile ids and actions: an error object and exit 1, never a panic
/// and never a launch of the wrong entry. Compositor-free.
#[test]
fn hostile_ids_and_actions_are_error_objects() {
    let scratch_dir =
        tempfile::TempDir::with_prefix("scootlaunch-agent-hostile-").expect("scratch makes");
    let scratch = scratch_dir.path();
    let data_home = scratch.join("data-home");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).unwrap();
    let marker = scratch.join("right.launched");
    fs::write(
        applications.join("right.desktop"),
        "[Desktop Entry]\nType=Application\nName=Right\nExec=/nonexistent-scootlaunch-right\n",
    )
    .unwrap();
    // Something launchable, to prove none of the hostile ids ran it.
    let bins = scratch.join("bins");
    fs::create_dir_all(&bins).unwrap();
    let script = bins.join("right.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        applications.join("launchable.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Launchable\nExec=\"{}\"\n",
            script.display()
        ),
    )
    .unwrap();
    let dead_bus = [(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/nonexistent-scootlaunch-bus",
    )];
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["--run-id", "missing.desktop"], "unknown-id"),
        (vec!["--run-id", "righ"], "unknown-id"),
        (vec!["--run-id", ""], "invalid-id"),
        (vec!["--run-id", "a\nb.desktop"], "invalid-id"),
        // A NUL byte cannot cross the exec boundary at all (the OS
        // refuses it before main runs), so it is pinned in the
        // resolver's unit tests instead of here.
        (
            vec!["--run-id", "launchable.desktop", "--action", "nope"],
            "unknown-action",
        ),
        (
            vec!["--run-id", "launchable.desktop", "--action", ""],
            "invalid-action",
        ),
    ];
    for (n, (args, code)) in cases.iter().enumerate() {
        let tag = format!("agent-hostile-{n}");
        let (status, stdout, stderr) = run_headless(scratch, &data_home, &tag, args, &dead_bus);
        assert_eq!(status.code(), Some(1), "{args:?}: {status:?} {stderr:?}");
        let error = stdout_json(&stdout);
        assert_eq!(error["ok"], false, "{args:?}: {error}");
        assert_eq!(error["code"], *code, "{args:?}: {error}");
        assert!(
            !error["error"].as_str().unwrap_or("").contains('\n'),
            "no raw newline in the object: {error}"
        );
    }
    // A huge id is refused before any lookup.
    let huge = "A".repeat(4096);
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-huge",
        &["--run-id", &huge],
        &dead_bus,
    );
    assert_eq!(status.code(), Some(1), "huge: {status:?} {stderr:?}");
    assert_eq!(stdout_json(&stdout)["code"], "invalid-id");
    // None of the hostile ids launched anything.
    assert!(!marker.exists(), "no hostile id launched the wrong entry");
    // Usage stays exit 2 (the CLI, not an error object).
    let (status, stdout, stderr) = run_headless(
        scratch,
        &data_home,
        "agent-usage",
        &["--action", "x"],
        &dead_bus,
    );
    assert_eq!(status.code(), Some(2), "{status:?} {stdout:?} {stderr:?}");
}
