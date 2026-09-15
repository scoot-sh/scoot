//! What the server sends back.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub id: u64,
    pub name: String,
    pub rect: Rect,
    /// The output's scale: its `rect` is in logical pixels, while
    /// [`Screenshot`] pixels are physical, so an agent needs this to convert
    /// between the two (`physical = logical * scale`, rounded down where an
    /// edge lands mid-pixel -- `logical` is `ceil(physical / scale)`, so a
    /// full-output product can overshoot by under one pixel).
    ///
    /// Defaulted rather than required, deliberately: adding a field does not
    /// change the internally-tagged `Response` discriminant an older client
    /// keys on, and serde ignores an unknown field, so a new client talking to
    /// an older server gets 1.0 (the only scale that existed before output
    /// scaling) instead of a decode failure. That is why this does *not* bump
    /// `PROTOCOL_VERSION` -- see this crate's `PROTOCOL_VERSION` doc, which
    /// reserves a bump for a change that *breaks* existing clients.
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// The part of `rect` ordinary windows are arranged within: the full
    /// output minus whatever a bar reserved at its edges (layer-shell
    /// exclusive zones). An agent mapping screenshot pixels to "where can a
    /// window be" wants this, not `rect`.
    ///
    /// Defaulted like `scale`, for the same wire reason -- but unlike
    /// `scale`, the default is a sentinel, not a truthful value: all
    /// zeros, which no output the platform produces in practice can have
    /// (a zero-sized output has no pixels to arrange into). An older
    /// server *did* reserve bars, so "same as `rect`" would be a lie; the
    /// sentinel tells a new client the server predates the field and it
    /// should fall back to `rect`, exactly what it did before the field
    /// existed.
    #[serde(default)]
    pub usable: Rect,
}

/// The `scale` an [`OutputSnapshot`] carries when the server predates the
/// field. `1.0` because it is the only scale such a server could have run at.
fn default_scale() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub output: u64,
    pub rect: Rect,
    pub visible: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    /// A PNG image; base64 on the wire.
    #[serde(with = "base64_bytes")]
    pub png: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Success. `locked` is the session-lock state when the response was
    /// built -- an agent typing a password over IPC learns the unlock
    /// landed from the very next reply, without polling anything else.
    ///
    /// Defaulted like `OutputSnapshot::scale`, for the same wire reason --
    /// but read it asymmetrically: `true` is always truthful (only a
    /// server new enough to know the state sends it), while `false` means
    /// "unlocked *or* the server predates the field". There is no version
    /// that distinguishes those, by design: this field strictly adds
    /// information (a `true` is new knowledge) without breaking a single
    /// existing client, which is what keeps it off the
    /// `PROTOCOL_VERSION`-bump list.
    Ok {
        #[serde(default)]
        locked: bool,
    },
    Version {
        version: String,
        protocol: u32,
    },
    Outputs {
        outputs: Vec<OutputSnapshot>,
    },
    Windows {
        windows: Vec<WindowSnapshot>,
    },
    Screenshot(Screenshot),
    Idle {
        waited_ms: u64,
    },
    /// Success, but with a side effect the caller should know about that
    /// isn't implied by the request having been accepted.
    Warning {
        message: String,
    },
    Error {
        message: String,
    },
}

impl Response {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}
