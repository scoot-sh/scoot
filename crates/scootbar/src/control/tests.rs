//! The control socket: paths, protocol, connections, claims and clients.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::claim::Claim;
use super::conn::{Conn, Handler, STALL_DEADLINE, Status};
use super::paths::{self, PathError, Paths};
use super::protocol::{self, PROTOCOL_VERSION, Request};
use super::{MAX_CONNECTIONS, Server};

/// A scratch directory, removed on drop (as in `config::tests`).
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!(
            "/tmp/opencode/scootbar-control-test-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn paths(&self) -> Paths {
        Paths {
            display: "test".to_owned(),
            socket: self.path.join("scootbar-test.sock"),
            lock: self.path.join("scootbar-test.lock"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn paths_follow_the_display() {
    let dir = std::ffi::OsStr::new("/run/user/1");
    let paths = paths::resolve(Some(std::ffi::OsStr::new("wayland-0")), Some(dir)).unwrap();
    assert_eq!(paths.display, "wayland-0");
    assert_eq!(
        paths.socket,
        PathBuf::from("/run/user/1/scootbar-wayland-0.sock")
    );
    assert_eq!(
        paths.lock,
        PathBuf::from("/run/user/1/scootbar-wayland-0.lock")
    );
    // An absolute display keeps its final component, sanitised.
    let paths = paths::resolve(Some(std::ffi::OsStr::new("/tmp/a b/c:d")), Some(dir)).unwrap();
    assert_eq!(paths.display, "c_d");
    // Unset and empty mean the default.
    let paths = paths::resolve(None, Some(dir)).unwrap();
    assert_eq!(paths.display, "wayland-0");
    let paths = paths::resolve(Some(std::ffi::OsStr::new("")), Some(dir)).unwrap();
    assert_eq!(paths.display, "wayland-0");
}

#[test]
fn paths_refuse_what_they_cannot_serve() {
    assert_eq!(
        paths::resolve(Some(std::ffi::OsStr::new("w")), None),
        Err(PathError::NoRuntimeDir)
    );
    assert_eq!(
        paths::resolve(
            Some(std::ffi::OsStr::new("w")),
            Some(std::ffi::OsStr::new("relative/dir"))
        ),
        Err(PathError::RelativeRuntimeDir(PathBuf::from("relative/dir")))
    );
    assert!(matches!(
        paths::resolve(
            Some(std::ffi::OsStr::new("/tmp/")),
            Some(std::ffi::OsStr::new("/run/user/1"))
        ),
        Err(PathError::NoDisplayName(_))
    ));
    let long = format!("/run/{}/x", "d".repeat(200));
    assert!(matches!(
        paths::resolve(
            Some(std::ffi::OsStr::new("wayland-0")),
            Some(std::ffi::OsStr::new(&long))
        ),
        Err(PathError::TooLong(_))
    ));
}

#[test]
fn every_request_round_trips_through_its_line() {
    let requests = [
        Request::Query { id: None },
        Request::Layout,
        Request::Reload,
        Request::Hide,
        Request::Show,
        Request::Toggle,
        Request::Kill,
        Request::Version,
        Request::Set {
            id: Cow::Borrowed("clock"),
            value: serde_json::json!({"on": true}),
        },
        // `null` is a value (a push module's way to clear), not an absent one.
        Request::Set {
            id: Cow::Borrowed("status"),
            value: serde_json::Value::Null,
        },
    ];
    for request in &requests {
        let line = request.line();
        assert!(line.ends_with('\n'), "{line}");
        let parsed = protocol::parse(line.trim_end_matches('\n').as_bytes());
        match (request, parsed) {
            (Request::Query { id: None }, Ok(Request::Query { id: None }))
            | (Request::Layout, Ok(Request::Layout))
            | (Request::Reload, Ok(Request::Reload))
            | (Request::Hide, Ok(Request::Hide))
            | (Request::Show, Ok(Request::Show))
            | (Request::Toggle, Ok(Request::Toggle))
            | (Request::Kill, Ok(Request::Kill))
            | (Request::Version, Ok(Request::Version)) => {}
            (
                Request::Set { id, value },
                Ok(Request::Set {
                    id: got,
                    value: got_value,
                }),
            ) => {
                assert_eq!(id, &got);
                assert_eq!(value, &got_value);
            }
            (request, parsed) => panic!("{request:?} parsed as {parsed:?}"),
        }
    }
}

#[test]
fn a_set_without_a_value_is_refused_and_with_null_is_not() {
    let missing = protocol::parse(br#"{"protocol":1,"type":"set","id":"x"}"#);
    assert!(
        matches!(missing, Err(protocol::RequestError::NoValue)),
        "{missing:?}"
    );
    let null = protocol::parse(br#"{"protocol":1,"type":"set","id":"x","value":null}"#);
    match null {
        Ok(Request::Set { value, .. }) => assert_eq!(value, serde_json::Value::Null),
        other => panic!("{other:?}"),
    }
}

#[test]
fn requests_refuse_loudly() {
    // Each case names what is wrong.
    let cases = [
        ("", "malformed"),
        ("[]", "one JSON object"),
        ("{\"type\":\"query\"}", "no `protocol`"),
        ("{\"protocol\":2,\"type\":\"query\"}", "protocol mismatch"),
        ("{\"protocol\":1}", "no `type`"),
        (
            "{\"protocol\":1,\"type\":\"halt\"}",
            "unknown request `halt`",
        ),
        (
            "{\"protocol\":1,\"type\":\"set\",\"value\":1}",
            "need an `id`",
        ),
        (
            "{\"protocol\":1,\"type\":\"set\",\"id\":\"clock\"}",
            "needs a `value`",
        ),
    ];
    for (line, says) in cases {
        let error = protocol::parse(line.as_bytes()).unwrap_err().to_string();
        assert!(error.contains(says), "{line}: {error}");
    }
    // Unknown fields are ignored, for later arguments' room.
    assert!(matches!(
        protocol::parse(b"{\"protocol\":1,\"type\":\"query\",\"output\":\"DP-1\"}"),
        Ok(Request::Query { id: None })
    ));
}

/// A handler answering every request with `ok`.
struct OkHandler;

impl Handler for OkHandler {
    fn handle(&mut self, _line: &[u8], out: &mut Vec<u8>) {
        protocol::write_reply(out, &protocol::Reply::Ok);
    }
}

/// Drives one connection to completion: every line the client sends is
/// answered, and the reply is read back.
fn roundtrip(client: &mut UnixStream, server: UnixStream, lines: &[&str]) -> Vec<String> {
    use rustix::event::PollFlags;
    let mut conn = Conn::new(server);
    let mut scratch = [0u8; 1024];
    let mut replies = Vec::new();
    client
        .set_nonblocking(true)
        .and_then(|()| conn.stream().set_nonblocking(true))
        .unwrap();
    for line in lines {
        client.write_all(line.as_bytes()).unwrap();
    }
    // Until every line is answered: readable, then drained.
    let mut handler = OkHandler;
    for _ in 0..100 {
        let status = conn.service(PollFlags::IN, &mut scratch, &mut handler);
        if status == Status::Close {
            break;
        }
        let mut buf = [0u8; 4096];
        match client.read(&mut buf) {
            Ok(0) | Err(_) => {}
            Ok(n) => replies.extend(
                String::from_utf8_lossy(&buf[..n])
                    .split_inclusive('\n')
                    .map(str::to_owned),
            ),
        }
        if replies.len() >= lines.len() {
            break;
        }
    }
    replies
}

#[test]
fn a_connection_answers_every_line() {
    let (mut client, server) = UnixStream::pair().unwrap();
    let replies = roundtrip(
        &mut client,
        server,
        &[
            "{\"protocol\":1,\"type\":\"query\"}\n",
            "{\"protocol\":1,\"type\":\"version\"}\n",
        ],
    );
    assert_eq!(replies, ["{\"type\":\"ok\"}\n", "{\"type\":\"ok\"}\n"]);
}

#[test]
fn an_overlong_line_is_refused_and_closed() {
    use rustix::event::PollFlags;
    let (mut client, server) = UnixStream::pair().unwrap();
    let mut conn = Conn::new(server);
    client
        .set_nonblocking(true)
        .and_then(|()| conn.stream().set_nonblocking(true))
        .unwrap();
    let long = "x".repeat(protocol::MAX_REQUEST_LINE + 2);
    client.write_all(long.as_bytes()).unwrap();
    client.write_all(b"\n").unwrap();
    let mut scratch = [0u8; 65536];
    let mut handler = OkHandler;
    let mut saw_error = false;
    for _ in 0..100 {
        if conn.service(PollFlags::IN, &mut scratch, &mut handler) == Status::Close {
            // Drain the reply before the close.
            let mut buf = [0u8; 4096];
            while let Ok(n) = client.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if String::from_utf8_lossy(&buf[..n]).contains("longer than") {
                    saw_error = true;
                }
            }
            break;
        }
    }
    assert!(saw_error, "the overlong line got no loud refusal");
}

#[test]
fn claiming_is_exclusive_and_releases() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("claim");
    let paths = scratch.paths();
    let mut first = Claim::acquire(&paths).unwrap();
    // The socket is there and owner-only.
    let mode = std::fs::metadata(&paths.socket)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    // A second claim refuses while the first holds the lock.
    let error = Claim::acquire(&paths).unwrap_err().to_string();
    assert!(error.contains("already running"), "{error}");
    first.release();
    assert!(!paths.socket.exists());
    assert!(
        paths.lock.exists(),
        "the lock file stays, as libwayland's does"
    );
    // After the release, a new claim succeeds (no stale file left).
    let mut second = Claim::acquire(&paths).unwrap();
    second.release();
}

#[test]
fn a_stale_socket_is_replaced_but_a_live_one_is_not() {
    let scratch = Scratch::new("stale");
    let paths = scratch.paths();
    // A dead daemon's socket: bound by nobody (lock free).
    let dead = UnixListener::bind(&paths.socket).unwrap();
    drop(dead);
    Claim::acquire(&paths).unwrap().release();
    // Something answering without the lock is refused, not stolen.
    let live = UnixListener::bind(&paths.socket).unwrap();
    let error = Claim::acquire(&paths).unwrap_err().to_string();
    assert!(error.contains("already answering"), "{error}");
    drop(live);
    // Something that is not a socket is refused too. (The refused claim
    // removes nothing, so the live listener's file is taken out by hand.)
    std::fs::remove_file(&paths.socket).unwrap();
    std::fs::write(&paths.socket, "not a socket").unwrap();
    let error = Claim::acquire(&paths).unwrap_err().to_string();
    assert!(error.contains("not a socket"), "{error}");
}

#[test]
fn the_server_admits_and_evicts() {
    let scratch = Scratch::new("server");
    let paths = scratch.paths();
    let mut claim = Claim::acquire(&paths).unwrap();
    let mut server = Server::new(claim.listener()).unwrap();
    assert!(server.has_spare());
    // More clients than fit: the oldest goes, the count stays bound.
    let mut clients = Vec::new();
    for _ in 0..MAX_CONNECTIONS + 4 {
        clients.push(UnixStream::connect(&paths.socket).unwrap());
    }
    server.accept(claim.listener()).unwrap();
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    drop(clients);
    claim.release();
}

#[test]
fn a_client_to_nothing_is_told_to_start_one() {
    let scratch = Scratch::new("client");
    let paths = scratch.paths();
    let error = super::client::send_to(&paths, &Request::Query { id: None })
        .unwrap_err()
        .to_string();
    assert!(error.contains("no scootbar daemon is running"), "{error}");
    assert!(error.contains("scootbar daemon"), "{error}");
}

#[test]
fn a_client_and_a_server_exchange_one_request() {
    let scratch = Scratch::new("exchange");
    let paths = scratch.paths();
    let listener = UnixListener::bind(&paths.socket).unwrap();
    let reply = std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap() > 0 {
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let request = protocol::parse(&line[..line.len() - 1]).unwrap();
            assert!(matches!(request, Request::Version));
            stream.write_all(b"{\"type\":\"ok\"}\n").unwrap();
        });
        super::client::send_to(&paths, &Request::Version).unwrap()
    });
    assert_eq!(reply, "{\"type\":\"ok\"}\n");
    assert_eq!(PROTOCOL_VERSION, 1);
}

