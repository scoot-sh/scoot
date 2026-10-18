//! Exec: from a desktop entry to an argv. Desktop Entry §7, applied
//! after the string escapes ("the general escape rule … is applied
//! before the quoting rule").
//!
//! 1. Split into arguments on unquoted spaces. A double-quoted
//!    argument unescapes `\" \$ \\ \``. Single quotes have no meaning
//!    (reserved characters must be inside double quotes, so an
//!    unquoted reserved character makes the entry invalid); a lenient
//!    mode accepts what GLib accepts, because real files break this
//!    rule.
//! 2. Field codes, outside quotes only: `%f %F %u %U` are removed;
//!    `%i` becomes `--icon <Icon>` as two arguments, or nothing when
//!    `Icon` is empty; `%c` becomes the translated `Name`; `%k`
//!    becomes the desktop file's path; `%%` becomes `%`; the
//!    deprecated `%d %D %n %N %v %m` are removed; any other `%x` makes
//!    the entry invalid. A field code inside a quoted argument is
//!    "undefined" by the spec: only `%%` is expanded there, and
//!    anything else makes the entry invalid.
//! 3. Field codes expand once: an expansion is one argument whatever
//!    it contains, and is never re-split or re-scanned. The
//!    executable (the first expanded argument) may not contain `=`
//!    (§7): checked after removal, so removed codes cannot promote a
//!    later `=` argument into the executable. If it does, the entry
//!    is invalid.
//!
//! `Terminal=true` prefixes the configured terminal argv (default
//! `["foot"]`); the caller passes `terminal`. `DBusActivatable` is
//! not this file's concern (it is recorded on the entry).
//!
//! This file is `std`-only, so the fuzz target can compile it by
//! `#[path]` with nothing else.

#[cfg(test)]
mod tests;

/// The default terminal argv prefixed for `Terminal=true`. The
/// desktop profile sets its own terminal; the user's file overrides
/// both (a later ticket's `launch.toml`).
pub const DEFAULT_TERMINAL: &str = "foot";

/// What the field codes expand from: the translated `Name`, the
/// `Icon` value (`%i` only), and the desktop file's own path (`%k`).
pub struct Expansion<'a> {
    pub icon: Option<&'a str>,
    pub name: &'a str,
    pub desktop_path: &'a str,
}

/// Why an `Exec` value cannot become an argv. Every variant drops the
/// entry: §7 says such command lines "are invalid and must not be
/// processed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No arguments at all (empty value, or only spaces).
    Empty,
    /// A `\` escape inside double quotes that is not one of the four
    /// (`\" \$ \\ \``), under the strict rule.
    QuotedEscape(char),
    /// An unquoted reserved character under the strict rule
    /// (strict: `'` `\` `>` `<` `~` `|` `&` `;` `$` `*` `?` `#` `(`
    /// `)` and backtick; `"` quotes, space and tab separate).
    UnquotedReserved(char),
    /// A quote never closed, in either mode.
    UnclosedQuote,
    /// The executable (argv[0]) contains `=`.
    EqualsInExecutable,
    /// A field code the spec does not list, outside quotes.
    UnknownFieldCode(char),
    /// Any field code but `%%` inside a quoted segment.
    QuotedFieldCode(char),
    /// A `%` with no code after it.
    TrailingPercent,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Empty => write!(f, "Exec has no arguments"),
            Error::QuotedEscape(char) => write!(f, "bad escape `\\{char}` in quotes in Exec"),
            Error::UnquotedReserved(char) => {
                write!(
                    f,
                    "unquoted reserved character `{char}` in Exec (try --lenient)"
                )
            }
            Error::UnclosedQuote => write!(f, "unclosed quote in Exec"),
            Error::EqualsInExecutable => write!(f, "executable contains `=` in Exec"),
            Error::UnknownFieldCode(char) => write!(f, "unknown field code `%{char}` in Exec"),
            Error::QuotedFieldCode(char) => {
                write!(f, "field code `%{char}` inside quotes in Exec")
            }
            Error::TrailingPercent => write!(f, "trailing `%` in Exec"),
        }
    }
}

/// Phase 1 for `Exec`: the general escapes (`\s \n \t \r \\`)
/// convert; anything else rides through verbatim (backslash kept) for
/// the quoting phase to judge.
fn unescape_exec(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
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
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// One piece of one argument: quoted pieces expand only `%%`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    text: String,
    quoted: bool,
    /// Single quotes (lenient only): literal, no escapes, no codes.
    literal: bool,
}

