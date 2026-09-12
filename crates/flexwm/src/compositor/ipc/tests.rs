//! Tests for the control socket: who may connect, how much one request may
//! cost, and when a request is answered rather than served.

use std::io::{BufReader, Cursor, Read};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;

use super::listener;
use super::*;

// --- wait-idle -----------------------------------------------------------

fn pending(now: Instant, quiet_ms: u64, timeout_ms: u64) -> PendingIdle {
    // A real stream is required by the struct but never touched by
    // idle_outcome; a socket pair is a cheap, sandboxed stand-in.
    let (a, _b) = UnixStream::pair().expect("socket pair");
    PendingIdle {
        stream: a,
        quiet: Duration::from_millis(quiet_ms),
        deadline: now + Duration::from_millis(timeout_ms),
        started: now,
    }
}

#[test]
fn already_quiet_client_still_waits_out_the_quiet_period() {
    let started = Instant::now();
    let wait = pending(started, 200, 5_000);
    // last_commit is long before `started` -- the client was already
    // idle when the request arrived. Immediately after registering,
    // this must NOT report idle: that was the race.
    let long_ago = started - Duration::from_secs(10);
    assert!(matches!(
        idle_outcome(started, long_ago, &wait),
        IdleOutcome::StillWaiting
    ));
    // Not idle either, partway through the quiet window...
    let mid = started + Duration::from_millis(100);
    assert!(matches!(
        idle_outcome(mid, long_ago, &wait),
        IdleOutcome::StillWaiting
    ));
    // ...but idle once quiet_ms has actually elapsed since `started`.
    let after = started + Duration::from_millis(201);
    assert!(matches!(
        idle_outcome(after, long_ago, &wait),
        IdleOutcome::Idle { .. }
    ));
}

#[test]
fn a_commit_during_the_wait_pushes_the_baseline_forward() {
    let started = Instant::now();
    let wait = pending(started, 200, 5_000);
    let commit_at = started + Duration::from_millis(150);
    // 200ms after start, but only 50ms after the commit: still waiting.
    let now = started + Duration::from_millis(200);
    assert!(matches!(
        idle_outcome(now, commit_at, &wait),
        IdleOutcome::StillWaiting
    ));
    let now = commit_at + Duration::from_millis(201);
    assert!(matches!(
        idle_outcome(now, commit_at, &wait),
        IdleOutcome::Idle { .. }
    ));
}

#[test]
fn times_out_when_never_idle_before_the_deadline() {
    let started = Instant::now();
    let wait = pending(started, 200, 500);
    // A commit keeps landing just inside every quiet window, so it's
    // never idle -- but the deadline still fires.
    let now = started + Duration::from_millis(501);
    let last_commit = now - Duration::from_millis(10);
    assert!(matches!(
        idle_outcome(now, last_commit, &wait),
        IdleOutcome::TimedOut
    ));
}

#[test]
fn waited_ms_is_measured_from_the_request_not_the_commit() {
    let started = Instant::now();
    let wait = pending(started, 50, 5_000);
    let now = started + Duration::from_millis(123);
    match idle_outcome(now, started, &wait) {
        IdleOutcome::Idle { waited_ms } => assert_eq!(waited_ms, 123),
        _ => panic!("expected idle"),
    }
}

// --- screenshot rate limiting --------------------------------------------

#[test]
fn a_connections_first_screenshot_is_never_throttled() {
    assert!(!screenshot_throttled(None, Instant::now()));
}

#[test]
fn a_second_screenshot_is_throttled_only_within_one_frame() {
    // `first` is when the previous capture finished, which is what the call
    // site stamps -- a capture can outlast a frame on its own, so stamping
    // when the request arrived instead would leave the window already
    // expired by the time the next request asked about it.
    let first = Instant::now();
    // Immediately after, and just inside the frame: refused.
    assert!(screenshot_throttled(Some(first), first));
    assert!(screenshot_throttled(
        Some(first),
        first + FRAME_INTERVAL - Duration::from_micros(1)
    ));
    // Exactly a frame later the screen may legitimately differ, so this is
    // the first request that must be served -- the boundary is `<`, not
    // `<=`, and a client polling at exactly the frame rate is not throttled.
    assert!(!screenshot_throttled(Some(first), first + FRAME_INTERVAL));
    assert!(!screenshot_throttled(
        Some(first),
        first + FRAME_INTERVAL + Duration::from_millis(1)
    ));
}

