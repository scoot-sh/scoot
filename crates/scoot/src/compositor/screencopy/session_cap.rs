//! A per-client bound on screen-capture and pointer-cursor sessions.
//!
//! `ext_image_copy_capture_manager_v1.create_session` and
//! `.create_pointer_cursor_session` each make a session that this compositor
//! and Smithay track in a list for as long as the client keeps the object,
//! and that is walked on every frame tick (`service_captures`,
//! `service_cursor_sessions`) and by Smithay on every `capture` and every
//! frame teardown. `MAX_FRAMES_PER_CLIENT` bounds the expensive half of that
//! (the full-screen copies); nothing bounded how many sessions one client
//! could hold, so a buggy or hostile client looping `create_session` grew
//! both lists, and every other client's tick and capture latency with them.
//!
//! # What is counted
//!
//! One unit per **logical session**: a capture session made by
//! `create_session`, or a pointer cursor session made by
//! `create_pointer_cursor_session`. A cursor session's derived capture
//! session (`ext_image_copy_capture_cursor_session_v1.get_capture_session`)
//! is not a unit of its own: the protocol allows exactly one per cursor
//! session, so it rides on its parent's unit. The claim is at the manager
//! request, before Smithay's handler; the release is in `dispatch.rs`'s
//! destruction hook, which sees every dead protocol object including the
//! ones a disconnect destroys, so a unit is freed the moment its session is
//! destroyed, the moment its client goes away, and never otherwise. Every
//! branch of the fork's handlers initialises exactly one object per request
//! (a served session, a stopped one, one for an invalid source), so a claim
//! is always paired with one release.
//!
//! # What a client sees at the cap
//!
//! - At [`MAX_SESSIONS_PER_CLIENT`] the next session is created **stopped**:
//!   `ext_image_copy_capture_session_v1.stopped` for a capture session, and
//!   for a cursor session, whose interface has no `stopped` event, a stopped
//!   derived capture session. That is the protocol's own "not this one"
//!   (the fork already answers a source the compositor rejects the same
//!   way), it costs no list entry, and the client stays connected: a client
//!   that reconnects by creating the new session before destroying the old
//!   one, or that reaches the cap through a bug, loses that one session and
//!   nothing else. The stopped object still counts, since the client still
//!   holds it, and frees its unit when destroyed like any other.
//! - At [`KILL_SESSIONS_PER_CLIENT`] the client is sent a protocol error
//!   instead. Past the soft cap a client is not reconnecting, it is looping,
//!   and stopped objects, cheap as they are, are still unbounded objects.
//!
//! # The value
//!
//! 32 live sessions. The largest legitimate holder is a remote-desktop
//! server (wayvnc: one capture and one cursor session per output) or a
//! screencast portal backend multiplexing apps, on up to
//! 8 outputs: 16 for the former, with the same again as headroom for a
//! reconnect that overlaps old and new sessions. What a session costs is
//! mostly per tick: an entry scanned by `service_captures` and Smithay (the
//! entry is 224 bytes, `size_of::<Capture>()` measured on this tree, so 32 of
//! them are 7 KiB), and for a cursor session a pointer lookup and a
//! deduplicated position event. Estimated at tens of nanoseconds each, not
//! measured, so 32 is about a microsecond per tick, while the copies a session
//! can demand are already bounded by `MAX_FRAMES_PER_CLIENT`. Toplevel capture
//! sources are not advertised; if they ever are, a screencast of each window is
//! a session each, and this number has to be revisited.
//!
//! Per client, not global, so one greedy client can never deny a
//! well-behaved one.

use std::collections::{HashMap, HashSet};

use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};

/// Live capture and cursor sessions one client may hold before its next one
/// is created stopped. See the module doc for the derivation.
pub(in crate::compositor) const MAX_SESSIONS_PER_CLIENT: u32 = 32;

