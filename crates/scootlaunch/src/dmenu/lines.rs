//! The dmenu stdin splitter: any bytes into bounded lines.
//!
//! This file is `std`-only, so the fuzz target compiles it by `#[path]`
//! with nothing else. Bounds (also in `dev/design/scootlaunch.md`, The
//! dmenu contract): stdin is at most [`MAX_BYTES`], at most
//! [`MAX_LINES`] lines, each at most [`MAX_LINE_BYTES`] (longer lines
//! are cut at a character boundary). Non-UTF-8 input is decoded
//! lossily; NUL bytes ride through as text (there is no icon protocol
//! in v1, so `foo\0icon\x1fbar` is one literal line). An empty input is
//! no lines; a lone trailing newline is one empty line.

/// At most this many stdin bytes are kept: past it the rest is
/// dropped, never allocated.
pub const MAX_BYTES: usize = 1024 * 1024;
/// At most this many lines are kept: past it the rest is dropped.
pub const MAX_LINES: usize = 65_536;
/// At most this many bytes per line: longer lines are cut at the last
/// character boundary that fits.
pub const MAX_LINE_BYTES: usize = 4096;

/// Splits `bytes` into lines: lossy UTF-8, one line per `\n`, a
/// trailing `\r` stripped per line (a lone `\r` stays text). A final
/// `\n` terminates the last line (no extra empty line); the final line
/// without one is still kept. Empty input is no lines. Lines past
/// [`MAX_LINES`] are dropped; lines past [`MAX_LINE_BYTES`] are cut at
/// a boundary. Never panics: cuts floor, indexes are checked.
pub fn split_input(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(bytes);
    // A final `\n` terminates the last line (no extra empty line).
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if lines.len() >= MAX_LINES {
            break;
        }
        lines.push(cut_line(line.strip_suffix('\r').unwrap_or(line)));
    }
    lines
}

/// Cuts `line` to [`MAX_LINE_BYTES`] at a character boundary. Short
/// lines are returned whole (reborrowed, never copied).
fn cut_line(line: &str) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line.to_string();
    }
    let mut end = MAX_LINE_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

#[cfg(test)]
mod tests;
