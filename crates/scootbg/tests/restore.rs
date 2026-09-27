//! Restoring the last wallpaper at start-up, end to end on a real `scoot
//! --headless`: `set`, stop the daemon, start it again, and the same
//! wallpaper is on screen (checked by screenshot); `--no-restore`; a moved
//! image; choices for an output that is not plugged in surviving; profiles
//! kept apart. Each session's `XDG_STATE_HOME` is a scratch directory
//! (`common::Session::state_home`), shared between sessions only where a
//! test says so.
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use common::{PATIENCE, Scratch, Session, Shot, rgb, stderr_of, wait_exit};
use serde_json::{Value, json};

const RED: &str = "#ff0000";
const BLUE: &str = "#0000ff";
const GREEN: &str = "#00ff00";
const YELLOW: &str = "#ffff00";

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

/// Stops the daemon with `scootbg kill`, which returns once it is gone
/// (and its state written).
fn stop(session: &Session, daemon: &mut Child) -> String {
    ok(session, &["kill"]);
    assert!(wait_exit(daemon).success());
    stderr_of(daemon)
}

/// The output names, once all `count` are listed and configured.
fn configured_names(session: &Session, count: usize) -> Vec<String> {
    session
        .query_until("all configured", |o| {
            o.len() == count && o.iter().all(|o| o["surface"]["state"] == "configured")
        })
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Waits until each output shows what `want` says (by name).
fn shows(session: &Session, want: &[(&str, Value)]) {
    session.query_until("showing what was restored", |outputs| {
        outputs.len() == want.len()
            && want.iter().all(|(name, shows)| {
                outputs
                    .iter()
                    .any(|o| o["name"] == *name && o["shows"] == *shows)
            })
    });
}

fn color(hex: &str) -> Value {
    json!({ "color": hex })
}

fn image(path: &Path) -> Value {
    json!({
        "image": path.to_str().unwrap(),
        "mode": "fill",
        "fill": "#000000",
        "filter": "lanczos3",
    })
}

/// A screenshot of the output named `name` (scoot's own ids, looked up).
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

/// A flat PNG of `rgb` at `path`.
fn write_png(path: &Path, rgb: [u8; 3]) {
    let (width, height) = (64, 40);
    let data: Vec<u8> = (0..width * height).flat_map(|_| rgb).collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
}

fn state_file(session: &Session, profile: &str) -> PathBuf {
    session.state_home.join("scootbg").join(profile)
}

fn read_state(session: &Session, profile: &str) -> String {
    std::fs::read_to_string(state_file(session, profile)).unwrap_or_default()
}

/// Waits until the state file holds `line`.
fn saved(session: &Session, profile: &str, line: &str) {
    let deadline = Instant::now() + PATIENCE;
    while !read_state(session, profile).lines().any(|l| l == line) {
        assert!(
            Instant::now() < deadline,
            "never saved {line:?}: {:?}",
            read_state(session, profile)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `set` a color and an image, stop the daemon, start it again: the same
/// wallpaper, by screenshot, with nothing on stderr. Once through `kill`,
/// once through SIGTERM (the file is written in the background: waited
/// for here, then the daemon killed where it stands).
#[test]
fn a_set_survives_a_restart() {
    let Some(session) = Session::start("rs-again") else {
        return;
    };
    let picture = session.scratch.0.join("a picture #1.png");
    write_png(&picture, [0, 0, 255]);
    let mut daemon = session.daemon();
    let names = configured_names(&session, 2);
    let background = shot(&session, &names[0]).at(10, 10);
    ok(&session, &["set", RED]);
    ok(
        &session,
        &["set", picture.to_str().unwrap(), "--output", &names[1]],
    );
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.is_empty(), "{stderr}");
    let text = read_state(&session, "default");
    assert!(
        text.starts_with("scootbg-state 1\nprofile default\n"),
        "{text}"
    );
    assert!(text.contains("a%20picture%20#1.png"), "escaped: {text}");

    let want = [
        (names[0].as_str(), color(RED)),
        (names[1].as_str(), image(&picture)),
    ];
    for round in ["kill", "SIGTERM"] {
        let mut daemon = session.daemon();
        shows(&session, &want);
        shot(&session, &names[0]).assert_all(rgb(RED), round);
        shot(&session, &names[1]).assert_all([0, 0, 255], round);
        assert_ne!(background, rgb(RED));
        // A restore writes nothing.
        assert_eq!(read_state(&session, "default"), text, "{round}");
        if round == "kill" {
            let stderr = stop(&session, &mut daemon);
            assert!(stderr.is_empty(), "{stderr}");
        } else {
            common::signal(&daemon, rustix::process::Signal::TERM);
            wait_exit(&mut daemon);
        }
    }

    // A change after a restore is saved too, and restored next time.
    let mut daemon = session.daemon();
    shows(&session, &want);
    ok(&session, &["set", GREEN, "--output", &names[0]]);
    saved(
        &session,
        "default",
        &format!("output {} color {GREEN}", names[0]),
    );
    common::signal(&daemon, rustix::process::Signal::TERM);
    wait_exit(&mut daemon);
    let mut daemon = session.daemon();
    shows(
        &session,
        &[
            (names[0].as_str(), color(GREEN)),
            (names[1].as_str(), image(&picture)),
        ],
    );
    shot(&session, &names[0]).assert_all(rgb(GREEN), "changed after a restore");
    // `clear` is saved as well: the next start shows nothing.
    ok(&session, &["clear"]);
    stop(&session, &mut daemon);
    assert_eq!(
        read_state(&session, "default"),
        "scootbg-state 1\nprofile default\nall clear\n"
    );
    let mut daemon = session.daemon();
    configured_names(&session, 2);
    shows(
        &session,
        &[
            (names[0].as_str(), Value::Null),
            (names[1].as_str(), Value::Null),
        ],
    );
    shot(&session, &names[1]).assert_all(background, "cleared, restored");
    stop(&session, &mut daemon);
}

/// `--no-restore` shows nothing, but keeps the state: a `set` then updates
/// it (keeping what it does not replace), and a later plain start shows
/// the lot.
#[test]
fn no_restore_starts_blank_and_keeps_the_state() {
    let Some(session) = Session::start("rs-none") else {
        return;
    };
    let mut daemon = session.daemon();
    let names = configured_names(&session, 2);
    let background = shot(&session, &names[0]).at(10, 10);
    ok(&session, &["set", RED]);
    stop(&session, &mut daemon);

    let mut daemon = session.daemon_with(&["--no-restore"]);
    names_configured_showing_nothing(&session, &names);
    for name in &names {
        shot(&session, name).assert_all(background, "not restored");
    }
    ok(&session, &["set", BLUE, "--output", &names[1]]);
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.is_empty(), "{stderr}");
    let text = read_state(&session, "default");
    assert!(text.contains(&format!("all color {RED}\n")), "kept: {text}");
    assert!(
        text.contains(&format!("output {} color {BLUE}\n", names[1])),
        "{text}"
    );

    let mut daemon = session.daemon();
    shows(
        &session,
        &[
            (names[0].as_str(), color(RED)),
            (names[1].as_str(), color(BLUE)),
        ],
    );
    shot(&session, &names[0]).assert_all(rgb(RED), "restored after --no-restore");
    shot(&session, &names[1]).assert_all(rgb(BLUE), "and the change made then");
    stop(&session, &mut daemon);
}

fn names_configured_showing_nothing(session: &Session, names: &[String]) {
    let want: Vec<(&str, Value)> = names.iter().map(|n| (n.as_str(), Value::Null)).collect();
    shows(session, &want);
    // Nothing is on its way either: still nothing a moment later.
    std::thread::sleep(Duration::from_millis(200));
    shows(session, &want);
}

/// A saved image that has moved: the daemon starts anyway, says why on
/// stderr, and that output shows the compositor's background; the entry
/// stays saved (another output's `set` does not drop it), so once the file
/// is back it is restored.
#[test]
fn a_moved_image_falls_back_and_stays_saved() {
    let Some(session) = Session::start("rs-moved") else {
        return;
    };
    let picture = session.scratch.0.join("hills.png");
    let moved = session.scratch.0.join("moved.png");
    write_png(&picture, [0, 255, 0]);
    let mut daemon = session.daemon();
    let names = configured_names(&session, 2);
    let background = shot(&session, &names[0]).at(10, 10);
    ok(&session, &["set", picture.to_str().unwrap()]);
    stop(&session, &mut daemon);
    std::fs::rename(&picture, &moved).unwrap();

    let mut daemon = session.daemon();
    names_configured_showing_nothing(&session, &names);
    for name in &names {
        shot(&session, name).assert_all(background, "moved: the compositor's own");
    }
    ok(&session, &["set", YELLOW, "--output", &names[1]]);
    shot(&session, &names[1]).assert_all(rgb(YELLOW), "a set still works");
    let stderr = stop(&session, &mut daemon);
    assert!(
        stderr.contains("cannot restore") && stderr.contains("hills.png"),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "one warning: {stderr}");
    let text = read_state(&session, "default");
    assert!(text.contains("all image "), "the moved entry stays: {text}");

    std::fs::rename(&moved, &picture).unwrap();
    let mut daemon = session.daemon();
    shows(
        &session,
        &[
            (names[0].as_str(), image(&picture)),
            (names[1].as_str(), color(YELLOW)),
        ],
    );
    shot(&session, &names[0]).assert_all([0, 255, 0], "back, restored");
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.is_empty(), "{stderr}");
}

/// A choice for an output that is not plugged in now survives a restore
/// and a `set` for the outputs that are: one state directory across a
/// two-output session, a one-output session, and two outputs again.
#[test]
fn a_disconnected_outputs_entry_survives() {
    let shared = Scratch::new("rs-shared");
    let second = {
        let Some(mut session) = Session::start_with("rs-two", 2, "") else {
            return;
        };
        session.state_home = shared.0.clone();
        let mut daemon = session.daemon();
        let names = configured_names(&session, 2);
        ok(&session, &["set", RED]);
        ok(&session, &["set", BLUE, "--output", &names[1]]);
        stop(&session, &mut daemon);
        names[1].clone()
    };
    let first = {
        let Some(mut session) = Session::start_with("rs-one", 1, "") else {
            return;
        };
        session.state_home = shared.0.clone();
        let mut daemon = session.daemon();
        let names = configured_names(&session, 1);
        assert_ne!(names[0], second, "the second output is not here");
        shows(&session, &[(names[0].as_str(), color(RED))]);
        shot(&session, &names[0]).assert_all(rgb(RED), "one output");
        ok(&session, &["set", GREEN, "--output", &names[0]]);
        let stderr = stop(&session, &mut daemon);
        assert!(stderr.is_empty(), "{stderr}");
        let text = read_state(&session, "default");
        assert!(
            text.contains(&format!("output {second} color {BLUE}\n")),
            "the unplugged output's entry: {text}"
        );
        names[0].clone()
    };
    let Some(mut session) = Session::start_with("rs-again", 2, "") else {
        return;
    };
    session.state_home = shared.0.clone();
    let mut daemon = session.daemon();
    configured_names(&session, 2);
    shows(
        &session,
        &[
            (first.as_str(), color(GREEN)),
            (second.as_str(), color(BLUE)),
        ],
    );
    shot(&session, &first).assert_all(rgb(GREEN), "set while alone");
    shot(&session, &second).assert_all(rgb(BLUE), "kept while unplugged");
    stop(&session, &mut daemon);
}

/// Two profiles, two files: neither restores the other's wallpaper.
#[test]
fn profiles_do_not_share_state() {
    let Some(session) = Session::start_with("rs-prof", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_with(&["--profile", "scoot"]);
    let names = configured_names(&session, 1);
    let background = shot(&session, &names[0]).at(10, 10);
    ok(&session, &["set", RED]);
    stop(&session, &mut daemon);

    let mut daemon = session.daemon_with(&["--profile=sway"]);
    names_configured_showing_nothing(&session, &names);
    shot(&session, &names[0]).assert_all(background, "another profile's");
    ok(&session, &["set", BLUE]);
    stop(&session, &mut daemon);

    let mut daemon = session.daemon_with(&["--profile", "scoot"]);
    shows(&session, &[(names[0].as_str(), color(RED))]);
    shot(&session, &names[0]).assert_all(rgb(RED), "its own profile's");
    stop(&session, &mut daemon);
    let mut daemon = session.daemon_with(&["--profile", "sway"]);
    shows(&session, &[(names[0].as_str(), color(BLUE))]);
    stop(&session, &mut daemon);
    // And the default profile has nothing of either.
    assert!(!state_file(&session, "default").exists());
    assert!(read_state(&session, "scoot").contains("profile scoot\n"));
    assert!(read_state(&session, "sway").contains("profile sway\n"));

    // A bad name is a usage error before anything starts.
    let out = session.run(&["daemon", "--profile", "../escape"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!session.state_home.join("escape").exists());
}

/// A state file the daemon cannot use never stops it: a newer format is
/// left alone (not restored, never written over), a broken one is
/// replaced at the next `set`.
#[test]
fn an_unusable_state_file_never_stops_the_daemon() {
    let Some(session) = Session::start_with("rs-bad", 1, "") else {
        return;
    };
    let file = state_file(&session, "default");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let newer = "scootbg-state 2\nall color #ff0000\n";
    std::fs::write(&file, newer).unwrap();
    let mut daemon = session.daemon();
    let names = configured_names(&session, 1);
    names_configured_showing_nothing(&session, &names);
    ok(&session, &["set", BLUE]);
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.contains("version 2"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), newer, "left alone");

    std::fs::write(&file, "garbage\n\u{0}\u{ff}").unwrap();
    let mut daemon = session.daemon();
    names_configured_showing_nothing(&session, &names);
    ok(&session, &["set", BLUE]);
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.contains("first line"), "{stderr}");
    assert!(
        read_state(&session, "default").contains("all color #0000ff\n"),
        "replaced"
    );
    let mut daemon = session.daemon();
    shows(&session, &[(names[0].as_str(), color(BLUE))]);
    stop(&session, &mut daemon);
}

/// Counts the times a file is opened, by anyone (inotify `IN_OPEN`): how
/// many times the daemon read an image, so decoded it.
///
/// Closes are watched too, though not counted: inotify merges an event
/// into the one before it when they are identical and the first is unread,
/// so two opens back to back would read as one; with each open's close
/// between them, no two in a row are alike.
struct Opens(std::os::fd::OwnedFd);

impl Opens {
    fn watch(path: &Path) -> Self {
        use rustix::fs::inotify::{CreateFlags, WatchFlags, add_watch, init};
        let fd = init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).unwrap();
        add_watch(&fd, path, WatchFlags::OPEN | WatchFlags::CLOSE_NOWRITE).unwrap();
        Self(fd)
    }

    /// The opens so far (the queue is drained).
    fn count(&self) -> usize {
        use rustix::fs::inotify::{ReadFlags, Reader};
        let mut buf = [std::mem::MaybeUninit::uninit(); 4096];
        let mut reader = Reader::new(&self.0, &mut buf);
        let mut opens = 0;
        while let Ok(event) = reader.next() {
            if event.events().contains(ReadFlags::OPEN) {
                opens += 1;
            }
        }
        opens
    }
}

