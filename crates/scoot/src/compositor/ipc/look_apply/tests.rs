//! The parked swap's pure policies: which names never spawn, when a
//! preview reverts, and what each child end state answers.
//!
//! Plus the gate, against a live `State`: a second swap while one is
//! parked refuses with the connection's queue and slot handed back (so a
//! refusal loses nothing queued and leaks no table capacity), and nothing
//! spawns for any refusal.

use scoot_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;
use crate::compositor::test_support::test_renderer;

#[test]
fn only_plain_look_names_spawn() {
    assert!(valid_look_apply_name("moonrise").is_ok());
    assert!(valid_look_apply_name("a").is_ok());
    assert!(valid_look_apply_name("ginger-night-2").is_ok());
    for bad in [
        "",
        "Moonrise",
        "moon_rise",
        "moon rise",
        "moon.rise",
        "../evil",
        "/abs",
        "a-very-long-look-name-well-past-32-chars",
        "look\nname",
    ] {
        assert!(
            valid_look_apply_name(bad).is_err(),
            "`{bad}` must never spawn"
        );
    }
}

#[test]
fn a_preview_reverts_only_onto_its_own_generation() {
    assert!(should_revert(Some(7), 7));
    assert!(!should_revert(Some(8), 7));
    assert!(!should_revert(None, 7));
}

#[test]
fn the_generation_comes_from_the_success_document() {
    let output = br#"{"applied":"moonrise","rendered":"/r","generation":7,"reloaded":[],"skipped":[],"shadowed":[],"invalid":[],"notes":[]}"#;
    assert_eq!(child_generation(output), Some(7));
    assert_eq!(child_generation(b"not json"), None);
    assert_eq!(child_generation(b""), None);
    // A failure document carries no generation: no timer arms off one.
    let failure = br#"{"target":"moonrise","applied":false,"error":"nope","rolled_back_to":null,"rollback_errors":[]}"#;
    assert_eq!(child_generation(failure), None);
}

#[test]
fn a_success_shapes_look_applied() {
    let output = br#"{"applied":"moonrise","rendered":"/r","generation":7,"reloaded":["compositor","bar wayland-1"],"skipped":[],"shadowed":["appearance.gap"],"invalid":[],"notes":[]}"#;
    assert_eq!(
        look_applied_from_child("moonrise", ChildEnd::Success, output),
        Response::LookApplied {
            name: "moonrise".into(),
            applied: vec!["compositor".into(), "bar wayland-1".into()],
            refused: Vec::new(),
            shadowed: vec!["appearance.gap".into()],
        }
    );
}

#[test]
fn a_zero_exit_with_a_failure_document_believes_the_document() {
    let output = br#"{"target":"moonrise","applied":false,"error":"the bar refused","rolled_back_to":"ginger-night","rollback_errors":[]}"#;
    match look_applied_from_child("moonrise", ChildEnd::Success, output) {
        Response::Error { message } => {
            assert!(message.contains("the bar refused"), "{message}");
            assert!(message.contains("rolled back to ginger-night"), "{message}");
        }
        other => panic!("a failure document answers Error, not {other:?}"),
    }
}

#[test]
fn an_unreadable_success_refuses_rather_than_guesses() {
    for output in [b"".as_slice(), b"not json", b"{\"applied\": 42}"] {
        match look_applied_from_child("moonrise", ChildEnd::Success, output) {
            Response::Error { message } => assert!(
                message.contains("unreadable"),
                "names the uncertainty: {message}"
            ),
            other => panic!("an undescribable swap must refuse, not {other:?}"),
        }
    }
}

#[test]
fn trailing_chatter_after_the_document_is_not_the_reply() {
    // A grandchild holding the pipe open past the document cannot rewrite
    // the answer: only the first value shapes it.
    let mut output =
        br#"{"applied":"moonrise","rendered":"/r","generation":7,"reloaded":[],"skipped":[],"shadowed":[],"invalid":[],"notes":[]}"#.to_vec();
    output.extend_from_slice(b"\n{\"type\":\"oops\"}\n");
    assert_eq!(
        look_applied_from_child("moonrise", ChildEnd::Success, &output),
        Response::LookApplied {
            name: "moonrise".into(),
            applied: Vec::new(),
            refused: Vec::new(),
            shadowed: Vec::new(),
        }
    );
}

#[test]
fn a_failure_names_the_refusal_and_the_rollback() {
    let output = br#"{"target":"moonrise","applied":false,"error":"the bar refused the look reload: boom","rolled_back_to":"ginger-night","rollback_errors":[]}"#;
    match look_applied_from_child("moonrise", ChildEnd::Code(1), output) {
        Response::Error { message } => {
            assert!(message.contains("the bar refused"), "{message}");
            assert!(message.contains("rolled back to ginger-night"), "{message}");
        }
        other => panic!("a refused swap answers Error, not {other:?}"),
    }
}

