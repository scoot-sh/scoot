//! Workspace occupancy events: a bar's occupied-vs-empty workspaces without
//! polling `windows`.
//!
//! The pins, on a live headless `State` with windows filed straight into
//! the core (the way `headless/bench.rs`'s scenes do -- every open, close
//! and move below still reaches the snapshot through `State::apply`, the
//! one choke point under test):
//!
//! 1. opening a window marks its output's snapshot, and the tick carries
//!    exactly one `workspaces` event with the new counts -- nothing goes
//!    out before the tick;
//! 2. closing a window and moving one between workspaces update the counts,
//!    switching to an empty workspace moves only the active index, and
//!    every event agrees with a `windows` query taken beside it;
//! 3. a refresh that changes nothing -- not even a bare `apply()` -- marks
//!    nothing and the tick stays silent;
//! 4. churn coalesces: ten opens, moves and closes across two outputs mark
//!    repeatedly but the tick carries exactly one event per output, with
//!    the final counts -- and a change reverted before the tick carries
//!    nothing at all;
//! 5. removing an output forgets its snapshot: the tick carries the
//!    adopter's new counts and nothing naming the gone output -- and
//!    plugging it back in reports both sides of the restore;
//! 6. a second subscriber arriving between the mark and the tick does not
//!    steal the first subscriber's event: the tick carries the snapshot
//!    to both;
//! 7. a workspace subscriber that never reads is dropped past the same
//!    high-water mark as every other subscriber, stalling nothing;
//! 8. the handshake round-trips (`Subscribed` echoes `workspace`), and an
//!    output-only subscriber never sees an occupancy event.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use scoot_core::{Action, Config, Event as CoreEvent, OutputId, Vertical, WindowId, WindowInfo};
use scoot_ipc::{EventKind, Request, Response, WorkspaceSnapshot, decode};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::set_sndbuf;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;

/// The canvas the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the `State` is a whole session, the way
/// `input/tests.rs`'s fixture builds one.
const CANVAS: i32 = 200;

/// A live compositor with a real headless backend and no client: windows
/// are filed straight into the core, and every change below still reaches
/// the snapshot through [`State::apply`].
struct Fixture {
    _event_loop: EventLoop<'static, State>,
    state: State,
}

impl Fixture {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");
        Self {
            _event_loop: event_loop,
            state,
        }
    }

    /// Opens window `id` on `output` (`None` opens on the first, the way a
    /// new window does with the pointer there) and runs the `apply()` a
    /// real mapping ends in.
    fn open(&mut self, id: u64, output: Option<OutputId>) {
        self.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(id),
            info: WindowInfo::default(),
            output,
            focus: true,
        });
        self.state.apply();
    }

    /// Closes window `id`: the core half of what `remove_window` files
    /// before its own `apply()`.
    fn close(&mut self, id: u64) {
        self.state
            .world
            .handle_event(CoreEvent::WindowClosed { id: WindowId(id) });
        self.state.apply();
    }

    /// Carries the tick: what `frame_tick` runs for occupancy, driven here
    /// directly instead of the timer.
    fn tick(&mut self) {
        self.state.flush_workspace_events();
    }

    /// The `windows` query's answer as a histogram: one count per
    /// workspace of `output`, in order -- what every event below is checked
    /// against. Sized to the core's workspace count, so the trailing empty
    /// workspace reads as a zero rather than vanishing.
    fn queried_counts(&mut self, output: u64) -> Vec<usize> {
        let id = OutputId(output);
        let workspaces = self.state.world.workspaces(id).expect("a known output");
        let mut histogram = vec![0; workspaces.count];
        match self.state.handle_request(scoot_ipc::Request::Windows) {
            Response::Windows { windows } => {
                for window in windows {
                    if window.output == output {
                        histogram[window.workspace] += 1;
                    }
                }
            }
            other => panic!("the windows query answers its listing, got {other:?}"),
        }
        histogram
    }
}

/// Subscribes a fresh connection to `kinds`, answering its end: what a
/// `Request::Subscribe` does once the connection loop hands it over. The
/// reader carries a timeout, so a missing event fails instead of hanging
/// the suite.
fn subscribed(state: &mut State, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    subscribed_as(state, 7, kinds)
}

/// The same, naming the connection: two subscribers in one test need two
/// ids, or the second subscription replaces the first.
fn subscribed_as(state: &mut State, conn: u64, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = state.subscribe(conn, server, kinds);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout, so a missing event fails instead of hanging the suite");
    BufReader::new(client)
}

