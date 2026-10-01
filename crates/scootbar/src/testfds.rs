//! Test support for "a child inherits none of the caller's descriptors".
//!
//! Such a test cannot demand that a child holds nothing above stdio: the
//! test process was itself started by something (a CI runner, `nohup`, a
//! shell with `exec 4<file`) that may have left descriptors open without
//! close-on-exec, and every child inherits those, correctly. What the tests
//! check is that the child holds nothing the *process under test* opened
//! itself, so they compare the child's set with [`inheritable`], the
//! descriptors this process would hand to any child it starts.

use std::collections::BTreeSet;
use std::fs;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// `O_CLOEXEC` on Linux, as `/proc/self/fdinfo` prints it (octal).
const O_CLOEXEC: u32 = 0o2_000_000;

static SPAWN_AND_LIST: Mutex<()> = Mutex::new(());

/// Held by a test that lists a child's descriptors, and by one that leaves
/// a descriptor without close-on-exec on purpose (the control of the
/// check). `cargo test` runs tests side by side in one process, and the
/// control's descriptor would otherwise reach a sibling's child while that
/// sibling's baseline did not have it.
pub fn serialize() -> MutexGuard<'static, ()> {
    SPAWN_AND_LIST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The descriptors this process would hand to a child it starts: every one
/// open now without close-on-exec, stdio included.
pub fn inheritable() -> BTreeSet<i32> {
    let mut set = BTreeSet::new();
    let Ok(entries) = fs::read_dir("/proc/self/fd") else {
        return set;
    };
    for entry in entries.flatten() {
        let Some(fd) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        // An entry that closed between the listing and now (the directory's
        // own descriptor) has no fdinfo and is not inherited.
        let Ok(info) = fs::read_to_string(format!("/proc/self/fdinfo/{fd}")) else {
            continue;
        };
        if flags(&info).is_some_and(|flags| flags & O_CLOEXEC == 0) {
            set.insert(fd);
        }
    }
    set
}

fn flags(fdinfo: &str) -> Option<u32> {
    let line = fdinfo
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))?;
    u32::from_str_radix(line.trim(), 8).ok()
}

/// The descriptors a child may hold, given that it was started by a process
/// whose [`inheritable`] set was `inherited`, and listed itself with `ls`:
/// stdio, what it inherited, and the one `ls` opens to read the directory,
/// which is the lowest number still free. A leaked descriptor cannot hide
/// behind that one: it would take the lowest free number itself and push
/// `ls`'s to the next, which is then not allowed.
pub fn allowed_in_child(inherited: &BTreeSet<i32>) -> BTreeSet<i32> {
    let mut allowed: BTreeSet<i32> = inherited.clone();
    allowed.extend(0..=2);
    let lowest_free = (3..).find(|fd| !allowed.contains(fd)).unwrap_or(3);
    allowed.insert(lowest_free);
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_reads_the_octal_field() {
        assert_eq!(
            flags("pos:\t0\nflags:\t02100002\nmnt_id:\t5\n"),
            Some(0o2_100_002)
        );
        assert_eq!(flags("pos:\t0\n"), None);
    }

    #[test]
    fn a_cloexec_descriptor_is_not_inheritable_and_a_cleared_one_is() {
        let _held = serialize();
        let file = std::fs::File::open("/dev/null").expect("open");
        let fd = rustix::io::fcntl_dupfd_cloexec(&file, 100).expect("dup");
        let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
        assert!(!inheritable().contains(&raw));
        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::empty()).expect("clear");
        assert!(inheritable().contains(&raw));
    }

    #[test]
    fn the_lowest_free_descriptor_is_the_one_ls_may_hold() {
        let inherited: BTreeSet<i32> = [0, 1, 2, 4, 5, 142].into();
        let allowed = allowed_in_child(&inherited);
        assert_eq!(allowed, [0, 1, 2, 3, 4, 5, 142].into());
        let none: BTreeSet<i32> = [0, 1, 2].into();
        assert_eq!(allowed_in_child(&none), [0, 1, 2, 3].into());
    }
}
