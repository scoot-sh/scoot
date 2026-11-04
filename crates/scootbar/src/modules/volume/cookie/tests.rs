use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use super::{COOKIE_LEN, read_cookie};

/// A scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "scootbar-cookie-{}-{nanos}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pattern() -> [u8; COOKIE_LEN] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1))
}

#[test]
fn a_regular_256_byte_cookie_is_read() {
    let scratch = Scratch::new("ok");
    let path = scratch.file("cookie", &pattern());
    assert_eq!(read_cookie(&path), Some(pattern()));
}

#[test]
fn a_missing_cookie_is_none() {
    let scratch = Scratch::new("missing");
    assert_eq!(read_cookie(&scratch.0.join("cookie")), None);
}

#[test]
fn a_cookie_of_the_wrong_size_is_none() {
    let scratch = Scratch::new("size");
    for len in [0, 1, COOKIE_LEN - 1, COOKIE_LEN + 1, 4096] {
        let path = scratch.file("cookie", &vec![7u8; len]);
        assert_eq!(read_cookie(&path), None, "{len} bytes");
    }
}

#[test]
fn a_huge_sparse_file_is_refused_without_being_read_whole() {
    let scratch = Scratch::new("huge");
    let path = scratch.file("cookie", &pattern());
    // 1 TiB sparse: a read-everything implementation would try to
    // allocate it.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1 << 40)
        .unwrap();
    assert_eq!(read_cookie(&path), None);
}

#[test]
fn a_symlink_to_a_cookie_is_followed() {
    // Dotfile managers link this file; the check is on the opened fd.
    let scratch = Scratch::new("link");
    let target = scratch.file("real", &pattern());
    let link = scratch.0.join("cookie");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(read_cookie(&link), Some(pattern()));
    // A dangling one is a missing file.
    std::fs::remove_file(&target).unwrap();
    assert_eq!(read_cookie(&link), None);
}

#[test]
fn a_directory_or_device_at_the_path_is_none() {
    let scratch = Scratch::new("kinds");
    assert_eq!(read_cookie(&scratch.0), None);
    assert_eq!(read_cookie(Path::new("/dev/zero")), None);
    assert_eq!(read_cookie(Path::new("/dev/null")), None);
}

#[test]
fn a_fifo_at_the_path_returns_promptly() {
    let scratch = Scratch::new("fifo");
    let fifo = scratch.0.join("cookie");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    // On a thread, so a regression fails here instead of hanging the run.
    let (tx, rx) = mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_cookie(&path));
    });
    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("read_cookie blocked on a FIFO");
    assert_eq!(got, None);
}

#[test]
fn a_fifo_holding_exactly_a_cookie_is_refused_by_the_type_check_alone() {
    use rustix::fs::{Mode, OFlags, open};
    use std::io::Write;

    let scratch = Scratch::new("fifo256");
    let fifo = scratch.0.join("cookie");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, Mode::from_raw_mode(0o600)).unwrap();
    // A reader of our own keeps the pipe (and its data) alive; then 256
    // bytes go in and the writer closes. Without the regular-file check,
    // `read_cookie` would read exactly a cookie and then see EOF.
    let keep = open(&fifo, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty()).unwrap();
    {
        let writer = open(&fifo, OFlags::WRONLY | OFlags::NONBLOCK, Mode::empty()).unwrap();
        std::fs::File::from(writer).write_all(&pattern()).unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_cookie(&path));
    });
    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("read_cookie blocked on a FIFO");
    drop(keep);
    assert_eq!(got, None);
}
