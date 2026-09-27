//! The request-to-reply mapping. The loop itself runs against a real
//! compositor in `crates/scootbg/tests/`.

use super::respond::Responder;
use crate::control::Handler;
use crate::protocol::{OutputEntry, PROTOCOL_VERSION, Request};

const NO_OUTPUTS: &[OutputEntry<'static>; 0] = &[];

fn ask(responder: &mut Responder, line: &str) -> serde_json::Value {
    let mut out = Vec::new();
    responder.handle(line.as_bytes(), &mut out);
    assert_eq!(out.last(), Some(&b'\n'), "one line, newline-terminated");
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn query_answers_an_empty_output_list() {
    let mut responder = Responder::new(NO_OUTPUTS);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "outputs", "outputs": []}));
    assert!(!responder.stop);
}

#[test]
fn version_answers_the_build_and_protocol() {
    let mut responder = Responder::new(NO_OUTPUTS);
    let reply = ask(&mut responder, Request::Version.line().trim_end());
    assert_eq!(reply["type"], "version");
    assert_eq!(reply["protocol"], PROTOCOL_VERSION);
    assert_eq!(reply["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn kill_answers_ok_and_asks_the_loop_to_stop() {
    let mut responder = Responder::new(NO_OUTPUTS);
    let reply = ask(&mut responder, Request::Kill.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "ok"}));
    assert!(responder.stop);
}

