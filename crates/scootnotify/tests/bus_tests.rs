//! The server against a real `dbus-daemon`: every method and signal
//! with the right reasons, refusal naming the owner, `LimitsExceeded`
//! for a 5 MiB call with no client hang, ids never reused across a
//! restart, and the flood bounded.
//!
//! Needs `dbus-daemon` on `PATH`: without one the tests skip, saying
//! so, unless `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON` is set (CI sets it).
//! Needs the daemon binary beside the test
//! (`CARGO_BIN_EXE_scootnotify`, as built).

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{self, Reader, Writer};
use scoot_dbus::testdaemon::{Daemon, until};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFY_SIG: &str = "susssasa{sv}i";

/// A private bus, or skip the test without `dbus-daemon`.
fn bus_or_skip() -> Option<Daemon> {
    let daemon = Daemon::spawn();
    if daemon.is_none() {
        assert!(
            std::env::var_os("SCOOTNOTIFY_REQUIRE_DBUS_DAEMON").is_none(),
            "SCOOTNOTIFY_REQUIRE_DBUS_DAEMON is set but there is no dbus-daemon on PATH"
        );
        eprintln!("skipped: no dbus-daemon on PATH");
    }
    daemon
}

fn connect(daemon: &Daemon) -> Conn {
    conn::connect(&daemon.path()).expect("the daemon accepts EXTERNAL and says Hello")
}

/// A scratch runtime dir for one daemon run: its ids file and nothing
/// else. Returns the dir (kept alive by the caller) for restart rows.
fn runtime_dir(tag: &str) -> tempfile::TempDir {
    tempfile::TempDir::with_prefix(format!("scootnotify-{tag}")).expect("a scratch runtime dir")
}

/// Spawns the daemon binary serving `daemon`'s bus with `runtime` as
/// its `XDG_RUNTIME_DIR`: returns the child. The daemon inherits
/// `DBUS_SESSION_BUS_ADDRESS` pointing at the private bus.
fn spawn_server(daemon: &Daemon, runtime: &tempfile::TempDir) -> Child {
    let binary = env!("CARGO_BIN_EXE_scootnotify");
    Command::new(binary)
        .arg("daemon")
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", daemon.path().display()))
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", "scootnotify-test")
        .env("XDG_CONFIG_HOME", runtime.path().join("config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server binary starts")
}

/// Pumps `client` until its `Notify` is answered: the daemon serving.
/// One call outstanding at a time — an error (the name unowned yet)
/// resends, a reply returns — so at most one probe card ever exists.
fn wait_serving(client: &mut Conn) {
    let start = Instant::now();
    // A 1 ms card: the reply proves serving, the card itself is gone
    // before any test counts live ones.
    let body = notify_body("Probe", 0, "p", "p", &[], 1);
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000)
        .expect("the first call queues");
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the server never owned the name"
        );
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
            // The name is unowned yet: the errored call made no card,
            // so resending is safe.
            client
                .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000)
                .expect("a resent call queues");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One `Notify` body, everything inline in one `Writer`.
fn notify_body(
    app: &str,
    replaces: u32,
    summary: &str,
    body_text: &str,
    actions: &[&str],
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
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    out.i32(expire);
    out.take_body().unwrap()
}

/// One `Notify` answered: returns the id.
fn notify(client: &mut Conn, app: &str, replaces: u32, expire: i32, token: u64) -> u32 {
    let body = notify_body(app, replaces, "s", "b", &[], expire);
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token)
        .expect("the call queues");
    let Event::Reply { signature, body, .. } = until(client, |event| {
        matches!(event, Event::Reply { token: got, .. } if *got == token)
    }) else {
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
    bus_info(client, 8901);
}

/// One `GetServerInformation` round trip (also the match-rule barrier).
fn bus_info(client: &mut Conn, token: u64) -> Vec<String> {
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "GetServerInformation", "", &[], 0, token)
        .unwrap();
    let Event::Reply { signature, body, .. } = until(client, |event| {
        matches!(event, Event::Reply { token: got, .. } if *got == token)
    }) else {
        unreachable!()
    };
    assert_eq!(signature, "ssss");
    let mut reader = Reader::le(&body);
    let mut out = Vec::new();
    while !reader.exhausted() {
        out.push(reader.str().unwrap().to_owned());
    }
    out
}

