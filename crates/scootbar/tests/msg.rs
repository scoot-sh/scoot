//! `scootbar msg` against a live bar on headless scoot: `query` reads every
//! placed module as JSON, `reload` live-applies an edited config file (an
//! appearance-only change redraws at once, checked in the pixels; a bad
//! one is refused with the running bar undisturbed), `version` answers,
//! `set` is refused loudly, and `kill` stops the daemon.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

mod common;

use std::process::{Child, Stdio};

use common::{Reaper, Session, rgb, wait_exit};
use serde_json::Value;

/// Starts `scootbar daemon --height 28 --config PATH` in the session, its
/// stderr in the session's bar log. Unlike [`Session::bar`], no other flag
/// is added: the file says everything else. The `--height` flag is on
/// purpose: it pins that a reload keeps the flags over the file.
fn bar_with_config(session: &Session, path: &std::path::Path) -> Reaper {
    use std::fs::File;
    let log = File::create(session.bar_log()).unwrap();
    let child: Child = session
        .scootbar()
        .arg("daemon")
        .arg("--height")
        .arg("28")
        .arg("--config")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    Reaper(child)
}

/// One `scootbar msg` command in the session.
fn msg(session: &Session, args: &[&str]) -> std::process::Output {
    session.scootbar().arg("msg").args(args).output().unwrap()
}

/// The reply's JSON: `msg` prints exactly one line on success.
fn reply(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "msg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(text.trim_end()).unwrap()
}

/// The output's usable height, as scoot's `outputs` reports it.
fn usable_height(session: &Session) -> i64 {
    session.scoot_outputs()[0]["usable"]["height"]
        .as_i64()
        .unwrap()
}

/// A config file placing the clock in `section` at `height`, in the
/// session's font, on the default background.
fn write_config(session: &Session, section: &str, height: u32) {
    write_config_bg(session, section, height, "#1e1e2e");
}

/// [`write_config`] with the bar's background color set.
fn write_config_bg(session: &Session, section: &str, height: u32, background: &str) {
    let text = format!(
        "{section} = [\"clock\"]\n\n[bar]\nheight = {height}\nfont = \"{}\"\n\n[colors]\nbackground = \"{background}\"\n",
        session.font().display()
    );
    std::fs::write(session.runtime_dir().join("bar.toml"), text).unwrap();
}

/// No module placed anywhere: an empty bar at `height`, in `background`.
fn write_empty_config(session: &Session, height: u32, background: &str) {
    let text = format!(
        "center = []\n\n[bar]\nheight = {height}\nfont = \"{}\"\n\n[colors]\nbackground = \"{background}\"\n",
        session.font().display()
    );
    std::fs::write(session.runtime_dir().join("bar.toml"), text).unwrap();
}

#[test]
fn query_reload_and_kill() {
    let Some(session) = Session::scoot("msg", 1, "") else {
        return;
    };
    write_config(&session, "left", 28);
    let path = session.runtime_dir().join("bar.toml");
    let mut bar = bar_with_config(&session, &path);

    // The bar reserves its 28 pixels.
    session.wait_for(&mut bar.0, "the zone reserved", |session| {
        (usable_height(session) == 1000 - 28).then_some(())
    });

    // `query` reads the placed clock as JSON.
    let queried = reply(&msg(&session, &["query"]));
    assert_eq!(queried["type"], "modules");
    let modules = queried["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0]["id"], "clock");
    assert_eq!(modules[0]["section"], "left");
    assert!(!modules[0]["text"].as_str().unwrap().is_empty());
    assert_eq!(modules[0]["class"], "normal");

    // `version` answers.
    let version = reply(&msg(&session, &["version"]));
    assert_eq!(version["type"], "version");
    assert_eq!(version["protocol"], 1);

    // A taller file, moved to the center and reloaded: the file's layout
    // applies, but the `--height 28` flag still wins over its 40.
    write_config(&session, "center", 40);
    let reloaded = reply(&msg(&session, &["reload"]));
    assert_eq!(reloaded["type"], "ok");
    session.wait_for(&mut bar.0, "the reloaded layout", |session| {
        let queried = msg(session, &["query"]);
        let queried = reply(&queried);
        (queried["modules"][0]["section"] == "center").then_some(())
    });
    assert_eq!(usable_height(&session), 1000 - 28);

    // An appearance-only change, nothing else touched: the reload redraws
    // at once, without waiting for the next tick. The clock sits in the
    // center of a 1600-pixel output, so the bar's left edge is background.
    assert_eq!(
        session.scoot_screenshot(1).at(5, 14),
        rgb("#1e1e2e"),
        "the bar's background before the reload"
    );
    write_config_bg(&session, "center", 40, "#ff0000");
    let recolored = reply(&msg(&session, &["reload"]));
    assert_eq!(recolored["type"], "ok");
    session.wait_for(&mut bar.0, "the reloaded background", |session| {
        (session.scoot_screenshot(1).at(5, 14) == rgb("#ff0000")).then_some(())
    });

    // N→0 at the same height: the bar repaints empty at once, no ghost
    // clock. The clock sits around the center, so some pixel there is
    // not the background while it is placed.
    let background = rgb("#ff0000");
    let shot = session.scoot_screenshot(1);
    assert!(
        (600..1000).any(|x| (0..28).any(|y| shot.at(x, y) != background)),
        "the clock draws something before the reload"
    );
    write_empty_config(&session, 40, "#ff0000");
    let emptied = reply(&msg(&session, &["reload"]));
    assert_eq!(emptied["type"], "ok");
    session.wait_for(&mut bar.0, "the emptied bar", |session| {
        let queried = reply(&msg(session, &["query"]));
        let empty = queried["modules"].as_array().is_some_and(Vec::is_empty);
        let shot = session.scoot_screenshot(1);
        let cleared = (600..1000).all(|x| (0..28).all(|y| shot.at(x, y) == background));
        (empty && cleared).then_some(())
    });

    // A bad file is refused, and the running bar stands (still empty).
    std::fs::write(&path, "[bar\nheight = ]\n").unwrap();
    let bad = msg(&session, &["reload"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("daemon:"));
    assert_eq!(usable_height(&session), 1000 - 28);
    let queried = reply(&msg(&session, &["query"]));
    assert_eq!(queried["modules"].as_array().unwrap().len(), 0);

    // `set` is refused loudly: nothing takes one yet.
    let refused = msg(&session, &["set", "clock", "{}"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("is not placed"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let refused = msg(&session, &["set", "battery", "{}"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("no module"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    // `kill` stops the daemon; its socket goes with it.
    let killed = msg(&session, &["kill"]);
    assert!(killed.status.success(), "{killed:?}");
    let status = wait_exit(&mut bar.0);
    assert!(status.success());
}
