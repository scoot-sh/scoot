//! The desktop entry format: Desktop Entry Specification §3 and §4.
//!
//! UTF-8, case-sensitive. `#` lines and blank lines are comments. A
//! `[Desktop Entry]` group must come first; a file without one is
//! skipped. `Key=Value` splits at the first `=` with surrounding
//! spaces trimmed. Key names are `A-Za-z0-9-` plus a locale suffix.
//! The escapes `\s \n \t \r \\` apply in string values. Lists split on
//! `;` with `\;` escaped, and the trailing `;` is optional. Booleans
//! are exactly `true` or `false`, and anything else is a parse error
//! for that key (the key is ignored, the entry kept). Duplicate keys
//! in a group: the first wins, and a debug line records it.
//!
//! Bounds (the design's Security section): lines at most 4 KiB, at most
//! 32 groups, string values cut at 1 KiB. A file breaking the line or
//! group bound is skipped whole, never truncated into a wrong entry.
//!
//! This file is `std`-only, so the fuzz target can compile it by
//! `#[path]` with nothing else.

#[cfg(test)]
mod tests;

/// A line longer than this skips the file. Real desktop files keep
/// lines short; the bound caps what one hostile line can cost.
pub const MAX_LINE: usize = 4096;
/// A file with more groups than this is skipped whole.
pub const MAX_GROUPS: usize = 32;
/// String values longer than this are cut, at a character boundary.
pub const MAX_STRING: usize = 1024;

/// One `[group]` and its keys in file order. Duplicate keys are
/// already resolved: the first occurrence wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub keys: Vec<(String, String)>,
}

impl Group {
    /// The raw value of the first `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

/// A parsed file: its groups in order, plus the debug lines the index
/// prints (duplicate keys, repeated groups, ignored lines).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    pub groups: Vec<Group>,
    /// Human-readable notes (`group/key: what happened`), in order.
    pub notes: Vec<String>,
}

/// Why a file is skipped whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not UTF-8. The spec's format is UTF-8; a file that is not is
    /// skipped, never decoded lossily (lossy decoding would mint
    /// entries from mojibake).
    NotUtf8,
    /// A line longer than [`MAX_LINE`].
    LineTooLong(usize),
    /// More than [`MAX_GROUPS`] groups.
    TooManyGroups,
    /// No groups at all, or the first group is not `[Desktop Entry]`.
    NoDesktopEntry,
}

/// Parses one desktop file's bytes. Malformed UTF-8 and hostile sizes
/// skip the file ([`Error`]); a bad key skips the key, never the file.
pub fn parse(bytes: &[u8]) -> Result<ParsedFile, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::NotUtf8)?;
    let mut file = ParsedFile::default();
    let mut current: Option<usize> = None;
    for (index, raw_line) in text.lines().enumerate() {
        // `str::lines` already cut the `\n`; a `\r` stays, and the
        // spec's files are plain `\n`, so a stray `\r` is trimmed as
        // surrounding space rather than kept in a value.
        let line_no = index + 1;
        if raw_line.len() > MAX_LINE {
            return Err(Error::LineTooLong(line_no));
        }
        let line = raw_line.trim_matches(|char: char| char == ' ' || char == '\t' || char == '\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = parse_group_header(line) {
            if file.groups.len() >= MAX_GROUPS {
                return Err(Error::TooManyGroups);
            }
            match file.groups.iter().position(|group| group.name == name) {
                Some(existing) => {
                    file.notes.push(format!(
                        "[{name}]: repeated group, keys merge into the first (line {line_no})"
                    ));
                    current = Some(existing);
                }
                None => {
                    file.groups.push(Group {
                        name,
                        keys: Vec::new(),
                    });
                    current = Some(file.groups.len() - 1);
                }
            }
            continue;
        }
        let Some(position) = current else {
            file.notes
                .push(format!("line {line_no}: key outside any group, ignored"));
            continue;
        };
        let Some((key, value)) = split_key_value(line) else {
            file.notes.push(format!("line {line_no}: no `=`, ignored"));
            continue;
        };
        if !valid_key_name(&key) {
            file.notes
                .push(format!("line {line_no}: bad key name `{key}`, ignored"));
            continue;
        }
        let group = &mut file.groups[position];
        if group.keys.iter().any(|(name, _)| name == &key) {
            file.notes.push(format!(
                "[{}]: duplicate key `{key}`, the first wins (line {line_no})",
                group.name
            ));
            continue;
        }
        group.keys.push((key, truncate_string(&value)));
    }
    match file.groups.first() {
        Some(first) if first.name == "Desktop Entry" => Ok(file),
        _ => Err(Error::NoDesktopEntry),
    }
}

