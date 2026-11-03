//! The per-client cap on capture and pointer cursor sessions: what a client
//! is told at the cap, that a destroy or a disconnect frees its slot at once,
//! that clients are counted independently, and that nothing under the cap
//! changes. See `screencopy/session_cap.rs`.

use crate::compositor::screencopy::session_cap::{
    Admission, KILL_SESSIONS_PER_CLIENT, MAX_SESSIONS_PER_CLIENT, decide,
};

use super::*;

/// A fixture whose client 0 holds exactly `MAX_SESSIONS_PER_CLIENT` live
/// sessions, the first of which (a capture session on the one output) is the
/// one `Step::Capture` works on.
fn at_the_cap() -> Fixture {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartSession {
        paint_cursors: false,
    });
    let stopped = fixture
        .run(Step::HoldSessions {
            count: MAX_SESSIONS_PER_CLIENT - 1,
        })
        .count();
    assert_eq!(stopped, 0, "every session under the cap is served");
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT
    );
    fixture
}

fn capture_works(fixture: &mut Fixture, client: Option<usize>) {
    let step = Step::Capture {
        width: CANVAS,
        height: CANVAS,
        format: wl_shm::Format::Argb8888,
    };
    let ack = match client {
        Some(index) => fixture.run_on(index, step),
        None => fixture.run(step),
    };
    let (outcome, captured) = ack.frame();
    assert_eq!(outcome, Outcome::Ready);
    assert_eq!(captured, fixture.pixels());
}

#[test]
fn the_boundaries_are_the_named_constants() {
    assert_eq!(decide(0), Admission::Admit);
    assert_eq!(decide(MAX_SESSIONS_PER_CLIENT - 1), Admission::Admit);
    assert_eq!(decide(MAX_SESSIONS_PER_CLIENT), Admission::Stop);
    assert_eq!(decide(KILL_SESSIONS_PER_CLIENT - 1), Admission::Stop);
    assert_eq!(decide(KILL_SESSIONS_PER_CLIENT), Admission::Kill);
    assert_eq!(decide(u32::MAX), Admission::Kill);
}

#[test]
fn the_session_at_the_cap_is_served_in_full() {
    // The regression half: the cap-th session behaves exactly as the first.
    let mut fixture = Fixture::start();
    fixture.run(Step::HoldSessions {
        count: MAX_SESSIONS_PER_CLIENT - 1,
    });
    let told = fixture
        .run(Step::StartSession {
            paint_cursors: false,
        })
        .constraints();
    assert!(!told.stopped);
    assert_eq!((told.width, told.height), (CANVAS as u32, CANVAS as u32));
    capture_works(&mut fixture, None);
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT
    );
}

#[test]
fn a_session_past_the_cap_is_stopped_and_the_client_stays_connected() {
    let mut fixture = at_the_cap();

    let stopped = fixture.run(Step::HoldSessions { count: 3 }).count();
    assert_eq!(stopped, 3, "each session past the cap is told `stopped`");
    assert_eq!(
        fixture.state.screencopy.session_count(),
        (
            MAX_SESSIONS_PER_CLIENT as usize,
            MAX_SESSIONS_PER_CLIENT as usize
        ),
        "a stopped session never enters either list"
    );

    // The client was not disconnected, and its served session still works.
    capture_works(&mut fixture, None);
}

#[test]
fn a_destroyed_session_frees_its_slot_at_once() {
    let mut fixture = at_the_cap();
    assert_eq!(fixture.run(Step::HoldSessions { count: 1 }).count(), 1);

    fixture.run(Step::ReleaseHeld { count: 1 });
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT,
        "destroying the stopped one frees its unit too"
    );
    fixture.run(Step::ReleaseHeld { count: 1 });
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT - 1
    );
    assert_eq!(
        fixture.run(Step::HoldSessions { count: 1 }).count(),
        0,
        "the freed slot admits the next session"
    );
    assert_eq!(fixture.run(Step::HoldSessions { count: 1 }).count(), 1);
}

#[test]
fn a_client_that_disconnects_at_the_cap_frees_everything() {
    let mut fixture = at_the_cap();
    fixture.run(Step::HoldSessions { count: 5 });
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT + 5
    );

    fixture.disconnect(0);
    assert_eq!(fixture.state.screencopy.session_cap.total(), 0);
    assert_eq!(fixture.state.screencopy.session_count(), (0, 0));
}