// ---- the streaming client ----

/// A daemon that reads the request, writes `bytes` (all of them, or as many
/// as the client reads) and closes, and what `stream_to` made of it: the
/// lines `each` was given, and the result.
fn streamed(name: &str, bytes: Vec<u8>) -> (Vec<String>, Result<(), super::client::Error>) {
    let scratch = Scratch::new(name);
    let paths = scratch.paths();
    let listener = UnixListener::bind(&paths.socket).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut stream, _) = listener.accept().unwrap();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap() > 0 && byte[0] != b'\n' {}
            // The client may stop reading (a bound): a write error is fine.
            let _ = stream.write_all(&bytes);
        });
        let mut lines = Vec::new();
        let request = Request::Subscribe { events: Vec::new() };
        let result = super::client::stream_to(&paths, &request, |line| {
            lines.push(line.to_owned());
            true
        });
        // A client that never connected (a path error) leaves the daemon
        // thread in `accept`, which the scope would wait for for ever: one
        // connection to the still-open listener lets it end.
        let _ = UnixStream::connect(&paths.socket);
        (lines, result)
    })
}

#[test]
fn a_stream_that_closes_between_lines_ends_cleanly() {
    let (lines, result) = streamed(
        "stream-whole",
        b"{\"type\":\"subscribed\",\"events\":[\"module\"]}\n{\"type\":\"module\"}\n".to_vec(),
    );
    result.unwrap();
    assert_eq!(
        lines,
        [
            "{\"type\":\"subscribed\",\"events\":[\"module\"]}\n",
            "{\"type\":\"module\"}\n"
        ]
    );
}