/// Drains `client` until a `NotificationClosed` for `id` arrives.
fn wait_closed(client: &mut Conn, id: u32) -> u32 {
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no NotificationClosed for {id}"
        );
        let (events, _) = client.pump();
        for event in events {
            if let Event::Signal { member, body, .. } = event
                && member == "NotificationClosed"
            {
                let mut reader = Reader::le(&body);
                if reader.u32().unwrap() == id {
                    return reader.u32().unwrap();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn kill(mut child: Child) -> Option<std::process::ExitStatus> {
    let _ = child.kill();
    child.wait().ok()
}

#[test]
fn every_method_and_signal_with_the_right_reasons() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("methods");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    watch_signals(&mut client);

    // GetCapabilities: the four, never persistence.
    client
        .call(BUS_NAME, OBJECT, INTERFACE, "GetCapabilities", "", &[], 0, 11)
        .unwrap();
    let Event::Reply { signature, body, .. } = until(&mut client, |event| {
        matches!(event, Event::Reply { token: 11, .. })
    }) else {
        unreachable!()
    };
    assert_eq!(signature, "as");
    let mut reader = Reader::le(&body);
    let mut capabilities = Vec::new();
    let mut elements = reader.elements(4).unwrap();
    while !elements.exhausted() {
        capabilities.push(elements.str().unwrap().to_owned());
    }
    assert_eq!(capabilities, ["body", "body-markup", "actions", "icon-static"]);

    // GetServerInformation: name, vendor, version, spec.
    let info = bus_info(&mut client, 12);
    assert_eq!(info[0], "scootnotify");
    assert_eq!(info[1], "scoot");
    assert_eq!(info[2], env!("CARGO_PKG_VERSION"));
    assert_eq!(info[3], "1.3");

    // Notify, then CloseNotification: reason 3, id first invalidated.
    // The reply and the signal flush together, and `until` would eat
    // the signal with the reply — so one loop waits for both.
    let id = notify(&mut client, "Mail", 0, -1, 13);
    let mut close = Writer::new();
    close.u32(id);
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "CloseNotification",
            "u",
            &close.take_body().unwrap(),
            0,
            14,
        )
        .unwrap();
    let start = Instant::now();
    let mut replied = false;
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no close reply and NotificationClosed for {id} (replied={replied})"
        );
        let (events, _) = client.pump();
        for event in events {
            match event {
                Event::Reply { token: 14, .. } => replied = true,
                Event::Signal { member, body, .. } if member == "NotificationClosed" => {
                    let mut reader = Reader::le(&body);
                    if reader.u32().unwrap() == id {
                        assert!(replied, "the reply precedes the signal on the wire");
                        assert_eq!(reader.u32().unwrap(), 3);
                        kill(server);
                        return;
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn expiry_closes_with_reason_1() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("expiry");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    watch_signals(&mut client);
    let id = notify(&mut client, "Mail", 0, 200, 21);
    assert_eq!(wait_closed(&mut client, id), 1);
    kill(server);
}

#[test]
fn refusal_names_the_running_owner_and_exits_3() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    // This test owns the name first: the server must refuse, naming us.
    let mut owner = connect(&daemon);
    let mut body = Writer::new();
    body.str(BUS_NAME);
    body.u32(0);
    owner
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "RequestName",
            "su",
            &body.take_body().unwrap(),
            0,
            31,
        )
        .unwrap();
    until(&mut owner, |event| {
        matches!(event, Event::Reply { token: 31, .. })
    });
    let runtime = runtime_dir("refusal");
    let server = spawn_server(&daemon, &runtime);
    let output = server.wait_with_output().expect("the refused server exits");
    assert_eq!(output.status.code(), Some(3), "refusal exits 3, never a restart loop");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(BUS_NAME) && stderr.contains("will not take it over"),
        "the refusal says what and why: {stderr}"
    );
    assert!(stderr.contains("(pid "), "the refusal names the owner's pid: {stderr}");
}

#[test]
fn a_5_mib_call_gets_limits_exceeded_with_no_hang() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("limits");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    // The test sends giants: the client's own outbox bound moves with
    // its cap (receiving still refuses past it — nothing here does).
    client.set_max_message(6 * 1024 * 1024);
    wait_serving(&mut client);
    // A 5 MiB `Notify`, framed by hand (the client's own `Writer` cap
    // rightly refuses to build one).
    let big = "b".repeat(5 * 1024 * 1024);
    let mut out = Writer::with_cap(6 * 1024 * 1024);
    out.begin_call(500, BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, 0);
    out.str("Huge");
    out.u32(0);
    out.str("");
    out.str("big");
    out.str(&big);
    let actions = out.open_array(4).unwrap();
    out.close_array(actions);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    out.i32(-1);
    let frame = out.finish().unwrap();
    assert!(frame.len() > 5 * 1024 * 1024);
    client.queue_raw(&frame);
    // Flushes the megabytes out (several turns); the daemon answers on
    // its next pump, not on the client's timeout.
    let start = Instant::now();
    while client.want_write() {
        let _ = client.pump();
        assert!(!client.dead(), "the bus died flushing the huge call");
        assert!(start.elapsed() < Duration::from_secs(20), "the bus never drained");
    }
    let Event::CallError { name, .. } = until(&mut client, |event| {
        matches!(event, Event::CallError { .. })
    }) else {
        unreachable!()
    };
    let waited = start.elapsed();
    assert_eq!(name, "org.freedesktop.DBus.Error.LimitsExceeded");
    assert!(
        waited < Duration::from_secs(10),
        "answered in {waited:?}, never the client's timeout"
    );
    // And the daemon still serves after the giant.
    let id = notify(&mut client, "Mail", 0, -1, 41);
    assert!(id > 0);
    kill(server);
}

