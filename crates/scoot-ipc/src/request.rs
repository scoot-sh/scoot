//! What a client can ask for.

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::key::KeyCombo;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// One request per line. Coordinates are global logical pixels: the same space
/// as window rectangles, and as screenshot pixels at scale 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Version,
    Outputs,
    Windows,
    /// Run a window-management action exactly as a keybinding would.
    Action(Action),
    /// Capture an output as PNG; the first output when `output` is omitted.
    Screenshot {
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<u64>,
    },
    PointerMove {
        x: f64,
        y: f64,
    },
    /// Press or release a button wherever the pointer is.
    PointerButton {
        button: PointerButton,
        pressed: bool,
    },
    /// Move, press and release in one step.
    Click {
        x: f64,
        y: f64,
        #[serde(default)]
        button: PointerButton,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    /// Press and release a key combination, holding exactly the modifiers it
    /// names and no others. The key is named as it is with *nothing* held --
    /// `shift+1`, not `exclam` -- because a compositor holding only what was
    /// asked for would otherwise press a key that types a different
    /// character. A name the active layout only carries above its unmodified
    /// level is an error rather than a guess. Use [`Request::Type`] for text.
    Key {
        keys: KeyCombo,
    },
    /// Type text as a sequence of key presses, with each character's own
    /// modifiers worked out from the active layout.
    Type {
        text: String,
    },
    /// Block until no window has redrawn for `quiet_ms`, or `timeout_ms` passes.
    WaitIdle {
        quiet_ms: u64,
        timeout_ms: u64,
    },
}
