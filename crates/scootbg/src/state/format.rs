//! The state file's format: hand-written lines, versioned
//! (dependencies-done.md §5 chose it over `toml`, 180 KB lighter).
//!
//! ```text
//! scootbg-state 2
//! profile default
//! fingerprint 9c1e...
//! all color #1e1e2e
//! output DP-1 image /home/me/My%20Pictures/hills.jpg fill #000000 lanczos3
//! output HDMI-A-1 clear
//! ```
//!
//! - **The first line** is `scootbg-state N`, `N` the format's version.
//!   A file whose first line is anything else is not restored from (and is
//!   replaced at the next save). A file of a *later* version than this
//!   build reads is not restored from either, and is never written over
//!   ([`Parsed::newer`]): it belongs to a newer scootbg, which would lose
//!   what this one cannot see.
//! - **Every other line** is a key and its fields, separated by single
//!   spaces. `profile` names the profile the file belongs to (its file
//!   name is the authority; this line is for a reader, and for ticket 10,
//!   whose daemon adopts profiles). `fingerprint` is the fingerprint of the
//!   last `[wallpaper]` section applied from scoot's config; only
//!   `apply-config` (ticket 10) sets it, and every other write keeps it as
//!   it was. `all` is the choice for every output, `output NAME` the choice
//!   for the outputs with that connector name; each is `clear`, `color
//!   #rrggbb`, or `image PATH MODE FILL FILTER`, with a downloaded image's
//!   URL (and pinned hash) after that: `image PATH MODE FILL FILTER url URL`
//!   and `image PATH MODE FILL FILTER url URL sha256 HEX`. PATH is the
//!   cache file; the URL is what re-downloads it.
//! - **Fields are escaped**: `%`, space and every ASCII control byte
//!   (newline and tab included) are written `%XX`, uppercase hex, so a
//!   field never holds a separator and a path round-trips exactly. Any
//!   other byte, UTF-8 beyond ASCII included, is written as it is.
//! - **Read defensively.** A line that is malformed (a field missing or
//!   extra, a bad escape, a relative path, an unknown mode), whose fields
//!   are not UTF-8 once unescaped, or whose key is unknown, is skipped with
//!   a warning, and the rest of the file still counts. A file over
//!   [`MAX_BYTES`] is not read at all; of more than [`MAX_OUTPUTS`]
//!   `output` lines, the last ones are kept (lines are oldest first, and
//!   [`encode`] itself never writes more, dropping the least recently set).
//!   The same key twice: the later line wins, with a warning.
//!
//! **Version 2** adds the `url` (and `sha256`) trailer to `image` lines, for
//! downloaded wallpapers. A version-1 file still reads (its `image` lines
//! have no trailer); a version-1 scootbg reading a version-2 file restores
//! nothing, as for any newer version.
//!
//! **Within version 2**, a key may be added only if a reader that skips
//! it (with its warning) loses nothing it needs; anything else bumps the
//! version. `profile` and `fingerprint` are both read and kept from this
//! first version on, so ticket 10 writes them with no bump.

use std::fmt::Write as _;

use crate::choices::Choice;
use crate::color::Color;
use crate::fetch::{Fetch, MAX_URL};
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::Wallpaper;

#[cfg(test)]
mod tests;

/// The format this build reads and writes.
pub const VERSION: u32 = 2;

/// The earliest format still read: version 1's `image` lines carry no
/// trailer, and read as file choices.
pub const OLDEST: u32 = 1;

/// The first line's first word.
pub const MAGIC: &str = "scootbg-state";

/// The largest file read. A real one is a few hundred bytes: a line per
/// connector ever given its own wallpaper, each with at most one path.
pub const MAX_BYTES: usize = 256 * 1024;

/// The most `output` lines read.
pub const MAX_OUTPUTS: usize = 256;

/// The longest fingerprint kept, in bytes (ticket 10's is a hash in hex).
pub const MAX_FINGERPRINT: usize = 256;

/// A choice as the file records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Nothing: the compositor's own background.
    Clear,
    Color(Color),
    Image {
        path: String,
        look: Look,
        /// A download, when the image is a URL: `path` is its cache file.
        fetch: Option<Fetch>,
    },
}

/// What a file says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Record {
    pub profile: Option<String>,
    pub fingerprint: Option<String>,
    /// For every output; `None` when the file has no `all` line.
    pub all: Option<Pick>,
    /// For single outputs, by connector name, in file order; one per name.
    pub named: Vec<(String, Pick)>,
}

