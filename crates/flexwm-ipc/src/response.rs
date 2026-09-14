//! What the server sends back.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    Ok,
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
