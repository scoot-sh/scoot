//! The server half against real sockets: `UnixStream::pair` for one
//! connection's behaviour, and a listener in a scratch directory for
//! claiming, eviction and the socket file's lifecycle.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use rustix::event::PollFlags;

use super::conn::OUT_SOFT_LIMIT;
use super::{Claim, ClaimError, Conn, Handler, MAX_CONNECTIONS, Server, Status};
use crate::paths::{self, Paths};
use crate::protocol::MAX_REQUEST_LINE;

/// Answers every line with `{"echo":<len>}` and counts them.
#[derive(Default)]
struct Echo {
    handled: usize,
}

impl Handler for Echo {
    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>) {
        self.handled += 1;
        out.extend_from_slice(format!("{{\"echo\":{}}}\n", line.len()).as_bytes());
    }
}

/// A scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir =
            std::env::temp_dir().join(format!("sbg-{}-{nanos:08x}-{tag}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn paths(&self) -> Paths {
        paths::resolve(Some("wayland-9".as_ref()), Some(self.0.as_os_str())).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pair() -> (Conn, UnixStream) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    ours.set_nonblocking(true).unwrap();
    (Conn::new(ours), theirs)
}

/// Services until the connection stops making progress, like the poll loop
/// would with the socket always readable.
fn service(conn: &mut Conn, echo: &mut Echo) -> Status {
    let mut scratch = [0u8; 4096];
    conn.service(PollFlags::IN, &mut scratch, echo)
}

fn read_available(stream: &mut UnixStream) -> String {
    stream.set_nonblocking(true).unwrap();
    let mut all = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => all.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8(all).unwrap()
}

#[test]
fn a_request_gets_one_reply_and_the_connection_stays_open() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    client.write_all(b"abc\n").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(read_available(&mut client), "{\"echo\":3}\n");
    assert_eq!(conn.interest(), PollFlags::IN);

    client.write_all(b"de\nf").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(read_available(&mut client), "{\"echo\":2}\n");
    client.write_all(b"g\n").unwrap();
    assert_eq!(service(&mut conn, &mut echo), Status::Keep);
    assert_eq!(read_available(&mut client), "{\"echo\":2}\n");
    assert_eq!(echo.handled, 3);
}

#[test]
fn a_half_closed_client_is_answered_then_closed() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    // An unterminated last request, then the client's write side closes:
    // `printf '{...}' | nc -U` does exactly this.
    client.write_all(b"one\ntwo").unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut status = Status::Keep;
    for _ in 0..4 {
        status = service(&mut conn, &mut echo);
        if status == Status::Close {
            break;
        }
    }
    assert_eq!(status, Status::Close);
    assert_eq!(read_available(&mut client), "{\"echo\":3}\n{\"echo\":3}\n");
}