#[test]
fn rollback_errors_ride_along_loudly() {
    let output = br#"{"target":"moonrise","applied":false,"error":"the compositor refused","rolled_back_to":"ginger-night","rollback_errors":["the bar refused too"]}"#;
    match look_applied_from_child("moonrise", ChildEnd::Code(1), output) {
        Response::Error { message } => {
            assert!(message.contains("ROLLED BACK WITH ERRORS"), "{message}");
            assert!(message.contains("the bar refused too"), "{message}");
        }
        other => panic!("rollback errors must stay loud, not {other:?}"),
    }
}

#[test]
fn a_child_with_no_document_reports_its_end() {
    match look_applied_from_child("moonrise", ChildEnd::Code(2), b"") {
        Response::Error { message } => assert!(message.contains("status 2"), "{message}"),
        other => panic!("an empty failure answers Error, not {other:?}"),
    }
    match look_applied_from_child("moonrise", ChildEnd::Signal(9), b"") {
        Response::Error { message } => assert!(message.contains("signal 9"), "{message}"),
        other => panic!("a killed child answers Error, not {other:?}"),
    }
    match look_applied_from_child("moonrise", ChildEnd::Unknown, b"") {
        Response::Error { message } => assert!(message.contains("moonrise"), "{message}"),
        other => panic!("an unknown end answers Error, not {other:?}"),
    }
}

fn error_message(response: &Response) -> &str {
    match response {
        Response::Error { message } => message,
        other => panic!("expected an error reply, got {other:?}"),
    }
}

/// A live `State` with no backend and no config file: enough for the
/// swap gate (which reads no files on these paths), far from enough to
/// run anything.
fn gate_state() -> State {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    State::new(
        &mut event_loop,
        display,
        Config::default(),
        Keybindings::default(),
        Appearance::default(),
        1.0,
        test_renderer(),
    )
    .expect("a compositor state")
}

/// A parked swap filed directly: the gate only reads presence, so no
/// child runs for these refusals.
fn parked_swap() -> PendingLookApply {
    let now = Instant::now();
    PendingLookApply {
        pid: Some(u32::MAX),
        stdout: None,
        name: "look-a".into(),
        for_ms: None,
        previous: None,
        started: now,
        finished: false,
        last_progress: now,
        client: None,
    }
}

#[test]
fn a_bad_name_refuses_before_anything_spawns() {
    let mut state = gate_state();
    let (a, _b) = UnixStream::pair().expect("a socket pair");
    let slots = super::super::slots::Slots::new();
    let refused = state
        .start_look_apply(
            "../evil".into(),
            None,
            a,
            Outbound::default(),
            slots.claim(),
        )
        .expect_err("a path never swaps");
    assert!(
        error_message(&refused.response).contains("not a look name"),
        "names the rule"
    );
    assert!(refused.slot.is_some(), "the slot rides back out");
    assert!(state.pending_look.is_none(), "nothing parked");
}

/// A queue the socket would not take, so the refusal's hand-back is not
/// vacuous: the peer never reads, the send buffer is tiny, and a
/// megabyte does not fit.
fn filled_outbound() -> (Outbound, UnixStream, UnixStream) {
    let (a, b) = UnixStream::pair().expect("a socket pair");
    a.set_nonblocking(true).expect("a nonblocking socket");
    super::super::tests::set_sndbuf(&a, 4096);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut &a, "x".repeat(1024 * 1024))
        .expect("a queued tail, not an error");
    assert!(
        outbound.pending() > 0,
        "a megabyte past a 4 KiB buffer leaves a tail"
    );
    (outbound, a, b)
}

#[test]
fn a_second_swap_hands_back_the_queue_and_the_slot() {
    let mut state = gate_state();
    state.pending_look = Some(parked_swap());
    let (outbound, _a, _b) = filled_outbound();
    let queued = outbound.pending();
    let slots = super::super::slots::Slots::new();
    let (c, _d) = UnixStream::pair().expect("a socket pair");
    let refused = state
        .start_look_apply("look-b".into(), None, c, outbound, slots.claim())
        .expect_err("one swap at a time");
    assert!(
        error_message(&refused.response).contains("already running"),
        "names the gate"
    );
    assert_eq!(
        refused.outbound.pending(),
        queued,
        "the queued tail rides back out whole"
    );
    assert!(refused.slot.is_some(), "the slot rides back out");
    // And the parked swap is untouched: still the first one.
    assert_eq!(
        state.pending_look.as_ref().expect("still parked").name,
        "look-a"
    );
}

#[test]
fn a_preview_with_no_recorded_choice_refuses() {
    let mut state = gate_state();
    // A state dir with no record: the preview has nothing to revert to.
    let dir = tempfile::tempdir().expect("a state dir");
    state.private_look_dir = Some(dir.path().to_owned());
    let (a, _b) = UnixStream::pair().expect("a socket pair");
    let slots = super::super::slots::Slots::new();
    let refused = state
        .start_look_apply(
            "look-b".into(),
            Some(1500),
            a,
            Outbound::default(),
            slots.claim(),
        )
        .expect_err("no previous look, no preview");
    assert!(
        error_message(&refused.response).contains("no look is recorded yet"),
        "names the missing revert"
    );
    assert!(state.pending_look.is_none(), "nothing spawned");
}
