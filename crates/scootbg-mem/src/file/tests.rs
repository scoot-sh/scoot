use std::io::Write;

use rustix::fs::StatVfsMountFlags;

use super::{is_read_only_mount, map_if_read_only};

/// A scratch file under the target directory's temp, removed on drop.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str, bytes: &[u8]) -> Self {
        let path =
            std::env::temp_dir().join(format!("scootbg-mem-file-{}-{tag}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn on_read_only_mount(path: &std::path::Path) -> bool {
    let file = std::fs::File::open(path).unwrap();
    is_read_only_mount(&rustix::fs::fstatvfs(&file).unwrap().f_flag)
}

#[test]
fn a_file_on_a_writable_mount_is_not_mapped() {
    let scratch = Scratch::new("writable", b"font bytes");
    if on_read_only_mount(&scratch.0) {
        eprintln!("skipped -- the temp dir is on a read-only mount");
        return;
    }
    let file = std::fs::File::open(&scratch.0).unwrap();
    assert_eq!(map_if_read_only(&file, 1 << 20).unwrap(), None);
}

#[test]
fn a_directory_is_not_mapped() {
    let dir = std::fs::File::open(std::env::temp_dir()).unwrap();
    assert_eq!(map_if_read_only(&dir, 1 << 20).unwrap(), None);
}

/// A read-only mount the test can find: `/nix/store` where it is mounted
/// read-only (NixOS). Skipped elsewhere; the mapping was also checked by
/// hand on a read-only bind mount (the PR's evidence).
#[test]
fn a_file_on_a_read_only_mount_is_mapped_whole() {
    let Some(path) = read_only_file() else {
        eprintln!("skipped -- no regular file on a read-only mount found");
        return;
    };
    let expected = std::fs::read(&path).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let len = expected.len() as u64;
    let mapped = map_if_read_only(&file, len)
        .unwrap()
        .expect("a file on a read-only mount is mapped");
    assert_eq!(mapped, &expected[..]);
    // Too large for the cap: not mapped.
    let file = std::fs::File::open(&path).unwrap();
    assert_eq!(map_if_read_only(&file, len - 1).unwrap(), None);
}

fn read_only_file() -> Option<std::path::PathBuf> {
    let store = std::fs::read_dir("/nix/store").ok()?;
    for entry in store.filter_map(Result::ok).take(200) {
        let path = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_file() && meta.len() > 0 && meta.len() < (1 << 20) && on_read_only_mount(&path) {
            return Some(path);
        }
    }
    None
}

#[test]
fn the_read_only_flag_is_what_decides() {
    assert!(is_read_only_mount(&StatVfsMountFlags::RDONLY));
    assert!(is_read_only_mount(
        &(StatVfsMountFlags::RDONLY | StatVfsMountFlags::NOSUID)
    ));
    assert!(!is_read_only_mount(&StatVfsMountFlags::NOSUID));
    assert!(!is_read_only_mount(&StatVfsMountFlags::empty()));
}