/// The next message the server sent: one subscribed event.
fn next_event(reader: &mut BufReader<UnixStream>) -> Response {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("the event arrived within the timeout");
    decode(&line).expect("a decodable event")
}

/// Asserts the server sent nothing: the socket must be unreadable right
/// now. Runs the stream non-blocking for the check, then restores blocking
/// (and its timeout) for whatever the test reads next.
fn assert_silent(reader: &mut BufReader<UnixStream>) {
    reader
        .get_mut()
        .set_nonblocking(true)
        .expect("non-blocking for the silence check");
    let mut line = String::new();
    let result = reader.read_line(&mut line);
    reader
        .get_mut()
        .set_nonblocking(false)
        .expect("blocking again afterwards");
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        _ => panic!("expected silence, got {result:?} line {line:?}"),
    }
}

/// Drains every event waiting on `reader`: what the coalescing tests count.
fn drain_events(reader: &mut BufReader<UnixStream>) -> Vec<Response> {
    reader
        .get_mut()
        .set_nonblocking(true)
        .expect("non-blocking for the drain");
    let mut events = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => events.push(decode(&line).expect("a decodable event")),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("could not drain the subscriber: {error}"),
        }
    }
    reader
        .get_mut()
        .set_nonblocking(false)
        .expect("blocking again afterwards");
    events
}

/// Pin 1: opening a window marks its output's snapshot, and the tick
/// carries exactly one event with the new counts -- nothing goes out
/// before the tick, however the change arrived.
#[test]
fn opening_a_window_sends_its_new_snapshot_on_the_tick() {
    let mut fixture = Fixture::new();
    fixture.open(1, None);
    fixture.open(2, None);
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);
    assert_silent(&mut reader);

    fixture.open(3, None);
    // Marked, not sent: the change waits for the tick.
    assert_silent(&mut reader);

    fixture.tick();
    assert_eq!(
        next_event(&mut reader),
        Response::Workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 0,
            counts: vec![3, 0],
        })
    );
    assert_silent(&mut reader);
    assert_eq!(fixture.queried_counts(1), vec![3, 0]);
}

/// Pin 2: closes, moves between workspaces and an active-only switch each
/// update the snapshot, and every event agrees with the `windows` query.
#[test]
fn closes_moves_and_an_active_only_switch_update_the_snapshot() {
    let mut fixture = Fixture::new();
    fixture.open(1, None);
    fixture.open(2, None);
    fixture.open(3, None);
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    // Carry the focused window (3) down into the trailing empty: ws1 `[3]`.
    fixture
        .state
        .act(Action::MoveWindowToWorkspace(Vertical::Down));
    fixture.tick();
    assert_eq!(
        next_event(&mut reader),
        Response::Workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 1,
            counts: vec![2, 1, 0],
        })
    );
    assert_eq!(fixture.queried_counts(1), vec![2, 1, 0]);

    fixture.close(1);
    fixture.tick();
    assert_eq!(
        next_event(&mut reader),
        Response::Workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 1,
            counts: vec![1, 1, 0],
        })
    );
    assert_eq!(fixture.queried_counts(1), vec![1, 1, 0]);

    // Switching to the first workspace moves only the active index: the
    // counts go out unchanged beside it, so a bar showing both never reads
    // them apart.
    fixture.state.act(Action::FocusWorkspaceIndex(0));
    fixture.tick();
    assert_eq!(
        next_event(&mut reader),
        Response::Workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 0,
            counts: vec![1, 1, 0],
        })
    );
    assert_silent(&mut reader);
}

/// Pin 3: a refresh that changes nothing marks nothing -- not even a bare
/// `apply()` with a subscriber watching -- and the tick stays silent.
#[test]
fn an_unchanged_snapshot_marks_nothing() {
    let mut fixture = Fixture::new();
    fixture.open(1, None);
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    fixture.state.apply();
    assert!(
        fixture.state.workspace_pending.is_empty(),
        "an apply that changed nothing must mark nothing"
    );
    fixture.tick();
    assert_silent(&mut reader);
}