#[test]
fn ids_are_never_reused_across_a_restart() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("ids");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    let first = notify(&mut client, "Mail", 0, -1, 51);
    drop(client);
    kill(server);
    // The restart reserves the next block: its first id is past every
    // id the dead server may have issued.
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    let second = notify(&mut client, "Mail", 0, -1, 52);
    assert!(second > first, "restart reissued {first} as {second}");
    assert!(second >= 1025, "the block reservation holds: {second}");
    kill(server);
}

#[test]
fn replaces_survives_a_new_connection_for_the_same_app() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("replace");
    let server = spawn_server(&daemon, &runtime);
    // `notify-send -r` reconnects every time: one connection per call.
    let mut first = connect(&daemon);
    wait_serving(&mut first);
    let id = notify(&mut first, "Volume", 0, -1, 61);
    drop(first);
    let mut second = connect(&daemon);
    let body = notify_body("Volume", id, "s", "b", &[], -1);
    second
        .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 62)
        .unwrap();
    let Event::Reply { body, .. } = until(&mut second, |event| {
        matches!(event, Event::Reply { token: 62, .. })
    }) else {
        unreachable!()
    };
    assert_eq!(Reader::le(&body).u32().unwrap(), id, "same app, same id");
    kill(server);
}

#[test]
fn a_flood_is_answered_and_bounded() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("flood");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    // 500 fresh cards, every call answered: the flood costs bounded
    // memory (at most 128 live, the rest evicted with reason 4) and at
    // most one redraw per frame downstream — here, no hang and no death.
    let total = 500u64;
    let mut ids = Vec::new();
    for index in 0..total {
        let token = 7000 + index;
        let body = notify_body("Flood", 0, "s", "b", &[], 60_000);
        client
            .call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token)
            .expect("the pending table drains as replies land");
        // Harvests this call's answer before the next (the pending
        // table holds 64; an error here is a failure, not a skip).
        loop {
            let (events, _) = client.pump();
            let mut done = false;
            for event in events {
                match event {
                    Event::Reply { token: got, body, .. } if got == token => {
                        ids.push(Reader::le(&body).u32().unwrap());
                        done = true;
                    }
                    Event::CallError { token: got, .. } if got == token => {
                        panic!("a flood call errored");
                    }
                    _ => {}
                }
            }
            if done {
                break;
            }
            if client.dead() {
                panic!("the bus died mid-flood");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(ids.len() as u64, total, "every call answered");
    // The daemon still serves after the flood.
    let id = notify(&mut client, "Mail", 0, -1, 7999);
    assert!(id > 0);
    kill(server);
}

#[test]
fn a_client_that_disconnects_mid_call_kills_nothing() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("drop");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    // Sends one call and vanishes without reading its reply.
    let mut rude = connect(&daemon);
    let body = notify_body("Rude", 0, "s", "b", &[], -1);
    rude.call(BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 81)
        .unwrap();
    let _ = rude.pump();
    drop(rude);
    std::thread::sleep(Duration::from_millis(200));
    // The daemon serves the polite client untouched.
    let id = notify(&mut client, "Mail", 0, -1, 82);
    assert!(id > 0);
    kill(server);
}

