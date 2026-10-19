//! The index cache: parsing every file at every open is the cost
//! the prototype measured (`docs/scootbar/backlog/resolved/
//! launch-prototype-done.md`: 47 ms at 3,000 files), and the plan
//! builds both and keeps the cache only if it wins by more than the
//! noise (`dev/design/scootlaunch.md`, Caching).
//!
//! `$XDG_CACHE_HOME/scootlaunch/index-v1` is a compact binary of the
//! visible entries, their search fields and Exec argv templates,
//! written atomically (tmp file, then `rename`), at most 1 MiB. The
//! key is the canonical path, inode and mtime of every directory
//! walked — canonical paths matter on NixOS, where a profile switch
//! changes the canonical path and never the mtime — plus every file
//! seen (an edited user file bumps nothing but its own mtime), the
//! locale, and `XDG_CURRENT_DESKTOP`. Any mismatch, a short read or
//! a version change means a rescan. A bad cache is never an error.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The cache file name under `$XDG_CACHE_HOME/scootlaunch/`.
pub const CACHE_FILE: &str = "index-v1";
/// Past this many bytes the cache is not written: the run works
/// without it, with one warning, rather than an unbounded file.
pub const MAX_CACHE: usize = 1024 * 1024;
const MAGIC: &[u8; 8] = b"SLIX0001";
/// The entries gained `workdir` (the `Path=` key) in the launch-spawn
/// ticket: a version change, so an old cache rescans instead of
/// misreading the new trailing field.
const VERSION: u32 = 2;

/// Where the cache lives. `None` (no `$HOME` and no
/// `$XDG_CACHE_HOME`) means no cache, never an error.
pub fn cache_path(env: &super::Env) -> Option<PathBuf> {
    env.xdg_cache_home
        .clone()
        .map(|dir| dir.join("scootlaunch").join(CACHE_FILE))
}

/// Loads the cache when its key matches `key`. Any mismatch, short
/// read or version change answers `None`: a rescan, never an error.
pub fn load(env: &super::Env, key: &CacheKey) -> Option<Vec<super::Entry>> {
    let path = cache_path(env)?;
    let bytes = std::fs::read(&path).ok()?;
    let mut cursor = Cursor {
        bytes: &bytes,
        position: 0,
    };
    if cursor.take_bytes(MAGIC.len()).ok()? != MAGIC {
        return None;
    }
    if cursor.take_u32().ok()? != VERSION {
        return None;
    }
    let stored = take_key(&mut cursor).ok()?;
    if &stored != key {
        return None;
    }
    let entries = take_entries(&mut cursor).ok()?;
    if cursor.position != cursor.bytes.len() {
        // Trailing bytes: not what `store` wrote.
        return None;
    }
    Some(entries)
}

