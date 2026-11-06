//! The pictures scootbg keeps for a locker, end to end: against a real
//! `scoot --headless` (and a headless sway for hot-unplug), what
//! `$XDG_RUNTIME_DIR/scootbg-DISPLAY.lockscreen/` holds after `set`,
//! `clear`, workspace switches, restarts and unplugs. The pixels of the
//! picture itself are `lockblur`'s unit tests; here it is which output
//! gets which picture, and when.
//!
//! Colors for most of it (a color's picture is a flat image of that color,
//! so a picture's color says which wallpaper it is of), one image for the
//! blur's direction.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{PATIENCE, Session, wait_exit};
use serde_json::Value;

const RED: &str = "#c03020";
const BLUE: &str = "#2030c0";
const GREEN: &str = "#20a040";

fn ok(session: &Session, args: &[&str]) {
    let out = session.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn configured(session: &Session, count: usize) -> Vec<Value> {
    session.query_until("all configured", |o| {
        o.len() == count
            && o.iter()
                .all(|o| o["surface"]["state"] == "configured" && o["surface"]["size"].is_object())
    })
}

fn names(outputs: &[Value]) -> Vec<String> {
    let mut names: Vec<String> = outputs
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

fn dir(session: &Session) -> PathBuf {
    session
        .runtime_dir()
        .join(format!("scootbg-{}.lockscreen", session.wayland_display))
}

/// The index: `(output name, picture path)`, in file order.
fn index(session: &Session) -> Vec<(String, PathBuf)> {
    std::fs::read_to_string(dir(session).join("index"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let (name, file) = line.split_once('\t').expect("a name, a tab, a file");
            (name.to_owned(), PathBuf::from(file))
        })
        .collect()
}

/// `name`'s picture path, if it has a line.
fn picture(session: &Session, name: &str) -> Option<PathBuf> {
    index(session)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, file)| file)
}