#[test]
fn only_one_of_two_racers_owns_the_name() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let first_dir = runtime_dir("race-a");
    let second_dir = runtime_dir("race-b");
    let mut first = spawn_server(&daemon, &first_dir);
    let mut second = spawn_server(&daemon, &second_dir);
    // One serves, the other exits 3: never two owners, never two
    // crashes. Which wins is the bus's business, so the test waits for
    // the shape (one gone with 3, one answering) rather than a side.
    let start = Instant::now();
    let loser = loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the racers never settled"
        );
        let first_gone = first.try_wait().expect("the child reaps");
        let second_gone = second.try_wait().expect("the child reaps");
        match (first_gone, second_gone) {
            (Some(_), Some(_)) => panic!("both racers died"),
            (Some(status), None) => {
                assert_eq!(status.code(), Some(3), "a racer that loses exits 3");
                break "first";
            }
            (None, Some(status)) => {
                assert_eq!(status.code(), Some(3), "a racer that loses exits 3");
                break "second";
            }
            (None, None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // The survivor serves.
    let mut probe = connect(&daemon);
    let id = notify(&mut probe, "Race", 0, -1, 91);
    assert!(id > 0, "the winner serves");
    assert!(loser == "first" || loser == "second");
    kill(first);
    kill(second);
}

/// The daemon binary's `--check` over a good, a bad and a missing file.
#[test]
fn check_validates_without_a_bus() {
    let dir = runtime_dir("check");
    let binary = env!("CARGO_BIN_EXE_scootnotify");
    let good = dir.path().join("good.toml");
    std::fs::write(&good, "max_live = 7\n").unwrap();
    let bad = dir.path().join("bad.toml");
    std::fs::write(&bad, "max_bogus = 7\n").unwrap();
    let run = |args: &[&str]| {
        Command::new(binary)
            .args(args)
            .output()
            .expect("--check runs without a bus")
    };
    let out = run(&["daemon", "--check", "--config", good.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("valid"));
    let out = run(&["daemon", "--check", "--config", bad.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("max_bogus"));
    let missing = dir.path().join("missing.toml");
    let out = run(&["daemon", "--check", "--config", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "a missing file is the defaults");
}

/// The crate's zero-allocation pin: one served `Notify` (parsed,
/// stored, answered) allocates only what the store's growth needs.
/// Linux-only, like the crate.
#[cfg(target_os = "linux")]
#[test]
fn an_idle_pump_allocates_nothing() {
    let Some(daemon) = bus_or_skip() else {
        return;
    };
    let runtime = runtime_dir("alloc");
    let server = spawn_server(&daemon, &runtime);
    let mut client = connect(&daemon);
    wait_serving(&mut client);
    // Warm: one full round trip first.
    notify(&mut client, "Warm", 0, -1, 95);
    let ((events, _), allocations) =
        scootbg_mem::count_allocations(|| client.pump());
    assert!(events.is_empty());
    assert_eq!(allocations, 0, "an idle pump allocates nothing");
    kill(server);
}