/// Expands an `Exec` value to an argv. `terminal` prefixes the
/// default terminal argv for `Terminal=true`; `lenient` accepts what
/// GLib accepts for quoting (single quotes group, unquoted reserved
/// characters and backslash-escapes pass through). Field codes stay
/// strict in both modes: the must-reject cases reject everywhere.
pub fn expand(
    exec: &str,
    expansion: &Expansion<'_>,
    terminal: bool,
    lenient: bool,
) -> Result<Vec<String>, Error> {
    // Phase 1: the general escape rule, before the quoting rule. The
    // known escapes convert; anything else rides through verbatim for
    // the quoting phase, where `\"` becomes a quote and a stray `\`
    // is refused (strict) or literal (lenient). Failing phase 1 on an
    // unknown escape would make `\"` unreachable, and the quoting
    // rule below names it explicitly.
    let unescaped = unescape_exec(exec);
    // Phase 2: split on unquoted spaces.
    let args = split(&unescaped, lenient)?;
    if args.is_empty() {
        return Err(Error::Empty);
    }
    // Phase 4: field codes, once, never re-split or re-scanned.
    let mut argv = Vec::with_capacity(args.len());
    if terminal {
        argv.push(DEFAULT_TERMINAL.to_string());
    }
    for arg in &args {
        expand_arg(arg, expansion, &mut argv)?;
    }
    if argv.is_empty() || (terminal && argv.len() == 1) {
        return Err(Error::Empty);
    }
    // The executable may not contain `=` (§7): checked on the
    // expanded argv, because field-code removal can drop the whole
    // first argument and promote a later one (`%f =foo` executes
    // `=foo`). A `%c`/`%k` expansion injecting `=` refuses the same
    // way: a strange name must not mint a strange executable.
    if argv[usize::from(terminal)].contains('=') {
        return Err(Error::EqualsInExecutable);
    }
    Ok(argv)
}

/// Splits into arguments: one `Vec<Segment>` per argument. Quoted
/// segments remember they were quoted; lenient single-quoted
/// segments remember they are literal.
fn split(exec: &str, lenient: bool) -> Result<Vec<Vec<Segment>>, Error> {
    let mut args: Vec<Vec<Segment>> = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut buf = String::new();
    // What `buf` is: none (no open segment), double-quoted, single-
    // quoted (lenient), or unquoted.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        None,
        Double,
        Single,
        Bare,
    }
    let mut mode = Mode::None;
    let mut chars = exec.chars().peekable();
    let flush = |buf: &mut String, mode: Mode, segments: &mut Vec<Segment>| {
        let (quoted, literal) = match mode {
            Mode::Double => (true, false),
            Mode::Single => (true, true),
            _ => (false, false),
        };
        segments.push(Segment {
            text: std::mem::take(buf),
            quoted,
            literal,
        });
    };
    while let Some(char) = chars.next() {
        match mode {
            Mode::Double => match char {
                '"' => {
                    flush(&mut buf, Mode::Double, &mut segments);
                    mode = Mode::None;
                }
                '\\' => match chars.next() {
                    Some(next @ ('"' | '$' | '\\' | '`')) => buf.push(next),
                    Some(next) => {
                        if lenient {
                            // What GLib leaves alone stays alone.
                            buf.push('\\');
                            buf.push(next);
                        } else {
                            return Err(Error::QuotedEscape(next));
                        }
                    }
                    None => {
                        if lenient {
                            buf.push('\\');
                        } else {
                            return Err(Error::UnclosedQuote);
                        }
                    }
                },
                _ => buf.push(char),
            },
            Mode::Single => match char {
                // Lenient single quotes group literally: no escapes,
                // no field codes, closed only by another `'`.
                '\'' => {
                    flush(&mut buf, Mode::Single, &mut segments);
                    mode = Mode::None;
                }
                _ => buf.push(char),
            },
            Mode::Bare | Mode::None => {
                let bare = mode == Mode::Bare;
                match char {
                    ' ' | '\t' => {
                        if bare {
                            flush(&mut buf, Mode::Bare, &mut segments);
                        }
                        if !segments.is_empty() {
                            args.push(std::mem::take(&mut segments));
                        }
                        mode = Mode::None;
                    }
                    '"' => {
                        if bare {
                            flush(&mut buf, Mode::Bare, &mut segments);
                        }
                        mode = Mode::Double;
                    }
                    '\'' => {
                        if !lenient {
                            return Err(Error::UnquotedReserved('\''));
                        }
                        if bare {
                            flush(&mut buf, Mode::Bare, &mut segments);
                        }
                        mode = Mode::Single;
                    }
                    '\\' => {
                        if !lenient {
                            return Err(Error::UnquotedReserved('\\'));
                        }
                        // A backslash quotes the next character,
                        // whatever it is; a trailing one stays.
                        match chars.next() {
                            Some(next) => {
                                buf.push(next);
                                mode = Mode::Bare;
                            }
                            None => {
                                buf.push('\\');
                                mode = Mode::Bare;
                            }
                        }
                    }
                    reserved if !lenient && is_reserved(reserved) => {
                        return Err(Error::UnquotedReserved(reserved));
                    }
                    _ => {
                        buf.push(char);
                        mode = Mode::Bare;
                    }
                }
            }
        }
    }
    match mode {
        Mode::Double | Mode::Single => return Err(Error::UnclosedQuote),
        Mode::Bare => flush(&mut buf, Mode::Bare, &mut segments),
        Mode::None => {}
    }
    if !segments.is_empty() {
        args.push(segments);
    }
    Ok(args)
}