/// Pin 4: churn coalesces. Ten opens, moves and closes across two outputs
/// mark repeatedly, but the tick carries exactly one event per output with
/// the final counts -- and a second tick stays silent.
#[test]
fn churn_across_two_outputs_coalesces_to_one_event_each() {
    let mut fixture = Fixture::new();
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    for id in 1..=6 {
        // Odd windows open on the first output, even ones on the second.
        let output = if id % 2 == 1 { None } else { Some(OutputId(2)) };
        fixture.open(id, output);
        assert_silent(&mut reader);
    }
    // Moves and closes in between: each apply re-marks, none sends.
    fixture
        .state
        .act(Action::MoveWindowToWorkspace(Vertical::Down));
    assert_silent(&mut reader);
    fixture.close(2);
    assert_silent(&mut reader);
    fixture.close(5);
    assert_silent(&mut reader);

    fixture.tick();
    let mut events = drain_events(&mut reader);
    events.sort_by_key(|event| match event {
        Response::Workspaces(snapshot) => snapshot.output,
        other => panic!("only occupancy events arrive here, got {other:?}"),
    });
    assert_eq!(events.len(), 2, "one event per output, got {events:?}");
    for event in &events {
        let Response::Workspaces(snapshot) = event else {
            unreachable!("sorted above");
        };
        assert_eq!(
            snapshot.counts,
            fixture.queried_counts(snapshot.output),
            "output {} ends where the query says",
            snapshot.output
        );
    }
    assert_eq!(
        events
            .iter()
            .map(|event| match event {
                Response::Workspaces(snapshot) => snapshot.counts.iter().sum::<usize>(),
                _ => unreachable!(),
            })
            .sum::<usize>(),
        4,
        "six opens minus two closes"
    );

    fixture.tick();
    assert_silent(&mut reader);
}

/// Pin 4, second half: a change reverted before the tick unmarks, and
/// carries nothing.
#[test]
fn a_change_reverted_before_the_tick_sends_nothing() {
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    fixture.open(1, None);
    assert!(
        !fixture.state.workspace_pending.is_empty(),
        "the open marks"
    );
    fixture.close(1);
    assert!(
        fixture.state.workspace_pending.is_empty(),
        "closing it back unmarks: net, nothing changed"
    );
    fixture.tick();
    assert_silent(&mut reader);
}

/// Pin 5: removing an output forgets its snapshot. The tick carries the
/// adopter's new counts, and nothing naming the gone output.
#[test]
fn removing_an_output_forgets_its_snapshot() {
    let mut fixture = Fixture::new();
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.open(1, Some(OutputId(2)));
    fixture.open(2, Some(OutputId(2)));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.tick();
    let events = drain_events(&mut reader);
    assert_eq!(events.len(), 1, "only the adopter reports, got {events:?}");
    let Response::Workspaces(snapshot) = &events[0] else {
        panic!("only occupancy events arrive here, got {events:?}");
    };
    assert_eq!(snapshot.output, 1);
    assert_eq!(
        snapshot.counts.iter().sum::<usize>(),
        2,
        "both adopted windows count on the adopter"
    );
    assert_eq!(snapshot.counts, fixture.queried_counts(1));
    assert!(
        fixture
            .state
            .workspace_published
            .iter()
            .all(|entry| entry.output != 2),
        "the gone output leaves no published snapshot behind"
    );

    fixture.tick();
    assert_silent(&mut reader);
}

/// A second subscriber arriving between the mark and the tick does not
/// steal the first subscriber's event: the subscribe-time sync leaves
/// marked outputs alone, and the tick carries the snapshot to both.
#[test]
fn a_second_subscriber_does_not_swallow_the_first_events_mark() {
    let mut fixture = Fixture::new();
    fixture.open(1, None);
    let mut first = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    fixture.open(2, None);
    // Marked, unsent -- and now a second subscription arrives.
    let mut second = subscribed_as(&mut fixture.state, 8, vec![EventKind::Workspace]);

    fixture.tick();
    for reader in [&mut first, &mut second] {
        assert_eq!(
            next_event(reader),
            Response::Workspaces(WorkspaceSnapshot {
                output: 1,
                name: "headless".into(),
                active: 0,
                counts: vec![2, 0],
            })
        );
        assert_silent(reader);
    }
}

