//! Writing the state file off the Wayland loop.
//!
//! A save is a small file written atomically: a temporary file in the same
//! directory, `fsync`, `rename` over the old one, `fsync` of the
//! directory. On a disk that is busy (or a network home) an `fsync` can
//! take far longer than the loop should ever stall, so the write happens
//! on a thread started for it, which ends once nothing more is waiting,
//! the way the decode worker does (`daemon::worker`): an idle daemon keeps
//! one thread.
//!
//! **Latest wins, nothing queues.** The loop hands over the whole file's
//! text; while a write is under way, a newer text replaces any that waits,
//! and the thread writes it next. A burst of `set`s is at most two
//! writes, and the file on disk ends as the last one.
//!
//! **On the way out** the daemon waits, bounded, for a write under way
//! ([`Saver::flush`]), so `scootbg kill` straight after a `set` keeps it.
//! A signal kills the process where it stands: the rename is atomic, so
//! the file is the old one or the new one, never half of each, and at
//! worst a temporary file is left behind (named for the one write, so it
//! is never another's: [`temp_path`]).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::print::warn;

#[cfg(test)]
mod tests;

/// What the thread shares with the loop.
#[derive(Debug, Default)]
struct Slot {
    /// The newest text not yet written.
    pending: Option<String>,
    /// A thread is writing, or about to.
    running: bool,
}

#[derive(Debug)]
pub struct Saver {
    file: PathBuf,
    shared: Arc<(Mutex<Slot>, Condvar)>,
}

