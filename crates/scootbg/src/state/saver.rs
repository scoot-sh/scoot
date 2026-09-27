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
//! worst a temporary file is left behind (named for the process, so it is
//! never another daemon's).

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
                "scootbg: cannot save the wallpaper to {}: {error}",
                file.display()
            ));
        }
    }
}

/// Replaces `file` with `bytes`, atomically: a reader (or a crash) sees
/// the old file or the new one, whole. The file is private (0600), its
/// directory made private (0700) if it has to be made.
pub fn write_atomic(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // Hidden, and named for this process: never another daemon's (two
    // sessions may share a profile), and never a profile's name (those
    // cannot start with a dot).
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&temp, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
        return written;
    }
    // The rename itself on disk. Best effort: some file systems refuse
    // to sync a directory, and the file is in place either way.
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}
