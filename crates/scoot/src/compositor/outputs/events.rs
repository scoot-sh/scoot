//! IPC event subscription over output remove, restore and in-place resize.
//!
//! The ticket's pins, on headless outputs with a real client holding
//! windows on them (identity is name-only here -- the EDID half is pinned
//! by `output_identity.rs`'s own tests):
//!
//! 1. remove output 2 holding two workspaces with a subscriber connected,
//!    and the removal arrives without polling, carrying the adopter, the
//!    adopted range, and the adopter's previous and new active workspace;
//! 2. add the output back under the same identity, and the restore arrives
//!    with the same record plus how many windows moved back;
//! 3. a hand move between remove and add (the renumbering the parent
//!    ticket's protocol tests pin for the names) keeps the restore payload
//!    honest: the range as recorded, `moved` counting only what went back;
//! 4. a subscriber that never reads is dropped and stalls nothing: flooding
//!    emits past the high-water mark disconnects it, and a removal with a
//!    stuffed subscriber still completes with the adoption intact;
//! 5. a part-written tail drains on the tick, and is given up on only with
//!    no progress at all -- a slow reader is never dropped;
//! 6. the adversarial halves: an empty subscribe is refused, and a dropped
//!    connection leaves no record;
//! 7. resizing an output in place tells the subscriber the new mode and the
//!    scale it keeps -- and a resize that fails fires nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use scoot_core::{Action, OutputId, Vertical, WindowId};
use scoot_ipc::{EventKind, OutputChanged, OutputRemoved, OutputRestored, Response, decode};

use super::per_output::{Ack, CANVAS, Step, session};
use crate::compositor::headless;
use crate::compositor::test_support::Harness;

/// Move the pointer onto the second output: windows mapped after this open
/// there (new-window placement follows the pointer).
fn point_at_second(harness: &mut Harness<Step, Ack>) {
    harness.state.pointer_move(f64::from(CANVAS) * 1.5, 50.0);
}

/// Map one xdg toplevel and answer the window the compositor focused for it.
fn map_window(harness: &mut Harness<Step, Ack>) -> WindowId {
    harness.run(Step::Window);
    harness.settle();
    harness.state.focus.expect("a mapped window takes focus")
}

/// The output the core placed `id` on.
fn output_of(harness: &Harness<Step, Ack>, id: WindowId) -> OutputId {
    harness
        .state
        .world
        .arrange()
        .get(id)
        .expect("the window is placed")
        .output
}

/// Output 2 holding two workspaces -- ws0 `[a][b]`, ws1 `[c]`, ws1 active --
/// with output 1 empty. Returns the three window ids in mapping order.
fn two_workspaces_on_second(harness: &mut Harness<Step, Ack>) -> (WindowId, WindowId, WindowId) {
    point_at_second(harness);
    let a = map_window(harness);
    let b = map_window(harness);
    let c = map_window(harness);
    assert!(
        a != b && b != c && a != c,
        "each mapping focused a new window"
    );
    // Carry the focused window (c) down into the trailing empty: ws1 `[c]`.
    harness
        .state
        .act(Action::MoveWindowToWorkspace(Vertical::Down));
    assert_eq!(output_of(harness, a), OutputId(2));
    assert_eq!(output_of(harness, b), OutputId(2));
    assert_eq!(output_of(harness, c), OutputId(2));
    (a, b, c)
}

/// Subscribes `conn` to output events, answering the client's end: what a
/// `Request::Subscribe` does once the connection loop hands it over.
fn subscribe(harness: &mut Harness<Step, Ack>, conn: u64) -> UnixStream {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = harness
        .state
        .subscribe(conn, server, vec![EventKind::Output]);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout, so a missing event fails instead of hanging the suite");
    client
}

/// The next message the server sent `client`: the subscribed event, read
/// without polling anything else first.
fn next_event(client: &UnixStream) -> Response {
    let mut line = String::new();
    BufReader::new(client)
        .read_line(&mut line)
        .expect("the event arrived");
    decode(&line).expect("a decodable event")
}

/// Pin 1+2: the removal arrives without polling, carrying the adopter, the
/// adopted range and the adopter's previous and new active workspace -- and
/// so does the restore, with how many windows moved back.
#[test]
fn a_subscriber_learns_removal_and_restore_without_polling() {
    let mut harness = session(2);
    let (a, b, c) = two_workspaces_on_second(&mut harness);
    let client = subscribe(&mut harness, 7);

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    assert_eq!(
        next_event(&client),
        Response::OutputRemoved(OutputRemoved {
            output: 2,
            name: "headless-2".into(),
            adopter: Some(1),
            adopted_start: 0,
            adopted_count: 2,
            adopter_prev_active: Some(0),
            // The adopted block lands at 0..2, pushing the adopter's own
            // empty workspace to the trailing end; c's adopted workspace
            // is index 1, and the switch shows it.
            adopter_active: Some(1),
            origin: Some("headless-2".into()),
        })
    );
    // The payload against the layout it describes: every window adopted
    // onto the named range, tagged with the named origin.
    for id in [a, b, c] {
        let (output, workspace, origin) = harness
            .state
            .world
            .window_workspace(id)
            .expect("the window is placed");
        assert_eq!(output, OutputId(1), "window {id:?} is adopted");
        assert!(
            (0..2).contains(&workspace),
            "window {id:?} sits on the named range"
        );
        assert!(origin.is_some(), "window {id:?} carries the origin");
    }

    let id = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();
    assert_eq!(id, OutputId(3), "a fresh id, never the removed one");
    assert_eq!(
        next_event(&client),
        Response::OutputRestored(OutputRestored {
            output: 3,
            name: "headless-2".into(),
            adopter: Some(1),
            adopted_start: 0,
            adopted_count: 2,
            adopter_prev_active: Some(1),
            adopter_active: Some(0),
            origin: Some("headless-2".into()),
            moved: 3,
        })
    );
    for window in [a, b, c] {
        assert_eq!(
            output_of(&harness, window),
            OutputId(3),
            "window {window:?} is back on the returned output"
        );
    }
}

