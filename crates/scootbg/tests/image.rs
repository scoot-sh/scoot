//! Images end to end: `set PATH` against a real `scoot --headless` (and a
//! headless sway where scoot cannot do it: outputs of different sizes,
//! plugging one in and out), checked by real pixels. Test images are made
//! here (`png`, `image-webp`'s lossless encoder) or from the tiny JPEG
//! fixtures in `tests/fixtures/`.
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{PATIENCE, Session, rgb, wait_exit};
use serde_json::{Value, json};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

const QUADRANTS_JPEG: &[u8] = include_bytes!("fixtures/quadrants.jpg");

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

/// `scootbg ARGS`, asserting it fails with exit 1; returns its stderr.
fn fails(session: &Session, args: &[&str]) -> String {
    let out = session.run(args);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
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

/// scoot's output ids, in order.
fn scoot_ids(session: &Session) -> Vec<u64> {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect()
}

fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

#[track_caller]
fn assert_near(got: [u8; 3], want: [u8; 3], tolerance: u8, what: &str) {
    assert!(
        close(got, want, tolerance),
        "{what}: {got:?}, want {want:?}"
    );
}

/// Four flat quadrants, red, green / blue, white.
fn quadrant(x: u32, y: u32, width: u32, height: u32) -> [u8; 3] {
    match (x < width / 2, y < height / 2) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    }
}

fn write_png(path: &Path, width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) {
    let data: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| pixel(x, y))
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
}

/// A minimal big-endian EXIF block recording `orientation`.
fn exif(orientation: u16) -> Vec<u8> {
    let mut out = b"MM\0*\0\0\0\x08\0\x01".to_vec();
    out.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1]);
    out.extend_from_slice(&orientation.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    out
}