impl Saver {
    /// Saves go to `file`; its directory is made (0700) at the first save.
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            shared: Arc::new((Mutex::new(Slot::default()), Condvar::new())),
        }
    }

    /// Writes `text` as the file, in the background: replaces any text
    /// still waiting, and starts a thread unless one is writing. If no
    /// thread can be started, writes here, on the caller's thread.
    pub fn save(&self, text: String) {
        let mut slot = lock(&self.shared.0);
        slot.pending = Some(text);
        if slot.running {
            return;
        }
        slot.running = true;
        drop(slot);
        let shared = Arc::clone(&self.shared);
        let file = self.file.clone();
        let started = std::thread::Builder::new()
            .name("scootbg-save".into())
            .spawn(move || drain(&file, &shared));
        if let Err(error) = started {
            warn(format_args!(
                "scootbg: cannot start a thread to save the wallpaper ({error}); saving it now"
            ));
            drain(&self.file, &self.shared);
        }
    }

    /// Waits until everything handed over is written, for at most `limit`.
    /// Returns whether it all was.
    pub fn flush(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let (mutex, done) = &*self.shared;
        let mut slot = lock(mutex);
        while slot.running {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            slot = done
                .wait_timeout(slot, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// The lock, whether or not a writer panicked holding it (it never holds
/// it while writing; and the release profile aborts on panic anyway).
fn lock(mutex: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes whatever waits, until nothing does; then says it is done.
fn drain(file: &Path, shared: &(Mutex<Slot>, Condvar)) {
    let (mutex, done) = shared;
    loop {
        let text = {
            let mut slot = lock(mutex);
            match slot.pending.take() {
                Some(text) => text,
                None => {
                    slot.running = false;
                    done.notify_all();
                    return;
                }
            }
        };
        if let Err(error) = write_atomic(file, text.as_bytes()) {
            warn(format_args!(
                "scootbg: cannot save the wallpaper to {} (written through a temporary \
                 file beside it, then renamed): {error}",
                file.display()
            ));
        }
    }
}

/// Why a state directory that already exists is not private to this
/// user, if it is not: owned by someone else, or writable by the group or
/// others. Anyone who can write there can replace the state file (which
/// only picks a wallpaper: its paths are opened read-only, and decoded by
/// code fuzzed for hostile input), so this is a warning, not a refusal: a
/// group-writable directory is the norm under a `umask` of 002 with a group
/// per user, and refusing would lose the user's wallpaper for no safety
/// gained there. One `stat`, at start-up.
pub fn exposed(dir: &Path) -> Option<String> {
    let stat = rustix::fs::stat(dir).ok()?;
    let uid = rustix::process::getuid().as_raw();
    if stat.st_uid != uid {
        return Some(format!("it is owned by uid {}, not {uid}", stat.st_uid));
    }
    let mode = stat.st_mode & 0o777;
    let who = match (mode & 0o020 != 0, mode & 0o002 != 0) {
        (true, true) => "its group and others",
        (true, false) => "its group",
        (false, true) => "others",
        (false, false) => return None,
    };
    Some(format!("its mode is {mode:03o}: {who} can write to it"))
}

/// Replaces `file` with `bytes`, atomically: a reader (or a crash) sees
/// the old file or the new one, whole. The file is private (0600), its
/// directory made private (0700) if it has to be made.
pub fn write_atomic(file: &Path, bytes: &[u8]) -> io::Result<()> {
    write_through(file, bytes, unique())
}

/// The temporary file [`write_atomic`] writes `file`'s next contents to,
/// told apart from every other write's by `unique`. Hidden, so never a
/// profile's name (those cannot start with a dot), and never another
/// write's: the pid alone is not enough, since two daemons of the same
/// pid can share a profile (in two pid namespaces, or on two hosts with
/// one network home), and one's remove-first below would take the other's
/// file from under it, which would then rename this one's half-written
/// file into place. The pid is there for whoever finds one left behind.
/// It comes through `rustix`'s raw syscall rather than `std::process::id`,
/// which calls libc's `getpid`: that one call alone kept another 64 KiB of
/// libc's code resident in the idle daemon (the kernel maps a fault's
/// neighbouring pages with it: docs/scootbg/backlog/idle-code-pages.md).
fn temp_path(file: &Path, unique: u64) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let pid = rustix::process::getpid().as_raw_nonzero();
    dir.join(format!(".{name}.{pid}.{unique:016x}.tmp"))
}

/// 64 bits for [`temp_path`] that no other write draws: random from the
/// kernel (`getrandom(2)`, a raw syscall through `rustix`), which waits
/// only while the kernel's pool is not yet initialized, early in boot,
/// long before a wallpaper is saved. Where the kernel refuses it (an old
/// kernel's `ENOSYS`, or a container's seccomp profile answering `EPERM`,
/// as webtop-style sandboxes can), [`fallback_unique`] stands in: the name
/// only needs to be unique, not secret (a guessed name is already handled
/// by remove-first and `O_EXCL` below), so saving never stops for it.
fn unique() -> u64 {
    let mut bytes = [0u8; 8];
    let mut filled = 0;
    while filled < bytes.len() {
        match rustix::rand::getrandom(&mut bytes[filled..], rustix::rand::GetRandomFlags::empty()) {
            Ok(read) => filled += read,
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return fallback_unique(),
        }
    }
    u64::from_ne_bytes(bytes)
}

/// Unique without the kernel's randomness: the wall clock's nanoseconds,
/// mixed with a per-process count so two writes in one nanosecond differ.
/// Two daemons of one pid would have to write in the same nanosecond to
/// meet, and even then only this write fails (`O_EXCL`), loudly.
fn fallback_unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    // A fixed odd multiplier spreads the count over the high bits, so the
    // sum is not just the clock with a small offset.
    nanos ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// [`write_atomic`] through the temporary file `unique` names.
fn write_through(file: &Path, bytes: &[u8], unique: u64) -> io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("/"));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let temp = temp_path(file, unique);
    // Whatever is at this write's own name (in practice nothing: a
    // symbolic link someone guessed it with) goes first; then `create_new`
    // (`O_CREAT | O_EXCL`) makes a new file or fails. `O_EXCL` never
    // follows a symbolic link, dangling or not, so the write can only land
    // in a fresh 0600 file of ours, never in a file a link points at.
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let written = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&temp, file)
    })();
    if let Err(error) = written {
        // Only a file this call made: after a failed `create_new` the name
        // may be someone else's again.
        if error.kind() != io::ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&temp);
        }
        return Err(error);
    }
    // The rename itself on disk. Best effort: some file systems refuse
    // to sync a directory, and the file is in place either way.
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}
