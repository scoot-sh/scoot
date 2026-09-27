//! Outputs of one size showing one image share its pixels: one memfd, one
//! `wl_shm_pool`, one mapping in the daemon, and a `wl_buffer` per output
//! over it (`src/share.rs` says why not one buffer on every surface).
//! Checked three ways: the daemon's protocol trace (pools made and the
//! buffers made from each), its memfd mappings in `/proc/PID/maps`, and
//! real pixels by screenshot. On scoot (two 1600×1000 outputs) and on sway
//! (outputs plugged in and out).
#![cfg(target_os = "linux")]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{PATIENCE, Session, wait_exit};
use serde_json::{Value, json};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

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

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Four flat quadrants, red, green / blue, white; `flip` swaps left and
/// right, so a second image is told apart from the first.
fn quadrant(x: u32, y: u32, width: u32, height: u32, flip: bool) -> [u8; 3] {
    match ((x < width / 2) != flip, y < height / 2) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    }
}

fn write_png(path: &Path, width: u32, height: u32, flip: bool) {
    let data: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| quadrant(x, y, width, height, flip))
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
}

#[track_caller]
fn assert_near(got: [u8; 3], want: [u8; 3], what: &str) {
    assert!(
        got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2),
        "{what}: {got:?}, want {want:?}"
    );
}

/// The quadrants (stretched over the whole output), sampled mid-quadrant.
#[track_caller]
fn assert_quadrants(shot: &common::Shot, flip: bool, what: &str) {
    let (w, h) = (shot.width, shot.height);
    for (x, y) in [
        (w / 4, h / 4),
        (3 * w / 4, h / 4),
        (w / 4, 3 * h / 4),
        (3 * w / 4, 3 * h / 4),
    ] {
        assert_near(
            shot.at(x, y),
            quadrant(x, y, w, h, flip),
            &format!("{what} ({x},{y})"),
        );
    }
}