/// Stores the index under its key, atomically (tmp file in the same
/// directory, then `rename`). Best-effort throughout: past 1 MiB, or
/// on any I/O failure, the run works without a cache. Answers whether
/// a cache was written.
pub fn store(env: &super::Env, key: &CacheKey, entries: &[super::Entry]) -> bool {
    let Some(path) = cache_path(env) else {
        return false;
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    put_key(&mut bytes, key);
    put_entries(&mut bytes, entries);
    if bytes.len() > MAX_CACHE {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return false;
    }
    // The tmp name carries the pid so two launchers racing an open
    // never share one; `rename` is atomic on the same directory.
    let tmp = parent.join(format!("{CACHE_FILE}.tmp.{}", std::process::id()));
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

/// Everything a rescan decision compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub entries: Vec<super::dirs::KeyEntry>,
    pub locale: String,
    pub current_desktop: String,
}

impl CacheKey {
    pub fn new(
        mut entries: Vec<super::dirs::KeyEntry>,
        locale: &str,
        current_desktop: &[String],
    ) -> Self {
        // The walk is deterministic, but the key compares across
        // runs: sort by path so readdir order can never matter.
        entries.sort_by(|a, b| key_path(a).cmp(key_path(b)));
        CacheKey {
            entries,
            locale: locale.to_string(),
            current_desktop: current_desktop.join(":"),
        }
    }
}

fn key_path(entry: &super::dirs::KeyEntry) -> &Path {
    match entry {
        super::dirs::KeyEntry::Dir { path, .. } => path,
        super::dirs::KeyEntry::File { path, .. } => path,
    }
}

// The binary format, little-endian throughout. `u32` lengths, `u64`
// dev/ino, `i64` seconds plus `u32` nanos (before-epoch clamps to
// zero), `u8` tags for options, `u8` booleans.

fn put_string(bytes: &mut Vec<u8>, text: &str) {
    bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
}

fn take_string(cursor: &mut Cursor<'_>) -> Result<String, ()> {
    let len = cursor.take_u32()? as usize;
    let raw = cursor.take_bytes(len)?;
    std::str::from_utf8(raw).map(str::to_string).map_err(|_| ())
}

fn put_option(bytes: &mut Vec<u8>, value: &Option<String>) {
    match value {
        Some(text) => {
            bytes.push(1);
            put_string(bytes, text);
        }
        None => bytes.push(0),
    }
}

fn take_option(cursor: &mut Cursor<'_>) -> Result<Option<String>, ()> {
    match cursor.take_byte()? {
        1 => Ok(Some(take_string(cursor)?)),
        0 => Ok(None),
        _ => Err(()),
    }
}

fn put_time(bytes: &mut Vec<u8>, time: SystemTime) {
    let (secs, nanos) = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs() as i64, duration.subsec_nanos()),
        Err(_) => (0, 0),
    };
    bytes.extend_from_slice(&secs.to_le_bytes());
    bytes.extend_from_slice(&nanos.to_le_bytes());
}

fn take_time(cursor: &mut Cursor<'_>) -> Result<SystemTime, ()> {
    let secs = i64::from_le_bytes(cursor.take_bytes(8)?.try_into().map_err(|_| ())?);
    let nanos = u32::from_le_bytes(cursor.take_bytes(4)?.try_into().map_err(|_| ())?);
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::new(secs.max(0) as u64, nanos))
}

fn put_key(bytes: &mut Vec<u8>, key: &CacheKey) {
    bytes.extend_from_slice(&(key.entries.len() as u32).to_le_bytes());
    for entry in &key.entries {
        let (tag, path, dev, ino, mtime) = match entry {
            super::dirs::KeyEntry::Dir {
                path,
                dev,
                ino,
                mtime,
            } => (0u8, path, dev, ino, mtime),
            super::dirs::KeyEntry::File {
                path,
                dev,
                ino,
                mtime,
            } => (1u8, path, dev, ino, mtime),
        };
        bytes.push(tag);
        put_string(bytes, &path.to_string_lossy());
        bytes.extend_from_slice(&dev.to_le_bytes());
        bytes.extend_from_slice(&ino.to_le_bytes());
        put_time(bytes, *mtime);
    }
    put_string(bytes, &key.locale);
    put_string(bytes, &key.current_desktop);
}

fn take_key(cursor: &mut Cursor<'_>) -> Result<CacheKey, ()> {
    let len = cursor.take_u32()? as usize;
    if len > 100_000 {
        return Err(());
    }
    let mut entries = Vec::with_capacity(len.min(4096));
    for _ in 0..len {
        let dir = match cursor.take_byte()? {
            0 => true,
            1 => false,
            _ => return Err(()),
        };
        let path = PathBuf::from(take_string(cursor)?);
        let dev = u64::from_le_bytes(cursor.take_bytes(8)?.try_into().map_err(|_| ())?);
        let ino = u64::from_le_bytes(cursor.take_bytes(8)?.try_into().map_err(|_| ())?);
        let mtime = take_time(cursor)?;
        entries.push(if dir {
            super::dirs::KeyEntry::Dir {
                path,
                dev,
                ino,
                mtime,
            }
        } else {
            super::dirs::KeyEntry::File {
                path,
                dev,
                ino,
                mtime,
            }
        });
    }
    let locale = take_string(cursor)?;
    let current_desktop = take_string(cursor)?;
    Ok(CacheKey {
        entries,
        locale,
        current_desktop,
    })
}

