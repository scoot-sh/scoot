//! Tests for the leaf steps: every backend scripts through fakes, so no
//! test touches the session bus or a real daemon.

use super::*;

use std::collections::BTreeMap;

/// A minimal valid look: the moonrise palette (which clears the contrast
/// bars), a color wallpaper (no image binary needed), and a license (so it
/// validates under every origin). A local copy, as in `apply/tests.rs`
/// and `toolkit/tests.rs`: the catalog's own helpers are not visible
/// outside `scoot-look`.
fn look_toml(name: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "dark"
license = "MIT"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DejaVu Sans"
sans = "DejaVu Sans"
mono = "DejaVu Sans Mono"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "#2B3648"
mode = "fill"
"##
    )
}

fn look_named(name: &str, night: Option<i64>) -> scoot_look::ResolvedLook {
    let text = look_toml(name);
    let raw = scoot_look::parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([(name.to_string(), (raw, scoot_look::Origin::User))]);
    let mut look = scoot_look::resolve(&map, name).expect("resolves");
    look.night_temperature = night;
    look
}

#[test]
fn all_backends_ok() {
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &ScriptedLeaves::ok(),
    );
    assert_eq!(
        report.reloaded,
        vec![
            "mako",
            "fcitx5",
            "scoot-osd unit",
            "scoot-nightlight unit (3400K)"
        ]
    );
    assert!(report.notes.is_empty(), "notes: {:?}", report.notes);
}

#[test]
fn missing_backends_are_notes() {
    let leaves = ScriptedLeaves::failing();
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &leaves,
    );
    assert!(report.reloaded.is_empty());
    assert_eq!(report.notes.len(), 5, "notes: {:?}", report.notes);
    assert!(report.notes.iter().any(|n| n.contains("mako")));
    assert!(report.notes.iter().any(|n| n.contains("scootnotify")));
    assert!(report.notes.iter().any(|n| n.contains("fcitx5")));
    assert!(report.notes.iter().any(|n| n.contains("wob")));
    assert!(report.notes.iter().any(|n| n.contains("3400K")));
    // Both units were still attempted (best-effort runs everything).
    assert_eq!(
        leaves.restarted().as_slice(),
        &["scoot-osd", "scoot-nightlight"]
    );
}

/// When mako is absent the scootnotify leaf reloads instead: the running
/// notifier re-reads its look layer, keeping cards, history and DND.
#[test]
fn scootnotify_reloads_when_mako_is_absent() {
    let leaves = ScriptedLeaves::failing();
    leaves.script_notify(vec![Ok(())]);
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &leaves,
    );
    assert!(
        report.reloaded.contains(&"scootnotify".to_owned()),
        "reloaded: {:?}",
        report.reloaded
    );
    assert!(
        report.notes.iter().any(|n| n.contains("mako")),
        "the mako miss is still a note: {:?}",
        report.notes
    );
    assert!(
        !report.notes.iter().any(|n| n.contains("scootnotify")),
        "a successful notify reload leaves no note: {:?}",
        report.notes
    );
}

/// When the mako leaf already reloaded, the daemon is mako, so no
/// scootnotify reload is attempted at all (the leaf already handles mako).
#[test]
fn mako_active_skips_notify_entirely() {
    // `ok()` scripts mako success and a loud notify failure: a stray
    // notify call would fail the swap's notes. It must never be called.
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &ScriptedLeaves::ok(),
    );
    assert!(
        report.reloaded.contains(&"mako".to_owned()),
        "reloaded: {:?}",
        report.reloaded
    );
    assert!(
        !report.reloaded.contains(&"scootnotify".to_owned()),
        "no notify reload beside mako: {:?}",
        report.reloaded
    );
    assert!(report.notes.is_empty(), "notes: {:?}", report.notes);
}

#[test]
fn no_night_temperature_skips_the_unit() {
    let leaves = ScriptedLeaves::ok();
    let report = apply_leaves(Path::new("/state"), &look_named("x", None), &leaves);
    assert_eq!(report.reloaded, vec!["mako", "fcitx5", "scoot-osd unit"]);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("no night temperature")),
        "notes: {:?}",
        report.notes
    );
    assert_eq!(leaves.restarted().as_slice(), &["scoot-osd"]);
}

#[test]
fn private_state_runs_nothing() {
    let leaves = ScriptedLeaves::ok();
    let dir = Path::new("/run/user/1000/scoot-look-test");
    let report = apply_leaves(dir, &look_named("moonrise", Some(3400)), &leaves);
    assert!(report.reloaded.is_empty());
    assert_eq!(report.notes.len(), 1);
    assert!(report.notes[0].contains("private look state"));
    assert!(leaves.restarted().is_empty());
}

