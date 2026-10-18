//! Frecency: how often and how recently each entry launched.
//!
//! `$XDG_STATE_HOME/scootlaunch/history` holds text lines of
//! `id<TAB>count<TAB>last-epoch`, at most 512 entries and 32 KiB,
//! rewritten atomically (tmp file, then `rename`) after a launch.
//! Malformed lines are skipped. An unreadable, corrupt or hostile
//! file is never an error and never a crash: loading answers "no
//! history". Concurrent writers each rename their own pid-unique tmp
//! file, so the file on disk is always one whole write (last writer
//! wins; a lost increment, never a torn file).
//!
//! Weight comes from recency buckets (last 4 days ×100, 14 ×70, 31
//! ×50, 90 ×30, older ×10) times count. An empty query orders by
//! weight, then name. A typed query adds a bonus capped at a quarter
//! of a perfect match ([`super::fuzzy::perfect`]), so a habit never
//! outranks a much better text match. Run mode keys the same file by
//! `run:<argv0>`.
//!
//! This file is `std`-only, so the fuzz target can compile it by
//! `#[path]` with nothing else.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The history file name under `$XDG_STATE_HOME/scootlaunch/`.
pub const FILE_NAME: &str = "history";
/// At most this many entries are kept; past it the oldest go.
pub const MAX_ENTRIES: usize = 512;
/// Past this many bytes the file is not written: the run works
/// without history, with no error, rather than an unbounded file.
pub const MAX_FILE: usize = 32 * 1024;
/// Past this many bytes a file is not even read: hostile input
/// answers "no history", never a large allocation.
const READ_CAP: usize = 1024 * 1024;
/// An id past this long is not recorded and not loaded: it cannot
/// come from a real desktop id, and it bounds one line.
const MAX_ID: usize = 512;

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// One history record: launches seen, and the last launch's epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub count: u64,
    pub last: u64,
}

/// The loaded history: ids to their records.
#[derive(Debug, Clone, Default)]
pub struct Frecency {
    map: HashMap<String, Record>,
}

impl Frecency {
    /// Empty history: no `XDG_STATE_HOME`, or nothing recorded yet.
    pub fn empty() -> Self {
        Frecency {
            map: HashMap::new(),
        }
    }

    /// The frecency weight of one id at `now` (epoch seconds):
    /// recency bucket times count. Unknown ids weigh 0. A `last`
    /// after `now` (clock skew, hand-edited file) counts as most
    /// recent, never as an error.
    pub fn weight(&self, id: &str, now: u64) -> u64 {
        let Some(record) = self.map.get(id) else {
            return 0;
        };
        let age = now.saturating_sub(record.last);
        let bucket: u64 = if age <= 4 * SECONDS_PER_DAY {
            100
        } else if age <= 14 * SECONDS_PER_DAY {
            70
        } else if age <= 31 * SECONDS_PER_DAY {
            50
        } else if age <= 90 * SECONDS_PER_DAY {
            30
        } else {
            10
        };
        bucket.saturating_mul(record.count)
    }

    /// Records one launch at `now`, rewriting the file atomically.
    /// Best-effort throughout: answers whether a history was written.
    /// Refuses an id that cannot round-trip a line (`\n`, `\r`,
    /// empty, overlong) rather than writing a file it cannot read.
    ///
    /// No caller in this ticket: querying never records. The
    /// launch-spawn ticket calls this after a launch; it lands here
    /// because the file's bounds and atomicity are this ticket's to
    /// pin, with tests.
    #[allow(dead_code)]
    pub fn record(&mut self, state_home: Option<&Path>, id: &str, now: u64) -> bool {
        if id.is_empty() || id.len() > MAX_ID || id.contains(['\n', '\r']) {
            return false;
        }
        let record = self.map.entry(id.to_string()).or_insert(Record {
            count: 0,
            last: now,
        });
        record.count = record.count.saturating_add(1);
        record.last = now;
        store(state_home, self)
    }

