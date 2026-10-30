//! Motion against a real compositor: a look that opts in animates a card
//! in, restacks the cards beside one that closes, lands on exactly the
//! pixels the static look draws, and then asks for nothing: no frame
//! callback and no wakeup while the stack sits there.
//!
//! Needs `dbus-daemon` on `PATH` and a `scoot` binary beside the test
//! binary, like `popups.rs`; without either the tests skip, saying so,
//! unless `SCOOTNOTIFY_REQUIRE_SCOOT` (or `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON`)
//! is set (CI sets them).
//!
//! The animation's own frames are not asserted from screenshots (a shot
//! lands at a time the test does not control); the settled pixels and the
//! frame requests on the Wayland wire are, and the frames themselves are
//! pinned by goldens in the repaint's unit tests.

#[path = "support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{Reader, Writer};
use scoot_dbus::testdaemon::{Daemon, until};
use scootui::test_support::{self, Session, rgb};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFY_SIG: &str = "susssasa{sv}i";
const REQUIRE_SCOOT: &str = "SCOOTNOTIFY_REQUIRE_SCOOT";
const REQUIRE_BUS: &str = "SCOOTNOTIFY_REQUIRE_DBUS_DAEMON";

/// The headless compositor width: the stack anchors top-right of this.
const WIDTH: u32 = 1600;
const FOCUS: &str = "#f9e2af";
const SURFACE: &str = "#1e1e2e";

/// The look every test here runs under: a spring over 300 ms, cards
/// sliding in from the right edge and fading out.
const MOTION_LOOK: &str = "motion = \"spring\"\nmotion-duration-ms = 300\nmotion-bounce = 0.5\n[card]\nenter = \"slide\"\nexit = \"fade\"\n";