/// The spec's reserved characters: everything that must be inside
/// double quotes (space, tab and `"` are handled by the caller).
fn is_reserved(char: char) -> bool {
    matches!(
        char,
        '\'' | '\\' | '>' | '<' | '~' | '|' | '&' | ';' | '$' | '*' | '?' | '#' | '(' | ')' | '`'
    )
}

/// Expands one argument's field codes into `argv`. A standalone `%i`
/// argument becomes `--icon <Icon>` (two arguments) or nothing; `%i`
/// inside a larger argument expands to the icon in place. An argument
/// of only removed codes vanishes; a literal empty (`""`) stays.
fn expand_arg(
    arg: &[Segment],
    expansion: &Expansion<'_>,
    argv: &mut Vec<String>,
) -> Result<(), Error> {
    if arg.len() == 1 && !arg[0].quoted && arg[0].text == "%i" {
        if let Some(icon) = expansion.icon
            && !icon.is_empty()
        {
            argv.push("--icon".to_string());
            argv.push(icon.to_string());
        }
        return Ok(());
    }
    let mut out = String::new();
    let mut keep_if_empty = false;
    for segment in arg {
        if segment.literal {
            // Lenient single quotes: no codes at all, not even `%%`.
            out.push_str(&segment.text);
            keep_if_empty = true;
            continue;
        }
        if segment.quoted {
            // A quoted segment is literal by construction: `""` stays
            // an empty argument.
            keep_if_empty = true;
            expand_quoted(&segment.text, &mut out)?;
            continue;
        }
        if expand_unquoted(&segment.text, expansion, &mut out)? {
            keep_if_empty = true;
        }
    }
    if !out.is_empty() || keep_if_empty {
        argv.push(out);
    }
    Ok(())
}

/// Field codes inside quotes: only `%%`. Anything else is invalid.
fn expand_quoted(text: &str, out: &mut String) -> Result<(), Error> {
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char != '%' {
            out.push(char);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(other) => return Err(Error::QuotedFieldCode(other)),
            None => return Err(Error::TrailingPercent),
        }
    }
    Ok(())
}

/// Field codes outside quotes: removals, `%i`, `%c`, `%k`, `%%`, the
/// deprecated removals, and nothing else. Answers whether any literal
/// content was seen (a normal character, or a code that is not a
/// removal): an argument of only removed codes vanishes, while a
/// literal empty stays.
fn expand_unquoted(text: &str, expansion: &Expansion<'_>, out: &mut String) -> Result<bool, Error> {
    let mut literal = false;
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char != '%' {
            out.push(char);
            literal = true;
            continue;
        }
        match chars.next() {
            // Removed: the launcher opens apps, not files.
            Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
            Some('i') => {
                literal = true;
                if let Some(icon) = expansion.icon
                    && !icon.is_empty()
                {
                    out.push_str(icon);
                }
            }
            Some('c') => {
                literal = true;
                out.push_str(expansion.name);
            }
            Some('k') => {
                literal = true;
                out.push_str(expansion.desktop_path);
            }
            Some('%') => {
                literal = true;
                out.push('%');
            }
            Some(other) => return Err(Error::UnknownFieldCode(other)),
            None => return Err(Error::TrailingPercent),
        }
    }
    Ok(literal)
}
