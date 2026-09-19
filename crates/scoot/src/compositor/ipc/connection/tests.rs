//! Tests for the connection loop, driven through a real event loop.
//!
//! These deliberately do not unit-test [`Connection::step`] against a stub:
//! every bug this module exists to fix was about *when the event loop gets its
//! thread back*, which only a real `EventLoop` can observe. So each test stands
//! up a real [`State`], hands [`super::super::accept`] one end of a socket pair
//! exactly as the listener would, and drives the loop by hand from the test
//! thread.
//!
//! Two properties of that setup are load-bearing:
//!
//! - **The client and the compositor share one thread.** The compositor only
//!   makes progress inside [`Harness::pump`], so a test that would block inside
//!   a connection callback hangs instead of failing -- which is the correct
//!   outcome for a regression here, and is what
//!   [`a_request_arriving_in_pieces_is_answered_once_it_is_whole`] does against
//!   the code this replaced.
//! - **A socket pair, not the real listening socket.** Its peer is this very
//!   process, so the uid check passes -- and, the reason that actually matters,
//!   `SO_SNDBUF` can be set on the compositor's own end before it is handed
//!   over. Shrinking it is the only way a test can make a reply not fit in one
//!   write without pushing megabytes through a debug build.
//!
//! Like `dispatch/tests.rs` and `cursor/tests.rs`, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here uses but which is created either way.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use scoot_core::Config;
use scoot_ipc::{Request, Response, decode, encode};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::fd_pressure::Table;
use crate::compositor::ipc::MAX_TYPE_CHARS;
use crate::compositor::ipc::slots::{MAX_CONNECTIONS, Slots};
use crate::compositor::ipc::tests::set_sndbuf;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;
use crate::compositor::{State, headless};

/// How long any "pump until something happens" loop waits before declaring the
/// compositor to have stopped answering. Generous: these run in a debug build
/// on a VM, and the failure it guards against (a wedged connection) does not
/// get better with time.
const PATIENCE: Duration = Duration::from_secs(20);

/// How many turns of the loop count as "nothing is going to happen". Each one
/// costs at most a millisecond when the loop is genuinely idle.
const SILENCE_ROUNDS: usize = 40;

/// A deliberately tiny send buffer for the compositor's end of a connection.
///
/// The kernel clamps this up (to `SOCK_MIN_SNDBUF`, and it doubles what is
/// asked for), so the real figure is a few kilobytes rather than one -- which
/// is the point: a handful of replies fills it, instead of the ~200 KB a
/// default-sized one takes.
const TINY_SNDBUF: usize = 1024;

/// A deliberately short write-stall deadline, for the eviction tests.
///
/// Short enough that waiting out the two windows an eviction can take is under
/// a second rather than the twenty [`WRITE_STALL_TIMEOUT`] would cost, and long
/// enough to leave real headroom above the one thing that could make a "never
/// evicted" test lie: those tests read a little between pumps, and a whole
/// window passing between two of those reads is an eviction. A debug build on a
/// four-core VM running its suite in parallel is exactly where a test thread
/// can lose a slice, so this is 300ms rather than the 150 it started at --
/// costing a second or so of suite time to put a scheduling stall well outside
/// the window.
///
/// Every other test here connects with the real constant, so nothing that holds
/// a queue across a slow pump loop --
/// `a_client_that_queues_more_than_it_reads_is_held_then_served_in_order` above
/// all -- is at the mercy of this number.
const TINY_STALL: Duration = Duration::from_millis(300);

/// The same idea for the cap on how long a `wait-idle` may park a connection:
/// short enough to wait out, long enough that the hand-off itself (which takes
/// a pump or two) cannot be mistaken for the cap firing.
const TINY_IDLE_WAIT: Duration = Duration::from_millis(300);

/// A compositor with a real event loop, and nothing on screen unless a test
/// asks for it.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
    /// This compositor's connection table. One per harness, so a test can fill
    /// it without any other test's connections in it, and can read off how
    /// many connections are live -- which is also how a test observes that a
    /// closed or evicted one really let go of its fds.
    slots: Slots,
}

impl Harness {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
        )
        .expect("a compositor state with a wayland socket");
        Self {
            event_loop,
            state,
            slots: Slots::new(),
        }
    }

    /// The same, with a real render target -- needed only by the screenshot
    /// test, since a capture has nothing to read back without one.
    fn with_output(width: i32, height: i32) -> Self {
        let mut harness = Self::new();
        headless::init(&mut harness.state, width, height).expect("a render target");
        harness
    }

    /// Hands the compositor one end of a socket pair through the same
    /// `accept()` a real client goes through, and keeps the other.
    ///
    /// With the deadlines production uses, so that a test holding a queue
    /// across a long pump loop is never evicted out from under itself.
    fn connect(&mut self, sndbuf: Option<usize>) -> TestClient {
        self.connect_limited(sndbuf, Limits::REAL)
    }

    /// The same, with a chosen write-stall deadline. For the tests that are
    /// about that deadline.
    fn connect_stalling(&mut self, sndbuf: Option<usize>, stall: Duration) -> TestClient {
        self.connect_limited(
            sndbuf,
            Limits {
                stall,
                ..Limits::REAL
            },
        )
    }

    /// The same, under a given fd-table reading rather than the live one.
    /// For the pressure-refusal tests below, which cannot fill the real
    /// table without starving every sibling test sharing this process's
    /// fds, so they drive `accept_under` with a canned reading instead.
    fn connect_under_table(&mut self, table: Option<Table>) -> TestClient {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        super::super::accept_under(&mut self.state, server, &self.slots, Limits::REAL, table)
            .expect("the connection is taken or refused, not failed");
        // Non-blocking on this side too, for the same deadlock reason as
        // `connect_limited` above.
        client
            .set_nonblocking(true)
            .expect("a non-blocking test client");
        TestClient {
            stream: client,
            received: Vec::new(),
            closed: false,
        }
    }

    /// The same, with both deadlines chosen.
    ///
    /// Note what this does *not* assert: that the connection was accepted at
    /// all. A refusal is a live socket the compositor writes one line to and
    /// closes, so it is the client end that can tell the difference -- see
    /// `the_connection_past_the_cap_is_refused_with_a_reason`.
    fn connect_limited(&mut self, sndbuf: Option<usize>, limits: Limits) -> TestClient {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        if let Some(size) = sndbuf {
            set_sndbuf(&server, size);
        }
        super::super::accept(&mut self.state, server, &self.slots, limits)
            .expect("the connection is taken or refused, not failed");
        // Non-blocking on this side too: the compositor only runs inside
        // `pump`, so a blocking read or write here would deadlock the test
        // against itself rather than test anything.
        client
            .set_nonblocking(true)
            .expect("a non-blocking test client");
        TestClient {
            stream: client,
            received: Vec::new(),
            closed: false,
        }
    }

    /// One turn of the event loop.
    ///
    /// The timeout is small rather than zero so the frame timer (which
    /// `wait-idle` depends on) can come due, and small rather than large so a
    /// test that is waiting for nothing does not pay for it. Either way this
    /// returns as soon as any source is ready.
    fn pump(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(1)), &mut self.state)
            .expect("a compositor dispatch");
    }

    /// Turns the loop for a while. For the tests that are waiting on a
    /// deadline rather than on an answer.
    fn pump_for(&mut self, how_long: Duration) {
        let until = Instant::now() + how_long;
        while Instant::now() < until {
            self.pump();
        }
    }

    /// Turns the loop until `settled` holds, or fails with `complaint`.
    fn pump_until(&mut self, complaint: &str, mut settled: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !settled(self) {
            assert!(Instant::now() < deadline, "{complaint}");
            self.pump();
        }
    }
}

/// The client half of one connection: non-blocking, with whatever has arrived
/// but not yet been claimed as a reply.
struct TestClient {
    stream: UnixStream,
    received: Vec<u8>,
    /// Whether the compositor has closed its end.
    closed: bool,
}

impl TestClient {
    /// Writes all of `bytes`, asserting the socket took them in one go -- true
    /// of any single request that is not deliberately flooding.
    fn send(&mut self, bytes: &[u8]) {
        let written = self.send_some(bytes);
        assert_eq!(
            written,
            bytes.len(),
            "the test's own write was short; it needs to pump and retry"
        );
    }

