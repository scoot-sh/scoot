//! Hostile stdin through the splitter: huge lines, no trailing
//! newline, invalid UTF-8, NUL, a million lines, CRLF, and the bounds
//! the ranker relies on.

use super::{MAX_BYTES, MAX_LINE_BYTES, MAX_LINES, cut_line, split_input};

#[test]
fn empty_input_is_no_lines() {
    assert!(split_input(b"").is_empty());
}

#[test]
fn lines_split_and_keep_no_newline() {
    assert_eq!(split_input(b"a\nb\nc"), vec!["a", "b", "c"]);
    // A final `\n` terminates the last line: no extra empty line.
    assert_eq!(split_input(b"a\n"), vec!["a"]);
    assert_eq!(split_input(b"\n"), vec![""]);
}

#[test]
fn carriage_returns_strip_per_line() {
    assert_eq!(split_input(b"a\r\nb\r\n"), vec!["a", "b"]);
    assert_eq!(split_input(b"a\rb"), vec!["a\rb"]);
}

#[test]
fn invalid_utf8_decodes_lossily() {
    let lines = split_input(b"ok \xff\xfe\nnext\n");
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("ok "));
    assert!(lines[0].contains('\u{fffd}'));
    assert_eq!(lines[1], "next");
}

#[test]
fn nul_rides_through_as_text() {
    // No icon protocol in v1: `foo\0icon\x1fbar` is one literal line.
    let lines = split_input(b"a\0b\nc\0icon\x1fd\n");
    assert_eq!(lines, vec!["a\0b", "c\0icon\x1fd"]);
}

#[test]
fn a_long_line_cuts_at_a_boundary() {
    let line = format!("{}\u{00e9} tail", "x".repeat(MAX_LINE_BYTES));
    let cut = cut_line(&line);
    assert!(cut.len() <= MAX_LINE_BYTES);
    assert!(cut.is_char_boundary(cut.len()));
    // The multibyte char straddling the cap is dropped whole, never
    // split: the cut ends at a boundary at or under the cap.
    assert!(line.starts_with(&cut));
    // A short line is untouched.
    assert_eq!(cut_line("abc"), "abc");
}

#[test]
fn lines_cap_at_65536() {
    let mut bytes = Vec::new();
    for _ in 0..(65_536 + 1000) {
        bytes.extend_from_slice(b"x\n");
    }
    let lines = split_input(&bytes);
    // The literal contract number, not the constant: a drift that
    // shrinks the cap fails here.
    assert_eq!(lines.len(), 65_536);
    assert!(lines.iter().all(|line| line == "x"));
}

#[test]
fn a_million_lines_stay_bounded() {
    // 1M short lines: the split keeps the first 65,536, never the
    // whole input.
    let mut bytes = Vec::new();
    for index in 0..1_000_000 {
        bytes.extend_from_slice(format!("line-{index}\n").as_bytes());
    }
    assert!(bytes.len() > MAX_BYTES);
    let lines = split_input(&bytes);
    assert_eq!(lines.len(), MAX_LINES);
    assert_eq!(lines[0], "line-0");
    assert_eq!(lines[MAX_LINES - 1], format!("line-{}", MAX_LINES - 1));
}

#[test]
fn one_line_past_a_megabyte_is_cut() {
    let bytes = vec![b'y'; MAX_BYTES + 999];
    let lines = split_input(&bytes);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].len(), MAX_LINE_BYTES);
}
