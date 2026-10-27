//! IPC reload wallpaper-wait: `Request::ReloadWaitWallpaper` runs a plain
//! reload and parks the `reloaded` reply until the wallpaper run lands.
//!
//! A plain `reload` hands the `[wallpaper]` section to `scootbg
//! apply-config` and answers at once (handed over, never waited on). That
//! used to be the only shape: the old `--wait-wallpaper` flag blocked the
//! event-loop thread with `thread::sleep` polling, freezing the whole
//! compositor for up to 30 s -- and it always timed out, because scootbg
//! needs the loop to service its Wayland requests while it works. The flag
//! was removed; this is its deferred replacement: the park is state plus a
//! wakeup, never a sleep.
//!
//! The requesting connection parks in `PendingReloadWait` while every other
//! connection keeps being served -- the `LookApply` pattern (see
//! `look_apply.rs`), not the `WaitIdle` one-shot. The `SIGCHLD` reaper
//! answers it when the `apply-config` run exits, or the frame tick answers
//! it past a bound, reporting handed-over-but-unwaited-on. One parked reply
//! per request: two reloads park two replies, each answered when its own
//! run (or the newest, if superseded) exits. `SIGHUP` never parks -- it has
//! no reply channel, so it runs the plain path.
//!
//! Edge cases, pinned by tests:
//! - the run exits before the park: no running pid at park time answers at
//!   once, like a plain reload;
//! - the client disconnects while parked: the reply write fails and the
//!   entry drops, holding its slot only until then (like `WaitIdle`, the
//!   slot bounds the parked count at `MAX_CONNECTIONS`);
//! - two reloads park two replies: each waits on its own submission seq;
//!   a superseded wait (its section replaced before it started) follows the
//!   newest instead of waiting for a run that will never start;
//! - the output goes away, the daemon dies, or a second reload arrives:
//!   every path ends in the reaper (exit, however it ended) or the timeout,
//!   so the reply is always eventually sent or dropped cleanly.
//!
//! No allocation on the common path: a plain `Reload` never touches
//! `pending_reload_wait` (see `State::reload`), and the reaper's fast path
//! (`wallpaper.is_idle()`) skips the drain when nothing is tracked. Only a
//! wait request allocates -- its own entry, plus the reply line when it
//! answers.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use scoot_ipc::{ReloadScope, Response, encode};

use super::outbound::Outbound;
use super::slots::Slot;
use crate::compositor::State;

#[cfg(test)]
mod tests;

/// How long a parked wallpaper-wait waits for the `apply-config` run
/// before answering anyway.
///
/// scootbg bounds itself (5 s to reach a daemon, 30 s for its reply), so a
/// healthy run lands well inside this; past it the reply reports the
/// wallpaper as handed over but unwaited-on, and the run's exit is still
/// logged when it comes. Matches the old blocking flag's bound, minus the
/// blocking: the loop stays live the whole wait.
pub(super) const RELOAD_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a parked reply is pushed at a client that stopped reading
/// before it is dropped. Reload replies are a few hundred bytes -- a peer
/// that will not take them is gone, not slow.
const RELOAD_REPLY_STALL: Duration = Duration::from_secs(10);

/// One `reload --wait-wallpaper` parked while its wallpaper run works -- or
/// while its reply drains to a slow client.
pub(crate) struct PendingReloadWait {
    /// The wallpaper submission this park waits on: the queue's newest seq
    /// at submit time. Answered when a reaped run's seq reaches it (see
    /// `answer_ready`). A superseded wait (its seq replaced before it
    /// started) is answered when the newest lands instead.
    waited_seq: u64,
    /// This reload's report, computed at park time: applied/refused as the
    /// plain reload decided them. Answered as `Response::Reloaded` when the
    /// run exits, plus one extra refused entry when the wait times out.
    applied: Vec<String>,
    refused: Vec<String>,
    shadowed: Vec<String>,
    invalid: Vec<String>,
    /// When the park started. Bounds the wait (see [`RELOAD_WAIT_TIMEOUT`]).
    started: Instant,
    /// Whether the run's exit has been turned into a queued reply.
    /// `false` until the reap or the timeout; afterwards the entry only
    /// drains.
    finished: bool,
    /// When a byte of the reply last went out -- or, if none has, when it
    /// was queued. Bounds the drain (see [`RELOAD_REPLY_STALL`]).
    last_progress: Instant,
    /// The parked connection, taken like `WaitIdle`'s: its socket (sharing
    /// the connection's non-blocking flags), its unsent queue ahead of the
    /// reply, and its connection-table slot.
    client: Option<ParkedClient>,
}

