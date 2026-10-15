//! Reload tests: display sanitizing, the runtime-dir rule, and the bar
//! wire against a scripted socket peer.

#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use super::send_bar_reload;
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

#[cfg(target_os = "linux")]
fn socket_path(tag: &str) -> PathBuf {
    // Short names: a Unix socket path must fit in `sun_path` (~104 bytes),
    // and the temp dir alone is already half of that.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("slt-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir.join(format!("{tag}.sock"))
}

/// The socket peers below need `SO_RCVTIMEO`/`SO_SNDTIMEO` on Unix
/// sockets, which this macOS toolchain refuses with `EINVAL`: they run on
/// Linux (CI and the dev VM), where the bar runs too.
#[cfg(target_os = "linux")]
#[test]
fn a_look_reload_sends_the_scoped_request_and_accepts_ok() {
    let socket = socket_path("ok");
    let peer = serve_once(socket.clone(), "{\"type\":\"ok\"}\n");
    send_bar_reload(&socket).expect("an ok reply reloads");
    let request = peer.join().expect("the peer answered");
    let wire: serde_json::Value = serde_json::from_str(&request).expect("JSON on the wire");
    assert_eq!(wire["protocol"], 1);
    assert_eq!(wire["type"], "reload");
    assert_eq!(wire["scope"], "look");
}

#[cfg(target_os = "linux")]
#[test]
fn an_error_reply_is_the_refusal() {
    let socket = socket_path("error");
    let peer = serve_once(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"bad layer\"}\n",
    );
    let error = send_bar_reload(&socket).expect_err("an error reply refuses");
    assert!(error.contains("bad layer"), "unexpected refusal: {error}");
    peer.join().expect("the peer answered");
}

#[cfg(target_os = "linux")]
#[test]
fn a_closed_connection_is_a_named_refusal() {
    let socket = socket_path("closed");
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
    let error = send_bar_reload(&socket).expect_err("a close refuses");
    assert!(
        error.contains("without replying"),
        "unexpected refusal: {error}"
    );
    peer.join().expect("the peer ran");
}

#[cfg(target_os = "linux")]
#[test]
fn a_missing_socket_is_a_reachable_refusal() {
    let missing = std::env::temp_dir().join(format!("slt-absent-{}.sock", std::process::id()));
    let error = send_bar_reload(&missing).expect_err("nothing listens");
    assert!(
        error.contains("cannot reach"),
        "unexpected refusal: {error}"
    );
}
