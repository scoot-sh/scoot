//! Reload tests: display sanitizing, the runtime-dir rule, and the bar
//! wire against a scripted socket peer.

#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use super::{CompositorReload, IpcCompositor, ReloadError};
#[cfg(target_os = "linux")]
use super::{bar_reload_outcome_with_timeout, send_bar_reload};
use super::{display_name, runtime_dir};

#[test]
fn display_names_sanitize_to_socket_names() {
    assert_eq!(display_name("wayland-1"), "wayland-1");
    assert_eq!(display_name("wayland-1/extra"), "extra");
    assert_eq!(display_name("way land;1"), "way_land_1");
    assert_eq!(display_name("a/b/../c"), "c");
}

#[test]
fn the_runtime_dir_prefers_xdg_and_falls_back_to_uid() {
    use std::ffi::OsStr;
    assert_eq!(
        runtime_dir(Some(OsStr::new("/run/user/7")), 99),
        Some(PathBuf::from("/run/user/7"))
    );
    assert_eq!(
        runtime_dir(Some(OsStr::new("")), 99),
        Some(PathBuf::from("/run/user/99"))
    );
    assert_eq!(runtime_dir(None, 99), Some(PathBuf::from("/run/user/99")));
}

/// Serves one bar reload reply on a scratch socket: reads the request line
/// and answers `reply`. The listener binds before returning, so the client
/// never connects before the server exists.
#[cfg(target_os = "linux")]
fn serve_once(socket: PathBuf, reply: &'static str) -> std::thread::JoinHandle<String> {
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("a scratch socket");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("one request line");
        std::io::Write::write_all(&mut &stream, reply.as_bytes()).expect("one reply line");
        line
    })
}

/// A scratch dir guard plus a socket path inside it. The `TempDir` guard
/// removes the tree on drop, so the suite leaves no `/tmp/slt-*` dirs
/// behind. Callers hold the guard (`let (_scratch, socket) = ...`) for the
/// test's duration: the peer binds inside it. Short names: a Unix socket
/// path must fit in `sun_path` (~104 bytes).
#[cfg(target_os = "linux")]
fn socket_path(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix(&format!("slt-{tag}-"))
        .tempdir()
        .expect("a scratch dir");
    let socket = scratch.path().join(format!("{tag}.sock"));
    (scratch, socket)
}

/// The socket peers below need `SO_RCVTIMEO`/`SO_SNDTIMEO` on Unix
/// sockets, which this macOS toolchain refuses with `EINVAL`: they run on
/// Linux (CI and the dev VM), where the bar runs too.
#[cfg(target_os = "linux")]
#[test]
fn a_look_reload_sends_the_scoped_verb_and_accepts_ok() {
    let (_scratch, socket) = socket_path("ok");
    let peer = serve_once(socket.clone(), "{\"type\":\"ok\",\"scope\":\"look\"}\n");
    send_bar_reload(&socket).expect("a scoped ok reply reloads");
    let request = peer.join().expect("the peer answered");
    let wire: serde_json::Value = serde_json::from_str(&request).expect("JSON on the wire");
    assert_eq!(wire["protocol"], 1);
    // The scoped reload is its own verb: an old bar has no such `type`
    // and answers `unknown request` with nothing applied. It is never
    // `reload` with a `scope` field, which an old bar would misread as a
    // plain reload (applying pending edits) before answering ok.
    assert_eq!(wire["type"], "reload-look");
    assert!(wire.get("scope").is_none(), "no scope field: {wire}");
}

