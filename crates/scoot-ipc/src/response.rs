//! What the server sends back.

use serde::{Deserialize, Serialize};

use crate::event::{
    EventKind, KeyboardLayout, OutputChanged, OutputRemoved, OutputRestored, WorkspaceSnapshot,
};

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
    /// Whether the output is powered on: `false` while
    /// `zwlr_output_power_v1`/`output-power` has it switched off (no page
    /// flips, no render work, no frame callbacks -- see
    /// `site/src/content/docs/scoot/protocols.md`'s power section).
    ///
    /// Defaulted rather than required, like `scale` above and for the same
    /// wire reason: adding a field does not change the internally-tagged
    /// `Response` discriminant an older client keys on, and serde ignores
    /// an unknown one -- so this strictly adds information without breaking
    /// a single existing client, which is why it does *not* bump
    /// `PROTOCOL_VERSION`. Read it asymmetrically: `false` is always
    /// truthful (only a server new enough to power screens off sends it),
    /// while `true` means "on, or a server predating the field" (whose
    /// screens were always on).
    #[serde(default = "default_powered")]
    pub powered: bool,
}

/// The `scale` an [`OutputSnapshot`] carries when the server predates the
/// field. `1.0` because it is the only scale such a server could have run at.
fn default_scale() -> f64 {
    1.0
}