    /// Writes as much of `bytes` as the socket takes right now.
    fn send_some(&mut self, bytes: &[u8]) -> usize {
        match self.stream.write(bytes) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => 0,
            // The compositor has closed on us, which some tests here expect;
            // noted rather than failed so the test can assert on what arrived
            // before it did.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                self.closed = true;
                0
            }
            Err(error) => panic!("the test client could not write: {error}"),
        }
    }

    /// Writes all of `bytes`, pumping the compositor whenever the socket is
    /// full. For the cases that deliberately send more than a socket holds.
    fn send_all(&mut self, bytes: &[u8], harness: &mut Harness) {
        let mut cursor = 0;
        let deadline = Instant::now() + PATIENCE;
        while cursor < bytes.len() {
            cursor += self.send_some(&bytes[cursor..]);
            harness.pump();
            assert!(
                Instant::now() < deadline,
                "the compositor stopped reading after {cursor} of {} bytes",
                bytes.len()
            );
        }
    }

    /// Takes at most `bytes` (up to a kilobyte) of whatever has arrived.
    ///
    /// For being a *slow* reader rather than a fast one: [`TestClient::collect`]
    /// drains everything available, which empties the compositor's queue in one
    /// go, and a queue that empties is the opposite of the case the write-stall
    /// deadline is about.
    fn sip(&mut self, bytes: usize) -> usize {
        let mut chunk = [0u8; 1024];
        let chunk = &mut chunk[..bytes.min(1024)];
        match self.stream.read(chunk) {
            Ok(0) => {
                self.closed = true;
                0
            }
            Ok(count) => {
                self.received.extend_from_slice(&chunk[..count]);
                count
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => 0,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                self.closed = true;
                0
            }
            Err(error) => panic!("the test client could not read: {error}"),
        }
    }

    /// Collects whatever has arrived, without blocking.
    fn collect(&mut self) {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.closed = true;
                    return;
                }
                Ok(count) => self.received.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                // Closed, the abrupt way. Linux gives the peer `ECONNRESET`
                // rather than a clean end of stream when a unix socket is
                // closed with data still unread in its own receive queue --
                // which is exactly the over-long-request case, where the
                // compositor deliberately stops reading the rest of the line.
                // Anything already queued for this side is still delivered
                // first (the kernel only reports the error once the receive
                // queue is empty), so the refusal itself is not lost.
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                    self.closed = true;
                    return;
                }
                Err(error) => panic!("the test client could not read: {error}"),
            }
        }
    }

    /// The next whole reply line, if one has already arrived.
    fn take_line(&mut self) -> Option<String> {
        let end = self.received.iter().position(|byte| *byte == b'\n')?;
        let line: Vec<u8> = self.received.drain(..=end).collect();
        Some(String::from_utf8(line).expect("a reply is utf-8"))
    }

    /// Pumps until one whole reply arrives, and decodes it. A timeout here
    /// means the compositor stopped answering, which is the failure every test
    /// in this file is about.
    fn expect_reply(&mut self, harness: &mut Harness) -> Response {
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.collect();
            if let Some(line) = self.take_line() {
                return decode(&line).expect("a decodable response");
            }
            assert!(!self.closed, "the connection was closed without a reply");
            assert!(Instant::now() < deadline, "no reply arrived");
            harness.pump();
        }
    }

    /// Pumps for a while and asserts nothing came back.
    ///
    /// This is where a blocking read would hang rather than fail: the
    /// compositor would still be inside the callback when `pump` was called.
    fn expect_silence(&mut self, harness: &mut Harness) {
        for _ in 0..SILENCE_ROUNDS {
            harness.pump();
            self.collect();
        }
        assert!(
            self.take_line().is_none(),
            "a reply arrived that should not have"
        );
        assert!(!self.closed, "the connection was closed unexpectedly");
    }

    /// Pumps until the compositor closes its end, with nothing unread left.
    fn expect_closed(&mut self, harness: &mut Harness) {
        let deadline = Instant::now() + PATIENCE;
        while !self.closed {
            harness.pump();
            self.collect();
            assert!(Instant::now() < deadline, "the connection stayed open");
        }
        assert!(
            self.take_line().is_none(),
            "an unread reply was left behind: {:?}",
            String::from_utf8_lossy(&self.received)
        );
    }

    /// Pumps until `count` replies have arrived, and returns them.
    fn expect_replies(&mut self, harness: &mut Harness, count: usize) -> Vec<Response> {
        let mut replies = Vec::with_capacity(count);
        let deadline = Instant::now() + PATIENCE;
        while replies.len() < count {
            self.collect();
            while let Some(line) = self.take_line() {
                replies.push(decode(&line).expect("a decodable response"));
            }
            if replies.len() >= count {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "only {} of {count} replies arrived",
                replies.len()
            );
            harness.pump();
        }
        replies
    }
}

/// A wayland client that exists only to be flushed.
///
/// Deliberately not a `wayland-client` connection (which `cursor/tests.rs`
/// needs, at the price of a second thread): the only question asked of it is
/// whether bytes the compositor *queued* for it have reached its socket, and a
/// raw fd answers that without a protocol implementation at either end. One
/// request is written by hand for the handshake, and after that it only reads.
struct WaylandProbe {
    socket: UnixStream,
}

impl WaylandProbe {
    /// Connects, asks for a registry, and drains the globals that answer --
    /// leaving the compositor with nothing queued for this client.
    fn connect(harness: &mut Harness) -> Self {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        harness
            .state
            .display_handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("an inserted wayland client");
        client
            .set_nonblocking(true)
            .expect("a non-blocking wayland probe");
        let mut probe = Self { socket: client };

        // `wl_display.get_registry(new_id)`, on the wire: the object the
        // request is for, then the message's length and opcode packed into one
        // word, then the one argument. Native byte order, as wayland's wire
        // format is defined.
        const WL_DISPLAY: u32 = 1;
        const GET_REGISTRY: u32 = 1;
        const REGISTRY_ID: u32 = 2;
        const LENGTH: u32 = 12;
        let mut request = [0u8; LENGTH as usize];
        request[..4].copy_from_slice(&WL_DISPLAY.to_ne_bytes());
        request[4..8].copy_from_slice(&((LENGTH << 16) | GET_REGISTRY).to_ne_bytes());
        request[8..].copy_from_slice(&REGISTRY_ID.to_ne_bytes());
        (&probe.socket)
            .write_all(&request)
            .expect("the probe's handshake is written");

        // The globals come back through the display's own event source, which
        // flushes after dispatching for exactly this reason (see `State::new`).
        let deadline = Instant::now() + PATIENCE;
        while probe.drain() == 0 {
            assert!(
                Instant::now() < deadline,
                "the compositor never answered the probe's get_registry"
            );
            harness.pump();
        }
        // Everything the handshake produced, and then the loop settled, so
        // anything this probe sees later was flushed by the wakeup under test.
        for _ in 0..SILENCE_ROUNDS {
            harness.pump();
            probe.drain();
        }
        probe
    }

    /// Reads and discards whatever has arrived, returning how much that was.
    fn drain(&mut self) -> usize {
        let mut chunk = [0u8; 4096];
        let mut total = 0;
        loop {
            match self.socket.read(&mut chunk) {
                Ok(0) => panic!("the compositor disconnected the wayland probe"),
                Ok(count) => total += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return total,
                Err(error) => panic!("the wayland probe could not read: {error}"),
            }
        }
    }
}

/// Queues one wayland message for every connected client, without flushing it.
///
/// A new global sends `wl_registry.global` to every registry already bound,
/// which wayland-server buffers until something flushes -- the same position a
/// keystroke injected by `Request::Key` is in when its request has been served.
/// Used rather than a keystroke because that needs a focused surface, i.e. a
/// mapped toplevel and a real toolkit client; what is under test is the flush,
/// not what filled the buffer.
fn queue_a_wayland_message(harness: &Harness) -> smithay::output::Output {
    let output = smithay::output::Output::new(
        "flush-probe".to_string(),
        smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "scoot".into(),
            model: "flush-probe".into(),
            serial_number: "0".into(),
        },
    );
    output.create_global::<State>(&harness.state.display_handle);
    // Returned, not dropped: dropping the `Output` would take its global with
    // it, and the point is to leave something queued.
    output
}

fn request_line(request: &Request) -> String {
    encode(request).expect("a request encodes")
}

/// How long one `version` reply is on the wire -- the reply the volume tests
/// here count in, computed rather than hardcoded so a change to the response
/// shape cannot quietly invalidate those counts.
fn reply_line_len() -> usize {
    encode(&Response::Version {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol: scoot_ipc::PROTOCOL_VERSION,
    })
    .expect("a response encodes")
    .len()
}

// --- symptom (a): a half-written request line -----------------------------

