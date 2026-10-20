//! The shared serving probe for the scootnotify integration tests:
//! pumps a bus client until its `GetServerInformation` probe is answered.
//!
//! The probe deliberately makes no card. An earlier shape probed with
//! a 1 ms `Notify`, but under parallel-startup contention the session
//! bus sometimes consumes a probe sent to an already-owned name
//! without ever forwarding or erroring it (measured on the dev VM:
//! the client flushed — `want_write` false over 5000+ pumps — the
//! kernel queues drained, the daemon owned the name and served
//! independent clients while the probe never reached its handler), and
//! any `Notify` resend for it risks a second live card landing inside
//! a test's exact live-card count (or a missing history entry breaking
//! its exact history count). `GetServerInformation` is answered by the
//! same daemon on the same name — proving the same serving — with no
//! store footprint at all, so the probe is invisible to every count in
//! every suite, green runs and rescued runs alike.
//!
//! Two resends, for two different silences. An error (the name unowned
//! yet) resends at once: the daemon simply got there later than the
//! probe. Silence with no error resends after [`PROBE_RESEND_AFTER`]:
//! the bus ate the probe, so ask again. A single all-or-nothing probe
//! would turn that one lost message into a 30 s failure; the resends
//! ride through it, and the suites' own `Notify` calls (with their
//! loud timeouts) still exercise the notification path seconds later.
//!
//! On timeout the panic names the daemon's state (its `/proc` wchan,
//! syscall, stack, fd count and CPU time on Linux, best-effort) next
//! to its stderr, so the next occurrence is self-diagnosing instead of
//! costing another debugging round.
//!
//! Shared across four test crates (`bus_tests`, `control_tests`,
//! `popups`, `probe_resend`), each using a subset: dead-code warnings
//! are off for the module, since every item serves at least one suite.
#![allow(dead_code)]

use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use scoot_dbus::conn::{Conn, Event};

/// The probed name, object and interface: the spec's §2 server,
/// identical in every suite.
const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

/// How long the probe waits for the daemon: thirty seconds, not ten —
/// the suite runs a dozen servers in parallel on a shared six-core
/// box, and a contended spawn plus exec can take seconds. Unloaded it
/// answers in a third of one.
pub const SERVING_BUDGET: Duration = Duration::from_secs(30);

/// How long of unanswered, error-free silence before the probe is
/// resent (see the module docs): long past any healthy round trip,
/// short against the budget.
pub const PROBE_RESEND_AFTER: Duration = Duration::from_secs(1);

/// Where the daemon's stderr lives for the timeout report: the tests
/// that pipe it drain the pipe, the popups tests read their log file.
#[derive(Clone, Copy)]
pub enum LogSource<'a> {
    /// Drain `server.stderr` (taken on failure).
    Pipe,
    /// Read this log file.
    File(&'a Path),
}

/// Pumps `client` until its serving probe is answered: the daemon
/// serving. The probe's token is 9000 in every suite: one call
/// outstanding at a time — an error (the name unowned yet) resends at
/// once, silence past [`PROBE_RESEND_AFTER`] resends too (see the
/// module docs) — and no probe ever touches the store, so the suites'
/// live-card and history counts hold exactly in green runs and rescued
/// runs alike. On timeout the panic names the server's fate (its exit
/// code and stderr when it died, which is the startup crash; its
/// `/proc` state and what its stderr says so far when it still runs,
/// which is how far startup got and where it sits).
pub fn wait_serving(client: &mut Conn, server: &mut Child, log: LogSource<'_>) {
    wait_serving_budget(client, server, log, SERVING_BUDGET, PROBE_RESEND_AFTER);
}

