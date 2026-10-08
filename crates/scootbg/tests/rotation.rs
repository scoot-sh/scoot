//! Slideshows end to end: `set DIR --every` against a real
//! `scoot --headless`, checked by `query` (and real pixels for the first
//! file). The timer's advance runs on a debug-shortened interval
//! (`SCOOTBG_DEBUG_ROTATION_EVERY`, debug builds only); the release
//! minimum of a minute is covered by the CLI and protocol unit tests.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{PATIENCE, Session};
use serde_json::{Value, json};

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

/// `scootbg ARGS`, asserting it fails with `code`; returns its stderr.
fn fails(session: &Session, code: i32, args: &[&str]) -> String {
    let out = session.run(args);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {stderr}");
    stderr
}

fn shows(session: &Session) -> Vec<Value> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["shows"].clone())
        .collect()
}

fn rotation(session: &Session) -> Value {
    session.query()["rotation"].clone()
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn write_png(path: &Path, width: u32, height: u32, rgb: [u8; 3]) {
    let data: Vec<u8> = (0..width * height).flat_map(|_| rgb).collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
}

/// A directory of two solid images, sorted red then blue.
fn two_images(dir: &Path) -> (String, String) {
    std::fs::create_dir_all(dir).unwrap();
    let red = dir.join("01-red.png");
    let blue = dir.join("02-blue.png");
    write_png(&red, 64, 64, [255, 0, 0]);
    write_png(&blue, 64, 64, [0, 0, 255]);
    (path_str(&red).to_owned(), path_str(&blue).to_owned())
}

fn kill(session: &Session, daemon: &mut std::process::Child) {
    assert!(session.run(&["kill"]).status.success());
    assert!(common::wait_exit(daemon).success());
}

/// Voluntary plus involuntary context switches of a process.
fn switches(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("ctxt_switches"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

#[test]
fn a_slideshow_starts_reports_and_stops() {
    let Some(session) = Session::start_with("rotation", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, blue) = two_images(&dir);
    let path = path_str(&dir);

    ok(&session, &["set", path, "--every", "30m"]);
    assert_eq!(
        shows(&session)[0],
        json!({"image": red, "mode": "fill", "fill": "#000000", "filter": "lanczos3"})
    );
    assert_eq!(
        rotation(&session),
        json!({"directory": path, "every_secs": 1800, "shuffle": false, "files": 2})
    );
    // Real pixels: the first file, filling the output.
    let id = session.scoot_ipc(r#"{"type":"outputs"}"#)["outputs"][0]["id"]
        .as_u64()
        .unwrap();
    let shot = session.scoot_screenshot(id);
    let at = shot.at(shot.width / 2, shot.height / 2);
    assert!(
        at[0] > 200 && at[1] < 60 && at[2] < 60,
        "the first file shows: {at:?}"
    );

    // A color `set` stops it; `query` no longer reports one.
    ok(&session, &["set", "#101014"]);
    assert_eq!(rotation(&session), Value::Null);
    assert_eq!(shows(&session)[0], json!({"color": "#101014"}));

    // Again, shuffled and scoped to one output.
    let name = session.query()["outputs"][0]["name"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        &session,
        &["set", path, "--every", "2h", "--shuffle", "--output", &name],
    );
    assert_eq!(
        rotation(&session),
        json!({"directory": path, "every_secs": 7200, "shuffle": true, "files": 2})
    );
    // A `clear` stops it too.
    ok(&session, &["clear"]);
    assert_eq!(rotation(&session), Value::Null);
    assert_eq!(shows(&session)[0], Value::Null);
    let _ = blue;

    kill(&session, &mut daemon);
}

#[test]
fn a_slideshow_advances_on_its_timer() {
    let Some(session) = Session::start_with("rotation-advance", 1, "") else {
        return;
    };
    // Debug-shortened to 2 s (release builds have no such knob, and refuse
    // under a minute at the CLI).
    let mut daemon = session.daemon_logged(&[("SCOOTBG_DEBUG_ROTATION_EVERY", "2")]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, blue) = two_images(&dir);

    ok(&session, &["set", path_str(&dir), "--every", "1m"]);
    assert_eq!(shows(&session)[0]["image"], json!(red));
    // The second file, then the first again: it cycles, not once.
    session.query_until("the rotation advances", |o| {
        o[0]["shows"]["image"] == json!(blue)
    });
    session.query_until("the rotation cycles", |o| {
        o[0]["shows"]["image"] == json!(red)
    });
    // Still reported meanwhile.
    assert_eq!(rotation(&session)["files"], json!(2));

    kill(&session, &mut daemon);
}

#[test]
fn misuse_is_refused_and_changes_nothing() {
    let Some(session) = Session::start_with("rotation-misuse", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, _) = two_images(&dir);
    let empty = session.runtime_dir().join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    // Usage errors (exit 2), before any daemon round trip.
    let stderr = fails(&session, 2, &["set", path_str(&dir)]);
    assert!(stderr.contains("--every"), "{stderr}");
    let stderr = fails(&session, 2, &["set", &red, "--every", "30m"]);
    assert!(stderr.contains("not one"), "{stderr}");
    let stderr = fails(&session, 2, &["set", path_str(&dir), "--every", "30s"]);
    assert!(stderr.contains("minute"), "{stderr}");
    // An empty directory reaches the daemon, which refuses it (exit 1).
    let stderr = fails(&session, 1, &["set", path_str(&empty), "--every", "30m"]);
    assert!(stderr.contains("no files"), "{stderr}");
    // Nothing changed anywhere.
    assert_eq!(shows(&session)[0], Value::Null);
    assert_eq!(rotation(&session), Value::Null);

    kill(&session, &mut daemon);
}

#[test]
fn apply_config_stops_a_changed_slideshow_and_keeps_an_unchanged_one() {
    let Some(session) = Session::start_with("rotation-config", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (_, _) = two_images(&dir);

    // A fingerprint on record, so the next identical section is unchanged.
    ok(&session, &["apply-config", "{}"]);
    ok(&session, &["set", path_str(&dir), "--every", "1m"]);
    assert!(rotation(&session).is_object());
    // Unchanged: the slideshow survives, like a `set` made since.
    ok(&session, &["apply-config", "{}"]);
    assert!(rotation(&session).is_object());
    // Changed: the section wins, and the slideshow stops.
    ok(&session, &["apply-config", r##"{"color":"#101014"}"##]);
    assert_eq!(rotation(&session), Value::Null);
    assert_eq!(shows(&session)[0], json!({"color": "#101014"}));

    kill(&session, &mut daemon);
}

#[test]
fn no_slideshow_means_no_wakeups() {
    let Some(session) = Session::start_with("rotation-idle", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (_, _) = two_images(&dir);
    ok(&session, &["set", path_str(&dir), "--every", "30m"]);
    // Back to static: the timer is gone, so the daemon sleeps.
    ok(&session, &["set", "#101014"]);
    let pid = daemon.id();
    let deadline = Instant::now() + PATIENCE;
    let mut last = switches(pid);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = switches(pid);
        if now == last {
            break;
        }
        assert!(Instant::now() < deadline, "the daemon never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(switches(pid), last, "the daemon woke up with no slideshow");
    kill(&session, &mut daemon);
}

/// A refused new slideshow leaves the running one alone: its reply says
/// nothing was changed, `query` still reports it, and it still advances.
/// The refusal is forced by filling the trial queue (32 waiting trials)
/// with bursts of concurrent slideshow `set`s of the same directory
/// behind a slow first decode, so every accepted one agrees on what runs.
/// The burst is retried until the probe is refused: the first decode to
/// land sweeps the queue behind it, so a late probe can miss a drained
/// queue without saying anything about the fix.
#[test]
fn a_refused_second_slideshow_keeps_the_running_one() {
    let Some(session) = Session::start_with("rotation-busy", 1, "") else {
        return;
    };
    // Debug-shortened to 1 s, so the surviving slideshow steps promptly.
    let mut daemon = session.daemon_logged(&[("SCOOTBG_DEBUG_ROTATION_EVERY", "1")]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, _) = two_images(&dir);
    let other = session.runtime_dir().join("elsewhere");
    let _ = two_images(&other);
    // A slow first decode: a 12 MP solid image decodes in milliseconds
    // but scales for hundreds, so every trial queued behind it is still
    // queued when the refused `set` arrives.
    let slow = session.runtime_dir().join("slow");
    std::fs::create_dir_all(&slow).unwrap();
    write_png(&slow.join("00-big.png"), 4000, 3000, [0, 128, 0]);
    write_png(&slow.join("01-red.png"), 64, 64, [255, 0, 0]);

    ok(&session, &["set", path_str(&dir), "--every", "1m"]);
    assert_eq!(shows(&session)[0]["image"], json!(red));

    // Fill the trial queue: far more concurrent slideshow `set`s than the
    // 32 waiting trials allowed, spawned from threads so the burst lands
    // inside one slow decode even under load. Every accepted one names
    // the same directory, so no ordering races; some are refused or lose
    // their connection on the way, and only the queue matters.
    let mut fillers = Vec::new();
    let mut probe_stderr = String::new();
    for _ in 0..6 {
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                handles.push(scope.spawn(|| {
                    let mut made = Vec::new();
                    for _ in 0..5 {
                        made.push(
                            session
                                .scootbg()
                                .args(["set", path_str(&slow), "--every", "1m"])
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::piped())
                                .spawn()
                                .unwrap(),
                        );
                    }
                    made
                }));
            }
            for handle in handles {
                fillers.extend(handle.join().unwrap());
            }
        });
        // The probe, while the queue is still full: refused, changing
        // nothing. A probe that missed the burst was accepted instead
        // (the first decode to land sweeps the queue behind it); blast
        // again behind it.
        let out = session.run(&["set", path_str(&other), "--every", "1m"]);
        probe_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() && probe_stderr.contains("too many images") {
            break;
        }
    }
    assert!(
        probe_stderr.contains("too many images"),
        "never managed a Busy refusal: {probe_stderr}"
    );
    // The running slideshow (the fillers' directory) survived the refusal.
    assert_eq!(
        rotation(&session)["directory"],
        json!(path_str(&slow)),
        "a refused `set` keeps the running slideshow"
    );
    for mut filler in fillers {
        let _ = filler.wait();
    }
    // ... and it still steps on its timer. The step is proven through the
    // state file, not pixels: each step records its file synchronously on
    // the loop, while pixels wait on the decode worker, which this test
    // deliberately keeps saturated.
    let state_file = session.state_home.join("scootbg").join("default");
    let needle = format!("{}/01-red", path_str(&slow));
    let deadline = Instant::now() + PATIENCE;
    loop {
        let text = std::fs::read_to_string(&state_file).unwrap_or_default();
        if text.contains(&needle) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the surviving slideshow never stepped; state: {text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = red;

    kill(&session, &mut daemon);
}

/// An entry the listing cannot read (here a symlink loop) refuses the
/// `set` naming the operating system's reason, not "not a directory",
/// and changes nothing. Exit 1: a daemon refusal, not a usage error.
#[test]
fn unreadable_entries_are_refused_naming_the_cause() {
    let Some(session) = Session::start_with("rotation-unreadable", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, _) = two_images(&dir);
    std::os::unix::fs::symlink("loop", dir.join("loop")).unwrap();

    let stderr = fails(&session, 1, &["set", path_str(&dir), "--every", "30m"]);
    assert!(stderr.contains("nothing was changed"), "{stderr}");
    assert!(!stderr.contains("not a directory"), "{stderr}");
    assert_eq!(shows(&session)[0], Value::Null);
    assert_eq!(rotation(&session), Value::Null);
    let _ = red;

    kill(&session, &mut daemon);
}

/// A directory past the listing cap is refused outright (exit 1, naming
/// the cap), changing nothing.
#[test]
fn a_directory_past_the_listing_cap_is_refused() {
    let Some(session) = Session::start_with("rotation-cap", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..=scootbg_listing_cap() {
        std::fs::write(dir.join(format!("{i:05}.png")), b"fake").unwrap();
    }

    let stderr = fails(&session, 1, &["set", path_str(&dir), "--every", "30m"]);
    assert!(
        stderr.contains(&scootbg_listing_cap().to_string()),
        "{stderr}"
    );
    assert!(stderr.contains("nothing was changed"), "{stderr}");
    assert_eq!(shows(&session)[0], Value::Null);
    assert_eq!(rotation(&session), Value::Null);

    kill(&session, &mut daemon);
}

/// The listing cap, as the daemon enforces it: kept beside the test that
/// needs it so the count cannot drift from the source.
fn scootbg_listing_cap() -> usize {
    10_000
}

/// If the directory goes away mid-rotation, the slideshow stops at the
/// next step instead of failing once a minute until stopped: `query` no
/// longer reports one, the stop says why on stderr, and the daemon sleeps
/// again like with no slideshow at all.
#[test]
fn a_slideshow_stops_when_its_directory_goes_away() {
    let Some(session) = Session::start_with("rotation-gone", 1, "") else {
        return;
    };
    // Debug-shortened to 1 s, so the step past the deletion comes at once.
    let mut daemon = session.daemon_logged(&[("SCOOTBG_DEBUG_ROTATION_EVERY", "1")]);
    configured(&session, 1);
    let dir = session.runtime_dir().join("wallpapers");
    let (red, _) = two_images(&dir);

    ok(&session, &["set", path_str(&dir), "--every", "1m"]);
    assert!(rotation(&session).is_object());
    // The first file is on screen before the directory goes (the `set`
    // waits for it); whatever follows, no later step draws anything new.
    assert_eq!(shows(&session)[0]["image"], json!(red));
    std::fs::remove_dir_all(&dir).unwrap();
    session.query_until("the slideshow stops", |_| rotation(&session) == Value::Null);
    // The stop says why on stderr: a vanished directory ends the show
    // instead of failing once a minute until stopped.
    let log = std::fs::read_to_string(session.daemon_log()).unwrap_or_default();
    assert!(log.contains("stopping the slideshow"), "{log}");
    // And the timer is gone: the daemon settles back to no wakeups.
    let pid = daemon.id();
    let deadline = Instant::now() + PATIENCE;
    let mut last = switches(pid);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = switches(pid);
        if now == last {
            break;
        }
        assert!(Instant::now() < deadline, "the daemon never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(switches(pid), last, "the daemon woke up after the stop");

    kill(&session, &mut daemon);
}
