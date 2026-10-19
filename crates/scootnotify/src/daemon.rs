//! The daemon: one `poll(2)` loop over the bus fd, one timerfd, and the
//! popup's Wayland fd.
//!
//! With a compositor the daemon shows the card stack on an on-demand
//! overlay surface (clocks start on the frame callback; hover and input
//! idle pause them); without one it stays headless (arrival is display,
//! cards wait for no surface). At idle — no cards with deadlines, no
//! surface, nothing staged — the timerfd disarms, and the loop sleeps on
//! the bus fd (and the Wayland fd, which speaks only for pointer, idle
//! and output events) alone: 0 wakeups with no cards.
//!
//! Exit statuses: 0 done (only `--check` exits on its own; the daemon
//! runs until killed), 1 failed, 3 the name is owned.

use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_rt::print::warn;

use crate::bus::server::{self, Server};
use crate::config::{self, Config};
use crate::look::{self};
use crate::model::store::IdFile;
use crate::popup::{self, Popup};

/// Runs the daemon: owns the name, serves until killed or the bus goes
/// away. Returns the exit status (3 when the name is owned).
pub fn serve(config_path: Option<&str>, font_path: Option<&str>) -> i32 {
    let path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path);
    let config = match load_or_defaults(&path) {
        Ok(config) => config,
        Err(error) => {
            // A bad config at start runs on built-in defaults and
            // reports the error: an exit would restart-loop with no
            // notification daemon at all. (Headless, stderr is the
            // report; with a surface, a card says so.)
            warn(format_args!(
                "scootnotify: {error}; running on built-in defaults"
            ));
            Config::default()
        }
    };
    let (look, notes) = look::load(None);
    for note in &notes {
        warn(format_args!("scootnotify: {note}"));
    }
    let font = font_path.map(PathBuf::from);
    let address = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
    let mut conn = match conn::bus_addr_for(address.as_deref()) {
        Ok(conn::BusAddr::Path(path)) => match conn::connect(&path) {
            Ok(conn) => conn,
            Err(error) => {
                warn(format_args!(
                    "scootnotify: cannot reach the session bus: {error}"
                ));
                return 1;
            }
        },
        Ok(conn::BusAddr::Abstract(name)) => match conn::connect_abstract(&name) {
            Ok(conn) => conn,
            Err(error) => {
                warn(format_args!(
                    "scootnotify: cannot reach the session bus: {error}"
                ));
                return 1;
            }
        },
        Err(()) => {
            warn(format_args!(
                "scootnotify: the session bus address names nothing this daemon dials"
            ));
            return 1;
        }
    };
    conn.set_max_message(server::MAX_MESSAGE);
    match server::request_name(&mut conn) {
        Ok(()) => {}
        Err(denied) => {
            warn(format_args!("scootnotify: {}", denied.report()));
            return 3;
        }
    }
    let ids = IdFile::open(ids_path());
    let mut server = Server::new(config.clone(), ids);
    // A compositor means popups (presentation on the frame callback);
    // none means headless (arrival is display). Either way the bus
    // serves: cards without a surface wait with their clocks stopped.
    let popup = match Popup::connect(&config, &look, font.as_ref()) {
        Ok(popup) => {
            server.set_present_on_arrival(false);
            Some(popup)
        }
        Err(note) => {
            warn(format_args!("scootnotify: {note}"));
            None
        }
    };
    match run_loop(&mut conn, &mut server, popup) {
        LoopEnd::BusGone => {
            warn(format_args!("scootnotify: the bus went away"));
            1
        }
        LoopEnd::CompositorGone => {
            warn(format_args!("scootnotify: the compositor went away"));
            1
        }
    }
}

/// Loads `path`, or the defaults when it is missing (a missing file is
/// the defaults, not an error): `Err` only for a present-but-bad file.
fn load_or_defaults(path: &std::path::Path) -> Result<Config, String> {
    match std::fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        Ok(bytes) => config::parse(&bytes),
    }
}

