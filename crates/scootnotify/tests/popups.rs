//! The popup stack against real compositors: pixels per card state,
//! a click invoking with the token first, a locked arrival held until
//! unlock, hotplug moving the stack, no surface or buffers when empty,
//! and no wakeups with no cards.
//!
//! Needs `dbus-daemon` on `PATH` (a private bus per test) and a `scoot`
//! binary beside the test binary: without either the tests skip, saying
//! so, unless `SCOOTNOTIFY_REQUIRE_SCOOT` (or
//! `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON`) is set (CI sets them). The hotplug
//! row needs sway (`SCOOTNOTIFY_REQUIRE_SWAY` in CI).

#[path = "lockclient/mod.rs"]
mod lockclient;

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{self, Reader, Writer};
use scoot_dbus::testdaemon::{Daemon, until};
use scootui::test_support::{self, Session, rgb};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFY_SIG: &str = "susssasa{sv}i";

/// The require variable turning these tests' skips into failures.
const REQUIRE_SCOOT: &str = "SCOOTNOTIFY_REQUIRE_SCOOT";
const REQUIRE_SWAY: &str = "SCOOTNOTIFY_REQUIRE_SWAY";
const REQUIRE_BUS: &str = "SCOOTNOTIFY_REQUIRE_DBUS_DAEMON";

/// The headless compositor width in the harness (the session's own
/// default): the stack anchors top-right of this.
const WIDTH: u32 = 1600;

/// The default look's card fill and rings, for the pixel rows.
const SURFACE: &str = "#1e1e2e";
const FOCUS: &str = "#f9e2af";
const BORDER_LOW: &str = "#6c7086";
const URGENT: &str = "#f38ba8";
const FOREGROUND: &str = "#cdd6f4";

fn notify_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootnotify"))
}

/// A private bus, or skip the test without `dbus-daemon`.
fn bus_or_skip() -> Option<Daemon> {
    let daemon = Daemon::spawn();
    if daemon.is_none() {
        assert!(
            std::env::var_os(REQUIRE_BUS).is_none(),
            "{REQUIRE_BUS} is set but there is no dbus-daemon on PATH"
        );
        eprintln!("skipped: no dbus-daemon on PATH");
    }
    daemon
}

/// A headless scoot session, or skip without a `scoot` binary.
fn scoot_or_skip(tag: &str, outputs: u32) -> Option<Session> {
    let session = Session::scoot_requiring(&notify_bin(), tag, outputs, "", REQUIRE_SCOOT);
    if session.is_none() {
        eprintln!("skipped: no scoot binary (set {REQUIRE_SCOOT}=1 to fail)");
    }
    session
}

/// A sway session, or skip without sway.
fn sway_or_skip(tag: &str, outputs: u32) -> Option<Session> {
    let session = Session::sway(&notify_bin(), tag, outputs);
    if session.is_none() {
        assert!(
            std::env::var_os(REQUIRE_SWAY).is_none(),
            "{REQUIRE_SWAY} is set but there is no sway on PATH"
        );
        eprintln!("skipped: no sway on PATH");
    }
    session
}

