//! The pictures the lock screen shows, one per output, in a directory a
//! locker reads.
//!
//! `$XDG_RUNTIME_DIR/scootbg-NAME.lockscreen/` (`NAME` as for the control
//! socket, [`crate::paths`]) holds one blurred PNG per output that
//! currently shows something ([`crate::lockblur`]) and an `index`:
//!
//! ```text
//! DP-1<TAB>/run/user/1000/scootbg-wayland-1.lockscreen/output-3.png
//! HDMI-A-1<TAB>/run/user/1000/scootbg-wayland-1.lockscreen/output-4.png
//! ```
//!
//! One line per output, the output's own name (the `wl_output` name a
//! locker matches), a tab, the file. A locker reads the index and passes
//! each line to `swaylock -i NAME:FILE`, so an output with no line shows
//! the locker's own background. Output names never reach a file name (they
//! are the compositor's text: a `/` or `..` in one must not leave the
//! directory), and an output whose name a locker could not take whole (a
//! `:`, a tab, a newline or other control byte, or nothing) is left out.
//!
//! **Atomic.** Every file is written beside its place and renamed over it,
//! so a locker that starts mid-update reads the old picture or the new,
//! never half of one, and an index that lists a file lists a complete one.
//!
//! **Kept current by the daemon, not asked for.** A picture is written when
//! the output's wallpaper choice changes and its pixels exist, an output
//! that goes away loses its line and file, and a `clear` does the same.
//! A start wipes whatever a daemon that died left, and a clean exit removes
//! the directory; a daemon killed by a signal leaves its last pictures,
//! which are still the last wallpapers it showed.
//!
//! This module only keeps the directory. Which picture an output gets is
//! `daemon::lockscreen`'s.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::color::Color;
use crate::outputs::OutputId;
use crate::print::warn;

#[cfg(test)]
mod tests;

const INDEX: &str = "index";

/// What a picture was made from: enough to know it is still current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Color(Color),
    /// The image of this request serial, rendered at this size.
    Image(u64, (u32, u32)),
}

#[derive(Debug)]
struct Entry {
    id: OutputId,
    name: String,
    source: Source,
    /// The file is on disk and the index lists it. False after a write
    /// failed: remembered so it is not retried on every turn of the loop.
    listed: bool,
}

#[derive(Debug, Default)]
pub struct Lockscreen {
    /// `None`: the directory could not be made, or this is a test double.
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    /// The index differs from what is on disk.
    dirty: bool,
}

/// Whether a locker can take `name` whole in `-i NAME:FILE` and a line of
/// the index: non-empty, no `:`, no control byte (tab and newline included).
fn listable(name: &str) -> bool {
    !name.is_empty() && !name.contains(':') && !name.chars().any(char::is_control)
}

impl Lockscreen {
    /// No directory: nothing is written (a test, or a daemon whose
    /// directory could not be made).
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Makes `dir` (private to the user) and empties it of whatever an
    /// earlier daemon left. Said once on stderr, and disabled, if it
    /// cannot: the wallpaper does not depend on it.
    pub fn open(dir: &Path) -> Self {
        match prepare(dir) {
            Ok(()) => Self {
                dir: Some(dir.to_path_buf()),
                entries: Vec::new(),
                dirty: false,
            },
            Err(error) => {
                warn(format_args!(
                    "scootbg: cannot keep the lock screen pictures in {}: {error}; the \
                     lock screen shows its own background",
                    dir.display()
                ));
                Self::disabled()
            }
        }
    }

    /// Whether `id` already has a picture made from `source` under `name`
    /// (or a failed attempt at one, not to be retried).
    pub fn has(&self, id: OutputId, name: &str, source: Source) -> bool {
        self.entries
            .iter()
            .any(|e| e.id == id && e.name == name && e.source == source)
    }

    /// Whether anything is written for `id`.
    #[cfg(test)]
    pub fn knows(&self, id: OutputId) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    /// Whether there is a directory to write to.
    pub fn enabled(&self) -> bool {
        self.dir.is_some()
    }

    /// Gives `id` (named `name`) the picture `png`, made from `source`.
    /// An output a locker could not name gets none (its old one goes).
    pub fn put(&mut self, id: OutputId, name: &str, source: Source, png: &[u8]) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        let file = dir.join(format!("output-{}.png", id.number()));
        let listed = listable(name)
            && match write_atomic(&file, png) {
                Ok(()) => true,
                Err(error) => {
                    warn(format_args!(
                        "scootbg: cannot write the lock screen picture for {name}: {error}; \
                         the lock screen shows its own background there"
                    ));
                    false
                }
            };
        if !listed {
            let _ = fs::remove_file(&file);
        }
        self.record(id, name, source, listed);
    }

    /// There is no picture of `source` to give `id` (it could not be made):
    /// the old one, of another wallpaper, goes, and the attempt is
    /// remembered so the next turn of the loop does not make it again.
    pub fn withhold(&mut self, id: OutputId, name: &str, source: Source) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        let _ = fs::remove_file(dir.join(format!("output-{}.png", id.number())));
        self.record(id, name, source, false);
    }

    fn record(&mut self, id: OutputId, name: &str, source: Source, listed: bool) {
        let entry = Entry {
            id,
            name: name.to_owned(),
            source,
            listed,
        };
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(slot) => *slot = entry,
            None => self.entries.push(entry),
        }
        self.dirty = true;
    }

    /// Takes `id`'s picture away.
    pub fn remove(&mut self, id: OutputId) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if let Some(at) = self.entries.iter().position(|e| e.id == id) {
            self.entries.swap_remove(at);
            let _ = fs::remove_file(dir.join(format!("output-{}.png", id.number())));
            self.dirty = true;
        }
    }

    /// Takes away the picture of every output `live` does not name.
    pub fn retain(&mut self, live: impl Fn(OutputId) -> bool) {
        // Not allocated: the ids to drop are found one at a time.
        while let Some(gone) = self.entries.iter().map(|e| e.id).find(|&id| !live(id)) {
            self.remove(gone);
        }
    }

    /// Writes the index if it changed.
    pub fn flush(&mut self) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut index = String::new();
        // Stable order: by output, so the index does not shuffle.
        let mut listed: Vec<&Entry> = self.entries.iter().filter(|e| e.listed).collect();
        listed.sort_by_key(|e| e.id.number());
        for entry in listed {
            index.push_str(&entry.name);
            index.push('\t');
            index.push_str(
                &dir.join(format!("output-{}.png", entry.id.number()))
                    .to_string_lossy(),
            );
            index.push('\n');
        }
        if let Err(error) = write_atomic(&dir.join(INDEX), index.as_bytes()) {
            warn(format_args!(
                "scootbg: cannot write the lock screen index: {error}; the lock screen \
                 shows its own background"
            ));
        }
    }

    /// Removes the directory (a clean exit): the pictures are of a
    /// wallpaper that is going away.
    pub fn close(&mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = fs::remove_dir_all(dir);
        }
        self.entries.clear();
    }
}

/// Makes `dir` private and empty.
fn prepare(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::DirBuilder::new().mode(0o700).create(dir)
}

/// Writes `bytes` to `path` through a sibling temporary file, renamed
/// over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let mut file: File = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