fn stack_origin() -> (u32, u32) {
    (WIDTH - 360 - 12, 12)
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

fn scoot_or_skip(tag: &str) -> Option<Session> {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_scootnotify"));
    let session = Session::scoot_requiring(&bin, tag, 1, "", REQUIRE_SCOOT);
    if session.is_none() {
        eprintln!("skipped: no scoot binary (set {REQUIRE_SCOOT}=1 to fail)");
    }
    session
}

/// Starts the daemon under `look` (a `notify.toml` look fragment) with
/// the Wayland wire log on, so the frame requests can be counted.
fn spawn_popup(session: &Session, bus: &Daemon, look: &str) -> (Child, PathBuf) {
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    std::fs::create_dir_all(&look_dir).unwrap();
    std::fs::write(look_dir.join("notify.toml"), look).unwrap();
    let config_path = session
        .scratch
        .0
        .join("config")
        .join("scoot")
        .join("notify.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, "").unwrap();
    let log = session.scratch.0.join("scootnotify.log");
    let log_file = std::fs::File::create(&log).expect("the daemon log creates");
    let child = Command::new(env!("CARGO_BIN_EXE_scootnotify"))
        .arg("daemon")
        .arg("--config")
        .arg(&config_path)
        .arg("--font")
        .arg(session.scratch.font())
        .env("WAYLAND_DEBUG", "1")
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
        .stderr(log_file)
        .spawn()
        .expect("the popup binary starts");
    (child, log)
}

/// How many `wl_surface.frame` requests the daemon has made so far.
fn frame_requests(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("-> wl_surface@") && line.contains(".frame("))
        .count()
}

fn notify(client: &mut Conn, server: &mut Child, log: &Path, summary: &str, token: u64) -> u32 {
    let mut out = Writer::new();
    out.str("a");
    out.u32(0);
    out.str("");
    out.str(summary);
    out.str("World");
    let cookie = out.open_array(4).unwrap();
    out.close_array(cookie);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    // Never expires: the card stays until the test closes it.
    out.i32(0);
    let body = out.take_body().unwrap();
    client
        .call(
            BUS_NAME, OBJECT, INTERFACE, "Notify", NOTIFY_SIG, &body, 0, token,
        )
        .expect("the call queues");
    let start = Instant::now();
    loop {
        if start.elapsed() >= Duration::from_secs(10) {
            let err = std::fs::read_to_string(log).unwrap_or_default();
            let fate = server.try_wait().expect("the server reaps");
            panic!("Notify {token} unanswered ({fate:?}): {err}");
        }
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

fn close(client: &mut Conn, id: u32, token: u64) {
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

fn wait_until(server: &mut Child, log: &Path, what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    loop {
        if check() {
            return;
        }
        if let Some(status) = server.try_wait().unwrap() {
            let err = std::fs::read_to_string(log).unwrap_or_default();
            panic!("{what}: the daemon died ({status}): {err}");
        }
        if start.elapsed() >= Duration::from_secs(15) {
            let err = std::fs::read_to_string(log).unwrap_or_default();
            panic!("never {what}; the daemon said: {err}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The first row of every run of `FOCUS` down column `x`: a card has two
/// rings, its top edge and its bottom edge, whatever the ring's width.
fn ring_runs(session: &Session, x: u32) -> Vec<u32> {
    let shot = session.scoot_screenshot(1);
    let mut runs = Vec::new();
    let mut inside = false;
    for y in 0..600 {
        let ring = shot.at(x, y) == rgb(FOCUS);
        if ring && !inside {
            runs.push(y);
        }
        inside = ring;
    }
    runs
}

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A card slides in, lands on the static pixels, and then the daemon
/// asks for no more frames and wakes for nothing: motion costs what it
/// moves and not a frame more.
#[test]
fn a_card_slides_in_lands_and_then_the_stack_is_idle() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("motion-idle") else {
        return;
    };
    let (mut server, log) = spawn_popup(&session, &bus, MOTION_LOOK);
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    support::wait_serving(&mut client, &mut server, support::LogSource::File(&log));
    let (x0, y0) = stack_origin();
    let frames_before = frame_requests(&log);
    let _ = notify(&mut client, &mut server, &log, "Hello", 301);
    // Landed: exactly where the static look puts it.
    wait_until(&mut server, &log, "the card landed", || {
        let shot = session.scoot_screenshot(1);
        shot.at(x0 + 180, y0 + 1) == rgb(FOCUS) && shot.at(x0 + 5, y0 + 30) == rgb(SURFACE)
    });
    // Let the last animated frame (and any late callback) drain.
    std::thread::sleep(Duration::from_millis(1_000));
    let animated = frame_requests(&log) - frames_before;
    eprintln!("frame requests while a 300 ms slide ran: {animated}");
    assert!(animated >= 2, "a 300 ms slide asks for frames: {animated}");
    assert!(
        animated <= 60,
        "one callback outstanding at a time: {animated}"
    );
    // At rest: no more frames, no more wakeups, however long it sits.
    let pid = server.id();
    let (frames, wakeups) = (frame_requests(&log), test_support::wakeups(pid));
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        frame_requests(&log),
        frames,
        "a settled stack asks for no frames"
    );
    assert_eq!(
        test_support::wakeups(pid),
        wakeups,
        "a settled stack wakes for nothing"
    );
    kill(server);
}

/// Closing the newest of two cards: the older settles up into its place
/// and the stack ends up exactly the static one-card stack.
#[test]
fn closing_a_card_restacks_the_one_beside_it() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("motion-restack") else {
        return;
    };
    let (mut server, log) = spawn_popup(&session, &bus, MOTION_LOOK);
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    support::wait_serving(&mut client, &mut server, support::LogSource::File(&log));
    let (x0, y0) = stack_origin();
    let column = x0 + 180;
    let _ = notify(&mut client, &mut server, &log, "First", 311);
    wait_until(&mut server, &log, "the first card", || {
        session.scoot_screenshot(1).at(column, y0 + 1) == rgb(FOCUS)
    });
    let newest = notify(&mut client, &mut server, &log, "Second", 312);
    // Two cards at rest: four rings down the column.
    wait_until(&mut server, &log, "two cards at rest", || {
        std::thread::sleep(Duration::from_millis(400));
        ring_runs(&session, column).len() == 4
    });
    close(&mut client, newest, 313);
    // The first card is now alone at the top: two rings, the first at
    // the stack's top edge.
    wait_until(&mut server, &log, "the survivor settled at the top", || {
        let rings = ring_runs(&session, column);
        rings.len() == 2 && rings[0] == y0
    });
    std::thread::sleep(Duration::from_millis(800));
    let frames = frame_requests(&log);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(frame_requests(&log), frames, "idle again after the restack");
    kill(server);
}

/// Twenty arrivals in a burst: the daemon keeps up, shows the newest
/// five, lands on the static stack and goes idle.
#[test]
fn a_burst_of_arrivals_lands_and_goes_idle() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("motion-burst") else {
        return;
    };
    let (mut server, log) = spawn_popup(&session, &bus, MOTION_LOOK);
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    support::wait_serving(&mut client, &mut server, support::LogSource::File(&log));
    let (x0, _) = stack_origin();
    for n in 0..20u64 {
        let _ = notify(&mut client, &mut server, &log, &format!("n{n}"), 400 + n);
    }
    // Five cards (the default `max_visible`) at rest: ten rings.
    wait_until(&mut server, &log, "five cards at rest", || {
        std::thread::sleep(Duration::from_millis(500));
        ring_runs(&session, x0 + 180).len() == 10
    });
    std::thread::sleep(Duration::from_millis(1_000));
    let frames = frame_requests(&log);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(frame_requests(&log), frames, "idle after the burst");
    assert!(server.try_wait().unwrap().is_none(), "the daemon survived");
    kill(server);
}

/// The last card closing: it fades out, the surface goes away with its
/// buffers, and nothing keeps asking for frames.
#[test]
fn the_last_card_fades_out_and_the_surface_goes_away() {
    let Some(bus) = bus_or_skip() else { return };
    let Some(session) = scoot_or_skip("motion-last") else {
        return;
    };
    let (mut server, log) = spawn_popup(&session, &bus, MOTION_LOOK);
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    support::wait_serving(&mut client, &mut server, support::LogSource::File(&log));
    let (x0, y0) = stack_origin();
    let pid = server.id();
    let id = notify(&mut client, &mut server, &log, "Only", 321);
    wait_until(&mut server, &log, "the card landed", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 1) == rgb(FOCUS)
    });
    std::thread::sleep(Duration::from_millis(600));
    assert!(test_support::shm_mappings_named(pid, "scootnotify") >= 1);
    close(&mut client, id, 322);
    wait_until(&mut server, &log, "the stack gone", || {
        session.scoot_screenshot(1).at(x0 + 180, y0 + 1) != rgb(FOCUS)
    });
    wait_until(&mut server, &log, "the buffers unmapped", || {
        test_support::shm_mappings_named(pid, "scootnotify") == 0
    });
    let frames = frame_requests(&log);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(frame_requests(&log), frames, "no frames with no cards");
    kill(server);
}
