//! The `Notify` hints: the spec's §8 table to a bounded struct.
//!
//! Read are `urgency`, `category`, `desktop-entry`, `image-data`,
//! `image-path`, `transient`, `resident`, `suppress-sound` (a no-op) and
//! the de-facto `value` (an int 0 to 100, a progress bar; not in the
//! spec, sent by volume and brightness scripts for mako and dunst).
//! `sound-file`, `sound-name`, `x`, `y` and `action-icons` are ignored.
//! `x-` vendor hints are skipped unread. Anything else unknown is
//! skipped unread too: the spec says to ignore what is not understood.
//! At most [`MAX_HINTS`] hints are read; the rest are skipped unread
//! (their keys still walked, which the framing requires).
//!
//! A hint whose type is not the one read here is type confusion, and the
//! hint is ignored: `urgency` is a byte, `category`/`desktop-entry`/
//! `image-path` strings, `transient`/`resident` booleans, `value` an
//! integer, `image-data` the §5.2 struct. Notably, an `urgency` that is
//! not a byte does not move the urgency off normal.

use scoot_dbus::proto::Reader;

use super::image::{self, Image};

/// The most hints read from one `Notify`: the rest are skipped unread.
pub const MAX_HINTS: usize = 64;

/// A short hint string past this is cut at a char boundary, never
/// refused: `category` and `desktop-entry` are match keys, not prose.
pub const MAX_SHORT_HINT: usize = 128;

/// An `image-path` past this is cut at a char boundary: paths are
/// references, and an unbounded one is a memory hook, not an icon.
pub const MAX_PATH_HINT: usize = 4096;

/// The urgency: the spec's §7 low/normal/critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low,
    /// The default: what missing and confused hints read as.
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    /// The hint byte's meaning: 0 low, 2 critical, anything else (1 or a
    /// stranger) normal.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// The hints one `Notify` carried, bounded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    pub urgency: Urgency,
    pub category: String,
    pub desktop_entry: String,
    /// The validated, downscaled icon, when `image-data` was honest.
    pub image_data: Option<Image>,
    pub image_path: String,
    /// `transient`: never enters history (kept for the history ticket).
    pub transient: bool,
    /// `resident`: the card stays after an invoke (kept for the UI).
    pub resident: bool,
    /// `value`, clamped to 0..=100: the progress bar.
    pub value: Option<u8>,
}

/// Reads the `a{sv}` at `reader`: `Err` refuses the whole `Notify`
/// (framing broke), while any single odd hint is ignored. Never panics.
pub fn read(reader: &mut Reader<'_>) -> Result<Hints, ()> {
    let mut hints = Hints::default();
    let mut elements = reader.elements(8)?;
    let mut read = 0;
    while !elements.exhausted() {
        elements.enter_struct()?;
        let key = elements.str()?.to_owned();
        let known = read < MAX_HINTS && is_known(&key);
        if known {
            read += 1;
            elements.variant(|sig, value| {
                let before = value.remaining();
                apply(&mut hints, &key, sig, value);
                if value.remaining() == before {
                    // Ignored (`suppress-sound`, sound keys, `x`, `y`,
                    // `action-icons`) or type-confused: the value is
                    // skipped unread, but it is still skipped — leaving
                    // it would desync every entry after it.
                    value.skip(sig)?;
                }
                Ok(())
            })?;
        } else {
            // Unknown, or past the cap: skipped unread.
            elements.variant(|sig, value| value.skip(sig))?;
        }
        elements.leave_struct();
    }
    Ok(hints)
}

/// A hint this daemon understands (suppressed sound included: read means
/// skipped over, not stored).
fn is_known(key: &str) -> bool {
    matches!(
        key,
        "urgency"
            | "category"
            | "desktop-entry"
            | "image-data"
            | "image_data"
            | "image-path"
            | "image_path"
            | "transient"
            | "resident"
            | "suppress-sound"
            | "value"
    )
}

/// Applies one known hint's value; type confusion is ignored (the hint
/// keeps its default), never an error.
fn apply(hints: &mut Hints, key: &str, sig: &str, value: &mut Reader<'_>) {
    match key {
        "urgency" if sig == "y" => {
            if let Ok(byte) = value.u8() {
                hints.urgency = Urgency::from_byte(byte);
            }
        }
        "category" if sig == "s" => {
            if let Ok(text) = value.str() {
                hints.category = cut(text, MAX_SHORT_HINT);
            }
        }
        "desktop-entry" if sig == "s" => {
            if let Ok(text) = value.str() {
                hints.desktop_entry = cut(text, MAX_SHORT_HINT);
            }
        }
        // The deprecated 1.1 spellings read as their modern twins.
        "image-data" | "image_data" if sig == "(iiibiiay)" => {
            if let Ok(raw) = image::read(value)
                && let Some(valid) = image::validate(&raw)
            {
                hints.image_data = Some(image::downscale(&valid));
            }
        }
        "image-path" | "image_path" if sig == "s" => {
            if let Ok(text) = value.str() {
                hints.image_path = cut(text, MAX_PATH_HINT);
            }
        }
        "transient" if sig == "b" => {
            if let Ok(flag) = value.boolean() {
                hints.transient = flag;
            }
        }
        "resident" if sig == "b" => {
            if let Ok(flag) = value.boolean() {
                hints.resident = flag;
            }
        }
        // The de-facto progress value: an integer 0 to 100, clamped.
        // `y` sorts with the integers; anything else is confusion.
        "value" if matches!(sig, "y" | "i" | "u") => {
            let number = match sig {
                "y" => value.u8().map(|number| number.min(100)).ok(),
                "i" => value.i32().map(|number| number.clamp(0, 100) as u8).ok(),
                _ => value.u32().map(|number| number.min(100) as u8).ok(),
            };
            if let Some(number) = number {
                hints.value = Some(number);
            }
        }
        // `suppress-sound` (a no-op), `sound-file`, `sound-name`, `x`,
        // `y`, `action-icons` and every unknown key: ignored.
        _ => {}
    }
}

/// Cuts `text` to `max` bytes at a char boundary: field caps never
/// refuse, and never split a character.
pub fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests;