/// Where the id reservation lives:
/// `$XDG_RUNTIME_DIR/scootnotify-<display>.ids`, `None` without a
/// runtime dir (ephemeral ids, said by the store). A missing
/// `WAYLAND_DISPLAY` reads as `no-display`; an unwritable directory
/// degrades inside the store, never here.
fn ids_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty())?;
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| String::from("no-display"));
    let base = display.rsplit('/').next().unwrap_or("no-display");
    let base = if base.is_empty() { "no-display" } else { base };
    Some(PathBuf::from(dir).join(format!("scootnotify-{base}.ids")))
}

/// Why the loop ended: the daemon otherwise runs until killed.
enum LoopEnd {
    BusGone,
    CompositorGone,
}

/// Serves until the bus or the compositor goes away: pumps method calls
/// into the server, arms one timerfd to the earliest expiry, fires what
/// is due, and draws the stack when dirty.
fn run_loop(conn: &mut Conn, server: &mut Server, mut popup: Option<Popup>) -> LoopEnd {
    let timer = match timerfd() {
        Some(timer) => timer,
        None => return LoopEnd::BusGone,
    };
    let mut hovered: Option<u32> = None;
    let spin_debug = std::env::var_os("SCOOTNOTIFY_SPIN_DEBUG").is_some();
    let mut spins: u64 = 0;
    loop {
        let (events, capped) = conn.pump();
        if conn.dead() {
            return LoopEnd::BusGone;
        }
        let now_ms = now_ms();
        for event in events {
            if let Event::MethodCall {
                sender,
                path,
                interface,
                member,
                serial,
                signature,
                body,
            } = event
            {
                server.handle(
                    conn, &sender, &path, &interface, &member, serial, &signature, &body, now_ms,
                );
                if conn.dead() {
                    return LoopEnd::BusGone;
                }
            }
        }
        if let Some(popup) = popup.as_mut() {
            for event in popup.dispatch() {
                apply_popup(conn, server, event, &mut hovered);
                if conn.dead() {
                    return LoopEnd::BusGone;
                }
            }
            popup.refresh(&server.store().ordered(), now_ms);
        }
        server.fire_expiries(conn);
        if let Some(popup) = popup.as_mut() {
            popup.draw();
        }
        arm_timer(&timer, server.earliest_deadline());
        if conn.dead() {
            return LoopEnd::BusGone;
        }
        if popup.as_ref().is_some_and(|popup| popup.gone()) {
            return LoopEnd::CompositorGone;
        }
        if capped || conn.has_staged_work() || conn.want_write() {
            if spin_debug {
                spins += 1;
                if spins == 200_000 {
                    // To a file, not stderr: the harness holds the
                    // pipe open while the daemon lives, so stderr is
                    // unreadable until it dies.
                    let _ = std::fs::write(
                        "/tmp/scootnotify-spin.log",
                        format!(
                            "200k no-poll turns (capped={capped} staged={} write={})\n",
                            conn.has_staged_work(),
                            conn.want_write()
                        ),
                    );
                }
            }
            continue;
        }
        spins = 0;
        // The Wayland read guard: prepared before the poll, read when
        // the fd speaks, dropped unread otherwise (the documented
        // read-after-poll shape: the next turn collects it).
        let guard = popup.as_ref().and_then(|popup| popup.prepare_read());
        let wayland = wait(conn.as_fd(), timer.as_fd(), popup.as_ref().map(|popup| popup.fd()));
        drain_timer(&timer);
        if wayland
            && let Some(guard) = guard
        {
            let _ = guard.read();
        }
        if conn.dead() {
            return LoopEnd::BusGone;
        }
    }
}