/// Decodes a picture: `(width, height, rgb)`.
fn decode(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

fn rgb(hex: &str) -> [u8; 3] {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

/// Whether `name`'s picture is solid `hex` now.
fn is_solid(session: &Session, name: &str, hex: &str) -> bool {
    let Some(path) = picture(session, name) else {
        return false;
    };
    let Ok(file) = std::fs::File::open(&path) else {
        return false;
    };
    let mut reader = match png::Decoder::new(std::io::BufReader::new(file)).read_info() {
        Ok(reader) => reader,
        Err(_) => return false,
    };
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let Ok(info) = reader.next_frame(&mut buffer) else {
        return false;
    };
    buffer[..info.buffer_size()]
        .as_chunks::<3>()
        .0
        .iter()
        .all(|p| *p == rgb(hex))
}

/// Waits until `name`'s picture is solid `hex`.
fn wait_solid(session: &Session, name: &str, hex: &str) {
    wait(session, &format!("{name} to be solid {hex}"), || {
        is_solid(session, name, hex)
    });
}

fn wait(session: &Session, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "never {what}; index: {:?}, directory: {:?}",
            index(session),
            std::fs::read_dir(dir(session)).map(|d| d
                .filter_map(Result::ok)
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn mtime(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

/// A `set` on one output changes only that output's picture; the others
/// are not even rewritten. `clear` takes the picture away (the locker's
/// own background shows); a `kill` removes the directory.
#[test]
fn a_set_on_one_output_changes_only_that_outputs_picture() {
    let Some(session) = Session::start_with("lockscreen", 2, "") else {
        return;
    };
    let mut daemon = session.daemon();
    let all = names(&configured(&session, 2));
    let (a, b) = (&all[0], &all[1]);

    // The directory is private, and empty of pictures before any wallpaper.
    assert!(index(&session).is_empty());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(dir(&session))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    ok(&session, &["set", RED]);
    wait_solid(&session, a, RED);
    wait_solid(&session, b, RED);
    assert_ne!(picture(&session, a), picture(&session, b), "a file each");
    let b_file = picture(&session, b).unwrap();
    let b_written = mtime(&b_file);

    ok(&session, &["set", BLUE, "--output", a]);
    wait_solid(&session, a, BLUE);
    assert!(is_solid(&session, b, RED), "the other output is untouched");
    assert_eq!(mtime(&b_file), b_written, "and was not even rewritten");

    // The picture has the output's aspect at a small size.
    let size = &session.query()["outputs"][0]["surface"]["size"];
    let (w, h, _) = decode(&picture(&session, a).unwrap());
    assert!(w <= 320 && h <= 320);
    let (sw, sh) = (
        size["width"].as_f64().unwrap(),
        size["height"].as_f64().unwrap(),
    );
    assert!(
        (f64::from(w) / f64::from(h) - sw / sh).abs() < 0.02,
        "{w}x{h} against {sw}x{sh}"
    );

    // Clearing one output takes its picture away and leaves the other.
    ok(&session, &["clear", "--output", a]);
    wait(&session, "the cleared output to lose its line", || {
        picture(&session, a).is_none()
    });
    assert!(
        !dir(&session)
            .join("index")
            .to_string_lossy()
            .contains(a.as_str())
    );
    assert!(is_solid(&session, b, RED));

    ok(&session, &["clear"]);
    wait(&session, "an empty index", || index(&session).is_empty());
    let files: Vec<_> = std::fs::read_dir(dir(&session))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    assert_eq!(files, ["index"], "no picture left over");

    ok(&session, &["set", GREEN]);
    wait_solid(&session, a, GREEN);
    wait_solid(&session, b, GREEN);

    // A clean exit removes the directory: those pictures are of a
    // wallpaper that is going away.
    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
    assert!(!dir(&session).exists());
}

/// An image's picture is its blur: the same left-to-right layout, soft.
#[test]
fn an_image_is_blurred_into_its_picture() {
    let Some(session) = Session::start_with("lockscreen-image", 1, "") else {
        return;
    };
    let mut daemon = session.daemon();
    let name = names(&configured(&session, 1)).remove(0);

    // Left half red, right half blue, 128 x 80.
    let file = session.runtime_dir().join("halves.png");
    let mut pixels = Vec::new();
    for _ in 0..80 {
        for x in 0..128 {
            pixels.extend_from_slice(&if x < 64 { [255, 0, 0] } else { [0, 0, 255] });
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 128, 80);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }
    std::fs::write(&file, out).unwrap();

    ok(
        &session,
        &["set", file.to_str().unwrap(), "--mode", "stretch"],
    );
    wait(&session, "the image's picture", || {
        picture(&session, &name).is_some()
    });
    let (w, h, rgb) = decode(&picture(&session, &name).unwrap());
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 3) as usize;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };
    let row = h / 2;
    let left = at(2, row);
    let right = at(w - 3, row);
    assert!(left[0] > 200 && left[2] < 60, "left is red: {left:?}");
    assert!(right[2] > 200 && right[0] < 60, "right is blue: {right:?}");
    let seam = at(w / 2, row);
    assert!(
        seam[0] > 60 && seam[0] < 200 && seam[2] > 60 && seam[2] < 200,
        "the seam is soft: {seam:?}"
    );

    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}

/// A start wipes what a dead daemon left, and the pictures come back from
/// the restored wallpaper after a restart.
#[test]
fn a_restart_wipes_stale_pictures_and_rebuilds_the_current_ones() {
    let Some(session) = Session::start_with("lockscreen-restart", 1, "") else {
        return;
    };
    // What a daemon killed by a signal leaves behind.
    let stale = dir(&session);
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("output-77.png"), b"stale").unwrap();
    std::fs::write(stale.join("index"), "GONE-1\t/nowhere/output-77.png\n").unwrap();

    let mut daemon = session.daemon();
    let name = names(&configured(&session, 1)).remove(0);
    assert!(index(&session).is_empty(), "the stale line is gone");
    assert!(!stale.join("output-77.png").exists());

    ok(&session, &["set", BLUE]);
    wait_solid(&session, &name, BLUE);
    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
    assert!(!dir(&session).exists());

    let mut daemon = session.daemon();
    configured(&session, 1);
    wait_solid(&session, &name, BLUE);

    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}

/// An unplugged output loses its picture and its line; a replugged one
/// gets its picture. Needs sway (the only compositor here that unplugs).
#[test]
fn an_unplugged_output_drops_its_picture() {
    let Some(session) = Session::sway("lockscreen-unplug") else {
        return;
    };
    let mut daemon = session.daemon();
    let first = names(&configured(&session, 1)).remove(0);
    session.swaymsg(&["create_output"]);
    let two = names(&configured(&session, 2));
    let second = two.iter().find(|n| **n != first).unwrap().clone();

    ok(&session, &["set", RED]);
    wait_solid(&session, &first, RED);
    wait_solid(&session, &second, RED);
    let gone_file = picture(&session, &first).unwrap();

    session.swaymsg(&["output", &first, "unplug"]);
    wait(&session, "the unplugged output's line to go", || {
        picture(&session, &first).is_none()
    });
    assert!(!gone_file.exists(), "and its file");
    assert!(
        is_solid(&session, &second, RED),
        "the other keeps its picture"
    );

    // A new output is given the current wallpaper and a picture.
    session.swaymsg(&["create_output"]);
    wait(&session, "a picture for the new output", || {
        index(&session).len() == 2
    });
    for (name, _) in index(&session) {
        wait_solid(&session, &name, RED);
    }

    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}

/// The picture follows the active workspace's wallpaper: a switch to a
/// mapped workspace re-points it, and back.
#[test]
fn a_workspace_switch_repoints_the_picture() {
    use std::process::{Command, Stdio};
    let Some(session) = Session::start_with("lockscreen-ws", 1, "") else {
        return;
    };
    if Command::new("foot")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skipped -- no foot to open a window with");
        return;
    }
    // A window makes a second (empty) workspace to switch to.
    let mut window = Command::new("foot")
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .env("XDG_RUNTIME_DIR", session.runtime_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("foot starts");
    let mut daemon = session.daemon();
    let name = names(&configured(&session, 1)).remove(0);

    ok(&session, &["set", RED]);
    ok(&session, &["set", BLUE, "--workspace", "2"]);
    wait_solid(&session, &name, RED);

    let switch = |index: &str, workspace: &str| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            session.scoot_msg(&["action", "focus-workspace-index", index]);
            if session.query()["outputs"][0]["workspace"] == workspace {
                return;
            }
            assert!(Instant::now() < deadline, "never on workspace {workspace}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    switch("1", "2");
    wait_solid(&session, &name, BLUE);
    switch("0", "1");
    wait_solid(&session, &name, RED);

    let _ = window.kill();
    let _ = window.wait();
    session.run(&["kill"]);
    assert!(wait_exit(&mut daemon).success());
}