/// A file read: what it says, and what was wrong with it.
#[derive(Debug, Default)]
pub struct Parsed {
    pub record: Record,
    /// One line each, for stderr.
    pub warnings: Vec<String>,
    /// Written by a newer scootbg (a later version): nothing was read from
    /// it, and it must not be written over.
    pub newer: bool,
}

/// What [`encode`] had to leave out to stay within what [`decode`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Left {
    /// The oldest per-output choices, this many from the front of `named`.
    pub named: usize,
    /// The choice for every output (only a line longer than the whole
    /// file may be, which only a hand-edited file can lead to).
    pub all: bool,
}

/// Appends the file for `profile` to `out`: the header, the profile, the
/// fingerprint if there is one, the choice for every output if one was
/// made, then the named choices, `named` given **oldest first**, and
/// written in that order, so a restore gives them generations in the same
/// order and "least recently set" survives a restart.
///
/// **Within what the reader takes** ([`MAX_OUTPUTS`] lines, [`MAX_BYTES`]
/// in all): when the named choices do not all fit, the oldest are left
/// out, so the newest always survive; the caller forgets them too
/// ([`Left`]), so the table and the file agree. A name that is empty is
/// left out (and not counted): no output can be given one by name, and
/// the format has no way to write it.
pub fn encode(
    out: &mut String,
    profile: &str,
    fingerprint: Option<&str>,
    all: Option<&Choice>,
    named: &[(&str, &Choice)],
) -> Left {
    let start = out.len();
    // Writing into a `String` cannot fail.
    let _ = writeln!(out, "{MAGIC} {VERSION}");
    out.push_str("profile ");
    escape(out, profile);
    out.push('\n');
    if let Some(fingerprint) = fingerprint.filter(|f| !f.is_empty()) {
        out.push_str("fingerprint ");
        escape(out, fingerprint);
        out.push('\n');
    }
    let mut left = Left::default();
    if let Some(choice) = all {
        let before = out.len();
        out.push_str("all ");
        pick(out, choice);
        out.push('\n');
        // A path from a request is at most `MAX_REQUEST_LINE` (64 KiB), so
        // three times that escaped fits; only a hand-edited file's can not.
        if out.len() - start > MAX_BYTES {
            out.truncate(before);
            left.all = true;
        }
    }
    // Newest first, each whole line while it fits, then written oldest
    // first. One `String` per line, per save: a save is per `set`.
    let mut budget = MAX_BYTES.saturating_sub(out.len() - start);
    let mut lines: Vec<String> = Vec::new();
    let mut kept = 0;
    for (name, choice) in named.iter().rev() {
        if name.is_empty() {
            kept += 1;
            continue;
        }
        if lines.len() == MAX_OUTPUTS {
            break;
        }
        let mut line = String::from("output ");
        escape(&mut line, name);
        line.push(' ');
        pick(&mut line, choice);
        line.push('\n');
        if line.len() > budget {
            break;
        }
        budget -= line.len();
        lines.push(line);
        kept += 1;
    }
    for line in lines.iter().rev() {
        out.push_str(line);
    }
    left.named = named.len() - kept;
    left
}

fn pick(out: &mut String, choice: &Choice) {
    match choice {
        None => out.push_str("clear"),
        Some(Wallpaper::Color(color)) => {
            let _ = write!(out, "color {color}");
        }
        Some(Wallpaper::Image(image)) => {
            out.push_str("image ");
            escape(out, &image.path);
            let look = image.look;
            let _ = write!(
                out,
                " {} {} {}",
                look.mode.name(),
                look.fill,
                look.filter.name()
            );
            if let Some(fetch) = &image.fetch {
                out.push_str(" url ");
                escape(out, &fetch.url);
                if let Some(sha256) = fetch.sha256 {
                    out.push_str(" sha256 ");
                    out.push_str(&crate::sha256::hex_bytes(sha256.as_slice()));
                }
            }
        }
    }
}

/// `field` with `%`, space and ASCII control bytes as `%XX`.
pub fn escape(out: &mut String, field: &str) {
    for ch in field.chars() {
        if ch == '%' || ch == ' ' || ch.is_ascii_control() {
            // ASCII, so one byte.
            let _ = write!(out, "%{:02X}", u32::from(ch));
        } else {
            out.push(ch);
        }
    }
}

/// Why one field could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldError {
    /// `%` not followed by two hex digits.
    Escape,
    /// Not UTF-8 once unescaped.
    NotUtf8,
}

/// `field` with its `%XX` escapes undone, as UTF-8.
pub fn unescape(field: &[u8]) -> Result<String, FieldError> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field;
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let [high, low, tail @ ..] = tail else {
                return Err(FieldError::Escape);
            };
            let (Some(high), Some(low)) = (hex(*high), hex(*low)) else {
                return Err(FieldError::Escape);
            };
            bytes.push(high << 4 | low);
            rest = tail;
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(bytes).map_err(|_| FieldError::NotUtf8)
}