/// Applies one popup event to the store, queueing its signals: the
/// token (if minted) always precedes its invoke.
fn apply_popup(conn: &mut Conn, server: &mut Server, event: popup::Event, hovered: &mut Option<u32>) {
    let now = Instant::now();
    match event {
        popup::Event::Presented(ids) => {
            let now_ms = now_ms();
            for id in ids {
                server.store_mut().presented(id, now, now_ms);
            }
        }
        popup::Event::ClickBody { id, token } => {
            let has_default = server
                .store()
                .get(id)
                .is_some_and(|note| note.actions.iter().any(|pair| pair.key == "default"));
            if has_default {
                server.invoke_and_signal(conn, id, "default", token.as_deref());
            } else {
                server.dismiss_and_signal(conn, id);
            }
        }
        popup::Event::ClickButton { id, key, token } => {
            server.invoke_and_signal(conn, id, &key, token.as_deref());
        }
        popup::Event::Dismiss(id) => {
            server.dismiss_and_signal(conn, id);
        }
        popup::Event::DismissAll => {
            server.dismiss_all_and_signal(conn);
        }
        popup::Event::Hover(card) => {
            if *hovered != card {
                if let Some(old) = hovered.take() {
                    server.store_mut().resume(old, now);
                }
                if let Some(id) = card {
                    server.store_mut().pause(id, now);
                    *hovered = Some(id);
                }
            }
        }
        popup::Event::Idle(idled) => {
            if idled {
                server.store_mut().pause_all(now);
            } else {
                server.store_mut().resume_all(now);
                // The hover still holds its card (the pointer never
                // left): re-park it after the resume-all.
                if let Some(id) = *hovered {
                    server.store_mut().pause(id, now);
                }
            }
        }
    }
}

/// CLOCK_REALTIME in ms: the JSON `created`/`shown`.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|left| left.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// One timerfd, nonblocking: the expiry arm.
fn timerfd() -> Option<rustix::fd::OwnedFd> {
    use rustix::time::{TimerfdClockId, TimerfdFlags, timerfd_create};
    timerfd_create(
        TimerfdClockId::Monotonic,
        TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
    )
    .ok()
}

/// Arms `timer` to `deadline` (disarmed for `None`): one timer for the
/// earliest expiry across all cards, re-armed on change.
fn arm_timer(timer: &rustix::fd::OwnedFd, deadline: Option<Instant>) {
    use rustix::time::{Itimerspec, TimerfdTimerFlags, Timespec, timerfd_settime};
    let to_spec = |left: Duration| {
        let secs = left.as_secs().min(i64::MAX as u64) as i64;
        Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: secs,
                tv_nsec: i64::from(left.subsec_nanos()),
            },
        }
    };
    let spec = match deadline {
        Some(at) => {
            let left = at.saturating_duration_since(Instant::now());
            // A zero arm fires at once: the loop fires what is due
            // before it waits, so expiry never strands.
            to_spec(left.max(Duration::from_millis(1)))
        }
        None => to_spec(Duration::ZERO),
    };
    let _ = timerfd_settime(timer, TimerfdTimerFlags::empty(), &spec);
}

/// Polls the bus fd, the timerfd, and the Wayland fd (when there is
/// one) with no timeout: woken by traffic or the next expiry, never by
/// time otherwise. Returns whether the Wayland fd spoke. A bounded poll
/// here would be a wakeup every bound at idle; the loop only reaches
/// this wait with nothing staged, nothing queued and the timer armed,
/// so nothing is missed by sleeping until the fds speak (all poll
/// level-triggered).
fn wait(bus: BorrowedFd<'_>, timer: BorrowedFd<'_>, wayland: Option<BorrowedFd<'_>>) -> bool {
    use rustix::event::{PollFd, PollFlags, poll};
    let third = wayland.unwrap_or(bus);
    let mut fds = [
        PollFd::new(&bus, PollFlags::IN),
        PollFd::new(&timer, PollFlags::IN),
        PollFd::new(&third, PollFlags::IN),
    ];
    let _ = poll(&mut fds, None);
    wayland.is_some() && !fds[2].revents().is_empty()
}

/// Reads a fired timerfd back to disarmed: edge-triggered peace.
fn drain_timer(timer: &rustix::fd::OwnedFd) {
    use rustix::io::read;
    let mut buf = [0u8; 8];
    let _ = read(timer, &mut buf);
}
