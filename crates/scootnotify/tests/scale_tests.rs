//! The stack draws at the compositor's scale, against a real compositor:
//! on a HiDPI output every buffer is exactly the surface's logical size
//! times the scale (not a 1x buffer the compositor upscales), the input
//! region stays in logical pixels, a click lands on the card the pixels
//! show, and a scale change while cards are open redraws them.
//!
//! Each test starts a headless scoot whose output has a scale (`[output]
//! scale` in its config, or `output-scale` live) and reads the daemon's own
//! `WAYLAND_DEBUG` trace: the `preferred_scale` the compositor sent, the
//! `wp_viewport.set_destination` (the surface's logical size), every
//! `wl_shm_pool.create_buffer` and the `wl_region.add` rectangles.
//!
//! Needs `dbus-daemon` on `PATH` and a `scoot` binary beside the test
//! binary, like `popups.rs`; without either the tests skip, saying so,
//! unless `SCOOTNOTIFY_REQUIRE_SCOOT` (or `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON`)
//! is set (CI sets them).

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
const FOCUS: &str = "#f9e2af";

/// The scales the output is set to, with the 120ths the compositor sends.
const SCALES: [(&str, u32); 5] = [
    ("1.0", 120),
    ("1.25", 150),
    ("1.5", 180),
    ("2", 240),
    ("3", 360),
];

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

/// A headless scoot whose one output is at `scale`.
fn scoot_at(tag: &str, scale: &str) -> Option<Session> {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_scootnotify"));
    let session = Session::scoot_requiring(
        &bin,
        tag,
        1,
        &format!("[output]\nscale = {scale}\n"),
        REQUIRE_SCOOT,
    );
    if session.is_none() {
        eprintln!("skipped: no scoot binary (set {REQUIRE_SCOOT}=1 to fail)");
    }
    session
}