fn hex(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// Reads a state file's bytes. Never fails: what cannot be read is a
/// warning, and whatever else the file says still counts.
pub fn decode(bytes: &[u8]) -> Parsed {
    let mut parsed = Parsed::default();
    if bytes.len() > MAX_BYTES {
        parsed.warnings.push(format!(
            "it is over {MAX_BYTES} bytes, which no state scootbg writes is; \
             nothing was restored from it"
        ));
        return parsed;
    }
    let mut lines = bytes.split(|&b| b == b'\n');
    let header = lines.next().unwrap_or_default();
    match version(header) {
        Some(OLDEST) | Some(VERSION) => {}
        Some(later) if later > VERSION => {
            parsed.newer = true;
            parsed.warnings.push(format!(
                "it is version {later} of the format, from a newer scootbg; this one reads \
                 version {VERSION}, so nothing was restored from it, and it is left as it is"
            ));
            return parsed;
        }
        _ => {
            parsed.warnings.push(format!(
                "its first line is not `{MAGIC} {OLDEST}` or `{MAGIC} {VERSION}`; nothing \
                 was restored from it"
            ));
            return parsed;
        }
    }
    let mut seen_all = false;
    for (index, line) in lines.enumerate() {
        // Line numbers from 1, the header being line 1.
        let number = index + 2;
        if line.is_empty() {
            continue;
        }
        if let Err(why) = entry(&mut parsed.record, line, &mut seen_all) {
            let note = match why {
                Skip::Duplicate(what) => {
                    format!("line {number}: a second {what}; the later one counts")
                }
                Skip::TooMany => format!("line {number}: {}", Skip::TooMany),
                other => format!("line {number}: {other}; skipped"),
            };
            parsed.warnings.push(note);
        }
    }
    parsed
}

/// The version a header line names, if it is one.
fn version(header: &[u8]) -> Option<u32> {
    let rest = header.strip_prefix(MAGIC.as_bytes())?.strip_prefix(b" ")?;
    // Digits only: `u32::from_str` would also take a leading `+`.
    if rest.is_empty() || !rest.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(rest).ok()?.parse().ok()
}

/// Why a line was skipped (or, for a duplicate, taken over an earlier one).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Skip {
    Unknown(String),
    Fields(&'static str),
    Field(&'static str, FieldError),
    Relative,
    Mode(String),
    Filter(String),
    Color(&'static str),
    FingerprintTooLong,
    Empty(&'static str),
    TooMany,
    /// Not skipped: taken, over an earlier line.
    Duplicate(&'static str),
    /// A `url` that is not an `http(s)` URL, or past [`MAX_URL`].
    Url,
    /// A `sha256` that is not 64 hex digits, or one without a `url`.
    Sha,
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(key) => write!(f, "unknown key {key:?}"),
            Self::Fields(want) => write!(f, "malformed: want {want}"),
            Self::Field(what, FieldError::Escape) => {
                write!(f, "the {what} has a `%` that is not `%` and two hex digits")
            }
            Self::Field(what, FieldError::NotUtf8) => write!(f, "the {what} is not UTF-8"),
            Self::Relative => write!(f, "the image path is not absolute"),
            Self::Mode(mode) => write!(f, "unknown mode {mode:?}"),
            Self::Filter(filter) => write!(f, "unknown filter {filter:?}"),
            Self::Color(what) => write!(f, "the {what} is not #rrggbb"),
            Self::FingerprintTooLong => {
                write!(f, "the fingerprint is over {MAX_FINGERPRINT} bytes")
            }
            Self::Empty(what) => write!(f, "the {what} is empty"),
            Self::TooMany => write!(
                f,
                "more than {MAX_OUTPUTS} outputs: the earliest `output` line kept so far is dropped"
            ),
            Self::Duplicate(what) => write!(f, "a second {what}"),
            Self::Url => write!(
                f,
                "the `url` is not an `http(s)` URL of at most {MAX_URL} bytes without a NUL byte"
            ),
            Self::Sha => write!(
                f,
                "the `sha256` is not 64 hex digits, or there is no `url` for it"
            ),
        }
    }
}

/// Reads one line into `record`.
fn entry(record: &mut Record, line: &[u8], seen_all: &mut bool) -> Result<(), Skip> {
    let mut fields = line.split(|&b| b == b' ');
    let key = fields.next().unwrap_or_default();
    let fields: Vec<&[u8]> = fields.collect();
    match key {
        b"profile" => {
            let [name] = fields[..] else {
                return Err(Skip::Fields("`profile NAME`"));
            };
            let name = text(name, "profile name")?;
            let again = record.profile.replace(name).is_some();
            again_is(again, "`profile` line")
        }
        b"fingerprint" => {
            let [value] = fields[..] else {
                return Err(Skip::Fields("`fingerprint VALUE`"));
            };
            let value = text(value, "fingerprint")?;
            if value.len() > MAX_FINGERPRINT {
                return Err(Skip::FingerprintTooLong);
            }
            let again = record.fingerprint.replace(value).is_some();
            again_is(again, "`fingerprint` line")
        }
        b"all" => {
            let pick = pick_of(&fields)?;
            record.all = Some(pick);
            let again = std::mem::replace(seen_all, true);
            again_is(again, "`all` line")
        }
        b"output" => {
            let [name, rest @ ..] = &fields[..] else {
                return Err(Skip::Fields("`output NAME` and a choice"));
            };
            let name = text(name, "output name")?;
            let pick = pick_of(rest)?;
            let full = record.named.len() >= MAX_OUTPUTS;
            match record.named.iter_mut().find(|(n, _)| *n == name) {
                Some((_, slot)) => {
                    *slot = pick;
                    Err(Skip::Duplicate("line for this output"))
                }
                // Lines are oldest first, as scootbg writes them: over the
                // limit, the oldest goes, as the writer would drop it.
                None if full => {
                    record.named.remove(0);
                    record.named.push((name, pick));
                    Err(Skip::TooMany)
                }
                None => {
                    record.named.push((name, pick));
                    Ok(())
                }
            }
        }
        other => Err(Skip::Unknown(String::from_utf8_lossy(other).into_owned())),
    }
}

fn again_is(again: bool, what: &'static str) -> Result<(), Skip> {
    if again {
        Err(Skip::Duplicate(what))
    } else {
        Ok(())
    }
}

/// A non-empty field, unescaped.
fn text(field: &[u8], what: &'static str) -> Result<String, Skip> {
    let value = unescape(field).map_err(|error| Skip::Field(what, error))?;
    if value.is_empty() {
        return Err(Skip::Empty(what));
    }
    Ok(value)
}

const PICK: &str =
    "`clear`, `color #rrggbb` or `image PATH MODE FILL FILTER [url URL [sha256 HEX]]`";

fn pick_of(fields: &[&[u8]]) -> Result<Pick, Skip> {
    match fields {
        [b"clear"] => Ok(Pick::Clear),
        [b"color", color] => color_of(color, "color").map(Pick::Color),
        [b"image", path, mode, fill, filter, trailer @ ..] => {
            let path = text(path, "image path")?;
            if !path.starts_with('/') {
                return Err(Skip::Relative);
            }
            let mode_name = text(mode, "mode")?;
            let mode = Mode::from_name(&mode_name).ok_or(Skip::Mode(mode_name))?;
            let fill = color_of(fill, "fill color")?;
            let filter_name = text(filter, "filter")?;
            let filter = Filter::from_name(&filter_name).ok_or(Skip::Filter(filter_name))?;
            let fetch = trailer_of(trailer)?;
            Ok(Pick::Image {
                path,
                look: Look { mode, fill, filter },
                fetch,
            })
        }
        _ => Err(Skip::Fields(PICK)),
    }
}

/// A downloaded image's trailer: `url URL`, then `sha256 HEX` when the hash
/// is pinned. Nothing else; a version-1 line has no trailer at all.
fn trailer_of(trailer: &[&[u8]]) -> Result<Option<Fetch>, Skip> {
    match trailer {
        [] => Ok(None),
        [b"url", url] => Ok(Some(Fetch {
            url: url_of(url)?,
            sha256: None,
        })),
        [b"url", url, b"sha256", sha] => Ok(Some(Fetch {
            url: url_of(url)?,
            sha256: Some(sha_of(sha)?),
        })),
        _ => Err(Skip::Fields(PICK)),
    }
}

fn url_of(field: &[u8]) -> Result<String, Skip> {
    let url = text(field, "url")?;
    if url.contains('\0') || !crate::fetch::is_url(&url) || url.len() > MAX_URL {
        return Err(Skip::Url);
    }
    Ok(url)
}

fn sha_of(field: &[u8]) -> Result<[u8; 32], Skip> {
    let text = text(field, "sha256")?;
    crate::fetch::parse_sha256(&text).map_err(|_| Skip::Sha)
}

fn color_of(field: &[u8], what: &'static str) -> Result<Color, Skip> {
    std::str::from_utf8(field)
        .ok()
        .and_then(|text| Color::parse(text).ok())
        .ok_or(Skip::Color(what))
}
