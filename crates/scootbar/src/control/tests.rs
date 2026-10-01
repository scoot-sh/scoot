//! The control socket: paths, protocol, connections, claims and clients.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use super::claim::Claim;
use super::conn::{Conn, Handler, Status};
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