/// Pin 5, second half: plugging the monitor back in restores the windows,
/// and the tick reports both sides -- the adopter losing them and the
/// returned output gaining them.
#[test]
fn replugging_reports_both_sides_of_the_restore() {
    let mut fixture = Fixture::new();
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.open(1, Some(OutputId(2)));
    fixture.open(2, Some(OutputId(2)));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Workspace]);

    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.tick();
    assert_eq!(drain_events(&mut reader).len(), 1);

    let id = headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    assert_eq!(id, OutputId(3), "a fresh id, never the removed one");
    fixture.tick();
    let mut events = drain_events(&mut reader);
    events.sort_by_key(|event| match event {
        Response::Workspaces(snapshot) => snapshot.output,
        other => panic!("only occupancy events arrive here, got {other:?}"),
    });
    assert_eq!(
        events.len(),
        2,
        "the adopter and the returned output both moved, got {events:?}"
    );
    let outputs: Vec<u64> = events
        .iter()
        .map(|event| match event {
            Response::Workspaces(snapshot) => snapshot.output,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(outputs, vec![1, 3]);
    for event in &events {
        let Response::Workspaces(snapshot) = event else {
            unreachable!();
        };
        assert_eq!(
            snapshot.counts,
            fixture.queried_counts(snapshot.output),
            "output {} ends where the query says",
            snapshot.output
        );
    }
    assert_eq!(
        fixture.queried_counts(1).iter().sum::<usize>()
            + fixture.queried_counts(3).iter().sum::<usize>(),
        2,
        "both windows accounted for"
    );
    fixture.tick();
    assert_silent(&mut reader);
}

/// Pin 6: a workspace subscriber that never reads is dropped past the same
/// high-water mark every subscriber observes, and the session carries on
/// without it. The socket is stuffed before subscribing, so every emission
/// queues; the queue passes 1 MiB after a few thousand small lines.
#[test]
fn a_workspace_subscriber_that_never_reads_is_dropped() {
    let mut fixture = Fixture::new();
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    let response = fixture
        .state
        .subscribe(7, server, vec![EventKind::Workspace]);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );

    let mut dropped_after = None;
    for n in 0..20_000u32 {
        fixture.state.emit_workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 0,
            counts: vec![1, 0],
        });
        if fixture.state.subscribers.is_empty() {
            dropped_after = Some(n);
            break;
        }
    }
    let n = dropped_after.expect("a subscriber that never reads is dropped");
    assert!(
        n > 10,
        "dropped after {n} small lines: the first few must queue, not drop"
    );
    // And emission with nobody left is a no-op, not a panic.
    fixture.state.flush_workspace_events();
}

/// Fills `stream`'s send buffer from its own end, so nothing more goes out
/// without a reader -- the state a client that stopped reading leaves
/// behind.
fn gorge(stream: &UnixStream) {
    let mut stream = stream;
    let chunk = [b'x'; 64 * 1024];
    for _ in 0..64 {
        match std::io::Write::write(&mut stream, &chunk) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("could not fill the socket: {error}"),
        }
    }
    panic!("the socket took 4 MiB without blocking; it is not stuffed");
}

/// Pin 7: the handshake echoes the workspace kind -- the subscribe half of
/// the query/subscribe round trip (the wire half lives in
/// `scoot-ipc/tests/wire.rs`, the query half is the `windows` agreement
/// pinned above).
#[test]
fn subscribing_to_workspace_answers_subscribed() {
    let mut fixture = Fixture::new();
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    assert_eq!(
        fixture
            .state
            .subscribe(7, server, vec![EventKind::Workspace]),
        Response::Subscribed {
            events: vec![EventKind::Workspace]
        }
    );
    // ...while a server that predates the kind answers an ordinary error:
    // an unknown kind is a decode failure, which the connection answers
    // with `Error` and keeps serving. Pinned at the decode itself, which
    // is the whole mechanism.
    assert!(
        scoot_ipc::decode::<Request>(r#"{"type":"subscribe","events":["workspace"]}"#).is_ok(),
        "this server knows the kind it just accepted"
    );
}

/// Pin 7, second half: an output-only subscriber never sees an occupancy
/// event -- filtering is by kind -- and the skipped reads while only it
/// was around never surface later.
#[test]
fn an_output_only_subscriber_gets_no_occupancy_events() {
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Output]);

    fixture.open(1, None);
    fixture.open(2, None);
    fixture.tick();
    assert_silent(&mut reader);

    // Subscribing to workspace now syncs instead of replaying the two
    // opens as one change: the next real change emits exactly once.
    let mut occupancy = subscribed(&mut fixture.state, vec![EventKind::Workspace]);
    assert_silent(&mut occupancy);
    fixture.open(3, None);
    fixture.tick();
    assert_eq!(
        next_event(&mut occupancy),
        Response::Workspaces(WorkspaceSnapshot {
            output: 1,
            name: "headless".into(),
            active: 0,
            counts: vec![3, 0],
        })
    );
    assert_silent(&mut occupancy);
}