/// As [`wait_serving`], with the budget and the silence resend after
/// it: the e2e suites use the defaults, the resend pin uses a short
/// budget against a scripted bus.
pub fn wait_serving_budget(
    client: &mut Conn,
    server: &mut Child,
    log: LogSource<'_>,
    budget: Duration,
    resend_after: Duration,
) {
    send_probe(client);
    let start = Instant::now();
    let mut last_sent = start;
    // How the wait ends, for the timeout panic: errors mean the name
    // was genuinely unowned (the daemon never got there); silence with
    // no errors at all means the bus never answered either way.
    let mut errored_total = 0u64;
    let mut events_total = 0u64;
    let mut probes_sent = 1u64;
    loop {
        if start.elapsed() >= budget {
            let state = describe_daemon(server);
            match server.try_wait().expect("the server reaps") {
                Some(status) => {
                    panic!(
                        "the server died at startup ({status}): {}",
                        read_log(log, server)
                    );
                }
                None => {
                    panic!(
                        "the server still runs but never owned the name in {budget:?} \
                         ({events_total} events, {errored_total} name-unowned errors, \
                         {probes_sent} probes sent); daemon {state}; \
                         its stderr so far: {}",
                        read_log(log, server)
                    );
                }
            }
        }
        // A dead-at-startup server fails here in milliseconds, not at
        // the budget deadline: otherwise a startup crash reads exactly
        // like a serving stall.
        if let Some(status) = server.try_wait().expect("the server reaps") {
            panic!(
                "the server died at startup ({status}): {}",
                read_log(log, server)
            );
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
        if errored || last_sent.elapsed() >= resend_after {
            // Unowned yet, or silent while owned (the probe was lost
            // below the daemon — see the module docs): ask again. The
            // probe makes no card, so a merely-delayed first probe can
            // never disturb a count. Green runs never reach the
            // silence half of this.
            if errored {
                errored_total += 1;
            }
            send_probe(client);
            probes_sent += 1;
            last_sent = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Sends (or resends) the serving probe: `GetServerInformation` on
/// the notifications name. Answered by the daemon once it owns the
/// name and pumps, with no store footprint.
fn send_probe(client: &mut Conn) {
    client
        .call(
            BUS_NAME,
            OBJECT,
            INTERFACE,
            "GetServerInformation",
            "",
            &[],
            0,
            9000,
        )
        .expect("the probe queues");
}

/// Reads the daemon's stderr for the timeout report: drains the pipe,
/// or reads the log file.
fn read_log(log: LogSource<'_>, server: &mut Child) -> String {
    match log {
        LogSource::Pipe => read_stderr_pipe(server.stderr.as_mut()),
        LogSource::File(path) => std::fs::read_to_string(path).unwrap_or_default(),
    }
}

/// Reads what a live daemon's stderr holds without blocking: the
/// daemon is still alive, so EOF never comes. Best effort (a failure
/// to drain says so); the pipe is left nonblocking, which is fine on
/// a path that panics right after.
fn read_stderr_pipe(stderr: Option<&mut std::process::ChildStderr>) -> String {
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

/// Where the still-running daemon sits, for the timeout report: its
/// kernel wait channel, current syscall, stack, scheduling counts and
/// open fd count on Linux (best-effort — any failure degrades to a
/// placeholder, never a second panic on the failure path); a platform
/// placeholder elsewhere.
fn describe_daemon(server: &Child) -> String {
    describe_pid(server.id())
}

#[cfg(target_os = "linux")]
fn describe_pid(pid: u32) -> String {
    let base = format!("/proc/{pid}");
    let one = |name: &str| {
        std::fs::read_to_string(format!("{base}/{name}"))
            .map(|text| text.trim().replace('\n', "|"))
            .unwrap_or_else(|_| String::from("(unavailable)"))
    };
    let state = std::fs::read_to_string(format!("{base}/status"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("State:"))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from("State: (unavailable)"));
    let fds = std::fs::read_dir(format!("{base}/fd")).map(|entries| entries.count());
    format!(
        "pid={pid} {} wchan={} syscall={} stack={} schedstat={} fds={}",
        state,
        one("wchan"),
        one("syscall"),
        one("stack"),
        one("schedstat"),
        fds.map(|count| count.to_string())
            .unwrap_or_else(|_| String::from("(unavailable)")),
    )
}

#[cfg(not(target_os = "linux"))]
fn describe_pid(pid: u32) -> String {
    format!("pid={pid} (daemon state unavailable on this platform)")
}
