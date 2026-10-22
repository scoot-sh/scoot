//! What a client can ask for.

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::event::EventKind;
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
    /// The browser-style back button (`BTN_BACK`, `0x115`).
    Back,
    /// The browser-style forward button (`BTN_FORWARD`, `0x116`).
    Forward,
}

/// What a look-scoped reload reply reports: which reload ran.
///
/// Only `Look` exists. It is the `scope` field of
/// [`Response::Reloaded`](crate::Response::Reloaded): absent on a plain
/// reload, `"look"` on a look-scoped one. It is response-only: the request
/// half is two distinct tags ([`Request::Reload`] and
/// [`Request::ReloadLook`]), because a `scope` field on the request would
/// not protect an older server. Serde's internally-tagged unit variants
/// drain and ignore extra fields, so an old compositor would decode
/// `{"type":"reload","scope":"look"}` (and even `"scope":"bogus"`) as a
/// plain `Reload` and apply the pending edits the scope exists to skip.
/// A distinct tag fails that decode with an ordinary unknown-request
/// `Error` instead, never a silent plain reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadScope {
    /// Only the look layer, under the snapshot: a look swap that never
    /// applies pending edits, spawns no autostart entry and swaps no binds.
    Look,
}

/// What [`Request::OutputScale`] names: an output id from `outputs` (a
/// JSON number), or a connector name (a JSON string: `DP-1`,
/// `headless-2`). Ids are stable for the session but fresh after every
/// unplug; names are stable across both, so a script that follows one
/// monitor names it -- and skips the `outputs` round trip whose answer a
/// hotplug could make stale before the set lands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutputTarget {
    Id(u64),
    Name(String),
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
    /// Scroll by a delta, sent as a wheel scroll that also carries its
    /// detents (eight v120 units per delta unit), so clients that only
    /// listen for steps still see it.
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
    /// Plain reload: both the file and the look layer are re-read. A look
    /// swap uses [`Request::ReloadLook`] instead, never this with extra
    /// fields. Additive like `MoveWindowToWorkspaceIndex`: a client that
    /// never sends the new tag decodes exactly as before, and this tag is
    /// byte-for-byte the old shape, so no `PROTOCOL_VERSION` bump for the
    /// request half (the reply half's `Reloaded` variant is what moved the
    /// version long ago -- see `Response::Reloaded`).
    Reload,
    /// Re-read only the look layer, merged under a snapshot of the file
    /// from the last successful plain load -- so a look swap never applies
    /// pending edits, spawns no autostart entry and swaps no binds.
    ///
    /// A distinct tag, not a `scope` field on [`Request::Reload`]: an
    /// older server answers it with an ordinary unknown-request `Error`
    /// and keeps serving, never a silent plain reload (which would apply
    /// the pending edits the scope exists to skip). Additive like any new
    /// tag, so no `PROTOCOL_VERSION` bump: a client that never sends it
    /// speaks byte-for-byte what it did before. Answered by
    /// `Response::Reloaded` with `"scope":"look"`.
    ReloadLook,
    /// Subscribe this connection to events of the named kinds, dedicating
    /// it to them: the reply is `Response::Subscribed`, and afterwards the
    /// connection carries events only (any other request on it is refused
    /// with an error -- open another connection for requests).
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half. (The reply and event halves are new `Response` variants, which
    /// is why the version still moved -- see `Response::Subscribed`.)
    Subscribe {
        events: Vec<EventKind>,
    },
    /// Power an output off or on -- the IPC half of
    /// `zwlr_output_power_manager_v1`, for agents and idle daemons that do
    /// not speak Wayland (what `swayidle ... resume 'wlopm --on *'` does over
    /// the protocol, this does over the socket).
    ///
    /// `output` names the output id `outputs` reports; `None` (`all` on the
    /// command line) means every output. An unknown id is refused with an
    /// error. Session-level like `outputs`/`reload`/`keyboard`, not an
    /// `action`: actions are window/focus operations (refused while locked),
    /// while this is output hardware state -- like gamma it applies while
    /// locked, which the idle cycle (off after lock, on at resume) needs.
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half -- and the reply half is the existing `Response::Ok`, so the
    /// version does not move at all.
    OutputPower {
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<u64>,
        powered: bool,
    },
    /// Set an output's scale live, or drop that live scale again -- the
    /// IPC half of what `reload` does to `[[outputs]]` scales, for
    /// display-profile watchers and agents that must not rewrite the
    /// user's config file to change a scale.
    ///
    /// `output` names the output id `outputs` reports, or its connector
    /// name; an unknown id or name is refused with an error. `scale` is a
    /// number, validated like the config loader's range (`0.5` to `4.0`,
    /// finite, resolved to 120ths) but refused outside it with a reason --
    /// never clamped silently, so a caller knows its value did not land as
    /// written. `null` drops the output's live scale, so it goes back to
    /// the config file's (`reset` on the command line). The field is
    /// required either way: a request that forgot it is refused rather
    /// than read as a reset.
    ///
    /// Runtime state like `OutputPower`, not config state: it moves the
    /// live session now (re-layout, re-tell, repaint, like a reload that
    /// changed only this output's scale) and is kept by connector name, so
    /// a replugged monitor comes back at it -- but a successful `reload`
    /// or a restart goes back to the config file's scales. Applies while
    /// locked, like `reload`; refused under `--nested`, where the host owns
    /// the scale and a reload refuses it too.
    ///
    /// Additive like `OutputPower`: a client that never sends this tag
    /// decodes exactly as before, and an older server answers it with an
    /// ordinary `Error` (unknown tag), so no `PROTOCOL_VERSION` bump for
    /// the request half -- and the reply half is the existing
    /// `Response::Ok`, so the version does not move at all.
    OutputScale {
        output: OutputTarget,
        /// `Option::deserialize` as an explicit deserializer makes the
        /// field required: serde's derive otherwise reads a missing
        /// `Option` as `None`, which here would be a silent reset.
        #[serde(deserialize_with = "Option::deserialize")]
        scale: Option<f64>,
    },
    /// Report the seat keyboard's currently effective layout (xkb group):
    /// its index and the keymap's name for it -- see
    /// [`KeyboardLayout`](crate::KeyboardLayout). What a layout indicator
    /// shows, and what tells an agent which layout the next `type` will
    /// produce.
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half. (The reply half is a new `Response` variant, which is why the
    /// version still moved -- see `Response::Keyboard`.)
    Keyboard,
    /// Whether the session is locked: the side-effect-free lock probe.
    ///
    /// Answered whether or not the session is locked -- unlike
    /// `Request::Action`, which is refused while locked, and unlike the old
    /// `focus-window-id u64::MAX` probe, which spent an `on_demand` layer
    /// surface's keyboard focus on every copy (see the compositor's
    /// `handle_request`). What the desktop clipboard asks before recording,
    /// and what tells an agent whether injected input would reach the lock
    /// screen instead of the desktop.
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half. (The reply half is a new `Response` variant, which is why the
    /// version still moved -- see `Response::Locked`.)
    Locked,
    /// The live keymap: every combo the running compositor binds, in its
    /// canonical spelling, with each row's action, where it came from
    /// (`default`, `config`, `config (replaces default: ...)`, `config
    /// (unbinds default: ...)` or `session`), its `repeat` and
    /// `allow_when_locked` flags, plus the config binds that were skipped
    /// with their reasons -- see [`Response::Binds`](crate::Response::Binds).
    /// What `Super+Shift+/` opens in a terminal, and what an agent reads
    /// instead of re-parsing the user's config file.
    ///
    /// Session-level like `outputs`: read off the live merged table on every
    /// request, never refused while locked -- listing what keys do is not a
    /// window-management operation (the `Super+Shift+/` bind itself never
    /// fires while locked, exactly like any other non-allow-listed bind).
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half. (The reply half is a new `Response` variant, which is why the
    /// version still moved -- see `Response::Binds`.)
    Binds,
    /// Swap the active look: what `scoot look apply` does to local files,
    /// reachable from a client that has no files -- a Mac driving a VM's
    /// compositor over the socket.
    ///
    /// `name` is a look name (`[a-z0-9-]{1,32}`), never a path: anything
    /// else is refused before anything is spawned. `for_ms` asks for a
    /// timed preview instead of a stay: after `for_ms` milliseconds the
    /// compositor swaps back to the look that was active, unless a newer
    /// choice moved the generation meanwhile (then the revert does
    /// nothing); omitted -- or zero -- means stay. A swap that only
    /// touches local files cannot be reached this way, and
    /// `Action(Spawn "scoot look apply …")` answers before the swap and
    /// never says whether it refused -- this answers when the swap lands,
    /// with [`Response::LookApplied`](crate::Response::LookApplied).
    ///
    /// The compositor runs the swap in a child (`scoot look apply NAME
    /// --from-compositor`, answered off the loop like `WaitIdle`), so one
    /// swap at a time: a second `LookApply` while one runs is refused with
    /// an error. Applies under session lock, like `Reload`.
    ///
    /// Additive like `Reload`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half -- and an older server answers it with an ordinary `Error`
    /// (unknown tag), never a misapplied swap. (The reply half is a new
    /// `Response` variant, which is why the version still moved -- see
    /// `Response::LookApplied`.)
    LookApply {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        for_ms: Option<u64>,
    },
    /// The look catalog, the active look and the session's state directory:
    /// what `scoot look list` and `scoot look current` read from local
    /// files, reachable from a client that has no files. Answered with
    /// [`Response::Looks`](crate::Response::Looks).
    ///
    /// Additive like `LookApply`: a client that never sends this tag
    /// decodes exactly as before, so no `PROTOCOL_VERSION` bump for the
    /// request half -- and an older server answers it with an ordinary
    /// `Error`, like any unknown tag. (The reply half is a new `Response`
    /// variant, which shares the same version move -- see
    /// `Response::Looks`.)
    Looks,
    /// Register a portal global shortcut: idempotent upsert by
    /// `(app_id, shortcut_id)`. The app's `preferred_trigger` is a hint
    /// only (stored, listed, never auto-bound); the user binds it via
    /// config `[global-shortcuts]` or `ShortcutBind`. Answered with
    /// [`Response::Shortcuts`](crate::Response::Shortcuts) re-listing, so
    /// a backend learns the bound-or-unbound outcome in one round trip.
    ///
    /// Additive like `Looks`: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump for the request
    /// half -- and an older server answers it with an ordinary `Error`.
    /// (The reply half is a new `Response` variant, which is why the
    /// version still moves -- see `Response::Shortcuts`.)
    ShortcutRegister {
        app_id: String,
        shortcut_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preferred_trigger: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// Remove one registration. Unknown ids are refused with an error.
    /// Answered with [`Response::Shortcuts`](crate::Response::Shortcuts).
    ///
    /// Additive like `ShortcutRegister`: a client that never sends this
    /// tag decodes exactly as before, so no `PROTOCOL_VERSION` bump for
    /// the request half.
    ShortcutUnregister {
        app_id: String,
        shortcut_id: String,
    },
    /// List every registered shortcut with its user binding and conflict
    /// state. Answered with
    /// [`Response::Shortcuts`](crate::Response::Shortcuts).
    ///
    /// Additive like `ShortcutRegister`: a client that never sends this
    /// tag decodes exactly as before, so no `PROTOCOL_VERSION` bump for
    /// the request half.
    Shortcuts,
    /// Bind a registered shortcut to a user chord, or clear it with
    /// `null`. The chord uses the same `super+ctrl+r` spelling as
    /// `Request::Key`. Binding to a chord a live compositor bind holds
    /// stays listed but conflicted and never fires (compositor wins);
    /// unknown ids and bad combos are refused. Answered with
    /// [`Response::Shortcuts`](crate::Response::Shortcuts).
    ///
    /// A distinct `null`-for-clear shape, not a missing field: serde's
    /// derive reads a missing `Option` as `None`, so a missing `combo`
    /// would silently clear. `Option::deserialize` makes the field
    /// required, the same shape as `OutputScale::scale`.
    ///
    /// Additive like `ShortcutRegister`: a client that never sends this
    /// tag decodes exactly as before, so no `PROTOCOL_VERSION` bump for
    /// the request half.
    ShortcutBind {
        app_id: String,
        shortcut_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        combo: Option<KeyCombo>,
    },
}
