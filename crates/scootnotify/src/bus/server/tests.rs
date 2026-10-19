//! The server's contract, against a fake peer: every method, every
//! reason, every refusal — without a daemon. The daemon-level rows
//! (ownership against a rival, `LimitsExceeded` over the wire, ids
//! across a restart, the flood) live in `tests/bus_tests.rs`.
//!
//! The driver calls `Server::handle` directly (framing the bus's
//! delivery is scoot-dbus's tested half) and reads the reply and signal
//! frames off the peer socket: routing, bodies and reasons are all
//! observed on the wire.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use scoot_dbus::conn::Conn;
use scoot_dbus::proto::{Kind, Message, Reader, Writer, frame_at};

use super::{INTERFACE, OBJECT_PATH, Server};
use crate::bus::notify::NOTIFY_SIGNATURE;
use crate::config::Config;
use crate::model::store::IdFile;

/// A connected `Conn` whose peer is the test: the handshake mirrors
/// `testdaemon::serve_setup`, but the peer stays open for reading
/// replies and signals.
fn connected() -> (Conn, UnixStream) {
    let (client, peer) = UnixStream::pair().unwrap();
    let thread = std::thread::spawn(move || {
        let mut peer = peer;
        peer.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        read_until(&mut peer, b"AUTH EXTERNAL\r\n");
        peer.write_all(b"DATA\r\n").unwrap();
        read_until(&mut peer, b"DATA\r\n");
        peer.write_all(b"OK 0123456789abcdef\r\n").unwrap();
        read_until(&mut peer, b"BEGIN\r\n");
        // Hello call.
        let mut staged = Vec::new();
        let mut chunk = [0u8; 4096];
        let len = loop {
            if let Ok(Some(len)) = frame_at(&staged) {
                break len;
            }
            let n = peer.read(&mut chunk).unwrap();
            staged.extend_from_slice(&chunk[..n]);
        };
        let call = Message::parse(&staged[..len]).unwrap();
        assert_eq!(call.member, Some("Hello"));
        let mut body = Writer::new();
        body.str(":9.9");
        let body = body.take_body().unwrap();
        let mut reply = Writer::new();
        reply.begin_return_to(1, ":9.9", call.serial, "s");
        reply.raw(&body);
        peer.write_all(&reply.finish().unwrap()).unwrap();
        peer
    });
    let conn = scoot_dbus::conn::setup(client).unwrap();
    let peer = thread.join().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    (conn, peer)
}

fn read_until(stream: &mut UnixStream, want: &[u8]) {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(want) {
        assert_eq!(stream.read(&mut byte).unwrap(), 1, "the client talks");
        got.push(byte[0]);
    }
}

/// Reads one whole frame from the peer: what the server flushed.
/// Leftover bytes stay staged: a reply and its signal often arrive in
/// one read, and dropping the second would hang the test exactly the
/// way a real client that did so would hang itself.
fn read_frame(peer: &mut UnixStream, staged: &mut Vec<u8>) -> Vec<u8> {
    let mut chunk = [0u8; 4096];
    loop {
        if let Ok(Some(len)) = frame_at(staged) {
            return staged.drain(..len).collect();
        }
        let n = peer.read(&mut chunk).expect("the server answers");
        assert!(n > 0, "the server answers before closing");
        staged.extend_from_slice(&chunk[..n]);
    }
}

struct Harness {
    server: Server,
    conn: Conn,
    peer: UnixStream,
    staged: Vec<u8>,
}

impl Harness {
    fn new() -> Self {
        let (conn, peer) = connected();
        Self {
            server: Server::new(Config::default(), IdFile::open(None)),
            conn,
            peer,
            staged: Vec::new(),
        }
    }