/// A unit that was never installed is skipped silently: no reload, no
/// note. (On a non-systemd box every swap would otherwise nag about
/// units that never existed.)
#[test]
fn absent_units_are_silent() {
    struct AbsentUnits;
    impl super::Leaves for AbsentUnits {
        fn mako_reload(&self) -> Result<(), String> {
            Ok(())
        }
        fn notify_reload(&self) -> Result<(), String> {
            Ok(())
        }
        fn fcitx5_reload(&self) -> Result<(), String> {
            Ok(())
        }
        fn unit_try_restart(&self, _unit: &str) -> Result<(), String> {
            Err(super::UNIT_ABSENT.to_owned())
        }
        fn helix_signal(&self) -> Result<usize, String> {
            Ok(0)
        }
        fn btop_signal(&self) -> Result<usize, String> {
            Ok(0)
        }
    }
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &AbsentUnits,
    );
    assert_eq!(report.reloaded, vec!["mako", "fcitx5"]);
    assert!(report.notes.is_empty(), "notes: {:?}", report.notes);
}

#[test]
fn unit_absent_patterns() {
    for output in [
        "Unit scoot-osd.service not found.",
        "Failed to get unit: Unit not found.",
        "Unknown unit: scoot-osd.service",
        "System has not been booted with systemd as init system.",
        "No such file or directory",
        "Could not find unit scoot-osd",
    ] {
        assert!(super::is_unit_absent(output), "absent: {output}");
    }
    for output in [
        "Failed to restart unit: Unit entered failed state.",
        "no `systemctl` on PATH",
        "Job failed. See \"journalctl --user -xe\" for details.",
    ] {
        assert!(!super::is_unit_absent(output), "present: {output}");
    }
}

#[test]
fn leaf_comm_shapes() {
    // Bare names and full paths match exactly.
    assert!(super::leaf_comm("hx", "hx"));
    assert!(super::leaf_comm("hx", "/nix/store/abc-helix/bin/hx"));
    assert!(super::leaf_comm("btop", "btop"));
    // A Nix-style wrapper launcher unwraps to its inner name (Helix on
    // NixOS runs as `.hx-wrapped`, verified live).
    assert!(super::leaf_comm("hx", ".hx-wrapped"));
    assert!(super::leaf_comm("btop", ".btop-wrapped"));
    assert!(super::leaf_comm(
        "hx",
        "/run/current-system/sw/bin/.hx-wrapped"
    ));
    // Near misses never match: a longer name, a wrapper for another
    // app, a bare `-wrapped` without the leading dot.
    assert!(!super::leaf_comm("hx", "hx-something"));
    assert!(!super::leaf_comm("hx", ".btop-wrapped"));
    assert!(!super::leaf_comm("hx", "hx-wrapped"));
    assert!(!super::leaf_comm("hx", ""));
    assert!(!super::leaf_comm("hx", ".hx-wrappe"));
}

/// One-shot fake scootnotify control socket: accepts a single connection,
/// reads one request line, optionally checks it, writes `reply`, and
/// returns the request. The listener lives on a scratch socket path (short
/// enough for `sun_path`), so no test touches the session's real daemon.
fn fake_notify_server(socket: &std::path::Path, reply: &str) -> std::thread::JoinHandle<String> {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::os::unix::net::UnixListener::bind(socket).expect("a fake socket");
    let reply = reply.to_owned();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the reload connects");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("a timeout");
        let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
        let mut request = String::new();
        reader.read_line(&mut request).expect("the request reads");
        stream
            .write_all(reply.as_bytes())
            .expect("the reply writes");
        request
    })
}

fn scratch_socket(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::with_prefix(format!("snc-{tag}-")).expect("a scratch dir");
    let socket = dir.path().join("scootnotify-wayland-0.sock");
    assert!(
        socket.as_os_str().len() <= 107,
        "fake socket path too long: {}",
        socket.display()
    );
    (dir, socket)
}

#[test]
fn notify_good_reload_resolves() {
    let (_dir, socket) = scratch_socket("good");
    let server = fake_notify_server(&socket, "{\"type\":\"ok\",\"scope\":\"look\"}\n");
    super::notify_look_reload(&socket).expect("a good reload resolves");
    let request = server.join().expect("the server joins");
    assert!(
        request.contains("\"protocol\":2"),
        "the request speaks protocol 2: {request}"
    );
    assert!(
        request.contains("\"scope\":\"look\""),
        "the reload is scoped to the look: {request}"
    );
}

