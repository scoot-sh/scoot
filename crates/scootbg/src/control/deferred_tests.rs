//! Replies that come later (`Answer::Later`): the connection waits without
//! being read or polled for anything, keeps its requests in order, and
//! survives its client going away.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use rustix::event::PollFlags;

use super::tests::{Echo, Scratch, claim, pair, read_available, service};
use super::{MAX_CONNECTIONS, Server, Status};

fn ok(out: &mut Vec<u8>) {
    out.extend_from_slice(b"{\"type\":\"ok\"}\n");
}

#[test]
fn a_deferred_reply_waits_then_arrives_in_order() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    // One write, three requests: one answered now, one later, one after.
    client.write_all(b"ab\nlater\nxyz\n").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(read_available(&mut client), "{\"echo\":2}\n");
    assert_eq!(echo.deferred, [conn.id()]);
    // Waiting: nothing asked of poll, and the request behind it untouched.
    assert_eq!(conn.interest(), PollFlags::empty());
    assert_eq!(echo.handled, 2);
    // Spurious readiness changes nothing.
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(echo.handled, 2);

    assert_eq!(conn.complete(ok, &mut echo), Status::Keep);
    assert_eq!(
        read_available(&mut client),
        "{\"type\":\"ok\"}\n{\"echo\":3}\n",
        "the reply first, then the request behind it"
    );
    assert_eq!(echo.handled, 3);
    assert_eq!(conn.interest(), PollFlags::IN);
    // A second completion is a no-op.
    assert_eq!(conn.complete(ok, &mut echo), Status::Keep);
    assert_eq!(read_available(&mut client), "");
}

#[test]
fn two_deferred_requests_in_a_row_wait_one_at_a_time() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    client.write_all(b"later 1\nlater 2\nz\n").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(echo.deferred.len(), 1);
    assert_eq!(conn.complete(ok, &mut echo), Status::Keep);
    assert_eq!(
        echo.deferred.len(),
        2,
        "the second is handled after the first reply"
    );
    assert_eq!(read_available(&mut client), "{\"type\":\"ok\"}\n");
    assert_eq!(conn.complete(ok, &mut echo), Status::Keep);
    assert_eq!(
        read_available(&mut client),
        "{\"type\":\"ok\"}\n{\"echo\":1}\n"
    );
}

/// A client that hangs up while waiting is reported by `POLLHUP` alone
/// (it is polled for nothing) and closed, not reported again forever.
#[test]
fn a_client_that_hangs_up_while_waiting_is_closed() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    client.write_all(b"later\n").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    drop(client);
    // What poll reports for it now, with no events asked for.
    let mut fds = [rustix::event::PollFd::new(conn.stream(), conn.interest())];
    let zero = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(rustix::event::poll(&mut fds, Some(&zero)).unwrap(), 1);
    let revents = fds[0].revents();
    assert!(revents.contains(PollFlags::HUP), "{revents:?}");
    let mut scratch = [0u8; 64];
    assert_eq!(
        conn.service(revents, &mut scratch, &mut echo),
        Status::Close
    );
}

/// `printf 'set …' | nc -U`: the request, then the end of the stream. The
/// reply still arrives, then the connection closes.
#[test]
fn a_half_closed_client_gets_its_deferred_reply_then_a_close() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    client.write_all(b"later").unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut status = Status::Keep;
    for _ in 0..4 {
        status = service(&mut conn, &mut echo);
    }
    assert_eq!(status, Status::Keep, "waiting, not closed");
    assert_eq!(echo.deferred.len(), 1);
    assert_eq!(conn.interest(), PollFlags::empty());
    assert_eq!(conn.complete(ok, &mut echo), Status::Close);
    // The server drops a connection it is told to close.
    drop(conn);
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let mut all = String::new();
    client.read_to_string(&mut all).unwrap();
    assert_eq!(all, "{\"type\":\"ok\"}\n");
}

/// The server finds the waiting connection by id however the list moved,
/// and a reply for a connection that is gone (evicted, closed) is dropped
/// without touching anyone else.
#[test]
fn the_server_delivers_by_id_and_drops_replies_for_gone_clients() {
    let scratch = Scratch::new("defer");
    let claim = claim(&scratch).unwrap();
    let mut server = Server::new(claim.listener()).unwrap();
    let socket = scratch.paths().socket;
    let mut clients: Vec<UnixStream> = (0..MAX_CONNECTIONS)
        .map(|_| UnixStream::connect(&socket).unwrap())
        .collect();
    server.accept(claim.listener()).unwrap();
    let mut echo = Echo::default();
    // The first and the third wait.
    for index in [0, 2] {
        clients[index].write_all(b"later\n").unwrap();
        assert!(server.service(index, PollFlags::IN, &mut echo));
    }
    let (first, third) = (echo.deferred[0], echo.deferred[1]);
    assert_ne!(first, third);
    // One more client evicts the oldest, the first waiter.
    clients.push(UnixStream::connect(&socket).unwrap());
    server.accept(claim.listener()).unwrap();
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    assert!(server.conns().iter().all(|c| c.id() != first));
    // Its reply goes nowhere; the third still gets its own.
    server.complete(first, ok, &mut echo);
    server.complete(third, ok, &mut echo);
    assert_eq!(read_available(&mut clients[2]), "{\"type\":\"ok\"}\n");
    for (index, client) in clients.iter_mut().enumerate().skip(3) {
        assert_eq!(read_available(client), "", "client {index}");
    }
    // The newest connection has a new id, never an old one.
    let newest = server.conns().last().unwrap().id();
    assert!(newest != first && newest != third);
}
