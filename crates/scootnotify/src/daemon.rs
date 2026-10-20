//! The daemon: one `poll(2)` loop over the bus fd, one timerfd, the
//! popup's Wayland fd, and the control socket's listener and clients.
//!
//! With a compositor the daemon shows the card stack on an on-demand
//! overlay surface (clocks start on the frame callback; hover and input
//! idle pause them); without one it stays headless (arrival is display,
//! cards wait for no surface). At idle — no cards with deadlines, no
//! surface, nothing staged — the timerfd disarms, and the loop sleeps on
//! the bus fd, the control listener and the clients (the Wayland fd, which
//! speaks only for pointer, idle and output events) alone: 0 wakeups with
//! no cards. The control socket adds a poll timeout only while a stalled
//! peer's deadline or the listener's rest lapses, never at idle.
//!
//! Exit statuses: 0 done (only `--check` exits on its own; the daemon
//! runs until killed), 1 failed, 3 the name is owned.

use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_rt::print::warn;
use scootui::control::{Claim, Server as ControlServer};

use crate::bus::server::{self, Server};
use crate::config::{self, Config};
use crate::control::respond::{AppliedReload, Responder};
use crate::feed::Feed;
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
    // Resolved once: every reload reads the layer from here, and tests
    // hand the responder their own.
    let look_state = look::state_dir();
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
    // The control socket first, so a second daemon refuses before it
    // takes the name's work: one daemon per display, like the bus name.
    let control_paths = match scootui::control::paths::from_env("scootnotify") {
        Ok(paths) => paths,
        Err(error) => {
            warn(format_args!("scootnotify: control socket: {error}"));
            return 1;
        }
    };
    let claim = match Claim::acquire(&control_paths) {
        Ok(claim) => claim,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 1;
        }
    };
    let mut control = match ControlServer::new(claim.listener()) {
        Ok(server) => server,
        Err(error) => {
            warn(format_args!("scootnotify: control socket: {error}"));
            return 1;
        }
    };
    let ids = IdFile::open(ids_path());
    let mut server = Server::new(config.clone(), ids);
    // The bar feed baselines without sending: the startup state counts
    // as placed, so the first push waits for the first change (a bar
    // started later shows its placeholder until then).
    let mut feed = Feed::new(&config);
    feed.baseline(server.store());
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
    match run_loop(
        &mut conn,
        &mut server,
        popup,
        &mut control,
        claim.listener(),
        &path,
        look_state.as_ref(),
        &mut feed,
    ) {
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
/// into the server, serves the control clients (`list`, `dismiss`,
/// `invoke`, `reload`, `subscribe`, `version`), arms one timerfd to the
/// earliest expiry, fires what is due, broadcasts the turn's control
/// events, and draws the stack when dirty.
fn run_loop(
    conn: &mut Conn,
    server: &mut Server,
    mut popup: Option<Popup>,
    control: &mut ControlServer,
    listener: &UnixListener,
    config_path: &PathBuf,
    look_state: Option<&PathBuf>,
    feed: &mut Feed,
) -> LoopEnd {
    let timer = match timerfd() {
        Some(timer) => timer,
        None => return LoopEnd::BusGone,
    };
    let mut hovered: Option<u32> = None;
    // The control listener rests after a failed accept (see below), and
    // stalled control peers set the poll's timeout: both `None` at idle,
    // so the socket adds no wakeups.
    let mut rest_until: Option<Instant> = None;
    let mut stall_anchor: Option<Instant> = None;
    loop {
        let (events, capped) = conn.pump();
        if conn.dead() {
            return LoopEnd::BusGone;
        }
        let now = Instant::now();
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
        }
        let applied = serve_control(
            control,
            listener,
            server,
            conn,
            config_path,
            look_state,
            now,
            now_ms,
            &mut rest_until,
        );
        if let Some(applied) = applied {
            apply_reload(&mut popup, feed, applied);
        }
        // Rule `exec`s fire here: spawn what the turn queued (bus and
        // control alike), then reap what exited. A spawn refusal is a
        // line on the log, never a wedge.
        for request in server.take_execs() {
            if let Err(error) = server.spawn_exec(&request) {
                warn(format_args!("scootnotify: {error}"));
            }
        }
        server.reap_exec();
        if let Some(popup) = popup.as_mut() {
            popup.refresh(&server.store().ordered(), now_ms);
        }
        server.fire_expiries(conn);
        // The bar feed pushes on change only: arrivals, expiries,
        // dismissals, restores and DND toggles above all landed before
        // this, so the pushed count is the turn's.
        if let Some(line) = feed.sync(server.store()) {
            warn(format_args!("{line}"));
        }
        // The turn's control events, from the bus, the pointer and `msg`
        // alike, go to this turn's subscribers. Drained every turn, so a
        // flood cannot grow the queue past one turn's worth.
        for event in server.take_control_events() {
            control.broadcast(event.kind as u8, &event.line);
        }
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
            continue;
        }
        let stalled = control.sweep(now);
        if stalled > 0 {
            warn(format_args!(
                "scootnotify: dropped {stalled} control connection(s) stalled past the write deadline"
            ));
        }
        // The Wayland read guard: prepared before the poll, read when
        // the fd speaks, dropped unread otherwise (the documented
        // read-after-poll shape: the next turn collects it).
        let guard = popup.as_ref().and_then(|popup| popup.prepare_read());
        let (wayland, feed_watch) = poll_turn(
            conn.as_fd(),
            timer.as_fd(),
            popup.as_ref().map(|popup| popup.fd()),
            control,
            listener,
            rest_until,
            control.stall_timeout(&mut stall_anchor),
            server,
            feed,
        );
        drain_timer(&timer);
        if wayland && let Some(guard) = guard {
            let _ = guard.read();
        }
        // The bar's socket may have appeared: a send owed retries.
        if feed_watch
            && let Some(line) = feed.retry(server.store())
        {
            warn(format_args!("{line}"));
        }
        if conn.dead() {
            return LoopEnd::BusGone;
        }
    }
}

