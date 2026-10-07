//! Transitions end to end: `set --transition` against a real
//! `scoot --headless`, checked by the reply (which waits for the last
//! frame), by `query` (which reports the running kind mid-flight) and by
//! real pixels (scoot's own screenshot mid-transition and at the end).
//!
//! Timing-dependent asserts poll (`query_until`, screenshots until mixed):
//! long durations make the mid-flight window wide, and completion waits
//! for the reply rather than sleeping.

mod common;

use std::process::Child;
use std::time::{Duration, Instant};

use common::{PATIENCE, Session, rgb, wait_exit};
use serde_json::{Value, json};

const RED: &str = "#c03020";
const BLUE: &str = "#101014";
const GREEN: &str = "#1e5c2e";

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

/// What each output shows and runs, by name, from `query`.
fn shows(session: &Session) -> Vec<(String, Value, Value)> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["name"].as_str().unwrap().to_owned(),
                o["shows"].clone(),
                o["transition"].clone(),
            )
        })
        .collect()
}

fn color(hex: &str) -> Value {
    json!({ "color": hex })
}

/// scoot's outputs as `(id, name)`, for screenshots.
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

/// Polls until every output reports `transition` null (or runs out of
/// patience): the animation is over on screen, not just answered.
fn wait_still(session: &Session, what: &str) {
    session.query_until(what, |o| o.iter().all(|o| o["transition"].is_null()));
}