struct ParkedClient {
    stream: UnixStream,
    outbound: Outbound,
    _slot: Option<Slot>,
}

/// What `start_reload_wait` refused before (or without) parking: the reply,
/// plus the connection's queue and slot back. The queue and slot are taken
/// up front (the park needs them on success); dropping them on a refusal
/// would lose replies already queued and leak connection-table capacity.
pub(super) struct ReloadWaitRefused {
    pub(super) response: Response,
    pub(super) outbound: Outbound,
    pub(super) slot: Option<Slot>,
}

impl State {
    /// Parks a wallpaper-wait reload: runs the plain reload, then holds the
    /// connection until the wallpaper run exits or the bound passes.
    /// `Ok(())` means parked (the caller closes the connection -- the reply
    /// comes later through the cloned socket); `Err` answers now, with the
    /// connection's queue and slot handed back.
    pub(super) fn start_reload_wait(
        &mut self,
        stream: UnixStream,
        outbound: Outbound,
        slot: Option<Slot>,
    ) -> Result<(), Box<ReloadWaitRefused>> {
        let refused = |response: Response, outbound: Outbound, slot: Option<Slot>| {
            Box::new(ReloadWaitRefused {
                response,
                outbound,
                slot,
            })
        };
        // Same shared path as `Request::Reload`, minus the reply: the layer
        // resolves here, the file loads and applies below. A failure answers
        // at once, never parked -- there is no run to wait on.
        let layer = crate::compositor::reload::look_layer_state();
        let report = match self.reload_for_wait(layer) {
            Ok(report) => report,
            Err(message) => {
                return Err(refused(Response::error(message), outbound, slot));
            }
        };
        let Some(waited_seq) = self.wallpaper.newest_seq() else {
            // No submission (no `[wallpaper]` section, or nothing new): the
            // run already "exited" before the park. Answer at once, like a
            // plain reload.
            let response = Response::Reloaded {
                applied: report.applied,
                refused: report.refused,
                shadowed: report.shadowed,
                invalid: report.invalid,
                scope: None,
            };
            return Err(refused(response, outbound, slot));
        };
        if self.wallpaper.running_pid().is_none() {
            // The run exited before the park (reaped between submit and
            // here), or the spawn failed and the section is held: either way
            // there is nothing in flight to wait on. Answer at once -- the
            // outcome (or its log line) is already decided.
            let response = Response::Reloaded {
                applied: report.applied,
                refused: report.refused,
                shadowed: report.shadowed,
                invalid: report.invalid,
                scope: None,
            };
            return Err(refused(response, outbound, slot));
        }
        let now = Instant::now();
        self.pending_reload_wait.push(PendingReloadWait {
            waited_seq,
            applied: report.applied,
            refused: report.refused,
            shadowed: report.shadowed,
            invalid: report.invalid,
            started: now,
            finished: false,
            last_progress: now,
            client: Some(ParkedClient {
                stream,
                outbound,
                _slot: slot,
            }),
        });
        Ok(())
    }

    /// Answers every parked wait whose submission has landed: the reaped
    /// run's seq reached (or passed, for a superseded wait) its waited seq.
    /// Called from the wallpaper reaper, after the queue advances and the
    /// next run (if any) starts -- so a second parked wait keeps waiting on
    /// its own run while the first is answered.
    pub(crate) fn answer_ready_reload_waits(&mut self, ended_seq: u64) {
        if self.pending_reload_wait.is_empty() {
            return;
        }
        let mut ready: Vec<PendingReloadWait> = Vec::new();
        let mut waiting: Vec<PendingReloadWait> =
            Vec::with_capacity(self.pending_reload_wait.len());
        for pending in self.pending_reload_wait.drain(..) {
            if !pending.finished && pending.waited_seq <= ended_seq {
                ready.push(pending);
            } else {
                waiting.push(pending);
            }
        }
        self.pending_reload_wait = waiting;
        for mut pending in ready {
            let response = Response::Reloaded {
                applied: std::mem::take(&mut pending.applied),
                refused: std::mem::take(&mut pending.refused),
                shadowed: std::mem::take(&mut pending.shadowed),
                invalid: std::mem::take(&mut pending.invalid),
                scope: None,
            };
            self.answer_reload_wait(pending, response);
        }
    }