#[test]
fn bad_requests_answer_errors_and_change_nothing() {
    let mut responder = Responder::new(NO_OUTPUTS);
    for line in [
        "garbage",
        "",
        r#"{"type":"kill"}"#,
        r#"{"protocol":99,"type":"kill"}"#,
        r#"{"protocol":1,"type":"set"}"#,
        r#"{"protocol":1}"#,
    ] {
        let reply = ask(&mut responder, line);
        assert_eq!(reply["type"], "error", "{line:?}");
        assert!(reply["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    assert!(!responder.stop);
}

/// The poll set's allocation survives the round trip through `reuse`, so
/// the loop allocates nothing once warm. If std ever stopped collecting in
/// place, this fails rather than the loop quietly allocating per wakeup.
#[test]
fn the_poll_set_keeps_its_allocation() {
    use rustix::event::{PollFd, PollFlags};

    let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut fds: Vec<PollFd<'static>> = Vec::with_capacity(19);
    let ptr = fds.as_ptr();
    for _ in 0..3 {
        let mut round: Vec<PollFd<'_>> = super::reuse(fds);
        for _ in 0..19 {
            round.push(PollFd::new(&a, PollFlags::IN));
        }
        assert_eq!(round.as_ptr(), ptr);
        fds = super::reuse(round);
        assert!(fds.is_empty());
        assert_eq!(fds.as_ptr(), ptr);
        assert!(fds.capacity() >= 19);
    }
}

/// The one panic message the crash hook turns into exit 1: std's own for a
/// failed `print!`/`eprint!`.
#[test]
fn the_crash_hook_recognises_stds_broken_stdio_panic() {
    use super::crash::is_stdio_message;

    // std's `_eprint` cannot be pointed at a test double, so the real
    // panic, an `eprintln!` into a pipe with no reader, is exercised end
    // to end in `tests/daemon.rs` (`WAYLAND_DEBUG` with a broken stderr).
    assert!(is_stdio_message(
        "failed printing to stderr: Broken pipe (os error 32)"
    ));
    assert!(is_stdio_message(
        "failed printing to stdout: Broken pipe (os error 32)"
    ));
    assert!(!is_stdio_message("index out of bounds"));
    assert!(!is_stdio_message(
        "called `Option::unwrap()` on a `None` value"
    ));
}

/// The hook removes the socket only while armed, and at most once; after
/// `disarm` (which the daemon calls before releasing its claim) a panic
/// leaves the path alone, whoever has bound it since.
#[test]
fn the_crash_hook_removes_the_socket_only_while_armed() {
    use super::crash::{Armed, remove_if_armed};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let dir = std::env::temp_dir().join(format!("sbg-crash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s.sock");

    // Armed: removed, and the hold is spent.
    std::fs::write(&socket, b"").unwrap();
    let armed = Armed::for_test(Arc::new(AtomicBool::new(true)));
    remove_if_armed(&armed, &socket);
    assert!(!socket.exists());
    // A new daemon's socket at the same path afterwards survives a second
    // panic of the old one.
    std::fs::write(&socket, b"new").unwrap();
    remove_if_armed(&armed, &socket);
    assert!(socket.exists());

    // Disarmed before any panic: never removed.
    let disarmed = Armed::for_test(Arc::new(AtomicBool::new(true)));
    disarmed.disarm();
    remove_if_armed(&disarmed, &socket);
    assert!(socket.exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Normal operation: the listener is always polled, with no timeout, and
/// the clock is not even read.
#[test]
fn an_armed_listener_polls_forever_and_reads_no_clock() {
    use super::listen::Listening;
    let mut listening = Listening::default();
    for _ in 0..3 {
        let plan = listening.poll_plan(|| panic!("read the clock while armed"));
        assert_eq!(plan, (true, None));
    }
    assert!(!listening.worked(), "nothing to recover from");
}

/// A failed accept rests the listener for `REST`, bounded, then re-arms
/// it; a failure that persists is reported once, and its end once.
#[test]
fn a_failed_accept_rests_the_listener_then_rearms_it() {
    use super::listen::{Listening, REST};
    use std::time::{Duration, Instant};

    let t0 = Instant::now();
    let mut listening = Listening::default();
    assert!(listening.failed(t0), "the first failure is reported");
    assert!(listening.is_resting());
    // Resting: not polled, and the timeout is what is left of the rest.
    assert_eq!(listening.poll_plan(|| t0), (false, Some(REST)));
    let half = t0 + REST / 2;
    assert_eq!(listening.poll_plan(|| half), (false, Some(REST / 2)));
    // Never a zero timeout while resting (that would spin): at the end of
    // the rest it is re-armed instead.
    assert_eq!(listening.poll_plan(|| t0 + REST), (true, None));
    assert!(!listening.is_resting());
    // A clock read late (a long stall) re-arms too.
    assert!(
        !listening.failed(t0 + REST),
        "still failing: not re-reported"
    );
    assert_eq!(
        listening.poll_plan(|| t0 + REST * 10),
        (true, None),
        "re-armed after a stall"
    );
    // Accepting works again: reported once, then quiet.
    assert!(listening.worked());
    assert!(!listening.worked());
    // A new failure later is news again.
    assert!(listening.failed(t0 + Duration::from_secs(60)));
}

/// However many times accepting fails in a row, the loop polls at most once
/// per `REST` with the listener out and once with it in: a bounded rate,
/// never a spin, and never deaf for longer than `REST`.
#[test]
fn a_persistent_accept_failure_is_retried_at_a_bounded_rate() {
    use super::listen::{Listening, REST};
    use std::time::Instant;

    let t0 = Instant::now();
    let mut listening = Listening::default();
    let mut now = t0;
    let mut polls_with_listener = 0;
    let mut reports = 0;
    // Simulate 10 s of a listener whose every accept fails at once.
    while now < t0 + REST * 10 {
        match listening.poll_plan(|| now) {
            (true, None) => {
                polls_with_listener += 1;
                // Readable at once, and the accept fails.
                reports += usize::from(listening.failed(now));
            }
            (false, Some(timeout)) => {
                assert!(!timeout.is_zero());
                now += timeout;
            }
            other => panic!("unexpected plan {other:?}"),
        }
    }
    assert_eq!(reports, 1);
    assert_eq!(polls_with_listener, 10);
}

/// `query` lists the model's outputs as they stand, in order.
#[test]
fn query_reports_each_output_and_its_surface() {
    use crate::outputs::Outputs;

    let mut outputs = Outputs::<()>::default();
    let a = outputs.add(3, |_| ());
    let b = outputs.add(4, |_| ());
    let c = outputs.add(5, |_| ());
    {
        let first = &mut outputs.get_mut(a).unwrap().output;
        first.stage_name("HEADLESS-1".into());
        first.stage_description("Headless output 1".into());
        first.stage_mode(true, 2560, 1440);
        first.stage_scale(2);
        first.done();
        let _ = first.settled();
        let _ = first.configure(7, 1280, 720);
    }
    {
        let second = &mut outputs.get_mut(b).unwrap().output;
        second.stage_name("HEADLESS-2".into());
        second.done();
        let _ = second.settled();
        let _ = second.closed();
        let _ = second.retry();
        let _ = second.closed();
    }
    let _ = c; // bound, not yet reported: waiting

    let mut responder = Responder::new(&outputs);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(
        reply,
        serde_json::json!({"type": "outputs", "outputs": [
            {
                "name": "HEADLESS-1",
                "description": "Headless output 1",
                "mode": {"width": 2560, "height": 1440},
                "scale": 2,
                "transform": "normal",
                "logical": {"width": 1280, "height": 720},
                "surface": {"state": "configured", "size": {"width": 1280, "height": 720}},
                "shows": null,
            },
            {
                "name": "HEADLESS-2",
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "gave-up", "size": null},
                "shows": null,
            },
            {
                "name": null,
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "waiting", "size": null},
                "shows": null,
            },
        ]})
    );
    // Removing one updates the next reply.
    let _ = outputs.remove_global(4);
    let mut responder = Responder::new(&outputs);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(reply["outputs"].as_array().unwrap().len(), 2);
}

/// A `query` reply into a warm buffer allocates nothing: the entries are
/// borrowed from the model and serialized straight into the connection's
/// reused output buffer.
#[test]
fn a_query_reply_reuses_the_output_buffer() {
    use crate::outputs::Outputs;

    let mut outputs = Outputs::<()>::default();
    for global in 0..4 {
        let id = outputs.add(global, |_| ());
        let output = &mut outputs.get_mut(id).unwrap().output;
        output.stage_name(format!("DP-{global}"));
        output.stage_mode(true, 3840, 2160);
        output.done();
        let _ = output.settled();
        let _ = output.configure(1, 3840, 2160);
    }
    let mut responder = Responder::new(&outputs);
    let line = Request::Query.line();
    let mut out = Vec::with_capacity(4096);
    let ptr = out.as_ptr();
    for _ in 0..3 {
        out.clear();
        responder.handle(line.trim_end().as_bytes(), &mut out);
        assert!(out.len() < 4096);
        assert_eq!(out.as_ptr(), ptr, "the buffer was reallocated");
    }
}