#[test]
fn notify_refused_reload_is_loud() {
    let (_dir, socket) = scratch_socket("refused");
    let server = fake_notify_server(
        &socket,
        "{\"type\":\"error\",\"message\":\"radius = \\n\"}\n",
    );
    let error = super::notify_look_reload(&socket).expect_err("a refusal fails");
    assert!(error.contains("refused"), "loud, not silent: {error}");
    assert!(
        error.contains("radius"),
        "the daemon's message survives: {error}"
    );
    server.join().expect("the server joins");
}

#[test]
fn notify_protocol_mismatch_is_loud() {
    let (_dir, socket) = scratch_socket("mismatch");
    let server = fake_notify_server(
        &socket,
        "{\"type\":\"error\",\"message\":\"protocol mismatch: the request speaks 2, this daemon speaks 1\"}\n",
    );
    let error = super::notify_look_reload(&socket).expect_err("a mismatch fails");
    assert!(
        error.contains("mismatch"),
        "the version wall names itself: {error}"
    );
    server.join().expect("the server joins");
}

#[test]
fn notify_plain_ok_without_scope_is_refused() {
    // A plain `ok` answers a different request (the unscoped reload): the
    // leaf must refuse it loudly, never treat a scopeless receipt as the
    // scoped one it asked for.
    let (_dir, socket) = scratch_socket("plainscope");
    let server = fake_notify_server(&socket, "{\"type\":\"ok\"}\n");
    let error = super::notify_look_reload(&socket).expect_err("a scopeless ok fails");
    assert!(
        error.contains("scope"),
        "the missing receipt names itself: {error}"
    );
    server.join().expect("the server joins");
}

#[test]
fn notify_stalled_peer_never_hangs_apply() {
    use std::io::BufRead as _;
    let (_dir, socket) = scratch_socket("stalled");
    let listener = std::os::unix::net::UnixListener::bind(&socket).expect("a fake socket");
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the reload connects");
        let mut reader = std::io::BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).expect("the request reads");
        // Accepted and read, never answered: the caller must time out
        // instead of hanging the swap.
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
    let start = std::time::Instant::now();
    let error =
        super::notify_look_reload_with_timeout(&socket, std::time::Duration::from_millis(200))
            .expect_err("a stall fails");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "bounded, never the 30 s sleep"
    );
    assert!(
        error.contains("did not answer")
            || error.contains("Timed out")
            || error.contains("timed out"),
        "a stall reads as unanswered: {error}"
    );
    // The sleeper is detached: the test proved the bound without waiting
    // for it (a join would hang the suite 30 s).
    let _ = server.thread().id();
}

#[test]
fn notify_no_socket_is_a_skip_note() {
    let dir = tempfile::TempDir::with_prefix("snc-nosock-").expect("a scratch dir");
    let socket = dir.path().join("scootnotify-wayland-0.sock");
    let error = super::notify_look_reload(&socket).expect_err("no listener fails");
    assert!(
        error.contains("nothing listens"),
        "absent reads as not-running, never a refusal: {error}"
    );
}

#[test]
fn notify_socket_path_shapes() {
    use std::ffi::OsStr;
    // Unset/empty display names the default; the name is the final
    // component, sanitized.
    assert_eq!(
        super::notify_socket_path(None, Some(OsStr::new("/run/user/1000"))).expect("a path"),
        std::path::PathBuf::from("/run/user/1000/scootnotify-wayland-0.sock")
    );
    assert_eq!(
        super::notify_socket_path(Some(OsStr::new("")), Some(OsStr::new("/run/user/1000")))
            .expect("a path"),
        std::path::PathBuf::from("/run/user/1000/scootnotify-wayland-0.sock")
    );
    assert_eq!(
        super::notify_socket_path(
            Some(OsStr::new("/tmp/wayland-1")),
            Some(OsStr::new("/run/user/1000"))
        )
        .expect("a path"),
        std::path::PathBuf::from("/run/user/1000/scootnotify-wayland-1.sock")
    );
    // No runtime dir, a relative one, and an overlong path refuse instead
    // of naming a socket the daemon never listens on.
    assert!(super::notify_socket_path(None, None).is_err());
    assert!(super::notify_socket_path(None, Some(OsStr::new("relative"))).is_err());
    let long = format!("/run/user/1000/{}", "x".repeat(200));
    assert!(super::notify_socket_path(None, Some(OsStr::new(&long))).is_err());
}