#[cfg(target_os = "linux")]
#[test]
fn an_old_bar_unknown_verb_is_a_predates_refusal() {
    // A bar older than the look scope has no `reload-look` verb: this is
    // the byte-for-byte reply a real old bar sends (its parser answers an
    // unknown `type` with an ordinary error, applying nothing).
    let (_scratch, socket) = socket_path("oldbar");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"unknown request `reload-look`\"}\n",
    );
    let error = send_bar_reload(&socket).expect_err("an old bar refuses");
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("predates the look-scoped reload"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn an_ok_without_the_scope_receipt_is_a_refusal() {
    // A scoped reply must carry its receipt: a plain `ok` answers a
    // different request. Refuse it loudly, never retry as plain.
    let (_scratch, socket) = socket_path("plainok");
    let peer = serve_once(socket.clone(), "{\"type\":\"ok\"}\n");
    let error = send_bar_reload(&socket).expect_err("a plain ok refuses");
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("without its scope receipt"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn a_bar_reply_is_not_a_reload_receipt() {
    // `{"type":"bar"}` answers a hide/show/toggle, never a reload: it must
    // refuse as an unexpected reply, not count as reloaded.
    let (_scratch, socket) = socket_path("barreply");
    let peer = serve_once(socket.clone(), "{\"type\":\"bar\",\"visible\":true}\n");
    let error = send_bar_reload(&socket).expect_err("a bar reply refuses");
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("unexpected reply"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn an_error_reply_is_the_refusal() {
    let (_scratch, socket) = socket_path("error");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"bad layer\"}\n",
    );
    let error = send_bar_reload(&socket).expect_err("an error reply refuses");
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("bad layer"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn a_bar_close_after_the_write_is_sent_but_unanswered() {
    let (_scratch, socket) = socket_path("closed");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("a scratch socket");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        // Read the request first, then close cleanly: the client sees EOF,
        // not a reset.
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("one request line");
        drop(reader);
        stream.shutdown(std::net::Shutdown::Both).ok();
    });
    let error = send_bar_reload(&socket).expect_err("a close is unanswered");
    assert!(
        matches!(error, super::ReloadError::Unanswered(_)),
        "sent-but-unanswered: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("without replying"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer ran");
}

#[cfg(target_os = "linux")]
#[test]
fn a_bar_hung_peer_is_sent_but_unanswered_within_the_bound() {
    // The timeout fake, mirrored from the compositor's: the peer reads the
    // request and never answers. The reload must refuse as `Unanswered`
    // within the bound, never wedge the swap (production waits 30 s; the
    // test binds 300 ms and asserts well under it).
    use std::time::Instant;
    let (_scratch, socket) = socket_path("barhung");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("a scratch socket");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("one request line");
        std::thread::sleep(Duration::from_secs(30));
    });
    let start = Instant::now();
    let error = bar_reload_outcome_with_timeout(&socket, Duration::from_millis(300))
        .expect_err("a hung bar is unanswered");
    assert!(
        matches!(error, ReloadError::Unanswered(_)),
        "sent-but-unanswered: {error:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "the wait is bounded: {:?}",
        start.elapsed()
    );
    // Detached, not joined: the peer sleeps past the test (joining would
    // wait out its 30 s sleep for no reason).
    drop(peer);
}

#[cfg(target_os = "linux")]
#[test]
fn a_missing_socket_is_a_reachable_refusal() {
    let missing = std::env::temp_dir().join(format!("slt-absent-{}.sock", std::process::id()));
    let error = send_bar_reload(&missing).expect_err("nothing listens");
    assert!(
        format!("{error:?}").contains("nothing listens"),
        "unexpected refusal: {error:?}"
    );
    // The production outcome skips an absent listener with a note instead.
    assert!(!super::bar_reload_outcome(&missing).expect("absent is not a refusal"));
}

/// The compositor wire below needs the same socket-timeout syscalls as the
/// bar's, so it shares the Linux gate (see above).
#[cfg(target_os = "linux")]
#[test]
fn a_partially_applied_reloaded_reply_is_ok_with_refused() {
    // The faithful fake of the real `Reloaded` wire: the real `type` tag
    // (`reloaded`), the real field names (`applied`, `refused`,
    // `shadowed`, `invalid`, `scope`) and the real scope value (`look`).
    // A non-empty `refused` is partially applied (the applied fields
    // landed: the compositor commits its layer identity after the apply),
    // so it reads as `Ok` — `apply` turns the list into the swap failure
    // and re-runs this peer on rollback, rather than skipping it as never
    // accepted.
    let (_scratch, socket) = socket_path("partial");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"reloaded\",\"applied\":[\"appearance.background_color\"],\"refused\":[\"output:scale\"],\"shadowed\":[],\"invalid\":[],\"scope\":\"look\"}\n",
    );
    let mut compositor = IpcCompositor::explicit(socket);
    let reloaded = compositor.reload_look().expect("a partial reply is Ok");
    assert_eq!(reloaded.applied, vec!["appearance.background_color"]);
    assert_eq!(reloaded.refused, vec!["output:scale"]);
    let request = peer.join().expect("the peer answered");
    let wire: serde_json::Value = serde_json::from_str(&request).expect("JSON on the wire");
    assert_eq!(wire["type"], "reload_look");
}

#[cfg(target_os = "linux")]
#[test]
fn a_clean_reloaded_reply_is_ok() {
    let (_scratch, socket) = socket_path("clean");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"reloaded\",\"applied\":[\"appearance.background_color\"],\"refused\":[],\"shadowed\":[],\"invalid\":[],\"scope\":\"look\"}\n",
    );
    let mut compositor = IpcCompositor::explicit(socket);
    let reloaded = compositor.reload_look().expect("a clean reply is Ok");
    assert_eq!(reloaded.applied, vec!["appearance.background_color"]);
    assert!(reloaded.refused.is_empty());
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn an_explicit_error_reply_is_a_never_accepted_refusal() {
    // An explicit error reply never applied anything: `Refused`, never
    // `Unanswered` (the rollback skips re-running it).
    let (_scratch, socket) = socket_path("comperror");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"bad layer\"}\n",
    );
    let mut compositor = IpcCompositor::explicit(socket);
    let error = compositor
        .reload_look()
        .expect_err("an error reply refuses");
    assert!(
        format!("{error:?}").contains("bad layer"),
        "unexpected refusal: {error:?}"
    );
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn a_predates_wall_is_a_never_accepted_refusal() {
    // An older compositor answers the unknown `ReloadLook` tag with a
    // serde wall: an ordinary error, which refuses without a re-run.
    let (_scratch, socket) = socket_path("comppredates");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"unknown variant `reload_look`, expected `reload`\"}\n",
    );
    let mut compositor = IpcCompositor::explicit(socket);
    let error = compositor
        .reload_look()
        .expect_err("a predates wall refuses");
    assert!(
        matches!(error, super::ReloadError::Refused(_)),
        "never accepted: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("predates"),
        "unexpected refusal: {error:?}"
    );
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn a_close_after_the_write_is_sent_but_unanswered() {
    // The peer reads the request, then closes cleanly: sent, but no
    // definitive answer — it may have applied first, so `Unanswered` (the
    // rollback re-runs it), never a never-accepted `Refused`.
    let (_scratch, socket) = socket_path("compclosed");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("a scratch socket");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("one request line");
        drop(reader);
        stream.shutdown(std::net::Shutdown::Both).ok();
    });
    let mut compositor = IpcCompositor::explicit(socket);
    let error = compositor.reload_look().expect_err("a close is unanswered");
    assert!(
        matches!(error, super::ReloadError::Unanswered(_)),
        "sent-but-unanswered: {error:?}"
    );
    peer.join().expect("the peer ran");
}

#[cfg(target_os = "linux")]
#[test]
fn a_hung_peer_is_sent_but_unanswered_within_the_bound() {
    // The timeout fake: the peer reads the request and never answers. The
    // reload must refuse as `Unanswered` within the bound, never wedge the
    // swap (production waits 30 s; the test binds 300 ms and asserts well
    // under it).
    use std::time::Instant;
    let (_scratch, socket) = socket_path("comphung");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("a scratch socket");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("one request line");
        std::thread::sleep(Duration::from_secs(30));
    });
    let start = Instant::now();
    let mut compositor = IpcCompositor::with_timeout(socket, Duration::from_millis(300));
    let error = compositor
        .reload_look()
        .expect_err("a hung peer is unanswered");
    assert!(
        matches!(error, super::ReloadError::Unanswered(_)),
        "sent-but-unanswered: {error:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "the wait is bounded: {:?}",
        start.elapsed()
    );
    // Detached, not joined: the peer sleeps past the test (joining would
    // wait out its 30 s sleep for no reason).
    drop(peer);
}
