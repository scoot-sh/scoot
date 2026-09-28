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
//!   kinds (`output` today); the server sends every event of those kinds,
//!   and the client filters or debounces further itself. Standby cycles
//!   fire removal/restore pairs routinely -- that is accepted, and stated
//!   on each payload, rather than filtered server-side.
//!
//! [`Request`]: crate::Request
//! [`Response`]: crate::Response

use serde::{Deserialize, Serialize};

/// One class of event a connection can subscribe to.
///
/// Only [`EventKind::Output`] exists today (output removed/restored -- see
/// [`OutputRemoved`] and [`OutputRestored`]). Adding a kind is additive on
/// the request half, like adding an action: a client that never names it
/// sends -- and a server decodes -- byte-for-byte what it did before, and
/// an older server meets the new name with an ordinary `Error`, not a kill.
/// (The reply half is a new `Response` variant, which is why the first
/// subscription moved `PROTOCOL_VERSION` -- see that constant's doc.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Output removed and output restored, carrying the adoption: which
    /// output adopted the removed output's workspaces, the adopted workspace
    /// range, and the adopter's active workspace before and after.
    Output,
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
