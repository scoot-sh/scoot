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

use flexwm_core::Config;
use flexwm_ipc::{Request, Response, decode, encode};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::*;
use crate::compositor::decorations::Appearance;
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

/// A compositor with a real event loop, and nothing on screen unless a test
/// asks for it.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
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
        );
        Self { event_loop, state }
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
    fn connect(&mut self, sndbuf: Option<usize>) -> TestClient {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        if let Some(size) = sndbuf {
            set_sndbuf(&server, size);
        }
        super::super::accept(&mut self.state, server).expect("the connection is accepted");
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
            make: "flexwm".into(),
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
        protocol: flexwm_ipc::PROTOCOL_VERSION,
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
                | (Request::Type { .. }, Response::Ok)
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
    // arriving in pieces, must still be answered rather than refused.
    let mut harness = Harness::new();
    let mut client = harness.connect(None);
    let request = request_line(&Request::Type {
        text: "x".repeat(500_000),
    });
    assert!(request.len() < MAX_REQUEST_BYTES);
    client.send_all(request.as_bytes(), &mut harness);
    // `Response::Ok` exactly, not "anything but a crash": a reassembly that
    // lost or duplicated a chunk decodes as a *different* failure
    // (`Response::Error` carrying a serde message), so accepting an error here
    // would pass against the very bug this is for.
    match client.expect_reply(&mut harness) {
        Response::Ok => {}
        other => panic!("the request did not survive being split up: {other:?}"),
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
    let mut harness = Harness::with_output(32, 32);
    let mut client = harness.connect(None);
    let batch = request_line(&Request::Screenshot { output: None }).repeat(2);
    client.send(batch.as_bytes());
    let replies = client.expect_replies(&mut harness, 2);
    assert!(
        matches!(replies[0], Response::Screenshot(_)),
        "the first capture failed: {:?}",
        replies[0]
    );
    match &replies[1] {
        Response::Error { message } => assert!(
            message.contains("limited to one per connection"),
            "wrong refusal: {message}"
        ),
        other => panic!("the second capture was not refused: {other:?}"),
    }
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
    let mut connection = source(server).expect("a connection").connection;
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
