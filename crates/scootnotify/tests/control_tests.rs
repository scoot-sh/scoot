//! The control socket end to end: every `msg` verb against the real
//! daemon binary on a private bus, headless (no compositor, so arrival is
//! display and no surface exists).
//!
//! Needs `dbus-daemon` on `PATH`: without one the tests skip, saying
//! so, unless `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON` is set (CI sets it).
//! Needs the daemon binary beside the test
//! (`CARGO_BIN_EXE_scootnotify`, as built).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{self, Reader, Writer};
use scoot_dbus::testdaemon::{Daemon, until};
use scootnotify::control::client;
use scootnotify::control::paths::Paths;
use scootnotify::control::protocol::Request;

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFY_SIG: &str = "susssasa{sv}i";
const DISPLAY: &str = "control-test";

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

/// A scratch tree for one daemon run: its runtime dir, config home,
/// state home and home. Nothing real is read or written.
fn scratch(tag: &str) -> tempfile::TempDir {
    let dir = tempfile::TempDir::with_prefix(format!("snc-{tag}")).expect("a scratch dir");
    // `sun_path` holds 108 bytes on Linux, one the NUL: the control
    // socket must fit 107 (see `scootui/src/control/paths.rs`). Fail
    // fast with the tag named instead of burning 30 s per test in
    // `wait_serving`.
    debug_assert!(
        dir.path()
            .join(format!("scootnotify-{DISPLAY}.sock"))
            .as_os_str()
            .len()
            <= 107,
        "control socket path too long for tag {tag}: {}",
        dir.path().display()
    );
    dir
}