/// Spawns the daemon binary on `session`'s compositor and `bus`'s bus,
/// with `config` as its `notify.toml` and the test font: returns the
/// child. The compositor choice is the session's (null output); the
/// look layer is absent (scratch state home), so every pixel row reads
/// the built-in defaults.
fn spawn_popup(session: &Session, bus: &Daemon, config: &str) -> Child {
    let binary = notify_bin();
    let config_path = session.scratch.0.join("config").join("scoot").join("notify.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, config).unwrap();
    let font = session.scratch.font();
    let mut command = Command::new(binary);
    command
        .arg("daemon")
        .arg("--config")
        .arg(&config_path)
        .arg("--font")
        .arg(&font)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", bus.path().display()),
        )
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .env("XDG_CONFIG_HOME", session.scratch.config_home())
        .env("XDG_STATE_HOME", session.scratch.state_home())
        .env("XDG_CACHE_HOME", session.scratch.cache_home())
        .env("HOME", &session.scratch.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().expect("the popup binary starts")
}

/// Pumps `client` until its `Notify` is answered: the daemon serving.
/// (A 1 ms card: the reply proves serving, the card itself is gone
/// before any test counts live ones.)
fn wait_serving(client: &mut Conn, server: &mut Child) {
    let start = Instant::now();
    let body = notify_body("Probe", 0, "p", "p", &[], &[], 1);
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000)
        .expect("the first call queues");
    loop {
        if start.elapsed() >= Duration::from_secs(30) {
            match server.try_wait().expect("the server reaps") {
                Some(status) => {
                    let mut err = String::new();
                    if let Some(mut stderr) = server.stderr.take() {
                        let _ = stderr.read_to_string(&mut err);
                    }
                    panic!("the server died at startup ({status}): {err}");
                }
                None => {
                    panic!("the server still runs but never owned the name in 30 s");
                }
            }
        }
        if client.dead() {
            panic!("the bus died while waiting for the server");
        }
        let (events, _) = client.pump();
        let mut answered = false;
        let mut errored = false;
        for event in events {
            match event {
                Event::Reply { token: 9000, .. } => answered = true,
                Event::CallError { token: 9000, .. } => errored = true,
                _ => {}
            }
        }
        if answered {
            return;
        }
        if errored {
            client
                .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000)
                .expect("a resent call queues");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One `Notify` body: `actions` as key/label pairs, `bytes` as
/// (`name`, byte value) hints (`urgency`, `value`).
fn notify_body(
    app: &str,
    replaces: u32,
    summary: &str,
    body_text: &str,
    actions: &[&str],
    bytes: &[(&str, u8)],
    expire: i32,
) -> Vec<u8> {
    let mut out = Writer::new();
    out.str(app);
    out.u32(replaces);
    out.str("");
    out.str(summary);
    out.str(body_text);
    let cookie = out.open_array(4).unwrap();
    for action in actions {
        out.str(action);
    }
    out.close_array(cookie);
    let hints_cookie = out.open_array(8).unwrap();
    for (name, value) in bytes {
        assert!(out.open_struct());
        out.str(name);
        out.variant("y");
        out.u8(*value);
        out.close_struct();
    }
    out.close_array(hints_cookie);
    out.i32(expire);
    out.take_body().unwrap()
}

/// One `Notify` answered: returns the id.
fn notify(
    client: &mut Conn,
    app: &str,
    replaces: u32,
    summary: &str,
    body: &str,
    actions: &[&str],
    bytes: &[(&str, u8)],
    expire: i32,
    token: u64,
) -> u32 {
    let body = notify_body(app, replaces, summary, body, actions, bytes, expire);
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token)
        .expect("the call queues");
    let Event::Reply {
        signature, body, ..
    } = until(client, |event| matches!(event, Event::Reply { token: got, .. } if *got == token))
    else {
        unreachable!()
    };
    assert_eq!(signature, "u");
    Reader::le(&body).u32().unwrap()
}

/// Subscribes `client` to the server's signals, barriered so the rule
/// is installed before the test triggers anything.
fn watch_signals(client: &mut Conn) {
    let mut rule = Writer::new();
    rule.str("type='signal',interface='org.freedesktop.Notifications'");
    client
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "AddMatch",
            "s",
            &rule.take_body().unwrap(),
            proto::flag::NO_REPLY_EXPECTED,
            0,
        )
        .unwrap();
    // The barrier: the bus installs one connection's calls in order, so
    // the reply below proves the rule is live.
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "GetServerInformation", "", &[], 0, 8901)
        .unwrap();
    let _ = until(client, |event| matches!(event, Event::Reply { token: 8901, .. }));
}

/// The stack's top-right corner: the surface's left edge and the first
/// card's top edge (360 px wide, 12 px margin).
fn stack_origin() -> (u32, u32) {
    (WIDTH - 360 - 12, 12)
}

