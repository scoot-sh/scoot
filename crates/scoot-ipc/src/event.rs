//! Event subscriptions: push notifications for polling consumers.
//!
//! A connection is a request/reply loop (see the crate root): one [`Request`]
//! per line, one [`Response`] per line. A subscription does not change that
//! framing -- it dedicates one connection to receiving [`Response`]s the
//! compositor sends unasked:
//!
//! ```json
//! {"type": "subscribe", "events": ["output"]}
//! {"type": "subscribed", "events": ["output"]}
//! {"type": "output_removed", "output": 2, ...}
//! ```
//!
//! The rules, which mirror the request/reply contract beside them:
//!
//! - **Subscribe is a request like any other.** `Request::Subscribe` names
//!   the [`EventKind`]s the connection wants, and the reply is
//!   `Response::Subscribed` echoing what it got. An unknown kind is a decode
//!   error the server answers with an ordinary `Error` and keeps serving --
//!   the same asymmetric answer an unknown request tag gets (see
//!   `PROTOCOL_VERSION`'s doc): an older server meets a newer client with an
//!   error, not a kill.
//! - **A subscribed connection serves no further requests.** After the
//!   `Subscribed` reply it carries events only; any other request on it is
//!   refused with an error naming the rule. That keeps reply/event ordering
//!   trivial (the `Subscribed` answer is sent synchronously, before any
//!   event can exist) and a one-shot client that accidentally subscribes
//!   cannot wedge itself waiting for a reply behind an event it never reads.
//!   Open another connection for requests -- requests pipeline, so one is
//!   enough for any number of them.
//! - **Filtering is by kind, not by field.** The subscription names event
//!   kinds (`output`, `keyboard`, `workspace`, `lock`); the server sends every event of those
//!   kinds,
//!   and the client filters or debounces further itself. Standby cycles
//!   fire removal/restore pairs routinely -- that is accepted, and stated
//!   on each payload, rather than filtered server-side.
//!
//! [`Request`]: crate::Request
//! [`Response`]: crate::Response
//! [`KeyboardLayout`]: crate::KeyboardLayout

use serde::{Deserialize, Serialize};

/// One class of event a connection can subscribe to.
///
/// [`EventKind::Output`] covers output removed/restored/changed -- see
/// [`OutputRemoved`], [`OutputRestored`] and [`OutputChanged`].
/// [`EventKind::Keyboard`] covers the active keyboard layout changing -- see
/// [`KeyboardLayout`].
/// [`EventKind::Workspace`] covers workspace occupancy changing -- see
/// [`WorkspaceSnapshot`]. [`EventKind::Lock`] covers the session locking
/// and unlocking -- see [`Request::Locked`](crate::Request::Locked). Adding a kind is additive on
/// the request half, like adding an action: a client that never names it
/// sends -- and a server decodes -- byte-for-byte what it did before, and
/// an older server meets the new name with an ordinary `Error`, not a kill.
/// (The reply half is a new `Response` variant, which is why the first
/// subscription moved `PROTOCOL_VERSION` -- see that constant's doc.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Output removed, restored and changed, carrying the adoption for the
    /// first two (which output adopted the removed output's workspaces, the
    /// adopted workspace range, and the adopter's active workspace before
    /// and after) and the new mode for the third.
    Output,
    /// The active keyboard layout (xkb group) changed. No standard Wayland
    /// protocol reports this to an unfocused client -- `wl_keyboard` sends
    /// the keymap and the modifier group only to the client holding keyboard
    /// focus, which a bar never does -- so this is the channel a layout
    /// indicator reads. Read-only: scoot has no layout-switch bind, option
    /// or action, so nothing here switches the layout; the group moves only
    /// through the keymap's own mechanics (a toggle key from the
    /// `XKB_DEFAULT_OPTIONS` the session started with, e.g.
    /// `grp:caps_toggle`).
    Keyboard,
    /// Workspace occupancy changed on an output: which workspaces hold
    /// windows, as a full per-output snapshot -- see [`WorkspaceSnapshot`].
    /// What a bar's workspace module draws (dimming the empty ones) without
    /// polling `windows`, and what tells an agent "workspace 3 now has
    /// windows". No standard Wayland protocol reports occupancy to an
    /// unfocused client -- `ext-workspace-v1` carries the list, positions
    /// and the one `active` bit, but no "holds windows" bit, and no other
    /// standard protocol maps a toplevel to a workspace.
    Workspace,
    /// The session locked or unlocked: what tells an agent "injected input
    /// now reaches the lock screen" without polling `locked`, and what a
    /// clipboard watcher leans on instead of its probe interval. No
    /// standard Wayland protocol reports this to an unfocused client --
    /// `ext-session-lock-v1` events go only to the lock client itself.
    /// Read-only: nothing over IPC locks or unlocks the session.
    Lock,
}

