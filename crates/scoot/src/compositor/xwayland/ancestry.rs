//! Whether one process descends from another, by walking `/proc/<pid>/stat`
//! parent pids -- what binds an X startup-id redemption to the process its
//! token was minted for (`focus.rs`, rule 2).
//!
//! Heap-free: the path is formatted into a stack buffer (std opens a short
//! path through a stack C string too) and only the head of each `stat` file
//! is read, into another. It runs once per startup-id redemption -- a map
//! or a `_NET_ACTIVE_WINDOW`, never a per-event path -- and costs at most
//! [`MAX_ANCESTRY_DEPTH`] `open`/`read` pairs on procfs.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[cfg(test)]
mod tests;

/// How many parent links the walk follows before giving up. A wrapper
/// script is one (`sh -c 'app'`; `exec` makes it zero), a launcher shim
/// stacked on one two or three; `flatpak run`'s `bwrap` chain is believed to
/// fit too (unverified -- no Flatpak on the machines this was measured on).
/// A bound at all is what matters: every link is a filesystem round trip,
/// and a pid reused mid-walk must not turn it into a long one. Past it the
/// answer is "not a descendant", so a deeper chain only costs its window the
/// focus it would have taken, never grants anything.
pub(super) const MAX_ANCESTRY_DEPTH: usize = 8;

/// The most of `/proc/<pid>/stat` the walk reads: enough to reach the
/// parent pid through the longest `comm` field the kernel prints (64 bytes,
/// for a workqueue kernel thread; 15 for any other task) and a 7-digit pid,
/// with room to spare.
const STAT_HEAD: usize = 256;

/// Whether `pid` is `ancestor` or descends from it within
/// [`MAX_ANCESTRY_DEPTH`] parent links. `false` whenever the answer cannot
/// be read: a process gone mid-walk, a `stat` file that does not parse, a
/// pid namespace hiding the chain -- the walk fails closed.
pub(super) fn descends_from(pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    for _ in 0..MAX_ANCESTRY_DEPTH {
        if current == ancestor {
            return true;
        }
        match parent_of(current) {
            // Pid 0 is the parent of init (and of kernel threads): the top.
            Some(parent) if parent != 0 && parent != current => current = parent,
            _ => return false,
        }
    }
    current == ancestor
}

/// `pid`'s parent pid, from `/proc/<pid>/stat`, or `None` when it cannot be
/// read.
fn parent_of(pid: u32) -> Option<u32> {
    // "/proc/4294967295/stat" is 21 bytes.
    let mut path = [0u8; 32];
    let len = {
        let mut cursor = &mut path[..];
        write!(cursor, "/proc/{pid}/stat").ok()?;
        32 - cursor.len()
    };
    let mut file = File::open(Path::new(OsStr::from_bytes(&path[..len]))).ok()?;
    let mut head = [0u8; STAT_HEAD];
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    parse_ppid(&head[..filled])
}

/// The parent pid out of the head of a `/proc/<pid>/stat` line:
/// `pid (comm) state ppid …`.
///
/// `comm` is the process's own name, which it may set to anything
/// (`prctl(PR_SET_NAME)`) -- spaces and parentheses included, so a process
/// could name itself `x) S 1` to fake the fields after it. The fields are
/// therefore read after the *last* `)`, never the first: every field after
/// `comm` is a number or a state letter, so the last parenthesis on the line
/// is `comm`'s own. The parent pid must be followed by its separator inside
/// `stat`: a head cut short mid-number would otherwise parse as a different,
/// shorter pid.
pub(super) fn parse_ppid(stat: &[u8]) -> Option<u32> {
    let close = stat.iter().rposition(|&byte| byte == b')')?;
    let mut fields = stat[close + 1..].splitn(4, |&byte| byte == b' ');
    // The empty field between `)` and the state, then the state itself.
    if !fields.next()?.is_empty() || fields.next()?.len() != 1 {
        return None;
    }
    let ppid = fields.next()?;
    // Only a field with a separator after it is complete.
    fields.next()?;
    if ppid.is_empty() || !ppid.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(ppid).ok()?.parse().ok()
}