/// Starts the daemon with `config` as its `notify.toml` under `look` (a
/// look fragment, or empty), with the Wayland wire log on.
fn spawn_popup(session: &Session, bus: &Daemon, config: &str, look: &str) -> (Child, PathBuf) {
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
    std::fs::write(&config_path, config).unwrap();
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

/// One card: a summary, a body long enough to wrap, and two buttons, so
/// the stack's height is not a round number at any fractional scale.
fn notify(client: &mut Conn, server: &mut Child, log: &Path, summary: &str, token: u64) -> u32 {
    let mut out = Writer::new();
    out.str("a");
    out.u32(0);
    out.str("");
    out.str(summary);
    out.str("A body long enough that it wraps onto a second line in a card");
    let cookie = out.open_array(4).unwrap();
    for text in ["open", "Open", "later", "Later"] {
        out.str(text);
    }
    out.close_array(cookie);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
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

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The numbers of every request or event `name` in the trace, in order:
/// requests print `.name(1, 2)`, events `.name, (1, 2)`.
fn calls(trace: &str, name: &str) -> Vec<Vec<i64>> {
    let request = format!(".{name}(");
    let event = format!(".{name}, (");
    trace
        .lines()
        .filter_map(|line| {
            let args = line
                .split(&request)
                .nth(1)
                .or_else(|| line.split(&event).nth(1))?;
            let args = args.split(')').next()?;
            Some(
                args.split(", ")
                    .filter_map(|n| n.trim().parse().ok())
                    .collect(),
            )
        })
        .collect()
}

/// The `preferred_scale`s the compositor sent, in 120ths.
fn preferred(log: &Path) -> Vec<u32> {
    let trace = std::fs::read_to_string(log).unwrap_or_default();
    calls(&trace, "preferred_scale")
        .iter()
        .filter_map(|n| u32::try_from(*n.first()?).ok())
        .collect()
}

/// The surface's logical size: the last viewport destination.
fn destination(log: &Path) -> Option<(u32, u32)> {
    let trace = std::fs::read_to_string(log).unwrap_or_default();
    let last = calls(&trace, "set_destination").pop()?;
    Some((
        u32::try_from(*last.first()?).ok()?,
        u32::try_from(*last.get(1)?).ok()?,
    ))
}

fn buffers(log: &Path) -> Vec<(u32, u32)> {
    test_support::created_buffers(&std::fs::read_to_string(log).unwrap_or_default())
}

/// The size of the buffer the last commit damaged: what is on screen. A
/// pooled buffer of the right size is attached again without being created
/// again, so after a scale change the last `create_buffer` can be older.
fn drawn(log: &Path) -> Option<(u32, u32)> {
    let trace = std::fs::read_to_string(log).unwrap_or_default();
    let last = calls(&trace, "damage_buffer").pop()?;
    Some((
        u32::try_from(*last.get(2)?).ok()?,
        u32::try_from(*last.get(3)?).ok()?,
    ))
}

/// The rectangles of the last input region the daemon set, as
/// `(x, y, width, height)`.
fn input_region(log: &Path) -> Vec<(i64, i64, i64, i64)> {
    let trace = std::fs::read_to_string(log).unwrap_or_default();
    let mut building: Vec<(i64, i64, i64, i64)> = Vec::new();
    let mut last = Vec::new();
    for line in trace.lines() {
        if line.contains(".create_region(") {
            building.clear();
        } else if line.contains("wl_region@") && line.contains(".add(") {
            if let Some(n) = calls(line, "add").pop()
                && let [x, y, w, h] = n[..]
            {
                building.push((x, y, w, h));
            }
        } else if line.contains(".set_input_region(wl_region@") {
            last = building.clone();
        }
    }
    last
}

/// `logical` at `v120`/120 in buffer pixels, rounded half up as the
/// protocol says.
fn scaled(logical: u32, v120: u32) -> u32 {
    (logical * v120 + 60) / 120
}

/// The bounding box of the focus ring on the output's screenshot (device
/// pixels): `(x0, y0, x1, y1)`.
fn ring_box(session: &Session) -> Option<(u32, u32, u32, u32)> {
    let shot = session.scoot_screenshot(1);
    let ring = rgb(FOCUS);
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..1000 {
        for x in 0..1600 {
            if shot.at(x, y) == ring {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    found
}

/// Starts the daemon on an output at `scale` and shows one card.
fn show_one(tag: &str, scale: &str, look: &str) -> Option<(Daemon, Session, Child, PathBuf, Conn)> {
    show_configured(tag, scale, "", look)
}

/// [`show_one`] with a `notify.toml`.
fn show_configured(
    tag: &str,
    scale: &str,
    config: &str,
    look: &str,
) -> Option<(Daemon, Session, Child, PathBuf, Conn)> {
    let bus = bus_or_skip()?;
    let session = scoot_at(tag, scale)?;
    let (mut server, log) = spawn_popup(&session, &bus, config, look);
    let mut client = conn::connect(&bus.path()).expect("the bus accepts");
    support::wait_serving(&mut client, &mut server, support::LogSource::File(&log));
    let _ = notify(&mut client, &mut server, &log, "Hello", 501);
    wait_until(&mut server, &log, "the card", || {
        ring_box(&session).is_some()
    });
    // Let the draws behind the first one land.
    std::thread::sleep(Duration::from_millis(500));
    Some((bus, session, server, log, client))
}

/// Subscribes `client` to the server's signals, barriered so the rule
/// is installed before the test triggers anything: a bare `AddMatch`
/// only leaves the outbox on the wait loop's first pump — after the
/// clicks — so a fast invoke's signals can reach the bus before the
/// rule exists, and a rule-less broadcast is dropped, which reads as
/// "no action invoked" (a ~2% CI flake). The barrier below closes it:
/// the bus installs one connection's calls in order, so the reply
/// proves the rule is live before the first click.
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
            scoot_dbus::proto::flag::NO_REPLY_EXPECTED,
            0,
        )
        .unwrap();
    // The barrier: the bus installs one connection's calls in order, so
    // the reply below proves the rule is live.
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "GetServerInformation",
            "",
            &[],
            0,
            8901,
        )
        .unwrap();
    let _ = until(client, |event| {
        matches!(event, Event::Reply { token: 8901, .. })
    });
}

/// The buffer is the surface's logical size times the scale, to the pixel,
/// from the first draw on, at the scales laptops ship with; the card is as
/// many pixels wide on screen as its logical width times the scale; and the
/// input region is the same rectangles at every scale (they are logical).
#[test]
fn the_buffer_is_the_surface_times_the_scale_at_every_scale() {
    let mut region_at_1x: Option<Vec<(i64, i64, i64, i64)>> = None;
    for (scale, v120) in SCALES {
        let Some((_bus, session, server, log, _client)) =
            show_one(&format!("scale-every-{scale}"), scale, "")
        else {
            return;
        };
        if v120 != 120 {
            assert_eq!(preferred(&log).last(), Some(&v120), "{scale}");
        }
        let (width, height) = destination(&log).expect("a viewport destination");
        let all = buffers(&log);
        assert!(!all.is_empty(), "{scale}: no buffer");
        for buffer in &all {
            assert_eq!(
                *buffer,
                (scaled(width, v120), scaled(height, v120)),
                "{scale}: a buffer for a {width}x{height} surface"
            );
        }
        eprintln!("{scale}: {width}x{height} logical at {v120}/120 -> buffers {all:?}");
        // On screen the ring spans the card's logical width times the scale.
        let (x0, _, x1, _) = ring_box(&session).expect("the card");
        let drawn = x1 - x0 + 1;
        let want = scaled(360, v120);
        assert!(
            drawn.abs_diff(want) <= 2,
            "{scale}: the card is {drawn} px wide on screen, {want} expected"
        );
        // The input region is in the surface's logical pixels: the same
        // rectangles whatever the scale, inside the surface.
        let region = input_region(&log);
        assert!(!region.is_empty(), "{scale}: no input region");
        for (x, y, w, h) in &region {
            assert!(
                *x >= 0 && *y >= 0 && x + w <= i64::from(width) && y + h <= i64::from(height),
                "{scale}: {x},{y} {w}x{h} outside {width}x{height}"
            );
        }
        match &region_at_1x {
            None => region_at_1x = Some(region),
            Some(first) => assert_eq!(&region, first, "{scale}: the region is logical"),
        }
        kill(server);
    }
}

/// A click lands on the card the pixels show, at a fractional scale: a
/// button invokes its action, a click beside the card does not.
#[test]
fn a_click_lands_on_the_card_at_a_fractional_scale() {
    let Some((_bus, session, server, log, mut client)) = show_one("scale-click", "1.5", "") else {
        return;
    };
    // Subscribe to the server's signals.
    watch_signals(&mut client);
    let (x0, y0, _, y1) = ring_box(&session).expect("the card");
    // The screenshot is in device pixels, the click in logical ones.
    let logical = |device: u32| (f64::from(device) / 1.5) as u32;
    // Outside the card, left of it: nothing is invoked, the card stays.
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{},"y":{},"button":"left"}}"#,
        logical(x0).saturating_sub(30),
        logical((y0 + y1) / 2)
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        ring_box(&session).is_some(),
        "a click beside it keeps the card"
    );
    // The first button: the bottom row of the card, left side.
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{},"y":{},"button":"left"}}"#,
        logical(x0) + 30,
        logical(y1) - 20
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    let start = Instant::now();
    let mut invoked = None;
    while invoked.is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no action invoked; the daemon said: {}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        let (events, _) = client.pump();
        for event in events {
            if let Event::Signal { member, body, .. } = event
                && member == "ActionInvoked"
            {
                let mut reader = Reader::le(&body);
                let _ = reader.u32();
                invoked = reader.str().ok().map(str::to_owned);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(invoked.as_deref(), Some("open"), "the first button");
    kill(server);
}

/// A scale change while cards are open redraws them at the new scale, keeps
/// the surface's logical size, and a card that arrives after is drawn at it
/// too.
#[test]
fn a_scale_change_while_cards_are_open_redraws_them() {
    let Some((_bus, session, mut server, log, mut client)) =
        show_one("scale-live", "1", "shadow = \"soft\"\n")
    else {
        return;
    };
    let (width, height) = destination(&log).expect("a destination");
    assert_eq!(drawn(&log), Some((width, height)));
    let (x0, _, x1, _) = ring_box(&session).expect("the card");
    let before = x1 - x0 + 1;
    for (setting, v120) in [("2.0", 240u32), ("1.5", 180), ("1.0", 120)] {
        let reply = session.scoot_ipc(&format!(
            r#"{{"type":"output_scale","output":1,"scale":{setting}}}"#
        ));
        assert_eq!(reply["type"], "ok", "{reply}");
        wait_until(
            &mut server,
            &log,
            &format!("a buffer at {v120}/120"),
            || drawn(&log) == Some((scaled(width, v120), scaled(height, v120))),
        );
        assert_eq!(destination(&log), Some((width, height)), "{setting}");
        std::thread::sleep(Duration::from_millis(300));
        let (x0, _, x1, _) = ring_box(&session).expect("the card");
        let drawn = x1 - x0 + 1;
        let want = scaled(before, v120);
        eprintln!("at {v120}/120: the card is {drawn} px wide on screen (1x: {before})");
        assert!(drawn.abs_diff(want) <= 2, "{drawn} wide at {v120}/120");
    }
    // A second card at 1.5x: the surface grows, the buffer follows.
    let reply = session.scoot_ipc(r#"{"type":"output_scale","output":1,"scale":1.5}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    wait_until(&mut server, &log, "a buffer at 180/120", || {
        drawn(&log) == Some((scaled(width, 180), scaled(height, 180)))
    });
    let second = notify(&mut client, &mut server, &log, "Second", 502);
    wait_until(&mut server, &log, "a taller surface", || {
        destination(&log).is_some_and(|(_, h)| h > height)
    });
    std::thread::sleep(Duration::from_millis(300));
    let (w2, h2) = destination(&log).unwrap();
    assert_eq!(drawn(&log), Some((scaled(w2, 180), scaled(h2, 180))));
    close(&mut client, second, 503);
    kill(server);
}

