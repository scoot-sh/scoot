//! The bar feed against a real scootbar on a headless compositor: the
//! daemon binary pushes `set notifications` on change only, a bar
//! started later catches up on the next change, and an idle daemon
//! holds no watch and wakes nothing.
//!
//! Needs a private `dbus-daemon`, a `scoot` binary and a `scootbar`
//! binary beside the test binary (CI builds `-p scoot -p scootnotify
//! -p scootbar`): without any of them the tests skip, saying so,
//! unless `SCOOTNOTIFY_REQUIRE_SCOOT` is set.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{Reader, Writer};
use scoot_dbus::testdaemon::{Daemon, until};
use scootui::test_support::Session;

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFY_SIG: &str = "susssasa{sv}i";

const REQUIRE_SCOOT: &str = "SCOOTNOTIFY_REQUIRE_SCOOT";
const REQUIRE_BUS: &str = "SCOOTNOTIFY_REQUIRE_DBUS_DAEMON";

fn notify_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootnotify"))
}

/// The bar binary beside the test binary, or `None`: CI builds it
/// beside this package's tests, like it builds `scoot` for the
/// harness.
fn scootbar_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SCOOTNOTIFY_TEST_SCOOTBAR").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let sibling = notify_bin().with_file_name("scootbar");
    sibling.is_file().then_some(sibling)
}

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

fn session_or_skip(tag: &str) -> Option<Session> {
    let session = Session::scoot_requiring(&notify_bin(), tag, 1, "", REQUIRE_SCOOT);
    if session.is_none() {
        eprintln!("skipped: no scoot binary (set {REQUIRE_SCOOT}=1 to fail)");
    }
    session
}

fn bar_or_skip(session: &Session) -> Option<PathBuf> {
    let bar = scootbar_bin();
    if bar.is_none() {
        assert!(
            std::env::var_os(REQUIRE_SCOOT).is_none(),
            "{REQUIRE_SCOOT} is set but there is no scootbar binary beside the test binary"
        );
        eprintln!(
            "skipped: no scootbar binary (build -p scootbar, or set {REQUIRE_SCOOT}=1 to fail)"
        );
    }
    let _ = session;
    bar
}

fn connect(bus: &Daemon) -> Conn {
    conn::connect(&bus.path()).expect("the daemon accepts EXTERNAL and says Hello")
}

/// One `Notify` body: app, no replace, empty icon, summary, body, no
/// actions, no hints, `expire`.
fn notify_body(app: &str, replaces: u32, summary: &str, body_text: &str, expire: i32) -> Vec<u8> {
    let mut out = Writer::new();
    out.str(app);
    out.u32(replaces);
    out.str("");
    out.str(summary);
    out.str(body_text);
    let cookie = out.open_array(4).unwrap();
    out.close_array(cookie);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    out.i32(expire);
    out.take_body().unwrap()
}

/// One `Notify` answered: returns the id. Never expires (`expire` 0),
/// so counts stay where the test puts them.
fn notify(client: &mut Conn, app: &str, token: u64) -> u32 {
    let body = notify_body(app, 0, "s", "b", 0);
    client
        .call(
            BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token,
        )
        .expect("the call queues");
    let Event::Reply {
        signature, body, ..
    } = until(
        client,
        |event| matches!(event, Event::Reply { token: got, .. } if *got == token),
    )
    else {
        unreachable!()
    };
    assert_eq!(signature, "u");
    Reader::le(&body).u32().expect("a Notify answers its id")
}

/// One `CloseNotification`, answered or refused.
fn close(client: &mut Conn, id: u32, token: u64) {
    let mut out = Writer::new();
    out.u32(id);
    let body = out.take_body().unwrap();
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "CloseNotification",
            "u",
            &body,
            0,
            token,
        )
        .expect("the call queues");
    let _ = until(
        client,
        |event| matches!(event, Event::Reply { token: got, .. } if *got == token),
    );
}