/// `jpeg` with an APP1 EXIF segment recording `orientation`, as a camera
/// writes it.
fn jpeg_with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let exif = exif(orientation);
    let len = u16::try_from(2 + 6 + exif.len()).unwrap();
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&exif);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Voluntary plus involuntary context switches, and threads.
fn switches(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .filter(|l| l.contains("ctxt_switches"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

fn threads(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap()
}

/// Settles, then stays asleep for 1.5 s, with one thread: the decoding
/// thread is gone once its job is done.
fn assert_idle(pid: u32, what: &str) {
    let deadline = Instant::now() + PATIENCE;
    let mut last = switches(pid);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = switches(pid);
        if now == last && threads(pid) == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(switches(pid), last, "{what}: the daemon woke up while idle");
    assert_eq!(threads(pid), 1, "{what}: a thread stayed");
}

fn assert_quiet_log(session: &Session) {
    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    for quiet in [
        "panicked",
        "cannot draw",
        "giving up",
        "cannot accept",
        "thread",
    ] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }
}

fn kill(session: &Session, daemon: &mut std::process::Child) {
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(daemon).success());
}

/// Every fit mode on a 1600×1000 output, from a 400×200 image of four
/// quadrants, checked by scoot's own screenshot: where each quadrant lands,
/// the fill color around it, and `query`'s `shows`. `center` and `tile`
/// copy pixels unscaled, so those are exact to the pixel.
#[test]
fn every_fit_mode_on_scoot() {
    let Some(session) = Session::start_with("modes", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let image = session.runtime_dir().join("quadrants.png");
    write_png(&image, 400, 200, |x, y| quadrant(x, y, 400, 200));
    let path = path_str(&image);
    let id = scoot_ids(&session)[0];
    let fill = "#102030";
    let shot = |mode: &str| {
        ok(&session, &["set", path, "--mode", mode, "--fill", fill]);
        assert_eq!(
            shows(&session)[0],
            json!({"image": path, "mode": mode, "fill": fill, "filter": "lanczos3"})
        );
        session.scoot_screenshot(id)
    };

    // fill: the middle 320×200 (x 40..360) scaled 5×, so the quadrants
    // meet at the centre (800, 500).
    let s = shot("fill");
    assert_eq!((s.width, s.height), (1600, 1000));
    for (x, y, want) in [
        (400, 250, RED),
        (1200, 250, GREEN),
        (400, 750, BLUE),
        (1200, 750, WHITE),
    ] {
        assert_near(s.at(x, y), want, 2, &format!("fill ({x},{y})"));
    }
    assert_near(s.at(0, 0), RED, 2, "fill covers the corner");
    assert_near(s.at(1599, 999), WHITE, 2, "fill covers the corner");

    // fit: 1600×800, 100-pixel bars of fill above and below.
    let s = shot("fit");
    for x in [0, 800, 1599] {
        for y in [0, 99, 900, 999] {
            assert_eq!(s.at(x, y), rgb(fill), "fit bar ({x},{y})");
        }
    }
    for (x, y, want) in [
        (400, 300, RED),
        (1200, 300, GREEN),
        (400, 700, BLUE),
        (1200, 700, WHITE),
    ] {
        assert_near(s.at(x, y), want, 2, &format!("fit ({x},{y})"));
    }

    // stretch: the whole image, 4× by 5×.
    let s = shot("stretch");
    for (x, y, want) in [
        (400, 250, RED),
        (1200, 250, GREEN),
        (400, 750, BLUE),
        (1200, 750, WHITE),
    ] {
        assert_near(s.at(x, y), want, 2, &format!("stretch ({x},{y})"));
    }

    // center: 400×200 at (600, 400), unscaled: exact.
    let s = shot("center");
    for y in 0..1000 {
        for x in 0..1600 {
            let inside = (600..1000).contains(&x) && (400..600).contains(&y);
            let want = if inside {
                quadrant(x - 600, y - 400, 400, 200)
            } else {
                rgb(fill)
            };
            assert_eq!(s.at(x, y), want, "center ({x},{y})");
        }
    }

    // tile: repeated from the top-left, unscaled: exact.
    let s = shot("tile");
    for y in 0..1000 {
        for x in 0..1600 {
            assert_eq!(
                s.at(x, y),
                quadrant(x % 400, y % 200, 400, 200),
                "tile ({x},{y})"
            );
        }
    }

    // nearest: hard edges, exactly four colors.
    ok(
        &session,
        &["set", path, "--mode", "stretch", "--filter", "nearest"],
    );
    let s = session.scoot_screenshot(id);
    assert_eq!(
        s.colors(),
        [[0, 0, 255], [0, 255, 0], [255, 0, 0], [255, 255, 255]]
    );
    assert_eq!(s.at(799, 499), RED);
    assert_eq!(s.at(800, 500), WHITE);

    assert_idle(daemon.id(), "with an image set");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// A JPEG whose EXIF orientation says "rotate": shown upright. The fixture
/// is stored as red, green / blue, white; orientation 6 (rotate 90°
/// clockwise to display) shows blue, red / white, green, and 8 (counter-
/// clockwise) green, white / red, blue. A WebP's EXIF too.
#[test]
fn an_exif_rotated_jpeg_is_shown_upright() {
    let Some(session) = Session::start_with("exif", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let id = scoot_ids(&session)[0];
    for (orientation, want) in [
        (1, [RED, GREEN, BLUE, WHITE]),
        (3, [WHITE, BLUE, GREEN, RED]),
        (6, [BLUE, RED, WHITE, GREEN]),
        (8, [GREEN, WHITE, RED, BLUE]),
    ] {
        let file = session.runtime_dir().join(format!("o{orientation}.jpg"));
        std::fs::write(&file, jpeg_with_orientation(QUADRANTS_JPEG, orientation)).unwrap();
        ok(&session, &["set", path_str(&file), "--mode", "stretch"]);
        let s = session.scoot_screenshot(id);
        for ((x, y), want) in [(400, 250), (1200, 250), (400, 750), (1200, 750)]
            .into_iter()
            .zip(want)
        {
            assert_near(
                s.at(x, y),
                want,
                12,
                &format!("orientation {orientation} at ({x},{y})"),
            );
        }
    }
    // Lossless WebP with EXIF orientation 6: exact colors.
    let (w, h) = (40, 20);
    let data: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| quadrant(x, y, w, h))
        .collect();
    let mut bytes = Vec::new();
    let mut encoder = image_webp::WebPEncoder::new(&mut bytes);
    encoder.set_exif_metadata(exif(6));
    encoder
        .encode(&data, w, h, image_webp::ColorType::Rgb8)
        .unwrap();
    let file = session.runtime_dir().join("o6.webp");
    std::fs::write(&file, bytes).unwrap();
    ok(
        &session,
        &[
            "set",
            path_str(&file),
            "--mode",
            "stretch",
            "--filter",
            "nearest",
        ],
    );
    let s = session.scoot_screenshot(id);
    for ((x, y), want) in [(400, 250), (1200, 250), (400, 750), (1200, 750)]
        .into_iter()
        .zip([BLUE, RED, WHITE, GREEN])
    {
        assert_eq!(s.at(x, y), want, "webp orientation 6 at ({x},{y})");
    }
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// Every way an image can fail is an error reply (exit 1) saying why, and
/// changes nothing: both outputs keep what they showed, in `query` and on
/// screen.
#[test]
fn a_failed_set_leaves_the_previous_wallpaper() {
    let Some(session) = Session::start_with("fail", 2, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 2);
    let ids = scoot_ids(&session);
    let dir = session.runtime_dir();
    let good = dir.join("good.png");
    write_png(&good, 64, 40, |_, _| [0x30, 0x60, 0x90]);
    ok(&session, &["set", "#c03020"]);
    ok(
        &session,
        &["set", path_str(&good), "--output", "headless-2"],
    );
    let before = shows(&session);
    assert_eq!(before[0], json!({"color": "#c03020"}));
    assert_eq!(before[1]["image"], path_str(&good));

    let truncated = dir.join("truncated.png");
    let whole = std::fs::read(&good).unwrap();
    std::fs::write(&truncated, &whole[..whole.len() / 2]).unwrap();
    let text = dir.join("text.jpg");
    std::fs::write(&text, "not a picture").unwrap();
    // A PNG header claiming 20000×20000 (400 megapixels), and no pixels:
    // refused from the header, before anything is allocated.
    let bomb = dir.join("bomb.png");
    {
        let file = std::fs::File::create(&bomb).unwrap();
        let encoder = png::Encoder::new(file, 20000, 20000);
        drop(encoder.write_header().unwrap());
    }
    let missing = dir.join("missing.png");
    for (target, reason) in [
        (path_str(&missing), "no such file"),
        (path_str(dir), "not a regular file"),
        (path_str(&text), "not an image"),
        (path_str(&truncated), "truncated or corrupt"),
        (path_str(&bomb), "image too large"),
    ] {
        for output in [None, Some("headless-2")] {
            let mut args = vec!["set", target];
            args.extend(output.iter().flat_map(|o| ["--output", *o]));
            let stderr = fails(&session, &args);
            assert!(stderr.contains(reason), "{target}: {stderr}");
            assert!(stderr.contains("nothing was changed"), "{target}: {stderr}");
            assert_eq!(shows(&session), before, "{target} changed what shows");
        }
    }
    // A relative path sent straight to the socket (the CLI never does).
    let stream = UnixStream::connect(session.socket()).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    (&stream)
        .write_all(b"{\"protocol\":1,\"type\":\"set\",\"image\":\"good.png\"}\n")
        .unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["type"], "error");
    assert!(
        reply["message"].as_str().unwrap().contains("not absolute"),
        "{reply}"
    );

    session
        .scoot_screenshot(ids[0])
        .assert_all(rgb("#c03020"), "the color stays");
    session
        .scoot_screenshot(ids[1])
        .assert_all([0x30, 0x60, 0x90], "the image stays");
    assert_eq!(shows(&session), before);
    // And the daemon still works.
    ok(&session, &["set", path_str(&good)]);
    session
        .scoot_screenshot(ids[0])
        .assert_all([0x30, 0x60, 0x90], "set after failures");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// Requests in flight together: the newest wins whatever order they
/// finish in, and every one is answered. A slow image followed at once by
/// a color ends on the color; a color followed by an image ends on the
/// image; a burst of images ends on the last, and all say `ok`.
#[test]
fn the_newest_set_wins() {
    let Some(session) = Session::start_with("newest", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let id = scoot_ids(&session)[0];
    let dir = session.runtime_dir();
    let colors: Vec<[u8; 3]> = (0..8).map(|i| [i * 30, 255 - i * 30, 100]).collect();
    let files: Vec<PathBuf> = colors
        .iter()
        .enumerate()
        .map(|(i, &color)| {
            let file = dir.join(format!("c{i}.png"));
            write_png(&file, 1200, 800, move |_, _| color);
            file
        })
        .collect();
    // Each request on its own connection, all written before any reply is
    // read: the daemon takes them in this order.
    let send = |line: String| {
        let stream = UnixStream::connect(session.socket()).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        (&stream).write_all(line.as_bytes()).unwrap();
        // Let the daemon take it before the next is sent.
        std::thread::sleep(Duration::from_millis(20));
        stream
    };
    let reply = |stream: UnixStream| -> Value {
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let image = |i: usize| {
        format!(
            "{{\"protocol\":1,\"type\":\"set\",\"image\":{:?}}}\n",
            path_str(&files[i])
        )
    };
    let color = |hex: &str| format!("{{\"protocol\":1,\"type\":\"set\",\"color\":\"{hex}\"}}\n");

    // The older request is superseded: its `ok` comes once the newer
    // choice is on screen, as a superseded color's does.
    let a = send(image(0));
    let b = send(color("#c03020"));
    assert_eq!(reply(a), json!({"type": "ok"}));
    session.scoot_screenshot(id).assert_all(
        rgb("#c03020"),
        "on screen when the superseded image answers",
    );
    assert_eq!(reply(b), json!({"type": "ok"}));
    session
        .scoot_screenshot(id)
        .assert_all(rgb("#c03020"), "image then color");
    assert_eq!(shows(&session)[0], json!({"color": "#c03020"}));

    let a = send(color("#101014"));
    let b = send(image(1));
    assert_eq!(reply(a), json!({"type": "ok"}));
    assert_eq!(reply(b), json!({"type": "ok"}));
    session
        .scoot_screenshot(id)
        .assert_all(colors[1], "color then image");

    // The first finds the worker idle and runs (and is shown); while it
    // decodes the rest queue, the newest runs next, and the four it
    // supersedes are never decoded and answer only once it is on screen.
    let burst: Vec<UnixStream> = (2..8).map(|i| send(image(i))).collect();
    for (i, stream) in burst.into_iter().enumerate() {
        assert_eq!(reply(stream), json!({"type": "ok"}));
        if i == 1 {
            session
                .scoot_screenshot(id)
                .assert_all(colors[7], "on screen when the oldest of the burst answers");
        }
    }
    session
        .scoot_screenshot(id)
        .assert_all(colors[7], "the last of a burst");
    assert_eq!(shows(&session)[0]["image"], path_str(&files[7]));
    // A failing newest request does not stop an older one from showing.
    let a = send(image(2));
    let b = send(format!(
        "{{\"protocol\":1,\"type\":\"set\",\"image\":{:?}}}\n",
        path_str(&dir.join("missing.png"))
    ));
    assert_eq!(reply(a), json!({"type": "ok"}));
    assert_eq!(reply(b)["type"], "error");
    session
        .scoot_screenshot(id)
        .assert_all(colors[2], "the older one, as the newer failed");
    assert_idle(daemon.id(), "after the burst");
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

/// Two outputs of different sizes (sway: scoot's headless outputs are all
/// one size) each get the image fitted to their own size, and an output
/// plugged in later, or while the image decodes, gets it too (decoded
/// again). An output unplugged while its image decodes is dropped from the
/// job, and the reply still comes.
#[test]
fn outputs_of_different_sizes_and_hotplug_on_sway() {
    let Some(session) = Session::sway("imgsway") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    session.swaymsg(&["create_output"]);
    configured(&session, 2);
    let names: Vec<String> = sway_outputs(&session).into_iter().map(|o| o.0).collect();
    session.swaymsg(&["--", "output", &names[1], "mode", "--custom", "1024x768"]);
    session.query_until("resized", |o| {
        o.iter()
            .any(|o| o["surface"]["size"] == json!({"width": 1024, "height": 768}))
    });
    let dir = session.runtime_dir();
    let image = dir.join("quadrants.png");
    write_png(&image, 400, 200, |x, y| quadrant(x, y, 400, 200));
    ok(&session, &["set", path_str(&image), "--mode", "stretch"]);
    for (name, width, height) in sway_outputs(&session) {
        let s = session.screencopy(&name);
        assert_eq!((u64::from(s.width), u64::from(s.height)), (width, height));
        let (w, h) = (s.width, s.height);
        for (x, y) in [
            (w / 4, h / 4),
            (3 * w / 4, h / 4),
            (w / 4, 3 * h / 4),
            (3 * w / 4, 3 * h / 4),
        ] {
            assert_near(
                s.at(x, y),
                quadrant(x, y, w, h),
                2,
                &format!("{name} ({x},{y})"),
            );
        }
    }

    // Plugged in later: drawn for it (a new decode).
    session.swaymsg(&["create_output"]);
    configured(&session, 3);
    let third = sway_outputs(&session)[2].0.clone();
    ok(&session, &["set", "#000000", "--output", &third]);
    ok(&session, &["set", path_str(&image), "--mode", "stretch"]);
    let s = session.screencopy(&third);
    assert_near(s.at(s.width / 4, s.height / 4), RED, 2, "the new output");

    // A slow image, with an output unplugged and another plugged in while
    // it decodes: whichever way the race goes, the reply is `ok`, and every
    // output there afterwards shows it.
    let big = dir.join("big.png");
    write_png(&big, 1200, 800, |x, y| quadrant(x, y, 1200, 800));
    let stream = UnixStream::connect(session.socket()).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    let line = format!(
        "{{\"protocol\":1,\"type\":\"set\",\"image\":{:?},\"mode\":\"fill\"}}\n",
        path_str(&big)
    );
    let asked = Instant::now();
    (&stream).write_all(line.as_bytes()).unwrap();
    session.swaymsg(&["output", &names[1], "unplug"]);
    session.swaymsg(&["create_output"]);
    let hotplugged = asked.elapsed();
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).unwrap();
    // Whether the race went the interesting way (the outputs changed while
    // the image decoded) depends on the machine; both ways must be right.
    println!(
        "hotplug done {hotplugged:?} after the request, reply after {:?}",
        asked.elapsed()
    );
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap(),
        json!({"type": "ok"})
    );
    let outputs = configured(&session, 3);
    // The one plugged in during the decode may configure after the reply:
    // `set` waited only for the outputs it targeted. Wait for it to show.
    session.query_until("every output shows the big image", |o| {
        o.len() == 3 && o.iter().all(|o| o["shows"]["image"] == path_str(&big))
    });
    for output in outputs {
        let name = output["name"].as_str().unwrap();
        let s = session.screencopy(name);
        assert_near(s.at(s.width / 4, s.height / 4), RED, 2, name);
        assert_near(s.at(3 * s.width / 4, 3 * s.height / 4), WHITE, 2, name);
    }
    assert_idle(daemon.id(), "after hotplug");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// Requests sent in the daemon's `WAYLAND_DEBUG=client` trace containing
/// `needle`.
fn sent(session: &Session, needle: &str) -> Vec<String> {
    std::fs::read_to_string(session.daemon_log())
        .unwrap()
        .lines()
        .filter(|l| l.contains("-> ") && l.contains(needle))
        .map(str::to_owned)
        .collect()
}

fn reload(session: &Session, config: &str) {
    std::fs::write(session.runtime_dir().join("config.toml"), config).unwrap();
    let reloaded = session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reloaded["type"], "reloaded", "{reloaded}");
}

/// An image is drawn at the output's real pixels, the surface size times
/// its integer scale, as one full-size buffer with its opaque region set.
/// A new scale that keeps that buffer size (scale 2 on a 1600×1000 mode:
/// 800×500 at 2) is redrawn from the buffer already there, with no new
/// decode; one that changes it (1.5, which `wl_output` rounds to 2 over a
/// 1067×667 surface) decodes and draws again at 2134×1334.
#[test]
fn a_new_scale_redraws_at_the_real_pixel_size() {
    let Some(session) = Session::start_with("iscale", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    configured(&session, 1);
    let id = scoot_ids(&session)[0];
    let image = session.runtime_dir().join("q.png");
    write_png(&image, 400, 200, |x, y| quadrant(x, y, 400, 200));
    ok(&session, &["set", path_str(&image), "--mode", "stretch"]);
    assert_eq!(sent(&session, "create_pool(").len(), 1);
    assert_eq!(sent(&session, ", 0, 1600, 1000, 6400, 1)").len(), 1);
    assert_eq!(
        sent(&session, ".add(0, 0, 1600, 1000)").len(),
        1,
        "opaque region"
    );
    assert!(sent(&session, "set_buffer_scale").is_empty(), "scale 1");
    assert!(sent(&session, "create_u32_rgba_buffer").is_empty());
    // No `set` waits on these redraws, and the screenshot is another
    // client's request, so the compositor may take it before the commit:
    // polled until right, within the usual patience.
    let check = |what: &str| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let s = session.scoot_screenshot(id);
            assert_eq!((s.width, s.height), (1600, 1000));
            let wrong: Vec<_> = [(400, 250), (1200, 250), (400, 750), (1200, 750)]
                .into_iter()
                .filter(|&(x, y)| !close(s.at(x, y), quadrant(x, y, 1600, 1000), 2))
                .map(|(x, y)| ((x, y), s.at(x, y)))
                .collect();
            if wrong.is_empty() {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: {wrong:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    check("scale 1");

    reload(&session, "[output]\nscale = 2.0\n");
    session.query_until("800x500", |o| {
        o[0]["surface"]["size"] == json!({"width": 800, "height": 500})
    });
    let deadline = Instant::now() + PATIENCE;
    while sent(&session, "set_buffer_scale(2)").is_empty() {
        assert!(Instant::now() < deadline, "never redrawn at scale 2");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        sent(&session, "create_pool(").len(),
        1,
        "the same buffer, no new decode"
    );
    check("scale 2");

    reload(&session, "[output]\nscale = 1.5\n");
    let deadline = Instant::now() + PATIENCE;
    while sent(&session, ", 0, 2134, 1334, 8536, 1)").is_empty() {
        assert!(Instant::now() < deadline, "never drawn at 2134x1334");
        std::thread::sleep(Duration::from_millis(20));
    }
    session.query_until("1067x667", |o| {
        o[0]["surface"]["size"] == json!({"width": 1067, "height": 667})
    });
    assert_eq!(sent(&session, "create_pool(").len(), 2);
    check("scale 1.5");
    assert_idle(daemon.id(), "after the scale changes");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

fn status_kb(pid: u32, key: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG claiming 16384×16384 with `color` type whose data is one stored
/// deflate block of 4 KiB of zeros, never finished: a few KB of file.
fn header_only_png(color: u8) -> Vec<u8> {
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    };
    let mut ihdr = 16384_u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&16384_u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, color, 0, 0, 0]);
    let mut idat = vec![0x78, 0x01, 0x00, 0x00, 0x10, 0xff, 0xef];
    idat.extend_from_slice(&[0; 4096]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &idat));
    png.extend(chunk(b"IEND", &[]));
    png
}

/// A lossless WebP of 16×16 with its header rewritten to 16383×16383 (the
/// largest `image-webp` reads), the rest of the bitstream as it was.
fn header_only_webp(alpha: bool) -> Vec<u8> {
    let data: Vec<u8> = (0..16 * 16)
        .flat_map(|i| [i as u8, 7, 200, if alpha { 128 } else { 255 }])
        .collect();
    let mut bytes = Vec::new();
    let color = if alpha {
        image_webp::ColorType::Rgba8
    } else {
        image_webp::ColorType::Rgb8
    };
    let pixels: Vec<u8> = if alpha {
        data
    } else {
        data.chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect()
    };
    image_webp::WebPEncoder::new(&mut bytes)
        .encode(&pixels, 16, 16, color)
        .unwrap();
    let at = bytes.windows(4).position(|w| w == b"VP8L").unwrap() + 8;
    assert_eq!(bytes[at], 0x2f);
    let header = u32::from_le_bytes(bytes[at + 1..at + 5].try_into().unwrap());
    let header = (header & !((1 << 28) - 1)) | 16382 | (16382 << 14);
    bytes[at + 1..at + 5].copy_from_slice(&header.to_le_bytes());
    bytes
}

/// A file that claims a large size in budget but holds a few KB is
/// refused without committing the size it claims: each raises the
/// daemon's peak RSS by less than 16 MB (they used to raise it by 0.8 to
/// 1.05 GB, the decoded buffer written in full before any data was read,
/// and the JPEG was even shown, as 805 MB of grey).
#[test]
fn a_file_that_claims_a_large_size_costs_what_it_holds() {
    let Some(session) = Session::start_with("liars", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let pid = daemon.id();
    let dir = session.runtime_dir();
    let mut jpeg = QUADRANTS_JPEG.to_vec();
    let sof = jpeg
        .windows(2)
        .position(|w| w[0] == 0xff && (w[1] == 0xc0 || w[1] == 0xc2))
        .unwrap();
    jpeg[sof + 5..sof + 9].copy_from_slice(&[0x40, 0x00, 0x40, 0x00]);
    for (name, bytes) in [
        ("rgb.png", header_only_png(2)),
        ("rgba.png", header_only_png(6)),
        ("grey.png", header_only_png(0)),
        ("baseline.jpg", jpeg),
        ("opaque.webp", header_only_webp(false)),
        ("alpha.webp", header_only_webp(true)),
    ] {
        let file = dir.join(name);
        std::fs::write(&file, &bytes).unwrap();
        let before = status_kb(pid, "VmRSS:");
        std::fs::write(format!("/proc/{pid}/clear_refs"), "5").unwrap();
        let stderr = fails(&session, &["set", path_str(&file)]);
        assert!(stderr.contains("truncated or corrupt"), "{name}: {stderr}");
        let peak = status_kb(pid, "VmHWM:");
        assert!(
            peak < before + 16 * 1024,
            "{name} ({} bytes): peak {peak} kB from {before} kB",
            bytes.len()
        );
    }
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}
