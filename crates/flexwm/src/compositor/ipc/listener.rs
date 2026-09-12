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
//! - the socket file is `0600` before it is reachable under its published
//!   name, whatever the umask, so a weak directory mode alone is not enough to
//!   reach it;
//! - and a connection whose peer is not this compositor's own user is refused
//!   before it can send anything, so a weak *file* mode alone is not enough
//!   either -- which is also the only one of the two that stops root, since
//!   root ignores file permissions entirely.
//!
//! [`bind`] is written for that same hostile directory throughout: one where
//! another user may be creating, deleting and replacing names while it runs.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use rustix::rand::GetRandomFlags;

/// The mode the socket is given before anyone can reach it: owner read/write.
const SOCKET_MODE: u32 = 0o600;

/// The mode of the directory the socket is built in: owner only, so nobody
/// else can list it, never mind create anything inside it.
const STAGING_MODE: u32 = 0o700;

/// What the socket is called while it is in there.
const STAGED_NAME: &str = "s";

/// Marks a staging directory as flexwm's, for whoever finds one left behind by
/// a compositor that died mid-startup. Nothing removes those: telling a stale
/// one from a live one is not possible, and removing another process's is the
/// mistake this module is built around avoiding.
const STAGING_PREFIX: &str = ".flexwm-";

/// How many random bytes go in a staging directory's name, 36 choices each.
const STAGING_RANDOM: usize = 12;

/// How many staging names to try before giving up. Every one has to have been
/// taken already to get that far.
const STAGING_ATTEMPTS: usize = 16;

/// `sockaddr_un.sun_path` holds 108 bytes including its terminating NUL, so
/// 107 is the longest path a unix socket can be bound at. std rejects 108
/// itself, with "path must be shorter than SUN_LEN".
const SUN_PATH_MAX: usize = 107;