/// `[name]` with surrounding spaces trimmed, or `None`. A `[` without
/// a closing `]` is not a header (it falls through to key parsing,
/// where it is ignored for having no `=`). A header holding control
/// characters is not a group either: it is ignored with a note, so
/// controls can never ride a group name into an action id.
fn parse_group_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let name = inner.trim_matches(|char: char| char == ' ' || char == '\t');
    if name.is_empty() || name.chars().any(|char| char.is_control()) {
        return None;
    }
    Some(name.to_string())
}

/// Splits at the first `=`, trimming ASCII spaces and tabs around
/// both sides. No `=` means no key.
fn split_key_value(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim_matches(|char: char| char == ' ' || char == '\t');
    let value = value.trim_matches(|char: char| char == ' ' || char == '\t');
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

/// `A-Za-z0-9-` plus one optional `[locale]` suffix. The suffix may
/// hold anything but `]` (it is matched literally at lookup).
fn valid_key_name(key: &str) -> bool {
    let (base, suffix) = match key.split_once('[') {
        Some((base, suffix)) => (base, Some(suffix)),
        None => (key, None),
    };
    if base.is_empty()
        || !base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return false;
    }
    match suffix {
        None => true,
        Some(suffixed) => match suffixed.strip_suffix(']') {
            Some(inner) => {
                !inner.is_empty()
                    && !inner.contains(['[', ']', '='])
                    && !inner.chars().any(|char| char.is_control())
            }
            None => false,
        },
    }
}

/// Cuts a value to [`MAX_STRING`] bytes at a character boundary.
fn truncate_string(value: &str) -> String {
    if value.len() <= MAX_STRING {
        return value.to_string();
    }
    let mut end = MAX_STRING;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// The escapes of string values: `\s \n \t \r \\`. Anything else is a
/// parse error for that key (the key is ignored, the entry kept).
/// Unknown escapes are refused rather than passed through: a
/// backslash that survives here would reach the Exec splitter, where
/// it means quoting.
pub fn unescape(value: &str) -> Result<String, char> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(char) = chars.next() {
        if char != '\\' {
            out.push(char);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => return Err(other),
            None => return Err('\\'),
        }
    }
    Ok(out)
}

/// Splits a list value on unescaped `;` (`\;` is an escaped
/// separator), then unescapes each item. The trailing `;` the spec
/// asks writers to add is optional: a trailing empty item is dropped,
/// inner empty items are kept.
pub fn split_list(value: &str) -> Result<Vec<String>, char> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    let mut any = false;
    while let Some(char) = chars.next() {
        any = true;
        if char == '\\' {
            match chars.peek() {
                Some(';') => {
                    current.push(';');
                    chars.next();
                }
                // Any other escape rides along for `unescape` to
                // judge per item, so its error names the item.
                _ => current.push('\\'),
            }
            continue;
        }
        if char == ';' {
            items.push(std::mem::take(&mut current));
        } else {
            current.push(char);
        }
    }
    if !any {
        return Ok(Vec::new());
    }
    if !current.is_empty() {
        items.push(current);
    }
    items.into_iter().map(|item| unescape(&item)).collect()
}

/// Exactly `true` or `false`, per the spec. Anything else is a parse
/// error for that key: the key is ignored, the entry kept.
pub fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Drops control characters from a display string (the `Name`,
/// `GenericName`, `Comment` and `Keywords` the panel draws). A
/// control character in a desktop file is never legitimate text.
pub fn clean_display(value: &str) -> String {
    value.chars().filter(|char| !char.is_control()).collect()
}
