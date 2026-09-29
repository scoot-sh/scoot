//! Just enough of Rust's lexer to tell whether a test file calls the
//! harness: code split into identifiers and single punctuation
//! characters, with whitespace, comments (line, doc and nested block),
//! string literals (plain, byte, C and raw) and character literals left
//! out, so a mention in any of them is not a call. Not a Rust lexer:
//! numbers come out as identifiers and lifetimes as a `'` and a name,
//! which is all a call's shape needs.

/// Whether `source` calls an associated function of `Harness`
/// (`Harness::new(`, `Harness::start(`, spaced or not) in its code.
pub fn calls_harness(source: &str) -> bool {
    tokens(source)
        .windows(5)
        .any(|w| w[0] == "Harness" && w[1] == ":" && w[2] == ":" && is_ident(w[3]) && w[4] == "(")
}

fn is_ident(token: &str) -> bool {
    token.chars().next().is_some_and(is_ident_char)
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// The code's tokens, in order. Never panics: an unterminated comment or
/// literal runs to the end of the file. Every delimiter it looks for is
/// ASCII, which never occurs inside a multi-byte character, so each slice
/// falls on a character boundary.
pub fn tokens(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = source.get(i..).and_then(|rest| rest.chars().next()) {
        let rest = &bytes[i..];
        if c.is_whitespace() {
            i += c.len_utf8();
        } else if rest.starts_with(b"//") {
            i = bytes[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |n| i + n);
        } else if rest.starts_with(b"/*") {
            i = block_comment_end(bytes, i + 2);
        } else if c == '"' {
            i = string_end(bytes, i + 1);
        } else if c == '\'' {
            i = quote(source, i, &mut out);
        } else if is_ident_char(c) {
            let start = i;
            while let Some(c) = source.get(i..).and_then(|rest| rest.chars().next()) {
                if !is_ident_char(c) {
                    break;
                }
                i += c.len_utf8();
            }
            let word = &source[start..i];
            let next = bytes.get(i).copied();
            match (word, next) {
                ("r" | "br" | "cr", Some(b'"' | b'#')) => i = raw(source, i, &mut out),
                ("b" | "c", Some(b'"')) => i = string_end(bytes, i + 1),
                ("b", Some(b'\'')) => i = quote(source, i, &mut out),
                _ => out.push(word),
            }
        } else {
            out.push(&source[i..i + c.len_utf8()]);
            i += c.len_utf8();
        }
    }
    out
}

/// Past the `*/` closing a block comment opened just before `i`, counting
/// nested ones.
fn block_comment_end(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Past the `"` closing a string whose body starts at `i`, skipping
/// escapes.
fn string_end(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// At a `'` (`i`): past a character literal, or past the `'` alone for a
/// lifetime or a label, pushed as a token.
fn quote<'s>(source: &'s str, i: usize, out: &mut Vec<&'s str>) -> usize {
    let bytes = source.as_bytes();
    let body = i + 1;
    if bytes.get(body) == Some(&b'\\') {
        // An escape: `'\n'`, `'\''`, `'\u{1F600}'`.
        let mut j = body + 2;
        while j < bytes.len() && bytes[j] != b'\'' {
            j += 1;
        }
        return (j + 1).min(bytes.len());
    }
    if let Some(c) = source.get(body..).and_then(|rest| rest.chars().next()) {
        let close = body + c.len_utf8();
        if bytes.get(close) == Some(&b'\'') {
            return close + 1;
        }
    }
    out.push("'");
    body
}

/// Just after a raw prefix (`r`, `br`, `cr`), at `i`: past a raw string,
/// or, for `r#name`, past the raw identifier, pushed as `name`.
fn raw<'s>(source: &'s str, i: usize, out: &mut Vec<&'s str>) -> usize {
    let bytes = source.as_bytes();
    let hashes = bytes[i..].iter().take_while(|&&b| b == b'#').count();
    let open = i + hashes;
    if bytes.get(open) != Some(&b'"') {
        // `r#name`: the identifier itself (a lone `r#` just goes on).
        let start = open;
        let mut end = start;
        while let Some(c) = source.get(end..).and_then(|rest| rest.chars().next()) {
            if !is_ident_char(c) {
                break;
            }
            end += c.len_utf8();
        }
        if end > start {
            out.push(&source[start..end]);
        }
        return end.max(open);
    }
    let mut j = open + 1;
    while j < bytes.len() {
        if bytes[j] == b'"' && bytes[j + 1..].iter().take_while(|&&b| b == b'#').count() >= hashes {
            return j + 1 + hashes;
        }
        j += 1;
    }
    bytes.len()
}