#[test]
fn a_request_arriving_in_pieces_is_answered_once_it_is_whole() {
    // Any agent that writes a request in chunks, or gets killed mid-write,
    // does this. Against the blocking version of this code the `pump` inside
    // `expect_silence` never returns: the compositor is parked in `fill_buf`
    // waiting for the rest of the line.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(b"{\"type\":\"ver");
    client.expect_silence(&mut harness);
    client.send(b"sion");
    client.expect_silence(&mut harness);
    client.send(b"\"}\n");
    assert!(matches!(
        client.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn a_half_written_request_does_not_stall_another_connection() {
    // The finding as reported: one connection holding a partial line froze the
    // whole event loop, so every other client's perfectly valid request went
    // unanswered for as long as it held on.
    let mut harness = Harness::new();
    let mut stalled = harness.connect(None);
    let mut other = harness.connect(None);
    stalled.send(b"{\"type\":\"vers");

    other.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        other.expect_reply(&mut harness),
        Response::Version { .. }
    ));
    // ...and the half-written one is still just waiting, not answered wrongly.
    stalled.expect_silence(&mut harness);
    stalled.send(b"ion\"}\n");
    assert!(matches!(
        stalled.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn one_connections_partial_line_does_not_hold_up_many_others() {
    // The same again with the proportions a real session has: several working
    // clients and one stuck one, each working client answered on its own.
    let mut harness = Harness::new();
    let mut stalled = harness.connect(None);
    stalled.send(b"{");
    let mut others: Vec<TestClient> = (0..4).map(|_| harness.connect(None)).collect();
    for client in &mut others {
        client.send(request_line(&Request::Windows).as_bytes());
    }
    for client in &mut others {
        assert!(matches!(
            client.expect_reply(&mut harness),
            Response::Windows { .. }
        ));
    }
    stalled.expect_silence(&mut harness);
}

#[test]
fn several_connections_each_holding_a_partial_line_are_each_answered_their_own() {
    // Not just "one stuck client does not stop the others": several clients
    // stuck at once, finished in an order unrelated to how they started, each
    // answered for the request *it* sent. A connection's half-read line lives
    // on that connection, so mixing two up would show here as the wrong
    // response variant coming back.
    let mut harness = Harness::new();
    let requests = [
        Request::Version,
        Request::Windows,
        Request::Outputs,
        Request::Type {
            text: "hi".to_string(),
        },
    ];
    let mut clients: Vec<TestClient> = Vec::new();
    let mut tails: Vec<String> = Vec::new();
    for request in &requests {
        let line = request_line(request);
        // Split somewhere inside the JSON, at a different point for each.
        let cut = 4 + clients.len();
        let mut client = harness.connect(None);
        client.send(&line.as_bytes()[..cut]);
        tails.push(line[cut..].to_string());
        clients.push(client);
    }
    // Every one of them is mid-request, and none of them has been answered.
    for client in &mut clients {
        client.expect_silence(&mut harness);
    }

    // Finished in an order unrelated to the order they arrived in.
    for index in [2usize, 0, 3, 1] {
        clients[index].send(tails[index].as_bytes());
        let reply = clients[index].expect_reply(&mut harness);
        let matched = matches!(
            (&requests[index], &reply),
            (Request::Version, Response::Version { .. })
                | (Request::Windows, Response::Windows { .. })
                | (Request::Outputs, Response::Outputs { .. })
                | (Request::Type { .. }, Response::Ok { .. })
        );
        assert!(
            matched,
            "connection {index} asked for {:?} and got {reply:?}",
            requests[index]
        );
    }
}

// --- symptom (b): more than one request in a single write -----------------

#[test]
fn every_request_in_one_write_is_answered_not_just_the_first() {
    // Both requests arrive in one kernel read, so the second is drained into
    // the connection's own buffer and no further readiness event will ever be
    // reported for it. Before this change the second reply only came out when
    // unrelated traffic happened to wake the connection again.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let batch = format!(
        "{}{}",
        request_line(&Request::Version),
        request_line(&Request::Windows)
    );
    client.send(batch.as_bytes());
    let replies = client.expect_replies(&mut harness, 2);
    assert!(matches!(replies[0], Response::Version { .. }));
    assert!(matches!(replies[1], Response::Windows { .. }));
}

#[test]
fn a_long_pipeline_in_one_write_is_answered_in_order() {
    // Enough requests to cross several read chunks, so the answers have to
    // survive the connection being picked up and put down repeatedly. Whether
    // it is actually put down mid-batch is the *next* test's job -- this one
    // only cares that nothing is lost or reordered along the way.
    const REQUESTS: usize = 500;
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let batch = request_line(&Request::Version).repeat(REQUESTS);
    client.send_all(batch.as_bytes(), &mut harness);
    let replies = client.expect_replies(&mut harness, REQUESTS);
    assert_eq!(replies.len(), REQUESTS);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. })),
        "a reply came back damaged or out of order"
    );
}

/// A `version` request padded with spaces to exactly `bytes` on the wire.
///
/// `decode` trims the line before parsing, so this is still an ordinary
/// `Request::Version` -- the padding only controls where line boundaries fall
/// relative to the chunks a socket read comes back in, which is the whole point
/// of [`one_wakeup_serves_at_most_one_read_off_the_socket`]. Cheap to answer,
/// unlike a `Request::Type` of the same length.
fn padded_version(bytes: usize) -> String {
    let body = r#"{"type":"version"}"#;
    assert!(bytes > body.len());
    let line = format!("{body:<width$}\n", width = bytes - 1);
    assert_eq!(line.len(), bytes);
    assert!(
        matches!(decode::<Request>(&line), Ok(Request::Version),),
        "padding must not change what the request is"
    );
    line
}

#[test]
fn one_wakeup_serves_at_most_one_read_off_the_socket() {
    // The fairness bound, observed rather than reasoned about: a client that
    // pipelines a large batch in a single `write` must *not* have all of it
    // answered in one callback, because nothing else -- input, the frame timer,
    // wayland dispatch, any other connection -- runs until that callback
    // returns. This is the test that fails against the first version of this
    // PR, which looped on "keep going while something is still buffered".
    //
    // Two things about its shape are load-bearing, both found by measuring
    // rather than by reasoning:
    //
    // - **The batch is large** (160 KB of requests, the scale this PR's
    //   reviewer measured a 48ms stall at). At a few kilobytes, even the
    //   unbounded loop stops soon enough to look fine.
    // - **The request length is an odd prime, not the 19 bytes of a plain
    //   `version` request.** Reads off a real unix socket come back a sender
    //   buffer at a time -- 2641 bytes on the dev VM's kernel -- and the
    //   unbounded loop stops the first time one of those boundaries lands on a
    //   line boundary, i.e. after `lcm(chunk, line)` bytes. 19 divides 2641
    //   exactly, so with `version` requests that is the *first* chunk and the
    //   bug is invisible; with a length coprime to the chunk it is 260 KB away,
    //   i.e. past the end of this batch. 101 is coprime with any chunk size
    //   that is not a multiple of it, so this does not quietly lose its teeth on
    //   a kernel that buffers differently.
    const BYTES: usize = 160 * 1024;
    const LINE: usize = 101;
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let request = padded_version(LINE);
    let chunk = super::super::line::DEFAULT_CAPACITY;
    let batch = request.repeat(BYTES / LINE);
    // One `write`, as much of it as the client's own send buffer takes.
    let pipelined = client.send_some(batch.as_bytes()) / LINE;
    assert!(
        pipelined * LINE > 8 * chunk,
        "only {pipelined} requests went out; the batch has to span many read \
         chunks for this to test anything"
    );

    // Exactly one turn of the event loop.
    harness.pump();
    client.collect();
    let mut served = 0usize;
    while client.take_line().is_some() {
        served += 1;
    }
    assert!(served > 0, "one wakeup served nothing at all");
    assert!(
        served <= chunk / LINE + 1,
        "one wakeup answered {served} of {pipelined} pipelined requests -- more \
         than the one read off the socket it is allowed, so every other source \
         waited for all of them (the +1 is the line straddling the end of that \
         read, which the same read paid for)"
    );

    // The rest still arrive, in order, across the wakeups that follow.
    let rest = client.expect_replies(&mut harness, pipelined - served);
    assert!(
        rest.iter()
            .all(|reply| matches!(reply, Response::Version { .. })),
        "a reply came back damaged after the yield"
    );
}

// --- symptom (c): a client that does not read its replies -----------------

#[test]
fn a_client_that_never_reads_its_replies_stalls_only_itself() {
    // The write-side half of the finding: `write_all` on a blocking socket put
    // the whole compositor to sleep until this client read something, which it
    // never does. Now its replies queue and everyone else carries on.
    let mut harness = Harness::new();
    let mut greedy = harness.connect(Some(TINY_SNDBUF));
    let mut other = harness.connect(None);

    let request = request_line(&Request::Version);
    // Far more answers than a few-kilobyte send buffer can hold.
    let written = greedy.send_some(request.repeat(400).as_bytes());
    let queued = written / request.len();
    assert!(queued > 1, "the test wrote nothing to queue");
    for _ in 0..SILENCE_ROUNDS {
        harness.pump();
    }

    other.send(request.as_bytes());
    assert!(matches!(
        other.expect_reply(&mut harness),
        Response::Version { .. }
    ));

    // And the stalled connection is buffered, not closed or truncated: once it
    // reads, every reply is there, in order, decodable.
    let replies = greedy.expect_replies(&mut harness, queued);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. })),
        "a queued reply came back damaged"
    );
    // It is still a working connection afterwards, too.
    greedy.send(request.as_bytes());
    assert!(matches!(
        greedy.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn a_client_that_queues_more_than_it_reads_is_held_then_served_in_order() {
    // Back-pressure, which is what bounds the outbound queue instead of a cap
    // that would have to refuse or truncate a reply. Written from the outside,
    // the claim is: a client that never reads eventually stops being read
    // *itself* -- and when it does catch up, not one reply was lost.
    let mut harness = Harness::new();
    let mut greedy = harness.connect(Some(TINY_SNDBUF));
    let request = request_line(&Request::Version);
    // Far more requests than the connection can be holding answers to at any
    // one time; the loop below stops as soon as it stops being read.
    let batch = request.repeat(40_000);

    let mut cursor = 0;
    let mut stalled_rounds = 0;
    while cursor < batch.len() && stalled_rounds < SILENCE_ROUNDS {
        let written = greedy.send_some(&batch.as_bytes()[cursor..]);
        cursor += written;
        stalled_rounds = if written == 0 { stalled_rounds + 1 } else { 0 };
        harness.pump();
    }
    let queued = cursor / request.len();
    assert_eq!(
        stalled_rounds,
        SILENCE_ROUNDS,
        "the compositor kept reading all {} bytes without ever pushing back",
        batch.len()
    );
    // And it really was buffering in userspace, not just riding the kernel's
    // own few kilobytes: the replies owed are orders of magnitude past what
    // this connection's (deliberately tiny) send buffer can hold.
    let owed = queued * reply_line_len();
    assert!(
        owed > 16 * TINY_SNDBUF,
        "only {owed} bytes of replies were owed; nothing was really queued"
    );

    let replies = greedy.expect_replies(&mut harness, queued);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. })),
        "a queued reply came back damaged"
    );
    // Finish off whatever request the stall cut in half -- or start the next
    // one, if it happened to cut cleanly. Either way this completes exactly one
    // more request, and an answer to it is the proof that a connection which
    // stopped being read is read again once it catches up. (Sending a *fresh*
    // request here instead would append to the half-written line still in the
    // compositor's buffer and decode as garbage, which is correct behavior and
    // a useless assertion.)
    let next = request.len() - (cursor % request.len());
    greedy.send(&batch.as_bytes()[cursor..cursor + next]);
    match greedy.expect_reply(&mut harness) {
        Response::Version { .. } => {}
        other => panic!("the connection was not read again after catching up: {other:?}"),
    }
}