#[test]
fn one_connections_screenshot_does_not_throttle_another() {
    // The state is per-`Connection` (`last_screenshot`), so a client that
    // just took one has no bearing on a different connection's first
    // request, however hard the first one is hammering.
    let served = Instant::now();
    let now = served + Duration::from_micros(1);
    assert!(screenshot_throttled(Some(served), now));
    assert!(!screenshot_throttled(None, now));
}

#[test]
fn throttling_never_outlives_a_frame_however_long_the_gap() {
    // An agent that takes one screenshot and comes back an hour later is
    // served, not refused: the window is one frame, not a budget.
    let long_ago = Instant::now() - Duration::from_secs(3600);
    assert!(!screenshot_throttled(Some(long_ago), Instant::now()));
}

// --- bounded request lines -----------------------------------------------

/// Reads with a deliberately tiny buffer, so every line of any length
/// crosses several `fill_buf`/`consume` rounds.
fn tiny_reader(bytes: &[u8]) -> BufReader<Cursor<Vec<u8>>> {
    BufReader::with_capacity(4, Cursor::new(bytes.to_vec()))
}

#[test]
fn reads_one_line_at_a_time_leaving_the_rest() {
    let mut reader = tiny_reader(b"{\"type\":\"version\"}\n{\"type\":\"windows\"}\n");
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert_eq!(line, b"{\"type\":\"version\"}\n");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert_eq!(line, b"{\"type\":\"windows\"}\n");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Eof
    );
    assert!(line.is_empty());
}

#[test]
fn a_last_line_without_a_newline_is_still_a_request() {
    // What `read_line` does (`Ok(n > 0)`), kept: a client that writes one
    // request and shuts its write half down still gets an answer.
    let mut reader = tiny_reader(b"{\"type\":\"version\"}");
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert_eq!(line, b"{\"type\":\"version\"}");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Eof
    );
}

#[test]
fn a_client_that_says_nothing_at_all_is_just_eof() {
    // Connect and disconnect without a byte: not an error, nothing to reply
    // to, close the connection.
    let mut reader = tiny_reader(b"");
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Eof
    );
    assert!(line.is_empty());
}

#[test]
fn an_empty_line_is_read_rather_than_mistaken_for_eof() {
    let mut reader = tiny_reader(b"\n\n");
    let mut line = Vec::new();
    for _ in 0..2 {
        assert_eq!(
            read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
            LineRead::Line
        );
        assert_eq!(line, b"\n");
    }
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Eof
    );
}

#[test]
fn the_limit_is_the_whole_line_newline_included() {
    let mut line = Vec::new();
    // One under: 8 bytes plus the newline is exactly 9.
    let mut reader = tiny_reader(b"aaaaaaaa\n");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, 10),
        LineRead::Line
    );
    assert_eq!(line.len(), 9);
    // Exactly at the limit: accepted.
    let mut reader = tiny_reader(b"aaaaaaaaa\n");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, 10),
        LineRead::Line
    );
    assert_eq!(line.len(), 10);
    // One over: refused, and the buffer never grew past the limit.
    let mut reader = tiny_reader(b"aaaaaaaaaa\n");
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, 10),
        LineRead::TooLong
    );
    assert!(line.len() <= 10, "buffer grew to {}", line.len());
}

