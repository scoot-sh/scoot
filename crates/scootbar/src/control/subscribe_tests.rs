//! The agent requests on the wire, and a subscribed connection: what it
//! carries, what it refuses, and that a subscriber who stops reading is
//! dropped rather than buffered.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use rustix::event::PollFlags;

use super::conn::{Conn, Handler, Kinds, Status};
use super::protocol::{self, EventKind, Request, RequestError};
use super::{MAX_CONNECTIONS, MAX_SUBSCRIBERS, Server};

fn parse(line: &str) -> Result<Request<'_>, RequestError> {
    protocol::parse(line.as_bytes())
}

// ---- the requests ----

#[test]
fn a_query_may_name_one_module() {
    assert_eq!(
        parse(r#"{"protocol":1,"type":"query","id":"clock"}"#).unwrap(),
        Request::Query {
            id: Some(Cow::Borrowed("clock"))
        }
    );
    assert_eq!(
        parse(r#"{"protocol":1,"type":"query"}"#).unwrap(),
        Request::Query { id: None }
    );
    assert_eq!(
        parse(r#"{"protocol":1,"type":"layout"}"#).unwrap(),
        Request::Layout
    );
}

#[test]
fn an_invoke_carries_its_action_number_and_output() {
    let request = parse(
        r#"{"protocol":1,"type":"invoke","id":"volume","action":"raise","arg":5,"output":"DP-1"}"#,
    )
    .unwrap();
    assert_eq!(
        request,
        Request::Invoke {
            id: Cow::Borrowed("volume"),
            action: Cow::Borrowed("raise"),
            arg: Some(5),
            output: Some(Cow::Borrowed("DP-1")),
        }
    );
    // The number and the output are optional.
    assert_eq!(
        parse(r#"{"protocol":1,"type":"invoke","id":"b","action":"click"}"#).unwrap(),
        Request::Invoke {
            id: Cow::Borrowed("b"),
            action: Cow::Borrowed("click"),
            arg: None,
            output: None,
        }
    );
    // `null` is no number.
    assert!(matches!(
        parse(r#"{"protocol":1,"type":"invoke","id":"b","action":"click","arg":null}"#),
        Ok(Request::Invoke { arg: None, .. })
    ));
}

#[test]
fn an_invoke_is_refused_by_what_is_missing_or_wrong() {
    for (line, says) in [
        (r#"{"protocol":1,"type":"invoke","action":"click"}"#, "`id`"),
        (r#"{"protocol":1,"type":"invoke","id":"b"}"#, "`action`"),
        (
            r#"{"protocol":1,"type":"invoke","id":"b","action":"x","arg":1.5}"#,
            "whole number",
        ),
        (
            r#"{"protocol":1,"type":"invoke","id":"b","action":"x","arg":"3"}"#,
            "whole number",
        ),
        (
            r#"{"protocol":1,"type":"invoke","id":"b","action":"x","arg":99999999999}"#,
            "whole number",
        ),
        (
            r#"{"protocol":1,"type":"invoke","id":"b","action":"x","arg":-99999999999}"#,
            "whole number",
        ),
    ] {
        let error = parse(line).unwrap_err().to_string();
        assert!(error.contains(says), "{line}: {error}");
    }
    // A negative number that fits is a number (a module may take one).
    assert!(matches!(
        parse(r#"{"protocol":1,"type":"invoke","id":"b","action":"x","arg":-3}"#),
        Ok(Request::Invoke { arg: Some(-3), .. })
    ));
}

#[test]
fn a_subscribe_names_kinds_and_none_means_all() {
    let kinds = |line: &str| match parse(line).unwrap() {
        Request::Subscribe { events } => events,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        kinds(r#"{"protocol":1,"type":"subscribe"}"#),
        [EventKind::Module, EventKind::Output]
    );
    assert_eq!(
        kinds(r#"{"protocol":1,"type":"subscribe","events":[]}"#),
        [EventKind::Module, EventKind::Output]
    );
    assert_eq!(
        kinds(r#"{"protocol":1,"type":"subscribe","events":["output"]}"#),
        [EventKind::Output]
    );
    // Repeats are one.
    assert_eq!(
        kinds(r#"{"protocol":1,"type":"subscribe","events":["module","module"]}"#),
        [EventKind::Module]
    );
    let error = parse(r#"{"protocol":1,"type":"subscribe","events":["window"]}"#)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("window") && error.contains("module, output"),
        "{error}"
    );
}

#[test]
fn every_new_request_round_trips_through_its_line() {
    let requests = [
        Request::Query {
            id: Some(Cow::Borrowed("a b")),
        },
        Request::Layout,
        Request::Invoke {
            id: Cow::Borrowed("volume"),
            action: Cow::Borrowed("scroll-up"),
            arg: Some(-2),
            output: Some(Cow::Borrowed("DP \"1\"")),
        },
        Request::Subscribe {
            events: vec![EventKind::Output, EventKind::Module],
        },
    ];
    for request in requests {
        let line = request.line();
        assert!(line.ends_with('\n'));
        let parsed = parse(line.trim_end_matches('\n')).unwrap();
        assert_eq!(parsed, request, "{line}");
    }
}

// ---- a subscribed connection ----

/// Subscribes on the line `subscribe`, answers anything else `ok`.
#[derive(Default)]
struct Subscriber {
    to: Option<Kinds>,
    handled: usize,
}

impl Handler for Subscriber {
    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>) {
        self.handled += 1;
        if line == b"subscribe" {
            protocol::write_reply(
                out,
                &protocol::Reply::Subscribed {
                    events: vec!["module"],
                },
            );
            self.to = Some(Kinds::of(&[EventKind::Module]));
        } else {
            protocol::write_reply(out, &protocol::Reply::Ok);
        }
    }

    fn take_subscription(&mut self) -> Option<Kinds> {
        self.to.take()
    }
}

fn pair() -> (UnixStream, Conn) {
    let (client, server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server.set_nonblocking(true).unwrap();
    (client, Conn::new(server))
}

fn read_all(client: &mut UnixStream) -> String {
    let mut text = String::new();
    let mut buf = [0u8; 8192];
    loop {
        match client.read(&mut buf) {
            Ok(0) | Err(_) => return text,
            Ok(n) => text.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }
}

fn subscribe(client: &mut UnixStream, conn: &mut Conn, handler: &mut Subscriber) {
    client.write_all(b"subscribe\n").unwrap();
    let mut scratch = [0u8; 1024];
    assert_eq!(
        conn.service(PollFlags::IN, &mut scratch, handler),
        Status::Keep
    );
    assert_eq!(
        read_all(client),
        "{\"type\":\"subscribed\",\"events\":[\"module\"]}\n"
    );
    assert!(conn.subscription().is_some());
}

#[test]
fn subscribing_answers_once_and_dedicates_the_connection() {
    let (mut client, mut conn) = pair();
    let mut handler = Subscriber::default();
    assert!(conn.subscription().is_none());
    subscribe(&mut client, &mut conn, &mut handler);
    let kinds = conn.subscription().unwrap();
    assert!(kinds.wants(EventKind::Module));
    assert!(!kinds.wants(EventKind::Output));
    // It waits for the client to speak, never to write.
    assert_eq!(conn.interest(), PollFlags::IN);
}

#[test]
fn a_request_on_a_subscribed_connection_is_refused_and_the_connection_stays() {
    let (mut client, mut conn) = pair();
    let mut handler = Subscriber::default();
    subscribe(&mut client, &mut conn, &mut handler);
    let mut scratch = [0u8; 1024];
    client.write_all(b"query\n").unwrap();
    assert_eq!(
        conn.service(PollFlags::IN, &mut scratch, &mut handler),
        Status::Keep
    );
    let said = read_all(&mut client);
    assert!(
        said.contains("\"error\"") && said.contains("serves no further requests"),
        "{said}"
    );
    // The handler was not asked: the connection answers no request.
    assert_eq!(handler.handled, 1);
    // A burst of requests is one refusal, not a line each.
    client.write_all(&b"query\n".repeat(50)).unwrap();
    assert_eq!(
        conn.service(PollFlags::IN, &mut scratch, &mut handler),
        Status::Keep
    );
    assert_eq!(read_all(&mut client).matches("\"error\"").count(), 1);
    // And it still carries events.
    assert_eq!(conn.send_event(b"{\"type\":\"module\"}\n"), Status::Keep);
    assert_eq!(read_all(&mut client), "{\"type\":\"module\"}\n");
}

#[test]
fn a_subscribed_client_that_hangs_up_ends_the_connection() {
    let (client, mut conn) = pair();
    let mut handler = Subscriber::default();
    let mut client = client;
    subscribe(&mut client, &mut conn, &mut handler);
    drop(client);
    let mut scratch = [0u8; 1024];
    assert_eq!(
        conn.service(PollFlags::IN | PollFlags::HUP, &mut scratch, &mut handler),
        Status::Close
    );
}

#[test]
fn a_subscriber_that_stops_reading_is_dropped_not_buffered() {
    let (mut client, mut conn) = pair();
    let mut handler = Subscriber::default();
    subscribe(&mut client, &mut conn, &mut handler);
    // The client never reads again. Events go out one write each, so the
    // socket fills, and the first one it cannot take whole closes the
    // connection: nothing was queued for it.
    let event = [b'x'; 4096];
    let mut sent = 0usize;
    let status = loop {
        match conn.send_event(&event) {
            Status::Keep => sent += event.len(),
            Status::Close => break Status::Close,
        }
        assert!(sent < 64 << 20, "the socket never filled");
    };
    assert_eq!(status, Status::Close);
    // Bounded by the socket's own buffer, which is the kernel's.
    assert!(sent < 8 << 20, "{sent} bytes were taken");
}

#[test]
fn an_event_larger_than_the_socket_takes_at_once_closes_the_connection() {
    // Never half an event: a write that is partial ends the subscriber.
    let (mut client, mut conn) = pair();
    let mut handler = Subscriber::default();
    subscribe(&mut client, &mut conn, &mut handler);
    let huge = vec![b'y'; 32 << 20];
    assert_eq!(conn.send_event(&huge), Status::Close);
}

// ---- the server ----

fn server_with(clients: usize) -> (Server, Vec<UnixStream>) {
    let mut server = Server::with_spare(None);
    let mut ends = Vec::new();
    for _ in 0..clients {
        let (client, accepted) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server.admit(accepted);
        ends.push(client);
    }
    (server, ends)
}

#[test]
fn a_broadcast_reaches_only_the_subscribers_of_that_kind() {
    let (mut server, mut clients) = server_with(3);
    let mut handler = Subscriber::default();
    // Client 0 subscribes to `module`; client 1 only talks; client 2 does
    // not subscribe either.
    clients[0].write_all(b"subscribe\n").unwrap();
    assert!(server.service(0, PollFlags::IN, &mut handler));
    assert_eq!(server.subscribers(), 1);
    read_all(&mut clients[0]);
    server.broadcast(EventKind::Module, b"{\"type\":\"module\"}\n");
    server.broadcast(EventKind::Output, b"{\"type\":\"output\"}\n");
    assert_eq!(read_all(&mut clients[0]), "{\"type\":\"module\"}\n");
    assert_eq!(read_all(&mut clients[1]), "");
    assert_eq!(read_all(&mut clients[2]), "");
}

#[test]
fn a_slow_subscriber_is_dropped_and_the_others_keep_theirs() {
    let (mut server, mut clients) = server_with(2);
    let mut handler = Subscriber::default();
    for (index, client) in clients.iter_mut().enumerate() {
        client.write_all(b"subscribe\n").unwrap();
        assert!(server.service(index, PollFlags::IN, &mut handler));
        read_all(client);
    }
    assert_eq!(server.subscribers(), 2);
    // Client 1 reads everything; client 0 reads nothing.
    let event = vec![b'z'; 8192];
    let mut line = event.clone();
    line.push(b'\n');
    let mut got = 0usize;
    for _ in 0..4096 {
        server.broadcast(EventKind::Module, &line);
        got += read_all(&mut clients[1]).len();
        if server.subscribers() == 1 {
            break;
        }
    }
    assert_eq!(server.subscribers(), 1, "the slow subscriber stayed");
    assert_eq!(server.conns().len(), 1);
    assert!(got > 0);
    // The one that kept up still gets what comes.
    server.broadcast(EventKind::Module, b"{\"type\":\"module\"}\n");
    assert!(read_all(&mut clients[1]).ends_with("{\"type\":\"module\"}\n"));
}

#[test]
fn with_no_subscriber_a_broadcast_does_nothing() {
    let (mut server, mut clients) = server_with(1);
    server.broadcast(EventKind::Module, b"x\n");
    assert_eq!(read_all(&mut clients[0]), "");
    assert_eq!(server.subscribers(), 0);
}

#[test]
fn the_subscriber_count_follows_the_connections() {
    let (mut server, mut clients) = server_with(1);
    let mut handler = Subscriber::default();
    clients[0].write_all(b"subscribe\n").unwrap();
    assert!(server.service(0, PollFlags::IN, &mut handler));
    assert_eq!(server.subscribers(), 1);
    // The client goes away: the next service closes it and the count drops.
    let client = clients.remove(0);
    drop(client);
    assert!(!server.service(0, PollFlags::IN | PollFlags::HUP, &mut handler));
    assert_eq!(server.subscribers(), 0);
}

/// Whether the peer closed: nothing more to read, and not just nothing yet.
fn closed(client: &mut UnixStream) -> bool {
    let mut byte = [0u8; 1];
    matches!(client.read(&mut byte), Ok(0))
}

/// `clients` connections of which the first `subscribed` subscribe.
fn server_subscribed(clients: usize, subscribed: usize) -> (Server, Vec<UnixStream>) {
    let (mut server, mut ends) = server_with(clients);
    let mut handler = Subscriber::default();
    for (index, client) in ends.iter_mut().enumerate().take(subscribed) {
        client.write_all(b"subscribe\n").unwrap();
        assert!(server.service(index, PollFlags::IN, &mut handler));
        read_all(client);
    }
    assert_eq!(server.subscribers(), subscribed);
    (server, ends)
}

#[test]
fn a_flood_of_connections_closes_the_idle_ones_and_not_the_subscribers() {
    // The two oldest connections are subscribers; the cap is full of idle
    // clients behind them. Each new client closes the oldest idle one.
    let (mut server, mut clients) = server_subscribed(MAX_CONNECTIONS, 2);
    for round in 0..MAX_CONNECTIONS * 3 {
        let (client, accepted) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server.admit(accepted);
        clients.push(client);
        assert_eq!(server.conns().len(), MAX_CONNECTIONS, "round {round}");
        assert_eq!(server.subscribers(), 2, "round {round}");
    }
    // They still carry events.
    server.broadcast(EventKind::Module, b"{\"type\":\"module\"}\n");
    for subscriber in &mut clients[..2] {
        assert_eq!(read_all(subscriber), "{\"type\":\"module\"}\n");
        assert!(!closed(subscriber));
    }
    // The idle ones that were there at the start are the ones that went.
    for idle in &mut clients[2..MAX_CONNECTIONS] {
        assert!(closed(idle));
    }
}

#[test]
fn the_oldest_idle_client_goes_first_not_the_oldest_connection() {
    let (mut server, mut clients) = server_subscribed(MAX_CONNECTIONS, 1);
    let (client, accepted) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server.admit(accepted);
    // Client 0 (a subscriber, the oldest) stays; client 1 (the oldest idle) went.
    assert!(!closed(&mut clients[0]));
    assert!(closed(&mut clients[1]));
    assert!(!closed(&mut clients[2]));
    assert_eq!(server.subscribers(), 1);
}

#[test]
fn with_only_subscribers_left_to_close_the_oldest_is_told_it_was_dropped() {
    // Out of descriptors with nothing but subscribers open (the cap cannot
    // get here: there are fewer subscribers than it admits).
    let (mut server, mut clients) = server_subscribed(MAX_SUBSCRIBERS, MAX_SUBSCRIBERS);
    assert!(server.free_an_fd());
    assert_eq!(server.subscribers(), MAX_SUBSCRIBERS - 1);
    // Whole, as the last line, then closed; the others untouched.
    assert_eq!(read_all(&mut clients[0]), "{\"type\":\"dropped\"}\n");
    assert!(closed(&mut clients[0]));
    for kept in &mut clients[1..] {
        assert_eq!(read_all(kept), "");
        assert!(!closed(kept));
    }
}

#[test]
fn a_dropped_notice_is_only_for_subscribers() {
    let (mut server, mut clients) = server_subscribed(2, 0);
    assert!(server.free_an_fd());
    assert_eq!(read_all(&mut clients[0]), "");
    assert!(closed(&mut clients[0]));
    assert!(server.free_an_fd());
    assert!(!server.free_an_fd(), "nothing left, and no spare");
}

#[test]
fn a_subscriber_whose_socket_is_full_is_dropped_without_waiting() {
    // The common drop: it stopped reading, its socket is full. The notice is
    // one nonblocking write that may not fit, and nothing waits for it.
    let (mut client, mut conn) = pair();
    let mut handler = Subscriber::default();
    subscribe(&mut client, &mut conn, &mut handler);
    let event = [b'x'; 4096];
    while conn.send_event(&event) == Status::Keep {}
    let before = std::time::Instant::now();
    conn.notify_dropped();
    assert!(before.elapsed() < std::time::Duration::from_secs(1));
}