    /// Routes one call as `sender`, flushing the answer: the reply
    /// frame, still unparsed.
    fn call(
        &mut self,
        sender: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Vec<u8> {
        self.server.handle(
            &mut self.conn,
            sender,
            OBJECT_PATH,
            interface,
            member,
            77,
            signature,
            body,
            0,
        );
        let (_, _) = self.conn.pump();
        let peer = &mut self.peer;
        let staged = &mut self.staged;
        read_frame(peer, staged)
    }

    /// The reply's body strings: what most assertions read.
    fn strings(frame: &[u8]) -> Vec<String> {
        let message = Message::parse(frame).unwrap();
        assert_eq!(message.kind, Kind::MethodReturn);
        let mut reader = Reader::le(message.body.rest());
        let mut out = Vec::new();
        while !reader.exhausted() {
            out.push(reader.str().unwrap().to_owned());
        }
        out
    }
}

/// One `Notify` body, everything inline in one `Writer` (see
/// `testutils`: separately marshalled values would sit misaligned).
fn notify_body(
    app: &str,
    replaces: u32,
    summary: &str,
    body_text: &str,
    actions: &[&str],
    expire: i32,
) -> Vec<u8> {
    let mut out = Writer::new();
    out.str(app);
    out.u32(replaces);
    out.str("");
    out.str(summary);
    out.str(body_text);
    let cookie = out.open_array(4).unwrap();
    for action in actions {
        out.str(action);
    }
    out.close_array(cookie);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    out.i32(expire);
    out.take_body().unwrap()
}

/// The reply's single u32: `Notify` answers, `NotificationClosed`
/// carries.
fn reply_u32(frame: &[u8]) -> u32 {
    let message = Message::parse(frame).unwrap();
    assert_eq!(message.kind, Kind::MethodReturn, "{message:?}");
    assert_eq!(message.signature, "u");
    Reader::le(message.body.rest()).u32().unwrap()
}

/// An error reply: its name, and whether it carries a body.
fn reply_error(frame: &[u8]) -> (String, bool) {
    let message = Message::parse(frame).unwrap();
    assert_eq!(message.kind, Kind::Error, "{message:?}");
    let name = message.error.unwrap_or("").to_owned();
    (name, !message.body.rest().is_empty())
}

#[test]
fn capabilities_list_the_four_without_persistence() {
    let mut harness = Harness::new();
    let frame = harness.call(":1.1", INTERFACE, "GetCapabilities", "", &[]);
    let message = Message::parse(&frame).unwrap();
    assert_eq!(message.signature, "as");
    let mut reader = Reader::le(message.body.rest());
    let mut capabilities = Vec::new();
    let mut elements = reader.elements(4).unwrap();
    while !elements.exhausted() {
        capabilities.push(elements.str().unwrap().to_owned());
    }
    assert_eq!(
        capabilities,
        ["body", "body-markup", "actions", "icon-static"]
    );
}

#[test]
fn server_information_names_the_server() {
    let mut harness = Harness::new();
    let frame = harness.call(":1.1", INTERFACE, "GetServerInformation", "", &[]);
    let message = Message::parse(&frame).unwrap();
    assert_eq!(message.signature, "ssss");
    assert_eq!(
        Harness::strings(&frame),
        ["scootnotify", "scoot", env!("CARGO_PKG_VERSION"), "1.3"]
    );
}

#[test]
fn notify_answers_an_id_and_close_takes_it_with_reason_3() {
    let mut harness = Harness::new();
    let body = notify_body("Mail", 0, "Hi", "there", &[], -1);
    let id = reply_u32(&harness.call(":1.1", INTERFACE, "Notify", NOTIFY_SIGNATURE, &body));
    assert_eq!(id, 1);
    // Closing from the same sender: an empty return, then the signal
    // with reason 3 (id invalidated before the signal).
    let mut close = Writer::new();
    close.u32(id);
    let close = close.take_body().unwrap();
    let frame = harness.call(":1.1", INTERFACE, "CloseNotification", "u", &close);
    let message = Message::parse(&frame).unwrap();
    assert_eq!((message.kind, message.signature), (Kind::MethodReturn, ""));
    let (_, _) = harness.conn.pump();
    let signal = read_frame(&mut harness.peer, &mut harness.staged);
    let message = Message::parse(&signal).unwrap();
    assert_eq!(message.kind, Kind::Signal);
    assert_eq!(message.member, Some("NotificationClosed"));
    let mut reader = Reader::le(message.body.rest());
    assert_eq!((reader.u32().unwrap(), reader.u32().unwrap()), (id, 3));
}

#[test]
fn close_of_an_unknown_id_is_an_empty_error() {
    let mut harness = Harness::new();
    let mut close = Writer::new();
    close.u32(4242);
    let close = close.take_body().unwrap();
    let frame = harness.call(":1.1", INTERFACE, "CloseNotification", "u", &close);
    let (name, has_body) = reply_error(&frame);
    assert!(!name.is_empty(), "an error names itself");
    assert!(!has_body, "the spec's empty D-Bus Error message");
}

#[test]
fn misshapen_calls_are_invalid_args_with_a_body() {
    let mut harness = Harness::new();
    // `Notify` with a stranger's signature.
    let frame = harness.call(":1.1", INTERFACE, "Notify", "s", &[]);
    let (name, has_body) = reply_error(&frame);
    assert_eq!(name, "org.freedesktop.DBus.Error.InvalidArgs");
    assert!(has_body, "limits and bad arguments tell the user why");
    // `Notify` with the right signature but a short body.
    let frame = harness.call(":1.1", INTERFACE, "Notify", NOTIFY_SIGNATURE, b"short");
    assert_eq!(
        reply_error(&frame).0,
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
    // `CloseNotification` with a stranger's signature.
    let frame = harness.call(":1.1", INTERFACE, "CloseNotification", "s", &[]);
    assert_eq!(
        reply_error(&frame).0,
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
    // A stranger's method, and a stranger's object.
    let frame = harness.call(":1.1", INTERFACE, "DeleteAll", "", &[]);
    assert_eq!(
        reply_error(&frame).0,
        "org.freedesktop.DBus.Error.UnknownMethod"
    );
    harness.server.handle(
        &mut harness.conn,
        ":1.1",
        "/nowhere",
        INTERFACE,
        "Notify",
        78,
        NOTIFY_SIGNATURE,
        &[],
        0,
    );
    let (_, _) = harness.conn.pump();
    assert_eq!(
        reply_error(&read_frame(&mut harness.peer, &mut harness.staged)).0,
        "org.freedesktop.DBus.Error.UnknownObject"
    );
}

#[test]
fn peer_and_introspection_answer() {
    let mut harness = Harness::new();
    let frame = harness.call(":1.1", "org.freedesktop.DBus.Peer", "Ping", "", &[]);
    let message = Message::parse(&frame).unwrap();
    assert_eq!((message.kind, message.signature), (Kind::MethodReturn, ""));
    let frame = harness.call(":1.1", "org.freedesktop.DBus.Peer", "GetMachineId", "", &[]);
    let message = Message::parse(&frame).unwrap();
    assert_eq!(message.signature, "s");
    assert!(
        !Harness::strings(&frame)[0].is_empty(),
        "this host has an id"
    );
    let frame = harness.call(
        ":1.1",
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        "",
        &[],
    );
    let text = &Harness::strings(&frame)[0];
    for word in [
        "Notify",
        "CloseNotification",
        "GetCapabilities",
        "GetServerInformation",
        "NotificationClosed",
        "ActionInvoked",
        "ActivationToken",
    ] {
        assert!(text.contains(word), "{word} introspected");
    }
}

#[test]
fn expiry_emits_reason_1() {
    let mut harness = Harness::new();
    // A 50 ms card, presented headless at once.
    let body = notify_body("Mail", 0, "Hi", "there", &[], 50);
    let id = reply_u32(&harness.call(":1.1", INTERFACE, "Notify", NOTIFY_SIGNATURE, &body));
    std::thread::sleep(std::time::Duration::from_millis(120));
    harness.server.fire_expiries(&mut harness.conn);
    let (_, _) = harness.conn.pump();
    let signal = read_frame(&mut harness.peer, &mut harness.staged);
    let message = Message::parse(&signal).unwrap();
    assert_eq!(message.member, Some("NotificationClosed"));
    let mut reader = Reader::le(message.body.rest());
    assert_eq!((reader.u32().unwrap(), reader.u32().unwrap()), (id, 1));
}

#[test]
fn a_dnd_held_notify_still_answers_its_id() {
    use crate::model::dnd::Dnd;
    let mut harness = Harness::new();
    harness.server.set_dnd(Dnd::On);
    let body = notify_body("Mail", 0, "Hi", "there", &[], -1);
    let id = reply_u32(&harness.call(":1.1", INTERFACE, "Notify", NOTIFY_SIGNATURE, &body));
    assert_eq!(id, 1);
    // Nothing live, but history keeps it, unseen: no signal went out
    // for it (the peer stays quiet past the reply).
    assert!(harness.server.store().live_ids().is_empty());
    assert_eq!(harness.server.store().history().len(), 1);
    assert_eq!(harness.server.store().unread(), 1);
}