#[test]
fn a_line_cut_by_the_connection_ending_is_discarded_and_an_error() {
    // The daemon drops a subscriber its write cannot finish: the client may
    // be left a prefix of an event. It is not printed as if whole, and the
    // outcome is not a success.
    let (lines, result) = streamed(
        "stream-cut",
        b"{\"type\":\"subscribed\",\"events\":[\"module\"]}\n{\"type\":\"module\",\"id\":\"clo"
            .to_vec(),
    );
    assert_eq!(
        lines,
        ["{\"type\":\"subscribed\",\"events\":[\"module\"]}\n"]
    );
    let error = result.unwrap_err();
    assert!(matches!(error, super::client::Error::Cut), "{error:?}");
    assert!(error.to_string().contains("discarded"), "{error}");
}

#[test]
fn a_first_line_cut_is_discarded_too() {
    let (lines, result) = streamed("stream-cut-first", b"{\"type\":\"subscr".to_vec());
    assert!(lines.is_empty(), "{lines:?}");
    assert!(
        matches!(result, Err(super::client::Error::Cut)),
        "{result:?}"
    );
}

#[test]
fn a_daemon_that_says_nothing_is_no_reply_not_a_cut() {
    let (lines, result) = streamed("stream-silent", Vec::new());
    assert!(lines.is_empty());
    assert!(
        matches!(result, Err(super::client::Error::NoReply)),
        "{result:?}"
    );
}