/// Live sessions (stopped ones included) at which a client is disconnected
/// with a protocol error instead of being refused politely.
pub(in crate::compositor) const KILL_SESSIONS_PER_CLIENT: u32 = 2 * MAX_SESSIONS_PER_CLIENT;

/// What to do with one more session request from a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) enum Admission {
    /// Serve it normally.
    Admit,
    /// Create it stopped.
    Stop,
    /// Disconnect the client; nothing was claimed.
    Kill,
}

/// The decision for a client already holding `held` sessions. Pure, so the
/// two boundaries are pinned without a compositor.
pub(super) fn decide(held: u32) -> Admission {
    if held >= KILL_SESSIONS_PER_CLIENT {
        Admission::Kill
    } else if held >= MAX_SESSIONS_PER_CLIENT {
        Admission::Stop
    } else {
        Admission::Admit
    }
}

/// The bookkeeping behind the cap.
#[derive(Default)]
pub(in crate::compositor) struct SessionCap {
    /// Live sessions by client. An entry exists only while the client holds
    /// at least one, so the map is bounded by connected clients.
    held: HashMap<ClientId, u32>,
    /// Set from the moment a request is admitted as `Stop` until Smithay's
    /// handler for it returns; the constraints callbacks answer `None`
    /// while it is, which is how the fork creates a stopped session.
    refusing: bool,
    /// Cursor sessions that already made their one capture session, by
    /// protocol id. The fork's `get_capture_session` returns without
    /// initialising the new object when asked twice, which this compositor
    /// would otherwise take as the uninitialised-object panic; the second
    /// request is refused with the protocol's `duplicate_session` first.
    cursor_with_capture: HashSet<ObjectId>,
}

impl SessionCap {
    /// Whether a session being created right now must come out stopped.
    pub(super) fn refusing(&self) -> bool {
        self.refusing
    }

    /// Ends a refusal. Called after the delegated handler returns, whether or
    /// not it asked for the constraints: the fork's invalid-source branch
    /// never does, and a flag left armed would stop the next client's session.
    pub(in crate::compositor) fn disarm(&mut self) {
        self.refusing = false;
    }

    /// Claims a unit for `client` and arms the refusal if it must be stopped.
    pub(in crate::compositor) fn admit(&mut self, client: &Client) -> Admission {
        let held = self.held.entry(client.id()).or_insert(0);
        let verdict = decide(*held);
        match verdict {
            Admission::Admit => *held += 1,
            Admission::Stop => {
                *held += 1;
                self.refusing = true;
                if *held == MAX_SESSIONS_PER_CLIENT + 1 {
                    // Once per episode: the count only passes through this
                    // value once until the client drops back under the cap.
                    tracing::warn!(
                        "a client reached {MAX_SESSIONS_PER_CLIENT} live capture sessions; \
                         its further sessions are created stopped"
                    );
                }
            }
            Admission::Kill => {}
        }
        verdict
    }

    /// Frees one unit for a client whose session object just died.
    pub(in crate::compositor) fn release(&mut self, client: &ClientId) {
        if let Some(held) = self.held.get_mut(client) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                self.held.remove(client);
            }
        }
    }

    /// Records that a cursor session has made its capture session; `false`
    /// when it had already.
    pub(in crate::compositor) fn note_capture_session(&mut self, cursor: ObjectId) -> bool {
        self.cursor_with_capture.insert(cursor)
    }

    /// Forgets a dead cursor session.
    pub(in crate::compositor) fn forget_cursor_session(&mut self, cursor: &ObjectId) {
        self.cursor_with_capture.remove(cursor);
    }

    /// Live units across all clients. Test-only.
    #[cfg(test)]
    pub(super) fn total(&self) -> u32 {
        self.held.values().sum()
    }

    /// Cursor sessions remembered as having a capture session. Test-only.
    #[cfg(test)]
    pub(super) fn cursor_marks(&self) -> usize {
        self.cursor_with_capture.len()
    }
}