/// Serves the control socket for one turn: drains the listener (unless it
/// is resting after a failed accept), then serves every client. All
/// traffic is non-blocking, so every client is served optimistically: a
/// client with nothing to say costs one refused read. Returns the turn's
/// reload for the popup, if any.
#[allow(clippy::too_many_arguments)] // The loop's one control wiring: every serving input in one place.
fn serve_control(
    control: &mut ControlServer,
    listener: &UnixListener,
    server: &mut Server,
    bus: &mut Conn,
    config_path: &PathBuf,
    look_state: Option<&PathBuf>,
    now: Instant,
    now_ms: u64,
    rest_until: &mut Option<Instant>,
) -> Option<AppliedReload> {
    use rustix::event::PollFlags;
    if rest_until.is_some_and(|until| now < until) {
        // Resting: a failed accept is retried once the poll times out
        // (see `poll_turn`), not spun on.
    } else if let Err(error) = control.accept(listener) {
        warn(format_args!(
            "scootnotify: cannot accept control clients: {error}"
        ));
        *rest_until = Some(now + LISTEN_REST);
    } else {
        *rest_until = None;
    }
    let mut responder = Responder::new(
        server,
        bus,
        config_path,
        look_state.map(PathBuf::as_path),
        now,
        now_ms,
        control.subscribers(),
    );
    // Served optimistically (see above): readiness is assumed, and the
    // non-blocking reads refuse clients with nothing to say.
    let ready = PollFlags::IN | PollFlags::OUT | PollFlags::HUP | PollFlags::ERR;
    let mut index = 0;
    while index < control.conns().len() {
        if control.service(index, ready, &mut responder) {
            index += 1;
        }
    }
    responder.applied
}

/// Applies a reload's popup half: the new config lays out, the new look
/// re-themes, under the live cards. Infallible: the request half already
/// validated everything, so a refused file or fragment never reaches
/// here. The warnings go to the log, as at startup.
fn apply_reload(popup: &mut Option<Popup>, feed: &mut Feed, applied: AppliedReload) {
    for note in &applied.notes {
        warn(format_args!("scootnotify: note: {note}"));
    }
    if let Some(config) = applied.config.as_ref() {
        feed.set_icons(config);
    }
    if let Some(popup) = popup.as_mut() {
        if let Some(config) = applied.config {
            popup.reconfigure(config);
        }
        popup.retheme(applied.look);
    }
}

/// How long the control listener rests after a failed accept: retrying at
/// once would spin (the listener is level-triggered), exiting would take
/// the daemon with it.
const LISTEN_REST: Duration = Duration::from_secs(1);

