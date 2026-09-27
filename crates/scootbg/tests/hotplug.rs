//! Outputs coming and going, on a headless sway: the second layer-shell
//! compositor scootbg is checked on, and the only one here that can add and
//! remove outputs at runtime (`swaymsg create_output`, `output X unplug`).
//!
//! Skipped without sway (see `Session::sway`); `SCOOTBG_REQUIRE_SWAY`
//! makes that a failure.
#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{Session, wait_exit};
use serde_json::{Value, json};

/// Sway's outputs: `(name, logical size)`, in its order.
fn sway_outputs(session: &Session) -> Vec<(String, Value)> {
    let reply: Value =
        serde_json::from_str(&session.swaymsg(&["-t", "get_outputs", "-r"])).unwrap();
    reply
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let rect = &o["rect"];
            (
                o["name"].as_str().unwrap().to_owned(),
                json!({"width": rect["width"], "height": rect["height"]}),
            )
        })
        .collect()
}

/// Waits until scootbg's `query` lists exactly sway's outputs, by name,
/// each with a surface configured to sway's size for it.
fn wait_matching(session: &Session, what: &str) -> Vec<(String, Value)> {
    let deadline = Instant::now() + common::PATIENCE;
    loop {
        let sway = sway_outputs(session);
        let ours = session.query()["outputs"].as_array().cloned().unwrap();
        let mut ours: Vec<(String, Value)> = ours
            .iter()
            .filter(|o| o["surface"]["state"] == "configured")
            .map(|o| {
                (
                    o["name"].as_str().unwrap_or("").to_owned(),
                    o["surface"]["size"].clone(),
                )
            })
            .collect();
        let mut theirs = sway.clone();
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        theirs.sort_by(|a, b| a.0.cmp(&b.0));
        let all_listed = session.query()["outputs"].as_array().unwrap().len() == sway.len();
        if ours == theirs && all_listed {
            return sway;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: scootbg never matched sway: ours {ours:?}, sway {theirs:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn open_fds(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .count()
}

/// The daemon's fd count once steady: the same on four reads 100 ms apart.
/// A `query` that has just returned may still hold its connection on the
/// daemon's side until the daemon reads the EOF, so a single read right
/// after one can count it.
fn settled_fds(pid: u32) -> usize {
    let deadline = Instant::now() + common::PATIENCE;
    let mut count = open_fds(pid);
    let mut stable = 0;
    while stable < 3 {
        assert!(Instant::now() < deadline, "fds never settled");
        std::thread::sleep(Duration::from_millis(100));
        let again = open_fds(pid);
        stable = if again == count { stable + 1 } else { 0 };
        count = again;
    }
    count
}

/// Voluntary plus involuntary context switches: a count that moves only
/// when the process runs.
fn switches(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .filter(|l| l.contains("ctxt_switches"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

/// With nothing to do (zero outputs included), the daemon does not wake.
///
/// First it must settle (the last `query`'s EOF, the last events: the same
/// count on two reads 300 ms apart, which a loaded machine may take a while
/// to reach), then stay silent for 1.5 s. A periodic wakeup of any period
/// up to that fails one of the two.
fn assert_idle(pid: u32, what: &str) {
    let deadline = Instant::now() + common::PATIENCE;
    let mut last = switches(pid);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = switches(pid);
        if now == last {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    let after = switches(pid);
    assert_eq!(after, last, "{what}: the daemon woke up while idle");
}

/// Counts in the `WAYLAND_DEBUG=client` trace. The format is
/// `wayland-backend`'s; a change makes these counts 0 and the test fails.
struct Counts {
    layers_made: usize,
    layers_destroyed: usize,
    surfaces_destroyed: usize,
    outputs_bound: usize,
    outputs_released: usize,
}

fn count(trace: &str) -> Counts {
    let n = |needle: &str| trace.lines().filter(|l| l.contains(needle)).count();
    Counts {
        layers_made: n(".get_layer_surface(zwlr_layer_surface_v1@"),
        layers_destroyed: trace
            .lines()
            .filter(|l| l.contains("-> zwlr_layer_surface_v1@") && l.ends_with(".destroy()"))
            .count(),
        surfaces_destroyed: trace
            .lines()
            .filter(|l| l.contains("-> wl_surface@") && l.ends_with(".destroy()"))
            .count(),
        outputs_bound: trace
            .lines()
            .filter(|l| {
                l.contains("-> wl_registry@")
                    && l.contains(".bind(")
                    && l.contains("Some(\"wl_output\")")
            })
            .count(),
        outputs_released: trace
            .lines()
            .filter(|l| l.contains("-> wl_output@") && l.ends_with(".release()"))
            .count(),
    }
}

/// Add an output: a surface appears on it and is configured to its size.
/// Unplug one: its surface is destroyed (layer surface, then `wl_surface`),
/// its `wl_output` released, and the daemon keeps serving. Down to zero
/// outputs it idles with no wakeups; a new output after that is covered
/// again. Scale and transform changes reconfigure. Twenty more add/unplug
/// cycles leak neither fds nor surfaces, and nothing is ever given up.
#[test]
fn outputs_come_and_go_and_nothing_leaks() {
    let Some(session) = Session::sway("hotplug") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let pid = daemon.id();
    let first = wait_matching(&session, "at start");
    assert_eq!(first.len(), 1, "{first:?}");
    let first_name = first[0].0.clone();
    let fds_with_one = settled_fds(pid);

    session.swaymsg(&["create_output"]);
    let two = wait_matching(&session, "after create_output");
    assert_eq!(two.len(), 2, "{two:?}");

    session.swaymsg(&["output", &first_name, "unplug"]);
    let one = wait_matching(&session, "after unplugging the first");
    assert_eq!(one.len(), 1);
    assert_ne!(one[0].0, first_name);

    session.swaymsg(&["output", &one[0].0, "unplug"]);
    wait_matching(&session, "with zero outputs");
    assert_eq!(session.query()["outputs"], json!([]));
    assert_idle(pid, "zero outputs");

    session.swaymsg(&["create_output"]);
    let back = wait_matching(&session, "after an output came back");
    assert_eq!(back.len(), 1);
    let name = back[0].0.clone();
    let size = back[0].1.clone();
    assert_idle(pid, "one output");

    // Scale and transform: sway reconfigures, scootbg follows.
    session.swaymsg(&["output", &name, "scale", "2"]);
    let scaled = wait_matching(&session, "after scale 2");
    assert_eq!(
        scaled[0].1,
        json!({"width": size["width"].as_u64().unwrap() / 2,
               "height": size["height"].as_u64().unwrap() / 2})
    );
    let entry = &session.query()["outputs"][0];
    assert_eq!(entry["scale"], 2, "{entry}");
    session.swaymsg(&["output", &name, "transform", "90"]);
    let turned = wait_matching(&session, "after transform 90");
    assert_eq!(turned[0].1["width"], scaled[0].1["height"]);
    let entry = &session.query()["outputs"][0];
    // Sway's `transform 90` turns clockwise; `wl_output`'s transforms,
    // which `query` reports, count counter-clockwise, so this sway says
    // 270. Either quarter turn is right for the check.
    assert!(
        entry["transform"] == "270" || entry["transform"] == "90",
        "{entry}"
    );
    session.swaymsg(&["output", &name, "scale", "1", "transform", "normal"]);
    wait_matching(&session, "back to scale 1");

    // Twenty cycles, one output always present, so fds compare with the
    // one-output count from the start.
    let mut current = name;
    for cycle in 0..20 {
        session.swaymsg(&["create_output"]);
        let now = wait_matching(&session, &format!("cycle {cycle}: added"));
        assert_eq!(now.len(), 2);
        session.swaymsg(&["output", &current, "unplug"]);
        let now = wait_matching(&session, &format!("cycle {cycle}: unplugged"));
        assert_eq!(now.len(), 1);
        current = now[0].0.clone();
    }
    assert_idle(pid, "after the cycles");
    assert_eq!(settled_fds(pid), fds_with_one, "fds leaked across hotplug");

    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    let counts = count(&log);
    // Outputs: 1 + 1 + 1 + 20 bound, all but the live one released.
    assert_eq!(counts.outputs_bound, 23, "trace format changed?");
    assert_eq!(counts.outputs_released, counts.outputs_bound - 1);
    // One surface per output ever bound, each destroyed with its output
    // but the live one's: none leaked, none made twice.
    assert_eq!(counts.layers_made, counts.outputs_bound);
    assert_eq!(counts.layers_destroyed, counts.layers_made - 1);
    assert_eq!(counts.surfaces_destroyed, counts.layers_destroyed);
    // Every output was v4, with its own `name`: `xdg-output` is advertised
    // by sway but never bound.
    assert!(
        log.contains("Some(\"zxdg_output_manager_v1\")"),
        "not advertised?"
    );
    assert!(
        !log.lines()
            .any(|l| l.contains(".bind(") && l.contains("zxdg_output_manager_v1")),
        "xdg-output bound with only v4 outputs"
    );
    // A removal always closes first on sway; the retry found the output
    // gone every time, so nothing was re-created or given up.
    for quiet in [
        "creating it again",
        "giving up",
        "panicked",
        "cannot accept",
    ] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }

    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// A daemon started with no outputs at all (a headless session before its
/// first, a laptop with the lid shut) serves, idles, and covers the first
/// output that arrives.
#[test]
fn a_daemon_started_with_no_outputs_waits_for_one() {
    let Some(session) = Session::sway("nooutputs") else {
        return;
    };
    let only = sway_outputs(&session);
    assert_eq!(only.len(), 1, "{only:?}");
    session.swaymsg(&["output", &only[0].0, "unplug"]);
    assert!(sway_outputs(&session).is_empty());

    let mut daemon = session.daemon_logged(&[]);
    assert_eq!(session.query()["outputs"], json!([]));
    assert_idle(daemon.id(), "started with zero outputs");

    session.swaymsg(&["create_output"]);
    let now = wait_matching(&session, "the first output");
    assert_eq!(now.len(), 1);

    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    for quiet in [
        "creating it again",
        "giving up",
        "panicked",
        "cannot accept",
    ] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }
}