/// Spawns the real bar on `session` with a `notifications` push module
/// showing `placeholder` until the first set. Returns the child.
fn spawn_bar(session: &Session, bar: &Path, placeholder: &str) -> Child {
    let config = session.scratch.config_home().join("scoot");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("bar.toml"),
        format!(
            "center = [\"notifications\"]\n[push.notifications]\nplaceholder = \"{placeholder}\"\n"
        ),
    )
    .unwrap();
    // The daemon reads its own bar.toml (never the machine's): the
    // harness sandbox pins every XDG dir at the scratch.
    let log = std::fs::File::create(session.scratch.0.join("scootbar.log")).unwrap();
    let mut command = Command::new(bar);
    command
        .arg("daemon")
        .arg("--font")
        .arg(session.font())
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    session.scratch.sandbox(&mut command);
    command.spawn().expect("the bar starts")
}

/// Spawns the real daemon binary headless-but-composited: on
/// `session`'s compositor (popups have somewhere to go) and `bus`'s
/// bus, with defaults (no notify.toml in the scratch home).
fn spawn_notify(session: &Session, bus: &Daemon) -> Child {
    let log = std::fs::File::create(session.scratch.0.join("scootnotify.log")).unwrap();
    let mut command = Command::new(notify_bin());
    command
        .arg("daemon")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", bus.path().display()),
        )
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    session.scratch.sandbox(&mut command);
    command.spawn().expect("the daemon starts")
}

