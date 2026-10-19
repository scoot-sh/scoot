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
fn a_stale_file_is_taken_over() {
    let dir = scratch("stale");
    let path = dir.join("scootlaunch-wayland-1.sock");
    std::fs::write(&path, b"stale").expect("plant");
    let claim = match open(&path).expect("stale is taken over") {
        Open::Listen(claim) => claim,
        Open::Signalled => panic!("no listener lived"),
    };
    // The stale bytes are gone: the path is a live socket again.
    assert!(UnixStream::connect(&path).is_ok());
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
