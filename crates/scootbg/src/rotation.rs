//! A slideshow from a directory: `scootbg set DIR --every 30m [--shuffle]`
//! (docs/scootbg/backlog/config-and-rotation.md).
//!
//! The daemon lists the directory once, when the request arrives, and shows
//! its regular files one after another on a single timer: the directory is
//! never polled, so a file added later starts showing only after the next
//! `set` of the directory. No config file, no recursion, no watching; one
//! slideshow at a time (a new `set`, `clear` or changed `apply-config`
//! stops it).
//!
//! This module is the pure part (durations, listing order, shuffling), with
//! no Wayland objects, so each is a unit test. The daemon
//! (`daemon::rotation`) carries it out.

#[cfg(test)]
mod tests;

/// At least a minute, whole minutes: the timer then wakes the daemon at most
/// once a minute, and minute-aligned by construction.
pub const MIN_EVERY_SECS: u64 = 60;

/// At most a week: longer than any reasonable slideshow, and far from any
/// overflow in the timer math.
pub const MAX_EVERY_SECS: u64 = 7 * 24 * 3600;

/// At most this many files listed for one slideshow: the listing runs
/// synchronously on the daemon's loop thread (one `read_dir` plus one
/// `stat` per entry, then a sort), so an unbounded directory would stall
/// Wayland event dispatch for as long as it takes. Past the cap the `set`
/// is refused, naming the cap, rather than showing a silent subset: what
/// shows is exactly the directory, or nothing. 10,000 entries list in
/// ~14 ms (measured on the Asahi M2), cost ~100 B of file list each, and
/// no real wallpaper directory is that large: ten times the files would
/// stall the loop ten times as long, with no bound at all.
pub const MAX_LISTED: usize = 10_000;

/// Why an `--every` duration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EveryError {
    /// Not digits and one of `s`, `m`, `h` or `d`.
    BadFormat(String),
    /// Under a minute.
    TooShort,
    /// Not a whole number of minutes.
    NotAligned,
    /// Over a week.
    TooLong,
}

impl std::fmt::Display for EveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFormat(text) => write!(
                f,
                "bad duration {text:?}: a number and `s`, `m`, `h` or `d`, such as `30m`"
            ),
            Self::TooShort => write!(
                f,
                "the rotation is under a minute: at least `1m` (the timer wakes at most \
                 once a minute)"
            ),
            Self::NotAligned => write!(
                f,
                "the rotation is not a whole number of minutes: whole minutes only, such \
                 as `90s` never, `2m` instead"
            ),
            Self::TooLong => write!(f, "the rotation is over a week: at most `7d`"),
        }
    }
}

impl std::error::Error for EveryError {}

/// Parses an `--every` duration into seconds: digits and one unit (`s`,
/// `m`, `h`, `d`, lowercase), at least a minute, whole minutes, at most a
/// week. Refused otherwise, so a typo never becomes a hot timer.
pub fn parse_every(text: &str) -> Result<u64, EveryError> {
    let unit = text
        .chars()
        .last()
        .ok_or_else(|| EveryError::BadFormat(text.to_owned()))?;
    let digits = &text[..text.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EveryError::BadFormat(text.to_owned()));
    }
    let per: u64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return Err(EveryError::BadFormat(text.to_owned())),
    };
    let count: u64 = digits
        .parse()
        .map_err(|_| EveryError::BadFormat(text.to_owned()))?;
    let secs = count.checked_mul(per).ok_or(EveryError::TooLong)?;
    if secs > MAX_EVERY_SECS {
        return Err(EveryError::TooLong);
    }
    if secs < MIN_EVERY_SECS {
        return Err(EveryError::TooShort);
    }
    if secs % 60 != 0 {
        return Err(EveryError::NotAligned);
    }
    Ok(secs)
}

/// Why a directory could not be listed.
#[derive(Debug)]
pub enum ListError {
    /// Opening the directory failed (`read_dir`): it is gone, or was
    /// never one.
    Open(std::io::Error),
    /// Reading one entry, or stating it, failed (permissions, a symlink
    /// loop): the directory is there, the entry is not readable. Holds
    /// the entry's path (the directory itself when the entry could not
    /// even be read) and the operating system's reason.
    Entry {
        path: std::path::PathBuf,
        error: std::io::Error,
    },
    /// More than [`MAX_LISTED`] files: refused rather than truncated, so
    /// what shows is exactly the directory, or nothing. Holds how many
    /// files were seen (one past the cap).
    TooMany { seen: usize },
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(error) => write!(f, "cannot list the directory: {error}"),
            Self::Entry { path, error } => {
                write!(f, "cannot read the entry {path:?}: {error}")
            }
            Self::TooMany { seen } => write!(
                f,
                "the directory holds more than {MAX_LISTED} files ({seen} seen)"
            ),
        }
    }
}

impl std::error::Error for ListError {}

/// What listing found: the regular files' absolute paths, sorted by name,
/// and how many names were skipped for not being UTF-8 (the control
/// protocol cannot carry them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub files: Vec<String>,
    pub skipped_non_utf8: usize,
}

/// Lists `dir`'s regular files once (no recursion, links followed): their
/// absolute paths, sorted. A file that is not an image is still listed; it
/// fails to draw when its turn comes, like a `set` of it would, until the
/// next rotation. Anything else (a subdirectory, a socket, a name that is
/// not UTF-8) is left out. Past [`MAX_LISTED`] files the listing stops and
/// refuses, rather than growing the loop thread's stall and the list
/// without bound.
pub fn list_dir(dir: &std::path::Path) -> Result<Listed, ListError> {
    let entries = std::fs::read_dir(dir).map_err(ListError::Open)?;
    let mut files = Vec::new();
    let mut skipped_non_utf8 = 0;
    for entry in entries {
        let entry = entry.map_err(|error| ListError::Entry {
            path: dir.to_path_buf(),
            error,
        })?;
        // Following links (`std::fs::metadata`, not `DirEntry::metadata`,
        // which stats the link itself): a symlinked image is listed, and
        // an unreadable target (a loop, a dangling link, a denied
        // directory) is an entry error naming the entry, not a silent skip.
        let path = entry.path();
        let meta = std::fs::metadata(&path).map_err(|error| ListError::Entry { path, error })?;
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            skipped_non_utf8 += 1;
            continue;
        };
        files.push(dir.join(name).to_string_lossy().into_owned());
        if files.len() > MAX_LISTED {
            return Err(ListError::TooMany { seen: files.len() });
        }
    }
    files.sort();
    Ok(Listed {
        files,
        skipped_non_utf8,
    })
}

/// Shuffles `files` (already sorted) with `seed`: Fisher-Yates over an
/// xorshift64, so a fixed seed replays the same order in tests, and the
/// daemon seeds from the kernel. No allocation beyond the permutation.
pub fn shuffle_with_seed(files: &mut [String], seed: u64) {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..files.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        files.swap(i, j);
    }
}

/// Orders listed files for showing: sorted as listed, or shuffled once with
/// `seed` (`--shuffle`), then cycled in that order.
pub fn order(mut listed: Vec<String>, shuffle: bool, seed: u64) -> Vec<String> {
    if shuffle {
        shuffle_with_seed(&mut listed, seed);
    }
    listed
}
