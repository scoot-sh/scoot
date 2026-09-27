//! Drawing at an output's real pixels, fractional scales included, checked
//! by real pixels: a one-pixel checkerboard the size of the output, shown
//! unscaled (`--mode center`), comes back from the screenshot exact only if
//! the buffer lands one pixel to one device pixel. Any resampling by the
//! compositor (a buffer larger than the output scaled down, or one a pixel
//! off stretched) turns the checker grey.
#![cfg(target_os = "linux")]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::Shot;
use common::{PATIENCE, Session, wait_exit};
use serde_json::{Value, json};

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// The checkerboard's pixel at (`x`, `y`).
fn checker(x: u32, y: u32) -> [u8; 3] {
    if (x + y).is_multiple_of(2) {
        WHITE
    } else {
        BLACK
    }
}

fn write_checker(path: &Path, width: u32, height: u32) {
    let data: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| checker(x, y))
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
}

/// A pixel's position and color.
type Pixel = ((u32, u32), [u8; 3]);

/// The pixels in the `width` × `height` top-left of `shot` that are not
/// the checkerboard's: how many, and the first few.
fn off_checker(shot: &Shot, width: u32, height: u32) -> (usize, Vec<Pixel>) {
    let mut count = 0;
    let mut first = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let got = shot.at(x, y);
            if got != checker(x, y) {
                count += 1;
                if first.len() < 8 {
                    first.push(((x, y), got));
                }
            }
        }
    }
    (count, first)
}

fn configured(session: &Session) -> Value {
    session
        .query_until("configured", |o| {
            o.len() == 1 && o[0]["surface"]["state"] == "configured"
        })
        .remove(0)
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

fn scoot_id(session: &Session) -> u64 {
    session.scoot_ipc(r#"{"type":"outputs"}"#)["outputs"][0]["id"]
        .as_u64()
        .unwrap()
}

/// Polls scoot's screenshot until the checker is exact over `width` ×
/// `height`: nothing waits on a redraw after a reload, and the screenshot
/// is another client's request, so it may come before the commit.
fn assert_exact_on_scoot(session: &Session, id: u64, what: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.scoot_screenshot(id);
        assert_eq!((shot.width, shot.height), (1600, 1000), "{what}");
        let (count, first) = off_checker(&shot, 1600, 1000);
        if count == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {count} pixels off the checker, first {first:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn kill(session: &Session, daemon: &mut std::process::Child) {
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(daemon).success());
}

/// scoot at 1.5 on a 1600×1000 mode: `wl_output` says 2, the surface is
/// 1067×667 and its `preferred_scale` 180/120, so the buffer is 1601×1001
/// (1067 × 1.5 = 1600.5, rounded as the protocol says) at buffer scale 1
/// under a 1067×667 viewport. Smithay draws the surface 1601×1001 device
/// pixels and clips the last column and row, so every pixel on screen is
/// the image's own: the checker comes back exact.
#[test]
fn a_fractional_scale_is_drawn_at_device_pixels_on_scoot() {
    let Some(session) = Session::start_with("frac", 1, "[output]\nscale = 1.5\n") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let output = configured(&session);
    assert_eq!(output["scale"], 2, "wl_output rounds up");
    assert_eq!(
        output["surface"],
        json!({
            "state": "configured",
            "size": {"width": 1067, "height": 667},
            "scale": 1.5,
            "pixels": {"width": 1601, "height": 1001},
        })
    );
    assert_eq!(output["logical"], json!({"width": 1067, "height": 667}));
    let image = session.runtime_dir().join("checker.png");
    write_checker(&image, 1600, 1000);
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "center"],
    );
    assert_eq!(sent(&session, "get_fractional_scale(").len(), 1);
    assert_eq!(
        sent(&session, ".create_buffer(").len(),
        1,
        "{:?}",
        sent(&session, ".create_buffer(")
    );
    assert_eq!(sent(&session, ", 0, 1601, 1001, 6404, 1)").len(), 1);
    assert_eq!(sent(&session, "set_destination(1067, 667)").len(), 1);
    assert!(sent(&session, "set_buffer_scale").is_empty());
    assert_exact_on_scoot(&session, scoot_id(&session), "scale 1.5");
    kill(&session, &mut daemon);
}

/// Whether the `width` × `height` top-left of `shot` is a one-pixel
/// checkerboard in either phase: see [`sharp_checker_in`].
fn sharp_checker(shot: &Shot, width: u32, height: u32) -> Result<(), String> {
    sharp_checker_in(shot, (0, 0), (width, height))
}

/// Whether the `size` rectangle of `shot` at `at` is a one-pixel
/// checkerboard in either phase: black and white only, every pixel unlike
/// its right and lower neighbours. For an image cropped or placed by an
/// offset this test does not work out, the phase is free; any resampling
/// still fails.
fn sharp_checker_in(shot: &Shot, at: (u32, u32), size: (u32, u32)) -> Result<(), String> {
    let (right, bottom) = (at.0 + size.0, at.1 + size.1);
    for y in at.1..bottom {
        for x in at.0..right {
            let here = shot.at(x, y);
            if here != BLACK && here != WHITE {
                return Err(format!("({x},{y}) is {here:?}"));
            }
            if x + 1 < right && shot.at(x + 1, y) == here {
                return Err(format!("({x},{y}) and its right neighbour are {here:?}"));
            }
            if y + 1 < bottom && shot.at(x, y + 1) == here {
                return Err(format!("({x},{y}) and the pixel below are {here:?}"));
            }
        }
    }
    Ok(())
}

/// Settles, then stays asleep for 1.5 s, with one thread.
fn assert_idle(pid: u32, what: &str) {
    let read = |key: &str| -> u64 {
        std::fs::read_to_string(format!("/proc/{pid}/status"))
            .unwrap()
            .lines()
            .filter(|l| l.starts_with(key) || l.contains(key))
            .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum()
    };
    let deadline = Instant::now() + PATIENCE;
    let mut last = read("ctxt_switches");
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = read("ctxt_switches");
        if now == last && read("Threads:") == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: never settled");
        last = now;
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(read("ctxt_switches"), last, "{what}: woke up while idle");
}

fn assert_quiet_log(session: &Session) {
    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    for quiet in ["panicked", "cannot draw", "giving up", "thread", "error"] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }
}

