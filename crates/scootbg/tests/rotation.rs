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
