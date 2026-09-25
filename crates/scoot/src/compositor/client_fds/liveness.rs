//! Whether a recorded fd number still holds the fd that was recorded on it.
//! See the parent module's "How a record is found dead".

use std::mem::MaybeUninit;
use std::os::fd::{BorrowedFd, RawFd};

use super::Kind;

/// How a sweep tells whether the fd on a recorded number is still the one
/// that arrived there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// A pool or plane fd: still the same file, by `fstat` identity.
    Same { dev: u64, ino: u64 },
    /// A timeline fd: still open, and still a syncobj. Syncobj files share
    /// the kernel's one anonymous inode (with eventfds and most other
    /// anonymous files), so identity cannot tell two apart; the link name
    /// can at least tell a syncobj from everything else.
    Syncobj,
    /// A pool or plane fd whose identity could not be read on arrival: still
    /// open. Only an `fstat` failure on a just-received fd leads here, which
    /// nothing is known to cause. It over-counts a reused number, the
    /// conservative direction, rather than not counting the fd at all.
    Open,
}

/// The check a sweep will use for `fd`, arriving now as `kind`: its identity
/// for a pool or a plane (one `fstat`), the syncobj link check for a
/// timeline (no syscall now).
// `st_dev`/`st_ino` are already `u64` on the 64-bit targets scoot builds
// for, but not on every Linux target; `u64::from` is the lossless spelling
// for all of them.
#[allow(clippy::useless_conversion)]
pub(crate) fn capture(fd: BorrowedFd<'_>, kind: Kind) -> Check {
    match kind {
        Kind::Timeline => Check::Syncobj,
        Kind::Pool | Kind::Plane => match rustix::fs::fstat(fd) {
            Ok(stat) => Check::Same {
                dev: u64::from(stat.st_dev),
                ino: u64::from(stat.st_ino),
            },
            Err(_) => Check::Open,
        },
    }
}

/// Whether fd number `fd` still holds the fd `check` describes. One or two
/// syscalls and no allocation.
// See `capture` for the conversions.
#[allow(clippy::useless_conversion)]
pub(crate) fn still_held(fd: RawFd, check: Check) -> bool {
    match check {
        Check::Same { dev, ino } => with_borrowed(fd, |fd| {
            rustix::fs::fstat(fd)
                .is_ok_and(|stat| u64::from(stat.st_dev) == dev && u64::from(stat.st_ino) == ino)
        }),
        Check::Syncobj => timeline_fd_open(fd),
        Check::Open => with_borrowed(fd, |fd| rustix::io::fcntl_getfd(fd).is_ok()),
    }
}

/// Runs `f` on number `fd` borrowed, or answers `false` for a negative one.
fn with_borrowed(fd: RawFd, f: impl FnOnce(BorrowedFd<'_>) -> bool) -> bool {
    if fd < 0 {
        return false;
    }
    // SAFETY: the borrow lives only for `f`, whose callers here only
    // `fstat`/`fcntl(F_GETFD)` it, which never close or otherwise change the
    // fd. A number that is not open simply fails with `EBADF`. Nothing is
    // read or written through it.
    f(unsafe { BorrowedFd::borrow_raw(fd) })
}

/// What `readlink(/proc/self/fd/N)` reads for a DRM syncobj fd. It is the
/// name `drm_syncobj.c` gives `anon_inode_getfile`. Verified on the dev VM
/// (kernel 6.18): an exported timeline syncobj reads exactly this, an eventfd
/// reads `anon_inode:[eventfd]`, and a memfd reads `/memfd:<name> (deleted)`.
const SYNCOBJ_LINK: &[u8] = b"anon_inode:syncobj_file";

/// Whether fd number `fd` is still open in this process and is still a
/// syncobj, and so presumably the timeline fd recorded on it.
///
/// Two syscalls and no allocation (both paths and the link are in stack
/// buffers). `F_GETFD` answers "open?" definitively, and does not need
/// `/proc`. The type check does need it: when the `readlink` fails (no
/// `/proc`, or any other error), an open fd counts as held. That is a
/// deliberate, conservative over-count -- the number may have been reused
/// by something that is not a syncobj -- and it can only raise the count of
/// the client the record names, never lower anyone's.
pub(crate) fn timeline_fd_open(fd: RawFd) -> bool {
    if !with_borrowed(fd, |fd| rustix::io::fcntl_getfd(fd).is_ok()) {
        return false;
    }
    let mut path = [0u8; 32];
    let Some(path) = proc_fd_path(fd, &mut path) else {
        return true;
    };
    let mut link = [MaybeUninit::<u8>::uninit(); 64];
    match rustix::fs::readlinkat_raw(rustix::fs::CWD, path, &mut link) {
        Ok((read, _)) => &*read == SYNCOBJ_LINK,
        // Unknown kind: count it (the conservative over-count above).
        Err(_) => true,
    }
}

/// `/proc/self/fd/<fd>` as a C string in `buf`, or `None` if it does not fit
/// (it always does: 14 bytes of prefix, at most 10 digits and the nul).
pub(crate) fn proc_fd_path(fd: RawFd, buf: &mut [u8; 32]) -> Option<&std::ffi::CStr> {
    const PREFIX: &[u8] = b"/proc/self/fd/";
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut digits = [0u8; 10];
    let mut value = u32::try_from(fd).ok()?;
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let end = PREFIX.len() + count;
    for (slot, digit) in buf[PREFIX.len()..end]
        .iter_mut()
        .zip(digits[..count].iter().rev())
    {
        *slot = *digit;
    }
    buf[end] = 0;
    std::ffi::CStr::from_bytes_with_nul(&buf[..=end]).ok()
}