#[test]
fn another_client_is_admitted_while_one_is_at_the_cap() {
    let mut fixture = at_the_cap();
    let other = fixture.spawn(run_client);
    // A refusal has just happened: a flag left armed would stop the next
    // client's session.
    assert_eq!(fixture.run(Step::HoldSessions { count: 1 }).count(), 1);

    let told = fixture
        .run_on(
            other,
            Step::StartSession {
                paint_cursors: false,
            },
        )
        .constraints();
    assert!(!told.stopped, "the cap is per client");
    capture_works(&mut fixture, Some(other));

    // And the other client has a full cap of its own.
    assert_eq!(
        fixture
            .run_on(
                other,
                Step::HoldSessions {
                    count: MAX_SESSIONS_PER_CLIENT - 1
                }
            )
            .count(),
        0
    );
    assert_eq!(
        fixture
            .run_on(other, Step::HoldSessions { count: 1 })
            .count(),
        1
    );
}

#[test]
fn a_client_looping_past_the_hard_cap_is_disconnected_and_others_are_not() {
    let mut fixture = Fixture::start();
    let other = fixture.spawn(run_client);
    fixture.run_on(
        other,
        Step::StartSession {
            paint_cursors: false,
        },
    );

    let error = fixture.run_expecting_disconnect(Step::HoldSessions {
        count: KILL_SESSIONS_PER_CLIENT + 10,
    });
    assert!(error.contains("session refused"), "got: {error}");
    fixture.settle();
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        1,
        "the dead client's units are gone; the other client's one remains"
    );
    capture_works(&mut fixture, Some(other));
}

#[test]
fn cursor_sessions_share_the_cap_and_cost_one_unit_each() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartCursorSession);
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        1,
        "a cursor session and its capture session are one unit"
    );

    // Drop just the derived capture session: the cursor session still holds
    // the unit.
    fixture.run(Step::DestroySession);
    assert_eq!(fixture.state.screencopy.session_cap.total(), 1);
    fixture.run(Step::DestroyCursorSession);
    assert_eq!(fixture.state.screencopy.session_cap.total(), 0);
    assert_eq!(fixture.state.screencopy.session_cap.cursor_marks(), 0);
}

#[test]
fn a_cursor_session_past_the_cap_is_stopped_and_frees_its_slot() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartCursorSession);
    assert_eq!(
        fixture
            .run(Step::HoldCursorSessions {
                count: MAX_SESSIONS_PER_CLIENT - 1
            })
            .count(),
        0
    );
    assert_eq!(
        fixture.state.screencopy.cursor_session_count(),
        (
            MAX_SESSIONS_PER_CLIENT as usize,
            MAX_SESSIONS_PER_CLIENT as usize
        )
    );

    assert_eq!(
        fixture.run(Step::HoldCursorSessions { count: 2 }).count(),
        2,
        "the derived capture session of a refused cursor session is stopped"
    );
    assert_eq!(
        fixture.state.screencopy.cursor_session_count().0,
        MAX_SESSIONS_PER_CLIENT as usize,
        "and it never entered the cursor list"
    );

    fixture.run(Step::ReleaseHeld { count: 3 });
    assert_eq!(
        fixture.state.screencopy.session_cap.total(),
        MAX_SESSIONS_PER_CLIENT - 1
    );
    assert_eq!(
        fixture.run(Step::HoldCursorSessions { count: 1 }).count(),
        0
    );

    fixture.disconnect(0);
    assert_eq!(fixture.state.screencopy.session_cap.total(), 0);
    assert_eq!(fixture.state.screencopy.session_cap.cursor_marks(), 0);
    assert_eq!(fixture.state.screencopy.cursor_session_count(), (0, 0));
}

#[test]
fn capture_and_cursor_sessions_count_against_the_same_cap() {
    let mut fixture = Fixture::start();
    fixture.run(Step::HoldSessions {
        count: MAX_SESSIONS_PER_CLIENT - 1,
    });
    assert_eq!(
        fixture.run(Step::HoldCursorSessions { count: 1 }).count(),
        0
    );
    assert_eq!(
        fixture.run(Step::HoldCursorSessions { count: 1 }).count(),
        1
    );
    assert_eq!(fixture.run(Step::HoldSessions { count: 1 }).count(), 1);
}

#[test]
fn a_second_capture_session_on_a_cursor_session_is_a_protocol_error_not_a_panic() {
    let mut fixture = Fixture::start();
    fixture.run(Step::StartCursorSession);
    let error = fixture.run_expecting_disconnect(Step::GetCaptureSessionTwice);
    assert!(error.contains("duplicate_session"), "got: {error}");
    fixture.settle();
    assert_eq!(fixture.state.screencopy.session_cap.total(), 0);
    assert_eq!(fixture.state.screencopy.session_cap.cursor_marks(), 0);
}
