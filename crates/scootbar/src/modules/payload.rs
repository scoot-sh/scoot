//! The update payload: what a module shows when something outside the bar
//! says so. `push` takes it as the value of `scootbar msg set ID VALUE`,
//! and an `exec` module takes one per line of its command's output (text,
//! or one JSON object per line).
//!
//! ## The shape, version 1
//!
//! ```json
//! {"version": 1, "text": "72%", "class": "warn", "tooltip": "battery low"}
//! ```
//!
//! Every key is optional. `text` and `tooltip` are strings, `class` one of
//! `normal`, `warn`, `urgent` or `muted` (what the bar colors it with, its
//! theme's tokens), `version` the shape this was written for. **It is
//! scootbar's own and deliberately not Waybar's**: no `alt`, `percentage`
//! or `class` arrays, nothing to translate. Keys it does not know are
//! ignored, so a later version can add some; a `version` it does not
//! speak (higher than [`VERSION`]) is refused by name rather than half
//! understood. As the value of `msg set`, a JSON string alone is the text
//! (`scootbar msg set weather '"sunny"'`) and `null` clears the module.
//!
//! ## Bounds
//!
//! A line or value is at most [`MAX_PAYLOAD`] bytes, the JSON at most
//! [`MAX_DEPTH`] levels deep; the text and the tooltip are cut at
//! [`MAX_TEXT`] bytes (on a character boundary) and every control
//! character in them (a tab, a carriage return, an escape) becomes a space,
//! so nothing but printable text reaches the bar. Every refusal is a
//! named [`Invalid`], never a panic and never a half-applied update.

use std::fmt;

use serde_json::Value;

use super::{Class, MAX_TEXT};

#[cfg(test)]
mod fuzz;
#[cfg(test)]
mod tests;

/// The shape this bar speaks: `"version": 1`.
pub const VERSION: u64 = 1;

/// The longest update line or `set` value taken, in bytes.
pub const MAX_PAYLOAD: usize = 4096;

/// The deepest JSON nesting read (`serde_json`'s own limit is 128; a
/// payload has two levels, so far less is plenty).
pub const MAX_DEPTH: usize = 8;

/// How an `exec` module's lines are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Each line is the text.
    #[default]
    Text,
    /// Each line is one JSON object of the shape above.
    Json,
}

impl Format {
    /// The value in the config file: `text` or `json`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// What a module shows, as an update last set it. Small and bounded: the
/// text and the tooltip are each at most [`MAX_TEXT`] bytes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub class: Class,
    pub tooltip: String,
}

impl Shown {
    /// Shows `text` alone, as a placeholder does.
    pub fn text(text: &str) -> Self {
        let mut shown = Self::default();
        set_text(&mut shown.text, text);
        shown
    }
}

/// Why an update was refused: said on stderr (an exec line) or in the
/// reply to the agent (`msg set`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// Longer than [`MAX_PAYLOAD`].
    TooLong,
    /// Not JSON, or nested past [`MAX_DEPTH`]: the parser's own words.
    Json(String),
    /// Valid JSON that is not a string, an object or (for `set`) `null`.
    NotAnObject,
    /// A key of the wrong type: the key, and what it takes.
    Field(&'static str, &'static str),
    /// A `class` that is none of the four.
    Class(String),
    /// A `version` newer than [`VERSION`], or not a whole number.
    Version(String),
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(f, "longer than {MAX_PAYLOAD} bytes"),
            Self::Json(why) => write!(f, "not valid JSON: {why}"),
            Self::NotAnObject => write!(f, "takes a JSON object (or a string for the text alone)"),
            Self::Field(key, takes) => write!(f, "`{key}` takes {takes}"),
            Self::Class(class) => write!(
                f,
                "`class` takes normal, warn, urgent or muted, not `{}`",
                class.escape_debug()
            ),
            Self::Version(got) => write!(
                f,
                "`version` {got} is not one this bar speaks (it speaks up to {VERSION})"
            ),
        }
    }
}

/// Replaces `into` with `text`, cut at [`MAX_TEXT`] bytes on a character
/// boundary, every control character a space, the ends trimmed.
fn set_text(into: &mut String, text: &str) {
    into.clear();
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if into.len() + c.len_utf8() > MAX_TEXT {
            break;
        }
        into.push(c);
    }
    let kept = into.trim_end().len();
    into.truncate(kept);
    let lead = into.len() - into.trim_start().len();
    if lead > 0 {
        into.drain(..lead);
    }
}

/// One line of an `exec` module's output (without its newline) read as
/// `format` into `into`. On `Err`, `into` is untouched.
pub fn parse_line(format: Format, line: &[u8], into: &mut Shown) -> Result<(), Invalid> {
    if line.len() > MAX_PAYLOAD {
        return Err(Invalid::TooLong);
    }
    match format {
        Format::Text => {
            // A line that is not UTF-8 shows its valid parts: a script's
            // output is not the bar's to refuse over one stray byte.
            let text = String::from_utf8_lossy(line);
            set_text(&mut into.text, &text);
            into.tooltip.clear();
            into.class = Class::Normal;
            Ok(())
        }
        Format::Json => {
            let value = parse_json(line)?;
            if !value.is_object() {
                return Err(Invalid::NotAnObject);
            }
            from_value(&value, into)
        }
    }
}

/// A JSON value no deeper than [`MAX_DEPTH`].
fn parse_json(bytes: &[u8]) -> Result<Value, Invalid> {
    // Depth first, by the brackets alone: `serde_json` would build a deep
    // tree (and its recursion limit is far past what a payload needs).
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(Invalid::Json(format!("nested more than {MAX_DEPTH} deep")));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    serde_json::from_slice(bytes).map_err(|error| Invalid::Json(error.to_string()))
}

/// A `scootbar msg set` value, or a parsed JSON line, read into `into`:
/// an object of the shape above, a string (the text alone) or `null`
/// (clears). On `Err`, `into` is untouched.
pub fn from_value(value: &Value, into: &mut Shown) -> Result<(), Invalid> {
    let mut shown = Shown::default();
    match value {
        Value::Null => {}
        Value::String(text) => {
            if text.len() > MAX_PAYLOAD {
                return Err(Invalid::TooLong);
            }
            set_text(&mut shown.text, text);
        }
        Value::Object(map) => {
            if let Some(version) = map.get("version") {
                match version.as_u64() {
                    Some(version) if version <= VERSION => {}
                    _ => return Err(Invalid::Version(version.to_string())),
                }
            }
            if let Some(text) = map.get("text") {
                let Some(text) = text.as_str() else {
                    return Err(Invalid::Field("text", "a string"));
                };
                set_text(&mut shown.text, text);
            }
            if let Some(tooltip) = map.get("tooltip") {
                let Some(tooltip) = tooltip.as_str() else {
                    return Err(Invalid::Field("tooltip", "a string"));
                };
                set_text(&mut shown.tooltip, tooltip);
            }
            if let Some(class) = map.get("class") {
                let Some(class) = class.as_str() else {
                    return Err(Invalid::Field("class", "a string"));
                };
                shown.class =
                    Class::parse(class).ok_or_else(|| Invalid::Class(class.to_owned()))?;
            }
        }
        _ => return Err(Invalid::NotAnObject),
    }
    *into = shown;
    Ok(())
}
