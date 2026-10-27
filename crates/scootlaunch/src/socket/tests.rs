//! The toggle's tests: paths, the close handshake, stale files.

use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scootlaunch-socket-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch makes");
    dir
}

#[test]
fn paths_need_a_runtime_dir_and_a_display() {
    assert!(socket_path(None, Some(std::ffi::OsStr::new("wayland-1"))).is_err());
    assert!(socket_path(Some(std::ffi::OsStr::new("/run/user/1")), None).is_err());
    assert_eq!(
        socket_path(
            Some(std::ffi::OsStr::new("/run/user/1")),
            Some(std::ffi::OsStr::new("wayland-1"))
        )
        .expect("shapes"),
        PathBuf::from("/run/user/1/scootlaunch-wayland-1.sock")
    );
}

#[test]
fn a_hostile_display_is_refused() {
    for display in ["", "../x", "a/b", "a\nb", "a=b", "wayland-1\0"] {
        assert!(
            socket_path(
                Some(std::ffi::OsStr::new("/run/user/1")),
                Some(std::ffi::OsStr::new(display))
            )
            .is_err(),
            "{display:?}"
        );
    }
    let long = "w".repeat(65);
    assert!(
        socket_path(
            Some(std::ffi::OsStr::new("/run/user/1")),
            Some(long.as_ref())
        )
        .is_err()
    );
}

#[test]
fn a_second_open_signals_and_the_claim_sees_close() {
    let dir = scratch("signal");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let claim = match open(&path).expect("first listens") {
        Open::Listen(claim) => claim,
        Open::Signalled => panic!("nothing was listening"),
    };
    assert!(path.exists());
    assert!(matches!(open(&path), Ok(Open::Signalled)));
    assert!(claim.accept_close(), "the close line arrives");
    drop(claim);
    assert!(!path.exists(), "dropping the claim removes the file");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_stale_socket_is_taken_over() {
    let dir = scratch("stale");
    let path = dir.join("scootlaunch-wayland-1.sock");
    // A dead panel's socket file: bound, then the listener dropped
    // without removing the file (a crash leaves exactly this).
    let stale = UnixListener::bind(&path).expect("plant");
    drop(stale);
    let claim = match open(&path).expect("stale is taken over") {
        Open::Listen(claim) => claim,
        Open::Signalled => panic!("no listener lived"),
    };
    // The stale file is gone: the path is a live socket again.
    assert!(UnixStream::connect(&path).is_ok());
    drop(claim);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_regular_file_is_refused_not_removed() {
    let dir = scratch("not-socket");
    let path = dir.join("scootlaunch-wayland-1.sock");
    std::fs::write(&path, b"stale").expect("plant");
    assert!(open(&path).is_err(), "a regular file is refused");
    assert_eq!(
        std::fs::read(&path).expect("still there"),
        b"stale",
        "the refused file is left alone"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn two_simultaneous_opens_yield_one_panel() {
    use std::sync::{Arc, Barrier};
    let dir = scratch("race");
    let path = dir.join("scootlaunch-wayland-1.sock");
    // Two openers at the same instant: a barrier so both fail the
    // initial connect before either binds (no sleeps as sync).
    let barrier = Arc::new(Barrier::new(2));
    let path_a = path.clone();
    let barrier_a = barrier.clone();
    let first = std::thread::spawn(move || {
        barrier_a.wait();
        open(&path_a)
    });
    let path_b = path.clone();
    barrier.wait();
    let second = open(&path_b).expect("second opens");
    let first = first.join().expect("first joins").expect("first opens");
    let listens = [&first, &second]
        .iter()
        .filter(|result| matches!(result, Open::Listen(_)))
        .count();
    assert_eq!(
        listens, 1,
        "two simultaneous opens yield one panel, not two (one orphaned)"
    );
    // The surviving panel still answers; dropping both leaves no file.
    assert!(UnixStream::connect(&path).is_ok(), "one panel answers");
    drop(first);
    drop(second);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn socket_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("mode");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let claim = match open(&path).expect("listens") {
        Open::Listen(claim) => claim,
        Open::Signalled => panic!("nothing was listening"),
    };
    let mode = std::fs::metadata(&path)
        .expect("socket stats")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "the toggle socket is 0600, not umask-dependent"
    );
    drop(claim);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn garbage_from_a_peer_is_ignored() {
    let dir = scratch("garbage");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let claim = match open(&path).expect("listens") {
        Open::Listen(claim) => claim,
        Open::Signalled => panic!("nothing was listening"),
    };
    let mut rude = UnixStream::connect(&path).expect("connects");
    rude.write_all(b"hello\n").expect("writes");
    drop(rude);
    assert!(!claim.accept_close(), "garbage is not a close");
    assert!(matches!(open(&path), Ok(Open::Signalled)));
    assert!(claim.accept_close(), "the close line still arrives");
    drop(claim);
    std::fs::remove_dir_all(&dir).ok();
}