#[test]
fn a_line_with_no_end_past_the_bound_is_refused_not_passed_on() {
    let mut bytes = b"{\"type\":\"subscribed\",\"events\":[]}\n".to_vec();
    bytes.resize(bytes.len() + (16 << 20) + 1024, b'x');
    let (lines, result) = streamed("stream-long", bytes);
    assert_eq!(lines.len(), 1, "only the first line was whole");
    assert!(
        matches!(result, Err(super::client::Error::BadReply(_))),
        "{result:?}"
    );
}

#[test]
fn a_dropped_line_is_passed_on_and_then_ends_the_stream_as_an_error() {
    // What the daemon sends a subscriber it evicts: the whole line, then it
    // closes. `each` sees it (a script reading the stream does), and the
    // result is not a success.
    let (lines, result) = streamed(
        "stream-dropped",
        [
            "{\"type\":\"subscribed\",\"events\":[\"module\"]}\n",
            "{\"type\":\"dropped\"}\n",
        ]
        .concat()
        .into_bytes(),
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1].as_bytes(), super::protocol::DROPPED);
    let error = result.unwrap_err();
    assert!(matches!(error, super::client::Error::Dropped), "{error:?}");
    assert!(error.to_string().contains("subscribe again"), "{error}");
}

#[test]
fn the_dropped_line_is_json_of_that_type() {
    let value: serde_json::Value = serde_json::from_slice(super::protocol::DROPPED).unwrap();
    assert_eq!(value, serde_json::json!({"type": "dropped"}));
    assert_eq!(super::protocol::DROPPED.last(), Some(&b'\n'));
}

// ---- the write-stall deadline ----

/// A handler answering every request with one `bytes`-long reply.
struct BigHandler {
    bytes: usize,
}

impl Handler for BigHandler {
    fn handle(&mut self, _line: &[u8], out: &mut Vec<u8>) {
        out.resize(out.len() + self.bytes, b'x');
        out.push(b'\n');
    }
}

/// A server with one request peer stalled on a megabyte reply: small
/// buffers both ways, so a single answer fills them at once, and a client
/// that never reads again. Returns the server, the client end, and the
/// handler (kept alive by the caller for further turns).
fn stalled_server() -> (Server, UnixStream, BigHandler) {
    use rustix::event::PollFlags;
    let (mut client, server) = UnixStream::pair().unwrap();
    rustix::net::sockopt::set_socket_recv_buffer_size(&client, 4096).unwrap();
    rustix::net::sockopt::set_socket_send_buffer_size(&server, 4096).unwrap();
    client.set_nonblocking(true).unwrap();
    let mut server_obj = Server::with_spare(None);
    server_obj.admit(server);
    client
        .write_all(b"{\"protocol\":1,\"type\":\"version\"}\n")
        .unwrap();
    let mut handler = BigHandler { bytes: 1 << 20 };
    // One turn answers the megabyte and blocks on the full socket: kept,
    // not dropped, and nothing more is read from the peer.
    assert!(server_obj.service(0, PollFlags::IN, &mut handler));
    assert_eq!(server_obj.conns().len(), 1);
    (server_obj, client, handler)
}

