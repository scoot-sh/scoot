//! Telling `install PATH` from `install GIT-URL`, and refusing every URL
//! that is not a plain `https://` or `ssh://` one.
//!
//! The split is textual and never touches the filesystem, so a directory
//! cannot change what a word means. The rules apply in this order:
//!
//! - a word starting with `/`, `./` or `../` is a path, whatever it
//!   contains. No URL, remote shorthand or option starts that way, so this
//!   is the one way to spell a local path that holds `://` or `:`;
//! - otherwise a word containing `://` is a URL, always. Its scheme must be
//!   exactly `https` or `ssh` (lowercase); `http`, `git`, `file`, `ftp` and
//!   the rest refuse. It never falls through to the PATH branch;
//! - a word with no `://` whose first `:` comes before its first `/` is a
//!   git remote shorthand (`host:path`, `user@host:path`, `ext::cmd`,
//!   `transport::addr`) and refuses, since git would run a remote helper
//!   or open ssh for it. A local path with a colon spells `./a:b`;
//! - anything else is a path.
//!
//! A URL is checked field by field rather than with a general URL parser,
//! because git, ssh and curl each parse their own way and the only safe
//! input is one every parser reads identically: ASCII with no control
//! characters, whitespace or backslashes, no query or fragment, a host of
//! letters, digits, dots and hyphens that starts with neither, an optional
//! numeric port, and a non-empty repository path. A user name is allowed
//! on `ssh://` (a bare `git@host`) and never on `https://`; a password is
//! never allowed, so scoot never sees or stores a credential. The `--`
//! scoot puts before the URL does not cover `ssh://-oProxyCommand=...@h/`
//! style hosts and users (ssh reads those as options), which is why a
//! leading hyphen is refused in both.

/// The longest URL accepted.
const MAX_URL_BYTES: usize = 2048;

/// What an `install` argument names.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// A directory or tarball on disk.
    Path,
    /// A validated `https://` or `ssh://` URL, safe to hand to `git clone`
    /// after a `--`.
    Git(GitUrl),
}

/// A URL that passed [`classify`]. The only way to build one.
#[derive(Debug, PartialEq, Eq)]
pub struct GitUrl(String);

impl GitUrl {
    /// Skips validation so tests can aim the runner at a `file://`
    /// repository; production builds one only through [`classify`].
    #[cfg(test)]
    pub fn for_test(url: &str) -> Self {
        Self(url.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The URL with any user name dropped, for messages. Passwords never
    /// pass validation, so this is a courtesy, not a secret filter.
    pub fn display(&self) -> String {
        match self.0.split_once("://") {
            Some((scheme, rest)) => {
                let (authority, tail) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
                let host = authority.rsplit('@').next().unwrap_or(authority);
                format!("{scheme}://{host}{tail}")
            }
            None => self.0.clone(),
        }
    }
}

/// Sorts `word` into a path or a git URL, refusing every other shape.
pub fn classify(word: &str) -> Result<Source, String> {
    if word.starts_with('-') {
        let shown = printable(word, 40);
        return Err(format!(
            "refused: `{shown}` looks like an option: spell a path starting with `-` as ./{shown}"
        ));
    }
    if word.starts_with('/') || word.starts_with("./") || word.starts_with("../") {
        return Ok(Source::Path);
    }
    if let Some((scheme, rest)) = word.split_once("://") {
        return check_url(word, scheme, rest).map(|()| Source::Git(GitUrl(word.to_owned())));
    }
    let colon = word.find(':');
    let slash = word.find('/');
    if let Some(colon) = colon
        && slash.is_none_or(|slash| colon < slash)
    {
        return Err(format!(
            "refused: `{}` is a git remote shorthand (host:path or a transport helper such as ext::): \
             only https:// and ssh:// URLs install from git; spell a local path ./a:b",
            printable(&word[..colon], 40)
        ));
    }
    Ok(Source::Path)
}

fn check_url(word: &str, scheme: &str, rest: &str) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("refused: git URL: {why}"));
    if scheme != "https" && scheme != "ssh" {
        return Err(format!(
            "refused: `{}://` is not installable: only https:// and ssh:// URLs install from git \
             (a local path starts with /, ./ or ../)",
            printable(scheme, 20)
        ));
    }
    if word.len() > MAX_URL_BYTES {
        return refuse("longer than 2048 bytes");
    }
    if let Some(bad) = word
        .chars()
        .find(|c| !c.is_ascii() || c.is_ascii_control() || c.is_whitespace() || *c == '\\')
    {
        return refuse(&format!(
            "contains {}: URLs are ASCII with no spaces, control characters or backslashes \
             (an international host spells in punycode)",
            if bad.is_ascii() {
                format!("{bad:?}")
            } else {
                "a non-ASCII character".to_owned()
            }
        ));
    }
    if rest.contains(['?', '#']) {
        return refuse("carries a query or fragment");
    }
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if path.trim_start_matches('/').is_empty() {
        return refuse("names no repository path");
    }
    let (user, hostport) = match authority.rsplit_once('@') {
        Some((user, hostport)) => (Some(user), hostport),
        None => (None, authority),
    };
    if let Some(user) = user {
        if scheme == "https" {
            return refuse(
                "https:// takes no user name: scoot never handles credentials \
                 (a private repository installs over ssh://)",
            );
        }
        if user.contains(':') {
            return refuse("carries a password: scoot never handles credentials");
        }
        let ok = !user.is_empty()
            && !user.starts_with('-')
            && user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !ok {
            return refuse(
                "the ssh user name is empty, starts with `-` or holds characters outside a-z 0-9 . _ -",
            );
        }
    }
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (hostport, None),
    };
    if host.starts_with('[') {
        return refuse("a bracketed IPv6 host is not supported: use a host name");
    }
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
    if !host_ok {
        return refuse(&format!(
            "the host `{}` is empty, starts with `-` or `.`, or holds characters outside a-z 0-9 . -",
            printable(host, 60)
        ));
    }
    if let Some(port) = port {
        let valid = (1..=5).contains(&port.len())
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|n| n != 0);
        if !valid {
            return refuse(&format!(
                "`{}` is not a port (1 to 65535)",
                printable(port, 10)
            ));
        }
    }
    Ok(())
}

/// Whether `c` can steer or deceive a terminal or a reader: a control
/// character, or a Unicode format character (bidi overrides and isolates,
/// zero-width and joiner characters, the BOM, soft hyphen, interlinear
/// annotation and tag characters).
pub(super) fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{ad}'
                | '\u{61c}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// `c` itself, or `?` when [`is_unsafe_char`].
pub(super) fn sanitize(c: char) -> char {
    if is_unsafe_char(c) { '?' } else { c }
}

/// `text` cut to `max` characters with every unsafe character written as
/// its Rust escape (`\n`, `\u{202e}`), safe to echo into a terminal and
/// never more than one line.
pub(super) fn printable(text: &str, max: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for c in text.chars().take(max) {
        if is_unsafe_char(c) {
            let _ = write!(out, "{}", c.escape_default());
        } else {
            out.push(c);
        }
    }
    if text.chars().count() > max {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests;
