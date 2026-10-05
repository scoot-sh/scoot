//! One background layer surface per output, against a real
//! `scoot --headless`: what `query` reports, what scoot reports, and the
//! protocol requests that made each surface.

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::{Session, wait_exit};
use serde_json::{Value, json};

/// `{"width":W,"height":H}`, as both `query` and scoot's IPC write sizes.
fn size_of(rect: &Value) -> Value {
    json!({"width": rect["width"], "height": rect["height"]})
}

fn all_configured(outputs: &[Value], count: usize) -> bool {
    outputs.len() == count
        && outputs
            .iter()
            .all(|o| o["surface"]["state"] == "configured" && o["surface"]["size"].is_object())
}

/// The `WAYLAND_DEBUG=client` trace, read into what the test checks. The
/// format is `wayland-backend`'s (`[rs] -> iface@id.request(args)`, `[rs] <-`
/// for events); if a new version changes it, the parse finds nothing and
/// the test fails saying so, rather than pass on an empty trace.
#[derive(Debug, Default)]
struct Trace {
    /// Per layer surface: (its wl_surface, the wl_output it was asked for,
    /// the layer, the namespace).
    layers: HashMap<String, (String, String, String, String)>,
    /// Requests sent to each object, in order, without arguments.
    requests: HashMap<String, Vec<String>>,
    /// Arguments of `set_*` requests per object: "request(args)".
    calls: HashMap<String, Vec<String>>,
    /// `wl_surface.enter` events: surface -> outputs.
    entered: HashMap<String, Vec<String>>,
    /// `wl_surface.set_input_region`: surface -> region.
    input_region: HashMap<String, String>,
}

fn parse_trace(text: &str) -> Trace {
    let mut trace = Trace::default();
    for line in text.lines() {
        let Some((_, message)) = line.split_once("] ") else {
            continue;
        };
        let (sent, message) = if let Some(m) = message.strip_prefix("-> ") {
            (true, m)
        } else if let Some(m) = message.strip_prefix("<- ") {
            (false, m)
        } else {
            continue;
        };
        let Some((target, rest)) = message.split_once('.') else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // Requests print `name(args)`, events `name, (args)`.
        let tail = rest.get(name.len()..).unwrap_or("");
        let tail = tail.trim_start_matches([',', ' ']);
        let args = tail
            .strip_prefix('(')
            .and_then(|t| t.strip_suffix(')'))
            .unwrap_or(tail)
            .to_owned();
        let parts: Vec<String> = args.split(", ").map(str::to_owned).collect();
        if sent {
            trace
                .requests
                .entry(target.to_owned())
                .or_default()
                .push(name.clone());
            trace
                .calls
                .entry(target.to_owned())
                .or_default()
                .push(format!("{name}({args})"));
            if name == "get_layer_surface" && parts.len() == 5 {
                trace.layers.insert(
                    parts[0].clone(),
                    (
                        parts[1].clone(),
                        parts[2].clone(),
                        parts[3].clone(),
                        parts[4].clone(),
                    ),
                );
            }
            if name == "set_input_region" {
                trace
                    .input_region
                    .insert(target.to_owned(), parts[0].clone());
            }
        } else if name == "enter" && target.starts_with("wl_surface@") {
            trace
                .entered
                .entry(target.to_owned())
                .or_default()
                .push(parts[0].clone());
        }
    }
    trace
}