/// Whether the peer closed: everything queued before the close reads
/// first; only the end of the stream counts. Returns what was queued.
/// A momentarily empty socket is polled briefly rather than failed at
/// once (one run in dozens saw `WouldBlock` here under full-suite load,
/// with the close already done and the data still arriving through the
/// loopback buffers): a peer that never closes still fails, after the
/// bound, since nothing ever becomes readable.
fn peer_drained(client: &mut UnixStream) -> (bool, usize) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut buf = [0u8; 4096];
    let mut drained = 0usize;
    let start = Instant::now();
    let bound = Duration::from_secs(5);
    loop {
        match client.read(&mut buf) {
            Ok(0) => return (true, drained),
            Ok(n) => drained += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= bound {
                    return (false, drained);
                }
                let mut fds = [PollFd::new(&*client, PollFlags::IN)];
                let wait = Timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                };
                let _ = poll(&mut fds, Some(&wait));
            }
            Err(_) => return (false, drained),
        }
    }
}

#[test]
fn a_request_peer_stalled_past_the_deadline_is_dropped() {
    let (mut server, mut client, _) = stalled_server();
    // Freshly stalled: the sweep keeps it, and the loop would sleep until
    // the deadline for it.
    assert_eq!(server.sweep(Instant::now()), 0);
    assert_eq!(server.conns().len(), 1);
    let mut now = None;
    assert!(
        server
            .stall_timeout(&mut now)
            .is_some_and(|wait| wait <= STALL_DEADLINE),
        "no wakeup armed for the stall"
    );
    // Past the deadline with nothing delivered: dropped, and the peer sees
    // the end of the stream.
    let late = Instant::now() + STALL_DEADLINE + Duration::from_secs(1);
    // Already due: the loop would not sleep at all, but wake and sweep.
    let mut due = Some(late);
    assert_eq!(server.stall_timeout(&mut due), Some(Duration::ZERO));
    assert_eq!(server.sweep(late), 1);
    assert!(server.conns().is_empty());
    let (closed, drained) = peer_drained(&mut client);
    assert!(closed, "the stalled peer was not closed");
    assert!(drained > 0, "nothing was delivered before the drop");
}

#[test]
fn a_request_peer_that_drains_is_not_swept() {
    use rustix::event::PollFlags;
    let (mut server, mut client, mut handler) = stalled_server();
    // The client reads everything, turn by turn: each turn delivers about
    // the buffers' worth, so hundreds of turns drain the megabyte for
    // sure, with no sleeps. Every turn keeps the connection, and the last
    // one clears the stall: a sweep far past the old deadline then keeps
    // it, with no wakeup armed.
    let mut buf = [0u8; 65536];
    let mut read_total = 0usize;
    for _ in 0..300 {
        assert!(server.service(0, PollFlags::OUT, &mut handler));
        for _ in 0..100 {
            match client.read(&mut buf) {
                Ok(n) => read_total += n,
                Err(_) => break,
            }
        }
    }
    assert!(server.service(0, PollFlags::OUT, &mut handler));
    assert!(
        read_total >= (1 << 20),
        "only {read_total} of the megabyte arrived"
    );
    let late = Instant::now() + STALL_DEADLINE + Duration::from_secs(1);
    assert_eq!(server.sweep(late), 0);
    assert_eq!(server.conns().len(), 1);
    let mut now = None;
    assert_eq!(server.stall_timeout(&mut now), None);
}

#[test]
fn a_slow_reader_gets_a_fresh_deadline_while_it_moves() {
    use rustix::event::PollFlags;
    let (mut server, mut client, mut handler) = stalled_server();
    // One byte delivered is progress: the deadline starts over, so a sweep
    // just short of a full deadline past the first stall keeps the peer.
    let mut byte = [0u8; 1];
    assert!(matches!(client.read(&mut byte), Ok(1)));
    assert!(server.service(0, PollFlags::OUT, &mut handler));
    let almost = Instant::now() + STALL_DEADLINE - Duration::from_secs(1);
    assert_eq!(server.sweep(almost), 0);
    assert_eq!(server.conns().len(), 1);
}