    /// Test hook: the record for one id, if any.
    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<Record> {
        self.map.get(id).copied()
    }

    /// Test hook: plant a record without touching the clock or disk.
    #[cfg(test)]
    pub fn insert_for_tests(&mut self, id: &str, count: u64, last: u64) {
        self.map.insert(id.to_string(), Record { count, last });
    }

    /// How many ids are loaded. The fuzz target holds the bound;
    /// the panel ticket reads it for its result count.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Where the history lives. `None` (no `$HOME` and no
/// `$XDG_STATE_HOME`) means no history, never an error.
pub fn history_path(state_home: Option<&Path>) -> Option<PathBuf> {
    state_home.map(|dir| dir.join("scootlaunch").join(FILE_NAME))
}

/// Loads the history under `state_home`. Any failure — missing, too large,
/// unreadable, all lines malformed — answers empty, never an error.
pub fn load(state_home: Option<&Path>) -> Frecency {
    let Some(path) = history_path(state_home) else {
        return Frecency::empty();
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return Frecency::empty(),
    };
    if bytes.len() > READ_CAP {
        return Frecency::empty();
    }
    Frecency::from_bytes(&bytes)
}

impl Frecency {
    /// Parses history bytes: every well-formed line wins, the first
    /// occurrence of a repeated id. Never panics, whatever the
    /// input; the fuzz target holds this to hostile bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut map = HashMap::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let text = match std::str::from_utf8(line) {
                Ok(text) => text,
                Err(_) => continue,
            };
            // The id is everything before the last two tabs, so an
            // id holding a tab still round-trips.
            let Some(restart) = text.rfind('\t') else {
                continue;
            };
            let (head, last_text) = text.split_at(restart);
            let last_text = &last_text[1..];
            let Some(resplit) = head.rfind('\t') else {
                continue;
            };
            let (id, count_text) = head.split_at(resplit);
            let count_text = &count_text[1..];
            if id.is_empty() || id.len() > MAX_ID {
                continue;
            }
            let (Ok(count), Ok(last)) = (count_text.parse::<u64>(), last_text.parse::<u64>())
            else {
                continue;
            };
            if map.len() >= MAX_ENTRIES {
                break;
            }
            map.entry(id.to_string()).or_insert(Record { count, last });
        }
        Frecency { map }
    }

    /// Serializes for the file: most recently launched first, so a
    /// later truncation keeps the freshest records.
    fn to_bytes(&self) -> Vec<u8> {
        let mut records: Vec<(&String, &Record)> = self.map.iter().collect();
        records.sort_by(|a, b| b.1.last.cmp(&a.1.last).then(a.0.cmp(b.0)));
        let mut out = Vec::new();
        for (id, record) in records.into_iter().take(MAX_ENTRIES) {
            out.extend_from_slice(id.as_bytes());
            out.push(b'\t');
            out.extend_from_slice(record.count.to_string().as_bytes());
            out.push(b'\t');
            out.extend_from_slice(record.last.to_string().as_bytes());
            out.push(b'\n');
            if out.len() > MAX_FILE {
                break;
            }
        }
        out
    }
}

/// Writes the history atomically (pid-unique tmp file, then
/// `rename`). Past 32 KiB the oldest records are dropped until it
/// fits; on any I/O failure the run works without a history.
fn store(state_home: Option<&Path>, frecency: &Frecency) -> bool {
    let Some(path) = history_path(state_home) else {
        return false;
    };
    let mut bytes = frecency.to_bytes();
    while bytes.len() > MAX_FILE {
        // Drop the last (oldest) line until the file fits. An id is
        // capped at 512 bytes, so one line always fits.
        let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
            bytes.clear();
            break;
        };
        bytes.truncate(end);
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return false;
    }
    // The tmp name carries the pid so two launchers racing a record
    // never share one; `rename` is atomic on the same directory.
    let tmp = parent.join(format!("{FILE_NAME}.tmp.{}", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// Epoch seconds now. The caller passes this into [`Frecency`]
/// methods so tests use fixed times instead of the clock.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