/// A restored image is decoded once, when its output is configured: not
/// once to check it and again to draw it.
#[test]
fn a_restored_image_is_decoded_once() {
    let Some(session) = Session::start_with("rs-once", 1, "") else {
        return;
    };
    let picture = session.scratch.0.join("once.png");
    write_png(&picture, [0, 0, 255]);
    let mut daemon = session.daemon();
    let names = configured_names(&session, 1);
    ok(&session, &["set", picture.to_str().unwrap()]);
    stop(&session, &mut daemon);

    let opens = Opens::watch(&picture);
    let mut daemon = session.daemon();
    shows(&session, &[(names[0].as_str(), image(&picture))]);
    assert_eq!(opens.count(), 1, "one decode for the restore");
    stop(&session, &mut daemon);
}

/// An image `set` sent the moment the daemon's socket exists, before its
/// outputs are configured (what a session's autostart does), is decoded
/// once: it waits for the `configure` rather than decode once to check the
/// file and again for the output (ticket 8 counted two opens).
#[test]
fn an_image_set_as_the_daemon_starts_is_decoded_once() {
    let Some(session) = Session::start_with("rs-early", 1, "") else {
        return;
    };
    let picture = session.scratch.0.join("early.png");
    write_png(&picture, [0, 255, 0]);
    for round in 0..3 {
        let opens = Opens::watch(&picture);
        let mut daemon = session
            .scootbg()
            .args(["daemon", "--no-restore"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // The socket listens before the daemon connects to the
        // compositor; a client connecting now waits in its backlog, and
        // its request is read on the loop's first turns, before any
        // output is configured. From this process, not a spawned
        // `scootbg set`, which would take long enough to start that the
        // outputs could be configured first.
        let deadline = Instant::now() + PATIENCE;
        let stream = loop {
            if let Ok(stream) = UnixStream::connect(session.socket()) {
                break stream;
            }
            assert!(Instant::now() < deadline, "the daemon never listened");
            std::thread::sleep(Duration::from_micros(100));
        };
        let request = json!({"protocol": 1, "type": "set", "image": picture});
        (&stream)
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).unwrap();
        assert_eq!(reply, "{\"type\":\"ok\"}\n");
        shot(&session, &configured_names(&session, 1)[0]).assert_all([0, 255, 0], "shown");
        assert_eq!(opens.count(), 1, "round {round}: one decode");
        let stderr = stop(&session, &mut daemon);
        assert!(stderr.is_empty(), "{stderr}");
    }
}

/// A restored image on two outputs of different sizes (sway: 1280×720 and
/// 1920×1080) is decoded once for both: their `configure`s come in one
/// batch, so both sizes are asked of one render job (`crate::jobs` says
/// why renders need no hold of their own; this is the check).
#[test]
fn a_restore_on_outputs_of_two_sizes_decodes_once_on_sway() {
    let Some(session) = Session::sway("rs-sizes") else {
        return;
    };
    let picture = session.scratch.0.join("sizes.png");
    write_png(&picture, [0, 0, 255]);
    let mut daemon = session.daemon();
    configured_names(&session, 1);
    session.swaymsg(&["create_output"]);
    let names = configured_names(&session, 2);
    let sizes: Vec<Value> = session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["surface"]["size"].clone())
        .collect();
    assert_ne!(sizes[0], sizes[1], "two sizes: {sizes:?}");
    ok(&session, &["set", picture.to_str().unwrap()]);
    stop(&session, &mut daemon);

    let opens = Opens::watch(&picture);
    let mut daemon = session.daemon();
    shows(
        &session,
        &[
            (names[0].as_str(), image(&picture)),
            (names[1].as_str(), image(&picture)),
        ],
    );
    for name in &names {
        session
            .screencopy(name)
            .assert_all([0, 0, 255], "restored on both");
    }
    assert_eq!(opens.count(), 1, "one decode for both sizes");
    let stderr = stop(&session, &mut daemon);
    assert!(stderr.is_empty(), "{stderr}");
}