/// Applies one popup event to the store, queueing its signals: the
/// token (if minted) always precedes its invoke.
fn apply_popup(
    conn: &mut Conn,
    server: &mut Server,
    event: popup::Event,
    hovered: &mut Option<u32>,
) {
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

/// Polls the bus fd, the timerfd, the Wayland fd (when there is one),
/// the bar feed's socket watch (only while a send is owed), the
/// control listener (unless it is resting after a failed accept)
/// and its clients, and the rule-exec pidfds, with a timeout only while
/// a stalled peer's deadline, an exec child without a pidfd, or the
/// listener's rest lapses. Returns whether the Wayland fd spoke, and
/// whether the feed's watch did. The feed never sets the timeout: an
/// owed send waits on the watch fd alone, so nothing wakes at idle.
/// A bounded poll here would be a wakeup every bound at idle; the loop
/// only reaches this wait with nothing staged, nothing queued and the
/// timer armed, so nothing is missed by sleeping until the fds speak (all
/// poll level-triggered).
#[allow(clippy::too_many_arguments)] // The loop's one poll wiring: every fd and timeout in one place.
fn poll_turn(
    bus: BorrowedFd<'_>,
    timer: BorrowedFd<'_>,
    wayland: Option<BorrowedFd<'_>>,
    control: &ControlServer,
    listener: &UnixListener,
    rest_until: Option<Instant>,
    stall: Option<Duration>,
    server: &Server,
    feed: &Feed,
) -> (bool, bool) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut fds = Vec::with_capacity(8 + control.conns().len());
    fds.push(PollFd::new(&bus, PollFlags::IN));
    fds.push(PollFd::new(&timer, PollFlags::IN));
    // Unwrapped beside the bus: the binding lives for the poll, and the
    // index records whether it is the Wayland fd or a second bus slot.
    let wayland_fd = wayland.unwrap_or(bus);
    let wayland_at = if wayland.is_some() {
        fds.push(PollFd::new(&wayland_fd, PollFlags::IN));
        Some(fds.len() - 1)
    } else {
        None
    };
    // Rule children that exited wake the loop through their pidfds, so
    // no zombie outlives its exit by more than a turn.
    let mut len = fds.len();
    fds.resize_with(fds.capacity(), || PollFd::new(&bus, PollFlags::empty()));
    server.exec_sources(&mut fds, &mut len);
    fds.truncate(len);
    // The listener rests after a failed accept: it keeps no slot, so a
    // connection it cannot accept does not keep the level-triggered poll
    // returning (no spin). The poll's timeout re-arms it.
    if rest_until.is_none_or(|until| Instant::now() >= until) {
        fds.push(PollFd::new(listener, PollFlags::IN));
    }
    for conn in control.conns() {
        fds.push(PollFd::new(conn.stream(), conn.interest()));
    }
    // The feed's socket watch, only while a send is owed: a bar
    // started later wakes this poll once, and an idle daemon holds no
    // watch at all.
    let feed_at = if let Some(watch) = feed.poll_fd() {
        fds.push(PollFd::new(&watch, PollFlags::IN));
        Some(fds.len() - 1)
    } else {
        None
    };
    let rest = rest_until.and_then(|until| until.checked_duration_since(Instant::now()));
    let timeout = match (stall, server.exec_poll_timeout(), rest) {
        (Some(left), exec, Some(rest)) => Some(left.min(exec.unwrap_or(rest)).min(rest)),
        (Some(left), exec, None) => Some(left.min(exec.unwrap_or(left))),
        (None, Some(exec), Some(rest)) => Some(exec.min(rest)),
        (None, Some(exec), None) => Some(exec),
        (None, None, Some(rest)) => Some(rest),
        (None, None, None) => None,
    };
    let timespec = timeout.map(|left| Timespec {
        tv_sec: left.as_secs() as _,
        tv_nsec: left.subsec_nanos() as _,
    });
    let _ = poll(&mut fds, timespec.as_ref());
    (
        wayland_at.is_some_and(|at| !fds[at].revents().is_empty()),
        feed_at.is_some_and(|at| !fds[at].revents().is_empty()),
    )
}

/// Reads a fired timerfd back to disarmed: edge-triggered peace.
fn drain_timer(timer: &rustix::fd::OwnedFd) {
    use rustix::io::read;
    let mut buf = [0u8; 8];
    let _ = read(timer, &mut buf);
}
