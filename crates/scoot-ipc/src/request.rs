//! What a client can ask for.

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::key::KeyCombo;

/// Whether a [`Request::Screenshot`] that does not say draws the pointer:
/// yes. An agent reading a screenshot needs to see where the pointer is --
/// and needs that not to depend on which backend or renderer the session
/// runs, which is what the default guarantees.
pub const SCREENSHOT_CURSOR_DEFAULT: bool = true;

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
    /// Capture an output as PNG.
    ///
    /// Omitting `output` means the one output scoot composites, which is what
    /// every single-output session wants. Naming any *other* output is
    /// refused rather than answered from the composited one's pixels -- a
    /// picture of one screen labelled as another would be worse than an
    /// error. (Only `--headless --outputs N` can produce another output
    /// today; compositing more than one is future work.)
    ///
    /// `cursor` is whether the pointer is drawn into the capture, the same
    /// on every backend and renderer: `Some(false)` leaves it out, and
    /// omitting it means [`SCREENSHOT_CURSOR_DEFAULT`] (drawn in). Added
    /// without a [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION) bump: a
    /// request that omits it is the old shape exactly, and a server that
    /// predates it ignores the field rather than refusing the request -- so
    /// on such a server `cursor: false` has no effect.
    Screenshot {
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<bool>,
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
    /// Re-read the config file the session started from and re-apply what
    /// can be re-applied live (layout gap, appearance, keybindings).
    ///
    /// Additive like `MoveWindowToWorkspaceIndex`: a client that never sends
    /// this tag decodes exactly as before, so no `PROTOCOL_VERSION` bump for
    /// the request half. (The reply half is a new `Response` variant, which
    /// is why the version still moved -- see `Response::Reloaded`.)
    Reload,
}