/// Pin 3: a hand move between remove and add keeps the restore payload
/// honest -- the range as recorded, `moved` counting only what went back.
#[test]
fn the_restore_payload_survives_a_hand_move_between_remove_and_add() {
    let mut harness = session(2);
    let (a, b, c) = two_workspaces_on_second(&mut harness);
    let client = subscribe(&mut harness, 7);

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    let _ = next_event(&client);
    // Carry c by hand onto the panel's own first workspace, beside nothing.
    harness.state.act(Action::FocusWindowId(c));
    harness.state.act(Action::MoveWindowToWorkspaceIndex(0));
    assert_eq!(output_of(&harness, c), OutputId(1));

    headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();
    assert_eq!(
        next_event(&client),
        Response::OutputRestored(OutputRestored {
            output: 3,
            name: "headless-2".into(),
            adopter: Some(1),
            adopted_start: 0,
            adopted_count: 2,
            adopter_prev_active: Some(0),
            adopter_active: Some(1),
            origin: Some("headless-2".into()),
            moved: 2,
        })
    );
    assert_eq!(
        output_of(&harness, c),
        OutputId(1),
        "the hand-moved window stays where it was put"
    );
    for window in [a, b] {
        assert_eq!(
            output_of(&harness, window),
            OutputId(3),
            "window {window:?} is back on the returned output"
        );
    }
}

/// One removal event with no reason behind it, for the flood below: the
/// backpressure policy is about bytes queued, not about which removal the
/// bytes describe.
fn synthetic_removed() -> OutputRemoved {
    OutputRemoved {
        output: 9,
        name: "DP-9".into(),
        adopter: Some(1),
        adopted_start: 0,
        adopted_count: 1,
        adopter_prev_active: Some(0),
        adopter_active: Some(1),
        origin: Some("DP-9".into()),
    }
}