/// 1 → 1.25 → 2 → 1 on a 1600×1000 mode: the surface is 1600×1000, then
/// 1280×800, 800×500 and 1600×1000 again, and every one of those at its
/// scale is the same 1600×1000 buffer: decoded once, never again, each
/// step only a new viewport destination over it (which Smithay applies
/// without a new buffer). Then 1.5, which needs 1601×1001: decoded again,
/// from the file. Exact to the pixel at every step; no `set_buffer_scale`
/// ever (the fraction is always known on scoot).
#[test]
fn a_round_of_scales_stays_exact_on_scoot() {
    let Some(session) = Session::start_with("round", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    configured(&session);
    let id = scoot_id(&session);
    let image = session.runtime_dir().join("checker.png");
    write_checker(&image, 1600, 1000);
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "center"],
    );
    assert_exact_on_scoot(&session, id, "scale 1");
    for (scale, logical) in [
        ("1.25", (1280, 800)),
        ("2.0", (800, 500)),
        ("1.0", (1600, 1000)),
    ] {
        reload(&session, &format!("[output]\nscale = {scale}\n"));
        let destination = format!("set_destination({}, {})", logical.0, logical.1);
        let deadline = Instant::now() + PATIENCE;
        // At scale 1 the buffer is the surface's size and needs no
        // viewport, so each destination here is sent once.
        while sent(&session, &destination).is_empty() {
            assert!(Instant::now() < deadline, "{scale}: never redrawn");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_exact_on_scoot(&session, id, scale);
        assert_eq!(sent(&session, ".create_buffer(").len(), 1, "{scale}");
    }
    reload(&session, "[output]\nscale = 1.5\n");
    let deadline = Instant::now() + PATIENCE;
    while sent(&session, ", 0, 1601, 1001, 6404, 1)").is_empty() {
        assert!(Instant::now() < deadline, "never drawn at 1601x1001");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_exact_on_scoot(&session, id, "1.5");
    assert_eq!(sent(&session, ".create_buffer(").len(), 2);
    assert!(sent(&session, "set_buffer_scale").is_empty());
    assert!(sent(&session, ".frame(").is_empty());
    assert_idle(daemon.id(), "after the round");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// The scale changes while the image decodes (a large file, a debug
/// build): the render for the old size is dropped when it lands, the image
/// is rendered again from the file at the new size, the reply is `ok` once
/// that is on screen, and it is sharp. Only buffers of the new size are
/// ever given to the compositor.
#[test]
fn a_scale_change_while_an_image_decodes_draws_at_the_new_scale() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let Some(session) = Session::start_with("middec", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    configured(&session);
    let id = scoot_id(&session);
    let image = session.runtime_dir().join("big.png");
    write_checker(&image, 4000, 2500);
    let stream = UnixStream::connect(session.socket()).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    let line = format!(
        "{{\"protocol\":1,\"type\":\"set\",\"image\":{:?},\"mode\":\"center\"}}\n",
        image.to_str().unwrap()
    );
    let asked = Instant::now();
    (&stream).write_all(line.as_bytes()).unwrap();
    reload(&session, "[output]\nscale = 1.5\n");
    let rescaled = asked.elapsed();
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).unwrap();
    println!(
        "rescaled {rescaled:?} after the request, reply after {:?}",
        asked.elapsed()
    );
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap(),
        json!({"type": "ok"})
    );
    let buffers = sent(&session, ".create_buffer(");
    assert!(
        buffers
            .iter()
            .all(|b| b.ends_with(", 0, 1601, 1001, 6404, 1)")),
        "{buffers:#?}"
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.scoot_screenshot(id);
        match sharp_checker(&shot, 1600, 1000) {
            Ok(()) => break,
            Err(e) => assert!(Instant::now() < deadline, "{e}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_idle(daemon.id(), "after the decode");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// A compositor without `wp_fractional_scale_v1` (a debug build leaves it
/// unbound), or without a viewporter (the forced `full-shm` path stands for
/// one): the image falls back to the integer scale, the surface times 2
/// with `set_buffer_scale(2)`, which the compositor scales down: drawn, not
/// device-exact. `query` says what it draws at. A color is exact either
/// way: a 1×1 buffer under the viewport, or, with no viewporter, a
/// full-size one at the integer scale.
#[test]
fn without_fractional_scale_the_integer_scale_is_used() {
    if !cfg!(debug_assertions) {
        eprintln!("skipped -- the debug knobs exist only in debug builds");
        return;
    }
    for knob in [
        ("SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE", "1"),
        ("SCOOTBG_DEBUG_PATH", "full-shm"),
    ] {
        let Some(session) = Session::start_with("nofrac", 1, "[output]\nscale = 1.5\n") else {
            return;
        };
        let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client"), knob]);
        let output = configured(&session);
        assert_eq!(output["surface"]["scale"], 2, "{knob:?}");
        assert_eq!(
            output["surface"]["pixels"],
            json!({"width": 2134, "height": 1334})
        );
        let image = session.runtime_dir().join("checker.png");
        write_checker(&image, 1600, 1000);
        ok(
            &session,
            &["set", image.to_str().unwrap(), "--mode", "stretch"],
        );
        assert!(sent(&session, "get_fractional_scale").is_empty());
        assert_eq!(sent(&session, ", 0, 2134, 1334, 8536, 1)").len(), 1);
        assert_eq!(sent(&session, "set_buffer_scale(2)").len(), 1);
        let id = scoot_id(&session);
        // Not exact: a larger buffer scaled down resamples the checker.
        let shot = session.scoot_screenshot(id);
        assert!(sharp_checker(&shot, 1600, 1000).is_err(), "{knob:?}");
        ok(&session, &["set", "#c03020"]);
        let shot = session.scoot_screenshot(id);
        assert_eq!(shot.colors(), [[0xc0, 0x30, 0x20]], "{knob:?}");
        if knob.1 == "full-shm" {
            assert!(sent(&session, "get_viewport").is_empty());
            assert_eq!(sent(&session, ".create_buffer(").len(), 2, "one each");
        }
        kill(&session, &mut daemon);
    }
}

/// sway at 1.5 on 1600×1000 truncates the logical size to 1066×666, and
/// draws it `round(1066 × 1.5)` = 1599 by 999 device pixels: the buffer is
/// that, and lands one to one (the last column and row of the output are
/// sway's, uncovered by any 1066-wide surface). A scale change is followed
/// (wlroots sends the new `preferred_scale` when the output changes), and
/// an output plugged in at a fractional scale is drawn at its own.
#[test]
fn fractional_scales_are_exact_on_sway() {
    let Some(session) = Session::sway("fracsway") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    configured(&session);
    let name = configured(&session)["name"].as_str().unwrap().to_owned();
    // On screen first (a color), so wlroots sends the surface its new
    // scale as the output changes.
    ok(&session, &["set", "#102030"]);
    session.swaymsg(&[
        "--",
        "output",
        &name,
        "mode",
        "--custom",
        "1600x1000",
        "scale",
        "1.5",
    ]);
    let output = session
        .query_until("1066x666 at 1.5", |o| {
            o[0]["surface"]["size"] == json!({"width": 1066, "height": 666})
                && o[0]["surface"]["scale"] == 1.5
        })
        .remove(0);
    assert_eq!(output["surface"]["scale"], 1.5);
    assert_eq!(
        output["surface"]["pixels"],
        json!({"width": 1599, "height": 999})
    );
    let image = session.runtime_dir().join("checker.png");
    write_checker(&image, 1600, 1000);
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "center"],
    );
    assert_eq!(sent(&session, ", 0, 1599, 999, 6396, 1)").len(), 1);
    let shot = session.screencopy(&name);
    assert_eq!((shot.width, shot.height), (1600, 1000));
    sharp_checker(&shot, 1599, 999).unwrap_or_else(|e| panic!("sway at 1.5: {e}"));

    // 1.25: 1280×800, 1600×1000 pixels, the whole output.
    session.swaymsg(&["output", &name, "scale", "1.25"]);
    let deadline = Instant::now() + PATIENCE;
    while sent(&session, ", 0, 1600, 1000, 6400, 1)").is_empty() {
        assert!(Instant::now() < deadline, "never redrawn at 1.25");
        std::thread::sleep(Duration::from_millis(20));
    }
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.screencopy(&name);
        match sharp_checker(&shot, 1600, 1000) {
            Ok(()) => break,
            Err(e) => assert!(Instant::now() < deadline, "sway at 1.25: {e}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Plugged in with every output at 1.5 (sway's new headless output is
    // 1920×1080: 1280×720 logical, 1920×1080 pixels): drawn at its own
    // 1.5, the checker centred in the fill color, exact.
    session.swaymsg(&["output", "*", "scale", "1.5"]);
    session.swaymsg(&["create_output"]);
    let outputs = session.query_until("two, both at 1.5", |o| {
        o.len() == 2
            && o.iter().all(|o| {
                o["surface"]["scale"] == 1.5 && o["shows"]["image"] == image.to_str().unwrap()
            })
    });
    let second = outputs[1]["name"].as_str().unwrap().to_owned();
    assert_eq!(
        outputs[1]["surface"]["pixels"],
        json!({"width": 1920, "height": 1080})
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.screencopy(&second);
        assert_eq!((shot.width, shot.height), (1920, 1080));
        assert_eq!(shot.at(0, 0), BLACK, "the fill");
        match sharp_checker_in(&shot, (160, 40), (1600, 1000)) {
            Ok(()) => break,
            Err(e) => assert!(Instant::now() < deadline, "plugged in at 1.5: {e}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_idle(daemon.id(), "on sway");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// sway changed to 1.5 while nothing was on screen: wlroots re-sends a
/// surface's scales only while it is mapped, so the surface still says 1.0
/// and `wl_output` says 2. The larger wins: the first draw is 1066×666 at
/// buffer scale 2 (sharp, scaled down; never 1066×666 pixels scaled up),
/// and once on screen sway sends 1.5 and it is redrawn at 1599×999, exact.
/// A stale fraction that rounds up to the same integer as the new one is
/// `a_stale_fraction_that_rounds_alike_is_not_stretched_on_sway`.
#[test]
fn a_scale_gone_stale_while_unmapped_is_healed_on_sway() {
    let Some(session) = Session::sway("stale") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let name = configured(&session)["name"].as_str().unwrap().to_owned();
    session.swaymsg(&[
        "--",
        "output",
        &name,
        "mode",
        "--custom",
        "1600x1000",
        "scale",
        "1.5",
    ]);
    let output = session
        .query_until("1066x666", |o| {
            o[0]["surface"]["size"] == json!({"width": 1066, "height": 666})
        })
        .remove(0);
    assert_eq!(output["scale"], 2);
    assert_eq!(output["surface"]["scale"], 2, "the stale 1.0 gives way");
    let image = session.runtime_dir().join("checker.png");
    write_checker(&image, 1600, 1000);
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "center"],
    );
    let deadline = Instant::now() + PATIENCE;
    while sent(&session, ", 0, 1599, 999, 6396, 1)").is_empty() {
        assert!(Instant::now() < deadline, "never redrawn at 1.5");
        std::thread::sleep(Duration::from_millis(20));
    }
    let buffers = sent(&session, ".create_buffer(");
    assert_eq!(buffers.len(), 2, "{buffers:#?}");
    assert!(
        buffers[0].ends_with(", 0, 2132, 1332, 8528, 1)"),
        "{buffers:#?}"
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.screencopy(&name);
        match sharp_checker(&shot, 1599, 999) {
            Ok(()) => break,
            Err(e) => assert!(Instant::now() < deadline, "healed at 1.5: {e}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(session.query()["outputs"][0]["surface"]["scale"], 1.5);
    assert_idle(daemon.id(), "healed");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}

/// The buffers the daemon gave the compositor, as `(width, height)`.
fn buffers(session: &Session) -> Vec<(u32, u32)> {
    sent(session, ".create_buffer(")
        .iter()
        .filter_map(|line| {
            let args = line.split_once(".create_buffer(")?.1;
            let mut parts = args.split(", ").skip(2);
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
        .collect()
}

/// A stale fraction that rounds up to the same integer as the new one
/// (1.25 and 1.5 are both `wl_output` scale 2), so "the larger wins" cannot
/// see it: sway re-creates nothing while the wallpaper is cleared, and a
/// cleared surface is not on screen, so it keeps the 150 it was made with
/// after the output goes to 1.5. Its buffer (1066×666 at 1.25: 1333×833)
/// would fall 267 pixels short of the 1600-pixel mode and be stretched over
/// it. It is taken as stale instead: the image is drawn at the integer
/// scale (larger, scaled down, never up), and redrawn at 1599×999, exact,
/// once sway sends 180.
#[test]
fn a_stale_fraction_that_rounds_alike_is_not_stretched_on_sway() {
    let Some(session) = Session::sway("stalefr") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
    let name = configured(&session)["name"].as_str().unwrap().to_owned();
    session.swaymsg(&[
        "--",
        "output",
        &name,
        "mode",
        "--custom",
        "1600x1000",
        "scale",
        "1.25",
    ]);
    session.query_until("1280x800", |o| {
        o[0]["surface"]["size"] == json!({"width": 1280, "height": 800})
    });
    ok(&session, &["set", "#203040"]);
    ok(&session, &["clear"]);
    session.query_until("re-created at 1.25", |o| {
        o[0]["surface"]["size"] == json!({"width": 1280, "height": 800})
            && o[0]["surface"]["scale"] == 1.25
    });
    session.swaymsg(&["output", &name, "scale", "1.5"]);
    let output = session
        .query_until("1066x666", |o| {
            o[0]["surface"]["size"] == json!({"width": 1066, "height": 666})
        })
        .remove(0);
    assert_eq!(output["scale"], 2);
    assert_ne!(output["surface"]["scale"], 1.25, "the stale 1.25 is used");
    let image = session.runtime_dir().join("checker.png");
    write_checker(&image, 1600, 1000);
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "center"],
    );
    // Whatever was drawn by the reply covers the output's pixels: never a
    // buffer smaller than the 1599×999 sway draws the surface into.
    let drawn = buffers(&session);
    assert!(!drawn.is_empty());
    assert!(
        drawn.iter().all(|&(w, h)| w >= 1599 && h >= 999),
        "a buffer stretched over the output: {drawn:?}"
    );
    let deadline = Instant::now() + PATIENCE;
    while !buffers(&session).contains(&(1599, 999)) {
        assert!(
            Instant::now() < deadline,
            "never redrawn at 1.5: {:?}",
            buffers(&session)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let deadline = Instant::now() + PATIENCE;
    loop {
        let shot = session.screencopy(&name);
        match sharp_checker(&shot, 1599, 999) {
            Ok(()) => break,
            Err(e) => assert!(Instant::now() < deadline, "healed at 1.5: {e}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_idle(daemon.id(), "healed");
    assert_quiet_log(&session);
    kill(&session, &mut daemon);
}
