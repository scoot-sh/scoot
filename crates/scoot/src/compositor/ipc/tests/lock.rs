//! The side-effect-free lock query and event: what an agent asks before
//! acting, and what the desktop clipboard probes on every copy.
//!
//! The pins, on a live headless session driven through a real lock client
//! (`is_locked` is defined by a live lock object, not a flag a test could
//! set -- see `sighup/tests.rs`):
//!
//! 1. the `locked` query answers `false` unlocked and `true` locked, and it
//!    answers while locked -- unlike `Action`, which is refused there;
//! 2. a fresh `lock` subscription starts silent (read `locked` once for the
//!    baseline), a fresh lock emits exactly one `lock_changed` with
//!    `locked: true`, and the release emits exactly one with `false`;
//! 3. a refused lock (another client holds it) emits nothing -- it changes
//!    nothing a subscriber does not already know.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use scoot_ipc::{EventKind, Request, Response, decode};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the `State` is a whole session.
const CANVAS: i32 = 200;

/// How long to wait for the lock request to land: `is_locked` flips at
/// accept, before the `locked` event the locker itself waits for confirms
/// it, so the query can be pinned without a blanked frame.
const PATIENCE: Duration = Duration::from_secs(10);

/// One instruction for the locker client.
#[derive(Debug)]
enum Step {
    /// `ext_session_lock_manager_v1.lock`, waiting for `locked` (or
    /// `finished`, when another client holds the session).
    TakeSessionLock,
    /// `unlock_and_destroy` on the session lock.
    ReleaseSessionLock,
}

/// What the locker answers a [`Step`] with.
#[derive(Debug)]
enum Ack {
    /// The `locked` event arrived: this client holds the session.
    SessionLocked,
    /// The lock was refused: another client holds the session, and the
    /// compositor answered `finished`.
    LockRefused,
    /// The unlock was requested and flushed.
    SessionReleased,
}

