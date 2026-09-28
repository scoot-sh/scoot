//! `scootbg apply-config`, end to end on a real `scoot --headless`, with
//! scoot only as the Wayland host: the tests run `apply-config` the way
//! scoot will (at start-up and on each reload), and check what shows by
//! `query` and by screenshot.
//!
//! The precedence table of docs/scootbg/backlog/resolved/scoot-integration-done.md, row
//! by row, each across a restart (the compositor killed, a new one started
//! over the same state directory, and `apply-config` run again as scoot's
//! start-up would); then starting the daemon (detached, raced, failing),
//! adopting profiles, the `{}` path with no daemon, validation, a daemon
//! from another build (a fake one, on the socket), and a daemon killed
//! mid-apply.
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{PATIENCE, Scratch, Session, Shot, answers, rgb, wait_exit};
use serde_json::{Value, json};

const RED: &str = "#ff0000";
const BLUE: &str = "#0000ff";
const GREEN: &str = "#00ff00";
const YELLOW: &str = "#ffff00";

/// What one `apply-config` run did.
struct Run {
    status: ExitStatus,
    stderr: String,
}

/// Every run's stderr goes to a file of its own: a daemon it starts keeps
/// writing there (it inherits it), and a pipe read to its end would wait
/// for that daemon to exit.
static RUNS: AtomicUsize = AtomicUsize::new(0);

fn run_apply(session: &Session, profile: &str, json: &str) -> Run {
    let log = session.scratch.0.join(format!(
        "apply-{}.log",
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    let started = Instant::now();
    let output = session
        .scootbg()
        .args(["apply-config", "--profile", profile, json])
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&log).unwrap())
        .output()
        .unwrap();
    assert!(
        started.elapsed() < PATIENCE,
        "apply-config took {:?}",
        started.elapsed()
    );
    assert!(
        output.stdout.is_empty(),
        "apply-config printed {:?}",
        output.stdout
    );
    Run {
        status: output.status,
        stderr: std::fs::read_to_string(&log).unwrap_or_default(),
    }
}

/// `apply-config` that must succeed.
fn apply(session: &Session, profile: &str, json: &str) -> String {
    let run = run_apply(session, profile, json);
    assert!(
        run.status.success(),
        "apply-config {json}: {}: {}",
        run.status,
        run.stderr
    );
    run.stderr
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

fn configured_names(session: &Session, count: usize) -> Vec<String> {
    session
        .query_until("all configured", |o| {
            o.len() == count && o.iter().all(|o| o["surface"]["state"] == "configured")
        })
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

/// What `query` says each output shows now, by name, and the profile.
fn showing(session: &Session) -> (Vec<(String, Value)>, String) {
    let reply = session.query();
    let outputs = reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["name"].as_str().unwrap().to_owned(), o["shows"].clone()))
        .collect();
    (outputs, reply["profile"].as_str().unwrap().to_owned())
}

fn color(hex: &str) -> Value {
    json!({ "color": hex })
}

/// A screenshot of the output named `name`.
fn shot(session: &Session, name: &str) -> Shot {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    let id = reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .unwrap_or_else(|| panic!("scoot has no output {name}: {reply}"))["id"]
        .as_u64()
        .unwrap();
    session.scoot_screenshot(id)
}

/// Waits until `query` says all `count` outputs show `want`. For a daemon
/// no reply waited for: one started other than by `apply-config` (a plain
/// `scootbg daemon`, a `--serve` that took over), or a reply that comes at
/// once (an error). Such a daemon draws a round trip after its surface is
/// `configured`, and `query` truthfully says `configured` with `shows:
/// null` in between, so waiting for `configured` alone is too early.
/// Once `shows` says so the draw has gone to the compositor: the daemon
/// flushes it before it next reads a request.
fn wait_all_show(session: &Session, count: usize, want: &Value, what: &str) {
    session.query_until(what, |o| {
        o.len() == count && o.iter().all(|o| o["shows"] == *want)
    });
}

/// Both outputs show `hex`, by `query` and by screenshot, right after the
/// command that returned (`apply-config` returns once it is on screen) or
/// [`wait_all_show`].
fn both_show(session: &Session, hex: &str, what: &str) {
    let (outputs, _) = showing(session);
    assert_eq!(outputs.len(), 2, "{what}: {outputs:?}");
    for (name, shows) in &outputs {
        assert_eq!(*shows, color(hex), "{what}: {name}");
        shot(session, name).assert_all(rgb(hex), what);
    }
}

fn state_text(state_home: &Path, profile: &str) -> String {
    std::fs::read_to_string(state_home.join("scootbg").join(profile)).unwrap_or_default()
}

fn fingerprint_line(state_home: &Path, profile: &str) -> Option<String> {
    state_text(state_home, profile)
        .lines()
        .find(|line| line.starts_with("fingerprint "))
        .map(str::to_owned)
}

