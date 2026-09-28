use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{Saver, temp_path, write_atomic, write_through};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sbg-saver-{}-{tag}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn writes_are_whole_private_and_leave_nothing_behind() {
    let scratch = Scratch::new("atomic");
    let dir = scratch.0.join("scootbg");
    let file = dir.join("default");
    write_atomic(&file, b"one\n").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"one\n");
    assert_eq!(mode(&dir), 0o700, "a directory it makes is private");
    assert_eq!(mode(&file), 0o600, "so is the file");
    write_atomic(&file, b"two, longer\n").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"two, longer\n");
    write_atomic(&file, b"3\n").unwrap();
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"3\n",
        "not a shorter overwrite"
    );
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["default"]);
}

#[test]
fn a_write_that_fails_says_so_and_leaves_the_old_file() {
    let scratch = Scratch::new("fails");
    let file = scratch.0.join("default");
    write_atomic(&file, b"old\n").unwrap();
    // The target is a non-empty directory now: the rename must fail.
    let blocked = scratch.0.join("blocked");
    std::fs::create_dir_all(blocked.join("inside")).unwrap();
    assert!(write_atomic(&blocked, b"new\n").is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"old\n");
    let temps = std::fs::read_dir(&scratch.0)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(temps, 0, "the temporary file is removed");
}

/// Many saves in a burst end with the last one on disk, and `flush`
/// returns once it is.
#[test]
fn the_last_save_wins() {
    let scratch = Scratch::new("burst");
    let file = scratch.0.join("default");
    let saver = Saver::new(file.clone());
    for i in 0..200 {
        saver.save(format!("save {i}\n"));
    }
    assert!(saver.flush(Duration::from_secs(20)));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "save 199\n");
    // Nothing running: flush returns at once.
    let started = Instant::now();
    assert!(saver.flush(Duration::from_secs(20)));
    assert!(started.elapsed() < Duration::from_secs(1));
    // And it can save again after going idle.
    saver.save("again\n".to_owned());
    assert!(saver.flush(Duration::from_secs(20)));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "again\n");
}

/// A save that cannot be written is a warning, not a hang: the thread
/// still ends and `flush` returns.
#[test]
fn a_failing_save_does_not_wedge() {
    let scratch = Scratch::new("wedge");
    std::fs::create_dir_all(&scratch.0).unwrap();
    // A file where the directory should be.
    let parent = scratch.0.join("not-a-dir");
    std::fs::write(&parent, b"").unwrap();
    let saver = Saver::new(parent.join("default"));
    saver.save("x\n".to_owned());
    assert!(saver.flush(Duration::from_secs(20)));
}

/// A symbolic link put at the temporary name is not followed: the write
/// is refused (`O_EXCL`), the file it points at is untouched, and so is
/// the state file. Dangling links too (`O_EXCL` would otherwise create
/// their target).
#[test]
fn a_symlink_at_the_temporary_name_is_not_followed() {
    let scratch = Scratch::new("symlink");
    let dir = scratch.0.join("scootbg");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("default");
    std::fs::write(&file, b"before\n").unwrap();
    // A name guessed right: `write_through` takes the one `write_atomic`
    // draws at random.
    let temp = temp_path(&file, 7);
    let victim = scratch.0.join("victim");
    std::fs::write(&victim, b"precious\n").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::os::unix::fs::symlink(&victim, &temp).unwrap();
    let refused = write_through(&file, b"state\n", 7).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious\n", "untouched");
    assert_eq!(mode(&victim), 0o644);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"before\n",
        "state file untouched"
    );
    assert!(
        std::fs::symlink_metadata(&temp)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is left where it was"
    );

    std::fs::remove_file(&temp).unwrap();
    let nowhere = scratch.0.join("never-made");
    std::os::unix::fs::symlink(&nowhere, &temp).unwrap();
    assert!(write_through(&file, b"again\n", 7).is_err());
    assert!(!nowhere.exists(), "a dangling link's target is not created");
    assert_eq!(std::fs::read(&file).unwrap(), b"before\n");

    // The next write draws its own name, and goes through.
    write_atomic(&file, b"state\n").unwrap();
    assert_eq!(mode(&file), 0o600);
    assert_eq!(std::fs::read(&file).unwrap(), b"state\n");
}

/// A file already at a write's own temporary name (a leftover, or another
/// writer that drew the same name) refuses that write rather than being
/// replaced or appended to, and is left as it is.
#[test]
fn a_file_at_the_temporary_name_refuses_the_write() {
    let scratch = Scratch::new("leftover");
    std::fs::create_dir_all(&scratch.0).unwrap();
    let file = scratch.0.join("default");
    let temp = temp_path(&file, 7);
    std::fs::write(&temp, b"half a file from before, and longer").unwrap();
    let refused = write_through(&file, b"whole\n", 7).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!file.exists(), "nothing renamed into place");
    assert_eq!(
        std::fs::read(&temp).unwrap(),
        b"half a file from before, and longer"
    );
}

/// Another writer's temporary file, even one of the same pid (a daemon
/// in another pid namespace, or on another host sharing the home) is
/// never touched: each write's name is its own, and nothing at a name is
/// removed, so no write can take another's file and leave it renaming a
/// half-written one into place (review of PR #315).
#[test]
fn another_writers_temporary_file_is_left_alone() {
    let scratch = Scratch::new("other");
    std::fs::create_dir_all(&scratch.0).unwrap();
    let file = scratch.0.join("default");
    let pid = rustix::process::getpid().as_raw_nonzero();
    // The name every write of this pid used before, and one drawn like
    // `write_atomic`'s.
    let others = [
        scratch.0.join(format!(".default.{pid}.tmp")),
        temp_path(&file, super::unique()),
    ];
    for other in &others {
        std::fs::write(other, b"another writer, mid-write").unwrap();
    }
    for _ in 0..100 {
        write_atomic(&file, b"ours\n").unwrap();
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"ours\n");
    for other in &others {
        assert_eq!(
            std::fs::read(other).unwrap(),
            b"another writer, mid-write",
            "{other:?}"
        );
    }
    // Two writes never draw the same name.
    assert_ne!(
        temp_path(&file, super::unique()),
        temp_path(&file, super::unique())
    );
}

#[test]
fn a_directory_open_to_others_is_noticed() {
    let scratch = Scratch::new("exposed");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(super::exposed(&scratch.0), None);
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(super::exposed(&scratch.0), None, "readable is fine");
    for mode in [0o775, 0o757, 0o777] {
        std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(mode)).unwrap();
        let why = super::exposed(&scratch.0).unwrap();
        assert!(why.contains(&format!("{mode:03o}")), "{why}");
    }
    // Owned by someone else: `/` is root's; skip the case when running as
    // root, where it is ours.
    if rustix::process::getuid().as_raw() != 0 {
        assert!(
            super::exposed(std::path::Path::new("/"))
                .unwrap()
                .contains("owned by")
        );
    }
    assert_eq!(super::exposed(&scratch.0.join("missing")), None);
}

/// Where the kernel refuses `getrandom`, the stand-in still names every
/// write differently, so saving goes on (a seccomp'd container).
#[test]
fn the_fallback_never_repeats_a_name() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..10_000 {
        assert!(
            seen.insert(super::fallback_unique()),
            "a repeated fallback name"
        );
    }
}