    /// Queues the parked reply behind the connection's inherited tail, and
    /// drops the entry when the socket takes it whole. A part-written reply
    /// stays parked for the frame tick; a dead peer drops it there.
    fn answer_reload_wait(&mut self, mut pending: PendingReloadWait, response: Response) {
        let line = match encode(&response) {
            Ok(line) => line,
            Err(_) => return,
        };
        if let Some(client) = pending.client.as_mut() {
            if client.outbound.send(&mut &client.stream, line).is_err() {
                tracing::debug!("dropped a reload-wait reply its client never read");
                return;
            }
            if client.outbound.is_empty() {
                tracing::info!("answered a reload --wait-wallpaper");
                return;
            }
            pending.finished = true;
            pending.last_progress = Instant::now();
            self.pending_reload_wait.push(pending);
            self.ensure_ticking();
        }
    }

    /// Gives up on parks past their bound: answers each with one extra
    /// refused entry reporting handed-over-but-unwaited-on, and hands the
    /// wallpaper run (still going) to the queue's own abandon path. The
    /// run's exit is still reaped and logged when it comes.
    fn timeout_reload_waits(&mut self) {
        if self.pending_reload_wait.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut timed_out: Vec<PendingReloadWait> = Vec::new();
        let mut waiting: Vec<PendingReloadWait> =
            Vec::with_capacity(self.pending_reload_wait.len());
        for mut pending in self.pending_reload_wait.drain(..) {
            if !pending.finished && now.duration_since(pending.started) >= RELOAD_WAIT_TIMEOUT {
                pending.refused.push(
                    "wallpaper (timed out waiting for the wallpaper run; handed over, \
                     exit unwaited-on)"
                        .to_owned(),
                );
                tracing::warn!(
                    waited_seq = pending.waited_seq,
                    timeout_s = RELOAD_WAIT_TIMEOUT.as_secs(),
                    "a reload --wait-wallpaper has not finished; answering handed-over \
                     (the run keeps going, and its exit is logged when it comes)"
                );
                timed_out.push(pending);
            } else {
                waiting.push(pending);
            }
        }
        self.pending_reload_wait = waiting;
        for mut pending in timed_out {
            let response = Response::Reloaded {
                applied: std::mem::take(&mut pending.applied),
                refused: std::mem::take(&mut pending.refused),
                shadowed: std::mem::take(&mut pending.shadowed),
                invalid: std::mem::take(&mut pending.invalid),
                scope: None,
            };
            self.answer_reload_wait(pending, response);
        }
    }

    /// Moves parked waits along by one frame tick: times out hung waits (an
    /// exited run is answered from the SIGCHLD drain instead, so completion
    /// never waits for a tick), and drains queued replies. Returns whether
    /// work remains -- a frame-timer re-arm condition.
    pub fn settle_reload_wait(&mut self) -> bool {
        self.timeout_reload_waits();
        let mut clear: Vec<usize> = Vec::new();
        for (index, pending) in self.pending_reload_wait.iter_mut().enumerate() {
            if !pending.finished {
                continue;
            }
            let Some(client) = pending.client.as_mut() else {
                clear.push(index);
                continue;
            };
            let before = client.outbound.pending();
            match client.outbound.flush(&mut &client.stream) {
                Ok(()) => {
                    if client.outbound.pending() < before {
                        pending.last_progress = Instant::now();
                    }
                    if client.outbound.is_empty() {
                        clear.push(index);
                    } else if Instant::now().duration_since(pending.last_progress)
                        >= RELOAD_REPLY_STALL
                    {
                        tracing::warn!(
                            "gave up writing a reload-wait reply: the client stopped reading"
                        );
                        clear.push(index);
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "dropped a reload-wait reply its client never read");
                    clear.push(index);
                }
            }
        }
        for index in clear.into_iter().rev() {
            self.pending_reload_wait.swap_remove(index);
        }
        !self.pending_reload_wait.is_empty()
    }

    /// Ages every parked wait by `age` (test only): lets the timeout path
    /// pin without parking a test thread for 30 s.
    #[cfg(test)]
    pub(crate) fn age_reload_waits_for_test(&mut self, age: Duration) {
        for pending in &mut self.pending_reload_wait {
            if let Some(earlier) = pending.started.checked_sub(age) {
                pending.started = earlier;
            }
        }
    }
}

/// The reload report a wait parks on: the plain reload's applied/refused
/// plus the look layer's shadowed/invalid lists.
#[derive(Debug)]
pub(crate) struct WaitReport {
    pub(crate) applied: Vec<String>,
    pub(crate) refused: Vec<String>,
    pub(crate) shadowed: Vec<String>,
    pub(crate) invalid: Vec<String>,
}

/// The scope a wait reports: always plain (`None`). Kept as a field for
/// symmetry with `Response::Reloaded`, so a future scoped wait fills it.
#[allow(dead_code)]
fn wait_scope() -> Option<ReloadScope> {
    None
}