/// An output was removed: its workspaces were adopted onto a remaining
/// output (or, with no adopter, wait in `unplaced` -- see `adopter`).
///
/// All workspace indices are 0-based, the numbering `windows` and
/// `focus-workspace-index` speak. `adopted_start`/`adopted_count` name the
/// block the adoption put on the adopter, in the post-removal list --
/// which is what a restore verifies positions against, so a client holding
/// this range can tell whether later workspace changes moved it.
///
/// Fires on every removal with an adopter, including standby cycles (a
/// monitor dropping its connection in standby is a routine unplug to
/// scoot) and removals that adopted nothing (`adopted_count` 0, `origin`
/// `None`, both actives equal): a consumer that only cares about moved
/// windows filters on `adopted_count`, one that wants every plug event
/// debounces. scoot draws and sends nothing itself either way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRemoved {
    /// The removed output's id, as `outputs` reported it.
    pub output: u64,
    /// Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) --
    /// the same string `outputs` names it by, and the identity a later
    /// restore matches on.
    pub name: String,
    /// The output that adopted its workspaces. `None` when nothing adopted
    /// them (they wait in `unplaced` for an output to appear for them).
    pub adopter: Option<u64>,
    /// The 0-based workspace index the adopted block starts at on the
    /// adopter, in the post-removal list.
    pub adopted_start: usize,
    /// How many workspaces were adopted (0 when the removed output held no
    /// windows).
    pub adopted_count: usize,
    /// The adopter's active workspace before the removal, 0-based. `None`
    /// with no live adopter to read it off.
    pub adopter_prev_active: Option<usize>,
    /// The adopter's active workspace after the removal, 0-based: where it
    /// was already looking, or the adopted workspace the switch moved it
    /// to when focus was on the removed output.
    pub adopter_active: Option<usize>,
    /// Which connector the adopted workspaces are tagged as coming from
    /// (`DP-1`) -- what bars show and what `windows` reports each adopted
    /// window adopted from. `None` when nothing was adopted.
    pub origin: Option<String>,
}

/// An output came back under a matching identity: the still-open windows
/// moved back onto it.
///
/// The adoption fields describe the record this restore consumed, as filed
/// by the removal: `adopter` which output had adopted the block,
/// `adopted_start`/`adopted_count` where the record placed it, `origin`
/// the connector name the adopted workspaces were tagged with until now.
/// Intervening changes (a hand move, a close, an unrelated shift) keep a
/// window where it is instead -- `moved` says how many still-open windows
/// actually went back -- and a dead adopter (a chained unplug whose middle
/// monitor never returned) leaves the actives `None`: there is no view to
/// report, and nothing moved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRestored {
    /// The returned output's id. A fresh one, never the removed output's --
    /// ids are stable for the session, not across unplug cycles.
    pub output: u64,
    /// Its connector name -- the identity the removal filed, matched again.
    pub name: String,
    /// The output that had adopted the removed output's workspaces, if the
    /// removal recorded one.
    pub adopter: Option<u64>,
    /// Where the removal's record placed the adopted block on the adopter.
    pub adopted_start: usize,
    /// How many workspaces the removal adopted.
    pub adopted_count: usize,
    /// The adopter's active workspace before the restore, 0-based. `None`
    /// when the adopter is gone.
    pub adopter_prev_active: Option<usize>,
    /// The adopter's active workspace after the restore, 0-based: back at
    /// its pre-adopt view while it was still showing the adopted block,
    /// left alone otherwise. `None` when the adopter is gone.
    pub adopter_active: Option<usize>,
    /// The connector name the adopted workspaces were tagged with, cleared
    /// by this restore.
    pub origin: Option<String>,
    /// How many still-open windows actually moved back. Windows moved by
    /// hand or closed in between stay where they are.
    pub moved: usize,
}

