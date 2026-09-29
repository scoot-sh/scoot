//! Which time zone the clock shows, read the way glibc reads it, and read
//! again when it changes.
//!
//! ## `TZ`, then `/etc/localtime`
//!
//! As glibc's `tzset` ([`Spec::from_env`]):
//!
//! - `TZ` unset: the file `/etc/localtime`.
//! - `TZ` empty: UTC.
//! - Otherwise, a leading `:` is dropped, and the rest names a zone file:
//!   an absolute path as it is, anything else under `$TZDIR` (NixOS sets
//!   it) or `/usr/share/zoneinfo`. If that file cannot be read as a zone,
//!   the value is read as a POSIX TZ string (`EST5EDT,M3.2.0,M11.1.0`,
//!   `<+0530>-5:30`), and failing that the zone is UTC.
//!
//! Anything unreadable ends in UTC, with one line on stderr saying so,
//! never in a refusal to start: a clock in the wrong zone is better than no
//! bar.
//!
//! ## Reading a file safely
//!
//! Opened `O_NONBLOCK` (a FIFO named by `TZ` cannot block the loop), and
//! read only if `fstat` says it is a regular file, at most
//! [`tzif::MAX_FILE`] + 1 bytes, so `TZ=:/dev/zero` or a huge file cannot
//! hang or balloon the bar; a file past the cap is refused.
//!
//! ## A change
//!
//! On every wake the clock `statx`es the zone file (following symlinks, so
//! `/etc/localtime` re-pointed by `timedatectl` counts) and reads it again
//! when its device, inode, modification time or size changed: one system
//! call a tick, no extra fd and no extra wakeup, and a new zone shows at the
//! next tick (M0 measured it, the record's §3c). A file that has
//! disappeared keeps the zone last read (an update that removes and
//! re-creates the link would otherwise flash UTC); one that appears later
//! is read then.

use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, StatxFlags, fstat, open, statx};

use super::tzif::{MAX_FILE, Tz};

#[cfg(test)]
mod tests;

/// The zone file when `TZ` is unset.
pub const LOCALTIME: &str = "/etc/localtime";
/// Where zone names are looked up when `$TZDIR` is unset.
pub const ZONEINFO: &str = "/usr/share/zoneinfo";

/// Where the zone comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    /// A TZif file; if it cannot be read, `fallback` (the `TZ` value as a
    /// POSIX string, when there was one), else UTC.
    File {
        path: PathBuf,
        fallback: Option<Vec<u8>>,
    },
    Utc,
}

impl Spec {
    /// From `TZ` and `TZDIR` as the process has them.
    pub fn from_env() -> Self {
        Self::resolve(
            std::env::var_os("TZ").as_deref(),
            std::env::var_os("TZDIR").as_deref(),
        )
    }

    /// From the values of `TZ` and `TZDIR` (`None`: unset).
    pub fn resolve(tz: Option<&OsStr>, tzdir: Option<&OsStr>) -> Self {
        let Some(tz) = tz else {
            return Self::File {
                path: PathBuf::from(LOCALTIME),
                fallback: None,
            };
        };
        let name = tz.as_bytes();
        let name = name.strip_prefix(b":").unwrap_or(name);
        if name.is_empty() {
            return Self::Utc;
        }
        let path = if name.starts_with(b"/") {
            PathBuf::from(OsStr::from_bytes(name))
        } else {
            let dir = tzdir
                .filter(|dir| !dir.is_empty())
                .unwrap_or(OsStr::new(ZONEINFO));
            Path::new(dir).join(OsStr::from_bytes(name))
        };
        Self::File {
            path,
            fallback: Some(name.to_vec()),
        }
    }

    /// The file to watch for a change, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File { path, .. } => Some(path),
            Self::Utc => None,
        }
    }
}

/// What identifies one version of a zone file: device, inode, modification
/// time and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    dev: (u32, u32),
    ino: u64,
    mtime: (i64, u32),
    size: u64,
}

/// `statx` of `path`, following symlinks; `None` when it cannot be read.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let stat = statx(
        CWD,
        path,
        AtFlags::empty(),
        StatxFlags::BASIC_STATS | StatxFlags::MTIME,
    )
    .ok()?;
    Some(Stamp {
        dev: (stat.stx_dev_major, stat.stx_dev_minor),
        ino: stat.stx_ino,
        mtime: (stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec),
        size: stat.stx_size,
    })
}

/// The zone as read, and why it fell back if it did.
#[derive(Debug)]
pub struct Loaded {
    pub tz: Tz,
    /// Set when the zone asked for could not be used and UTC stands in:
    /// what went wrong, for one line on stderr.
    pub problem: Option<String>,
}

/// Reads the zone `spec` names (see the module docs for the order).
pub fn load(spec: &Spec) -> Loaded {
    let Spec::File { path, fallback } = spec else {
        return Loaded {
            tz: Tz::utc(),
            problem: None,
        };
    };
    let file_error = match read_capped(path) {
        Ok(bytes) => match Tz::parse(&bytes) {
            Ok(tz) => {
                return Loaded { tz, problem: None };
            }
            Err(error) => error.to_owned(),
        },
        Err(error) => error,
    };
    if let Some(text) = fallback {
        if let Ok(tz) = Tz::posix(text) {
            return Loaded { tz, problem: None };
        }
    }
    Loaded {
        tz: Tz::utc(),
        problem: Some(format!(
            "cannot read the time zone {}: {file_error}; showing UTC",
            path.display()
        )),
    }
}

/// `path`'s bytes: a regular file of at most [`MAX_FILE`] bytes, read
/// without blocking on anything that is not one.
pub fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(|errno| std::io::Error::from(errno).to_string())?;
    let stat = fstat(&fd).map_err(|errno| std::io::Error::from(errno).to_string())?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err("not a regular file".to_owned());
    }
    let cap = MAX_FILE + 1;
    let mut bytes = Vec::with_capacity(usize::try_from(stat.st_size).unwrap_or(0).min(cap));
    std::fs::File::from(fd)
        .take(cap as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err(format!("larger than {MAX_FILE} bytes"));
    }
    Ok(bytes)
}