// --- closing cleanly ------------------------------------------------------

#[test]
fn a_client_that_shuts_down_its_write_half_still_gets_its_reply() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Version).as_bytes());
    client
        .stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shuts down writing");
    assert!(matches!(
        client.expect_reply(&mut harness),
        Response::Version { .. }
    ));
    client.expect_closed(&mut harness);
}

#[test]
fn a_client_that_stops_writing_with_replies_still_queued_gets_all_of_them() {
    // The truncation this avoids: the end of the client's requests is also
    // where the connection would be closed, and closing drops anything still
    // queued. So `closing` has to mean "no more requests", not "close now".
    let mut harness = Harness::new();
    let mut client = harness.connect(Some(TINY_SNDBUF));
    let request = request_line(&Request::Version);
    let written = client.send_some(request.repeat(400).as_bytes());
    let queued = written / request.len();
    assert!(queued > 1, "the test wrote nothing to queue");
    client
        .stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shuts down writing");

    let replies = client.expect_replies(&mut harness, queued);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. }))
    );
    client.expect_closed(&mut harness);
}

#[test]
fn a_client_that_disconnects_mid_request_is_simply_forgotten() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(b"{\"type\":\"vers");
    harness.pump();
    drop(client);
    // Nothing panics, and the loop is still serving: a fresh connection works.
    let mut other = harness.connect(None);
    other.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        other.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

// --- the request-line cap, across partial reads ---------------------------

#[test]
fn an_over_long_request_line_is_refused_across_however_many_writes_it_takes() {
    // Item 9's cap, now that a line can take any number of reads to arrive:
    // the limit has to hold over the whole line, not per read, or preserving
    // the prefix would have handed a client a way around it.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let chunk = vec![b'x'; 64 * 1024];
    let mut sent = 0usize;
    let deadline = Instant::now() + PATIENCE;
    while sent <= MAX_REQUEST_BYTES && Instant::now() < deadline {
        let written = client.send_some(&chunk);
        if written == 0 && client.closed {
            break;
        }
        sent += written;
        harness.pump();
        client.collect();
        if client.closed {
            break;
        }
    }
    match client.expect_reply(&mut harness) {
        Response::Error { message } => assert!(
            message.contains("exceeds"),
            "wrong refusal message: {message}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    client.expect_closed(&mut harness);
}

#[test]
fn a_large_but_legal_request_split_across_writes_still_works() {
    // The other side of the same cap: a real `Request::Type` with a big paste,
    // arriving in pieces, must still be answered rather than refused. Sized
    // under [`MAX_TYPE_CHARS`] (the per-`type` cap below) but over one
    // [`super::super::line::DEFAULT_CAPACITY`] read chunk, so it still spans
    // several reads the way the half-megabyte paste this used to send did.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let request = request_line(&Request::Type {
        text: "x".repeat(10_000),
    });
    assert!(request.len() < MAX_REQUEST_BYTES);
    assert!(request.len() > super::super::line::DEFAULT_CAPACITY);
    client.send_all(request.as_bytes(), &mut harness);
    // `Response::Ok` exactly, not "anything but a crash": a reassembly that
    // lost or duplicated a chunk decodes as a *different* failure
    // (`Response::Error` carrying a serde message), so accepting an error here
    // would pass against the very bug this is for.
    match client.expect_reply(&mut harness) {
        Response::Ok { .. } => {}
        other => panic!("the request did not survive being split up: {other:?}"),
    }
}

// --- the `type` text cap ----------------------------------------------------

/// An over-cap `type` is refused before a single character is typed, in both
/// character classes: shifted text costs ~2x (four key events per character,
/// not two), so the cap has to refuse the worst case, not the mean.
#[test]
fn an_over_cap_type_is_refused_before_anything_is_typed() {
    for text in [
        "x".repeat(MAX_TYPE_CHARS + 1),
        "X".repeat(MAX_TYPE_CHARS + 1),
        // Multi-byte past the cap, refused for its length before `type_text`
        // ever gets to refuse it for its content (`é` is not on a US layout
        // -- see `input::tests::an_untypable_character_errors...`).
        "\u{e9}".repeat(MAX_TYPE_CHARS + 1),
    ] {
        let mut harness = Harness::new();
        let mut client = harness.connect(None);
        client.send(request_line(&Request::Type { text }).as_bytes());
        match client.expect_reply(&mut harness) {
            Response::Error { message } => {
                assert!(
                    message.contains(&MAX_TYPE_CHARS.to_string()),
                    "the refusal must name the limit: {message}"
                );
                assert!(
                    message.to_lowercase().contains("split"),
                    "the refusal must name the workaround: {message}"
                );
            }
            other => panic!("an over-cap type was not refused, got {other:?}"),
        }
    }
}

/// Exactly at the cap is still served: the boundary belongs to the client,
/// in both character classes (the cap refuses the worst case, so it must
/// serve it at the boundary too).
#[test]
fn a_type_at_exactly_the_cap_is_still_served() {
    for text in ["x".repeat(MAX_TYPE_CHARS), "X".repeat(MAX_TYPE_CHARS)] {
        let mut harness = Harness::new();
        let mut client = harness.connect(None);
        client.send(request_line(&Request::Type { text }).as_bytes());
        match client.expect_reply(&mut harness) {
            Response::Ok { .. } => {}
            other => panic!("an at-cap type was not served, got {other:?}"),
        }
    }
}

/// The cap counts characters, not bytes: each character becomes key events,
/// so characters are the cost unit whatever their UTF-8 length.
///
/// No multi-byte character the US layout can type exists to assert `Ok`
/// with (that layout has no `é` -- see
/// `input::tests::an_untypable_character_errors...`), so this pins the
/// choice from the other side instead: 9,000 `é` is 18,000 bytes on the
/// wire -- past the number -- but only 9,000 characters. A bytes-counted
/// cap would refuse it naming the limit; the characters-counted one lets
/// it through to `type_text`, which refuses it for its own reason. The
/// message tells the two refusals apart.
#[test]
fn the_type_cap_counts_characters_not_bytes() {
    let text = "é".repeat(9_000);
    assert_eq!(text.chars().count(), 9_000);
    assert!(
        text.len() > MAX_TYPE_CHARS,
        "the test string must be more bytes than the cap's number"
    );
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Type { text }).as_bytes());
    match client.expect_reply(&mut harness) {
        Response::Error { message } => {
            assert!(
                message.contains('é'),
                "this refusal should come from the layout, naming the character: {message}"
            );
            assert!(
                !message.contains(&MAX_TYPE_CHARS.to_string()),
                "the cap fired on bytes: {message}"
            );
        }
        other => panic!("a within-cap multi-byte type should reach the layout, got {other:?}"),
    }
}

/// The everyday path, pinned: a realistic few-hundred-character shell command
/// line is orders of magnitude under the cap and must be unaffected by it.
#[test]
fn a_realistic_few_hundred_character_type_is_unaffected() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let text = "git log --oneline -20 -- crates/scoot/src/compositor/ipc.rs | head -40 && \
                cargo test -p scoot --lib compositor::ipc 2>&1 | tail -5; echo done";
    assert!(text.len() < 300, "the test string drifted from realistic");
    client.send(
        request_line(&Request::Type {
            text: text.to_string(),
        })
        .as_bytes(),
    );
    match client.expect_reply(&mut harness) {
        Response::Ok { .. } => {}
        other => panic!("an ordinary type was not served, got {other:?}"),
    }
}

/// Empty text is accepted and is a no-op: this confirms the current behavior
/// rather than changing it, so the cap cannot turn a harmless request into
/// an error.
#[test]
fn an_empty_type_is_accepted_as_a_no_op() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(
        request_line(&Request::Type {
            text: String::new(),
        })
        .as_bytes(),
    );
    match client.expect_reply(&mut harness) {
        Response::Ok { .. } => {}
        other => panic!("an empty type was not served, got {other:?}"),
    }
}

// --- wait-idle ------------------------------------------------------------

#[test]
fn wait_idle_is_still_answered_and_then_closes_its_connection() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(
        request_line(&Request::WaitIdle {
            quiet_ms: 10,
            timeout_ms: 5_000,
        })
        .as_bytes(),
    );
    match client.expect_reply(&mut harness) {
        Response::Idle { .. } => {}
        other => panic!("expected an idle reply, got {other:?}"),
    }
    // Its source left the event loop when it handed over, which is how this
    // has worked since `wait-idle` was added.
    client.expect_closed(&mut harness);
}