/// Creates the listening socket at `path`, replacing whatever was there.
///
/// The socket is built inside a `0700` directory this process creates beside
/// `path`, and [renamed](std::fs::rename) onto `path` once it is `0600` --
/// atomically, so a client only ever finds the previous socket or this one,
/// never a half-built one, and never a missing name another process could
/// claim in between.
///
/// Three rules hold it up, each of which replaced a version of this function
/// that was wrong in the hostile directory described in the module doc:
///
/// - **Nothing is ever removed to make room.** `mkdir(2)` neither follows a
///   symlink nor replaces an existing name: it fails `EEXIST`. That makes it
///   the claim -- either this process created the directory, or it tries
///   another name. A `remove_file`/`remove_dir` *before* creating, at a name
///   another user may have replaced with a symlink, deletes a file of their
///   choosing through it; two earlier versions of this did exactly that.
/// - **Everything after the claim goes through a pinned directory fd**, opened
///   `O_DIRECTORY | O_NOFOLLOW` so anything but a real directory at that name
///   fails (`ENOTDIR`, verified) rather than being worked on by proxy. Naming
///   the socket under `/proc/self/fd/<n>` means no component of its path can
///   be swapped after that check -- and, just as usefully, that the staged
///   name's length owes nothing to the published path's (see [`staged_path`]).
/// - **The mode is set before the socket is reachable.** `set_permissions`
///   follows symlinks, so chmodding a socket that sits in a directory another
///   user can write to lets them swap it for a link and have the compositor
///   chmod a file of their choosing. Inside a `0700` directory this process
///   owns, reached through a pinned fd, there is nobody who could.
pub(super) fn bind(path: &Path) -> io::Result<UnixListener> {
    // Checked up front because the bind no longer happens at this path: the
    // staged name is short and unrelated, so a path too long for `sun_path`
    // would otherwise bind and rename perfectly happily and leave a socket no
    // client could ever connect to. Failing at startup is both what happened
    // before any staging existed and the only useful answer.
    if path.as_os_str().len() > SUN_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the socket path is {} bytes; a unix socket path can be at most {SUN_PATH_MAX}",
                path.as_os_str().len()
            ),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the socket path names no directory to create it in",
        )
    })?;

    let mut attempts = STAGING_ATTEMPTS;
    loop {
        let staging = parent.join(staging_name()?);
        let claimed = DirBuilder::new().mode(STAGING_MODE).create(&staging);
        attempts -= 1;
        match claimed {
            Ok(()) => return publish(&staging, path),
            // Taken -- by a file, a directory, a symlink, anything at all.
            // Another name, never a removal.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts > 0 => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Builds the socket inside `staging` and publishes it at `path`, then removes
/// `staging` again whether that worked or not.
///
/// `staging` is a directory [`bind`] has just created -- but that precondition
/// is not what makes this safe: [`stage`]'s `O_NOFOLLOW` open rechecks it, so a
/// `staging` that turns out to be a symlink, a file, or somebody else's
/// directory is refused rather than used.
pub(super) fn publish(staging: &Path, path: &Path) -> io::Result<UnixListener> {
    let result = stage(staging, path);
    // `rmdir` neither follows a symlink nor removes a non-empty directory, so
    // this can remove the directory created above and very little else. It is
    // also allowed to fail: if something replaced that name, whatever is there
    // now is not flexwm's to clean up.
    let _ = std::fs::remove_dir(staging);
    result
}

/// Binds the socket inside `staging`, gives it [`SOCKET_MODE`] and renames it
/// onto `path`.
fn stage(staging: &Path, path: &Path) -> io::Result<UnixListener> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(staging)?;
    // `fchmod` through the fd rather than a chmod through the path: `mkdir`
    // only ever gets `STAGING_MODE & !umask`, which can only be *tighter* than
    // asked for -- never more open -- but tighter includes unusable (a
    // directory with no owner `x` cannot be written in at all). This pins the
    // mode either way, without naming a path again.
    directory.set_permissions(Permissions::from_mode(STAGING_MODE))?;

    // `staged` borrows `directory`'s fd number, not `directory` itself --
    // correct only because `directory` stays alive for every use of `staged`
    // below. A future refactor that drops `directory` early before `staged`
    // is done with would resolve `/proc/self/fd/<n>` against whatever that
    // fd number has since become (fds are reused), silently operating on the
    // wrong file. Keep `directory` in scope through the end of this function.
    let staged = staged_path(&directory);
    let listener = UnixListener::bind(&staged)?;
    let published = (|| {
        std::fs::set_permissions(&staged, Permissions::from_mode(SOCKET_MODE))?;
        listener.set_nonblocking(true)?;
        std::fs::rename(&staged, path)
    })();
    if let Err(error) = published {
        // Inside the pinned directory, so this is the socket bound just above
        // and cannot have become anything else.
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(listener)
}

/// Where the socket lives while it is being built: inside the directory
/// `directory` holds open, named through that fd rather than by its own path.
///
/// `/proc/self/fd/<n>` resolves to the directory the fd is open on, so this
/// reaches the same place the directory's own path does without re-resolving
/// any of that path's components -- nothing along it can be swapped between
/// [`stage`]'s `O_NOFOLLOW` check and the bind.
///
/// It is also short, and the same length whatever `path` is, which is the
/// other half of why it is used: `sun_path` holds 107 bytes, and a staged name
/// derived from the published path spends some of them on top of it. An earlier
/// version of this function spent 17, which stopped a 93-byte path that binds
/// fine on its own from binding at all.
///
/// This does make a mounted `/proc` a startup requirement, which nothing else
/// in `--headless` startup needs (udev and libinput want `/sys` and `/dev`).
/// Without one, [`bind`] fails with a bare "No such file or directory" (the
/// kernel's `ENOENT` resolving this path, with no path named in the message
/// itself), and the compositor refuses to start rather than starting with no
/// control socket.
/// Every environment flexwm targets mounts `/proc`, webtop containers
/// included, so that is a statement of the dependency rather than a caveat
/// about it.
fn staged_path(directory: &File) -> PathBuf {
    PathBuf::from(format!(
        "/proc/self/fd/{}/{STAGED_NAME}",
        directory.as_raw_fd()
    ))
}

/// A name for the staging directory: [`STAGING_PREFIX`] plus
/// [`STAGING_RANDOM`] lowercase-alphanumeric bytes from the kernel's random
/// pool.
///
/// Unpredictable rather than merely unique (a pid is neither): a name another
/// user can work out in advance is a name they can occupy before flexwm
/// starts, over and over, which turns into a startup failure of their
/// choosing. Nothing here *depends* on the name being secret -- [`bind`]
/// treats a taken name as a taken name and moves on -- so the slight modulo
/// bias below costs nothing; the name only has to be impractical to guess.
pub(super) fn staging_name() -> io::Result<OsString> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

    let mut bytes = [0u8; STAGING_RANDOM];
    let mut filled = 0;
    while filled < bytes.len() {
        // Short only if a signal interrupts it, which is not an error here.
        filled += rustix::rand::getrandom(&mut bytes[filled..], GetRandomFlags::empty())?;
    }
    let mut name = String::with_capacity(STAGING_PREFIX.len() + bytes.len());
    name.push_str(STAGING_PREFIX);
    name.extend(
        bytes
            .iter()
            .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()])),
    );
    Ok(OsString::from(name))
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