/// One `scootnotify msg` verb: the reply line, trimmed.
fn notify_msg(session: &Session, args: &[&str]) -> String {
    let mut command = Command::new(notify_bin());
    command
        .arg("msg")
        .args(args)
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    session.scratch.sandbox(&mut command);
    let output = command.output().expect("msg runs");
    assert!(
        output.status.success(),
        "msg {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("msg speaks UTF-8")
        .trim()
        .to_owned()
}

/// One `scootbar msg query notifications`: the module's `(text, class)`.
fn bar_module(session: &Session, bar: &Path) -> (String, String) {
    let mut command = Command::new(bar);
    command
        .arg("msg")
        .arg("query")
        .arg("notifications")
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    session.scratch.sandbox(&mut command);
    let output = command.output().expect("the bar answers queries");
    assert!(
        output.status.success(),
        "bar query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the reply is JSON");
    let module = reply
        .get("modules")
        .and_then(|modules| modules.get(0))
        .expect("one module listed");
    (
        module
            .get("text")
            .and_then(|text| text.as_str())
            .unwrap_or_default()
            .to_owned(),
        module
            .get("class")
            .and_then(|class| class.as_str())
            .unwrap_or_default()
            .to_owned(),
    )
}

/// Spins until `want` holds or the deadline passes: the feed pushes a
/// turn after the call that changed the count answers.
fn until_bar(session: &Session, bar: &Path, text: &str, class: &str) -> (String, String) {
    let got = poll_bar(session, bar, text, class, Duration::from_secs(10));
    assert_eq!(
        got,
        (String::from(text), String::from(class)),
        "the bar never showed ({text}, {class}): last ({}, {})",
        got.0,
        got.1
    );
    got
}

/// Polls the bar until it shows `(text, class)` or `budget` passes:
/// returns the last reading either way, so callers can nudge and keep
/// waiting instead of failing on one transient push failure.
fn poll_bar(
    session: &Session,
    bar: &Path,
    text: &str,
    class: &str,
    budget: Duration,
) -> (String, String) {
    let start = Instant::now();
    loop {
        let got = bar_module(session, bar);
        if got.0 == text && got.1 == class {
            return got;
        }
        if start.elapsed() >= budget {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits for the bar to show `(text, class)`, driving one net-zero
/// change (an arrival closed at once) halfway through: with the
/// product fix a transient push failure converges on its own bounded
/// retry, and the nudge unsticks even a retry-exhausted spell — while
/// the final state stays exact. `nudge` supplies fresh bus tokens.
fn until_bar_converged(
    session: &Session,
    bar: &Path,
    client: &mut Conn,
    text: &str,
    class: &str,
    nudge: &mut u64,
) -> (String, String) {
    let got = poll_bar(session, bar, text, class, Duration::from_secs(5));
    if got.0 == text && got.1 == class {
        return got;
    }
    // One net-zero change: the count returns exactly where it
    // started (an arrival closed at once), but the fresh push carries
    // the current state again.
    let id = notify(client, "nudge", *nudge);
    *nudge += 1;
    close(client, id, *nudge);
    *nudge += 1;
    until_bar(session, bar, text, class)
}

/// Spins until the bar answers queries: spawning returns before the
/// daemon serves. Fails fast when the child died (its log names why),
/// instead of timing out on a crash.
fn wait_bar_serving(session: &Session, bar: &Path, child: &mut Child) {
    let start = Instant::now();
    loop {
        let mut command = Command::new(bar);
        command
            .arg("msg")
            .arg("query")
            .arg("notifications")
            .env("XDG_RUNTIME_DIR", &session.scratch.0)
            .env("WAYLAND_DISPLAY", &session.wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        session.scratch.sandbox(&mut command);
        if command
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return;
        }
        // A dead child never serves: fail on its status now, with the
        // log path, not at the deadline.
        if let Some(status) = child.try_wait().expect("the bar reaps") {
            panic!("the bar died at startup ({status}): see scootbar.log in the scratch dir");
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the bar never served"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Spins until the daemon answers `msg version`: spawning returns
/// before it owns the name. Fails fast when the child died.
fn wait_serving(session: &Session, notify: &mut Child) {
    let start = Instant::now();
    loop {
        let mut command = Command::new(notify_bin());
        command
            .arg("msg")
            .arg("version")
            .env("XDG_RUNTIME_DIR", &session.scratch.0)
            .env("WAYLAND_DISPLAY", &session.wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        session.scratch.sandbox(&mut command);
        if command
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return;
        }
        if let Some(status) = notify.try_wait().expect("the daemon reaps") {
            panic!("the daemon died at startup ({status}): see scootnotify.log in the scratch dir");
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the daemon never served"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Kills `child` and waits for it out: SIGKILL leaves a stale socket
/// file, which the next bar's claim unlinks (the feed's watch fires
/// on the rebind either way).
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn arrivals_dismissals_and_dnd_toggles_reflect_exactly() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = session_or_skip("barfeed-exact") else {
        return;
    };
    let Some(bar) = bar_or_skip(&session) else {
        return;
    };
    let mut bar_child = spawn_bar(&session, &bar, "idle");
    let mut notify_child = spawn_notify(&session, &bus);
    wait_serving(&session, &mut notify_child);
    wait_bar_serving(&session, &bar, &mut bar_child);
    // The bar starts on its placeholder: the daemon baselined without
    // sending, so nothing pushed yet.
    assert_eq!(
        bar_module(&session, &bar),
        (String::from("idle"), String::from("normal"))
    );
    let mut client = connect(&bus);
    // One arrival: the count pushes. Each stage converges (nudging
    // once past a transient push failure) on the exact final state —
    // never less, never "eventually something close".
    let mut nudge = 1000;
    let id = notify(&mut client, "a", 101);
    assert_eq!(
        until_bar_converged(&session, &bar, &mut client, "1", "normal", &mut nudge).0,
        "1"
    );
    // Its dismissal: idle again.
    close(&mut client, id, 102);
    assert_eq!(
        until_bar_converged(&session, &bar, &mut client, "", "normal", &mut nudge).0,
        ""
    );
    // DND on: the next arrival holds, and the bar mutes.
    assert!(notify_msg(&session, &["dnd", "on"]).contains("\"dnd\":\"on\""));
    notify(&mut client, "b", 103);
    assert_eq!(
        until_bar_converged(&session, &bar, &mut client, "DND 1", "muted", &mut nudge).0,
        "DND 1"
    );
    // DND off with no new arrival: the same held card reshapes the bar.
    assert!(notify_msg(&session, &["dnd", "off"]).contains("\"dnd\":\"off\""));
    assert_eq!(
        until_bar_converged(&session, &bar, &mut client, "1", "normal", &mut nudge).0,
        "1"
    );
    kill(&mut notify_child);
    kill(&mut bar_child);
}

#[test]
fn a_bar_started_later_catches_up_on_the_next_change() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = session_or_skip("barfeed-late") else {
        return;
    };
    let Some(bar) = bar_or_skip(&session) else {
        return;
    };
    // No bar: the daemon owes its first change.
    let mut notify_child = spawn_notify(&session, &bus);
    wait_serving(&session, &mut notify_child);
    let mut client = connect(&bus);
    notify(&mut client, "a", 201);
    // The bar starts later: the socket watch re-sends the owed state,
    // so it catches up without waiting for the next change.
    let mut bar_child = spawn_bar(&session, &bar, "idle");
    wait_bar_serving(&session, &bar, &mut bar_child);
    assert_eq!(until_bar(&session, &bar, "1", "normal").0, "1");
    // And the next change still lands exactly.
    notify(&mut client, "b", 202);
    assert_eq!(until_bar(&session, &bar, "2", "normal").0, "2");
    kill(&mut notify_child);
    kill(&mut bar_child);
}

/// Context switches of `pid`, voluntary and involuntary: a daemon
/// that truly sleeps holds both steady.
fn switches(pid: u32) -> (u64, u64) {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("status reads");
    let mut voluntary = 0;
    let mut involuntary = 0;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("voluntary_ctxt_switches:") {
            voluntary = value.trim().parse().expect("a number");
        }
        if let Some(value) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
            involuntary = value.trim().parse().expect("a number");
        }
    }
    (voluntary, involuntary)
}

/// Whether every timerfd of `pid` is disarmed (`it_value` zero): the
/// load-independent core of "the daemon sleeps" — no expiry arm, so no
/// timer wakeup whatever the load. (The feed holds no watch either,
/// asserted at the call site, and adds no poll timeout while nothing
/// is owed.)
fn timer_disarmed(pid: u32) -> bool {
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    let mut found = false;
    for entry in fds.filter_map(|entry| entry.ok()) {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if target.to_string_lossy() != "anon_inode:[timerfd]" {
            continue;
        }
        found = true;
        let name = entry.file_name();
        let info =
            std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", name.to_string_lossy()))
                .unwrap_or_default();
        if !info.lines().any(|line| line.trim() == "it_value: (0, 0)") {
            return false;
        }
    }
    // The daemon always holds its expiry timerfd: finding none means
    // the scan missed, not that the daemon sleeps.
    found
}

/// Whether `pid` holds an inotify fd: the feed's watch, armed only
/// while a send is owed.
fn has_inotify(pid: u32) -> bool {
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    fds.filter_map(|entry| entry.ok())
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .any(|target| target.to_string_lossy() == "anon_inode:inotify")
}

#[test]
fn idle_holds_no_watch_and_wakes_nothing() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = session_or_skip("barfeed-idle") else {
        return;
    };
    let Some(bar) = bar_or_skip(&session) else {
        return;
    };
    let mut bar_child = spawn_bar(&session, &bar, "idle");
    let mut notify_child = spawn_notify(&session, &bus);
    wait_serving(&session, &mut notify_child);
    wait_bar_serving(&session, &bar, &mut bar_child);
    let mut client = connect(&bus);
    // Settle one change through, so nothing is owed.
    notify(&mut client, "a", 301);
    assert_eq!(until_bar(&session, &bar, "1", "normal").0, "1");
    let pid = notify_child.id();
    // The load-independent core: no watch fd, and the expiry timerfd
    // disarmed — so neither the feed nor an expiry can have woken the
    // daemon, whatever the load.
    assert!(!has_inotify(pid), "no watch while nothing is owed");
    assert!(timer_disarmed(pid), "no timer while nothing is owed");
    // Settle: startup turns (compositor attach, the bus Hello, the
    // first push) trail the converged bar; the window below must not
    // count them.
    std::thread::sleep(Duration::from_secs(2));
    // Two seconds with no traffic and no queries: near-zero wakeups.
    // Bounded, not exact-zero: on a loaded runner the live
    // compositor/bus delivers unrelated turns (R-689 measured +3 on CI
    // with no watch armed — the feed provably cannot have woken it,
    // per the two asserts above). The bound says "asleep", not "dead".
    let before = switches(pid);
    std::thread::sleep(Duration::from_secs(2));
    let after = switches(pid);
    let woken = after
        .0
        .saturating_sub(before.0)
        .saturating_add(after.1.saturating_sub(before.1));
    assert!(
        woken <= 10,
        "near-zero wakeups at idle: {before:?} -> {after:?}"
    );
    kill(&mut notify_child);
    kill(&mut bar_child);
}
