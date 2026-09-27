use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{Saver, write_atomic};

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
