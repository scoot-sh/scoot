//! The toggle's tests: paths, the close handshake, stale files.

use super::*;
use std::io::Write as _;

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

// F1: a lock holder with no panel is a visible error, not a silent yield.
#[test]
fn a_lock_holder_with_no_panel_is_a_visible_error() {
    use rustix::fs::{FlockOperation, flock};
    let dir = scratch("wedge");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let lock_path = lock_path_for(&path);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("lock opens");
    flock(&lock, FlockOperation::NonBlockingLockExclusive).expect("holder takes the lock");
    // No panel: nothing answers. The second press must not silently claim
    // success.
    match open(&path) {
        Ok(Open::Signalled) => panic!("a lock with no panel must not yield Signalled"),
        Ok(Open::Listen(_)) => panic!("a held lock must not yield a second panel"),
        Err(_) => {}
    }
    drop(lock);
    std::fs::remove_dir_all(&dir).ok();
}

// A simultaneous open: the loser waits for the winner's panel and must
// then yield without signalling, or the panel that just won is closed.
#[test]
fn losing_a_simultaneous_open_does_not_close_the_winner() {
    use rustix::fs::{FlockOperation, flock};
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let dir = scratch("loser");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path_for(&path))
        .expect("lock opens");
    flock(&lock, FlockOperation::NonBlockingLockExclusive).expect("winner takes the lock");
    let stop = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(Mutex::new(Vec::<UnixStream>::new()));
    let panel = {
        let (path, stop, accepted) = (path.clone(), stop.clone(), accepted.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let listener = UnixListener::bind(&path).expect("winner binds");
            listener.set_nonblocking(true).expect("nonblocking");
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => accepted.lock().expect("lock").push(stream),
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        })
    };
    let outcome = open(&path);
    std::thread::sleep(std::time::Duration::from_millis(50));
    stop.store(true, Ordering::Relaxed);
    panel.join().expect("panel thread");
    assert!(matches!(outcome, Ok(Open::Signalled)), "the loser yields");
    for mut stream in accepted.lock().expect("lock").drain(..) {
        stream.set_nonblocking(true).expect("nonblocking");
        let mut got = Vec::new();
        let _ = stream.read_to_end(&mut got);
        assert!(
            !got.starts_with(b"close"),
            "the loser closed the panel that won: {got:?}"
        );
    }
    drop(lock);
    std::fs::remove_dir_all(&dir).ok();
}

// F2: a listener that stopped with a full backlog must not hang the press.
#[test]
fn a_full_backlog_does_not_hang_the_press() {
    use rustix::net::{
        AddressFamily, SocketAddrUnix, SocketFlags, SocketType, bind, connect, listen, socket_with,
    };
    let dir = scratch("backlog");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let address = SocketAddrUnix::new(&path).expect("listener address");
    // A backlog of 0 (the kernel admits one pending connection past it),
    // not std's default: `UnixListener::bind` asks for the kernel's whole
    // `somaxconn` (4096 here), and filling that holds one fd per
    // connection, so under a default `ulimit -n 1024` the probe sockets ran
    // out (EMFILE) before the backlog ever filled. What is under test, a
    // deaf listener with no room left, does not depend on how deep the
    // backlog was.
    let listener = {
        let fd = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .expect("listener socket");
        bind(&fd, &address).expect("plant listener");
        listen(&fd, 0).expect("listen");
        UnixListener::from(fd)
    };
    // Fill the backlog with nonblocking connects until the kernel says
    // room is gone (EAGAIN): never accept, so the listener stays deaf.
    // A blocking connect would now wait for an accept that never comes.
    let mut held = Vec::new();
    let mut filled = false;
    for _ in 0..64 {
        let probe = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .expect("probe socket");
        match connect(&probe, &address) {
            Ok(()) | Err(rustix::io::Errno::INPROGRESS) => {
                held.push(probe);
            }
            Err(rustix::io::Errno::AGAIN) => {
                filled = true;
                break;
            }
            Err(_) => break,
        }
    }
    assert!(filled, "the deaf backlog really filled (EAGAIN observed)");
    assert!(!held.is_empty(), "the backlog holds connections");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let path_clone = path.clone();
    std::thread::spawn(move || {
        let result = open(&path_clone);
        let _ = done_tx.send(result);
    });
    let result = done_rx
        .recv_timeout(std::time::Duration::from_secs(8))
        .expect("the press returns within 8s even with a full deaf backlog");
    assert!(
        matches!(result, Ok(Open::Signalled)),
        "a live (deaf) panel still toggles"
    );
    drop(held);
    drop(listener);
    std::fs::remove_dir_all(&dir).ok();
}

// F3: clearing a stale socket leaves the name resolving (rename replaces).
#[test]
fn clearing_a_stale_socket_leaves_no_missing_name() {
    let dir = scratch("nogap");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let stale = UnixListener::bind(&path).expect("plant stale");
    drop(stale);
    // Stale: bound file with no listener.
    clear_stale(&path).expect("stale is checkable");
    assert!(
        std::fs::symlink_metadata(&path).is_ok(),
        "checking a stale socket must not unlink it: the rename replaces it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// F4: staging through a symlink is refused (fd-pinned, O_NOFOLLOW).
#[test]
fn staging_through_a_symlink_is_refused() {
    let dir = scratch("staginglink");
    let real = dir.join("real");
    std::fs::create_dir_all(&real).expect("real dir makes");
    let link = dir.join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("symlink makes");
    let target = dir.join("scootlaunch-wayland-1.sock");
    assert!(
        stage(&link, &target).is_err(),
        "a symlinked staging dir must be refused"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// F5: a symlinked lock is refused, not followed.
#[test]
fn a_symlinked_lock_is_refused() {
    let dir = scratch("locklink");
    let path = dir.join("scootlaunch-wayland-1.sock");
    let lock_path = lock_path_for(&path);
    let real = dir.join("real.lock");
    std::fs::write(&real, b"real").expect("real lock makes");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &lock_path).expect("symlink makes");
    assert!(
        open(&path).is_err(),
        "a symlinked lock must be refused, not followed"
    );
    assert_eq!(
        std::fs::read(&real).expect("target intact"),
        b"real",
        "the link target is left alone"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// F6: an overlong path gets the friendly length error.
#[test]
fn an_overlong_path_gets_the_friendly_length_error() {
    let dir = scratch("toolong");
    let long_name = format!("scootlaunch-{}.sock", "w".repeat(120));
    let path = dir.join(long_name);
    assert!(
        path.as_os_str().len() > SUN_PATH_MAX,
        "the fixture really is overlong"
    );
    let error = match open(&path) {
        Ok(_) => panic!("an overlong path must be refused"),
        Err(error) => error,
    };
    assert!(
        error.contains("at most 107"),
        "the friendly length message, not a probe errno: {error}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