/// An output's mode changed in place: the same connector at a new
/// framebuffer size, with the scale it keeps running at.
///
/// This is the re-probe resize -- a `--tty` hotplug offering a new mode for
/// a connector that stays connected (a VM window moving between displays of
/// different densities), or a `--nested` host window being resized -- not a
/// removal: no workspace is adopted, nothing moves, and no restored event
/// follows. A script watching density (per-pixel size against physical size)
/// recomputes the scale it wants from `width`/`height` and applies it with
/// a config reload (`output.scale` and `outputs.<name>.scale` apply live);
/// polling `outputs` gives the same numbers, this just says when to look.
///
/// Fires once per applied resize, after the windows are re-laid-out at the
/// new size. A resize the render target refuses fires nothing: the output
/// stays at its old size, which no client was left believing otherwise. A
/// same-size call still counts as applied: callers skip those before
/// reaching here, so every event names a size that actually changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputChanged {
    /// The output's id, as `outputs` reports it.
    pub output: u64,
    /// Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) --
    /// the same string `outputs` names it by.
    pub name: String,
    /// The new framebuffer size in physical pixels.
    pub width: i32,
    /// The new framebuffer size in physical pixels.
    pub height: i32,
    /// The scale the output keeps running at: a mode change never changes
    /// the scale, so this is the scale to recompute *from*.
    pub scale: f64,
}

/// The seat keyboard's currently effective layout (xkb group): what
/// [`Request::Keyboard`](crate::Request::Keyboard) answers with, and what a
/// [`EventKind::Keyboard`] subscription carries when it changes.
///
/// The `index` is the 0-based group -- the same numbering `msg type`
/// resolves each character in, so an agent reads off which layout its next
/// `type` will produce. The `name` is the keymap's own name for that group
/// (`us` reads as `"English (US)"`, `ru` as `"Russian"`), which is what a
/// bar shows. Both are read live off the compositor's keymap: there is no
/// scoot-side copy to go stale.
///
/// Fires once per change, never per keypress: typing on one layout sends
/// nothing, and one group switch sends exactly one event. Rapid successive
/// switches each send their own -- every event names the layout in effect
/// when it was sent, so a reader that processes them in order ends where
/// the keyboard is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardLayout {
    /// The keymap's name for the active group (`"English (US)"`).
    pub name: String,
    /// The active group, 0-based.
    pub index: u32,
}

/// One output's workspace occupancy: which workspaces hold windows.
///
/// What an [`EventKind::Workspace`] subscription carries when it changes,
/// one event per output whose snapshot moved. A **full snapshot** rather
/// than a delta: every event names every workspace of the output, so a
/// subscriber that missed one is never wrong -- it just shows the latest.
///
/// `active` is the output's active workspace and `counts[i]` the number of
/// windows on its `i`-th workspace, both 0-based in the numbering `windows`
/// and `focus-workspace-index` speak. Counts rather than a plain occupied
/// flag: a bar derives the flag (`counts[i] > 0`), while an agent reads how
/// many windows each workspace holds -- and the extra bytes are one small
/// integer per workspace.
///
/// Fires at most once per output per frame tick, however fast windows open,
/// close or move: changes mark the snapshot dirty and the tick sends the
/// latest, so a client opening and closing windows at its maximum rate is
/// one event per tick, not an event stream. A fresh subscription starts
/// silent -- read `windows` once for the baseline, then apply snapshots
/// after it -- so subscribing never replays the whole unsubscribed interval
/// as one change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    /// The output's id, as `outputs` reports it.
    pub output: u64,
    /// Its connector name (`DP-1` under `--tty`, `headless-2` otherwise) --
    /// the same string `outputs` names it by.
    pub name: String,
    /// The output's active workspace, 0-based.
    pub active: usize,
    /// One entry per workspace, in order: how many windows sit on it.
    pub counts: Vec<usize>,
}
