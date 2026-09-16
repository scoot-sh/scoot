//! Tests for the control socket itself: where it comes from, who may connect,
//! and the two pure policies `ipc.rs` applies to a request -- when a
//! `wait-idle` is answered, and when a screenshot is refused.
//!
//! The pieces each have their own: `line/tests.rs` for request lines,
//! `outbound/tests.rs` for replies on their way out, and
//! `connection/tests.rs` for the event-loop machinery driving both.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;

use flexwm_ipc::decode;

use super::listener;
use super::*;

/// Shrinks how much unread data the kernel will hold for writes to `stream`.
///
/// Lives here rather than beside its busiest caller (`connection/tests.rs`)
/// because the wait-idle tests below need it too: a reply that does not fit is
/// the only interesting case on the write side, and a few-kilobyte send buffer
/// is how a test gets one without pushing megabytes through a debug build.
pub(super) fn set_sndbuf(stream: &UnixStream, bytes: usize) {
    let size = bytes as libc::c_int;
    // SAFETY: a live fd, a `c_int` of exactly the length claimed, nothing
    // borrowed past the call.
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::from_ref(&size).cast::<libc::c_void>(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(result, 0, "could not shrink the send buffer");
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// Reads everything `stream` has right now, returning how much that was.
/// `stream` must be non-blocking.
fn drain(stream: &UnixStream) -> usize {
    let mut source = stream;
    let mut chunk = [0u8; 16 * 1024];
    let mut total = 0;
    while let Ok(count) = source.read(&mut chunk) {
        if count == 0 {
            break;
        }
        total += count;
    }
    total
}

// --- wait-idle -----------------------------------------------------------

fn pending(now: Instant, quiet_ms: u64, timeout_ms: u64) -> PendingIdle {
    // A real stream is required by the struct but never touched by
    // idle_outcome; a socket pair is a cheap, sandboxed stand-in.
    let (a, _b) = UnixStream::pair().expect("socket pair");
    PendingIdle {
        stream: a,
        quiet: Duration::from_millis(quiet_ms),
        timeout: Duration::from_millis(timeout_ms),
        started: now,
        last_progress: now,
        answered: false,
        outbound: Outbound::default(),
        // No connection handed over to this one, so there is no slot to
        // inherit. What a real hand-off does with it is
        // `connection/tests.rs`'s `a_wait_idle_hand_off_keeps_its_slot`.
        _slot: None,
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

// --- wait-idle: getting the answer out -----------------------------------
//
// The happy path (an answer the socket takes straight away) is covered by
// `connection/tests.rs` end to end. What needs exercising here is the part that
// only happens when it *doesn't*: the answer is retried per frame tick, and
// given up on only after the client's own `timeout_ms` has passed with no write
// progress at all. Both halves of that are load-bearing and neither is
// observable from outside, so these drive `PendingIdle` directly.

/// A waiter whose socket is already stuffed full, so nothing it queues can go
/// out -- the state a client that pipelined requests and never read the replies
/// leaves behind, carried over from the connection (see `Connection::serve`'s
/// `WaitIdle` arm).
fn stalled(now: Instant, quiet_ms: u64, timeout_ms: u64) -> (PendingIdle, UnixStream) {
    let (server, client) = UnixStream::pair().expect("socket pair");
    set_sndbuf(&server, 1024);
    server.set_nonblocking(true).expect("non-blocking");
    let mut wait = PendingIdle {
        stream: server,
        quiet: ms(quiet_ms),
        timeout: ms(timeout_ms),
        started: now,
        last_progress: now,
        answered: false,
        outbound: Outbound::default(),
        _slot: None,
    };
    let PendingIdle {
        stream, outbound, ..
    } = &mut wait;
    outbound
        .send(&mut &*stream, "x".repeat(512 * 1024))
        .expect("queues an inherited tail");
    assert!(
        !wait.outbound.is_empty(),
        "the socket took the whole tail; this test needs one that cannot"
    );
    (wait, client)
}

#[test]
fn a_stalled_wait_idle_reply_is_given_up_on_only_after_the_clients_own_timeout() {
    let started = Instant::now();
    let (mut wait, _client) = stalled(started, 10, 500);
    // Before the quiet period: still waiting, and the stuck queue is no reason
    // to abandon it.
    assert!(wait.advance(started + ms(5), started));
    // Quiet has passed, so the answer is queued -- behind a tail that cannot go
    // out. This is where the no-progress clock starts.
    assert!(wait.advance(started + ms(20), started));
    assert!(
        wait.advance(started + ms(510), started),
        "given up on 490ms into a 500ms window: the clock has to start when the \
         answer was queued, not when the request arrived"
    );
    assert!(
        !wait.advance(started + ms(530), started),
        "510ms with not one byte written is a client that is not reading"
    );
}

#[test]
fn a_wait_idle_reply_that_is_still_draining_is_never_given_up_on() {
    // The distinction the whole mechanism turns on: slow is not dead. A client
    // taking a multi-megabyte reply a little at a time keeps its answer for as
    // long as it needs, however far past its own timeout that goes.
    let started = Instant::now();
    let (mut wait, client) = stalled(started, 10, 100);
    client.set_nonblocking(true).expect("non-blocking");
    assert!(wait.advance(started + ms(20), started));
    for tick in 1..=12u64 {
        // Everything available, not a fixed slice: on a unix socket the sender
        // only gets room back once whole queued buffers are consumed, so a
        // partial read can leave it still refusing writes -- which would make
        // this test about kernel accounting rather than about progress.
        let read = drain(&client);
        assert!(read > 0, "nothing to read on tick {tick}");
        // Every tick is more than a whole timeout window apart, so a mechanism
        // that measured total time rather than progress would give up at once.
        assert!(
            wait.advance(started + ms(20 + 150 * tick), started),
            "given up on at tick {tick} despite the client making progress"
        );
    }
}

#[test]
fn the_no_progress_window_runs_from_the_last_byte_written_not_from_the_answer() {
    // What separates "tracks progress" from "has a deadline": a client that
    // drains part of its reply and *then* stops has to get a fresh window from
    // the last byte that went out. Measuring from when the answer was queued
    // instead would drop a client that was reading steadily right up until the
    // window expired -- mid-reply, with no way for it to tell why.
    let started = Instant::now();
    let (mut wait, client) = stalled(started, 10, 100);
    client.set_nonblocking(true).expect("non-blocking");
    // The answer is queued at +20, so a window measured from *there* ends +120.
    assert!(wait.advance(started + ms(20), started));
    // A real read at +90 lets some of it out, which is what restarts the window.
    assert!(drain(&client) > 0);
    assert!(wait.advance(started + ms(90), started));
    assert!(
        wait.advance(started + ms(150), started),
        "given up on 60ms after the last byte went out, inside a 100ms window"
    );
    assert!(
        !wait.advance(started + ms(200), started),
        "110ms after the last byte went out is past the window"
    );
}

#[test]
fn a_timed_out_answer_gets_its_own_window_rather_than_none() {
    // `TimedOut` is the branch where the window used to be zero by
    // construction: `idle_outcome` only reports it once `timeout` has already
    // elapsed since the request arrived, so a window measured from *there* was
    // spent before the answer existed. Reachable only with an empty queue -- a
    // waiter carrying a tail that cannot drain is given up on by `push` first --
    // so the socket is stuffed from outside instead.
    let started = Instant::now();
    let (server, _client) = UnixStream::pair().expect("socket pair");
    set_sndbuf(&server, 1024);
    server.set_nonblocking(true).expect("non-blocking");
    let mut stuffing = &server;
    while stuffing.write(&[b'x'; 4096]).is_ok_and(|count| count > 0) {}
    let mut wait = PendingIdle {
        stream: server,
        quiet: ms(60_000),
        timeout: ms(100),
        started,
        last_progress: started,
        answered: false,
        outbound: Outbound::default(),
        _slot: None,
    };
    assert!(
        wait.advance(started + ms(150), started),
        "the timeout answer was dropped instead of queued"
    );
    assert!(!wait.outbound.is_empty(), "it went out after all");
    assert!(
        wait.advance(started + ms(200), started),
        "given up on 50ms into a 100ms window"
    );
    assert!(!wait.advance(started + ms(260), started));
}

#[test]
fn a_waiter_is_finished_with_once_its_answer_has_gone_out() {
    let started = Instant::now();
    let (server, client) = UnixStream::pair().expect("socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    let mut wait = PendingIdle {
        stream: server,
        quiet: ms(10),
        timeout: ms(5_000),
        started,
        last_progress: started,
        answered: false,
        outbound: Outbound::default(),
        _slot: None,
    };
    assert!(wait.advance(started + ms(5), started), "not quiet yet");
    assert!(
        !wait.advance(started + ms(20), started),
        "an answer that went out in one write leaves nothing to come back for"
    );
    let mut line = String::new();
    BufReader::new(client)
        .read_line(&mut line)
        .expect("the answer arrived");
    assert!(
        matches!(decode::<Response>(&line), Ok(Response::Idle { .. })),
        "the client got {line:?}"
    );
}

#[test]
fn a_waiter_whose_peer_is_gone_is_dropped_rather_than_retried_forever() {
    let started = Instant::now();
    let (mut wait, client) = stalled(started, 10, 60_000);
    drop(client);
    assert!(
        !wait.advance(started + ms(20), started),
        "a write to a closed peer has to end the waiter, whatever its timeout"
    );
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
    // Exactly a frame later is the first request that must be served: the
    // boundary is `<`, not `<=`, so a client polling at the compositor's own
    // frame rate is never throttled.
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

// --- the socket itself ---------------------------------------------------

#[test]
fn staging_names_are_marked_as_flexwms_and_do_not_repeat() {
    // A name another user could work out in advance is a name they can
    // occupy before flexwm starts, over and over.
    let names: std::collections::HashSet<_> = (0..64)
        .map(|_| listener::staging_name().expect("a staging name"))
        .collect();
    assert_eq!(names.len(), 64, "a repeat in 64 draws is not randomness");
    for name in &names {
        let name = name.to_str().expect("ascii");
        assert!(name.starts_with(".flexwm-"), "{name} is unattributable");
        assert_eq!(name.len(), ".flexwm-".len() + 12);
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-'),
            "{name} is not a plain name"
        );
    }
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

    // Nothing but the socket: the name it was staged at is gone.
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
    assert_eq!(entries(dir.path()), 1, "nothing staged left behind");
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

/// Everything in `dir`, as names, sorted.
fn listing(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .expect("readable")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn a_symlink_where_the_staging_directory_goes_is_refused_and_its_target_untouched() {
    // The attack both earlier versions of `bind` lost to: the name flexwm is
    // about to stage at, replaced with a symlink to something somebody else
    // wants deleted, chmodded, or bound over. Nothing may follow it, nothing
    // may be removed to get it out of the way, and nothing may be published.
    //
    // Driven through `publish` rather than `bind` deliberately: `bind`'s
    // staging name is unpredictable, which is most of why this is hard to
    // attack at all, and a test cannot plant a link at a name it cannot
    // guess. So this exercises the defence that does not depend on the name
    // being secret -- the `O_NOFOLLOW` open -- by handing it the name
    // directly.
    for target_is_a_directory in [false, true] {
        let dir = tempfile::tempdir().expect("a temp dir");
        let victim = dir.path().join("victim");
        if target_is_a_directory {
            std::fs::create_dir(&victim).expect("a victim directory");
            std::fs::write(victim.join("s"), b"someone else's socket").expect("writes");
        } else {
            std::fs::write(&victim, b"someone else's file").expect("writes");
        }
        let before = std::fs::symlink_metadata(&victim)
            .expect("exists")
            .permissions();
        let staging = dir.path().join("staging");
        std::os::unix::fs::symlink(&victim, &staging).expect("plants a symlink");
        let path = dir.path().join("flexwm.sock");

        let error = listener::publish(&staging, &path).expect_err("must refuse the name");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotADirectory,
            "a symlink must be refused as what it is, not followed"
        );

        // The victim is untouched: still there, still its own kind, still its
        // own mode, still its own contents.
        let after = std::fs::symlink_metadata(&victim).expect("the victim still exists");
        assert_eq!(after.is_dir(), target_is_a_directory);
        assert_eq!(
            after.permissions().mode(),
            before.mode(),
            "the victim's mode must not have been touched"
        );
        if target_is_a_directory {
            assert_eq!(
                std::fs::read(victim.join("s")).expect("readable"),
                b"someone else's socket",
                "nothing inside the victim may be unlinked or bound over"
            );
        } else {
            assert_eq!(
                std::fs::read(&victim).expect("readable"),
                b"someone else's file"
            );
        }
        // The planted link is left exactly as it was -- not cleared, not
        // replaced -- and nothing was published.
        assert!(
            std::fs::symlink_metadata(&staging)
                .expect("the link survives")
                .file_type()
                .is_symlink()
        );
        assert!(std::fs::symlink_metadata(&path).is_err());
    }
}

#[test]
fn a_plain_file_where_the_staging_directory_goes_is_refused_too() {
    // Same defence, without a link involved: whatever is at that name, if it
    // is not a directory this process just made, it is not used and not
    // removed.
    let dir = tempfile::tempdir().expect("a temp dir");
    let staging = dir.path().join("staging");
    std::fs::write(&staging, b"in the way").expect("writes");
    let error = listener::publish(&staging, &dir.path().join("flexwm.sock"))
        .expect_err("must refuse the name");
    assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
    assert_eq!(std::fs::read(&staging).expect("readable"), b"in the way");
}

#[test]
fn binding_leaves_everything_else_in_the_directory_alone() {
    // `bind` picks its own staging name and creates it with `mkdir`, which
    // neither follows a symlink nor replaces an existing name -- so whatever
    // else is in the directory, including a planted link, comes out the other
    // side untouched.
    let dir = tempfile::tempdir().expect("a temp dir");
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"not flexwm's").expect("writes");
    let planted = dir.path().join(".flexwm-aaaaaaaaaaaa");
    std::os::unix::fs::symlink(&victim, &planted).expect("plants a symlink");
    let path = dir.path().join("flexwm.sock");

    let _socket = listener::bind(&path).expect("binds");

    assert_eq!(
        listing(dir.path()),
        vec![
            std::ffi::OsString::from(".flexwm-aaaaaaaaaaaa"),
            std::ffi::OsString::from("flexwm.sock"),
            std::ffi::OsString::from("victim"),
        ],
        "only the socket may be added, and nothing removed"
    );
    assert_eq!(std::fs::read(&victim).expect("readable"), b"not flexwm's");
    assert!(
        std::fs::symlink_metadata(&planted)
            .expect("exists")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_path_at_the_full_sun_path_length_still_binds() {
    // The other regression this replaced: staging used to add 17 bytes to the
    // path it bound at, so a path that fits `sun_path` on its own stopped
    // fitting once staged. 107 bytes is the longest a unix socket path can
    // be, so it is the one length that proves staging costs nothing.
    let dir = tempfile::tempdir().expect("a temp dir");
    let room = 107 - (dir.path().as_os_str().len() + 1);
    assert!(room > 0, "the temp directory is too long for this test");
    let path = dir.path().join("s".repeat(room));
    assert_eq!(path.as_os_str().len(), 107);

    let _socket = listener::bind(&path).expect("binds at exactly 107 bytes");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("the socket exists")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(UnixStream::connect(&path).is_ok());
    assert_eq!(entries(dir.path()), 1, "and nothing staged is left behind");

    // One byte further is past what any unix socket can hold. It has to fail
    // at startup, the way it did before staging existed -- staging at a name
    // *shorter* than the published one would otherwise bind and rename
    // happily and leave a socket no client could ever connect to.
    let too_long = dir.path().join("s".repeat(room + 1));
    let error = listener::bind(&too_long).expect_err("107 bytes is the limit");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(entries(dir.path()), 1, "with nothing left behind either");
}

#[test]
fn a_bind_that_cannot_be_published_cleans_up_after_itself() {
    // A directory that does not exist fails at the bind, the first step that
    // touches the filesystem, leaving nothing behind.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("no-such-directory").join("flexwm.sock");
    assert!(listener::bind(&path).is_err());
    assert_eq!(entries(dir.path()), 0);

    // And a path that stages fine but cannot be published: `rename` onto an
    // existing *directory* fails after the socket is already bound, which is
    // the one window where a staged socket could be left behind.
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