/// Whether some daemon holds this session's lock.
fn lock_held(session: &Session) -> bool {
    let lock = session
        .scratch
        .0
        .join(format!("scootbg-{}.lock", session.wayland_display));
    let Ok(file) = std::fs::OpenOptions::new().read(true).open(lock) else {
        return false;
    };
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err()
}

/// Waits until no daemon holds this session's lock: it exited, and its
/// state is written.
fn wait_daemon_gone(session: &Session) {
    let deadline = Instant::now() + PATIENCE;
    while lock_held(session) {
        assert!(Instant::now() < deadline, "the daemon never exited");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// "Restart": the compositor goes away (the detached daemon with it, once
/// it has written its state), and a new one starts over the same state
/// directory. Returns the new session, where nothing runs yet.
fn restart(mut session: Session, tag: &str) -> Session {
    let state_home = session.state_home.clone();
    session.kill_compositor();
    wait_daemon_gone(&session);
    drop(session);
    let mut again = Session::start(tag).expect("scoot was there a moment ago");
    again.state_home = state_home;
    again
}

/// A session whose state lives in a directory of its own (outside its
/// scratch one, so it survives restarts).
fn start(tag: &str, shared: &Scratch) -> Option<Session> {
    let mut session = Session::start(tag)?;
    session.state_home = shared.0.clone();
    Some(session)
}

fn section(hex: &str) -> String {
    format!(r#"{{"color":"{hex}"}}"#)
}

// ---- The precedence table ------------------------------------------------

/// Row 1: section A, `set X`, restart: A unchanged, so X is restored.
/// Also the rule without a restart: an unchanged re-apply (a reload that
/// touched something else) keeps X, and so does a new `command`.
#[test]
fn a_set_survives_an_unchanged_section() {
    let shared = Scratch::new("ac-row1");
    let Some(session) = start("ac-r1", &shared) else {
        return;
    };
    let a = r##"{"color":"#ff0000","command":"/nix/store/one-scootbg/bin/scootbg"}"##;
    apply(&session, "scoot", a);
    configured_names(&session, 2);
    both_show(&session, RED, "A applied");
    ok(&session, &["set", BLUE]);
    // A reload with the section unchanged, and one where only `command`
    // moved (a home-manager upgrade): X stays.
    apply(&session, "scoot", a);
    both_show(&session, BLUE, "an unchanged section keeps the set");
    apply(
        &session,
        "scoot",
        r##"{"command":"/nix/store/two-scootbg/bin/scootbg","color":"#ff0000"}"##,
    );
    both_show(&session, BLUE, "a new command is no change");

    let session = restart(session, "ac-r1b");
    apply(&session, "scoot", a);
    configured_names(&session, 2);
    both_show(&session, BLUE, "restored across the restart");
}

/// Row 2: A, reload with B, `set X`, restart: B unchanged since its
/// reload, so X.
#[test]
fn a_set_after_a_reload_survives() {
    let shared = Scratch::new("ac-row2");
    let Some(session) = start("ac-r2", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    apply(&session, "scoot", &section(GREEN));
    both_show(&session, GREEN, "B applied on reload");
    ok(&session, &["set", BLUE]);
    let session = restart(session, "ac-r2b");
    apply(&session, "scoot", &section(GREEN));
    configured_names(&session, 2);
    both_show(&session, BLUE, "X restored, B unchanged");
}

/// Row 3: A, reload with B, restart: B shows (and it is B's fingerprint
/// that is recorded).
#[test]
fn a_reload_then_a_restart_shows_the_new_section() {
    let shared = Scratch::new("ac-row3");
    let Some(session) = start("ac-r3", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    // Written in the background: waited for.
    wait_for_state(&shared.0, "scoot", "fingerprint ");
    let a = fingerprint_line(&shared.0, "scoot").expect("A's fingerprint");
    configured_names(&session, 2);
    apply(&session, "scoot", &section(GREEN));
    both_show(&session, GREEN, "B applied");
    wait_for_state(&shared.0, "scoot", &format!("all color {GREEN}"));
    let b = fingerprint_line(&shared.0, "scoot").expect("B's fingerprint");
    assert_ne!(a, b);
    let session = restart(session, "ac-r3b");
    apply(&session, "scoot", &section(GREEN));
    configured_names(&session, 2);
    both_show(&session, GREEN, "B after the restart");
    assert_eq!(fingerprint_line(&shared.0, "scoot"), Some(b));
}

/// Row 4: the section removed by a reload (`{}`: cleared, the daemon
/// keeps running, so a `set` still works), later re-added as A: `{}` was
/// recorded at removal, so A differs and shows, over a `set` made while it
/// was removed; once without a restart in between, once across one.
#[test]
fn a_removed_then_re_added_section_shows() {
    let shared = Scratch::new("ac-row4");
    let Some(session) = start("ac-r4", &shared) else {
        return;
    };
    let names = scoot_names(&session);
    let background = shot(&session, &names[0]).at(10, 10);
    let a = section(RED);
    apply(&session, "scoot", &a);
    both_show(&session, RED, "A");

    remove_then_set(&session, background);
    apply(&session, "scoot", &a);
    both_show(&session, RED, "A re-added over the set");

    remove_then_set(&session, background);
    // scoot spawns nothing at start-up without a section, so the next
    // start with one is the re-add.
    let session = restart(session, "ac-r4b");
    apply(&session, "scoot", &a);
    configured_names(&session, 2);
    both_show(&session, RED, "A re-added across a restart");
}

/// A reload that removes the section, then a `set`.
fn remove_then_set(session: &Session, background: [u8; 3]) {
    apply(session, "scoot", "{}");
    let (outputs, _) = showing(session);
    for (name, shows) in &outputs {
        assert_eq!(*shows, Value::Null, "{name}");
        shot(session, name).assert_all(background, "removed: cleared");
    }
    assert!(answers(&session.socket()), "the daemon keeps running");
    ok(session, &["set", YELLOW]);
    both_show(session, YELLOW, "a set with the section removed");
}

/// Row 5: no daemon yet, section added by a reload: `apply-config` starts
/// the daemon, detached (its own session, and it outlives the command),
/// with the section as its starting point, recorded.
#[test]
fn a_section_starts_the_daemon() {
    let shared = Scratch::new("ac-row5");
    let Some(session) = start("ac-r5", &shared) else {
        return;
    };
    assert!(!answers(&session.socket()));
    let stderr = apply(&session, "scoot", &section(RED));
    assert!(stderr.is_empty(), "{stderr}");
    assert!(
        answers(&session.socket()),
        "the daemon outlives apply-config"
    );
    configured_names(&session, 2);
    both_show(&session, RED, "started with the section");
    let (_, profile) = showing(&session);
    assert_eq!(profile, "scoot");
    wait_for_state(&shared.0, "scoot", &format!("all color {RED}"));
    let text = state_text(&shared.0, "scoot");
    assert!(
        text.starts_with("scootbg-state 1\nprofile scoot\nfingerprint "),
        "{text}"
    );
    assert!(text.contains(&format!("all color {RED}\n")), "{text}");
    // Detached: a session of its own, so its process group is its own.
    let pid = daemon_pid(&session).expect("the daemon's pid");
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after = &stat[stat.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = after.split(' ').collect();
    // state ppid pgrp session
    assert_eq!(fields[2], pid.to_string(), "its own process group: {stat}");
    assert_eq!(fields[3], pid.to_string(), "its own session: {stat}");
    // Named as the binary it is, for `pgrep scootbg` (not `exe`, as an
    // exec of `/proc/self/exe` would make it).
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap();
    assert_eq!(comm.trim_end(), "scootbg");
    ok(&session, &["kill"]);
    wait_daemon_gone(&session);
}

/// Row 6: an `[autostart]` `scootbg daemon` (profile `default`) as well:
/// whichever binds first, the daemon ends up on scoot's profile, so a
/// `set X` from a previous boot is restored either way, and later changes
/// are saved to scoot's profile.
#[test]
fn an_autostart_daemon_adopts_the_profile() {
    let shared = Scratch::new("ac-row6");
    let Some(session) = start("ac-r6", &shared) else {
        return;
    };
    // The previous boot: section A, then `set X`.
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    ok(&session, &["set", BLUE]);
    // The default profile has a wallpaper of its own, which the autostart
    // daemon restores first.
    ok(&session, &["kill"]);
    wait_daemon_gone(&session);
    let mut daemon = session.daemon();
    ok(&session, &["set", YELLOW]);
    ok(&session, &["kill"]);
    assert!(wait_exit(&mut daemon).success());
    let session = restart(session, "ac-r6b");

    // The autostart daemon binds first.
    let mut daemon = session.daemon();
    wait_all_show(&session, 2, &color(YELLOW), "the default profile restored");
    both_show(&session, YELLOW, "the default profile, restored first");
    apply(&session, "scoot", &section(RED));
    both_show(&session, BLUE, "adopted scoot's profile: X");
    assert_eq!(showing(&session).1, "scoot");
    ok(&session, &["set", GREEN]);
    ok(&session, &["kill"]);
    let stderr = common::stderr_of(&mut daemon);
    assert!(wait_exit(&mut daemon).success());
    assert!(
        stderr.contains("restoring and saving profile \"scoot\" from now on (was \"default\")"),
        "{stderr}"
    );
    assert!(state_text(&shared.0, "scoot").contains(&format!("all color {GREEN}")));
    assert!(
        state_text(&shared.0, "default").contains(&format!("all color {YELLOW}")),
        "the default profile is left as it was"
    );

    // apply-config binds first: the autostart daemon is refused, and the
    // one running is on scoot's profile.
    let session = restart(session, "ac-r6c");
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    both_show(&session, GREEN, "X restored");
    let late = session.run(&["daemon"]);
    assert!(!late.status.success());
    assert!(
        String::from_utf8_lossy(&late.stderr).contains("already running"),
        "{late:?}"
    );
    assert_eq!(showing(&session).1, "scoot");
}

/// The one order the table cannot see (documented): the section removed
/// while scoot is not running and re-added unchanged before the next start
/// looks unchanged, so a `set` pick restores.
#[test]
fn a_removal_while_not_running_is_not_seen() {
    let shared = Scratch::new("ac-unseen");
    let Some(session) = start("ac-un", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    ok(&session, &["set", BLUE]);
    // scoot stops; the section is removed and re-added in the config
    // meanwhile; scoot never ran apply-config '{}'.
    let session = restart(session, "ac-unb");
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    both_show(&session, BLUE, "unchanged as far as scoot saw");
}

// ---- The section itself ---------------------------------------------------

/// Images and per-output tables: the top level for every output, an
/// output's own table over it, and an empty table for nothing there.
#[test]
fn per_output_tables_and_images() {
    let shared = Scratch::new("ac-tables");
    let Some(session) = start("ac-tab", &shared) else {
        return;
    };
    let picture = session.scratch.0.join("a picture.png");
    write_png(&picture, [0, 0, 255]);
    let names = scoot_names(&session);
    let background = shot(&session, &names[1]).at(10, 10);
    let json = json!({
        "image": picture.to_str().unwrap(),
        "mode": "stretch",
        "output": { names[1].clone(): { "color": GREEN } },
    })
    .to_string();
    apply(&session, "scoot", &json);
    shot(&session, &names[0]).assert_all([0, 0, 255], "the image, everywhere else");
    shot(&session, &names[1]).assert_all(rgb(GREEN), "the output's own table");
    wait_for_state(&shared.0, "scoot", "all image ");
    let text = state_text(&shared.0, "scoot");
    assert!(text.contains("all image "), "{text}");
    assert!(
        text.contains(&format!("output {} color {GREEN}", names[1])),
        "{text}"
    );

    // An empty table: nothing on that output, the rest as the top says.
    let json = json!({"color": RED, "output": { names[1].clone(): {} }}).to_string();
    apply(&session, "scoot", &json);
    shot(&session, &names[0]).assert_all(rgb(RED), "the top level");
    shot(&session, &names[1]).assert_all(background, "an empty table: nothing");
}

/// An image in the section that is not there: the rest is applied, the
/// reply says which, the exit status is 1, and the choice stays saved, so
/// it shows once the file is back (and the section is unchanged, so no
/// re-apply is needed: a restart restores it).
#[test]
fn a_missing_image_is_loud_and_the_rest_applies() {
    let shared = Scratch::new("ac-missing");
    let Some(session) = start("ac-mis", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    let names = configured_names(&session, 2);
    // Outside the session's scratch directory, which a restart removes.
    let picture = shared.0.join("later.png");
    let json = json!({
        "color": BLUE,
        "output": { names[1].clone(): { "image": picture.to_str().unwrap() } },
    })
    .to_string();
    let run = run_apply(&session, "scoot", &json);
    assert_eq!(run.status.code(), Some(1), "{}", run.stderr);
    assert!(
        run.stderr.contains("applied, except") && run.stderr.contains("later.png"),
        "{}",
        run.stderr
    );
    // That reply came at once, without waiting for the rest to show.
    session.query_until("the rest shown", |o| {
        o.len() == 2
            && o.iter()
                .any(|o| o["name"] == *names[0] && o["shows"] == color(BLUE))
    });
    shot(&session, &names[0]).assert_all(rgb(BLUE), "the rest is applied");
    assert_eq!(showing(&session).0[1].1, Value::Null);
    wait_for_state(&shared.0, "scoot", "later.png");

    write_png(&picture, [0, 255, 0]);
    let session = restart(session, "ac-misb");
    apply(&session, "scoot", &json);
    configured_names(&session, 2);
    shot(&session, &names[1]).assert_all([0, 255, 0], "back, restored");
}

/// A missing image is reported by every `apply-config` until it is back,
/// the cold start's first one included, and an unchanged one then shows
/// it; a `set` made meanwhile always stands (review of PR #293, F1).
#[test]
fn a_missing_image_is_reported_until_it_is_back() {
    let shared = Scratch::new("ac-back");
    let Some(session) = start("ac-back", &shared) else {
        return;
    };
    let names = scoot_names(&session);
    let image_of = |path: &Path| {
        json!({"image": path.to_str().unwrap(), "mode": "fill", "fill": "#000000",
               "filter": "lanczos3"})
    };
    let exits_1_naming = |session: &Session, json: &str, path: &Path| {
        let run = run_apply(session, "scoot", json);
        assert_eq!(run.status.code(), Some(1), "{json}: {}", run.stderr);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(run.stderr.contains(name), "{}", run.stderr);
    };

    // A per-output image, missing at a cold start: the first reply says so.
    let p = shared.0.join("p.png");
    let s1 = json!({"color": BLUE, "output": {names[1].clone(): {"image": p.to_str().unwrap()}}})
        .to_string();
    exits_1_naming(&session, &s1, &p);
    configured_names(&session, 2);
    exits_1_naming(&session, &s1, &p);
    // A `set` for the other output, meanwhile, then the file appears.
    ok(&session, &["set", YELLOW, "--output", &names[0]]);
    write_png(&p, [0, 255, 0]);
    let stderr = apply(&session, "scoot", &s1);
    assert!(stderr.is_empty(), "{stderr}");
    shot(&session, &names[1]).assert_all([0, 255, 0], "put back once there");
    shot(&session, &names[0]).assert_all(rgb(YELLOW), "the set stands");
    assert_eq!(showing(&session).0[1].1, image_of(&p));

    // The image for every output, missing; a `set` for one output; the
    // file appears: every other output shows it, the set one keeps its set.
    let q = shared.0.join("q.png");
    let s2 = json!({"image": q.to_str().unwrap()}).to_string();
    exits_1_naming(&session, &s2, &q);
    ok(&session, &["set", RED, "--output", &names[1]]);
    exits_1_naming(&session, &s2, &q);
    write_png(&q, [0, 0, 255]);
    apply(&session, "scoot", &s2);
    shot(&session, &names[0]).assert_all([0, 0, 255], "every output's, put back");
    shot(&session, &names[1]).assert_all(rgb(RED), "the set for this one stands");

    // A `set` on the very output the missing image was for replaces it:
    // nothing is reported any more, and the file coming back changes
    // nothing.
    let r = shared.0.join("r.png");
    let s3 = json!({"color": GREEN, "output": {names[1].clone(): {"image": r.to_str().unwrap()}}})
        .to_string();
    exits_1_naming(&session, &s3, &r);
    ok(&session, &["set", YELLOW, "--output", &names[1]]);
    apply(&session, "scoot", &s3);
    write_png(&r, [255, 0, 255]);
    apply(&session, "scoot", &s3);
    shot(&session, &names[1]).assert_all(rgb(YELLOW), "the set stands over the section");
    shot(&session, &names[0]).assert_all(rgb(GREEN), "the rest of the section");

    // And across a restart: the file gone at start, back by the next
    // (unchanged) reload.
    let t = shared.0.join("t.png");
    let s4 = json!({"image": t.to_str().unwrap()}).to_string();
    exits_1_naming(&session, &s4, &t);
    let session = restart(session, "ac-backb");
    exits_1_naming(&session, &s4, &t);
    write_png(&t, [0, 255, 255]);
    apply(&session, "scoot", &s4);
    configured_names(&session, 2);
    for name in &names {
        shot(&session, name).assert_all([0, 255, 255], "back after a restart");
    }
}

/// A section refused by validation is a usage error (2) that starts
/// nothing and changes nothing; so is malformed or non-UTF-8 JSON, and JSON
/// over the size limit.
#[test]
fn a_bad_section_changes_nothing() {
    let shared = Scratch::new("ac-bad");
    let Some(session) = start("ac-bad", &shared) else {
        return;
    };
    let huge = format!(r#"{{"image":"/{}"}}"#, "a".repeat(70_000));
    for json in [
        r#"{"imgae":"/a.png"}"#,
        r#"{"image":"a.png"}"#,
        r#"{"mode":"fit"}"#,
        r##"{"color":"#ff0000","color":"#00ff00"}"##,
        "{",
        "[]",
        huge.as_str(),
    ] {
        let run = run_apply(&session, "scoot", json);
        assert_eq!(run.status.code(), Some(2), "{json}: {}", run.stderr);
        assert!(run.stderr.contains("apply-config"), "{}", run.stderr);
    }
    use std::os::unix::ffi::OsStrExt;
    let status = session
        .scootbg()
        .arg("apply-config")
        .arg(std::ffi::OsStr::from_bytes(b"{\"image\":\"/\xff\"}"))
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
    assert!(!answers(&session.socket()), "nothing started");
    assert!(!shared.0.join("scootbg").exists(), "nothing written");
}

// ---- `{}` with no daemon ---------------------------------------------------

/// `apply-config '{}'` with no daemon records the clear and the
/// fingerprint in the profile's state and starts nothing; a later
/// `scootbg daemon --profile scoot` shows nothing, as the config asked;
/// re-adding A then shows A.
#[test]
fn an_empty_section_with_no_daemon_starts_none() {
    let shared = Scratch::new("ac-empty");
    let Some(session) = start("ac-emp", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    ok(&session, &["set", BLUE]);
    ok(&session, &["kill"]);
    wait_daemon_gone(&session);

    let stderr = apply(&session, "scoot", "{}");
    assert!(stderr.is_empty(), "{stderr}");
    assert!(!answers(&session.socket()), "no daemon was started");
    assert!(!lock_held(&session));
    let text = state_text(&shared.0, "scoot");
    assert!(
        text.contains("all clear\n") && !text.contains("color"),
        "{text}"
    );
    let recorded = std::fs::metadata(shared.0.join("scootbg/scoot"))
        .unwrap()
        .modified()
        .unwrap();
    // Again: already recorded, nothing written.
    apply(&session, "scoot", r#"{"command":"scootbg"}"#);
    assert_eq!(
        std::fs::metadata(shared.0.join("scootbg/scoot"))
            .unwrap()
            .modified()
            .unwrap(),
        recorded
    );

    let mut daemon = session.daemon_with(&["--profile", "scoot"]);
    let names = configured_names(&session, 2);
    assert!(showing(&session).0.iter().all(|(_, s)| s.is_null()));
    ok(&session, &["kill"]);
    wait_exit(&mut daemon);
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    both_show(&session, RED, "re-added");
    drop(names);
}

// ---- Races ---------------------------------------------------------------

/// Eight `apply-config`s at once with no daemon: one daemon runs, every
/// run exits 0, and the section shows.
#[test]
fn racing_starts_make_one_daemon() {
    let shared = Scratch::new("ac-race");
    let Some(session) = start("ac-race", &shared) else {
        return;
    };
    let json = section(RED);
    std::thread::scope(|scope| {
        let runs: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| run_apply(&session, "scoot", &json)))
            .collect();
        for run in runs {
            let run = run.join().unwrap();
            assert!(run.status.success(), "{}", run.stderr);
        }
    });
    configured_names(&session, 2);
    both_show(&session, RED, "raced");
    wait_one_daemon(&session);
}

/// An `apply-config` sent while the daemon is starting (its socket bound,
/// its loop not yet serving) waits in the backlog and is answered; so is
/// one racing a plain `scootbg daemon`.
#[test]
fn apply_config_while_a_daemon_starts() {
    let shared = Scratch::new("ac-starting");
    let Some(session) = start("ac-sta", &shared) else {
        return;
    };
    let mut daemon = session
        .scootbg()
        .args(["daemon", "--profile", "scoot"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Straight away: the daemon may hold its lock, have bound, or not yet.
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    both_show(&session, RED, "answered by the starting daemon");
    wait_one_daemon(&session);
    ok(&session, &["kill"]);
    wait_exit(&mut daemon);
}

/// `set` and `apply-config` at once, many times: whichever the daemon
/// takes last is on screen, and the state file agrees with the screen.
#[test]
fn a_set_racing_apply_config_stays_consistent() {
    let shared = Scratch::new("ac-set-race");
    let Some(session) = start("ac-sr", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    let names = configured_names(&session, 2);
    for round in 0..10 {
        let json = section(if round % 2 == 0 { GREEN } else { RED });
        std::thread::scope(|scope| {
            let set = scope.spawn(|| ok(&session, &["set", BLUE]));
            apply(&session, "scoot", &json);
            set.join().unwrap();
        });
        let (outputs, _) = showing(&session);
        let shown = outputs[0].1["color"].as_str().unwrap().to_owned();
        assert!(outputs.iter().all(|(_, s)| s["color"] == shown.as_str()));
        shot(&session, &names[0]).assert_all(rgb(&shown), "round");
        // The file follows: wait for the background write.
        let deadline = Instant::now() + PATIENCE;
        while !state_text(&shared.0, "scoot").contains(&format!("all color {shown}\n")) {
            assert!(Instant::now() < deadline, "the file never agreed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A started daemon that lost the lock to something that then went away
/// without serving (here the test itself, holding the lock as a `{}`
/// being recorded does) becomes the daemon itself, rather than drop the
/// section (review of PR #293, F3).
#[test]
fn a_loser_whose_winner_never_serves_becomes_the_daemon() {
    let shared = Scratch::new("ac-loser");
    let Some(session) = start("ac-los", &shared) else {
        return;
    };
    let lock_path = session
        .scratch
        .0
        .join(format!("scootbg-{}.lock", session.wayland_display));
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let log = session.scratch.0.join("loser.log");
    let mut loser = session
        .scootbg()
        .args([
            "apply-config",
            "--serve",
            "--profile",
            "scoot",
            &section(RED),
        ])
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        loser.try_wait().unwrap().is_none(),
        "it waits for the winner"
    );
    drop(lock);
    let deadline = Instant::now() + PATIENCE;
    while !answers(&session.socket()) {
        assert!(
            loser.try_wait().unwrap().is_none(),
            "it gave up: {}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        assert!(Instant::now() < deadline, "it never became the daemon");
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_all_show(&session, 2, &color(RED), "the loser's section shown");
    both_show(&session, RED, "the loser's section");
    ok(&session, &["kill"]);
    assert!(wait_exit(&mut loser).success());
}

/// Profiles adopted back and forth quickly: each state file ends as the
/// last change made in it (review of PR #293, F2). A consistency check
/// only: on this disk a save takes about 0.3 ms and each `apply-config` a
/// few, so it passed with the fix taken out, three runs of three; the
/// writer handover itself is `state::tests`'
/// `a_writer_is_taken_back_only_for_its_own_profile`.
#[test]
fn rapid_adoptions_keep_each_file_newest() {
    let shared = Scratch::new("ac-aba");
    let Some(session) = start("ac-aba", &shared) else {
        return;
    };
    apply(&session, "a", &section(RED));
    configured_names(&session, 2);
    let mut last_a = String::new();
    for round in 0..20u8 {
        let hex = format!("#{round:02x}00{:02x}", 255 - round);
        ok(&session, &["set", &hex]);
        last_a = hex;
        apply(&session, "b", &section(BLUE));
        apply(&session, "a", &section(RED));
    }
    ok(&session, &["kill"]);
    wait_daemon_gone(&session);
    let a = state_text(&shared.0, "a");
    assert!(a.contains(&format!("all color {last_a}\n")), "{a}");
    let b = state_text(&shared.0, "b");
    assert!(b.contains(&format!("all color {BLUE}\n")), "{b}");
}

// ---- Failures --------------------------------------------------------------

/// No compositor to connect to: the daemon apply-config starts fails, and
/// apply-config says so (exit 1) promptly, leaving no daemon behind.
#[test]
fn no_compositor_is_a_prompt_failure() {
    let scratch = Scratch::new("ac-nocomp");
    let started = Instant::now();
    let log = scratch.0.join("apply.log");
    let status = Command::new(common::scootbg_bin())
        .args([
            "apply-config",
            "--profile",
            "scoot",
            r##"{"color":"#ff0000"}"##,
        ])
        .env("XDG_RUNTIME_DIR", &scratch.0)
        .env("WAYLAND_DISPLAY", "wayland-nothing")
        .env("XDG_STATE_HOME", scratch.0.join("state"))
        .stdin(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .status()
        .unwrap();
    let stderr = std::fs::read_to_string(&log).unwrap();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        stderr.contains("cannot connect to the Wayland compositor"),
        "{stderr}"
    );
    assert!(stderr.contains("exited"), "{stderr}");
    assert!(!answers(&scratch.0.join("scootbg-wayland-nothing.sock")));
}

/// The daemon killed while an `apply-config` waits for its reply (an image
/// being decoded): apply-config exits 1 at once, saying so; the next
/// reload is consistent (the section either recorded, and restored, or
/// applied again).
#[test]
fn a_daemon_killed_mid_apply() {
    let shared = Scratch::new("ac-killed");
    let Some(session) = start("ac-kil", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    let big = session.scratch.0.join("big.png");
    write_noise_png(&big, 4000, 3000);
    let json = json!({"image": big.to_str().unwrap()}).to_string();
    let pid = daemon_pid(&session).expect("the daemon's pid");
    let run = std::thread::scope(|scope| {
        let run = scope.spawn(|| run_apply(&session, "scoot", &json));
        // Once the daemon has the request (it records the section at
        // once), and while the image decodes.
        wait_for_state(&shared.0, "scoot", "big.png");
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(pid).unwrap(),
            rustix::process::Signal::KILL,
        )
        .unwrap();
        run.join().unwrap()
    });
    assert_eq!(run.status.code(), Some(1), "{}", run.stderr);
    assert!(run.stderr.contains("before answering"), "{}", run.stderr);
    wait_daemon_gone(&session);
    // The next reload brings it back, with the section.
    apply(&session, "scoot", &json);
    for (name, shows) in showing(&session).0 {
        assert_eq!(shows["image"], big.to_str().unwrap(), "{name}");
    }
}

/// `apply-config` killed while it waits: the daemon carries on and shows
/// the section anyway.
#[test]
fn apply_config_killed_mid_wait_changes_it_all_the_same() {
    let shared = Scratch::new("ac-front");
    let Some(session) = start("ac-fro", &shared) else {
        return;
    };
    apply(&session, "scoot", &section(RED));
    configured_names(&session, 2);
    let big = session.scratch.0.join("big.png");
    write_noise_png(&big, 4000, 3000);
    let json = json!({"image": big.to_str().unwrap()}).to_string();
    let mut child = session
        .scootbg()
        .args(["apply-config", "--profile", "scoot", &json])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_state(&shared.0, "scoot", "big.png");
    child.kill().unwrap();
    child.wait().unwrap();
    session.query_until("the image shown", |outputs| {
        outputs
            .iter()
            .all(|o| o["shows"]["image"] == big.to_str().unwrap())
    });
}

// ---- Another build ----------------------------------------------------------

/// A fake daemon on the socket, answering `version` with `version_reply`
/// and whatever comes next with `apply_reply`; returns what apply-config
/// did, and the lines it sent.
fn against_fake(version_reply: &str, apply_reply: &str) -> (Run, Vec<String>) {
    let scratch = Scratch::new("ac-fake");
    let socket = scratch.0.join("scootbg-wayland-9.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (version_reply, apply_reply) = (version_reply.to_owned(), apply_reply.to_owned());
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut reader = BufReader::new(&stream);
        let mut lines = Vec::new();
        for reply in [version_reply, apply_reply] {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            lines.push(line);
            let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
        }
        lines
    });
    let log = scratch.0.join("apply.log");
    let status = Command::new(common::scootbg_bin())
        .args([
            "apply-config",
            "--profile",
            "scoot",
            r##"{"color":"#ff0000"}"##,
        ])
        .env("XDG_RUNTIME_DIR", &scratch.0)
        .env("WAYLAND_DISPLAY", "wayland-9")
        .env("XDG_STATE_HOME", scratch.0.join("state"))
        .stdin(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .status()
        .unwrap();
    let lines = server.join().unwrap();
    let stderr = std::fs::read_to_string(&log).unwrap();
    (Run { status, stderr }, lines)
}

#[test]
fn another_version_is_a_warning() {
    let (run, lines) = against_fake(
        r#"{"type":"version","protocol":1,"version":"0.0.1-old"}"#,
        r#"{"type":"ok"}"#,
    );
    assert_eq!(run.status.code(), Some(0), "{}", run.stderr);
    assert!(
        run.stderr.contains("warning") && run.stderr.contains("0.0.1-old"),
        "{}",
        run.stderr
    );
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(r#""type":"version""#), "{lines:?}");
    assert_eq!(
        lines[1],
        "{\"protocol\":1,\"type\":\"apply-config\",\"profile\":\"scoot\",\
         \"config\":{\"color\":\"#ff0000\"}}\n"
    );
}

#[test]
fn another_protocol_is_an_error() {
    let (run, _) = against_fake(
        r#"{"type":"error","message":"protocol mismatch: the request speaks 1, this daemon speaks 2"}"#,
        r#"{"type":"error","message":"protocol mismatch"}"#,
    );
    assert_eq!(run.status.code(), Some(1));
    assert!(
        run.stderr.contains("protocol mismatch") && run.stderr.contains("scootbg kill"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_daemon_that_predates_apply_config_is_an_error() {
    let (run, _) = against_fake(
        r#"{"type":"version","protocol":1,"version":"0.0.1-old"}"#,
        r#"{"type":"error","message":"unknown request `apply-config`"}"#,
    );
    assert_eq!(run.status.code(), Some(1));
    assert!(
        run.stderr.contains("predates apply-config") && run.stderr.contains("0.0.1-old"),
        "{}",
        run.stderr
    );
}

// ---- Helpers ----------------------------------------------------------------

/// The pids of this binary's processes that are, or may become, this
/// session's daemon: `daemon` or `apply-config --serve`, with its scratch
/// directory as `XDG_RUNTIME_DIR`.
fn daemon_pids(session: &Session) -> Vec<i32> {
    let binary = common::scootbg_bin();
    let runtime = format!("XDG_RUNTIME_DIR={}", session.scratch.0.display());
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if exe != binary {
            continue;
        }
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
        let serving = args.iter().any(|a| *a == b"--serve") || args.get(1) == Some(&&b"daemon"[..]);
        if !serving {
            continue;
        }
        let environ = std::fs::read(entry.path().join("environ")).unwrap_or_default();
        if environ
            .split(|&b| b == 0)
            .any(|var| var == runtime.as_bytes())
        {
            pids.push(pid);
        }
    }
    pids
}

fn daemon_pid(session: &Session) -> Option<i32> {
    // A zombie of an exited loser is not a daemon: only live ones.
    daemon_pids(session).into_iter().find(|pid| is_live(*pid))
}

fn write_png(path: &Path, rgb: [u8; 3]) {
    let (width, height) = (64, 40);
    let data: Vec<u8> = (0..width * height).flat_map(|_| rgb).collect();
    encode_png(path, width, height, &data);
}

/// A noisy PNG (so it compresses badly and takes a while to decode).
fn write_noise_png(path: &Path, width: u32, height: u32) {
    let mut seed: u32 = 0x1234_5678;
    let data: Vec<u8> = (0..width * height * 3)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    encode_png(path, width, height, &data);
}

fn encode_png(path: &Path, width: u32, height: u32, data: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
}

/// The output names, from scoot itself (no daemon needed).
fn scoot_names(session: &Session) -> Vec<String> {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    let mut names: Vec<String> = reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

/// Waits until one live daemon serves the session (a racer that lost
/// exits once it has forwarded its section).
fn wait_one_daemon(session: &Session) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let live = daemon_pids(session)
            .into_iter()
            .filter(|pid| is_live(*pid))
            .count();
        if live == 1 {
            return;
        }
        assert!(Instant::now() < deadline, "{live} daemons, not one");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn is_live(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| !stat.contains(") Z "))
        .unwrap_or(false)
}

/// Waits until `profile`'s state file mentions `text`.
fn wait_for_state(state_home: &Path, profile: &str, text: &str) {
    let deadline = Instant::now() + PATIENCE;
    while !state_text(state_home, profile).contains(text) {
        assert!(Instant::now() < deadline, "the state never said {text:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