/// Just enough of a screen locker to take and release the session lock.
/// Maps no surfaces at all: `is_locked` flips at accept, and the unlock
/// routes once `locked` has been sent, so neither step needs a blanked
/// frame -- the shape `presentation_time/tests.rs`'s locker takes.
#[derive(Default)]
struct LockerClient {
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    locked_seen: bool,
    finished_seen: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for LockerClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == ext_session_lock_manager_v1::ExtSessionLockManagerV1::interface().name {
            client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => client.locked_seen = true,
            ext_session_lock_v1::Event::Finished => client.finished_seen = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

fn run_locker(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LockerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    client
        .lock_manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1")?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::TakeSessionLock => {
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                let lock = manager.lock(&qh, ());
                client.lock = Some(lock);
                wait_for(&mut queue, &mut client, "locked or finished", |seen| {
                    (seen.locked_seen || seen.finished_seen).then_some(())
                })?;
                if client.locked_seen {
                    acks.send(Ack::SessionLocked).map_err(|e| e.to_string())?;
                } else {
                    client.lock = None;
                    acks.send(Ack::LockRefused).map_err(|e| e.to_string())?;
                }
            }
            Step::ReleaseSessionLock => {
                let lock = client.lock.take().ok_or("no session lock to release")?;
                lock.unlock_and_destroy();
                // Flush the unlock before acknowledging: the ack travels
                // over the step channel, not the Wayland socket, so without
                // this the compositor can settle while the unlock is still
                // sitting in this client's unsent buffer.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::SessionReleased).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Subscribes a fresh connection to `kinds`: what a `Request::Subscribe`
/// does once the connection loop hands it over. The reader carries a
/// timeout, so a missing event fails instead of hanging the suite.
fn subscribed(state: &mut State, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = state.subscribe(7, server, kinds);
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

/// Locks the session through a real lock client, returning the locker's
/// client index. Waits for the locker's own `locked` event (confirm), not
/// just accept: the unlock later routes only once `locked` has been sent,
/// so a test that releases must have confirmed first.
fn lock_session(fixture: &mut Harness<Step, Ack>) -> usize {
    let locker = fixture.spawn(run_locker);
    match fixture.run_on(locker, Step::TakeSessionLock) {
        Ack::SessionLocked => {}
        other => panic!("the locker answered a lock with something else: {other:?}"),
    }
    assert!(
        fixture.state.session_lock.is_locked(),
        "the confirmed lock left the session unlocked"
    );
    locker
}

/// Releases the locker's hold and waits until the compositor has processed
/// the unlock, so whatever the test asserts next observes the unlocked
/// session rather than a flush still in flight.
fn unlock_session(fixture: &mut Harness<Step, Ack>, locker: usize) {
    match fixture.run_on(locker, Step::ReleaseSessionLock) {
        Ack::SessionReleased => {}
        other => panic!("the locker answered an unlock with something else: {other:?}"),
    }
    let deadline = Instant::now() + PATIENCE;
    while fixture.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the unlock never landed; what follows would pass locked"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
}

/// Pin 1: the query reports the live lock state, locked or not -- and it
/// answers while locked, where every `Action` is refused.
#[test]
fn locked_query_reports_the_live_lock_state_locked_or_not() {
    let mut fixture = Harness::<Step, Ack>::headless(Appearance::default(), CANVAS);
    assert!(
        matches!(
            fixture.state.handle_request(Request::Locked),
            Response::Locked { locked: false }
        ),
        "an unlocked session answers unlocked"
    );

    let locker = lock_session(&mut fixture);
    assert!(
        matches!(
            fixture.state.handle_request(Request::Locked),
            Response::Locked { locked: true }
        ),
        "a locked session answers locked"
    );
    // The contrast this query exists for: window management stays refused
    // under lock, while the lock state itself stays readable.
    let action = Request::Action(scoot_ipc::Action::FocusWorkspaceIndex { index: 0 });
    assert!(
        matches!(fixture.state.handle_request(action), Response::Error { .. }),
        "an action answered while locked"
    );
    assert!(
        matches!(
            fixture.state.handle_request(Request::Locked),
            Response::Locked { locked: true }
        ),
        "the refused action disturbed the lock state it was gated on"
    );

    unlock_session(&mut fixture, locker);
    assert!(
        matches!(
            fixture.state.handle_request(Request::Locked),
            Response::Locked { locked: false }
        ),
        "an unlocked session answers unlocked again"
    );
}

/// Pin 2: one transition is exactly one event, carrying the state it moved
/// to -- and a fresh subscription starts silent, so the query above is the
/// baseline and nothing replays the unsubscribed interval.
#[test]
fn lock_transitions_emit_exactly_one_event_each() {
    let mut fixture = Harness::<Step, Ack>::headless(Appearance::default(), CANVAS);
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Lock]);
    assert_silent(&mut reader);

    let locker = lock_session(&mut fixture);
    assert_eq!(
        next_event(&mut reader),
        Response::LockChanged { locked: true },
        "a fresh lock emits exactly its new state"
    );

    unlock_session(&mut fixture, locker);
    assert_eq!(
        next_event(&mut reader),
        Response::LockChanged { locked: false },
        "the release emits exactly its new state"
    );
    assert_silent(&mut reader);
}

/// Pin 3: a refused lock emits nothing -- the session was locked and stays
/// locked, which is what the subscriber already knows.
#[test]
fn a_refused_lock_emits_nothing() {
    let mut fixture = Harness::<Step, Ack>::headless(Appearance::default(), CANVAS);
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Lock]);

    let locker = lock_session(&mut fixture);
    assert_eq!(
        next_event(&mut reader),
        Response::LockChanged { locked: true },
        "a fresh lock emits exactly its new state"
    );

    let second = fixture.spawn(run_locker);
    fixture.send_step(second, Step::TakeSessionLock);
    let refused = fixture.wait_for_ack(second);
    assert!(
        matches!(refused, Ack::LockRefused),
        "a second lock while locked is refused, got {refused:?}"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "the refused lock disturbed the lock it was refused by"
    );
    assert_silent(&mut reader);

    unlock_session(&mut fixture, locker);
    assert_eq!(
        next_event(&mut reader),
        Response::LockChanged { locked: false },
        "the release still emits exactly its new state"
    );
}