/// Shrinks how much unread data the kernel will hold for writes to
/// `stream`, so a test can fill a subscriber's socket without pushing a
/// megabyte through a debug build. Same `setsockopt` the connection tests
/// use, on this side of the module line.
fn set_sndbuf(stream: &UnixStream, bytes: usize) {
    let size = bytes as libc::c_int;
    // SAFETY: a live fd, a `c_int` of exactly the length claimed, nothing
    // borrowed past the call.
    let result = unsafe {
        libc::setsockopt(
            std::os::fd::AsRawFd::as_raw_fd(stream) as libc::c_int,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::from_ref(&size).cast::<libc::c_void>(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(result, 0, "could not shrink the send buffer");
}

/// Fills `stream`'s kernel buffer from this side, so the next emission
/// queues instead of going out: the state a client that never reads its
/// events leaves behind.
fn gorge(stream: &UnixStream) {
    let mut stream = stream;
    let chunk = [b'x'; 64 * 1024];
    for _ in 0..64 {
        match stream.write(&chunk) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("could not fill the socket: {error}"),
        }
    }
    panic!("the socket took 4 MiB without blocking; it is not stuffed");
}

/// Pin 4: a subscriber that never reads is dropped past the high-water
/// mark -- and a removal with a stuffed subscriber still completes, with
/// the adoption intact. Neither half may stall the other.
#[test]
fn a_subscriber_that_never_reads_is_dropped_and_stalls_nothing() {
    let mut harness = session(2);
    two_workspaces_on_second(&mut harness);
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    harness.state.subscribe(7, server, vec![EventKind::Output]);

    // Flood emits without a single read: the queue passes the 1 MiB
    // high-water mark after a few thousand small lines, and the
    // subscription is disconnected rather than buffered without bound.
    let mut dropped_after = None;
    for n in 0..20_000 {
        harness.state.emit_output_removed(synthetic_removed());
        if harness.state.subscribers.is_empty() {
            dropped_after = Some(n);
            break;
        }
    }
    let n = dropped_after.expect("a subscriber that never reads is dropped");
    assert!(
        n > 10,
        "dropped after {n} small lines: the first few must queue, not drop"
    );

    // And output removal with a stuffed subscriber still completes: it
    // never waits for one.
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    harness.state.subscribe(9, server, vec![EventKind::Output]);
    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
}

/// Pin 5: a part-written tail is given up on only with no progress at all.
#[test]
fn a_subscriber_tail_is_given_up_on_only_without_progress() {
    let mut harness = session(2);
    two_workspaces_on_second(&mut harness);
    let (server, client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    client.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    let subscribed_at = Instant::now();
    harness.state.subscribe(7, server, vec![EventKind::Output]);
    harness.state.emit_output_removed(synthetic_removed());
    assert_eq!(
        harness.state.subscribers.len(),
        1,
        "one small line queues behind a stuffed socket; it does not drop it"
    );

    // Nothing read: past the stall window the subscription is gone.
    harness
        .state
        .settle_subscribers_at(subscribed_at + Duration::from_secs(11));
    assert!(
        harness.state.subscribers.is_empty(),
        "no byte in 11 seconds is a client that is not reading"
    );
}

/// Pin 5, second half: a subscriber draining slowly is never given up on,
/// however far past its window that goes -- and a part-written tail that
/// finally goes out keeps the subscription, not just the bytes.
#[test]
fn a_subscriber_that_drains_slowly_is_never_given_up_on() {
    let mut harness = session(2);
    two_workspaces_on_second(&mut harness);
    let (server, client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    client.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    let subscribed_at = Instant::now();
    harness.state.subscribe(7, server, vec![EventKind::Output]);
    // Enough small lines to leave a real tail behind a stuffed socket.
    for _ in 0..100 {
        harness.state.emit_output_removed(synthetic_removed());
    }
    assert_eq!(harness.state.subscribers.len(), 1);

    // One partial drain: some of the tail went out, which restarts the
    // no-progress window from here rather than from the subscribe.
    let mut client = client;
    let mut chunk = [0u8; 1024];
    assert!(
        client.read(&mut chunk).expect("readable") > 0,
        "the stuffed socket has bytes to give"
    );
    harness.state.settle_subscribers_at(subscribed_at);
    assert_eq!(
        harness.state.subscribers.len(),
        1,
        "progress -- any byte at all -- keeps a draining subscriber"
    );
    // And once the tail is fully out, the subscription stays for the next
    // event: empty means done for now, not done forever.
    drain(&client);
    harness.state.settle_subscribers_at(subscribed_at);
    assert_eq!(harness.state.subscribers.len(), 1);
    harness.state.emit_output_removed(synthetic_removed());
    drain(&client);
    assert_eq!(harness.state.subscribers.len(), 1);
}

/// Reads everything `client` has right now.
fn drain(client: &UnixStream) {
    let mut client = client;
    let mut chunk = [0u8; 16 * 1024];
    while let Ok(count) = client.read(&mut chunk) {
        if count == 0 {
            break;
        }
    }
}

/// A resize that changes an output's mode in place tells subscribers the
/// new size and the scale it keeps: the hook a density-watching script
/// recomputes its scale from (gh #318), without polling `outputs`.
#[test]
fn a_subscriber_learns_about_an_in_place_resize() {
    let mut harness = session(2);
    let client = subscribe(&mut harness, 7);

    assert!(harness.state.resize_output_of(OutputId(2), 300, 220));
    harness.settle();
    assert_eq!(
        next_event(&client),
        Response::OutputChanged(OutputChanged {
            output: 2,
            name: "headless-2".into(),
            width: 300,
            height: 220,
            scale: 1.0,
        })
    );
}

/// A resize that fails (no such output) fires nothing: `false` means
/// nothing changed, so there is nothing to report.
#[test]
fn a_failed_resize_fires_no_changed_event() {
    let mut harness = session(2);
    let mut client = subscribe(&mut harness, 7);

    assert!(!harness.state.resize_output_of(OutputId(99), 300, 220));
    harness.settle();
    // The emission is synchronous with the resize, so whatever arrived is
    // already queued: a non-blocking read answers at once, with no timeout
    // to wait out either way.
    client.set_nonblocking(true).expect("non-blocking");
    let mut byte = [0u8; 1];
    match client.read(&mut byte) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("a failed resize must fire no event, got {other:?}"),
    }
}

/// Pin 6: an empty subscribe is refused with a reason, and files nothing.
#[test]
fn an_empty_subscribe_is_refused_and_files_nothing() {
    let mut harness = session(2);
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    let response = harness.state.subscribe(7, server, vec![]);
    assert!(
        matches!(response, Response::Error { .. }),
        "a subscription to nothing is refused, got {response:?}"
    );
    assert!(harness.state.subscribers.is_empty());
}

/// Pin 6, second half: a disconnected subscription leaves no record, even
/// when no event ever fires to reap it.
#[test]
fn dropping_a_subscription_forgets_the_connection() {
    let mut harness = session(2);
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    harness.state.subscribe(7, server, vec![EventKind::Output]);
    assert_eq!(harness.state.subscribers.len(), 1);
    harness.state.drop_subscriber(7);
    assert!(harness.state.subscribers.is_empty());
    // Unknown ids are a no-op, not a panic: connections come and go.
    harness.state.drop_subscriber(7);
    harness.state.drop_subscriber(12345);
}