/// Spawns the daemon binary serving `daemon`'s bus, headless (no
/// compositor at `WAYLAND_DISPLAY`, so arrival is display). Returns the
/// child. The daemon inherits `DBUS_SESSION_BUS_ADDRESS` pointing at the
/// private bus, and every home it reads under `runtime`.
fn spawn_server(daemon: &Daemon, runtime: &tempfile::TempDir) -> Child {
    let binary = env!("CARGO_BIN_EXE_scootnotify");
    Command::new(binary)
        .arg("daemon")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", daemon.path().display()),
        )
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", DISPLAY)
        .env("XDG_CONFIG_HOME", runtime.path().join("config"))
        .env("XDG_STATE_HOME", runtime.path().join("state"))
        .env("HOME", runtime.path().join("home"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server binary starts")
}

/// The control socket's paths for a spawned server.
fn control_paths(runtime: &tempfile::TempDir) -> Paths {
    Paths {
        app: String::from("scootnotify"),
        display: DISPLAY.to_owned(),
        socket: runtime.path().join(format!("scootnotify-{DISPLAY}.sock")),
        lock: runtime.path().join(format!("scootnotify-{DISPLAY}.lock")),
    }
}

/// Pumps `client` until its `Notify` is answered: the daemon serving.
/// (As `bus_tests.rs`: one call outstanding at a time, resending on the
/// name-unowned error, so at most one probe card ever exists.) On
/// timeout the panic names the server's fate (its exit code and stderr
/// when it died, which is the startup crash; what its stderr says so
/// far when it still runs, which is how far startup got), because a
/// bare deadline cost a debugging round already.
fn wait_serving(client: &mut Conn, server: &mut Child) {
    let body = notify_body("Probe", 0, "p", "p", &[], 1);
    client
        .call(
            BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000,
        )
        .expect("the first call queues");
    let start = Instant::now();
    // How the wait ends, for the timeout panic: errors mean the name
    // was genuinely unowned (the daemon never got there); silence with
    // no errors at all means the bus never answered either way.
    let mut errored_total = 0u64;
    let mut events_total = 0u64;
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
                    panic!(
                        "the server still runs but never owned the name in 30 s \
                         ({events_total} events, {errored_total} name-unowned errors); \
                         its stderr so far: {}",
                        read_available(server.stderr.as_mut())
                    );
                }
            }
        }
        // A dead-at-startup server fails here in milliseconds, not at
        // the 30 s deadline: otherwise a startup crash reads exactly
        // like a serving stall.
        if let Some(status) = server.try_wait().expect("the server reaps") {
            let mut err = String::new();
            if let Some(mut stderr) = server.stderr.take() {
                let _ = stderr.read_to_string(&mut err);
            }
            panic!("the server died at startup ({status}): {err}");
        }
        if client.dead() {
            panic!("the bus died while waiting for the server");
        }
        let (events, _) = client.pump();
        let mut answered = false;
        let mut errored = false;
        for event in events {
            events_total += 1;
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
            errored_total += 1;
            client
                .call(
                    BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 9000,
                )
                .expect("the retry queues");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

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

/// One `Notify` through `client`, returning its id. Times out loudly
/// instead of hanging the suite.
fn notify(client: &mut Conn, app: &str, summary: &str, actions: &[&str]) -> u32 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100);
    let token = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let body = notify_body(app, 0, summary, "body", actions, -1);
    client
        .call(
            BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token,
        )
        .expect("the call queues");
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(10), "no Notify reply");
        let (events, _) = client.pump();
        for event in events {
            if let Event::Reply {
                token: got, body, ..
            } = event
                && got == token
            {
                return Reader::le(&body).u32().unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Runs `scootnotify msg …` against a spawned server, with its
/// environment. Returns the output.
fn msg(runtime: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_scootnotify");
    Command::new(binary)
        .arg("msg")
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", DISPLAY)
        .env("HOME", runtime.path().join("home"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("the msg binary runs")
}

/// The parsed `list` reply: the live cards.
fn list_cards(runtime: &tempfile::TempDir) -> Vec<serde_json::Value> {
    let reply = client::send_to(&control_paths(runtime), &Request::List).expect("list answers");
    let parsed: serde_json::Value = serde_json::from_str(&reply).expect("the reply is JSON");
    assert_eq!(parsed["type"], "notifications");
    parsed["notifications"]
        .as_array()
        .expect("an array")
        .clone()
}

/// Subscribes `client` to the server's signals, barriered so the rule
/// is installed before the test triggers anything. (As `bus_tests.rs`:
/// without a match rule the bus delivers no signals.)
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
fn bus_info(client: &mut Conn, token: u64) {
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "GetServerInformation",
            "",
            &[],
            0,
            token,
        )
        .unwrap();
    let Event::Reply { .. } = until(
        client,
        |event| matches!(event, Event::Reply { token: got, .. } if *got == token),
    ) else {
        unreachable!()
    };
}

/// Drains `client` until an `ActionInvoked` for `id` arrives: its action
/// key. Times out loudly instead of hanging the suite.
fn wait_invoked(client: &mut Conn, id: u32) -> String {
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no ActionInvoked for {id}"
        );
        let (events, _) = client.pump();
        for event in events {
            if let Event::Signal { member, body, .. } = event
                && member == "ActionInvoked"
            {
                let mut reader = Reader::le(&body);
                if reader.u32().unwrap() == id {
                    return reader.str().unwrap().to_owned();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reads what the daemon's stderr holds without blocking: the daemon
/// is still alive, so EOF never comes. Best effort (a failure to drain
/// says so); the pipe is left nonblocking, which is fine on a path that
/// panics right after.
fn read_available(stderr: Option<&mut std::process::ChildStderr>) -> String {
    use std::os::fd::AsFd as _;
    let Some(stderr) = stderr else {
        return String::from("(stderr already taken)");
    };
    let fd = stderr.as_fd();
    let flags = match rustix::fs::fcntl_getfl(fd) {
        Ok(flags) => flags,
        Err(_) => return String::from("(stderr would not drain)"),
    };
    if rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).is_err() {
        return String::from("(stderr would not drain)");
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match rustix::io::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Drains `client` until a `NotificationClosed` for `id` arrives: its
/// reason code.
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

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// One control connection with a read timeout, for the streaming rows.
fn control_stream(runtime: &tempfile::TempDir) -> UnixStream {
    let paths = control_paths(runtime);
    let start = Instant::now();
    loop {
        match UnixStream::connect(&paths.socket) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                return stream;
            }
            Err(_) if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("nothing listens on {}: {error}", paths.socket.display()),
        }
    }
}

/// Reads one line from `reader`: the test fails loudly on timeout or EOF
/// instead of hanging the suite.
fn read_line(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    assert!(
        reader.read_line(&mut line).expect("the daemon answers") > 0,
        "the daemon closed the connection"
    );
    line
}

#[test]
fn version_and_list_answer() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("verbs");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let out = msg(&runtime, &["version"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"protocol\":2"), "{stdout}");

    let out = msg(&runtime, &["list", "--json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"notifications\":[]"), "{stdout}");

    let out = msg(&runtime, &["list"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8(out.stdout).unwrap().is_empty());

    kill(server);
}

#[test]
fn dnd_holds_history_keeps_and_restore_re_shows_end_to_end() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("dnd-hist");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    // DND on: a normal card holds (a new id, nothing live), a critical
    // still shows.
    let out = msg(&runtime, &["dnd", "on"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8(out.stdout).unwrap().contains("\"dnd\":\"on\""));
    let held = notify(&mut client, "Mail", "New mail", &[]);
    assert!(list_cards(&runtime).is_empty());
    let critical = notify(&mut client, "Alert", "Fire", &[]);
    assert_eq!(list_cards(&runtime).len(), 1);
    let _ = critical;

    // History keeps the held card, unseen; the shown one is not there yet.
    let out = msg(&runtime, &["history", "--json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(parsed["type"], "history");
    let entries = parsed["history"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["app_name"], "Mail");
    assert_eq!(entries[0]["seen"], false);
    let _ = held;

    // Restore re-shows the held card as a new live card.
    let out = msg(&runtime, &["restore"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(list_cards(&runtime).len(), 2);

    // DND off answers the mode; held cards stay held (nothing shows
    // retroactively), and an empty restore refuses loudly.
    let out = msg(&runtime, &["dnd", "off"]);
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("\"dnd\":\"off\""));
    let out = msg(&runtime, &["dnd"]);
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("\"dnd\":\"off\""));

    kill(server);
}

#[test]
fn a_rule_exec_fires_end_to_end_and_history_false_skips() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("rule-exec");
    // The daemon reads its own file at start: one rule hides its match
    // and rings a hook, another keeps its match out of history.
    let config_dir = runtime.path().join("config").join("scoot");
    std::fs::create_dir_all(&config_dir).expect("a config dir");
    let fired = runtime.path().join("hook-fired");
    std::fs::write(
        config_dir.join("notify.toml"),
        format!(
            "[[rule]]\napp_name = \"Hook\"\nhide = true\nexec = [\"sh\", \"-c\", \"touch {}\"]\n\
             [[rule]]\napp_name = \"Quiet\"\nhistory = false\n",
            fired.display()
        ),
    )
    .expect("a config file");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    notify(&mut client, "Hook", "ring", &[]);
    // Hidden: nothing live.
    assert!(list_cards(&runtime).is_empty());
    // The hook ran: the file appears (the daemon spawns and reaps it).
    let start = Instant::now();
    while fired.metadata().is_err() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(fired.metadata().is_ok(), "the rule exec ran");
    // But the hidden card is in history (hide is not history=false).
    let out = msg(&runtime, &["history", "--json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(parsed["history"].as_array().unwrap().len(), 1);

    // `history = false` keeps the match out of the ring entirely.
    notify(&mut client, "Quiet", "shh", &[]);
    let out = msg(&runtime, &["history", "--json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(parsed["history"].as_array().unwrap().len(), 1);

    kill(server);
}

#[test]
fn dismiss_and_invoke_end_to_end() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("dismiss-invoke");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);
    watch_signals(&mut client);

    let id = notify(
        &mut client,
        "Firefox",
        "Download complete",
        &["default", "Open"],
    );
    assert_eq!(list_cards(&runtime).len(), 1);

    let out = msg(&runtime, &["list"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Firefox") && stdout.contains("Download complete"),
        "{stdout}"
    );

    let out = msg(&runtime, &["invoke", &id.to_string()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(wait_closed(&mut client, id), 2);
    assert!(list_cards(&runtime).is_empty());

    let again = notify(&mut client, "Firefox", "Up to date", &["default", "Open"]);
    let out = msg(&runtime, &["dismiss", &again.to_string()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(wait_closed(&mut client, again), 2);
    assert!(list_cards(&runtime).is_empty());

    let out = msg(&runtime, &["dismiss", "999999"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("999999"));

    let third = notify(&mut client, "Firefox", "Third", &["default", "Open"]);
    let out = msg(&runtime, &["invoke", &third.to_string(), "snooze"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("snooze"));
    let out = msg(&runtime, &["dismiss", "--all"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(list_cards(&runtime).is_empty());

    kill(server);
}

#[test]
fn invoke_emits_action_invoked_on_the_bus() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("invoked-signal");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);
    watch_signals(&mut client);

    let id = notify(
        &mut client,
        "Firefox",
        "Done",
        &["default", "Open", "view", "View"],
    );
    let out = msg(&runtime, &["invoke", &id.to_string(), "view"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(wait_invoked(&mut client, id), "view");

    kill(server);
}

#[test]
fn subscribe_streams_notify_and_close() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("subscribe");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let stream = control_stream(&runtime);
    let mut reader = BufReader::new(stream);
    let subscribe = Request::Subscribe {
        events: vec![
            scootnotify::control::protocol::EventKind::Notify,
            scootnotify::control::protocol::EventKind::Close,
        ],
    };
    reader
        .get_mut()
        .write_all(subscribe.line().as_bytes())
        .unwrap();
    let reply = read_line(&mut reader);
    assert!(reply.contains("\"subscribed\""), "{reply}");

    let id = notify(
        &mut client,
        "Firefox",
        "Download complete",
        &["default", "Open"],
    );
    let event = loop {
        // The serving probe's expiry close (its 1 ms card) may overtake
        // our notify under load: anything but our card is skipped, never
        // asserted on.
        let event = read_line(&mut reader);
        let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
        if parsed.get("type") == Some(&serde_json::Value::from("notify"))
            && parsed["notification"]["summary"] == "Download complete"
        {
            break event;
        }
    };
    assert!(
        event.contains("\"notify\"") && event.contains("Download complete"),
        "{event}"
    );

    let out = msg(&runtime, &["dismiss", &id.to_string()]);
    assert!(out.status.success());
    // Likewise, the close for our card.
    let parsed = loop {
        let event = read_line(&mut reader);
        let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
        if parsed.get("type") == Some(&serde_json::Value::from("close")) && parsed["id"] == id {
            break parsed;
        }
    };
    assert_eq!(parsed["type"], "close");
    assert_eq!(parsed["reason"], "dismissed");

    kill(server);
}

#[test]
fn wait_matches_a_live_card_an_arrival_and_times_out() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("wait");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    // The baseline: already live before the wait starts.
    notify(&mut client, "Firefox", "Already here", &["default", "Open"]);
    let out = msg(
        &runtime,
        &["wait", "--summary-contains", "Already", "--timeout", "5"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Already here"), "{stdout}");

    // An arrival while waiting.
    let waiting = Command::new(env!("CARGO_BIN_EXE_scootnotify"))
        .arg("msg")
        .args(["wait", "--app", "Thunderbird", "--timeout", "10"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", DISPLAY)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("msg wait starts");
    std::thread::sleep(Duration::from_millis(500));
    notify(&mut client, "Thunderbird", "New mail", &["default", "Open"]);
    let out = waiting.wait_with_output().expect("msg wait exits");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stdout).unwrap().contains("New mail"),
        "the match prints"
    );

    // Nothing arrives: the timeout, fast.
    let start = Instant::now();
    let out = msg(
        &runtime,
        &["wait", "--summary-contains", "Never", "--timeout", "1"],
    );
    assert!(!out.status.success());
    assert!(start.elapsed() < Duration::from_secs(10));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("timed out"), "stderr: {stderr}");

    kill(server);
}

#[test]
fn a_reload_with_a_bad_file_keeps_the_running_config_and_state() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("reload-bad");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let config = runtime
        .path()
        .join("config")
        .join("scoot")
        .join("notify.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "max_live = 64\n").unwrap();
    let out = msg(&runtime, &["reload"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let id = notify(&mut client, "Firefox", "Held", &["default", "Open"]);
    std::fs::write(&config, "max_live = \n").unwrap();
    let out = msg(&runtime, &["reload"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("daemon:"),
        "the refusal names itself"
    );
    // The running daemon stands: the card is still listed.
    let cards = list_cards(&runtime);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["id"], id);

    kill(server);
}

#[test]
fn a_scoped_reload_ignores_a_half_edited_user_file() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("reload-scoped");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    // A look layer with one role set.
    let fragment = runtime
        .path()
        .join("state")
        .join("scoot")
        .join("look")
        .join("notify.toml");
    std::fs::create_dir_all(fragment.parent().unwrap()).unwrap();
    std::fs::write(&fragment, "radius = 4\n").unwrap();
    // The user file is half-edited: the scoped reload never reads it.
    let config = runtime
        .path()
        .join("config")
        .join("scoot")
        .join("notify.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "max_live = \n").unwrap();
    let out = msg(&runtime, &["reload", "--look"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("\"scope\":\"look\"")
    );

    // A half-written fragment refuses instead.
    std::fs::write(&fragment, "radius = \n").unwrap();
    let out = msg(&runtime, &["reload", "--look"]);
    assert!(!out.status.success());

    kill(server);
}

#[test]
fn the_subscriber_cap_holds() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("sub-cap");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let mut held = Vec::new();
    for _ in 0..4 {
        let stream = control_stream(&runtime);
        let mut reader = BufReader::new(stream);
        let subscribe = Request::Subscribe {
            events: scootnotify::control::protocol::EventKind::ALL.to_vec(),
        };
        reader
            .get_mut()
            .write_all(subscribe.line().as_bytes())
            .unwrap();
        let reply = read_line(&mut reader);
        assert!(reply.contains("\"subscribed\""), "{reply}");
        held.push(reader);
    }
    // The fifth is refused with the cap named.
    let stream = control_stream(&runtime);
    let mut reader = BufReader::new(stream);
    let subscribe = Request::Subscribe {
        events: scootnotify::control::protocol::EventKind::ALL.to_vec(),
    };
    reader
        .get_mut()
        .write_all(subscribe.line().as_bytes())
        .unwrap();
    let reply = read_line(&mut reader);
    assert!(reply.contains("\"type\":\"error\""), "{reply}");

    kill(server);
}

#[test]
fn the_connection_cap_evicts_the_oldest_non_subscriber() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("conn-cap");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let mut held = Vec::new();
    for _ in 0..16 {
        held.push(control_stream(&runtime));
    }
    // One more: the daemon admits it by closing the oldest.
    let _extra = control_stream(&runtime);
    let start = Instant::now();
    let mut byte = [0u8; 1];
    held[0]
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    loop {
        match held[0].read(&mut byte) {
            // EOF: the daemon closed the oldest connection.
            Ok(0) => break,
            Ok(_) => panic!("the evicted connection should read EOF, not bytes"),
            Err(error) if start.elapsed() < Duration::from_secs(10) => {
                assert!(
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ),
                    "unexpected: {error}"
                );
            }
            Err(error) => panic!("the oldest connection never closed: {error}"),
        }
    }

    kill(server);
}

#[test]
fn the_socket_is_owner_only() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("socket-mode");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let mode = std::fs::metadata(&control_paths(&runtime).socket)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the socket is owner-only");

    kill(server);
}

/// The daemon's contract answers `until`'s shape: a reply to a `Notify`
/// is a `u32` id. (Corpus-adjacent pin: the e2e rows above drive the
/// real binary; this one pins the reply shape through the shared
/// helper.)
#[test]
fn notify_replies_carry_the_id() {
    let Some(bus) = bus_or_skip() else { return };
    let runtime = scratch("reply-shape");
    let mut server = spawn_server(&bus, &runtime);
    let mut client = connect(&bus);
    wait_serving(&mut client, &mut server);

    let body = notify_body("App", 0, "Summary", "Body", &[], -1);
    client
        .call(
            BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, 4242,
        )
        .unwrap();
    let Event::Reply {
        signature, body, ..
    } = until(
        &mut client,
        |event| matches!(event, Event::Reply { token: got, .. } if *got == 4242),
    )
    else {
        unreachable!()
    };
    assert_eq!(signature, "u");
    assert!(Reader::le(&body).u32().is_ok());

    kill(server);
}