/// The `powered` an [`OutputSnapshot`] carries when the server predates the
/// field. `true` because such a server could never power a screen off.
fn default_powered() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    /// The freedesktop icon name the window's client set through
    /// `xdg-toplevel-icon-v1`, if it set one -- what a bar or dock shows
    /// beside the title, and what an agent matches a window against when the
    /// `app_id` is a generic one.
    ///
    /// `None` means the client set no *name*: it may have set none at all, or
    /// have supplied raw pixel buffers instead, which this protocol allows
    /// and scoot does not expose (see the compositor's `toplevel_icon.rs`).
    ///
    /// Defaulted rather than required, like `OutputSnapshot::scale` above and
    /// `Response::Ok`'s `locked`, and for the same wire reason: adding a
    /// field does not change the internally-tagged `Response` discriminant an
    /// older client keys on, and serde ignores an unknown one -- so this
    /// strictly adds information without breaking a single existing client,
    /// which is why it does *not* bump `PROTOCOL_VERSION`. `None` from an
    /// older server is truthful in the only sense that matters: it had no
    /// icon to report, because it did not implement the protocol.
    #[serde(default)]
    pub icon: Option<String>,
    pub output: u64,
    /// Which workspace of that output the window sits on, 0-based -- the
    /// same numbering `focus-workspace-index N` and
    /// `move-window-to-workspace-index N` take, so an agent can switch to
    /// the window's workspace or carry the window without converting.
    /// (Bars see the 1-based twin over `ext-workspace-v1`; see that
    /// module's doc for why the two numberings differ.)
    ///
    /// Defaulted like `floating` above, for the same wire reason, so no
    /// `PROTOCOL_VERSION` bump. Read it asymmetrically: any number from a
    /// new server is truthful, while `0` means "first workspace, or a
    /// server predating the field".
    #[serde(default)]
    pub workspace: usize,
    /// Whether the window's workspace was adopted from an unplugged monitor
    /// (see `adopted` workspaces): with [`WindowSnapshot::origin`], what
    /// tells an agent where an unplugged monitor's windows went.
    ///
    /// Defaulted like `workspace` above, for the same wire reason, so no
    /// `PROTOCOL_VERSION` bump. `false` means "not adopted, or a server
    /// predating the field".
    #[serde(default)]
    pub adopted: bool,
    /// Which connector the window's workspace was adopted from ("DP-1"),
    /// when [`WindowSnapshot::adopted`]. `None` for a workspace that was
    /// never adopted -- and from a server predating the field.
    ///
    /// Defaulted like `adopted` above (`None` decodes from absence), for
    /// the same wire reason, so no `PROTOCOL_VERSION` bump.
    #[serde(default)]
    pub origin: Option<String>,
    /// Where the window's toplevel surface is, in logical pixels (its
    /// popups can draw outside it). Only the part inside its `output`'s own
    /// `rect` is drawn and takes input: a column scrolled part-way past its
    /// output's edge (or a fullscreen window focused away from, below) is
    /// cut at that edge, and beyond it the neighbouring output's own content
    /// is what shows and is clicked -- or nothing, past the last output.
    pub rect: Rect,
    pub visible: bool,
    pub focused: bool,
    /// Whether this window's popup tree currently holds the keyboard through
    /// an explicit `xdg_popup.grab` -- a menu, combo box or typeahead the
    /// client opened and asked input be routed to.
    ///
    /// This is the field that tells "window B is focused" apart from
    /// "window B is focused but window A's menu holds the keyboard":
    /// compositor focus (`focused`) still moves while a grab is live --
    /// keybindings keep firing, by design -- but every keystroke reaches
    /// the grabbing popup instead. An agent that only reads `focused`
    /// would inject its next key at the wrong target with no error; one
    /// that checks this field first will not.
    ///
    /// Defaulted rather than required, like `icon` above and for the same
    /// wire reason: adding it does not change the internally-tagged
    /// `Response` discriminant an older client keys on, and serde ignores
    /// an unknown one -- which is why it does *not* bump
    /// `PROTOCOL_VERSION`. Read it asymmetrically: `true` is always
    /// truthful, while `false` means "no grab, or a server predating the
    /// field". A grab rooted at a layer surface (a bar's own dropdown)
    /// belongs to no window and leaves every window `false`; likewise a
    /// grab that just ended but has not been reaped yet.
    #[serde(default)]
    pub popup_grab: bool,
    /// Whether the window is fullscreen (entered by the client itself, by
    /// `toggle-fullscreen`, or by a taskbar). A fullscreen window covers its
    /// whole output -- `rect` equals that output's `rect`, and every other
    /// window on it is reported `visible: false` -- whenever its column is
    /// the focused one; focused away, it keeps that size and sits in the
    /// strip where a column that wide would, one ordinary gap from its
    /// neighbours (possibly partly `visible`, never overlapping the focused
    /// window, and like any window drawn and hit only within its own
    /// output -- see `rect`).
    ///
    /// Defaulted like `popup_grab` above, for the same wire reason, so no
    /// `PROTOCOL_VERSION` bump. `false` from an older server is truthful: it
    /// had no fullscreen state to report.
    #[serde(default)]
    pub fullscreen: bool,
    /// Whether the window is maximized (entered by the client itself, by
    /// `toggle-maximize`, or by a taskbar). A maximized window fills its
    /// output's usable area -- the full output minus whatever a bar reserved
    /// at its edges, minus the layout gap -- whenever its column is the
    /// focused one and no fullscreen window covers the output; focused away,
    /// it keeps that size and sits in the strip where a column that wide
    /// would, one ordinary gap from its neighbours. The bar stays visible:
    /// this is the difference from `fullscreen`, which covers it.
    ///
    /// Defaulted like `fullscreen` above, for the same wire reason, so no
    /// `PROTOCOL_VERSION` bump. `false` from an older server is truthful: it
    /// had no maximized state to report.
    #[serde(default)]
    pub maximized: bool,
    /// Whether the window floats above its workspace's scrolling strip
    /// (floated as it mapped -- a dialog, a transient, a fixed-size window,
    /// or a `[[window_rule]]` -- or by `toggle-floating` / `set-floating`).
    /// `rect` is then where it really is on screen, centred on its parent or
    /// its output and inside the output's usable area, and it is drawn and
    /// clicked above every tiled window of its output. A floating window
    /// that has not drawn its first frame yet is `visible: false`.
    ///
    /// Defaulted like `fullscreen` above, for the same wire reason, so no
    /// `PROTOCOL_VERSION` bump. `false` from an older server is truthful: it
    /// had no floating windows.
    #[serde(default)]
    pub floating: bool,
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
    /// What a `Reload` request answers on success: which config fields were
    /// re-applied live, and which differed from the running session but
    /// cannot be (`tty.gpu` and `renderer.backend`, which take effect on
    /// restart -- each refused explicitly, never silently ignored -- plus
    /// `output.scale` under `--nested`, where the host compositor owns the
    /// scale, and any reloaded `[autostart]` entry that is not a new `spawn`,
    /// refused by name; new spawns run, and report applied).
    ///
    /// A reload under session lock skips new autostart entries (refused as
    /// skipped-while-locked, run on the first unlocked reload) while
    /// everything else applies as usual -- see the compositor's `reload.rs`
    /// for the while-locked argument.
    ///
    /// Both lists name only fields that *differed*: a field the file and the
    /// running session agree on appears in neither. Two empty lists together
    /// therefore mean "the reload changed nothing it was asked to" -- that
    /// is the honest answer, not a missing one. A reload that could not load
    /// or validate the file at all answers `Error` instead (the running
    /// config untouched), which is why there is no third list here.
    ///
    /// A new variant, so this is the half that moved `PROTOCOL_VERSION` (see
    /// that constant's doc): an older client that somehow received one would
    /// fail to decode it. In practice only a client new enough to send
    /// `Reload` ever receives one.
    Reloaded {
        applied: Vec<String>,
        refused: Vec<String>,
    },
    /// What a `Subscribe` request answers on success: the event kinds this
    /// connection is now dedicated to, echoed back. Afterwards the
    /// connection carries [`OutputRemoved`]/[`OutputRestored`]/
    /// [`OutputChanged`]/[`KeyboardChanged`]/[`Workspaces`] unasked, and
    /// any other request on it is refused with an error.
    ///
    /// A new variant, so this is one of the halves that moved
    /// `PROTOCOL_VERSION` 3 → 4 (see that constant's doc): an older client
    /// handed one would fail its decode. In practice only a client new
    /// enough to send `Subscribe` ever receives one -- and a client that
    /// never subscribes never receives an event either, so the bump costs
    /// existing clients nothing at runtime.
    Subscribed {
        events: Vec<EventKind>,
    },
    /// An output was removed; see [`OutputRemoved`] for what each field
    /// means and when this fires. Sent only to connections subscribed to
    /// [`EventKind::Output`].
    ///
    /// A new variant, so this shares the 3 → 4 `PROTOCOL_VERSION` move with
    /// `Subscribed` above: an older client handed one would fail its
    /// decode, which can only happen to a client new enough to have
    /// subscribed.
    OutputRemoved(OutputRemoved),
    /// An output was restored; see [`OutputRestored`]. Sent only to
    /// connections subscribed to [`EventKind::Output`], under the same
    /// version move as `OutputRemoved`.
    OutputRestored(OutputRestored),
    /// An output's mode changed in place; see [`OutputChanged`] for what
    /// each field means and when this fires. Sent only to connections
    /// subscribed to [`EventKind::Output`].
    ///
    /// A new variant, so this moves `PROTOCOL_VERSION` 4 → 5 (see that
    /// constant's doc): an older client handed one would fail its decode,
    /// which can only happen to a client new enough to have subscribed.
    OutputChanged(OutputChanged),
    /// The answer to [`Request::Keyboard`](crate::Request::Keyboard): the
    /// seat keyboard's currently effective layout -- see [`KeyboardLayout`]
    /// for what each field means.
    ///
    /// A new variant, so this shares the 5 → 6 `PROTOCOL_VERSION` move with
    /// `KeyboardChanged` below: an older client handed one would fail its
    /// decode, which can only happen to a client new enough to have asked.
    Keyboard(KeyboardLayout),
    /// The active keyboard layout changed; see [`KeyboardLayout`]. Sent only
    /// to connections subscribed to [`EventKind::Keyboard`], under the same
    /// version move as `Keyboard` above.
    KeyboardChanged(KeyboardLayout),
    /// One output's workspace occupancy changed; see [`WorkspaceSnapshot`]
    /// for what each field means and when this fires. Sent only to
    /// connections subscribed to [`EventKind::Workspace`].
    ///
    /// A new variant, so this moves `PROTOCOL_VERSION` 6 → 7 (see that
    /// constant's doc): an older client handed one would fail its decode,
    /// which can only happen to a client new enough to have subscribed.
    Workspaces(WorkspaceSnapshot),
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
