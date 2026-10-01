//! What the payload fuzz target checks, written once: `crates/scootbar/fuzz`
//! compiles this file by `#[path]` (it uses only [`super`]'s `Format`,
//! `Shown`, `parse_line`, `from_value` and `MAX_TEXT`, which that crate
//! re-exports at its root as scootbar does here), and the stable test beside
//! it replays the committed seed corpus and every past finding through the
//! same function on every `cargo test`. A panic is a finding: the bar's
//! release profile is `panic = "abort"`.

use super::{Format, MAX_TEXT, Shown, from_value, parse_line};

#[cfg(test)]
mod tests;

/// Any bytes as an exec line (in both formats) and, where they are JSON, as
/// a `set` value: refused or accepted without a panic; an accepted update
/// holds text and tooltip within the bound, with no control character and
/// no space at either end; a refused one changes nothing.
pub fn payload(data: &[u8]) {
    for format in [Format::Text, Format::Json] {
        let mut shown = Shown::text("kept");
        match parse_line(format, data, &mut shown) {
            Ok(()) => printable(&shown),
            Err(_) => assert_eq!(shown, Shown::text("kept"), "a refused line changed it"),
        }
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let mut shown = Shown::text("kept");
        match from_value(&value, &mut shown) {
            Ok(()) => printable(&shown),
            Err(_) => assert_eq!(shown, Shown::text("kept"), "a refused value changed it"),
        }
    }
}

fn printable(shown: &Shown) {
    for text in [&shown.text, &shown.tooltip] {
        assert!(text.len() <= MAX_TEXT, "{text:?}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert_eq!(text.trim(), text, "{text:?}");
    }
}