fn take_entries(cursor: &mut Cursor<'_>) -> Result<Vec<super::Entry>, ()> {
    let len = cursor.take_u32()? as usize;
    if len > super::MAX_ENTRIES {
        return Err(());
    }
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        entries.push(take_entry(cursor)?);
    }
    Ok(entries)
}

fn put_entries(bytes: &mut Vec<u8>, entries: &[super::Entry]) {
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        put_string(bytes, &entry.id);
        put_string(bytes, &entry.name);
        put_option(bytes, &entry.generic_name);
        put_option(bytes, &entry.comment);
        bytes.extend_from_slice(&(entry.keywords.len() as u32).to_le_bytes());
        for keyword in &entry.keywords {
            put_string(bytes, keyword);
        }
        put_option(bytes, &entry.icon);
        bytes.extend_from_slice(&(entry.exec.len() as u32).to_le_bytes());
        for arg in &entry.exec {
            put_string(bytes, arg);
        }
        bytes.push(u8::from(entry.terminal));
        bytes.push(u8::from(entry.dbus_activatable));
        bytes.extend_from_slice(&(entry.actions.len() as u32).to_le_bytes());
        for action in &entry.actions {
            put_string(bytes, &action.id);
            put_string(bytes, &action.name);
            bytes.extend_from_slice(&(action.exec.len() as u32).to_le_bytes());
            for arg in &action.exec {
                put_string(bytes, arg);
            }
        }
        put_string(bytes, &entry.path.to_string_lossy());
        put_option(bytes, &entry.workdir);
        match &entry.hidden {
            Some(reason) => {
                bytes.push(1);
                put_string(bytes, reason.as_str());
            }
            None => bytes.push(0),
        }
    }
}

fn take_entry(cursor: &mut Cursor<'_>) -> Result<super::Entry, ()> {
    let id = take_string(cursor)?;
    let name = take_string(cursor)?;
    let generic_name = take_option(cursor)?;
    let comment = take_option(cursor)?;
    let keywords = take_strings(cursor, super::MAX_KEYWORDS)?;
    let icon = take_option(cursor)?;
    let exec = take_strings(cursor, 1024)?;
    let terminal = match cursor.take_byte()? {
        1 => true,
        0 => false,
        _ => return Err(()),
    };
    let dbus_activatable = match cursor.take_byte()? {
        1 => true,
        0 => false,
        _ => return Err(()),
    };
    let action_len = cursor.take_u32()? as usize;
    if action_len > super::actions::MAX_ACTIONS {
        return Err(());
    }
    let mut actions = Vec::with_capacity(action_len);
    for _ in 0..action_len {
        actions.push(super::Action {
            id: take_string(cursor)?,
            name: take_string(cursor)?,
            exec: take_strings(cursor, 1024)?,
        });
    }
    let path = PathBuf::from(take_string(cursor)?);
    let workdir = take_option(cursor)?;
    let hidden = match cursor.take_byte()? {
        1 => Some(super::filter::HiddenReason::from_str(take_string(cursor)?.as_str()).ok_or(())?),
        0 => None,
        _ => return Err(()),
    };
    Ok(super::Entry {
        id,
        name,
        generic_name,
        comment,
        keywords,
        icon,
        exec,
        terminal,
        dbus_activatable,
        actions,
        path,
        workdir,
        hidden,
    })
}

fn take_strings(cursor: &mut Cursor<'_>, cap: usize) -> Result<Vec<String>, ()> {
    let len = cursor.take_u32()? as usize;
    if len > cap {
        return Err(());
    }
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        out.push(take_string(cursor)?);
    }
    Ok(out)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn take_bytes(&mut self, len: usize) -> Result<&[u8], ()> {
        let end = self.position.checked_add(len).ok_or(())?;
        if end > self.bytes.len() {
            return Err(());
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn take_byte(&mut self) -> Result<u8, ()> {
        Ok(self.take_bytes(1)?[0])
    }

    fn take_u32(&mut self) -> Result<u32, ()> {
        Ok(u32::from_le_bytes(
            self.take_bytes(4)?.try_into().map_err(|_| ())?,
        ))
    }
}