#[test]
fn wait_idle_still_times_out_rather_than_waiting_forever() {
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(
        request_line(&Request::WaitIdle {
            quiet_ms: 60_000,
            timeout_ms: 30,
        })
        .as_bytes(),
    );
    match client.expect_reply(&mut harness) {
        Response::Error { message } => {
            assert!(message.contains("timed out"), "wrong message: {message}");
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    client.expect_closed(&mut harness);
}

#[test]
fn an_idle_reply_does_not_overtake_a_reply_still_going_out() {
    // The framing hazard in the hand-off: `wait-idle` takes the connection out
    // of the event loop, so anything still queued there has to travel with it
    // -- otherwise the idle answer, written through a clone of the same socket,
    // lands in the middle of a half-written response.
    const REQUESTS: usize = 300;
    let mut harness = Harness::new();
    let mut client = harness.connect(Some(TINY_SNDBUF));
    let batch = format!(
        "{}{}",
        request_line(&Request::Version).repeat(REQUESTS),
        request_line(&Request::WaitIdle {
            quiet_ms: 10,
            timeout_ms: 20_000,
        })
    );
    // Asserted, not assumed: the whole batch has to fit one read buffer, so
    // that every one of those replies is queued up behind a send buffer that
    // cannot take them *before* the `wait-idle` line is reached. Without that
    // this test would quietly stop exercising the hand-off rather than fail.
    assert!(
        batch.len() < super::super::line::DEFAULT_CAPACITY,
        "the batch no longer arrives in one read"
    );
    client.send_all(batch.as_bytes(), &mut harness);

    let replies = client.expect_replies(&mut harness, REQUESTS + 1);
    for (index, reply) in replies.iter().enumerate().take(REQUESTS) {
        assert!(
            matches!(reply, Response::Version { .. }),
            "reply {index} came back as {reply:?}"
        );
    }
    match &replies[REQUESTS] {
        Response::Idle { .. } => {}
        other => panic!("the idle answer was not last: {other:?}"),
    }
    client.expect_closed(&mut harness);
}

#[test]
fn the_wayland_side_is_flushed_even_when_the_request_served_ends_the_connection() {
    // The flush that used to happen per request now happens once per wakeup,
    // which is only equivalent if *every* way out of the serve loop goes
    // through it. `wait-idle` is the exit that makes that load-bearing: it is
    // answered later, from the frame timer, and the frame timer's own
    // `render()` does nothing when nothing marked the screen dirty -- which
    // injected input does not. So a skipped flush here is not a latency bug: a
    // pipelined `key` + `wait-idle` would leave the keystroke in the client's
    // buffer, the client could not have redrawn for a key it never saw, and the
    // wait would answer `idle` immediately. (See `idle_outcome`, whose own doc
    // names that race as the reason it exists.)
    //
    // One `wait-idle` and nothing else, so the only line served this wakeup is
    // the one that closes the connection: a test with a preceding request would
    // still pass if the flush were merely moved to after `serve`.
    let mut harness = Harness::new();
    let mut probe = WaylandProbe::connect(&mut harness);
    let mut client = harness.connect(None);
    let _output = queue_a_wayland_message(&harness);
    assert_eq!(
        probe.drain(),
        0,
        "something flushed the probe before the wakeup under test; this test \
         can no longer tell who flushed it"
    );

    client.send(
        request_line(&Request::WaitIdle {
            quiet_ms: 10,
            timeout_ms: 5_000,
        })
        .as_bytes(),
    );
    // Exactly one turn: the connection's readable wakeup. Nothing else is ready
    // -- the probe sent nothing, and the frame timer `wait-idle` arms is only
    // inserted during this very callback, so it cannot run until the next turn.
    harness.pump();
    assert!(
        probe.drain() > 0,
        "the wakeup served a request and returned without flushing the wayland \
         clients"
    );

    // ...and the hand-off itself still works, so this is not passing because
    // the request was mishandled.
    match client.expect_reply(&mut harness) {
        Response::Idle { .. } => {}
        other => panic!("expected an idle reply, got {other:?}"),
    }
}

// --- the screenshot limiter, across a pipelined write ---------------------

#[test]
fn two_screenshots_in_one_write_are_throttled_like_two_separate_ones() {
    // The limiter is per request, so draining several lines in one wakeup must
    // not let a connection past it. A small output keeps the first reply well
    // under the high-water mark, so back-pressure plays no part in the result.
    //
    // The order is the async consequence to note: the refusal is answered
    // immediately, while the capture it follows is still encoding on the
    // worker -- so the *second* request's reply arrives *first*. A client
    // reading replies in request order must match them by content, not by
    // position, for this pair. (A request pipelined behind a capture that is
    // *not* refused -- `version`, below -- is held instead, precisely so it
    // cannot overtake.)
    let mut harness = Harness::with_output(32, 32);
    let mut client = harness.connect(None);
    let batch = request_line(&Request::Screenshot { output: None }).repeat(2);
    client.send(batch.as_bytes());
    let replies = client.expect_replies(&mut harness, 2);
    match &replies[0] {
        Response::Error { message } => assert!(
            message.contains("limited to one per connection"),
            "wrong refusal: {message}"
        ),
        other => panic!("the second capture was not refused: {other:?}"),
    }
    assert!(
        matches!(replies[1], Response::Screenshot(_)),
        "the first capture failed: {:?}",
        replies[1]
    );
}

#[test]
fn one_connections_screenshot_limit_does_not_refuse_anothers() {
    let mut harness = Harness::with_output(32, 32);
    let mut first = harness.connect(None);
    let mut second = harness.connect(None);
    let shot = request_line(&Request::Screenshot { output: None });
    first.send(shot.as_bytes());
    assert!(matches!(
        first.expect_reply(&mut harness),
        Response::Screenshot(_)
    ));
    second.send(shot.as_bytes());
    assert!(
        matches!(second.expect_reply(&mut harness), Response::Screenshot(_)),
        "a second connection was refused its first capture"
    );
}

// --- screenshots encode off the event-loop thread -------------------------
//
// The worker is a real thread, so every test here that needs "still
// encoding" to hold while it acts uses a 1600x1000 framebuffer: at that size
// even a release encode takes double-digit milliseconds, and a debug one
// hundreds, while the test itself acts within a pump or two (single-digit
// milliseconds). The assertions that would go quiet if that ever stopped
// being true say so instead of passing vacuously.

#[test]
fn a_screenshot_waits_for_replies_still_going_out() {
    // The framing hazard in dispatch: a capture's reply goes out later,
    // through a clone of this socket, so dispatching one while earlier
    // replies are still queued would land it in the middle of a half-written
    // response. Refused with a retry instead -- deterministically here,
    // because the whole batch arrives in one read: small enough to fit, so
    // the capture is served in the same wakeup as the replies that block it.
    let mut harness = Harness::with_output(32, 32);
    let mut client = harness.connect(Some(TINY_SNDBUF));
    let batch = format!(
        "{}{}",
        request_line(&Request::Version).repeat(30),
        request_line(&Request::Screenshot { output: None })
    );
    assert!(
        batch.len() < super::super::line::DEFAULT_CAPACITY,
        "the batch no longer arrives in one read"
    );
    client.send_all(batch.as_bytes(), &mut harness);

    let replies = client.expect_replies(&mut harness, 31);
    for (index, reply) in replies.iter().enumerate().take(30) {
        assert!(
            matches!(reply, Response::Version { .. }),
            "reply {index} came back as {reply:?}"
        );
    }
    match &replies[30] {
        Response::Error { message } => assert!(
            message.contains("still going out"),
            "wrong refusal: {message}"
        ),
        other => panic!("the capture was dispatched over a queued reply: {other:?}"),
    }

    // ...and once the queue has drained, the retry works.
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    assert!(
        matches!(client.expect_reply(&mut harness), Response::Screenshot(_)),
        "the retried capture failed"
    );
}

#[test]
fn a_screenshot_without_a_backend_is_an_error_not_a_panic() {
    // The capture runs before it can fail; with no render target there is
    // nothing to read back. `Harness::new` (no `with_output`) is that state.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    match client.expect_reply(&mut harness) {
        Response::Error { message } => {
            assert!(message.contains("no backend"), "wrong error: {message}")
        }
        other => panic!("expected an error, got {other:?}"),
    }
    // Still a working connection afterwards, too.
    client.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        client.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn a_screenshot_is_answered_and_the_connection_stays_usable() {
    let mut harness = Harness::with_output(32, 32);
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    match client.expect_reply(&mut harness) {
        Response::Screenshot(shot) => {
            assert_eq!((shot.width, shot.height), (32, 32));
            assert!(!shot.png.is_empty());
        }
        other => panic!("the capture failed: {other:?}"),
    }
    assert_eq!(harness.state.pending_shot_count(), 0);
    // The ordering gate lifts once the reply has gone out: this connection
    // answers normally again.
    client.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        client.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn another_connections_version_is_answered_while_a_capture_encodes() {
    // The ticket's stall, observed rather than reasoned about: while one
    // connection's capture is still encoding, another connection is answered
    // -- which the synchronous encode could never do, parked as it was in
    // `serve` for the whole ~12ms.
    let mut harness = Harness::with_output(1600, 1000);
    let mut camera = harness.connect(None);
    let mut other = harness.connect(None);
    camera.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    harness.pump();
    assert_eq!(
        harness.state.pending_shot_count(),
        1,
        "the capture was not dispatched"
    );

    other.send(request_line(&Request::Version).as_bytes());
    assert!(
        matches!(other.expect_reply(&mut harness), Response::Version { .. }),
        "another connection waited on this one's encode"
    );
    assert_eq!(
        harness.state.pending_shot_count(),
        1,
        "the capture finished before the other connection was answered -- \
         this test no longer proves anything"
    );

    assert!(
        matches!(camera.expect_reply(&mut harness), Response::Screenshot(_)),
        "the capture itself was lost"
    );
    assert_eq!(harness.state.pending_shot_count(), 0);
}

#[test]
fn a_request_pipelined_behind_a_capture_is_refused_until_it_lands() {
    // The ordering half: a reply to a request sent *after* a capture must
    // not overtake the capture's, so it is refused with a retry instead.
    // Deterministic for the same reason as the test above -- the encode
    // outlasts these pumps -- with the refusal collected after exactly one
    // further turn, before the worker could possibly have delivered.
    let mut harness = Harness::with_output(1600, 1000);
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    harness.pump();
    client.send(request_line(&Request::Version).as_bytes());
    harness.pump();
    client.collect();
    match client.take_line() {
        Some(line) => match decode::<Response>(&line).expect("a decodable response") {
            Response::Error { message } => assert!(
                message.contains("still being encoded"),
                "wrong refusal: {message}"
            ),
            other => panic!("the pipelined request was served out of order: {other:?}"),
        },
        None => panic!("the pipelined request was left unanswered"),
    }
    assert!(
        matches!(client.expect_reply(&mut harness), Response::Screenshot(_)),
        "the capture itself was lost"
    );
    // ...and the gate lifts with the reply: the refused request works now.
    client.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        client.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn a_wait_idle_behind_a_capture_is_refused_until_it_lands() {
    // The worst swap the gate prevents: a `wait-idle` parking its connection
    // while its capture's answer is still owed, leaving the reply to arrive
    // on a connection that already handed over and closed.
    let mut harness = Harness::with_output(1600, 1000);
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    harness.pump();
    client.send(
        request_line(&Request::WaitIdle {
            quiet_ms: 10,
            timeout_ms: 5_000,
        })
        .as_bytes(),
    );
    harness.pump();
    client.collect();
    match client.take_line() {
        Some(line) => match decode::<Response>(&line).expect("a decodable response") {
            Response::Error { message } => assert!(
                message.contains("still being encoded"),
                "wrong refusal: {message}"
            ),
            other => panic!("the wait-idle was parked behind a capture: {other:?}"),
        },
        None => panic!("the wait-idle was left unanswered"),
    }
    assert!(
        harness.state.pending_idle.is_empty(),
        "the refused wait-idle parked anyway"
    );
    assert!(
        matches!(client.expect_reply(&mut harness), Response::Screenshot(_)),
        "the capture itself was lost"
    );
}

#[test]
fn a_capture_is_refused_while_the_encoder_is_full() {
    use crate::compositor::screenshot::MAX_IN_FLIGHT_SHOTS;

    // Each in-flight capture holds a full frame of pixels, so the worker
    // queue is bounded -- past it a capture is refused with a retry rather
    // than queued without bound. Parked deterministically through the
    // test-only helper (relying on a real burst to fill the queue would be
    // timing, not testing).
    // The encoder is spawned first, with a real capture: entries parked
    // without a live encoder are orphans by construction, and the next
    // request reaps those instead of refusing past them (see below).
    let mut harness = Harness::with_output(32, 32);
    let mut seeder = harness.connect(None);
    let shot = request_line(&Request::Screenshot { output: None });
    seeder.send(shot.as_bytes());
    assert!(
        matches!(seeder.expect_reply(&mut harness), Response::Screenshot(_)),
        "the seeding capture failed"
    );
    for conn in 0..MAX_IN_FLIGHT_SHOTS as u64 {
        harness.state.park_test_shot(1000 + conn);
    }
    assert_eq!(harness.state.pending_shot_count(), MAX_IN_FLIGHT_SHOTS);

    let mut client = harness.connect(None);
    client.send(shot.as_bytes());
    match client.expect_reply(&mut harness) {
        Response::Error { message } => {
            assert!(message.contains("busy"), "wrong refusal: {message}")
        }
        other => panic!("a capture past the bound was accepted: {other:?}"),
    }
    assert_eq!(
        harness.state.pending_shot_count(),
        MAX_IN_FLIGHT_SHOTS,
        "a refused capture parked itself anyway"
    );
}

#[test]
fn respawning_the_encoder_reaps_captures_its_predecessor_left_behind() {
    use crate::compositor::screenshot::MAX_IN_FLIGHT_SHOTS;

    // The latent wedge: entries parked by a worker that then dies can never
    // complete, and counting them toward the bound would refuse every future
    // screenshot until restart. The respawn answers them with an error and
    // releases them instead -- pinned deterministically (the parked entries
    // never complete on their own), through the same `drop_encoder` hook the
    // respawn test uses. Same setup as the full-encoder test above, except
    // the worker is gone when the new capture arrives.
    let mut harness = Harness::with_output(32, 32);
    let mut seeder = harness.connect(None);
    let shot = request_line(&Request::Screenshot { output: None });
    seeder.send(shot.as_bytes());
    assert!(
        matches!(seeder.expect_reply(&mut harness), Response::Screenshot(_)),
        "the seeding capture failed"
    );
    for conn in 0..MAX_IN_FLIGHT_SHOTS as u64 {
        harness.state.park_test_shot(1000 + conn);
    }
    harness.state.drop_encoder();

    // Without the reap this is refused as busy: four orphans still counted.
    // Exactly one pump: the new capture is dispatched (and parked), while a
    // completion -- real or orphaned -- needs another turn to be processed,
    // so the count here is reap-plus-dispatch and nothing else.
    let mut client = harness.connect(None);
    client.send(shot.as_bytes());
    harness.pump();
    assert_eq!(
        harness.state.pending_shot_count(),
        1,
        "the orphans were not reaped, or the new capture was not parked"
    );
    assert!(
        matches!(client.expect_reply(&mut harness), Response::Screenshot(_)),
        "no capture arrived after the encoder went away with a full queue"
    );
    assert_eq!(harness.state.pending_shot_count(), 0);
}

#[test]
fn a_client_that_disconnects_mid_encode_is_dropped_cleanly() {
    // No panic, no wedged worker, no reply written anywhere: the completion
    // finds a closed peer, fails its write, and the entry is dropped. The
    // compositor carries on serving everyone else.
    let mut harness = Harness::with_output(1600, 1000);
    let mut client = harness.connect(None);
    client.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    harness.pump();
    assert_eq!(harness.state.pending_shot_count(), 1);
    drop(client);
    harness.pump_until("the orphaned capture never finished", |harness| {
        harness.state.pending_shot_count() == 0
    });

    let mut other = harness.connect(None);
    other.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        other.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

#[test]
fn a_capture_mid_wait_does_not_move_the_idle_baseline() {
    // `wait-idle` watches `last_commit`, which only a client commit writes
    // -- the capture's synchronous render never touches it, and the encode
    // in flight parks no waiter. So a capture mid-wait neither extends the
    // quiet window nor wedges the waiter: the wait is answered `idle` on its
    // own clock, and the capture lands alongside it.
    let mut harness = Harness::with_output(32, 32);
    let mut waiter = harness.connect(None);
    waiter.send(
        request_line(&Request::WaitIdle {
            quiet_ms: 50,
            timeout_ms: 5_000,
        })
        .as_bytes(),
    );
    harness.pump_until("the wait-idle never handed over", |harness| {
        !harness.state.pending_idle.is_empty()
    });
    let baseline = harness.state.last_commit;

    let mut camera = harness.connect(None);
    camera.send(request_line(&Request::Screenshot { output: None }).as_bytes());
    assert!(
        matches!(camera.expect_reply(&mut harness), Response::Screenshot(_)),
        "the capture failed"
    );
    assert_eq!(
        harness.state.last_commit, baseline,
        "capturing moved the clock wait-idle watches"
    );
    match waiter.expect_reply(&mut harness) {
        Response::Idle { .. } => {}
        other => panic!("the waiter was wedged or timed out by the capture: {other:?}"),
    }
}

#[test]
fn the_encoder_serves_again_after_going_away() {
    // The worker exits when its job channel disconnects -- which dropping
    // the encoder does -- and the next request spawns a fresh one instead of
    // hanging every screenshot after it. (The first worker may still be
    // exiting while the second serves; the pairs are independent, so they
    // cannot interfere. The abandoned completion source stays registered and
    // simply never fires.)
    //
    // A second connection for the second capture, not a wait: the rate limit
    // would refuse two captures this close together on one connection, and
    // sleeping out the frame would be timing, not testing.
    let mut harness = Harness::with_output(32, 32);
    let mut first = harness.connect(None);
    let mut second = harness.connect(None);
    let shot = request_line(&Request::Screenshot { output: None });
    first.send(shot.as_bytes());
    assert!(
        matches!(first.expect_reply(&mut harness), Response::Screenshot(_)),
        "the first capture failed"
    );
    harness.state.drop_encoder();
    second.send(shot.as_bytes());
    assert!(
        matches!(second.expect_reply(&mut harness), Response::Screenshot(_)),
        "no capture arrived after the encoder went away"
    );
}

// --- what the connection asks the event loop for --------------------------

#[test]
fn a_connection_waits_for_writable_only_while_something_is_queued() {
    // The registration rule: never both at once. Staying registered for
    // readable bytes while deliberately not reading them would spin a
    // level-triggered loop at full speed, and staying registered for writable
    // with nothing to write would do the same.
    let (server, client) = UnixStream::pair().expect("a socket pair");
    set_sndbuf(&server, TINY_SNDBUF);
    server.set_nonblocking(true).expect("non-blocking");
    let slots = Slots::new();
    let slot = slots.claim().expect("a slot");
    let mut connection = source(server, slot, Limits::REAL, 1)
        .expect("a connection")
        .connection;
    assert!(connection.interest().readable);
    assert!(!connection.interest().writable);

    // Fill the socket with a reply nobody is reading.
    connection
        .reply(&Response::Warning {
            message: "x".repeat(256 * 1024),
        })
        .expect("queues");
    assert!(!connection.outbound.is_empty(), "the write should not fit");
    assert!(connection.interest().writable);
    assert!(
        !connection.interest().readable,
        "a connection with a queue must stop being read, or the loop spins"
    );

    // Drain it from the other end and the queue empties again.
    let mut sink = client;
    let mut chunk = [0u8; 64 * 1024];
    let deadline = Instant::now() + PATIENCE;
    while !connection.outbound.is_empty() {
        let read = sink.read(&mut chunk).expect("the client reads");
        assert!(read > 0, "the socket closed");
        connection.flush().expect("flushes");
        assert!(Instant::now() < deadline, "the queue never drained");
    }
    assert!(connection.interest().readable);
    assert!(!connection.interest().writable);
}

// --- how many connections there may be ------------------------------------

/// Fills the connection table, holding on to every client so none of them can
/// close itself and free a slot.
fn fill_the_table(harness: &mut Harness) -> Vec<TestClient> {
    let clients: Vec<TestClient> = (0..MAX_CONNECTIONS)
        .map(|_| harness.connect(None))
        .collect();
    assert_eq!(
        harness.slots.live(),
        MAX_CONNECTIONS,
        "the table did not fill"
    );
    clients
}

#[test]
fn the_connection_past_the_cap_is_refused_with_a_reason() {
    // What the cap is for: the per-connection screenshot limit (and every
    // other per-connection bound here) is only a bound at all if connections
    // are bounded too.
    let mut harness = Harness::new();
    let mut held = fill_the_table(&mut harness);

    let mut refused = harness.connect(None);
    match refused.expect_reply(&mut harness) {
        Response::Error { message } => assert!(
            message.contains("at most"),
            "the refusal did not say why: {message}"
        ),
        other => panic!("a connection past the cap was served: {other:?}"),
    }
    // And it is a refusal, not a queue: the socket is closed straight away
    // rather than held until a slot frees up.
    refused.expect_closed(&mut harness);

    // The refusal itself cost nothing -- no slot taken, and the connections
    // that do hold them still work.
    assert_eq!(
        harness.slots.live(),
        MAX_CONNECTIONS,
        "a refused connection took a slot anyway"
    );
    let first = held.first_mut().expect("a connection that was let in");
    first.send(request_line(&Request::Version).as_bytes());
    assert!(matches!(
        first.expect_reply(&mut harness),
        Response::Version { .. }
    ));
}

// --- fd-pressure refusals ----------------------------------------------------

#[test]
fn a_connection_under_fd_pressure_is_refused_with_a_reason() {
    // What the global ceiling is for on this socket: the table is nearly
    // full, so the newcomer is refused outright -- told why, unlike a shed
    // Wayland connection, because this channel can carry the reason -- and
    // costs the table nothing.
    let mut harness = Harness::new();
    let mut refused = harness.connect_under_table(Some(Table {
        used: 1024,
        soft: 1024,
    }));
    match refused.expect_reply(&mut harness) {
        Response::Error { message } => assert!(
            message.contains("pressure"),
            "the refusal did not say why: {message}"
        ),
        other => panic!("a connection under pressure was served: {other:?}"),
    }
    // And it is a refusal, not a queue: the socket is closed straight away.
    refused.expect_closed(&mut harness);

    // The refusal itself cost nothing -- no slot taken...
    assert_eq!(
        harness.slots.live(),
        0,
        "a pressure-refused connection took a slot anyway"
    );
    // ...and whoever is already connected still works: pressure sheds
    // newcomers, it never touches the living.
    let mut fresh = harness.connect_under_table(None);
    fresh.send(request_line(&Request::Version).as_bytes());
    assert!(
        matches!(fresh.expect_reply(&mut harness), Response::Version { .. }),
        "a calm newcomer was not served after a pressure refusal"
    );
}

#[test]
fn a_calm_fd_table_admits() {
    // An observed table with headroom is served exactly like a connection
    // through the live path.
    let mut harness = Harness::new();
    let mut fresh = harness.connect_under_table(Some(Table {
        used: 14,
        soft: 1024,
    }));
    fresh.send(request_line(&Request::Version).as_bytes());
    assert!(
        matches!(fresh.expect_reply(&mut harness), Response::Version { .. }),
        "a calm table did not admit"
    );
    assert_eq!(harness.slots.live(), 1);
}

#[test]
fn an_unknown_fd_table_admits() {
    // Fail open: a broken gauge must not deny innocents (the `EMFILE`
    // shed still catches real exhaustion underneath).
    let mut harness = Harness::new();
    let mut fresh = harness.connect_under_table(None);
    fresh.send(request_line(&Request::Version).as_bytes());
    assert!(
        matches!(fresh.expect_reply(&mut harness), Response::Version { .. }),
        "an unknown table did not admit"
    );
    assert_eq!(harness.slots.live(), 1);
}

#[test]
fn a_closed_connection_gives_its_slot_back() {
    let mut harness = Harness::new();
    let mut held = fill_the_table(&mut harness);

    // A client goes away, the compositor notices, and the table has room
    // again. (Dropping the `TestClient` closes its fd, which is the end of
    // stream the connection loop reads.)
    held.pop();
    harness.pump_until("the closed connection was never noticed", |harness| {
        harness.slots.live() < MAX_CONNECTIONS
    });
    assert_eq!(harness.slots.live(), MAX_CONNECTIONS - 1);

    let mut fresh = harness.connect(None);
    fresh.send(request_line(&Request::Version).as_bytes());
    assert!(
        matches!(fresh.expect_reply(&mut harness), Response::Version { .. }),
        "the freed slot was not handed out again"
    );
    assert_eq!(harness.slots.live(), MAX_CONNECTIONS);
}

#[test]
fn a_wait_idle_hand_off_keeps_its_slot() {
    // A `wait-idle` leaves the event loop but keeps its socket, answered later
    // from the render loop. If the slot went back to the table there, a client
    // could shed the cap entirely: park each connection in a `wait-idle` that
    // never comes due, and hold an unbounded number of fds in `pending_idle`.
    let mut harness = Harness::new();
    let mut held = fill_the_table(&mut harness);

    let waiter = held.last_mut().expect("a connection to hand off");
    waiter.send(
        request_line(&Request::WaitIdle {
            // Never quiet enough to be answered; only the timeout ends it.
            quiet_ms: u64::MAX,
            timeout_ms: 150,
        })
        .as_bytes(),
    );
    harness.pump_until("the wait-idle never handed over", |harness| {
        !harness.state.pending_idle.is_empty()
    });
    assert_eq!(
        harness.slots.live(),
        MAX_CONNECTIONS,
        "the hand-off released the slot its socket is still using"
    );
    let mut refused = harness.connect(None);
    assert!(
        matches!(refused.expect_reply(&mut harness), Response::Error { .. }),
        "a connection was let in over the cap while a waiter still held a slot"
    );

    // And once the waiter is finished with -- timed out, answered, written --
    // the slot really does come back.
    harness.pump_until("the waiter never timed out", |harness| {
        harness.state.pending_idle.is_empty()
    });
    assert_eq!(harness.slots.live(), MAX_CONNECTIONS - 1);
}

#[test]
fn a_parked_wait_idle_cannot_hold_its_slot_forever() {
    // The wedge a slot that moves into a waiter would otherwise open, and the
    // reason `MAX_IDLE_WAIT` exists. `timeout_ms` is a client-chosen `u64`,
    // and a parked waiter is the one thing here that cannot notice its peer
    // dying: nothing touches its socket until it has an answer to write, and
    // it has already left the event loop, so the write-stall deadline cannot
    // reach it either. Uncapped, this connection's slot is gone for the rest
    // of the session -- and 64 of them take the whole control channel with
    // them, for a bar and a notifier and every `scoot msg` that had nothing
    // to do with it.
    let mut harness = Harness::new();
    let mut client = harness.connect_limited(
        None,
        Limits {
            idle_wait: TINY_IDLE_WAIT,
            ..Limits::REAL
        },
    );
    client.send(
        request_line(&Request::WaitIdle {
            // Never quiet enough, and never out of time: the two values that
            // ask for forever.
            quiet_ms: u64::MAX,
            timeout_ms: u64::MAX,
        })
        .as_bytes(),
    );
    harness.pump_until("the wait-idle never handed over", |harness| {
        !harness.state.pending_idle.is_empty()
    });
    assert_eq!(
        harness.slots.live(),
        1,
        "the waiter should be holding the slot at this point"
    );

    // The peer dies, abruptly and without reading anything -- a killed agent,
    // not a polite disconnect.
    drop(client);
    harness.pump_until("a parked waiter held its slot for good", |harness| {
        harness.slots.live() == 0
    });
    assert!(
        harness.state.pending_idle.is_empty(),
        "the waiter outlived its slot"
    );
}

#[test]
fn a_wait_idle_within_the_cap_is_left_exactly_as_it_asked() {
    // The other half: capping must not quietly shorten a wait that was
    // already inside the bound -- nor lengthen it to the cap. This one asks
    // for a quiet period it will never get and a timeout well under the cap,
    // so the moment it is answered is its own `timeout_ms` and nothing else,
    // and the assertions below bracket it on both sides.
    const TIMEOUT: Duration = Duration::from_millis(100);

    let mut harness = Harness::new();
    let mut client = harness.connect_limited(
        None,
        Limits {
            idle_wait: TINY_IDLE_WAIT,
            ..Limits::REAL
        },
    );
    let started = Instant::now();
    client.send(
        request_line(&Request::WaitIdle {
            quiet_ms: u64::MAX,
            timeout_ms: TIMEOUT.as_millis() as u64,
        })
        .as_bytes(),
    );
    match client.expect_reply(&mut harness) {
        Response::Error { message } => assert!(
            message.contains("timed out"),
            "answered something else: {message}"
        ),
        other => panic!("a wait that can never be quiet was answered {other:?}"),
    }
    let waited = started.elapsed();
    assert!(
        waited >= TIMEOUT,
        "answered after {waited:?}, before the {TIMEOUT:?} it asked for"
    );
    assert!(
        waited < TINY_IDLE_WAIT,
        "answered after {waited:?}: waited the cap out rather than its own \
         {TIMEOUT:?}"
    );
    client.expect_closed(&mut harness);
}

// --- a peer that has stopped reading --------------------------------------

#[test]
fn a_half_closed_client_that_never_reads_is_evicted() {
    // The leak this closes. `shutdown(SHUT_WR)` raises `EPOLLIN`/`EPOLLRDHUP`,
    // not `EPOLLHUP`, and a connection with a queue is registered for
    // writability only -- so nothing wakes this connection again, ever, and it
    // used to hold its slot and two fds for the rest of the session.
    let mut harness = Harness::new();
    let mut client = harness.connect_stalling(Some(TINY_SNDBUF), TINY_STALL);
    let request = request_line(&Request::Version);
    let written = client.send_some(request.repeat(400).as_bytes());
    assert!(written > request.len(), "the test wrote nothing to answer");
    client
        .stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shuts down writing");

    // Deliberately never reading: a `collect()` here would be progress, and
    // progress is exactly what this connection is being given up on for.
    harness.pump_until("the half-closed connection was never evicted", |harness| {
        harness.slots.live() == 0
    });
}

#[test]
fn a_peer_that_keeps_reading_slowly_is_never_evicted() {
    // The other half of the rule: the deadline is on *progress*, not on how
    // long a client takes. A screenshot reply is megabytes, and a client
    // reading it a kilobyte at a time over a loaded machine is slow, not gone.
    let mut harness = Harness::new();
    let mut client = harness.connect_stalling(Some(TINY_SNDBUF), TINY_STALL);
    let request = request_line(&Request::Version);
    let written = client.send_some(request.repeat(400).as_bytes());
    let queued = written / request.len();
    assert!(queued > 1, "the test wrote nothing to queue");

    // Four windows' worth of sipping, which is twice the longest an eviction
    // can take.
    let until = Instant::now() + TINY_STALL * 4;
    while Instant::now() < until {
        harness.pump();
        client.sip(1024);
    }
    assert_eq!(
        harness.slots.live(),
        1,
        "a connection whose peer was reading all along was evicted"
    );

    // And nothing was lost along the way: every reply is still there, in
    // order, and the connection still answers.
    let replies = client.expect_replies(&mut harness, queued);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. })),
        "a queued reply came back damaged"
    );
}

#[test]
fn a_peer_reading_slower_than_its_replies_are_produced_is_never_evicted() {
    // The distinction the deadline turns on, and the reason it counts bytes
    // that *left* rather than bytes still queued: this client reads the whole
    // time, but slower than the compositor answers it, so its queue only ever
    // grows. Watching the queue's depth would read that as "no progress" and
    // drop a connection that is working exactly as intended -- an agent
    // pipelining a batch of requests and reading the answers as it goes.
    const PER_ROUND: usize = 8;
    const SIP: usize = 64;

    let mut harness = Harness::new();
    let mut client = harness.connect_stalling(Some(TINY_SNDBUF), TINY_STALL);
    let request = request_line(&Request::Version);
    let batch = request.repeat(PER_ROUND);

    let until = Instant::now() + TINY_STALL * 4;
    let mut asked = 0;
    let mut sipped = 0;
    while Instant::now() < until {
        asked += client.send_some(batch.as_bytes()) / request.len();
        harness.pump();
        sipped += client.sip(SIP);
    }
    assert_eq!(
        harness.slots.live(),
        1,
        "a connection whose peer was reading, only slowly, was evicted"
    );
    // The test is only worth anything if the queue really did outgrow what was
    // being read: otherwise this is the drain-as-you-go case again.
    assert!(
        asked * reply_line_len() > sipped * 2,
        "the client kept up after all ({asked} replies owed, {sipped} bytes read)"
    );
    assert!(sipped > 0, "the client never read a byte");
}

#[test]
fn a_queue_that_fills_again_gets_a_fresh_deadline() {
    // A `Timer` keeps the deadline it was last given. Re-arming one without
    // setting a new one would hand the event loop a deadline that had already
    // gone by while the queue was empty -- firing on the very next turn of the
    // loop, against a connection whose peer is a millisecond behind rather
    // than gone.
    let mut harness = Harness::new();
    let mut client = harness.connect_stalling(Some(TINY_SNDBUF), TINY_STALL);
    let request = request_line(&Request::Version);

    let written = client.send_some(request.repeat(400).as_bytes());
    let queued = written / request.len();
    assert!(queued > 1, "the test wrote nothing to queue");
    // Drained in full, so the connection goes back to waiting for requests and
    // its deadline is put away.
    let replies = client.expect_replies(&mut harness, queued);
    assert_eq!(replies.len(), queued);

    // Longer than the window, with nothing queued: an idle connection is not
    // on a deadline at all, so this must cost it nothing.
    harness.pump_for(TINY_STALL * 2);
    assert_eq!(
        harness.slots.live(),
        1,
        "an idle connection with an empty queue was evicted"
    );

    // Now fill it again. With a stale deadline this is evicted within a turn
    // or two of the loop; with a fresh one it has the whole window.
    let refilled = client.send_some(request.repeat(400).as_bytes());
    assert!(
        refilled > request.len(),
        "the test wrote nothing to requeue"
    );
    harness.pump_for(TINY_STALL / 2);
    assert_eq!(
        harness.slots.live(),
        1,
        "a connection was evicted on a deadline left over from an earlier queue"
    );
    // Still a working connection, too.
    let replies = client.expect_replies(&mut harness, refilled / request.len());
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, Response::Version { .. }))
    );
}