#[test]
fn an_overlong_line_gets_an_error_and_a_close() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    let writer = std::thread::spawn(move || {
        let big = vec![b'x'; MAX_REQUEST_LINE + 10];
        // The daemon closes partway; the write may fail with EPIPE.
        let _ = client.write_all(&big);
        client
    });
    let mut status = Status::Keep;
    for _ in 0..1000 {
        status = service(&mut conn, &mut echo);
        if status == Status::Close {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(status, Status::Close);
    assert_eq!(echo.handled, 0);
    drop(conn);
    let mut client = writer.join().unwrap();
    let reply = read_available(&mut client);
    let value: serde_json::Value = serde_json::from_str(reply.trim_end()).unwrap();
    assert_eq!(value["type"], "error");
    assert!(value["message"].as_str().unwrap().contains("longer than"));
}

#[test]
fn a_client_that_never_reads_stops_being_served() {
    let (mut conn, client) = pair();
    let mut echo = Echo::default();
    // Far more requests than the socket buffer can take replies for.
    let requests = "r\n".repeat(200_000);
    let writer = std::thread::spawn(move || {
        let mut client = client;
        let _ = client.write_all(requests.as_bytes());
        client
    });
    // Keep servicing: once the reply buffer is stuck, the connection asks
    // only for POLLOUT and handles nothing more.
    for _ in 0..20_000 {
        let revents = conn.interest();
        if revents == PollFlags::OUT {
            break;
        }
        let mut scratch = [0u8; 4096];
        conn.service(PollFlags::IN, &mut scratch, &mut echo);
    }
    assert_eq!(conn.interest(), PollFlags::OUT);
    let handled = echo.handled;
    // Servicing again does nothing new while the client does not read.
    let mut scratch = [0u8; 4096];
    assert_eq!(
        conn.service(PollFlags::IN, &mut scratch, &mut echo),
        Status::Keep
    );
    assert_eq!(echo.handled, handled);
    assert!(handled < 200_000);
    drop(conn);
    drop(writer.join().unwrap());
}

#[test]
fn a_burst_of_tiny_requests_is_all_answered() {
    let (mut conn, client) = pair();
    let mut echo = Echo::default();
    // 4096 empty lines in one read: 4096 replies would be 45 KB.
    let mut client = client;
    client.write_all(&[b'\n'; 4096]).unwrap();
    let mut scratch = [0u8; 4096];
    conn.service(PollFlags::IN, &mut scratch, &mut echo);
    // The socket took the replies as fast as they were made (it is far
    // from full), so all were handled; the bound is on what queues at once.
    assert_eq!(echo.handled, 4096);
    assert!(read_available(&mut client).len() > OUT_SOFT_LIMIT);
}

#[test]
fn a_client_that_vanishes_is_closed() {
    let (mut conn, client) = pair();
    let mut echo = Echo::default();
    drop(client);
    assert_eq!(service(&mut conn, &mut echo), Status::Close);
}

#[test]
fn a_reply_to_a_vanished_client_closes_without_a_panic() {
    let (mut conn, mut client) = pair();
    let mut echo = Echo::default();
    client.write_all(b"x\n").unwrap();
    drop(client);
    // The read gets the line and the EOF; the reply write gets EPIPE
    // (SIGPIPE is ignored by Rust's runtime).
    let first = service(&mut conn, &mut echo);
    let second = if first == Status::Keep {
        service(&mut conn, &mut echo)
    } else {
        first
    };
    assert_eq!(second, Status::Close);
}

fn claim(scratch: &Scratch) -> Result<Claim, ClaimError> {
    Claim::acquire(&scratch.paths())
}

fn is_socket(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

#[test]
fn a_claim_binds_an_owner_only_socket_and_release_removes_it() {
    let scratch = Scratch::new("claim");
    let paths = scratch.paths();
    let mut claim = claim(&scratch).unwrap();
    assert!(is_socket(&paths.socket));
    let mode = fs::metadata(&paths.socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(UnixStream::connect(&paths.socket).is_ok());
    claim.release();
    assert!(!paths.socket.exists());
    // The lock file stays: removing it would reopen the start-up race.
    assert!(paths.lock.exists());
    drop(claim);
    assert!(!paths.socket.exists());
}

#[test]
fn dropping_a_claim_removes_the_socket() {
    let scratch = Scratch::new("drop");
    let paths = scratch.paths();
    drop(claim(&scratch).unwrap());
    assert!(!paths.socket.exists());
}

#[test]
fn a_second_claim_is_refused_while_the_first_lives() {
    let scratch = Scratch::new("second");
    let first = claim(&scratch).unwrap();
    match claim(&scratch) {
        Err(ClaimError::AlreadyRunning { lock }) => assert_eq!(lock, scratch.paths().lock),
        other => panic!("expected AlreadyRunning, got {other:?}"),
    }
    // The refusal left the first daemon's socket alone.
    assert!(is_socket(&scratch.paths().socket));
    drop(first);
    // And once it is gone, a new claim succeeds.
    claim(&scratch).unwrap();
}

#[test]
fn a_stale_socket_is_replaced() {
    let scratch = Scratch::new("stale");
    let paths = scratch.paths();
    // A crashed daemon: the socket file is there, nothing listens.
    drop(UnixListener::bind(&paths.socket).unwrap());
    assert!(is_socket(&paths.socket));
    assert!(UnixStream::connect(&paths.socket).is_err());
    let claim = claim(&scratch).unwrap();
    assert!(UnixStream::connect(&paths.socket).is_ok());
    drop(claim);
}

#[test]
fn a_live_socket_without_the_lock_is_not_stolen() {
    let scratch = Scratch::new("live");
    let paths = scratch.paths();
    let _other = UnixListener::bind(&paths.socket).unwrap();
    assert!(matches!(claim(&scratch), Err(ClaimError::Answering { .. })));
    assert!(is_socket(&paths.socket));
}

#[test]
fn a_file_at_the_socket_path_is_not_removed() {
    let scratch = Scratch::new("file");
    let paths = scratch.paths();
    fs::write(&paths.socket, b"precious").unwrap();
    assert!(matches!(
        claim(&scratch),
        Err(ClaimError::NotASocket { .. })
    ));
    assert_eq!(fs::read(&paths.socket).unwrap(), b"precious");
}

#[test]
fn a_missing_runtime_dir_is_an_error() {
    let scratch = Scratch::new("missing");
    let paths =
        paths::resolve(Some("w".as_ref()), Some(scratch.0.join("gone").as_os_str())).unwrap();
    assert!(matches!(
        Claim::acquire(&paths),
        Err(ClaimError::Io { what: "open", .. })
    ));
}

#[test]
fn past_the_limit_the_oldest_connection_is_closed() {
    let scratch = Scratch::new("evict");
    let claim = claim(&scratch).unwrap();
    let mut server = Server::new();
    let socket = scratch.paths().socket;
    let mut clients: Vec<UnixStream> = (0..MAX_CONNECTIONS)
        .map(|_| UnixStream::connect(&socket).unwrap())
        .collect();
    server.accept(claim.listener());
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);

    clients.push(UnixStream::connect(&socket).unwrap());
    server.accept(claim.listener());
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    // The first client was closed: it reads EOF.
    let mut buf = [0u8; 1];
    assert_eq!(clients[0].read(&mut buf).unwrap(), 0);
    // The newest is served.
    let newest = clients.last_mut().unwrap();
    newest.write_all(b"hi\n").unwrap();
    let mut echo = Echo::default();
    let last = server.conns().len() - 1;
    assert!(server.service(last, PollFlags::IN, &mut echo));
    assert_eq!(read_available(newest), "{\"echo\":2}\n");
}

#[test]
fn a_flood_of_connections_is_accepted_in_bounded_batches() {
    let scratch = Scratch::new("flood");
    let claim = claim(&scratch).unwrap();
    let mut server = Server::new();
    let socket = scratch.paths().socket;
    let _clients: Vec<UnixStream> = (0..MAX_CONNECTIONS * 3)
        .map(|_| UnixStream::connect(&socket).unwrap())
        .collect();
    // One call takes one batch; the rest stay queued for the next wakeup.
    server.accept(claim.listener());
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    assert!(server.listening());
}

#[test]
fn after_release_a_new_claim_succeeds_and_the_old_one_leaves_it_alone() {
    let scratch = Scratch::new("handover");
    let paths = scratch.paths();
    let mut first = claim(&scratch).unwrap();
    // `kill`: the old daemon releases, then (still alive) closes its
    // clients. A new daemon started in between must win.
    first.release();
    let second = claim(&scratch).unwrap();
    // The old claim going away later must not remove the new socket.
    drop(first);
    assert!(is_socket(&paths.socket));
    assert!(UnixStream::connect(&paths.socket).is_ok());
    drop(second);
}
