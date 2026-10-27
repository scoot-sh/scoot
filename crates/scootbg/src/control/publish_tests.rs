//! The claim's publication: the socket is `0600` the moment its name exists,
//! whatever the umask, and a replaced stale socket never leaves the name
//! missing.

use std::collections::BTreeSet;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rustix::fs::Mode;

use super::{Claim, ClaimError};
use crate::paths::Paths;

/// `umask` is process-wide: tests that set it take turns.
static UMASK: Mutex<()> = Mutex::new(());

/// Sets the umask for its life and restores it on drop.
struct Umask {
    previous: Mode,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl Umask {
    fn set(mask: u32) -> Self {
        let turn = UMASK.lock().unwrap_or_else(|p| p.into_inner());
        let previous = rustix::process::umask(Mode::from_bits_truncate(mask));
        Self {
            previous,
            _turn: turn,
        }
    }
}

impl Drop for Umask {
    fn drop(&mut self) {
        rustix::process::umask(self.previous);
    }
}

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        use std::sync::atomic::AtomicU64;
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sbg-publish-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn paths(&self) -> Paths {
        Paths {
            display: "test".to_owned(),
            socket: self.path.join("scootbg-test.sock"),
            lock: self.path.join("scootbg-test.lock"),
        }
    }

    fn entries(&self) -> BTreeSet<String> {
        std::fs::read_dir(&self.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// What a client polling the published name sees while `work` runs: every
/// distinct mode of a socket at the name, and whether the name was ever
/// missing after it first appeared.
struct Seen {
    modes: BTreeSet<u32>,
    vanished: bool,
}

/// `work` is handed the count of times the watcher has found a socket at
/// the name, so it can keep going until the watcher has really looked (a
/// loaded machine may not schedule it for a while).
fn watching<T: Send>(path: &std::path::Path, work: impl FnOnce(&AtomicUsize) -> T) -> (T, Seen) {
    let stop = AtomicBool::new(false);
    let sightings = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            let mut seen = Seen {
                modes: BTreeSet::new(),
                vanished: false,
            };
            let mut appeared = false;
            while !stop.load(Ordering::Acquire) {
                match std::fs::symlink_metadata(path) {
                    Ok(metadata) => {
                        appeared = true;
                        if metadata.file_type().is_socket() {
                            seen.modes.insert(metadata.permissions().mode() & 0o777);
                            sightings.fetch_add(1, Ordering::Release);
                        }
                    }
                    Err(_) if appeared => seen.vanished = true,
                    Err(_) => {}
                }
            }
            seen
        });
        // Stops the watcher on every exit, a panic in `work` included:
        // the scope would otherwise join a thread that never ends.
        struct Stop<'a>(&'a AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let stopper = Stop(&stop);
        let value = work(&sightings);
        drop(stopper);
        (value, watcher.join().unwrap())
    })
}

/// Polls `condition` for up to 10 s (it never hangs a test on a watcher that
/// is not scheduled).
fn wait_until(condition: impl Fn() -> bool) {
    let started = Instant::now();
    while !condition() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Claims and releases once. A test thread elsewhere in the process may be
/// between `fork` and `exec` holding a copy of the just-closed lock fd, so
/// the kernel's `flock` can outlive `release` for an instant: retry that
/// one refusal rather than flake.
fn acquire_released(paths: &Paths) {
    for _ in 0..500 {
        match Claim::acquire(paths) {
            Ok(mut claim) => return claim.release(),
            Err(ClaimError::AlreadyRunning { .. }) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("{error}"),
        }
    }
    panic!("the lock stayed held after release");
}

#[test]
fn the_socket_is_owner_only_from_the_moment_its_name_exists() {
    for mask in [0o022, 0o000] {
        let _umask = Umask::set(mask);
        let scratch = Scratch::new("mode");
        let paths = scratch.paths();
        let ((), seen) = watching(&paths.socket, |sightings| {
            let started = Instant::now();
            let mut rounds = 0;
            // At least 400 rounds, and until the watcher has looked 50
            // times (but never past 10 s).
            while rounds < 400
                || (sightings.load(Ordering::Acquire) < 50
                    && started.elapsed() < Duration::from_secs(10))
            {
                acquire_released(&paths);
                rounds += 1;
            }
        });
        assert!(!seen.modes.is_empty(), "the watcher never saw the socket");
        assert_eq!(
            seen.modes,
            BTreeSet::from([0o600]),
            "umask {mask:03o}: a client found the socket with another mode"
        );
    }
}

#[test]
fn replacing_a_stale_socket_never_leaves_the_name_missing() {
    let _umask = Umask::set(0o022);
    let scratch = Scratch::new("stale");
    let paths = scratch.paths();
    drop(UnixListener::bind(&paths.socket).unwrap());
    let (claim, seen) = watching(&paths.socket, |sightings| {
        // Once the watcher has seen the stale socket, a missing name is a
        // gap in the handover.
        wait_until(|| sightings.load(Ordering::Acquire) >= 1);
        Claim::acquire(&paths).unwrap()
    });
    assert!(!seen.vanished, "the name was missing during the handover");
    assert!(UnixStream::connect(&paths.socket).is_ok());
    drop(claim);
}

#[test]
fn acquire_leaves_no_staging_directory_behind() {
    let scratch = Scratch::new("staging");
    let paths = scratch.paths();
    let expected = BTreeSet::from([
        "scootbg-test.sock".to_owned(),
        "scootbg-test.lock".to_owned(),
    ]);
    let claim = Claim::acquire(&paths).unwrap();
    assert_eq!(scratch.entries(), expected);
    // A refused second claim builds nothing either.
    assert!(matches!(
        Claim::acquire(&paths),
        Err(ClaimError::AlreadyRunning { .. })
    ));
    assert_eq!(scratch.entries(), expected);
    drop(claim);
    assert_eq!(
        scratch.entries(),
        BTreeSet::from(["scootbg-test.lock".to_owned()])
    );
}

#[test]
fn a_loser_leaves_the_winners_socket_and_lock_alone() {
    let scratch = Scratch::new("loser");
    let paths = scratch.paths();
    let winner = Claim::acquire(&paths).unwrap();
    let inode = std::fs::metadata(&paths.socket).unwrap().ino();
    for _ in 0..8 {
        assert!(matches!(
            Claim::acquire(&paths),
            Err(ClaimError::AlreadyRunning { .. })
        ));
    }
    // Same file, still connectable, still the winner's.
    assert_eq!(std::fs::metadata(&paths.socket).unwrap().ino(), inode);
    assert!(UnixStream::connect(&paths.socket).is_ok());
    drop(winner);
    assert!(!paths.socket.exists());
}

#[test]
fn a_failed_publish_removes_its_staging_and_the_socket_name() {
    let scratch = Scratch::new("toolong");
    let mut paths = scratch.paths();
    // Past `sun_path`: no bind can succeed, and nothing may be left over.
    paths.socket = scratch.path.join("s".repeat(120));
    assert!(matches!(
        Claim::acquire(&paths),
        Err(ClaimError::Io { what: "bind", .. })
    ));
    assert_eq!(
        scratch.entries(),
        BTreeSet::from(["scootbg-test.lock".to_owned()])
    );
}
