//! Where the control socket comes from, and who is allowed to speak on it.
//!
//! flexwm's IPC is a more powerful channel than most compositors': alongside
//! reading state it injects arbitrary keystrokes and pointer input, and hands
//! back screenshots. Reaching it is still gated the same way sway, i3, niri
//! and Hyprland gate theirs -- `$XDG_RUNTIME_DIR` is a `0700` per-user
//! directory, so nobody else can open the path in the first place -- but
//! `$FLEXWM_SOCKET` can point the socket at a directory anyone can reach, and
//! an unusual umask can leave the socket itself group- or world-writable
//! (`umask 000` really does publish `srwxrwxrwx`, and another user really can
//! then inject keystrokes through it). Those are the cases the two checks here
//! cover, each of which holds on its own:
//!
//! - the socket file is `0600` whatever the umask, so a weak directory mode
//!   alone is not enough to reach it;
//! - and a connection whose peer is not this compositor's own user is refused
//!   before it can send anything, so a weak *file* mode alone is not enough
//!   either -- which is also the only one of the two that stops root, since
//!   root ignores file permissions entirely.

use std::ffi::OsString;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The mode the socket file is given: owner read/write, nothing else.
const SOCKET_MODE: u32 = 0o600;

/// Creates the listening socket at `path`, replacing whatever was there.
///
/// The socket is bound at a temporary name in the same directory, made
/// `0600`, and only then [renamed](std::fs::rename) into place. That closes
/// two windows the `remove_file`-then-`bind` this replaces had:
///
/// - the socket existed at its published name, with whatever the umask
///   allowed, for as long as it took to `chmod` it -- long enough for another
///   process to connect and be served;
/// - and the published name was *missing* in between, so a process racing for
///   the same path could claim it and leave this one failing `EADDRINUSE`.
///
/// Not, despite how it looks, a symlink race in either form: `bind(2)` does
/// not follow a symlink at the final component (a dangling one at the path
/// makes it fail `EADDRINUSE` -- verified, errno 98 -- rather than creating
/// the socket at the target), and `remove_file` unlinks the link rather than
/// what it points at. That is also why the `remove_file` on the *temporary*
/// path below is safe: the worst anything planted there can do is make this
/// call fail, never make it write somewhere it was not asked to.
pub(super) fn bind(path: &Path) -> io::Result<UnixListener> {
    let temporary = temporary_path(path);
    let _ = std::fs::remove_file(&temporary);
    let listener = UnixListener::bind(&temporary)?;

    // Every step that can fail after the bind, so one cleanup covers them
    // all: a temporary socket left behind would be an invisible file nothing
    // ever removes.
    let published = (|| {
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        listener.set_nonblocking(true)?;
        std::fs::rename(&temporary, path)
    })();
    if let Err(error) = published {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(listener)
}

/// The name to bind at before publishing, always in `path`'s own directory
/// (`rename` cannot cross filesystems, and the socket has to end up where the
/// caller asked).
///
/// The pid keeps two flexwm instances racing for the same socket path from
/// clobbering each other's half-built one.
pub(super) fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = OsString::from(path);
    temporary.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(temporary)
}

/// The uid a connecting client has to match to be served.
///
/// Effective, not real: Linux fills `SO_PEERCRED` from the peer's *effective*
/// uid at connect time (`cred_to_ucred()` uses `cred->euid`), so comparing it
/// against `getuid()` would compare two different things in the one case
/// where the two differ.
pub(super) fn own_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// The effective uid of the process at the other end of `stream`, as the
/// kernel recorded it when that process connected -- not something the peer
/// can choose or forge.
///
/// `SO_PEERCRED` by hand, for want of anything safe that asks for just this:
///
/// - [`UnixStream::peer_cred`] is still unstable (rust#42839), so it cannot
///   be used at all.
/// - rustix is already a dependency here and has `socket_peercred`, but it
///   reads the kernel's `struct ucred` straight into a `rustix::net::UCred`
///   whose `pid` field is a `NonZeroI32` -- and the kernel writes a pid of
///   *zero* for a peer whose PID namespace this process cannot see it in
///   (`pid_vnr` inside `cred_to_ucred`). That is a niche-invalid value the
///   moment the struct is assumed initialized, which declining to read the
///   field does not avoid. `geteuid` above has no such hazard, so it still
///   goes through rustix's safe wrapper.
///
/// flexwm needs the uid and nothing else, so this asks for the credentials
/// directly and reads only that.
pub(super) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        // The same `-1` the kernel itself uses for "no credentials to
        // report", so a struct the kernel somehow left unwritten can never
        // read back as a valid uid -- including as uid 0, which would
        // otherwise be accepted by a compositor running as root.
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `stream` keeps the fd open for the whole call; `credentials`
    // is a live, initialized `struct ucred` of exactly the size `length`
    // claims, which is all the kernel writes through that pointer. Nothing
    // borrowed here outlives the call.
    let queried = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::from_mut(&mut credentials).cast::<libc::c_void>(),
            &mut length,
        )
    };
    if queried != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

/// Whether a client connecting as `peer` may drive this compositor.
///
/// An exact match, deliberately: root is refused along with everyone else.
/// Root can already read this process's memory or its socket by other means,
/// so allowing it would buy nothing, and "the same user, or nobody" is a rule
/// that stays easy to reason about.
pub(super) fn peer_is_allowed(peer: u32, own: u32) -> bool {
    peer == own
}
