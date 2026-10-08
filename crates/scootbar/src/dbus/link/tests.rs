//! The link against a real `dbus-daemon`, and against a bus that takes the
//! bar in and drops it: waiting without a bus, connecting when one
//! appears, healing a death, the quick-death latch, and bounded work per
//! wake.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use super::{Addr, Link, Session, scan_names};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::proto::{self, Writer};
use crate::dbus::testdaemon::{Daemon, serve_setup};

/// What a consumer is to the link: a connection that asked for one
/// interface's signals.
struct Probe {
    conn: Conn,
}

impl Session for Probe {
    fn conn(&self) -> &Conn {
        &self.conn
    }

    fn conn_mut(&mut self) -> &mut Conn {
        &mut self.conn
    }
}

/// The token of the call whose reply says the match rule is in place.
const MATCHED: u64 = 7;

fn probe(mut conn: Conn) -> Probe {
    let mut body = Writer::new();
    body.str("type='signal',interface='sh.scoot.Probe'");
    let _ = conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        "AddMatch",
        "s",
        &body.take_body().unwrap(),
        proto::flag::NO_REPLY_EXPECTED,
        0,
    );
    // The bus works one connection's calls in order: this one's answer
    // says the match above was added, so a signal sent after it is not
    // sent into a race with it.
    let mut body = Writer::new();
    body.str(conn::BUS_NAME);
    let _ = conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        "GetNameOwner",
        "s",
        &body.take_body().unwrap(),
        0,
        MATCHED,
    );
    Probe { conn }
}

/// Turns until the probe's match rule is in place.
fn matched(link: &mut Link<Probe>) {
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the match never settled"
        );
        if let Some((_, events)) = turn(link, Duration::from_millis(50)) {
            if events
                .iter()
                .any(|event| matches!(event, Event::Reply { token: MATCHED, .. }))
            {
                return;
            }
        }
    }
}

