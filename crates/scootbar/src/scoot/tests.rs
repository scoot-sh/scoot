use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use rustix::net::{bind, listen};

use super::*;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scootbar-scoot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("scoot.sock")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A scoot that answers one request with `reply` (`None`: closes without
/// answering; `Some("")` never answers and holds the connection), and
/// hands back the line it was sent.
fn scoot(dir: &Dir, reply: Option<&'static str>) -> JoinHandle<String> {
    let listener = UnixListener::bind(dir.socket()).expect("bind");
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .expect("the request");
        match reply {
            Some("") => thread::sleep(Duration::from_secs(2)),
            Some(reply) => {
                (&stream).write_all(reply.as_bytes()).expect("reply");
                (&stream).write_all(b"\n").expect("reply");
            }
            None => {}
        }
        line
    })
}

#[test]
fn quit_is_one_action_request_on_the_wire() {
    let dir = Dir::new("wire");
    let server = scoot(&dir, Some(r#"{"type":"ok","locked":false}"#));
    send_to(&dir.socket(), ScootAction::Quit).expect("accepted");
    assert_eq!(
        server.join().unwrap().trim(),
        r#"{"type":"action","action":"quit"}"#
    );
}

#[test]
fn a_scoot_that_quits_before_answering_is_success() {
    // `quit` ends the session: the connection closes with no reply.
    let dir = Dir::new("gone");
    let server = scoot(&dir, None);
    send_to(&dir.socket(), ScootAction::Quit).expect("closing is what quit does");
    server.join().unwrap();
}

#[test]
fn a_refusal_is_an_error_naming_why() {
    let dir = Dir::new("refused");
    let server = scoot(&dir, Some(r#"{"type":"error","message":"not now"}"#));
    let error = send_to(&dir.socket(), ScootAction::Quit).unwrap_err();
    assert!(
        error.contains("not now") && error.contains("quit"),
        "{error}"
    );
    server.join().unwrap();
}

#[test]
fn a_wedged_scoot_costs_a_bounded_wait() {
    let dir = Dir::new("wedged");
    let server = scoot(&dir, Some(""));
    let started = Instant::now();
    let error = send_to(&dir.socket(), ScootAction::Quit).unwrap_err();
    let waited = started.elapsed();
    assert!(error.contains("did not answer"), "{error}");
    // Bounded is what is proved, not fast: the real wait is about one
    // `TIMEOUT` (250 ms), and a loaded machine stretches it, so the bound is
    // a multiple a wedge that never answers could not meet.
    assert!(waited < Duration::from_secs(5), "waited {waited:?}");
    drop(server);
}

#[test]
fn no_scoot_is_an_error_and_nothing_else() {
    let dir = Dir::new("absent");
    let missing = dir.socket();
    let error = send_to(&missing, ScootAction::Quit).unwrap_err();
    assert!(error.contains("cannot reach scoot"), "{error}");
    // A path that is not a socket, and one too long for a socket address.
    std::fs::write(&missing, b"x").unwrap();
    assert!(send_to(&missing, ScootAction::Quit).is_err());
    let long = PathBuf::from(format!("/tmp/{}", "x".repeat(300)));
    assert!(send_to(&long, ScootAction::Quit).is_err());
}

#[test]
fn a_full_accept_queue_is_busy_not_a_hang() {
    // A listener that never accepts: fill its backlog with connections of
    // our own, and a connect to it must then fail at once (non-blocking)
    // rather than wait for room.
    let dir = Dir::new("busy");
    let address = SocketAddrUnix::new(dir.socket()).expect("address");
    // A backlog of one, which `UnixListener::bind` would not give.
    let listener = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .expect("socket");
    bind(&listener, &address).expect("bind");
    listen(&listener, 1).expect("listen");
    let mut queued = Vec::new();
    loop {
        let fd = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .expect("socket");
        if connect(&fd, &address).is_err() {
            break;
        }
        queued.push(fd);
        assert!(queued.len() < 100_000, "the backlog never filled");
    }
    let started = Instant::now();
    let error = send_to(&dir.socket(), ScootAction::Quit).unwrap_err();
    assert!(error.contains("cannot reach scoot"), "{error}");
    assert!(started.elapsed() < Duration::from_millis(200), "waited");
}

#[test]
fn the_request_is_what_scoots_protocol_encodes() {
    // The bar's line is written by hand; scoot-ipc (a dev-dependency only)
    // is the protocol's source of truth, so a change there shows here.
    let encoded =
        scoot_ipc::encode(&scoot_ipc::Request::Action(scoot_ipc::Action::Quit)).expect("encodes");
    assert_eq!(encoded.trim_end(), request(ScootAction::Quit).trim_end());
    assert!(request(ScootAction::Quit).ends_with('\n'));
}

#[test]
fn the_socket_is_found_as_scoot_finds_it() {
    let os = |s: &str| Some(OsString::from(s));
    assert_eq!(
        resolve(os("/tmp/a.sock"), os("/run/user/1000")),
        Some(PathBuf::from("/tmp/a.sock"))
    );
    assert_eq!(
        resolve(None, os("/run/user/1000")),
        Some(PathBuf::from("/run/user/1000/scoot.sock"))
    );
    assert_eq!(resolve(os(""), os("")), None);
    assert_eq!(resolve(None, None), None);
    assert_eq!(
        resolve(os(""), os("/run/user/1000")),
        Some(PathBuf::from("/run/user/1000/scoot.sock")),
        "an empty override is unset"
    );
}

#[test]
fn an_endless_reply_is_read_only_so_far() {
    let dir = Dir::new("endless");
    let listener = UnixListener::bind(dir.socket()).expect("bind");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .expect("request");
        // A megabyte with no newline: more than the bar will read.
        let junk = vec![b'x'; 1 << 20];
        let _ = (&stream).write_all(&junk);
    });
    let started = Instant::now();
    // Unparsable, so not an error: the request went out.
    assert_eq!(send_to(&dir.socket(), ScootAction::Quit), Ok(()));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}
