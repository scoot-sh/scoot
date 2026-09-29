//! Solid colors end to end: `set` and `clear` against a real
//! `scoot --headless` and a headless sway, checked by real pixels (scoot's
//! own screenshot, sway through wlr-screencopy) and by the protocol trace.
//!
//! The fallback paths (a 1×1 `wl_shm` buffer under a viewport, and a
//! full-size buffer) are forced with `SCOOTBG_DEBUG_PATH`, which only a
//! debug build reads; built without debug assertions those cases skip.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use common::{PATIENCE, Session, rgb, wait_exit};
use serde_json::{Value, json};

const RED: &str = "#c03020";
const BLUE: &str = "#101014";

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

/// What each output shows, by name, from `query`.
fn shows(session: &Session) -> Vec<(String, Value)> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["name"].as_str().unwrap().to_owned(), o["shows"].clone()))
        .collect()
}

fn color(hex: &str) -> Value {
    json!({ "color": hex })
}

/// scoot's outputs as `(id, name)`, for screenshots by name.
fn scoot_ids(session: &Session) -> Vec<(u64, String)> {
    let reply = session.scoot_ipc(r#"{"type":"outputs"}"#);
    reply["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["id"].as_u64().unwrap(),
                o["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
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

fn debug_paths_available() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    eprintln!("skipped -- SCOOTBG_DEBUG_PATH exists only in debug builds");
    false
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

/// Settles, then stays asleep for 1.5 s (see `hotplug.rs`).
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
    for quiet in ["panicked", "cannot draw", "giving up", "cannot accept"] {
        assert!(!log.contains(quiet), "{quiet}:\n{log}");
    }
}

/// The deferred screenshot check of ticket 3, and the ticket's own: on 1
/// and 2 outputs, after `set` returns, every pixel of every output (scoot's
/// own screenshot of each, the second included) is exactly the color. On
/// scoot that is the single-pixel path: no shared memory at all, the
/// viewport sized to the surface, the whole surface opaque, and no frame
/// callbacks; and the daemon is idle afterwards.
#[test]
fn a_color_covers_every_output_exactly_on_scoot() {
    for count in [1, 2] {
        let Some(session) = Session::start_with(&format!("col{count}"), count, "") else {
            return;
        };
        let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
        configured(&session, count as usize);
        ok(&session, &["set", RED]);
        for (name, shown) in shows(&session) {
            assert_eq!(shown, color(RED), "{name}");
        }
        let ids = scoot_ids(&session);
        assert_eq!(ids.len(), count as usize);
        for (id, name) in &ids {
            let shot = session.scoot_screenshot(*id);
            assert_eq!((shot.width, shot.height), (1600, 1000));
            shot.assert_all(rgb(RED), name);
        }

        let buffers = sent(&session, "create_u32_rgba_buffer");
        assert_eq!(buffers.len(), count as usize, "{buffers:?}");
        // v * 0x01010101 for c0, 30, 20; alpha opaque.
        assert!(
            buffers[0].ends_with("(3233857728, 808464432, 538976288, 4294967295)")
                || buffers[0].contains("3233857728, 808464432, 538976288, 4294967295"),
            "{buffers:?}"
        );
        assert!(sent(&session, "create_pool").is_empty(), "no shared memory");
        assert_eq!(
            sent(&session, "set_destination(1600, 1000)").len(),
            count as usize
        );
        assert_eq!(sent(&session, ".set_opaque_region(").len(), count as usize);
        assert_eq!(
            sent(&session, ".add(0, 0, 1600, 1000)").len(),
            count as usize
        );
        assert!(sent(&session, ".frame(").is_empty(), "no frame callbacks");
        assert!(
            sent(&session, "set_buffer_scale").is_empty(),
            "a 1x1 buffer stays at scale 1"
        );

        assert_idle(daemon.id(), "with a color set");

        // Viewport destination and opaque region persist: more color
        // changes at the same size send neither again. A fresh surface
        // (`clear`, then `set`) gets both once more, and so does a resize.
        let n = count as usize;
        ok(&session, &["set", BLUE]);
        ok(&session, &["set", RED]);
        assert_eq!(sent(&session, "create_u32_rgba_buffer").len(), 3 * n);
        assert_eq!(sent(&session, ".set_opaque_region(").len(), n);
        assert_eq!(sent(&session, ".set_destination(").len(), n);
        ok(&session, &["clear"]);
        ok(&session, &["set", BLUE]);
        assert_eq!(sent(&session, ".set_opaque_region(").len(), 2 * n);
        assert_eq!(sent(&session, ".set_destination(").len(), 2 * n);
        for (id, name) in &ids {
            session
                .scoot_screenshot(*id)
                .assert_all(rgb(BLUE), &format!("{name} after clear and set"));
        }
        std::fs::write(
            session.runtime_dir().join("config.toml"),
            "[output]\nscale = 2.0\n",
        )
        .unwrap();
        let reloaded = session.scoot_ipc(r#"{"type":"reload"}"#);
        assert_eq!(reloaded["type"], "reloaded", "{reloaded}");
        session.query_until("resized to 800x500", |o| {
            o.len() == n
                && o.iter()
                    .all(|o| o["surface"]["size"] == json!({"width": 800, "height": 500}))
        });
        ok(&session, &["set", BLUE]);
        assert_eq!(sent(&session, ".set_destination(800, 500)").len(), n);
        assert_eq!(sent(&session, ".add(0, 0, 800, 500)").len(), n);
        assert_eq!(sent(&session, ".set_opaque_region(").len(), 3 * n);
        for (id, name) in &ids {
            session
                .scoot_screenshot(*id)
                .assert_all(rgb(BLUE), &format!("{name} after the resize"));
        }

        assert_quiet_log(&session);
        assert!(session.run(&["kill"]).status.success());
        assert!(wait_exit(&mut daemon).success());
    }
}

/// `--output` changes one output only; `clear` takes the color off (the
/// compositor's background shows again) and `query` says `null`; an
/// unknown name is an error that changes nothing; `clear` everywhere, then
/// `set` again, works.
#[test]
fn one_output_is_targeted_and_cleared_on_scoot() {
    let Some(session) = Session::start("target") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    let outputs = configured(&session, 2);
    let names: Vec<String> = outputs
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect();
    let ids = scoot_ids(&session);
    let shot = |name: &str| {
        let id = ids.iter().find(|(_, n)| n == name).unwrap().0;
        session.scoot_screenshot(id)
    };
    // Nothing set yet: what scoot shows by itself.
    let background = shot(&names[0]).at(800, 500);
    assert_ne!(background, rgb(RED));
    shot(&names[1]).assert_all(background, "before any set");
    assert_eq!(shows(&session)[0].1, Value::Null);

    ok(&session, &["set", RED]);
    ok(&session, &["set", BLUE, "--output", &names[1]]);
    shot(&names[0]).assert_all(rgb(RED), "untouched by --output");
    shot(&names[1]).assert_all(rgb(BLUE), "targeted");
    assert_eq!(
        shows(&session),
        [
            (names[0].clone(), color(RED)),
            (names[1].clone(), color(BLUE)),
        ]
    );

    // An unknown name: an error, exit 1, nothing changed.
    let out = session.run(&["set", "#000000", "--output", "NOPE-9"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"NOPE-9\""), "{stderr}");
    assert!(stderr.contains("nothing was changed"), "{stderr}");
    shot(&names[0]).assert_all(rgb(RED), "after the refused set");

    ok(&session, &["clear", "--output", &names[0]]);
    shot(&names[0]).assert_all(background, "cleared");
    shot(&names[1]).assert_all(rgb(BLUE), "the other untouched");
    assert_eq!(shows(&session)[0].1, Value::Null);
    let state = &session.query()["outputs"][0]["surface"]["state"];
    assert_eq!(state, "configured", "a fresh surface, configured again");

    ok(&session, &["clear"]);
    for name in &names {
        shot(name).assert_all(background, "all cleared");
    }
    ok(&session, &["set", "#FFFFFF"]);
    for name in &names {
        shot(name).assert_all([255, 255, 255], "set again after clear");
    }
    assert_eq!(shows(&session)[1].1, color("#ffffff"), "lowercase");

    assert_quiet_log(&session);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// The reply comes only after the compositor has processed the commits:
/// a screenshot taken the moment `set` returns shows the new color, every
/// time, on both outputs.
#[test]
fn a_screenshot_straight_after_set_is_never_stale() {
    let Some(session) = Session::start("fresh") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 2);
    let ids = scoot_ids(&session);
    let colors = [
        "#c03020", "#101014", "#00ff00", "#0000ff", "#ffffff", "#000000",
    ];
    for round in 0..30 {
        let hex = colors[round % colors.len()];
        ok(&session, &["set", hex]);
        for (id, name) in &ids {
            let shot = session.scoot_screenshot(*id);
            for (point, pixel) in shot.samples() {
                assert_eq!(pixel, rgb(hex), "round {round}, {name} {point}: stale");
            }
        }
    }
    assert_quiet_log(&session);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// One connection, one write: `set`, then `query`, then `version`. The
/// replies come in order, the `query` after the `set` has been shown.
#[test]
fn requests_behind_a_waiting_set_are_answered_in_order() {
    let Some(session) = Session::start_with("pipe", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let stream = UnixStream::connect(session.socket()).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    (&stream)
        .write_all(
            b"{\"protocol\":1,\"type\":\"set\",\"color\":\"#c03020\"}\n\
              {\"protocol\":1,\"type\":\"query\"}\n\
              {\"protocol\":1,\"type\":\"version\"}\n",
        )
        .unwrap();
    let mut lines = BufReader::new(&stream).lines();
    let mut next = || -> Value { serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap() };
    assert_eq!(next(), json!({"type": "ok"}));
    let query = next();
    assert_eq!(query["outputs"][0]["shows"], color(RED), "{query}");
    assert_eq!(next()["type"], "version");
    drop(stream);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// `set`s pipelined on one connection: each is handled when the previous
/// one's reply is delivered, and must be answered too. The second sets the
/// same color, so it sends the compositor nothing and no event comes back
/// to wake the daemon: only the loop itself can notice it is done.
#[test]
fn a_set_pipelined_behind_a_set_is_answered() {
    let Some(session) = Session::start_with("pipe2", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    let stream = UnixStream::connect(session.socket()).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    (&stream)
        .write_all(
            b"{\"protocol\":1,\"type\":\"set\",\"color\":\"#c03020\"}\n\
              {\"protocol\":1,\"type\":\"set\",\"color\":\"#c03020\"}\n\
              {\"protocol\":1,\"type\":\"clear\"}\n\
              {\"protocol\":1,\"type\":\"query\"}\n",
        )
        .unwrap();
    let mut lines = BufReader::new(&stream).lines();
    let mut next = || -> Value { serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap() };
    for _ in 0..3 {
        assert_eq!(next(), json!({"type": "ok"}));
    }
    assert_eq!(next()["outputs"][0]["shows"], Value::Null);
    drop(stream);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// A client that sends `set` and hangs up at once never gets its reply;
/// the change still happens, and the daemon carries on. Many of them, fast,
/// stay within the bound on waiting requests or are refused, never pile up.
#[test]
fn a_client_that_leaves_before_its_reply_is_fine() {
    let Some(session) = Session::start_with("leave", 1, "") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    configured(&session, 1);
    for round in 0..100 {
        let stream = UnixStream::connect(session.socket()).unwrap();
        let hex = if round % 2 == 0 { RED } else { BLUE };
        let line = format!("{{\"protocol\":1,\"type\":\"set\",\"color\":\"{hex}\"}}\n");
        (&stream).write_all(line.as_bytes()).unwrap();
        drop(stream);
    }
    // The last one wins; a `set` that waits for it proves it was shown.
    ok(&session, &["set", BLUE]);
    let ids = scoot_ids(&session);
    session
        .scoot_screenshot(ids[0].0)
        .assert_all(rgb(BLUE), "after the flood");
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon died");
    assert_quiet_log(&session);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// How many wallpaper memfds the daemon maps once it has let go of every
/// buffer the compositor released: the same count on four reads 100 ms
/// apart.
fn settled_memfds(pid: u32) -> usize {
    let count = || {
        std::fs::read_to_string(format!("/proc/{pid}/maps"))
            .unwrap()
            .lines()
            .filter(|l| l.contains("memfd:scootbg-wallpaper"))
            .count()
    };
    let deadline = Instant::now() + PATIENCE;
    let mut last = count();
    let mut stable = 0;
    while stable < 3 {
        assert!(Instant::now() < deadline, "memfds never settled");
        std::thread::sleep(Duration::from_millis(100));
        let now = count();
        stable = if now == last { stable + 1 } else { 0 };
        last = now;
    }
    last
}

/// The fallback paths, forced on scoot, which offers everything: a 1×1
/// `wl_shm` buffer under a viewport, and a full-size buffer. Both are
/// exact to the edge. (A viewport-upscaled 1×1 shm buffer used to fade at
/// every edge under scoot's pixman renderer; that was fixed in the
/// scoot-sh/smithay fork, docs/backlog/resolved/shm-viewport-upscale-edge-fade-done.md,
/// and this is its check from the client side.) A color change reuses
/// the released buffer on screen or takes a second one, and the one it
/// replaced goes once released: one buffer per output at rest, no spare;
/// a new scale redraws the full-size buffer at the new size.
#[test]
fn the_fallback_paths_are_exact_on_scoot() {
    if !debug_paths_available() {
        return;
    }
    for path in ["viewport-shm", "full-shm"] {
        let Some(session) = Session::start_with(&format!("fb-{path}"), 1, "") else {
            return;
        };
        let mut daemon =
            session.daemon_logged(&[("WAYLAND_DEBUG", "client"), ("SCOOTBG_DEBUG_PATH", path)]);
        configured(&session, 1);
        let id = scoot_ids(&session)[0].0;
        for hex in [RED, BLUE, RED, BLUE, "#00ff00", RED] {
            ok(&session, &["set", hex]);
            session
                .scoot_screenshot(id)
                .assert_all(rgb(hex), &format!("{path} {hex}"));
        }
        assert!(
            sent(&session, "create_u32_rgba_buffer").is_empty(),
            "{path}"
        );
        let pools = sent(&session, "create_pool");
        assert!(
            (1..=6).contains(&pools.len()),
            "{path}: {} buffers for six changes: {pools:?}",
            pools.len()
        );
        assert_eq!(
            settled_memfds(daemon.id()),
            1,
            "{path}: one buffer at rest, no spare"
        );
        let buffers = sent(&session, ".create_buffer(");
        let (dims, viewport) = if path == "viewport-shm" {
            ("1, 1, 4, 1)", 1)
        } else {
            ("1600, 1000, 6400, 1)", 0)
        };
        assert!(buffers.iter().all(|b| b.ends_with(dims)), "{buffers:?}");
        assert_eq!(sent(&session, "get_viewport").len(), viewport, "{path}");

        // A new scale: scoot sends 800x500 at wl_output scale 2.
        std::fs::write(
            session.runtime_dir().join("config.toml"),
            "[output]\nscale = 2.0\n",
        )
        .unwrap();
        let reloaded = session.scoot_ipc(r#"{"type":"reload"}"#);
        assert_eq!(reloaded["type"], "reloaded", "{reloaded}");
        session.query_until("reconfigured at scale 2", |o| {
            o.len() == 1 && o[0]["surface"]["size"] == json!({"width": 800, "height": 500})
        });
        // A set that changes nothing still waits for everything to be
        // shown, so after it the redraw at the new size is done.
        ok(&session, &["set", RED]);
        let shot = session.scoot_screenshot(id);
        assert_eq!((shot.width, shot.height), (1600, 1000), "physical pixels");
        shot.assert_all(rgb(RED), &format!("{path} at scale 2"));
        if path == "full-shm" {
            // Only the buffer for the new size, at scale 2: never one at
            // the old surface size and the new scale (3200x2000), which a
            // redraw on `wl_output.done` before the `configure` would make.
            let buffers = sent(&session, ".create_buffer(");
            assert!(
                buffers.iter().all(|b| b.ends_with("1600, 1000, 6400, 1)")),
                "{buffers:#?}"
            );
            assert_eq!(sent(&session, "set_buffer_scale(2)").len(), 1);
        } else {
            assert!(!sent(&session, "set_destination(800, 500)").is_empty());
        }
        assert!(sent(&session, ".frame(").is_empty());
        assert_idle(daemon.id(), path);
        assert_quiet_log(&session);
        assert!(session.run(&["kill"]).status.success());
        assert!(wait_exit(&mut daemon).success());
    }
}

/// On sway: every path draws exactly (read back through wlr-screencopy),
/// a new output gets the color for every output as soon as it is
/// configured, and a scale change redraws.
#[test]
fn colors_on_sway_every_path_and_hotplug() {
    let paths: &[Option<&str>] = if debug_paths_available() {
        &[None, Some("viewport-shm"), Some("full-shm")]
    } else {
        &[None]
    };
    for path in paths {
        let Some(session) = Session::sway(&format!("sw-{}", path.unwrap_or("best"))) else {
            return;
        };
        let mut env = vec![("WAYLAND_DEBUG", "client")];
        if let Some(path) = path {
            env.push(("SCOOTBG_DEBUG_PATH", path));
        }
        let mut daemon = session.daemon_logged(&env);
        let first = configured(&session, 1)[0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(&session, &["set", RED]);
        session
            .screencopy(&first)
            .assert_all(rgb(RED), &format!("{path:?}"));
        if path.is_none() {
            assert_eq!(
                sent(&session, "create_u32_rgba_buffer").len(),
                1,
                "sway has it"
            );
        }

        // Hotplug: the new output shows the every-output color.
        session.swaymsg(&["create_output"]);
        let outputs = configured(&session, 2);
        let second = outputs
            .iter()
            .map(|o| o["name"].as_str().unwrap().to_owned())
            .find(|n| *n != first)
            .unwrap();
        session.query_until("the new output shows the color", |o| {
            o.iter().all(|o| o["shows"] == color(RED))
        });
        // A `set` of the same color waits for every output to show it.
        ok(&session, &["set", RED]);
        session
            .screencopy(&second)
            .assert_all(rgb(RED), "hotplugged");

        ok(&session, &["set", BLUE, "--output", &second]);
        session
            .screencopy(&second)
            .assert_all(rgb(BLUE), "targeted");
        session.screencopy(&first).assert_all(rgb(RED), "untouched");

        // Scale 2: sway reconfigures; the redraw is exact.
        session.swaymsg(&["output", &second, "scale", "2"]);
        session.query_until("scale 2", |o| {
            o.iter()
                .any(|o| o["name"] == second.as_str() && o["scale"] == 2)
        });
        ok(&session, &["set", BLUE, "--output", &second]);
        session
            .screencopy(&second)
            .assert_all(rgb(BLUE), "at scale 2");

        // Unplugged and plugged back under the same name? sway names new
        // outputs afresh, so a new output gets the every-output color.
        session.swaymsg(&["output", &second, "unplug"]);
        session.swaymsg(&["create_output"]);
        let third = configured(&session, 2)
            .iter()
            .map(|o| o["name"].as_str().unwrap().to_owned())
            .find(|n| *n != first)
            .unwrap();
        ok(&session, &["set", RED, "--output", &first]);
        session
            .screencopy(&third)
            .assert_all(rgb(RED), "a new output");

        ok(&session, &["clear"]);
        assert!(shows(&session).iter().all(|(_, s)| s.is_null()));
        assert!(sent(&session, ".frame(").is_empty());
        assert_idle(daemon.id(), "sway, cleared");
        assert_quiet_log(&session);
        assert!(session.run(&["kill"]).status.success());
        assert!(wait_exit(&mut daemon).success());
    }
}

/// An output unplugged just as a `set --output` for it arrives, raced ten
/// times. The reply is one of exactly two things, depending on which the
/// daemon handles first: `ok` if it took the request while the output was
/// there (the reply then covers the outputs that remain, and the output's
/// removal ends the wait), or the unknown-output error, which changes
/// nothing, if the removal came first. Never a hang and never anything
/// else; the every-output `set` queued behind it always succeeds, and the
/// remaining output shows it.
#[test]
fn an_output_unplugged_during_a_set_still_gets_a_reply() {
    let Some(session) = Session::sway("unplug") else {
        return;
    };
    let mut daemon = session.daemon_logged(&[]);
    let (mut ok_first, mut unknown_first) = (0, 0);
    for round in 0..10 {
        session.swaymsg(&["create_output"]);
        let outputs = configured(&session, 2);
        let doomed = outputs[1]["name"].as_str().unwrap().to_owned();
        let stream = UnixStream::connect(session.socket()).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let line = format!(
            "{{\"protocol\":1,\"type\":\"set\",\"color\":\"{RED}\",\"output\":\"{doomed}\"}}\n\
             {{\"protocol\":1,\"type\":\"set\",\"color\":\"{BLUE}\"}}\n"
        );
        (&stream).write_all(line.as_bytes()).unwrap();
        session.swaymsg(&["output", &doomed, "unplug"]);
        let mut lines = BufReader::new(&stream).lines();
        let first: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if first == json!({"type": "ok"}) {
            ok_first += 1;
        } else {
            assert_eq!(first["type"], "error", "round {round}: {first}");
            let message = first["message"].as_str().unwrap();
            assert!(
                message.contains(&format!("{doomed:?}")) && message.contains("nothing was changed"),
                "round {round}: {message}"
            );
            unknown_first += 1;
        }
        let second: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(second, json!({"type": "ok"}), "round {round}");
        let left = configured(&session, 1)[0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        session
            .screencopy(&left)
            .assert_all(rgb(BLUE), "the remaining output");
    }
    eprintln!("ok first: {ok_first}, unknown output first: {unknown_first}");
    assert_quiet_log(&session);
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}
