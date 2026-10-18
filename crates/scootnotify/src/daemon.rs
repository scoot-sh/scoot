//! The daemon: one `poll(2)` loop over the bus fd and one timerfd.
//!
//! Headless in this ticket: no Wayland, no surface, no pixels. Arrival
//! is display (the clock starts at once); the popups ticket moves
//! presentation to the frame callback, the control ticket adds the
//! socket. At idle — no cards with deadlines — the timerfd disarms, and
//! the loop sleeps on the bus fd alone.
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
use crate::model::store::IdFile;

/// Runs the daemon: owns the name, serves until killed or the bus goes
/// away. Returns the exit status (3 when the name is owned).
pub fn serve(config_path: Option<&str>) -> i32 {
    let path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path);
    let config = match load_or_defaults(&path) {
        Ok(config) => config,
        Err(error) => {
            // A bad config at start runs on built-in defaults and
            // reports the error: an exit would restart-loop with no
            // notification daemon at all. (The popups ticket shows the
            // card saying so; headless, stderr is the report.)
            warn(format_args!(
                "scootnotify: {error}; running on built-in defaults"
            ));
            Config::default()
        }
    };
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
    let mut server = Server::new(config, ids);
    match run_loop(&mut conn, &mut server) {
        LoopEnd::BusGone => {
            warn(format_args!("scootnotify: the bus went away"));
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
}

/// Serves until the bus goes away: pumps method calls into the server,
/// arms one timerfd to the earliest expiry, fires what is due.
fn run_loop(conn: &mut Conn, server: &mut Server) -> LoopEnd {
    let timer = match timerfd() {
        Some(timer) => timer,
        None => return LoopEnd::BusGone,
    };
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
        server.fire_expiries(conn);
        arm_timer(&timer, server.earliest_deadline());
        if conn.dead() {
            return LoopEnd::BusGone;
        }
        if capped || conn.has_staged_work() || conn.want_write() {
            continue;
        }
        wait(conn.as_fd(), timer.as_fd());
        drain_timer(&timer);
        if conn.dead() {
            return LoopEnd::BusGone;
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

/// Polls the bus fd and the timerfd: woken by traffic or the next
/// expiry, never by time otherwise.
fn wait(bus: BorrowedFd<'_>, timer: BorrowedFd<'_>) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut fds = [
        PollFd::new(&bus, PollFlags::IN),
        PollFd::new(&timer, PollFlags::IN),
    ];
    let timeout = Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    let _ = poll(&mut fds, Some(&timeout));
}

/// Reads a fired timerfd back to disarmed: edge-triggered peace.
fn drain_timer(timer: &rustix::fd::OwnedFd) {
    use rustix::io::read;
    let mut buf = [0u8; 8];
    let _ = read(timer, &mut buf);
}
