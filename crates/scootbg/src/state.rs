//! The last wallpaper, kept across daemon restarts.
//!
//! **Where.** One file per profile, `$XDG_STATE_HOME/scootbg/PROFILE`
//! (`~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset, empty
//! or relative, as the XDG base directory spec says). A profile, not a
//! display, names it: scoot binds the first free `wayland-N`, so the
//! socket name shifts with start order and is no session identity
//! (restore-state-done.md). `scootbg daemon --profile NAME` picks one; the
//! default is `default`.
//!
//! **What.** Exactly what the daemon's choices were, as `set` and `clear`
//! made them: the choice for every output and each choice for one output
//! by connector name ([`crate::choices`]), connected now or not. The file
//! mirrors a table of its own ([`Saved::choices`]) rather than what the
//! outputs show, so that
//!
//! - a restore does not count as a user's `set`: it writes nothing;
//! - an entry for an output that is not plugged in now, or for an image
//!   that could not be restored (a moved file, a disk not mounted yet),
//!   stays in the file across a restore and across a later `set` for other
//!   outputs, until a `set` or `clear` replaces it (a `set` without
//!   `--output` replaces every named entry, as it does on screen);
//! - with `--no-restore` the file is still read, so a `set` then updates it
//!   rather than dropping the rest.
//!
//! **When.** After each `set` or `clear` the daemon records: a color or a
//! `clear` at once; an image once it has decoded (one that cannot be shown
//! changes nothing, so saves nothing). Written off the loop
//! ([`saver`]); the format is [`format`]'s.

pub mod format;
pub mod saver;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::choices::{Choice, Choices};
use crate::print::warn;
use format::Parsed;
use saver::Saver;

/// The profile a daemon uses when told none.
pub const DEFAULT_PROFILE: &str = "default";

/// The longest profile name, in bytes.
pub const MAX_PROFILE: usize = 64;

/// A profile's name: the state file's name, so checked to be one plain
/// file name and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile(String);