/// Waits until the trace file holds `count` configured layer surfaces'
/// acks (the query already says so; the trace is written by the same
/// thread before the reply, but read it until it agrees anyway).
fn read_trace(session: &Session, count: usize) -> Trace {
    let deadline = Instant::now() + common::PATIENCE;
    loop {
        let text = std::fs::read_to_string(session.daemon_log()).unwrap();
        let trace = parse_trace(&text);
        let acked = trace
            .requests
            .iter()
            .filter(|(object, requests)| {
                object.starts_with("zwlr_layer_surface_v1@")
                    && requests.iter().any(|r| r == "ack_configure")
            })
            .count();
        if acked >= count && trace.entered.len() >= count {
            return trace;
        }
        assert!(
            Instant::now() < deadline,
            "the trace never showed {count} acked surfaces (has the WAYLAND_DEBUG \
             format changed?):\n{text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// On 1 and 2 outputs: `query` shows every output with a configured
/// surface of the output's size, scoot's `usable` area is untouched
/// (exclusive zone -1 reserves nothing), and the trace shows each surface
/// made as specified and placed by scoot on the output it was made for.
#[test]
fn each_output_gets_one_configured_background_surface() {
    for count in [1, 2] {
        let Some(session) = Session::start_with(&format!("each{count}"), count, "") else {
            return;
        };
        let scoot = session.scoot_ipc(r#"{"type":"outputs"}"#);
        let scoot_outputs = scoot["outputs"].as_array().unwrap().clone();
        assert_eq!(scoot_outputs.len(), count as usize, "{scoot}");

        let mut daemon = session.daemon_logged(&[("WAYLAND_DEBUG", "client")]);
        let outputs = session.query_until("all configured", |o| all_configured(o, count as usize));
        for (ours, theirs) in outputs.iter().zip(&scoot_outputs) {
            assert_eq!(ours["name"], theirs["name"], "{ours} vs {theirs}");
            let rect = size_of(&theirs["rect"]);
            assert_eq!(ours["surface"]["size"], rect, "{ours}");
            assert_eq!(ours["logical"], rect, "{ours}");
            assert_eq!(ours["mode"], rect, "scale 1: {ours}");
            assert_eq!(ours["scale"], 1);
            assert_eq!(ours["transform"], "normal");
            assert!(ours["description"].is_string(), "{ours}");
            assert_eq!(ours["shows"], Value::Null, "no color set: nothing shown");
        }
        // Background layer, exclusive zone -1: nothing reserved.
        let scoot = session.scoot_ipc(r#"{"type":"outputs"}"#);
        for output in scoot["outputs"].as_array().unwrap() {
            assert_eq!(output["usable"], output["rect"], "{output}");
        }

        let trace = read_trace(&session, count as usize);
        assert_eq!(trace.layers.len(), count as usize, "{trace:?}");
        let mut outputs_used = Vec::new();
        for (layer, (surface, output, level, namespace)) in &trace.layers {
            assert_eq!(level, "0", "the background layer");
            assert_eq!(namespace, "Some(\"wallpaper\")");
            let calls = &trace.calls[layer];
            for expected in [
                "set_anchor(15)",
                "set_size(0, 0)",
                "set_exclusive_zone(-1)",
                "set_keyboard_interactivity(0)",
            ] {
                assert!(calls.iter().any(|c| c == expected), "{layer}: {calls:?}");
            }
            // An empty input region: a region that was never added to.
            let region = &trace.input_region[surface];
            let region_requests = &trace.requests[region];
            assert_eq!(region_requests, &["destroy"], "{region}");
            // The compositor put it on the output it was made for.
            assert_eq!(
                trace.entered[surface],
                std::slice::from_ref(output),
                "{surface}"
            );
            // No color set: nothing attached.
            assert!(
                !trace.requests[surface].iter().any(|r| r == "attach"),
                "{surface}: {:?}",
                trace.requests[surface]
            );
            outputs_used.push(output.clone());
        }
        outputs_used.sort();
        outputs_used.dedup();
        assert_eq!(outputs_used.len(), count as usize, "one surface per output");

        assert!(session.run(&["kill"]).status.success());
        assert!(wait_exit(&mut daemon).success());
    }
}

/// A fractional scale: `wl_output` says 2 (1.5 rounded up), so a size
/// derived from it would be 800×500. The surface gets the compositor's own
/// size, 1067×667, and so does `logical`. A live change to 2.0 through
/// `scoot msg reload` reconfigures it to 800×500.
#[test]
fn a_fractional_scale_takes_the_compositors_size_and_follows_changes() {
    let Some(session) = Session::start_with("frac", 1, "[output]\nscale = 1.5\n") else {
        return;
    };
    let mut daemon = session.daemon();
    let outputs = session.query_until("configured at 1.5", |o| all_configured(o, 1));
    let output = &outputs[0];
    assert_eq!(output["scale"], 2);
    assert_eq!(output["mode"], json!({"width": 1600, "height": 1000}));
    let expected = json!({"width": 1067, "height": 667});
    assert_eq!(output["surface"]["size"], expected, "{output}");
    assert_eq!(output["logical"], expected, "{output}");
    let scoot = session.scoot_ipc(r#"{"type":"outputs"}"#);
    assert_eq!(size_of(&scoot["outputs"][0]["rect"]), expected, "{scoot}");

    std::fs::write(
        session.runtime_dir().join("config.toml"),
        "[output]\nscale = 2.0\n",
    )
    .unwrap();
    let reloaded = session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reloaded["type"], "reloaded", "{reloaded}");
    let half = json!({"width": 800, "height": 500});
    session.query_until("reconfigured at 2.0", |o| {
        o.len() == 1 && o[0]["surface"]["size"] == half && o[0]["logical"] == half
    });

    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}

/// The daemon's user plus system CPU ticks.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap()
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

fn soft_nofile(pid: u32, soft: &str) {
    let out = std::process::Command::new("prlimit")
        .args(["--pid", &pid.to_string(), &format!("--nofile={soft}:")])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "prlimit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The inodes of the sockets listening on `path`, from `/proc/net/unix`
/// (columns: `Num RefCount Protocol Flags Type St Inode Path`). Only
/// listeners: a connection accepted on a socket shows its path too, and
/// `__SO_ACCEPTCON` (0x10000) in `Flags` marks the listening one.
fn listening_inodes(path: &std::path::Path) -> Vec<String> {
    const ACCEPTCON: u32 = 0x1_0000;
    let table = std::fs::read_to_string("/proc/net/unix").unwrap();
    let path = path.to_str().unwrap();
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            // Seven fixed columns, then the path, which may hold spaces
            // (a `TMPDIR` with one): everything after the seventh.
            let mut columns = Vec::with_capacity(7);
            let mut rest = line;
            for _ in 0..7 {
                rest = rest.trim_start();
                let end = rest.find(char::is_whitespace)?;
                columns.push(&rest[..end]);
                rest = &rest[end..];
            }
            let flags = u32::from_str_radix(columns[3], 16).ok()?;
            let listening = flags & ACCEPTCON != 0;
            (listening && rest.trim() == path).then(|| columns[6].to_owned())
        })
        .collect()
}

/// The daemon's fd limit at which no new fd can exist and closing its
/// spare frees nothing usable: every number below it is taken, and the
/// listener and its spare are at or above it.
///
/// `RLIMIT_NOFILE` bounds fd *numbers* (a new fd takes the lowest free
/// number, only if below the limit), so that is the lowest free number or
/// the lower of the pair, whichever is less. Not 0: `poll` refuses more fds
/// than the limit with `EINVAL`, and the daemon polls two (Wayland and the
/// listener) with no clients, so the limit must stay at least that.
///
/// The pair is found by what it is, not by shape: the socket listening on
/// the daemon's path (`/proc/net/unix`), then the daemon's fds on that
/// inode. "The only socket open twice" broke on an inherited socket that
/// was itself open twice.
fn limit_past_the_spare(pid: u32, socket: &std::path::Path) -> u64 {
    let mut fds: Vec<(u64, String)> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let fd = entry.file_name().to_str()?.parse().ok()?;
            let target = std::fs::read_link(entry.path()).ok()?;
            Some((fd, target.to_string_lossy().into_owned()))
        })
        .collect();
    fds.sort();
    let lowest_free = (0..)
        .find(|n| fds.binary_search_by_key(n, |(fd, _)| *fd).is_err())
        .unwrap();
    let listeners: Vec<String> = listening_inodes(socket)
        .iter()
        .map(|inode| format!("socket:[{inode}]"))
        .collect();
    let pair: Vec<u64> = fds
        .iter()
        .filter(|(_, target)| listeners.contains(target))
        .map(|(fd, _)| *fd)
        .collect();
    assert_eq!(
        pair.len(),
        2,
        "not exactly a listener and its spare on {socket:?} (listening inodes \
         {listeners:?}) among {fds:?}"
    );
    let limit = lowest_free.min(pair[0]);
    assert!(limit >= 2, "limit {limit} below the poll set: {fds:?}");
    limit
}

fn current_soft_nofile(pid: u32) -> String {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let line = limits
        .lines()
        .find(|l| l.starts_with("Max open files"))
        .unwrap();
    line.split_whitespace().nth(3).unwrap().to_owned()
}

/// An `accept` that fails with nothing left to free no longer ends the
/// daemon (it would take the wallpaper with it). With the daemon's soft fd
/// limit set so no new fd can exist, whatever it closes (see
/// `limit_past_the_spare`): the listener rests,
/// the daemon neither spins nor exits, its surfaces stay configured, and
/// once the limit is back a waiting client is answered within about the
/// one-second rest. The failure and the recovery are each reported once.
///
/// Needs util-linux `prlimit`: skipped without it, a failure under
/// `SCOOTBG_REQUIRE_SCOOT`. Only the soft limit is changed, so no
/// privilege is needed to put it back.
#[test]
fn an_accept_that_cannot_succeed_rests_the_listener_and_the_wallpaper_stays() {
    let Some(session) = Session::start_with("rest", 1, "") else {
        return;
    };
    if std::process::Command::new("prlimit")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(
            std::env::var_os("SCOOTBG_REQUIRE_SCOOT").is_none(),
            "SCOOTBG_REQUIRE_SCOOT is set but there is no prlimit (util-linux) on PATH"
        );
        eprintln!("skipped -- no prlimit on PATH");
        return;
    }
    let mut daemon = session.daemon_logged(&[]);
    let pid = daemon.id();
    session.query_until("configured", |o| all_configured(o, 1));
    let original = current_soft_nofile(pid);

    // A steady state first: the start-up probe's connection is closed.
    let deadline = Instant::now() + common::PATIENCE;
    let mut limit = limit_past_the_spare(pid, &session.socket());
    let mut stable = 0;
    while stable < 3 {
        assert!(Instant::now() < deadline, "fds never settled");
        std::thread::sleep(Duration::from_millis(100));
        let again = limit_past_the_spare(pid, &session.socket());
        stable = if again == limit { stable + 1 } else { 0 };
        limit = again;
    }
    soft_nofile(pid, &limit.to_string());
    // A client arrives: the accept fails, the spare is spent, and there is
    // nothing more to free. It waits in the backlog.
    let waiting = session
        .scootbg()
        .arg("query")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // The failure is reported once.
    let deadline = Instant::now() + common::PATIENCE;
    let log = || std::fs::read_to_string(session.daemon_log()).unwrap();
    while !log().contains("cannot accept clients") {
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "the daemon exited: {}",
            log()
        );
        assert!(Instant::now() < deadline, "never reported: {}", log());
        std::thread::sleep(Duration::from_millis(20));
    }
    // Resting: alive, and not spinning. A spin would burn a CPU tick every
    // 10 ms; three rest periods allow a handful of wakeups, no more.
    let before = cpu_ticks(pid);
    std::thread::sleep(Duration::from_millis(3000));
    let used = cpu_ticks(pid) - before;
    assert!(
        used <= 2,
        "{used} ticks in 3 s while resting: it is spinning"
    );
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon exited");
    assert_eq!(
        log().matches("cannot accept clients").count(),
        1,
        "reported more than once: {}",
        log()
    );

    // The limit comes back: the waiting client is answered, with the
    // surface still configured.
    let restored = Instant::now();
    soft_nofile(pid, &original);
    let reply = waiting.wait_with_output().unwrap();
    assert!(
        reply.status.success(),
        "{}",
        String::from_utf8_lossy(&reply.stderr)
    );
    assert!(
        restored.elapsed() < Duration::from_secs(5),
        "answered only after {:?}",
        restored.elapsed()
    );
    let reply: Value = serde_json::from_slice(&reply.stdout).unwrap();
    assert!(
        all_configured(reply["outputs"].as_array().unwrap(), 1),
        "{reply}"
    );
    assert_eq!(
        log().matches("accepting clients again").count(),
        1,
        "{}",
        log()
    );

    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
    assert!(!log().contains("panicked"), "{}", log());
}
