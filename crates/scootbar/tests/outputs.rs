//! Which outputs get a bar, and how each differs, on headless scoot with
//! two outputs: one config gives them different module sets, heights and
//! edges; `outputs` leaves one out; a reload adds and removes bars; hide
//! and show keep to the list; and the second output costs a surface and
//! its buffers, not another set of file descriptors.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// Every test here places the clock (two also place workspaces, gated per
// test), so this file exists only where it does (docs/scootbar/testing.md:
// the feature matrix).
#![cfg(feature = "clock")]

mod common;

use std::process::{Child, Stdio};

use common::{Reaper, Session, assert_buffers, settled_fds};
use serde_json::Value;

const HEIGHT: i64 = 1000;

fn daemon(session: &Session, path: &std::path::Path) -> Reaper {
    let log = std::fs::File::create(session.bar_log()).unwrap();
    let child: Child = session
        .scootbar()
        .arg("daemon")
        .arg("--config")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    Reaper(child)
}

fn msg(session: &Session, command: &str) -> Value {
    let output = session.scootbar().arg("msg").arg(command).output().unwrap();
    assert!(
        output.status.success(),
        "msg {command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim_end()).unwrap()
}

/// scoot's names for its outputs, in order.
fn names(session: &Session) -> Vec<String> {
    session
        .scoot_outputs()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Output `index`'s usable rectangle as `(y, height)`.
fn usable(session: &Session, index: usize) -> (i64, i64) {
    let outputs = session.scoot_outputs();
    let u = &outputs[index]["usable"];
    (u["y"].as_i64().unwrap(), u["height"].as_i64().unwrap())
}

/// `[bar]` with the test font, then `rest` (top-level keys must come
/// first, so `head` is what precedes any table).
fn config(session: &Session, head: &str, rest: &str) -> std::path::PathBuf {
    let path = session.runtime_dir().join("bar.toml");
    let text = format!(
        "{head}\n[bar]\nheight = 28\nfont = \"{}\"\n{rest}\n",
        session.font().display()
    );
    std::fs::write(&path, text).unwrap();
    path
}

/// The modules `query` reports on `output`, as `(id, section)`, sorted.
fn shown(session: &Session, output: &str) -> Vec<(String, String)> {
    let reply = msg(session, "query");
    let mut shown: Vec<(String, String)> = reply["modules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["output"] == output)
        .map(|m| {
            (
                m["id"].as_str().unwrap().to_owned(),
                m["section"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    shown.sort();
    shown
}

fn pair(id: &str, section: &str) -> (String, String) {
    (id.to_owned(), section.to_owned())
}

#[cfg(all(feature = "clock", feature = "workspaces"))]
#[test]
fn two_outputs_show_different_module_sets_heights_and_edges_from_one_config() {
    let Some(session) = Session::scoot("multi", 2, "") else {
        return;
    };
    let names = names(&session);
    assert_eq!(names.len(), 2, "{names:?}");
    let path = config(
        &session,
        "left = [\"workspaces\"]\nright = [\"clock\"]",
        &format!(
            "[output.\"{}\"]\nheight = 40\nedge = \"bottom\"\nleft = [\"clock\"]\n",
            names[1]
        ),
    );
    let mut bar = daemon(&session, &path);
    session.wait_for(&mut bar.0, "a bar on each output", |session| {
        (usable(session, 0) == (28, HEIGHT - 28) && usable(session, 1) == (0, HEIGHT - 40))
            .then_some(())
    });
    assert_eq!(
        shown(&session, &names[0]),
        [pair("clock", "right"), pair("workspaces", "left")]
    );
    assert_eq!(shown(&session, &names[1]), [pair("clock", "left")]);
    assert!(bar.0.try_wait().unwrap().is_none());
}

#[test]
fn the_list_leaves_an_output_without_a_bar_and_a_reload_moves_it() {
    let Some(session) = Session::scoot("list", 2, "") else {
        return;
    };
    let names = names(&session);
    let path = config(
        &session,
        &format!("center = [\"clock\"]\noutputs = [\"{}\"]", names[1]),
        "",
    );
    let mut bar = daemon(&session, &path);
    session.wait_for(&mut bar.0, "the listed output's bar", |session| {
        (usable(session, 1) == (28, HEIGHT - 28)).then_some(())
    });
    assert_eq!(usable(&session, 0), (0, HEIGHT), "the unlisted output");
    // `query` lists modules only where there is a bar.
    assert!(shown(&session, &names[0]).is_empty());
    assert_eq!(shown(&session, &names[1]), [pair("clock", "center")]);
    let pid = bar.0.id();
    let fds = settled_fds(pid);

    // The other output instead: one bar goes, the other comes.
    config(
        &session,
        &format!("center = [\"clock\"]\noutputs = [\"{}\"]", names[0]),
        "",
    );
    assert_eq!(msg(&session, "reload")["type"], "ok");
    session.wait_for(&mut bar.0, "the bar moved", |session| {
        (usable(session, 0) == (28, HEIGHT - 28) && usable(session, 1) == (0, HEIGHT)).then_some(())
    });
    assert_eq!(shown(&session, &names[1]), []);
    assert_eq!(shown(&session, &names[0]), [pair("clock", "center")]);
    assert_eq!(settled_fds(pid), fds, "a reload leaked fds");
    assert_buffers(pid, 1);

    // Back to all: both.
    config(&session, "center = [\"clock\"]", "");
    assert_eq!(msg(&session, "reload")["type"], "ok");
    session.wait_for(&mut bar.0, "both bars", |session| {
        (usable(session, 0) == (28, HEIGHT - 28) && usable(session, 1) == (28, HEIGHT - 28))
            .then_some(())
    });
    assert_buffers(pid, 2);
}

/// Hide and show are per the list: a shown bar comes back only where the
/// list wants one, and a reload that changes the list while hidden takes
/// effect at the show.
#[test]
fn hide_and_show_keep_to_the_list() {
    let Some(session) = Session::scoot("vis", 2, "") else {
        return;
    };
    let names = names(&session);
    let path = config(
        &session,
        &format!("center = [\"clock\"]\noutputs = [\"{}\"]", names[0]),
        "",
    );
    let mut bar = daemon(&session, &path);
    session.wait_for(&mut bar.0, "a bar on the listed output", |session| {
        (usable(session, 0) == (28, HEIGHT - 28)).then_some(())
    });
    assert_eq!(msg(&session, "hide")["visible"], false);
    session.wait_for(&mut bar.0, "the zone released", |session| {
        (usable(session, 0) == (0, HEIGHT)).then_some(())
    });
    // Changed while hidden: applies at the show, not before.
    config(
        &session,
        &format!("center = [\"clock\"]\noutputs = [\"{}\"]", names[1]),
        "",
    );
    assert_eq!(msg(&session, "reload")["type"], "ok");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(usable(&session, 0), (0, HEIGHT));
    assert_eq!(usable(&session, 1), (0, HEIGHT));
    assert_eq!(msg(&session, "show")["visible"], true);
    session.wait_for(&mut bar.0, "the bar shown where listed", |session| {
        (usable(session, 1) == (28, HEIGHT - 28)).then_some(())
    });
    assert_eq!(usable(&session, 0), (0, HEIGHT));
    assert_buffers(bar.0.id(), 1);
}

/// A second output adds a surface and its buffers, not another set of
/// data sources: the daemon holds the same number of fds with two bars as
/// with one, and the modules are started once.
#[cfg(all(feature = "clock", feature = "workspaces"))]
#[test]
fn a_second_output_adds_no_fds() {
    let fds = |outputs: u32| {
        let session = Session::scoot(&format!("fds{outputs}"), outputs, "")?;
        let path = config(&session, "left = [\"workspaces\"]\nright = [\"clock\"]", "");
        let mut bar = daemon(&session, &path);
        session.wait_for(&mut bar.0, "every output's bar", |session| {
            (0..outputs as usize)
                .all(|i| usable(session, i) == (28, HEIGHT - 28))
                .then_some(())
        });
        let pid = bar.0.id();
        let count = settled_fds(pid);
        assert_buffers(pid, outputs as usize);
        eprintln!("fds with {outputs} output(s): {count}");
        Some(count)
    };
    let (Some(one), Some(two)) = (fds(1), fds(2)) else {
        return;
    };
    assert_eq!(one, two, "a second output added fds");
}