/// Why a profile name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Empty,
    TooLong,
    /// A byte outside `A-Z a-z 0-9 . _ -` (a `/` among them).
    Byte(char),
    /// Starts with `.`, or has `..`: `.` and `..` are directories, and a
    /// leading dot would hide the file.
    Dots,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let takes = "a profile name is 1 to 64 of A-Z, a-z, 0-9, '.', '_' and '-', \
                     not starting with '.' and without '..'";
        match self {
            Self::Empty => write!(f, "the profile name is empty ({takes})"),
            Self::TooLong => write!(f, "the profile name is too long ({takes})"),
            Self::Byte(ch) => write!(f, "the profile name has {ch:?} ({takes})"),
            Self::Dots => write!(f, "the profile name has a leading '.' or '..' ({takes})"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    pub fn parse(name: &str) -> Result<Self, ProfileError> {
        if name.is_empty() {
            return Err(ProfileError::Empty);
        }
        if let Some(bad) = name
            .chars()
            .find(|&ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
        {
            return Err(ProfileError::Byte(bad));
        }
        if name.len() > MAX_PROFILE {
            return Err(ProfileError::TooLong);
        }
        if name.starts_with('.') || name.contains("..") {
            return Err(ProfileError::Dots);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self(DEFAULT_PROFILE.to_owned())
    }
}

/// The directory state files live in, for a given `XDG_STATE_HOME` and
/// `HOME`: `None` when neither gives an absolute path.
pub fn dir(state_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let absolute = |value: Option<&OsStr>| {
        value
            .map(Path::new)
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
    };
    absolute(state_home)
        .or_else(|| absolute(home).map(|home| home.join(".local/state")))
        .map(|base| base.join("scootbg"))
}

/// [`dir`] for this process's environment.
pub fn dir_from_env() -> Option<PathBuf> {
    dir(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Reads the state file at `file`. `Ok(None)` when there is none.
pub fn load(file: &Path) -> io::Result<Option<Parsed>> {
    // Not something that would block the open (a FIFO) or read forever.
    let meta = match std::fs::metadata(file) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !meta.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    // One byte past the limit is enough to know it is over.
    std::fs::File::open(file)?
        .take(format::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(Some(format::decode(&bytes)))
}

/// What is saved, and where to.
#[derive(Debug)]
pub struct Saved {
    profile: Profile,
    /// Recorded choices, as the file has them.
    pub choices: Choices,
    /// The fingerprint of the last `[wallpaper]` section applied from
    /// scoot's config (`crate::section`): kept as read, and changed only by
    /// `apply-config` ([`Saved::applied_config`]).
    fingerprint: Option<String>,
    /// Requests older than this generation are not this profile's: it was
    /// adopted after them (`daemon::config`), so an image `set` sent
    /// before, landing after, is not saved here. 0 for a daemon's first
    /// profile.
    floor: u64,
    /// `None`: nowhere to save (no state directory, or a newer
    /// scootbg's file that must not be written over).
    saver: Option<Saver>,
}

impl Saved {
    /// Saving to `file` (`None`: not saving), with `fingerprint` kept from
    /// the file read; the choices start empty.
    pub fn new(profile: Profile, file: Option<PathBuf>, fingerprint: Option<String>) -> Self {
        Self {
            profile,
            choices: Choices::default(),
            fingerprint,
            floor: 0,
            saver: file.map(Saver::new),
        }
    }

    /// The profile saved to.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The fingerprint of the last section `apply-config` applied, as read
    /// or as set since.
    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// Adopted at `generation`: nothing older is recorded here.
    pub fn adopted_at(&mut self, generation: u64) {
        self.floor = generation;
    }

    /// `apply-config` applied the section with `fingerprint`, its choices
    /// already put in [`Saved::choices`]: the fingerprint is recorded and
    /// the file written, once for the whole section.
    pub fn applied_config(&mut self, fingerprint: String) {
        self.fingerprint = Some(fingerprint);
        self.save();
    }

    /// Nothing saved, nowhere to save it: for tests.
    #[cfg(test)]
    pub fn nowhere() -> Self {
        Self::new(Profile::default(), None, None)
    }

    /// A `set` or `clear` of `generation` was recorded as the daemon's
    /// choice: recorded here too (by the same rules, so newer choices are
    /// kept), and the file written in the background.
    pub fn record(&mut self, output: Option<&str>, choice: &Choice, generation: u64) {
        if generation < self.floor || !self.choices.set(output, choice.clone(), generation) {
            return;
        }
        self.save();
    }

    /// Writes the file for what is recorded now, in the background.
    fn save(&mut self) {
        if self.saver.is_none() {
            return;
        }
        let text = self.text();
        if let Some(saver) = &self.saver {
            saver.save(text);
        }
    }

    /// The file's text for what is recorded now. What the file has no room
    /// for (`format::encode`: the least recently set per-output choices
    /// beyond its limits) is forgotten here too, with a warning, so the
    /// table and the file agree.
    pub fn text(&mut self) -> String {
        let mut text = String::with_capacity(256);
        let mut named: Vec<(&str, &Choice, u64)> = self.choices.named().collect();
        named.sort_by_key(|&(.., made)| made);
        let oldest_first: Vec<(&str, &Choice)> = named
            .iter()
            .map(|&(name, choice, _)| (name, choice))
            .collect();
        let left = format::encode(
            &mut text,
            self.profile.as_str(),
            self.fingerprint.as_deref(),
            self.choices.every(),
            &oldest_first,
        );
        let dropped: Vec<String> = oldest_first
            .iter()
            .take(left.named)
            .map(|&(name, _)| name.to_owned())
            .collect();
        if left.all {
            warn(format_args!(
                "scootbg: the choice for every output is too long for the state file \
                 ({} bytes at most) and is not saved",
                format::MAX_BYTES
            ));
        }
        if !dropped.is_empty() {
            warn(format_args!(
                "scootbg: the state file keeps at most {} per-output choices ({} bytes in \
                 all); the {} least recently set are no longer saved",
                format::MAX_OUTPUTS,
                format::MAX_BYTES,
                dropped.len()
            ));
            for name in &dropped {
                self.choices.forget(name);
            }
        }
        text
    }

    /// Whether changes are saved: `false` when there is no state
    /// directory, or the file could not be read or is a newer scootbg's
    /// (said on stderr at start-up, with how to recover). `query` reports
    /// it.
    pub fn saving(&self) -> bool {
        self.saver.is_some()
    }

    /// Waits, at most `limit`, for a write under way (on the way out).
    pub fn flush(&self, limit: Duration) -> bool {
        self.saver.as_ref().is_none_or(|saver| saver.flush(limit))
    }
}
