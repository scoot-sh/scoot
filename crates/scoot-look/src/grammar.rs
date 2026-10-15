//! The value grammar of `look.toml` (design section 3.5).
//!
//! Every string a look carries is eventually interpolated into a line-based
//! app config, and several of those formats have keys that run code (mako
//! `on-button-*=exec`, fuzzel `launch-prefix`, GTK 3 `gtk-modules`). So one
//! newline in `fonts.ui` would be a key injection, and from there code
//! execution. These checks close that for every string; each renderer also
//! quotes for its own format, and neither half is trusted alone.

use std::fmt;

/// The longest `look.toml` this crate will parse: 64 KiB, scootbar's own
/// file cap (`crates/scootbar/src/config.rs:77-79`).
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// The longest `wallpaper.image` URL: printable ASCII, section 3.5.
pub const MAX_URL_BYTES: usize = 2048;

/// A single grammar refusal, naming the field and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError {
    /// Dotted field path, e.g. `fonts.ui` or `wallpaper.image`.
    pub field: String,
    /// What was wrong with it.
    pub reason: String,
}

impl GrammarError {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

fn has_nul_or_control(s: &str) -> bool {
    s.chars().any(|c| c == '\0' || c.is_control())
}

/// Exactly `#rrggbb` (section 3.5). No `#rgb`, no alpha: every consumer of a
/// look token takes six hex digits.
pub fn is_hex_color(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

/// `[cursor] accent` takes `#rrggbb` or `"none"`.
pub fn is_cursor_accent(s: &str) -> bool {
    s == "none" || is_hex_color(s)
}

/// Names (`name`, `extends`): `[a-z0-9-]{1,32}`.
pub fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Identifiers (font families, cursor and icon theme names): ASCII letters,
/// digits, space and `-_.+`, 1 to 64 bytes, starting with a letter or digit
/// (so never `.`, `..` or a hidden name: a theme named `..` would resolve to
/// the parent of `~/.icons`). ASCII-only is deliberate in v1: a font family
/// whose name is not Latin script is not expressible, and a look that needs
/// one carries the file in `[fonts.files]`.
pub fn is_identifier(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 64 {
        return false;
    }
    if !(b[0].is_ascii_alphanumeric()) {
        return false;
    }
    b.iter().all(|c| {
        c.is_ascii_alphanumeric()
            || *c == b' '
            || *c == b'-'
            || *c == b'_'
            || *c == b'.'
            || *c == b'+'
    })
}

/// Nix attribute paths (`[fonts.nix]`): one to eight segments joined by `.`,
/// each `[A-Za-z_][A-Za-z0-9_'-]{0,63}`. Resolved only with `lib.attrByPath`
/// against `pkgs`, never evaluated as an expression.
pub fn is_nix_attr_path(s: &str) -> bool {
    if s.is_empty() || s.len() > 8 * 64 + 7 {
        return false;
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() > 8 {
        return false;
    }
    segments.iter().all(|seg| {
        let b = seg.as_bytes();
        !b.is_empty()
            && b.len() <= 64
            && (b[0].is_ascii_alphabetic() || b[0] == b'_')
            && b.iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'\'' || *c == b'-')
    })
}

/// Display text (`title`, `description`, `credits`, `license`): UTF-8 without
/// control characters (C0, DEL, C1), no `=`, `[`, `]`, `"`, `\`. Display text
/// is never written into an app config.
pub fn check_display_text(field: &str, value: &str, max_bytes: usize) -> Option<GrammarError> {
    if value.len() > max_bytes {
        return Some(GrammarError::new(
            field,
            format!("longer than {max_bytes} bytes"),
        ));
    }
    if value
        .chars()
        .any(|c| c == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&c))
        || has_nul_or_control(value)
    {
        return Some(GrammarError::new(
            field,
            "must not contain control characters",
        ));
    }
    if value
        .chars()
        .any(|c| matches!(c, '=' | '[' | ']' | '"' | '\\'))
    {
        return Some(GrammarError::new(
            field,
            "must not contain `=`, `[`, `]`, `\"` or `\\`",
        ));
    }
    None
}

/// Glyph strings (allowlisted module glyph keys): 1 to 8 Unicode scalar
/// values, no control or format characters, no ASCII punctuation.
pub fn check_glyph(field: &str, value: &str) -> Option<GrammarError> {
    let count = value.chars().count();
    if count == 0 || count > 8 {
        return Some(GrammarError::new(field, "glyph must be 1 to 8 characters"));
    }
    if value.chars().any(|c| {
        c.is_control()
            || c.is_ascii_punctuation()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
    }) {
        return Some(GrammarError::new(
            field,
            "glyph must not contain control, format or ASCII punctuation characters",
        ));
    }
    None
}

/// Paths (`wallpaper.image`, `preview`, `fonts.files.*`, `apps/*`): relative,
/// normalized, no `..` component, no leading `/`. Whether the path resolves
/// to a regular file inside the look directory after symlink resolution is
/// an effectful check and lives in the caller, not here.
pub fn check_relative_path(field: &str, value: &str) -> Option<GrammarError> {
    if value.is_empty() || value.len() > 512 {
        return Some(GrammarError::new(field, "path must be 1 to 512 bytes"));
    }
    if has_nul_or_control(value) {
        return Some(GrammarError::new(
            field,
            "path must not contain control characters",
        ));
    }
    if value.starts_with('/') {
        return Some(GrammarError::new(
            field,
            "path must be relative, not absolute",
        ));
    }
    if value.split('/').any(|c| c == "..") {
        return Some(GrammarError::new(field, "path must not contain `..`"));
    }
    // Normalized: no `.` components, no empty segments from `//` or a
    // trailing `/`, no leading `./`. Two spellings of one file must never
    // compare unequal downstream.
    if value.split('/').any(|c| c.is_empty() || c == ".") {
        return Some(GrammarError::new(
            field,
            "path must be normalized: no `.`, `//` or trailing `/`",
        ));
    }
    if value.contains('\\') {
        return Some(GrammarError::new(field, "path must not contain `\\`"));
    }
    None
}

/// Suffixes the trust boundary accepts for look files (design section 8.3):
/// `look.toml`, `preview.png`, image wallpapers, bundled fonts, the five
/// named `apps/` leaves, `LICENSE*`, `COPYING*`, `README.md`.
pub fn is_permitted_look_file(name: &str) -> bool {
    if name == "look.toml" || name == "preview.png" || name == "README.md" {
        return true;
    }
    if name == "LICENSE" || name.starts_with("LICENSE.") || name.starts_with("LICENSE-") {
        return true;
    }
    if name == "COPYING" || name.starts_with("COPYING.") {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".ttf")
        || lower.ends_with(".otf")
    {
        return check_relative_path("file", name).is_none();
    }
    false
}

/// Wallpaper images only: `is_permitted_look_file` also passes
/// `look.toml`, `preview.png`, `README.md`, `LICENSE*`/`COPYING*` and
/// bundled `.ttf`/`.otf` fonts, none of which decodes as a wallpaper
/// (round 2 finding 7). Suffixes match the refusal message both
/// callers print.
pub fn is_image_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif"))
        && check_relative_path("file", name).is_none()
}