#[test]
fn the_limit_applies_to_each_line_not_to_the_connection() {
    // Three maximum-length requests in a row are three valid requests, not
    // one connection that has used up a budget.
    let mut reader = tiny_reader(b"aaaaa\naaaaa\naaaaa\n");
    let mut line = Vec::new();
    for _ in 0..3 {
        assert_eq!(read_line_bounded(&mut reader, &mut line, 6), LineRead::Line);
        assert_eq!(line, b"aaaaa\n");
    }
}

#[test]
fn a_shorter_line_after_a_longer_one_leaves_no_leftovers() {
    // The buffer is reused across requests; if it were not cleared, the tail
    // of the previous line would corrupt the next one's JSON.
    let mut reader = tiny_reader(b"aaaaaaaaaaaa\nbb\n");
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert_eq!(line, b"bb\n");
}

#[test]
fn an_endless_stream_with_no_newline_stops_at_the_limit() {
    // The finding itself: before the cap this read never returned and the
    // buffer grew until the machine gave out. `io::repeat` is an infinite
    // source with no newline in it, so this test hangs forever if the bound
    // is ever lost.
    let mut reader = BufReader::new(std::io::repeat(b'x'));
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, 4096),
        LineRead::TooLong
    );
    assert!(line.len() <= 4096, "buffer grew to {}", line.len());
}

#[test]
fn a_failing_read_closes_rather_than_looping() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("gone"))
        }
    }
    let mut reader = BufReader::new(Broken);
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Failed
    );
}

#[test]
fn the_default_limit_is_generous_enough_for_a_real_request() {
    // A `Request::Type` carrying a big paste has to keep working; the cap is
    // there for a client with no newline in sight, not for a large one.
    let text = "x".repeat(200_000);
    let encoded = encode(&Request::Type { text }).expect("encodes");
    assert!(encoded.len() < MAX_REQUEST_BYTES);
    let mut reader = tiny_reader(encoded.as_bytes());
    let mut line = Vec::new();
    assert_eq!(
        read_line_bounded(&mut reader, &mut line, MAX_REQUEST_BYTES),
        LineRead::Line
    );
    assert!(matches!(
        decode::<Request>(std::str::from_utf8(&line).expect("utf-8")),
        Ok(Request::Type { .. })
    ));
}

// --- the socket itself ---------------------------------------------------

#[test]
fn the_staging_directory_stays_in_the_sockets_own_directory() {
    // `rename` cannot cross filesystems, and the socket has to be published
    // where the caller asked -- so the directory it is built in has to share
    // a parent with it.
    let path = std::path::Path::new("/run/user/1000/flexwm.sock");
    let staging = listener::staging_path(path);
    assert_eq!(staging.parent(), path.parent());
    assert_ne!(staging, path);
}

#[test]
fn a_bound_socket_is_owner_only_and_leaves_no_temporary_behind() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("flexwm.sock");
    let socket = listener::bind(&path).expect("binds");

    let metadata = std::fs::metadata(&path).expect("the socket exists");
    assert!(metadata.file_type().is_socket());
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "the socket must not be reachable by anyone else, whatever the \
         directory's own mode or this process's umask"
    );

    // Nothing but the socket: the directory it was built in is gone.
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .expect("readable")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from("flexwm.sock")]);

    // And it is really listening, with the credentials to prove who
    // connected.
    let client = UnixStream::connect(&path).expect("connects");
    let (accepted, _) = accept_soon(&socket);
    assert_eq!(
        listener::peer_uid(&accepted).expect("peer credentials"),
        listener::own_uid()
    );
    drop(client);
}

#[test]
fn the_socket_is_owner_only_even_in_a_world_writable_directory() {
    // The configuration the whole item is about: `$FLEXWM_SOCKET` pointing
    // somewhere anyone can write, where `$XDG_RUNTIME_DIR`'s own `0700` is
    // not doing the work any more.
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777))
        .expect("opens the directory up");
    let path = dir.path().join("flexwm.sock");
    let _socket = listener::bind(&path).expect("binds");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("the socket exists")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(entries(dir.path()), 1, "no staging directory left behind");
}

