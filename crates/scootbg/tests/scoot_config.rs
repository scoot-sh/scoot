//! scoot's own `[wallpaper]` section driving scootbg, end to end: a real
//! `scoot --headless --outputs 2` whose config names this build's scootbg
//! as `command`, so scoot itself spawns `apply-config` at start-up and on
//! each reload. `tests/config.rs` runs `apply-config` by hand the way scoot
//! would; this checks that scoot really does, with JSON scootbg accepts.
//!
//! In the scootbg integration CI job, which runs when either crate changes:
//! a change to either side that breaks the pair fails here.
#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{PATIENCE, Session, Shot, rgb};
use serde_json::{Value, json};

const TOP: &str = "#2a7f62";
const SECOND: &str = "#7f2a4c";
const CHANGED: &str = "#4c2a7f";
const PICKED: &str = "#c8b400";
const BACKGROUND: &str = "#123456";

fn config(top: Option<&str>) -> String {
    let mut text = format!("[appearance]\nbackground_color = \"{BACKGROUND}\"\n");
    if let Some(top) = top {
        text.push_str(&format!(
            "[wallpaper]\ncommand = \"{}\"\ncolor = \"{top}\"\n\
             [wallpaper.output.\"headless-2\"]\ncolor = \"{SECOND}\"\n",
            common::scootbg_bin().display()
        ));
    }
    text
}

fn shot(session: &Session, id: u64) -> Shot {
    session.scoot_screenshot(id)
}

/// Waits until scoot's outputs 1 and 2 are the two colors, by screenshot.
fn wait_shows(session: &Session, first: &str, second: &str, what: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let (a, b) = (shot(session, 1), shot(session, 2));
        if a.colors() == [rgb(first)] && b.colors() == [rgb(second)] {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: output 1 {:?}, output 2 {:?}\n{}",
            a.samples(),
            b.samples(),
            session.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// How many `apply-config` runs scoot has logged as successful.
fn applied(session: &Session) -> usize {
    session
        .log()
        .matches("scootbg applied the [wallpaper] section")
        .count()
}

fn wait_applied(session: &Session, count: usize) {
    let deadline = Instant::now() + PATIENCE;
    while applied(session) < count {
        assert!(
            Instant::now() < deadline,
            "scoot never logged {count} successful runs:\n{}",
            session.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn reload(session: &Session) -> Value {
    let reply = session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reply["type"], "reloaded", "{reply}");
    reply
}

#[test]
fn scoots_section_sets_reloads_and_clears_the_wallpaper() {
    let Some(session) = Session::start_with("scootcfg", 2, &config(Some(TOP))) else {
        return;
    };
    let config_path = session.runtime_dir().join("config.toml");

    // At start-up: scoot ran `apply-config`, which started the daemon.
    wait_shows(&session, TOP, SECOND, "the section at start-up");
    wait_applied(&session, 1);
    assert_eq!(session.query()["profile"], "scoot");

    // A reload that changes the section.
    std::fs::write(&config_path, config(Some(CHANGED))).unwrap();
    let reply = reload(&session);
    assert_eq!(reply["applied"], json!(["wallpaper"]), "{reply}");
    wait_shows(&session, CHANGED, SECOND, "the changed section");
    wait_applied(&session, 2);

    // A `scootbg set` survives a reload that leaves the section as it is.
    let output = session.run(&["set", PICKED]);
    assert!(output.status.success(), "{output:?}");
    wait_shows(&session, PICKED, PICKED, "the set");
    let reply = reload(&session);
    assert_eq!(reply["applied"], json!([]), "{reply}");
    wait_applied(&session, 3);
    wait_shows(
        &session,
        PICKED,
        PICKED,
        "the set, after an unchanged reload",
    );

    // A reload that removes the section clears it (scoot sends `{}`).
    std::fs::write(&config_path, config(None)).unwrap();
    let reply = reload(&session);
    assert_eq!(reply["applied"], json!(["wallpaper"]), "{reply}");
    wait_applied(&session, 4);
    wait_shows(&session, BACKGROUND, BACKGROUND, "the section removed");

    let log = session.log();
    assert!(
        !log.contains("scootbg apply-config failed") && !log.contains("scootbg refused"),
        "a run failed:\n{log}"
    );
}