/// The live-daemon end to end: the real `scootnotify` binary headless on a
/// private bus, driven through the same leaf wire `apply` uses. A card
/// placed before the reload is still listed after it under the same pid
/// (no restart: a restart drops cards, history and DND), a changed
/// fragment applies (the reply carries the look scope receipt), and a
/// half-written fragment refuses while the daemon keeps the last good
/// look. Headless has no popup surface, so "re-theme" here is the daemon
/// accepting the new layer without restarting, not pixels.
///
/// Needs `dbus-daemon` on `PATH` and the built `scootnotify` binary:
/// without either the test skips, saying so.
#[cfg(target_os = "linux")]
#[test]
fn notify_live_daemon_rethemes_without_restart() {
    use std::io::{BufRead, BufReader, Write};
    use std::time::{Duration, Instant};

    let Some(bus) = scoot_dbus::testdaemon::Daemon::spawn() else {
        eprintln!("skipped: no dbus-daemon on PATH");
        return;
    };
    let Some(binary) = scootnotify_binary() else {
        eprintln!("skipped: no scootnotify binary beside the test binary");
        return;
    };
    // A scratch tree for the run: its runtime dir (the control socket),
    // its look state (the `look/notify.toml` layer the reload re-reads),
    // and the homes the daemon reads. Nothing real is touched, and the
    // parent process environment is never mutated (every path travels in
    // the child's env or as an explicit argument).
    let root = tempfile::TempDir::with_prefix("snc-leaf-e2e-").expect("a scratch dir");
    assert!(
        root.path().as_os_str().len() < 60,
        "scratch prefix too long for the control socket path"
    );
    let runtime = root.path().join("runtime");
    let state = root.path().join("state");
    let config = root.path().join("config");
    let home = root.path().join("home");
    for dir in [&runtime, &state, &config, &home] {
        std::fs::create_dir_all(dir).expect("a scratch dir");
    }
    // The look layer, v1: a single valid key the reload picks up.
    let layer = state.join("look");
    std::fs::create_dir_all(&layer).expect("a look dir");
    std::fs::write(layer.join("notify.toml"), "radius = 4\n").expect("a look layer");
    // The daemon's socket name derives from its own display: pin it.
    let display = "w41-e2e";
    let socket = super::notify_socket_path(
        Some(std::ffi::OsStr::new(display)),
        Some(runtime.as_os_str()),
    )
    .expect("a socket path");
    let mut server = std::process::Command::new(&binary)
        .arg("daemon")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", bus.path().display()),
        )
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("WAYLAND_DISPLAY", display)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", root.path().join("xdg-state"))
        .env("HOME", &home)
        .env("SCOOT_LOOK_STATE", &state)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the daemon starts");
    let pid = server.id();
    // The serving probe, as the daemon's own suites wait: one
    // `GetServerInformation` round trip (no card, so no live-count or
    // history-count interference), resent while silent, failing fast on a
    // dead child instead of burning the budget.
    let mut client = scoot_dbus::conn::connect(&bus.path()).expect("the bus connects");
    wait_serving(&mut client, &mut server);
    // One card through the bus: it must survive every reload below.
    let id = bus_notify(&mut client, "Mail", "Leaf re-theme");
    assert_eq!(control_list_ids(&socket).as_slice(), &[id]);
    // The layer changes out from under the daemon; the leaf wire (the
    // same call `apply_leaves` makes) picks it up with the scope receipt.
    std::fs::write(layer.join("notify.toml"), "radius = 12\n").expect("a changed layer");
    super::notify_look_reload(&socket).expect("the scoped reload resolves");
    assert_eq!(server.id(), pid, "the same daemon, not a restart");
    assert_eq!(
        control_list_ids(&socket).as_slice(),
        &[id],
        "the card survives the reload"
    );
    // The apply leaf path reports it the way a swap summary would.
    struct LiveNotify<'a> {
        socket: &'a std::path::Path,
    }
    impl super::Leaves for LiveNotify<'_> {
        fn mako_reload(&self) -> Result<(), String> {
            Err("no `makoctl` on PATH".to_owned())
        }
        fn notify_reload(&self) -> Result<(), String> {
            super::notify_look_reload(self.socket)
        }
        fn fcitx5_reload(&self) -> Result<(), String> {
            Err("no `busctl` on PATH".to_owned())
        }
        fn unit_try_restart(&self, _unit: &str) -> Result<(), String> {
            Err(super::UNIT_ABSENT.to_owned())
        }
        fn helix_signal(&self) -> Result<usize, String> {
            Err("no `ps` on PATH".to_owned())
        }
        fn btop_signal(&self) -> Result<usize, String> {
            Err("no `ps` on PATH".to_owned())
        }
    }
    let report = super::apply_leaves(
        std::path::Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &LiveNotify { socket: &socket },
    );
    assert!(
        report.reloaded.contains(&"scootnotify".to_owned()),
        "the apply leaf reports the reload: {report:?}"
    );
    assert_eq!(server.id(), pid, "the apply leaf restarts nothing");
    assert_eq!(
        control_list_ids(&socket).as_slice(),
        &[id],
        "the card survives the apply leaf too"
    );
    // A half-written fragment refuses loudly and the daemon keeps the
    // last good look (and the card).
    std::fs::write(layer.join("notify.toml"), "radius = \n").expect("a bad layer");
    let error = super::notify_look_reload(&socket).expect_err("a bad layer refuses");
    assert!(error.contains("refused"), "loud, not silent: {error}");
    assert_eq!(
        control_list_ids(&socket).as_slice(),
        &[id],
        "the card stands through the refusal"
    );
    assert_eq!(server.id(), pid, "a refusal restarts nothing either");
    server.kill().ok();
    server.wait().ok();

    /// Pumps `client` until its `GetServerInformation` probe is answered.
    fn wait_serving(client: &mut scoot_dbus::conn::Conn, server: &mut std::process::Child) {
        use scoot_dbus::conn::Event;
        let start = Instant::now();
        let budget = Duration::from_secs(30);
        let mut last_sent = Instant::now() - Duration::from_secs(10);
        loop {
            assert!(
                start.elapsed() < budget,
                "the daemon never owned the bus name in {budget:?}"
            );
            if let Some(status) = server.try_wait().expect("the server reaps") {
                panic!("the server died at startup ({status})");
            }
            if last_sent.elapsed() >= Duration::from_secs(1) {
                client
                    .call(
                        "org.freedesktop.Notifications",
                        "/org/freedesktop/Notifications",
                        "org.freedesktop.Notifications",
                        "GetServerInformation",
                        "",
                        &[],
                        0,
                        9000,
                    )
                    .expect("the probe queues");
                last_sent = Instant::now();
            }
            let (events, _) = client.pump();
            for event in events {
                if matches!(event, Event::Reply { token: 9000, .. }) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// One `Notify` through the bus: its id. Times out loudly instead of
    /// hanging the suite.
    fn bus_notify(client: &mut scoot_dbus::conn::Conn, app: &str, summary: &str) -> u32 {
        use scoot_dbus::conn::Event;
        use scoot_dbus::proto::{Reader, Writer};
        let mut out = Writer::new();
        out.str(app);
        out.u32(0);
        out.str("");
        out.str(summary);
        out.str("body");
        let actions = out.open_array(4).unwrap();
        out.close_array(actions);
        let hints = out.open_array(8).unwrap();
        out.close_array(hints);
        out.i32(-1);
        let body = out.take_body().unwrap();
        client
            .call(
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify",
                "susssasa{sv}i",
                &body,
                0,
                4242,
            )
            .expect("the call queues");
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < Duration::from_secs(10), "no Notify reply");
            let (events, _) = client.pump();
            for event in events {
                if let Event::Reply {
                    token: 4242, body, ..
                } = event
                {
                    return Reader::le(&body).u32().expect("an id");
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The live ids over the control socket: connects, asks `list`, reads
    /// the one reply line. A scope-free helper for the test (the leaf's
    /// own wire stays the thing under test).
    fn control_list_ids(socket: &std::path::Path) -> Vec<u32> {
        use std::io::Read as _;
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(socket).expect("the daemon listens");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        stream
            .write_all(b"{\"protocol\":2,\"type\":\"list\"}\n")
            .expect("the request writes");
        let mut reader = BufReader::new((&stream).take(64 * 1024));
        let mut line = String::new();
        reader.read_line(&mut line).expect("the reply reads");
        let reply: serde_json::Value = serde_json::from_str(&line).expect("the reply is JSON");
        assert_eq!(reply["type"], "notifications");
        reply["notifications"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|card| card["id"].as_u64().expect("an id") as u32)
            .collect()
    }

    /// The built daemon binary beside the test binary (the VM builds both
    /// into one target dir before running the suite), else the workspace
    /// target dir, else none (the test skips).
    fn scootnotify_binary() -> Option<std::path::PathBuf> {
        if let Ok(exe) = std::env::current_exe()
            && let Some(deps) = exe.parent()
            && let Some(debug) = deps.parent()
        {
            let candidate = debug.join("scootnotify");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if let Ok(target) = std::env::var("CARGO_TARGET_DIR") {
            let candidate = std::path::PathBuf::from(target)
                .join("debug")
                .join("scootnotify");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            let candidate =
                std::path::PathBuf::from(manifest).join("../../target/debug/scootnotify");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}