/// What is wrong with a wallpaper URL, if anything (section 3.5, findings
/// F2/R5). `https://` only, at most 2048 bytes, printable ASCII without
/// space, `"`, `'`, `\`, `<`, `>` or a backtick, and never `?`, `#` or `%`
/// nor any control character; the host must be a DNS name with at least one
/// dot: no userinfo, no IP literal (dotted, decimal, octal or hex IPv4, or
/// bracketed or bare IPv6), no explicit port, not `localhost` or a name
/// under `.localhost`/`.local`/`.internal`/`.home.arpa`. The host is
/// lowercased and one trailing dot stripped before those checks, and a host
/// whose last label is all digits or starts with `0x` is refused, because
/// curl and the WHATWG URL parser read `127.1` and `0x7f.0.0.1` as IPv4.
/// IDNs and punycode are fine given the hash pin.
pub fn check_wallpaper_url(field: &str, url: &str) -> Option<GrammarError> {
    let refuse = |reason: &str| Some(GrammarError::new(field, reason));
    if url.len() > MAX_URL_BYTES {
        return refuse("URL longer than 2048 bytes");
    }
    let rest = match url.strip_prefix("https://") {
        Some(rest) => rest,
        None => return refuse("wallpaper URL must start with `https://`"),
    };
    // Printable ASCII only (`0x21..=0x7E`): this refuses spaces, DEL,
    // non-ASCII and every control character outright.
    if !url.bytes().all(|c| (0x21..=0x7E).contains(&c)) {
        return refuse("URL must be printable ASCII with no spaces or control characters");
    }
    if url.bytes().any(|c| c == b'?' || c == b'#' || c == b'%') {
        return refuse("URL must not contain `?`, `#` or `%`");
    }
    if url
        .bytes()
        .any(|c| c == b'"' || c == b'\'' || c == b'\\' || c == b'<' || c == b'>' || c == b'`')
    {
        return refuse(
            "URL must be printable ASCII without space, `\"`, `'`, `\\`, `<`, `>` or backtick",
        );
    }
    // The authority ends at the first `/` (`?` and `#` cannot occur: they
    // are refused above, and splitting on them too keeps the parse honest
    // if that ever changes).
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or("");
    if authority.contains('@') {
        return refuse("URL must not contain userinfo");
    }
    if host.is_empty() {
        return refuse("URL has an empty host");
    }
    // No explicit port: a colon is only legal inside a bracketed IPv6
    // literal, and those are refused below anyway.
    let is_bracketed = host.starts_with('[') && host.ends_with(']');
    if !is_bracketed && host.contains(':') {
        return refuse("URL must not name a port");
    }
    if is_bracketed {
        return refuse("URL must be a DNS name, not an IP literal");
    }
    let mut host = host.to_ascii_lowercase();
    if host.ends_with('.') {
        host.pop();
    }
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".home.arpa")
    {
        return refuse("URL must not point at localhost or a local name");
    }
    if !host.contains('.') {
        return refuse("URL host must be a DNS name with at least one dot");
    }
    if host.split('.').any(|label| label.is_empty()) {
        return refuse("URL host has an empty label");
    }
    if is_dotted_ipv4(&host) {
        return refuse("URL must be a DNS name, not an IP literal");
    }
    // curl and the WHATWG URL parser read `127.1` and `0x7f.0.0.1` as IPv4.
    if let Some(last) = host.split('.').next_back()
        && !last.is_empty()
        && (last.bytes().all(|c| c.is_ascii_digit()) || last.starts_with("0x"))
    {
        return refuse("URL host looks like a numeric IP address");
    }
    None
}

fn is_dotted_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|c| c.is_ascii_digit()))
}

/// A `sha256` pin: exactly 64 hex digits.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;