/// A look that moves the cards, on a fractional output: the card slides in
/// through layers composited at device pixels, lands on exactly the static
/// pixels, and the last buffer is the surface times the scale.
#[test]
fn a_sliding_card_lands_on_the_static_pixels_at_a_fractional_scale() {
    const MOTION_LOOK: &str = "motion = \"spring\"\nmotion-duration-ms = 300\nmotion-bounce = 0.5\n[card]\nenter = \"slide\"\nexit = \"fade\"\n";
    let Some((_bus, session, server, log, _client)) = show_one("scale-motion", "1.5", MOTION_LOOK)
    else {
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    let (width, height) = destination(&log).expect("a destination");
    assert_eq!(drawn(&log), Some((scaled(width, 180), scaled(height, 180))));
    // At rest the ring is the static ring: one box, the card's width times
    // the scale.
    let (x0, y0, x1, _) = ring_box(&session).expect("the card");
    assert!((x1 - x0 + 1).abs_diff(scaled(360, 180)) <= 2);
    // 12 logical px from the top edge, at 1.5x.
    assert_eq!(y0, scaled(12, 180), "the card's top edge");
    kill(server);
}

/// A stack whose size does not divide by the scale (an odd width, a border
/// of 3 px, a shadow) still fits its buffer to the pixel at the fractional
/// scales: the buffer is the rounded product, the card is as wide as the
/// rounded product on screen, and the input region is inside the surface.
#[test]
fn an_odd_sized_stack_fits_its_buffer_at_fractional_scales() {
    for (scale, v120) in [("1.25", 150u32), ("1.5", 180), ("1.75", 210)] {
        let Some((_bus, session, server, log, _client)) = show_configured(
            &format!("scale-odd-{scale}"),
            scale,
            "width_px = 361\nborder_px = 3\n",
            "shadow = \"soft\"\n",
        ) else {
            return;
        };
        let (width, height) = destination(&log).expect("a destination");
        eprintln!("odd {scale}: {width}x{height} logical -> {:?}", drawn(&log));
        assert_eq!(width % 2, 1, "an odd surface width: {width}");
        assert_eq!(
            drawn(&log),
            Some((scaled(width, v120), scaled(height, v120)))
        );
        for buffer in buffers(&log) {
            assert_eq!(
                buffer,
                (scaled(width, v120), scaled(height, v120)),
                "{scale}"
            );
        }
        let (x0, _, x1, _) = ring_box(&session).expect("the card");
        assert!(
            (x1 - x0 + 1).abs_diff(scaled(361, v120)) <= 2,
            "{scale}: {} px wide",
            x1 - x0 + 1
        );
        for (x, y, w, h) in input_region(&log) {
            assert!(
                x >= 0 && y >= 0 && x + w <= i64::from(width) && y + h <= i64::from(height),
                "{scale}: {x},{y} {w}x{h} outside {width}x{height}"
            );
        }
        kill(server);
    }
}