/// Waits until `check` sees the stack (or the timeout names what never
/// showed, with the daemon's stderr).
fn wait_shown(server: &mut Child, what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    loop {
        if check() {
            return;
        }
        if let Some(status) = server.try_wait().unwrap() {
            let mut err = String::new();
            if let Some(mut stderr) = server.stderr.take() {
                let _ = stderr.read_to_string(&mut err);
            }
            panic!("{what}: the daemon died ({status}): {err}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "never {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The card's top edge reads the urgency ring: low the `border` role,
/// normal `focus`, critical `urgent`.
#[test]
fn pixels_per_card_state() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("pixels", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    let (x0, y0) = stack_origin();
    let shot = || session.scoot_screenshot(1);

    // Normal: the `focus` ring, the surface fill, foreground ink.
    let _ = notify(&mut client, "a", 0, "Hello", "World", &[], &[], 0, 101);
    wait_shown(&mut server, "the normal card", || {
        shot().at(x0 + 180, y0 + 1) == rgb(FOCUS)
    });
    let shown = shot();
    assert_eq!(shown.at(x0 + 180, y0 + 1), rgb(FOCUS));
    assert_eq!(shown.at(x0 + 5, y0 + 30), rgb(SURFACE));
    let ink = (0..360u32).any(|dx| shown.at(x0 + dx, y0 + 20) == rgb(FOREGROUND));
    assert!(ink, "the summary draws foreground ink");

    // Critical: the `urgent` ring over the same fill.
    let _ = notify(&mut client, "b", 0, "Fire", "Alarm", &[], &[("urgency", 2)], 0, 102);
    wait_shown(&mut server, "the critical card", || {
        shot().at(x0 + 180, y0 + 1) == rgb(URGENT)
    });

    // Actions: the button outline draws `focus`.
    let _ = notify(
        &mut client,
        "c",
        0,
        "Pick",
        "Choose",
        &["open", "Open", "shut", "Shut"],
        &[],
        0,
        103,
    );
    wait_shown(&mut server, "the action buttons", || {
        shot().at(x0 + 12, y0 + 120) == rgb(FOCUS)
            || shot().at(x0 + 100, y0 + 140) == rgb(FOCUS)
    });

    // Progress: the `value` fill draws `selection` (accent by default).
    let _ = notify(
        &mut client,
        "d",
        0,
        "Copy",
        "Halfway",
        &[],
        &[("value", 50)],
        0,
        104,
    );
    wait_shown(&mut server, "the progress bar", || {
        let pixels = shot();
        (0..360u32).any(|dx| pixels.at(x0 + dx, y0 + 200) == rgb(FOCUS))
    });

    // Low: the `border` ring over the same fill.
    let _ = notify(&mut client, "e", 0, "Quiet", "Low", &[], &[("urgency", 0)], 0, 100);
    wait_shown(&mut server, "the low card", || {
        shot().at(x0 + 180, y0 + 1) == rgb(BORDER_LOW)
    });
    kill(server);
}

/// A body click invokes `default` with the activation token first: the
/// token signal precedes `ActionInvoked` for the same id, and the close
/// (reason 2) follows.
#[test]
fn a_click_invokes_with_the_token_first() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("click", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    watch_signals(&mut client);
    let (x0, y0) = stack_origin();
    let id = notify(
        &mut client,
        "a",
        0,
        "Hello",
        "World",
        &["default", "Default"],
        &[],
        0,
        111,
    );
    wait_shown(&mut server, "the card", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 1) == rgb(FOCUS)
    });
    // Into the body (below the summary, above the buttons there are
    // none): the click lands on the card, not a gap.
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{},"y":{},"button":"left"}}"#,
        x0 + 180,
        y0 + 50
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    // The signals in order: token, invoke, close. The token must come
    // first — a token after the invoke is the bug this pins.
    let start = Instant::now();
    let mut order: Vec<String> = Vec::new();
    let mut token = String::new();
    let mut invoked = String::new();
    let mut reason = 0;
    while order.len() < 3 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no invoke for {id}: {order:?}"
        );
        let (events, _) = client.pump();
        for event in events {
            if let Event::Signal { member, body, .. } = event {
                let mut reader = Reader::le(&body);
                match member.as_str() {
                    "ActivationToken" if reader.u32().unwrap() == id => {
                        token = reader.str().unwrap().to_owned();
                        order.push("token".to_owned());
                    }
                    "ActionInvoked" if reader.u32().unwrap() == id => {
                        invoked = reader.str().unwrap().to_owned();
                        order.push("invoke".to_owned());
                    }
                    "NotificationClosed" if reader.u32().unwrap() == id => {
                        reason = reader.u32().unwrap();
                        order.push("closed".to_owned());
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(order, ["token", "invoke", "closed"], "{order:?}");
    assert!(!token.is_empty(), "the token is minted");
    assert_eq!(invoked, "default");
    assert_eq!(reason, 2);
    kill(server);
}

/// A card arriving while locked is not drawn, and its clock does not
/// start until after unlock: no pixels over the lock, no expiry through
/// it, then a normal show-and-expire once unlocked.
#[test]
fn a_locked_arrival_waits_for_unlock() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("lock", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    watch_signals(&mut client);
    let locker = lockclient::Locker::start(&session.wayland_socket());
    let start = Instant::now();
    while !locker.is_locked() {
        assert!(start.elapsed() < Duration::from_secs(10), "never locked");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The lock covers the corner the stack would take.
    let (x0, y0) = stack_origin();
    let covered = |session: &Session| {
        let shot = session.scoot_screenshot(1);
        [(x0 + 180, y0 + 1), (x0 + 5, y0 + 30), (x0 + 300, y0 + 60)]
            .iter()
            .all(|&(x, y)| shot.at(x, y) == rgb(lockclient::COLOR))
    };
    let start = Instant::now();
    while !covered(&session) {
        assert!(start.elapsed() < Duration::from_secs(10), "never covered");
        std::thread::sleep(Duration::from_millis(50));
    }
    // A 2 s card arrives under the lock: nothing drawn, and no expiry
    // through a full timeout and more (its clock never started).
    let id = notify(&mut client, "a", 0, "Hello", "World", &[], &[], 2000, 121);
    std::thread::sleep(Duration::from_millis(500));
    assert!(covered(&session), "a card drawn over the lock");
    std::thread::sleep(Duration::from_millis(2500));
    assert!(covered(&session), "a card drawn over the lock after its timeout");
    let (events, _) = client.pump();
    for event in events {
        if let Event::Signal { member, body, .. } = event {
            assert!(
                !(member == "NotificationClosed" && Reader::le(&body).u32().unwrap() == id),
                "a locked card expired unseen"
            );
        }
    }
    // Unlocked: the card shows, then expires normally (reason 1).
    drop(locker);
    wait_shown(&mut server, "the card after unlock", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 1) == rgb(FOCUS)
    });
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(10), "no expiry after unlock");
        let (events, _) = client.pump();
        let mut reason = None;
        for event in events {
            if let Event::Signal { member, body, .. } = event
                && member == "NotificationClosed"
            {
                let mut reader = Reader::le(&body);
                if reader.u32().unwrap() == id {
                    reason = Some(reader.u32().unwrap());
                }
            }
        }
        if let Some(reason) = reason {
            assert_eq!(reason, 1);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    kill(server);
}

/// Unplugging an output moves the stack: the card survives on the
/// output that is left, and the daemon with it.
#[test]
fn unplugging_an_output_moves_the_stack() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = sway_or_skip("hotplug", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    let shown_on = |session: &Session, name: &str| {
        let shot = session.screencopy(name);
        // Any card-colored pixel in the top-right: the ring or the fill.
        let (w, h) = (shot.width, shot.height);
        let mut found = false;
        let mut y = 12;
        while y < h.min(400) {
            let mut x = w.saturating_sub(372);
            while x < w {
                let at = shot.at(x, y);
                if at == rgb(FOCUS) || at == rgb(SURFACE) {
                    found = true;
                    break;
                }
                x += 4;
            }
            if found {
                break;
            }
            y += 4;
        }
        found.then_some(())
    };
    let _ = notify(&mut client, "a", 0, "Hello", "World", &[], &[], 0, 131);
    let names: Vec<String> = session
        .swaymsg(&["-t", "get_outputs", "-r"])
        .pipe_outputs();
    assert!(!names.is_empty());
    wait_shown(&mut server, "the card on sway", || {
        names.iter().any(|name| shown_on(&session, name).is_some())
    });
    // A second output, then unplug the first: the stack moves, nothing
    // is lost, and the daemon lives.
    session.swaymsg(&["create_output"]);
    let start = Instant::now();
    let pair = loop {
        let names: Vec<String> = session.swaymsg(&["-t", "get_outputs", "-r"]).pipe_outputs();
        if names.len() >= 2 {
            break names;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no second output");
        std::thread::sleep(Duration::from_millis(50));
    };
    session.swaymsg(&["output", &pair[0], "unplug"]);
    let left = pair[1].clone();
    wait_shown(&mut server, "the card on the output that is left", || {
        shown_on(&session, &left).is_some()
    });
    assert!(server.try_wait().unwrap().is_none(), "the daemon died on hotplug");
    kill(server);
}

/// The surface and buffers are gone when empty: the memfd mappings and
/// the fds the card cost return to the idle count.
#[test]
fn empty_holds_no_surface_or_buffers() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("empty", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    let pid = server.id();
    std::thread::sleep(Duration::from_secs(2));
    let idle_fds = test_support::open_fds(pid);
    assert_eq!(
        test_support::shm_mappings_named(pid, "scootnotify"),
        0,
        "an idle daemon maps no buffers"
    );
    let (x0, y0) = stack_origin();
    let id = notify(&mut client, "a", 0, "Hello", "World", &[], &[], 0, 141);
    wait_shown(&mut server, "the card", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 1) == rgb(FOCUS)
    });
    assert!(
        test_support::shm_mappings_named(pid, "scootnotify") >= 1,
        "a shown card maps its buffers"
    );
    // Dismiss it through the bus (a right click's path): the stack
    // empties, and everything unmaps.
    let mut body = Writer::new();
    body.u32(id);
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "CloseNotification",
            "u",
            &body.take_body().unwrap(),
            0,
            142,
        )
        .unwrap();
    wait_shown(&mut server, "the stack gone", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 30) != rgb(SURFACE)
    });
    let start = Instant::now();
    while test_support::shm_mappings_named(pid, "scootnotify") != 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "buffers never unmapped");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        test_support::open_fds(pid),
        idle_fds,
        "the card's fds return to the idle count"
    );
    kill(server);
}

/// No cards, no wakeups: the timerfd disarms and the loop sleeps on
/// its fds alone.
#[test]
fn no_cards_means_no_wakeups() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("idle", 1) else { return };
    let mut server = spawn_popup(&session, &bus, "");
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    wait_serving(&mut client, &mut server);
    // Past the probe card's 1 ms life, and settled.
    std::thread::sleep(Duration::from_secs(2));
    let pid = server.id();
    let before = test_support::wakeups(pid);
    std::thread::sleep(Duration::from_secs(10));
    assert_eq!(
        test_support::wakeups(pid),
        before,
        "an idle daemon wakes for nothing"
    );
    kill(server);
}

/// Sway's output names from `get_outputs -r`.
trait PipeOutputs {
    fn pipe_outputs(&self) -> Vec<String>;
}

impl PipeOutputs for String {
    fn pipe_outputs(&self) -> Vec<String> {
        let outputs: serde_json::Value = serde_json::from_str(self).expect("outputs JSON");
        outputs
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|output| output["name"].as_str().map(str::to_owned))
            .collect()
    }
}