/// Polls screenshots of scoot output `id` until `mixed` holds, failing
/// after [`PATIENCE`] with the last shot's samples.
fn wait_shot(
    session: &Session,
    id: u64,
    what: &str,
    mixed: impl Fn(&common::Shot) -> bool,
) -> common::Shot {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.scoot_screenshot(id);
        if mixed(&shot) {
            return shot;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: never mixed; samples {:?}",
            shot.samples()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop(session: &Session, daemon: &mut Child) {
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(daemon).success());
}

/// A transition completes: the reply waits for the last frame, and
/// afterwards the output shows the target with nothing running.
#[test]
fn a_fade_between_colors_completes() {
    let Some(session) = Session::start("trans-fade") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    ok(
        &session,
        &["set", BLUE, "--transition", "fade", "--duration-ms", "300"],
    );
    for (name, shown, transition) in shows(&session) {
        assert_eq!(shown, color(BLUE), "{name}");
        assert_eq!(transition, Value::Null, "{name}");
    }
    stop(&session, &mut daemon);
}

/// Every kind lands on its target, and `none` lands at once.
#[test]
fn each_kind_lands_on_its_target() {
    for (kind, extra) in [
        ("none", &[][..]),
        ("fade", &[]),
        ("wipe", &["--angle", "90"][..]),
        ("grow", &["--position", "0,0"][..]),
    ] {
        let Some(session) = Session::start(&format!("trans-{kind}")) else {
            return;
        };
        let mut daemon = session.daemon();
        configured(&session, 2);
        ok(&session, &["set", RED]);
        let mut args = vec!["set", BLUE, "--transition", kind, "--duration-ms", "300"];
        args.extend(extra);
        ok(&session, &args);
        for (name, shown, transition) in shows(&session) {
            assert_eq!(shown, color(BLUE), "{kind} on {name}");
            assert_eq!(transition, Value::Null, "{kind} on {name}");
        }
        // And back again, on one output.
        let name = &shows(&session)[0].0;
        let mut args = vec!["set", GREEN, "--output", name, "--transition", kind];
        args.extend(extra);
        ok(&session, &args);
        stop(&session, &mut daemon);
    }
}

/// Mid-flight, `query` reports the running kind, and the pixels are
/// between the endpoints: blended for a fade.
#[test]
fn a_fade_is_visible_mid_flight() {
    let Some(session) = Session::start("trans-mid-fade") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    let (id, _) = &scoot_ids(&session)[0];
    let id = *id;
    let mut changing = session
        .scootbg()
        .args(["set", BLUE, "--transition", "fade", "--duration-ms", "4000"])
        .spawn()
        .unwrap();
    // The reply waits for the last frame: still running here.
    assert!(changing.try_wait().unwrap().is_none());
    session.query_until("fading", |o| o.iter().all(|o| o["transition"] == "fade"));
    let red = rgb(RED);
    let blue = rgb(BLUE);
    let shot = wait_shot(&session, id, "a fade", |shot| {
        shot.colors().iter().any(|c| *c != red && *c != blue)
    });
    assert!(
        shot.colors().iter().any(|c| *c != red && *c != blue),
        "blended pixels: {:?}",
        shot.samples()
    );
    assert!(changing.wait().unwrap().success());
    wait_still(&session, "settled");
    stop(&session, &mut daemon);
}

/// Mid-flight, a wipe shows the old wallpaper ahead of its edge and the
/// new one behind it.
#[test]
fn a_wipe_is_visible_mid_flight() {
    let Some(session) = Session::start("trans-mid-wipe") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    let (id, _) = &scoot_ids(&session)[0];
    let id = *id;
    let mut changing = session
        .scootbg()
        .args([
            "set",
            BLUE,
            "--transition",
            "wipe",
            "--angle",
            "0",
            "--duration-ms",
            "4000",
        ])
        .spawn()
        .unwrap();
    assert!(changing.try_wait().unwrap().is_none());
    session.query_until("wiping", |o| o.iter().all(|o| o["transition"] == "wipe"));
    let red = rgb(RED);
    let blue = rgb(BLUE);
    // A hard edge: both endpoints on screen at once.
    wait_shot(&session, id, "a wipe", |shot| {
        let colors = shot.colors();
        colors.contains(&red) && colors.contains(&blue)
    });
    assert!(changing.wait().unwrap().success());
    wait_still(&session, "settled");
    stop(&session, &mut daemon);
}

/// Mid-flight, a grow shows the new wallpaper inside its disc and the old
/// one outside it.
#[test]
fn a_grow_is_visible_mid_flight() {
    let Some(session) = Session::start("trans-mid-grow") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    let (id, _) = &scoot_ids(&session)[0];
    let id = *id;
    let mut changing = session
        .scootbg()
        .args([
            "set",
            BLUE,
            "--transition",
            "grow",
            "--position",
            "0.5,0.5",
            "--duration-ms",
            "4000",
        ])
        .spawn()
        .unwrap();
    assert!(changing.try_wait().unwrap().is_none());
    session.query_until("growing", |o| o.iter().all(|o| o["transition"] == "grow"));
    let red = rgb(RED);
    let blue = rgb(BLUE);
    wait_shot(&session, id, "a grow", |shot| {
        let colors = shot.colors();
        colors.contains(&red) && colors.contains(&blue)
    });
    assert!(changing.wait().unwrap().success());
    wait_still(&session, "settled");
    stop(&session, &mut daemon);
}

/// A new request mid-transition starts from what is on screen now: no
/// queue, no flash of an endpoint. The first reply waits for what
/// replaced it.
#[test]
fn a_new_request_mid_transition_replaces_it() {
    let Some(session) = Session::start("trans-restart") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    let mut first = session
        .scootbg()
        .args(["set", BLUE, "--transition", "fade", "--duration-ms", "5000"])
        .spawn()
        .unwrap();
    session.query_until("fading", |o| o.iter().all(|o| o["transition"] == "fade"));
    // Still running: the reply waits for the last frame.
    assert!(first.try_wait().unwrap().is_none());
    ok(
        &session,
        &["set", GREEN, "--transition", "wipe", "--duration-ms", "300"],
    );
    // The first reply arrives once what replaced it is on screen, like a
    // superseded color's.
    assert!(first.wait().unwrap().success());
    for (name, shown, transition) in shows(&session) {
        assert_eq!(shown, color(GREEN), "{name}");
        assert_eq!(transition, Value::Null, "{name}");
    }
    // An instant request mid-transition aborts it and lands at once.
    let mut fading = session
        .scootbg()
        .args(["set", BLUE, "--transition", "fade", "--duration-ms", "5000"])
        .spawn()
        .unwrap();
    session.query_until("fading again", |o| {
        o.iter().all(|o| o["transition"] == "fade")
    });
    ok(&session, &["set", RED]);
    assert!(fading.wait().unwrap().success());
    for (name, shown, transition) in shows(&session) {
        assert_eq!(shown, color(RED), "{name}");
        assert_eq!(transition, Value::Null, "{name}");
    }
    stop(&session, &mut daemon);
}

/// An image fades in from a color: endpoints need not both be colors.
#[test]
fn an_image_fades_in_from_a_color() {
    let Some(session) = Session::start("trans-image") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    // An absolute path, whatever the test's working directory.
    let image = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/quadrants.jpg");
    ok(
        &session,
        &["set", image, "--transition", "fade", "--duration-ms", "300"],
    );
    session.query_until("an image", |o| {
        o.iter().all(|o| o["shows"]["image"].is_string())
    });
    stop(&session, &mut daemon);
}

/// Refusals change nothing: a bad kind, a bad value, and a transition on
/// a `clear` are usage errors, and the wallpaper stays.
#[test]
fn bad_transitions_change_nothing() {
    let Some(session) = Session::start("trans-bad") else {
        return;
    };
    let mut daemon = session.daemon();
    configured(&session, 2);
    ok(&session, &["set", RED]);
    for args in [
        vec!["set", BLUE, "--transition", "dissolve"],
        vec![
            "set",
            BLUE,
            "--transition",
            "fade",
            "--duration-ms",
            "99999",
        ],
        vec!["set", BLUE, "--duration-ms", "300"],
        vec!["clear", "--transition", "fade"],
    ] {
        let out = session.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
    for (name, shown, _) in shows(&session) {
        assert_eq!(shown, color(RED), "{name}");
    }
    stop(&session, &mut daemon);
}

/// scoot's `[wallpaper]` section carries transitions: a reload that
/// changes the section animates through its keys, and an unchanged one
/// replays nothing.
#[test]
fn the_section_animates_reloads() {
    let command = common::scootbg_bin();
    let config = |color: &str| {
        format!(
            "[wallpaper]\ncommand = \"{}\"\ncolor = \"{color}\"\n\
             transition = \"fade\"\nduration-ms = \"300\"\n",
            command.display()
        )
    };
    let Some(session) = Session::start_with("trans-section", 1, &config(RED)) else {
        return;
    };
    let config_path = session.runtime_dir().join("config.toml");
    // At start-up scoot runs `apply-config`, which starts the daemon:
    // wait for it before querying, the way `daemon()` does.
    let deadline = Instant::now() + PATIENCE;
    while !common::answers(&session.socket()) {
        assert!(
            Instant::now() < deadline,
            "apply-config never started a daemon"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The section is showing.
    session.query_until("the section at start-up", |o| {
        o.len() == 1 && o[0]["shows"] == color(RED)
    });
    // A reload that changes the section: the reply waits out the fade.
    std::fs::write(&config_path, config(BLUE)).unwrap();
    let reply = session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reply["type"], "reloaded", "{reply}");
    session.query_until("the changed section", |o| {
        o.len() == 1 && o[0]["shows"] == color(BLUE) && o[0]["transition"].is_null()
    });
    // An unchanged reload replays nothing: nothing runs.
    std::fs::write(&config_path, config(BLUE)).unwrap();
    let reply = session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reply["type"], "reloaded", "{reply}");
    session.query_until("still settled", |o| {
        o.len() == 1 && o[0]["shows"] == color(BLUE) && o[0]["transition"].is_null()
    });
    stop_daemon(&session);
}

fn stop_daemon(session: &Session) {
    assert!(session.run(&["kill"]).status.success());
}
