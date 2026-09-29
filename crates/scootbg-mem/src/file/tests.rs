use std::io::Write;

use rustix::fs::StatVfsMountFlags;

use super::{Premises, is_read_only_mount, map_if_immutable, may_map};

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
    assert_eq!(map_if_immutable(&file, 1 << 20).unwrap(), None);
}

#[test]
fn a_directory_is_not_mapped() {
    let dir = std::fs::File::open(std::env::temp_dir()).unwrap();
    assert_eq!(map_if_immutable(&dir, 1 << 20).unwrap(), None);
}

/// A file that passes every premise: `/nix/store` where it is mounted
/// read-only (NixOS), whose files are root's and `0444`. Skipped elsewhere;
/// the mapping was also checked by hand on a read-only bind mount of a
/// root-owned `0444` copy (the PR's evidence).
#[test]
fn a_file_on_a_read_only_mount_is_mapped_whole() {
    let Some(path) = read_only_file() else {
        eprintln!("skipped -- no regular file on a read-only mount found");
        return;
    };
    let expected = std::fs::read(&path).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let len = expected.len() as u64;
    let mapped = map_if_immutable(&file, len)
        .unwrap()
        .expect("a file on a read-only mount is mapped");
    assert_eq!(mapped, &expected[..]);
    // Too large for the cap: not mapped.
    let file = std::fs::File::open(&path).unwrap();
    assert_eq!(map_if_immutable(&file, len - 1).unwrap(), None);
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

/// Each premise is needed on its own: a read-only mount of a file someone
/// can write (the review's `SIGBUS`: a bind mount, `ProtectHome`, flatpak),
/// a file not root's, and a file with any write bit.
#[test]
fn every_premise_is_required() {
    let store = Premises {
        read_only_mount: true,
        owner: 0,
        mode: 0o100_444,
    };
    assert!(may_map(&store));
    assert!(may_map(&Premises {
        mode: 0o100_555,
        ..store
    }));
    assert!(may_map(&Premises {
        mode: 0o100_400,
        ..store
    }));
    // A writable mount.
    assert!(!may_map(&Premises {
        read_only_mount: false,
        ..store
    }));
    // A user's file behind a read-only view of their home.
    assert!(!may_map(&Premises {
        owner: 1000,
        ..store
    }));
    // Any write bit: owner, group or other.
    for bits in [0o200, 0o020, 0o002, 0o644, 0o666, 0o755] {
        assert!(
            !may_map(&Premises {
                mode: 0o100_000 | bits,
                ..store
            }),
            "mode {bits:o}"
        );
    }
}

/// A file with no write bit on a writable mount is still read: the mode
/// alone is not enough (its owner can `chmod` it back and truncate it).
#[test]
fn an_unwritable_file_on_a_writable_mount_is_not_mapped() {
    let scratch = Scratch::new("readonly-mode", b"font bytes");
    if on_read_only_mount(&scratch.0) {
        eprintln!("skipped -- the temp dir is on a read-only mount");
        return;
    }
    let mut permissions = std::fs::metadata(&scratch.0).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o444);
    std::fs::set_permissions(&scratch.0, permissions).unwrap();
    let file = std::fs::File::open(&scratch.0).unwrap();
    assert_eq!(map_if_immutable(&file, 1 << 20).unwrap(), None);
}