fn scratch(tag: &str) -> PathBuf {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "scootbar-link-{tag}-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A unique abstract bus name: the abstract namespace is host-global,
/// and other agents may test on the same machine at once.
fn abstract_name(tag: &str) -> String {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "scootbar-link-{tag}-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

fn fds(link: &Link<Probe>) -> usize {
    let mut count = 0;
    link.watch(&mut |_, _| count += 1);
    count
}

/// One turn the way the loop takes it: polls the link's fds for up to
/// `timeout` and hands each ready one over. `None` when nothing was ready;
/// else whether a call to `work` or a drop changed anything, and the
/// events worked.
fn turn(link: &mut Link<Probe>, timeout: Duration) -> Option<(bool, Vec<Event>)> {
    let ready: Vec<(usize, PollFlags)> = {
        let mut pollfds = Vec::new();
        link.watch(&mut |fd, flags| pollfds.push(PollFd::from_borrowed_fd(fd, flags)));
        let timeout = Timespec {
            tv_sec: timeout.as_secs() as i64,
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        poll(&mut pollfds, Some(&timeout)).unwrap();
        pollfds
            .iter()
            .enumerate()
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|(source, fd)| (source, fd.revents()))
            .collect()
    };
    if ready.is_empty() {
        return None;
    }
    let mut seen = Vec::new();
    let mut changed = false;
    for (source, events) in ready {
        changed |= link.on_ready(source, events, &mut |_, event| {
            seen.push(event);
            true
        });
    }
    Some((changed, seen))
}

/// Turns until `done` holds (ten seconds at most).
fn until(link: &mut Link<Probe>, mut done: impl FnMut(&Link<Probe>) -> bool) {
    let start = Instant::now();
    while !done(link) {
        assert!(start.elapsed() < Duration::from_secs(10), "never got there");
        let _ = turn(link, Duration::from_millis(50));
    }
}

#[test]
fn an_address_that_names_no_path_is_never_dialled_and_holds_nothing() {
    let link: Link<Probe> = Link::start("test", Addr::Unusable, probe);
    assert!(link.live().is_none());
    assert_eq!(fds(&link), 0);
    assert_eq!(link.source_count(), 0);
}

#[test]
fn with_no_bus_it_waits_on_the_directory_and_connects_when_the_socket_appears() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let dir = scratch("appear");
    let path = dir.join("bus");
    let mut link = Link::start("test", Addr::Path(path.clone()), probe);
    assert!(link.live().is_none());
    assert_eq!(fds(&link), 1, "the watch, nothing else");
    assert_eq!(link.source_count(), 1);
    // Quiet: nothing wakes it.
    assert!(turn(&mut link, Duration::from_millis(80)).is_none());
    // Other names appearing in the directory wake it and dial nothing.
    std::fs::write(dir.join("other"), b"").unwrap();
    let _ = turn(&mut link, Duration::from_millis(200));
    assert!(link.live().is_none());
    // The socket's name appears: it connects.
    std::os::unix::fs::symlink(daemon.path(), &path).unwrap();
    until(&mut link, |link| link.live().is_some());
    assert_eq!(fds(&link), 1, "the bus socket");
    let unique = link.live().unwrap().conn().unique().to_owned();
    assert!(unique.starts_with(":1."), "{unique}");
    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bus_that_goes_away_drops_the_session_and_comes_back_when_it_does() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let dir = scratch("heal");
    let path = dir.join("bus");
    std::os::unix::fs::symlink(daemon.path(), &path).unwrap();
    let mut link = Link::start("test", Addr::Path(path.clone()), probe);
    assert!(link.live().is_some(), "dials at once when the bus is there");
    drop(daemon);
    // The death is a change (what the session held is gone) and leaves
    // the link waiting, not spinning.
    let mut changed = false;
    until(&mut link, |link| link.live().is_none());
    assert!(
        turn(&mut link, Duration::from_millis(80)).is_none(),
        "quiet while waiting"
    );
    changed |= true;
    assert!(changed);
    assert_eq!(fds(&link), 1);
    // A new bus at the same name: connected again.
    let Some(second) = Daemon::spawn() else {
        return;
    };
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(second.path(), &path).unwrap();
    until(&mut link, |link| link.live().is_some());
    drop(second);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_abstract_bus_is_dialled_at_once_and_heals_after_a_restart() {
    let name = abstract_name("heal");
    let Some(daemon) = Daemon::spawn_abstract_on(&name) else {
        return;
    };
    let mut link = Link::start("test", Addr::Abstract(name.as_bytes().to_vec()), probe);
    assert!(link.live().is_some(), "dials at once when the bus is there");
    let unique = link.live().unwrap().conn().unique().to_owned();
    assert!(unique.starts_with(":1."), "{unique}");
    drop(daemon);
    until(&mut link, |link| link.live().is_none());
    // Waiting on the retry timer alone: one source, no directory watch.
    assert_eq!(fds(&link), 1, "the retry timer, nothing else");
    assert_eq!(link.source_count(), 1);
    // A new bus at the same name: the poll connects again.
    let Some(second) = Daemon::spawn_abstract_on(&name) else {
        return;
    };
    until(&mut link, |link| link.live().is_some());
    drop(second);
}

#[test]
fn with_no_abstract_bus_it_polls_the_name_on_the_retry_timer() {
    let name = abstract_name("absent");
    let mut link = Link::start("test", Addr::Abstract(name.as_bytes().to_vec()), probe);
    assert!(link.live().is_none());
    assert_eq!(fds(&link), 1, "the retry timer, nothing else");
    assert_eq!(link.source_count(), 1);
    // The timer fires and dials again: still nothing, still waiting (one
    // failed dial a retry, not a spin).
    let start = Instant::now();
    let mut fired = false;
    while start.elapsed() < Duration::from_secs(5) {
        if turn(&mut link, Duration::from_millis(100)).is_some() {
            fired = true;
            break;
        }
    }
    assert!(fired, "the retry timer fires while the bus is absent");
    assert!(link.live().is_none());
    assert_eq!(fds(&link), 1);
    // The bus appears later at the same name: the poll connects.
    let Some(daemon) = Daemon::spawn_abstract_on(&name) else {
        return;
    };
    until(&mut link, |link| link.live().is_some());
    drop(daemon);
}

#[test]
fn a_signal_is_worked_through_the_closure_and_reports_a_change() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut link = Link::start("test", Addr::Path(daemon.path()), probe);
    assert!(link.live().is_some());
    let mut sender = conn::connect(&daemon.path()).unwrap();
    matched(&mut link);
    sender.signal("/p", "sh.scoot.Probe", "Ping", "", &[]);
    let _ = sender.pump();
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(10), "no signal");
        if let Some((changed, events)) = turn(&mut link, Duration::from_millis(50)) {
            if events
                .iter()
                .any(|e| matches!(e, Event::Signal { member, .. } if member == "Ping"))
            {
                assert!(changed);
                break;
            }
        }
        let _ = sender.pump();
    }
}