/// The daemon's mappings of its wallpaper memfds, and their sizes in kB:
/// one per set of pixels, however many outputs show them.
fn memfd_maps(pid: u32) -> Vec<u64> {
    std::fs::read_to_string(format!("/proc/{pid}/maps"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("memfd:scootbg-wallpaper"))
        .map(|l| {
            let range = l.split_whitespace().next().unwrap();
            let (from, to) = range.split_once('-').unwrap();
            let from = u64::from_str_radix(from, 16).unwrap();
            let to = u64::from_str_radix(to, 16).unwrap();
            (to - from) / 1024
        })
        .collect()
}

/// A `width`×`height` buffer's mapping in kB: whole 4 KiB pages.
fn buffer_kb(width: u64, height: u64) -> u64 {
    (width * height * 4).div_ceil(4096) * 4
}

/// Waits until the daemon maps `count` wallpaper memfds (a buffer another
/// one replaced goes once the compositor releases it).
#[track_caller]
fn wait_maps(pid: u32, count: usize, what: &str) -> Vec<u64> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let maps = memfd_maps(pid);
        if maps.len() == count {
            return maps;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {} memfd mappings, want {count}: {maps:?}",
            maps.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Open fds that are wallpaper memfds: none, since each is closed as soon
/// as its pool is made.
fn memfd_fds(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|target| target.to_string_lossy().contains("memfd:scootbg-wallpaper"))
        .count()
}

/// From the daemon's `WAYLAND_DEBUG=client` trace: every pool made, in
/// order, with the number of buffers made from it. Object ids are reused
/// once destroyed, so a buffer counts for the latest pool with its pool's
/// id. (The format is `wayland-backend`'s; a change makes this empty, which
/// fails.)
fn pools(session: &Session) -> Vec<usize> {
    let trace = std::fs::read_to_string(session.daemon_log()).unwrap();
    let mut made: Vec<(String, usize)> = Vec::new();
    for line in trace.lines().filter(|l| l.contains("-> ")) {
        if line.contains(".create_pool(") {
            // `-> wl_shm@N.create_pool(wl_shm_pool@M, fd, size)`
            let id = line
                .split("wl_shm_pool@")
                .nth(1)
                .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
                .unwrap_or_else(|| panic!("no pool id in {line:?}"));
            made.push((id.to_owned(), 0));
        } else if line.contains(".create_buffer(") {
            // `-> wl_shm_pool@M.create_buffer(...)`
            let id = line
                .split("-> wl_shm_pool@")
                .nth(1)
                .and_then(|rest| rest.split('.').next())
                .unwrap_or_else(|| panic!("no pool in {line:?}"));
            let pool = made
                .iter_mut()
                .rev()
                .find(|(made, _)| made == id)
                .unwrap_or_else(|| panic!("a buffer from no pool: {line:?}"));
            pool.1 += 1;
        }
    }
    made.into_iter().map(|(_, buffers)| buffers).collect()
}

/// Voluntary plus involuntary context switches.
fn switches(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .filter(|l| l.contains("ctxt_switches"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

/// Settles, then stays asleep for 1.5 s.
fn assert_idle(pid: u32, what: &str) {
    let deadline = Instant::now() + PATIENCE;
    let mut last = switches(pid);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = switches(pid);
        if now == last {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(switches(pid), last, "{what}: the daemon woke up while idle");
}

fn assert_quiet_log(session: &Session) {
    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    for quiet in [
        "panicked",
        "cannot draw",
        "giving up",
        "not configured the wallpaper",
    ] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }
}

fn kill(session: &Session, daemon: &mut std::process::Child) {
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(daemon).success());
}

/// The outputs' names, as `query` lists them.
fn names(outputs: &[Value]) -> Vec<String> {
    outputs
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect()
}

fn scoot_ids(session: &Session) -> Vec<u64> {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect()
}

/// scoot's two 1600×1000 outputs: one image on both is one buffer's worth
/// of memory (6,250 kB), one pool with a buffer per output. A per-output
/// image on one splits them into two; a per-output color on the other
/// leaves only the second image; the first image on both again is shared
/// again. Every step by screenshot, and no memfd stays open.
#[test]
fn outputs_of_one_size_share_one_image_on_scoot() {
    let Some(session) = Session::start("share") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let pid = daemon.id();
    let names = names(&configured(&session, 2));
    let ids = scoot_ids(&session);
    let kb = buffer_kb(1600, 1000);
    let dir = session.runtime_dir();
    let first = dir.join("first.png");
    let second = dir.join("second.png");
    write_png(&first, 400, 200, false);
    write_png(&second, 400, 200, true);
    let shots = |flips: [Option<bool>; 2], what: &str| {
        for (id, flip) in ids.iter().zip(flips) {
            let shot = session.scoot_screenshot(*id);
            match flip {
                Some(flip) => assert_quadrants(&shot, flip, &format!("{what}, output {id}")),
                None => shot.assert_all(common::rgb("#203040"), &format!("{what}, output {id}")),
            }
        }
    };

    ok(&session, &["set", path_str(&first), "--mode", "stretch"]);
    shots([Some(false), Some(false)], "shared");
    assert_eq!(
        wait_maps(pid, 1, "one image on both"),
        [kb],
        "one 1600x1000 buffer's pages"
    );
    assert_eq!(memfd_fds(pid), 0, "the memfd is closed once pooled");
    assert_eq!(
        pools(&session),
        [2],
        "one pool for both outputs, a buffer per output"
    );

    // One output gets its own image: two sets of pixels now.
    ok(
        &session,
        &[
            "set",
            path_str(&second),
            "--mode",
            "stretch",
            "--output",
            "headless-2",
        ],
    );
    shots([Some(false), Some(true)], "split");
    assert_eq!(wait_maps(pid, 2, "split"), [kb, kb]);
    assert_eq!(pools(&session), [2, 1], "the second image's own");

    // The other one a color (a single-pixel buffer, no shm): only the
    // second image's pixels are left.
    ok(&session, &["set", "#203040", "--output", &names[0]]);
    shots([None, Some(true)], "a color beside it");
    wait_maps(pid, 1, "a color beside it");

    // The first image on both again: shared again, and the second image's
    // pixels gone with the last buffer over them.
    ok(&session, &["set", path_str(&first), "--mode", "stretch"]);
    shots([Some(false), Some(false)], "shared again");
    assert_eq!(wait_maps(pid, 1, "shared again"), [kb]);
    assert_eq!(pools(&session), [2, 1, 2]);
    assert_eq!(
        session.query()["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["shows"]["image"].clone())
            .collect::<Vec<_>>(),
        [json!(path_str(&first)), json!(path_str(&first))]
    );
    assert_eq!(memfd_fds(pid), 0);
    assert_idle(pid, "shared");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// On the full-size color path (a compositor without a viewporter, forced
/// in a debug build), a color is a full-size shm buffer too, and one that
/// replaces a shared image may reuse its pixels only once no other output
/// shows them. Every change is checked by screenshot on both outputs:
/// writing into pixels the other output still showed would show there.
#[test]
fn full_size_colors_never_write_into_pixels_another_output_shows() {
    let Some(session) = Session::start("sharefull") else {
        return;
    };
    if !cfg!(debug_assertions) {
        eprintln!("skipped -- the forced path needs a debug build");
        return;
    }
    let mut daemon = session.daemon_logged(&[("SCOOTBG_DEBUG_PATH", "full-shm")]);
    let pid = daemon.id();
    let names = names(&configured(&session, 2));
    let ids = scoot_ids(&session);
    let image = session.runtime_dir().join("q.png");
    write_png(&image, 400, 200, false);
    let both = |what: &str, check: &dyn Fn(&common::Shot, &str)| {
        for id in &ids {
            check(
                &session.scoot_screenshot(*id),
                &format!("{what}, output {id}"),
            );
        }
    };
    for round in 0..3 {
        ok(&session, &["set", path_str(&image), "--mode", "stretch"]);
        both(&format!("round {round}: image"), &|s, w| {
            assert_quadrants(s, false, w)
        });
        // One output's color first: the other keeps the image, untouched.
        ok(&session, &["set", "#c03020", "--output", &names[0]]);
        let one = session.scoot_screenshot(ids[0]);
        one.assert_all(common::rgb("#c03020"), "the colored one");
        assert_quadrants(
            &session.scoot_screenshot(ids[1]),
            false,
            &format!("round {round}: the image beside a color"),
        );
        // Then both.
        ok(&session, &["set", "#2040c0"]);
        both(&format!("round {round}: color"), &|s, w| {
            s.assert_all(common::rgb("#2040c0"), w)
        });
    }
    // Colors are never shared: one full-size buffer each, no spare, and
    // the image's pages gone with its last buffer.
    let kb = buffer_kb(1600, 1000);
    assert_eq!(wait_maps(pid, 2, "a color each"), [kb, kb]);
    assert_eq!(memfd_fds(pid), 0);
    assert_idle(pid, "full-size colors");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// sway's outputs by name, with their current mode.
fn sway_outputs(session: &Session) -> Vec<(String, u64, u64)> {
    let outputs: Value =
        serde_json::from_str(&session.swaymsg(&["-t", "get_outputs", "-r"])).unwrap();
    outputs
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["name"].as_str().unwrap().to_owned(),
                o["current_mode"]["width"].as_u64().unwrap_or(0),
                o["current_mode"]["height"].as_u64().unwrap_or(0),
            )
        })
        .collect()
}

/// Two 1920×1080 outputs on sway share one image. Unplugging the first
/// leaves the second showing it; one plugged in later, of the same size,
/// is served from those pixels (a buffer over the same pool, no new pool,
/// so no decode); a per-output image on it splits them.
#[test]
fn sharing_survives_hotplug_on_sway() {
    let Some(session) = Session::sway("sharesway") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let pid = daemon.id();
    configured(&session, 1);
    session.swaymsg(&["create_output"]);
    configured(&session, 2);
    // The first is 1280×720, one made later 1920×1080: made one size.
    let first_name = sway_outputs(&session)[0].0.clone();
    session.swaymsg(&["--", "output", &first_name, "mode", "--custom", "1920x1080"]);
    session.query_until("one size", |o| {
        o.len() == 2
            && o.iter()
                .all(|o| o["surface"]["size"] == json!({"width": 1920, "height": 1080}))
    });
    let outputs = sway_outputs(&session);
    assert!(
        outputs
            .iter()
            .all(|o| (o.1, o.2) == (outputs[0].1, outputs[0].2)),
        "one size: {outputs:?}"
    );
    let kb = buffer_kb(outputs[0].1, outputs[0].2);
    let dir = session.runtime_dir();
    let first = dir.join("first.png");
    let second = dir.join("second.png");
    write_png(&first, 400, 200, false);
    write_png(&second, 400, 200, true);

    ok(&session, &["set", path_str(&first), "--mode", "stretch"]);
    for (name, ..) in &outputs {
        assert_quadrants(&session.screencopy(name), false, name);
    }
    assert_eq!(wait_maps(pid, 1, "shared"), [kb]);
    assert_eq!(pools(&session), [2]);

    // Unplug the first: the second still shows the shared pixels.
    session.swaymsg(&["output", &outputs[0].0, "unplug"]);
    configured(&session, 1);
    let kept = &outputs[1].0;
    assert_quadrants(
        &session.screencopy(kept),
        false,
        "after unplugging the other",
    );
    assert_eq!(wait_maps(pid, 1, "after unplugging"), [kb]);

    // Plug one in: the same size, served from the pixels already there.
    session.swaymsg(&["create_output"]);
    configured(&session, 2);
    session.query_until("the new output shows the image", |o| {
        o.len() == 2 && o.iter().all(|o| o["shows"]["image"] == path_str(&first))
    });
    let now = sway_outputs(&session);
    for (name, ..) in &now {
        assert_quadrants(&session.screencopy(name), false, name);
    }
    assert_eq!(
        pools(&session),
        [3],
        "no new pool (so no new decode), a third buffer over the one pool"
    );
    assert_eq!(wait_maps(pid, 1, "after plugging one in"), [kb]);

    // A per-output image on the new one splits them.
    let new = now.iter().find(|o| &o.0 != kept).unwrap().0.clone();
    ok(
        &session,
        &[
            "set",
            path_str(&second),
            "--mode",
            "stretch",
            "--output",
            &new,
        ],
    );
    assert_quadrants(&session.screencopy(kept), false, "the kept one");
    assert_quadrants(&session.screencopy(&new), true, "the new one");
    assert_eq!(wait_maps(pid, 2, "split"), [kb, kb]);
    assert_eq!(memfd_fds(pid), 0);
    assert_idle(pid, "after hotplug");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}
