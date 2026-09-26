//! The request-to-reply mapping. The loop itself runs against a real
//! compositor in `crates/scootbg/tests/`.

use super::respond::Responder;
use crate::control::Handler;
use crate::protocol::{PROTOCOL_VERSION, Request};

fn ask(responder: &mut Responder, line: &str) -> serde_json::Value {
    let mut out = Vec::new();
    responder.handle(line.as_bytes(), &mut out);
    assert_eq!(out.last(), Some(&b'\n'), "one line, newline-terminated");
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn query_answers_an_empty_output_list() {
    let mut responder = Responder::default();
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "outputs", "outputs": []}));
    assert!(!responder.stop);
}

#[test]
fn version_answers_the_build_and_protocol() {
    let mut responder = Responder::default();
    let reply = ask(&mut responder, Request::Version.line().trim_end());
    assert_eq!(reply["type"], "version");
    assert_eq!(reply["protocol"], PROTOCOL_VERSION);
    assert_eq!(reply["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn kill_answers_ok_and_asks_the_loop_to_stop() {
    let mut responder = Responder::default();
    let reply = ask(&mut responder, Request::Kill.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "ok"}));
    assert!(responder.stop);
}

#[test]
fn bad_requests_answer_errors_and_change_nothing() {
    let mut responder = Responder::default();
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