#[test]
fn a_flood_is_worked_a_wake_at_a_time_and_loses_nothing() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut link = Link::start("test", Addr::Path(daemon.path()), probe);
    let mut sender = conn::connect(&daemon.path()).unwrap();
    matched(&mut link);
    const SENT: usize = 3000;
    for _ in 0..SENT {
        sender.signal("/p", "sh.scoot.Probe", "Ping", "", &[]);
    }
    let mut got = 0;
    let mut most_in_a_wake = 0;
    let start = Instant::now();
    while got < SENT {
        assert!(start.elapsed() < Duration::from_secs(20), "{got} of {SENT}");
        let _ = sender.pump();
        if let Some((_, events)) = turn(&mut link, Duration::from_millis(20)) {
            most_in_a_wake = most_in_a_wake.max(events.len());
            got += events
                .iter()
                .filter(|e| matches!(e, Event::Signal { member, .. } if member == "Ping"))
                .count();
        }
    }
    assert_eq!(got, SENT);
    assert!(
        most_in_a_wake <= conn::MAX_EVENTS_PER_TURN * super::MAX_PUMPS_PER_WAKE,
        "{most_in_a_wake} events in one wake"
    );
}

/// A bus that takes the bar in and drops it at once: authenticates, says
/// `Hello`, hangs up. Counts the connections it took.
fn quick_death_bus(path: &Path) -> Arc<AtomicUsize> {
    let listener = UnixListener::bind(path).unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            serve_setup(&mut stream);
            count.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    accepted
}

#[test]
fn a_bus_that_keeps_dropping_the_bar_is_left_alone_and_tried_once_more_later() {
    let dir = scratch("latch");
    let path = dir.join("bus");
    let accepted = quick_death_bus(&path);
    let mut link = Link::start("test", Addr::Path(path.clone()), probe);
    // Three deaths, each redialled at once, the third not.
    let start = Instant::now();
    while link.live().is_some() || accepted.load(Ordering::SeqCst) < 3 {
        assert!(start.elapsed() < Duration::from_secs(10), "never latched");
        let _ = turn(&mut link, Duration::from_millis(20));
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
    assert!(link.live().is_none());
    // Waiting, with the retry timer armed beside the directory watch: it
    // does not hot-loop.
    assert_eq!(fds(&link), 2, "the watch and the retry timer");
    let before = accepted.load(Ordering::SeqCst);
    assert!(turn(&mut link, Duration::from_millis(100)).is_none());
    assert_eq!(accepted.load(Ordering::SeqCst), before);
    // The retry fires after its delay: one more dial.
    let start = Instant::now();
    while accepted.load(Ordering::SeqCst) == before {
        assert!(start.elapsed() < Duration::from_secs(10), "no retry");
        let _ = turn(&mut link, Duration::from_millis(50));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_watch_scan_matches_whole_names_and_probes_when_unsure() {
    fn event(name: &[u8]) -> Vec<u8> {
        let mut padded = name.to_vec();
        padded.push(0);
        while !padded.len().is_multiple_of(8) {
            padded.push(0);
        }
        let mut out = vec![0u8; 12];
        out.extend_from_slice(&(padded.len() as u32).to_ne_bytes());
        out.extend_from_slice(&padded);
        out
    }
    assert!(scan_names(&event(b"bus"), b"bus"));
    assert!(!scan_names(&event(b"bus2"), b"bus"));
    assert!(!scan_names(&event(b"bu"), b"bus"));
    assert!(!scan_names(&[], b"bus"));
    let mut two = event(b"x");
    two.extend_from_slice(&event(b"bus"));
    assert!(scan_names(&two, b"bus"));
    // A buffer that does not frame: probe, since missing the bus is worse.
    assert!(scan_names(&event(b"bus")[..10], b"bus"));
    assert!(scan_names(&event(b"bus")[..18], b"bus"));
}