#[test]
fn binding_replaces_a_stale_socket_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("flexwm.sock");
    std::fs::write(&path, b"left over from a crash").expect("writes");
    let _socket = listener::bind(&path).expect("binds over the stale file");
    assert!(
        std::fs::metadata(&path)
            .expect("the socket exists")
            .file_type()
            .is_socket()
    );
    assert!(UnixStream::connect(&path).is_ok());
}

#[test]
fn binding_replaces_a_symlink_rather_than_writing_through_it() {
    // The unlink-then-bind case: a symlink planted at the published path.
    // `rename` replaces the link itself, so the socket lands where it was
    // asked to and the link's target is never touched.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("flexwm.sock");
    let target = dir.path().join("somewhere-else");
    std::os::unix::fs::symlink(&target, &path).expect("plants a symlink");

    let _socket = listener::bind(&path).expect("binds");
    assert!(
        !std::fs::symlink_metadata(&path)
            .expect("exists")
            .file_type()
            .is_symlink(),
        "the published path must be the socket, not still a link"
    );
    assert!(
        std::fs::symlink_metadata(&target).is_err(),
        "nothing may be created at the link's target"
    );
    assert!(UnixStream::connect(&path).is_ok());
}

#[test]
fn a_bind_that_cannot_be_published_cleans_up_after_itself() {
    // A path whose parent does not exist fails at the staging directory, the
    // first step that touches the filesystem, leaving nothing behind.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("no-such-directory").join("flexwm.sock");
    assert!(listener::bind(&path).is_err());
    assert_eq!(entries(dir.path()), 0);

    // And a path that stages fine but cannot be published: `rename` onto an
    // existing *directory* fails (`EISDIR`/`ENOTDIR`) after the socket is
    // already bound, which is the window where a half-built socket could
    // have been left behind.
    let occupied = dir.path().join("flexwm.sock");
    std::fs::create_dir(&occupied).expect("a directory in the way");
    assert!(listener::bind(&occupied).is_err());
    assert_eq!(
        entries(dir.path()),
        1,
        "only the directory that was in the way may be left"
    );
    assert_eq!(entries(&occupied), 0, "and nothing may be left inside it");
}

fn entries(path: &std::path::Path) -> usize {
    std::fs::read_dir(path)
        .expect("readable")
        .filter_map(Result::ok)
        .count()
}

#[test]
fn only_this_users_connections_are_served() {
    let own = listener::own_uid();
    assert!(listener::peer_is_allowed(own, own));
    // Root is refused along with every other uid: it can reach this process
    // by other means anyway, and "the same user, or nobody" is the rule
    // that stays simple to reason about.
    assert!(!listener::peer_is_allowed(0, own.max(1)));
    assert!(!listener::peer_is_allowed(own.wrapping_add(1), own));
}

#[test]
fn peer_credentials_are_compared_against_the_right_uid() {
    // The real risk in the check is comparing the peer's uid against the
    // wrong source -- linux reports the peer's *effective* uid, so
    // `own_uid()` has to be `geteuid`, not `getuid` -- or reading the wrong
    // field out of the kernel's `struct ucred` by hand. A socket pair's peer
    // is this very process, so these must agree, and they only can if both
    // the sockopt read and the uid it is compared against are right.
    let (a, _b) = UnixStream::pair().expect("socket pair");
    let peer = listener::peer_uid(&a).expect("peer credentials");
    assert_eq!(peer, listener::own_uid());
    assert!(listener::peer_is_allowed(peer, listener::own_uid()));
    assert_ne!(peer, u32::MAX, "the sockopt must actually have been filled");
}

/// Accepts one connection from a non-blocking listener, allowing for the
/// connection not having been queued yet rather than failing the test on a
/// `WouldBlock` that says nothing about correctness.
fn accept_soon(
    listener: &std::os::unix::net::UnixListener,
) -> (UnixStream, std::os::unix::net::SocketAddr) {
    for _ in 0..100 {
        match listener.accept() {
            Ok(accepted) => return accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
    panic!("no connection arrived");
}