/// An agent that asked `outputs` before the screen changed size, and asks
/// again after, is told the new rectangle -- with no new plumbing on the IPC
/// side and no event to subscribe to.
///
/// The question `--tty`'s DRM hotplug support (`tty/hotplug.rs`) raised: a
/// mode change reaches `wl_output` and `wlr-output-management` through
/// `State::resize_output`, but `outputs` is answered from a different place
/// (`ipc.rs`'s `output_snapshots`), and a snapshot cached at startup would
/// serve an agent a stale size forever. It is not cached -- every field is
/// read from the live `Output` and the live `Space` -- and this is what says
/// so, over a real socket rather than by inspection.
///
/// `usable` is asserted alongside `rect` because they come from different
/// sources (the core's usable area, not the output's geometry) and only one
/// of them following the resize would be the more likely bug.
///
/// There is deliberately no `outputs`-changed event to wait on: `wait-idle`
/// already covers it, because `resize_output` ends in `apply()`, which ends
/// in `request_render()`. An agent that resizes something, waits for idle and
/// then asks is reading a settled compositor.
#[test]
fn an_agent_asking_outputs_again_after_a_resize_is_told_the_new_size() {
    const BEFORE: i32 = 64;
    const AFTER: i32 = 32;

    let mut harness = Harness::with_output(BEFORE, BEFORE);
    let mut client = harness.connect(None);
    let request = request_line(&Request::Outputs);

    let rects = |reply: Response| match reply {
        Response::Outputs { outputs } => {
            let output = outputs.first().expect("one output").clone();
            (output.rect, output.usable)
        }
        other => panic!("not an outputs reply: {other:?}"),
    };

    client.send(request.as_bytes());
    let (rect, usable) = rects(client.expect_reply(&mut harness));
    assert_eq!((rect.width, rect.height), (BEFORE, BEFORE));
    assert_eq!((usable.width, usable.height), (BEFORE, BEFORE));

    harness.state.resize_output(AFTER, AFTER);

    client.send(request.as_bytes());
    let (rect, usable) = rects(client.expect_reply(&mut harness));
    assert_eq!(
        (rect.width, rect.height),
        (AFTER, AFTER),
        "the reply still describes the pre-resize mode"
    );
    assert_eq!(
        (usable.width, usable.height),
        (AFTER, AFTER),
        "the usable area still describes the pre-resize mode"
    );
}
