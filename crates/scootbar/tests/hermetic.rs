//! The integration tests are hermetic: every daemon a test starts reads
//! the sandbox's empty XDG directories (see `common`), never the real
//! home. This pins it with a conflicting `bar.toml` where the default
//! lookup would find it: without the sandbox the daemon would read it and
//! refuse `--center clock` against its `left = ["clock"]` (`clock is
//! placed twice`), and start its `volume` module with no sound server
//! behind it.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

mod common;

use common::{Reaper, Session};
use serde_json::Value;

const BAR: &str = "#c03020";

/// A conflicting `bar.toml` under a `HOME` the daemon is pointed at must
/// not change the result: the sandbox's `XDG_CONFIG_HOME` stays empty, so
/// the default lookup never reaches the home.
#[test]
fn a_conflicting_bar_toml_in_the_home_is_ignored() {
    let Some(session) = Session::scoot("hermetic", 1, "") else {
        return;
    };
    // Deterministic whatever the ambient environment sets: without the
    // sandbox the lookup falls back to `HOME`, which is the fake one below.
    // (Safe: this binary holds this one test, and nextest gives each test
    // its own process; the sandbox sets the variable back on every child
    // it starts, so nothing else can observe this.)
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    let home = session.runtime_dir().join("home");
    let dir = home.join(".config").join("scoot");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("bar.toml"),
        "left = [\"clock\"]\nright = [\"volume\"]\n",
    )
    .unwrap();
    let mut bar =
        Reaper(session.bar_with_home(&["--background", BAR, "--center", "clock"], &[], &home));
    // The bar comes up on the flags alone: the file's clock is not placed
    // (which would be a usage error and exit before anything is drawn).
    session.wait_for(&mut bar.0, "the zone reserved", |session| {
        let outputs = session.scoot_outputs();
        (outputs.len() == 1
            && outputs[0]["usable"]["y"] == 28
            && outputs[0]["usable"]["height"] == 972)
            .then_some(())
    });
    // And the file's volume is not started either: the layout holds only
    // the flag's clock, and the daemon never warned of no sound server.
    let output = session
        .scootbar()
        .arg("msg")
        .arg("layout")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "layout failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let layout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(layout["type"], "layout", "{layout}");
    let outputs = layout["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1, "{layout}");
    let ids: Vec<&str> = outputs[0]["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["clock"], "{layout}");
    assert!(
        session.bar_stderr().is_empty(),
        "the daemon said: {}",
        session.bar_stderr()
    );
}
