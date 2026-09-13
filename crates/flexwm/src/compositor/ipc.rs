//! The control socket: newline-delimited JSON, one connection per client.
//!
//! Every byte in and out of here moves on the compositor's *only* thread, the
//! one that also runs wayland dispatch, input and the render loop -- so
//! nothing in this module may block, ever. A blocking read of half a request
//! line, or a blocking write to a client that has stopped reading, does not
//! stall one connection: it stalls the whole compositor, for every client, for
//! as long as the offender likes. Both used to happen (see `ROADMAP.md`'s
//! entry for this item). The shape that replaces them:
//!
//! - the accepted socket is non-blocking, and a line that has only half
//!   arrived leaves its bytes in [`line::Lines`] to be finished on a later
//!   wakeup ([`line::LineRead::Incomplete`]);
//! - one wakeup answers *every* request already buffered, not just the first,
//!   because a level-triggered readiness source will not fire again for bytes
//!   that have already left the kernel;
//! - a reply the socket will not take in one go waits in [`outbound::Outbound`]
//!   and goes out when the event loop reports the socket writable, with the
//!   connection's read interest dropped while too much is queued so it cannot
//!   be made to buffer without bound.
//!
//! This file holds the socket's setup and what a request *means*
//! ([`State::handle_request`]); [`connection`] holds the event-loop machinery
//! that gets requests in and replies out.

mod connection;
mod line;
mod listener;
mod outbound;
#[cfg(test)]
mod tests;

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use flexwm_core::Action;
use flexwm_ipc::{
    OutputSnapshot, PROTOCOL_VERSION, Rect as WireRect, Request, Response, WindowSnapshot, encode,
    socket_path,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};

use self::outbound::Outbound;
use super::State;
use super::headless::FRAME_INTERVAL;
use super::tty::VtSwitchOutcome;

/// A `wait-idle` request that hasn't been answered yet.
///
/// Its connection has already left the event loop (see the `WaitIdle` arm of
/// `Connection::serve`), so this carries everything needed to finish with it
/// from the render loop: the socket, when the request started and how long it
/// may run, and the outbound queue it inherited.
pub struct PendingIdle {
    stream: UnixStream,
    /// How long the screen must stay unchanged before this is answered.
    quiet: Duration,
    /// How long the client is prepared to wait in total -- both for the screen
    /// to settle and, afterwards, for its answer to be written (see
    /// [`PendingIdle::push`]).
    timeout: Duration,
    started: Instant,
    /// When this waiter's queue last got smaller. Only meaningful once
    /// something has been queued; until then it is just `started`.
    last_progress: Instant,
    /// Whether this request's own answer has been decided and queued.
    ///
    /// Not derivable from `outbound`: that is also non-empty *before* the
    /// answer exists, when it holds a tail inherited from the connection, and
    /// empty again once the answer has gone out.
    answered: bool,
    /// This request's answer on its way out, plus -- ahead of it -- whatever
    /// the connection had not finished writing when it handed over.
    outbound: Outbound,
}

pub fn init(
    event_loop: &mut EventLoop<'static, State>,
    state: &mut State,
    socket: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = socket
        .or_else(socket_path)
        .ok_or("no socket path: set FLEXWM_SOCKET or XDG_RUNTIME_DIR")?;
    let listener = listener::bind(&path)?;

    event_loop.handle().insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        |_, listener, state: &mut State| {
            while let Ok((stream, _)) = listener.accept() {
                if let Err(error) = accept(state, stream) {
                    tracing::warn!(%error, "could not take an ipc client");
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    state.ipc_path = Some(path);
    Ok(())
}

fn accept(state: &mut State, stream: UnixStream) -> std::io::Result<()> {
    // First, before this connection costs anything: an fd duplicated, a
    // buffer allocated, a place in the event loop -- and long before any
    // request of its own is read. A client that is not this compositor's own
    // user gets nothing but a closed socket.
    let peer = listener::peer_uid(&stream)?;
    let own = listener::own_uid();
    if !listener::peer_is_allowed(peer, own) {
        // Loud on purpose. This channel injects keystrokes and hands back
        // screenshots, so somebody else's process reaching it at all is
        // worth a trace at the default log level, the same way a discarded
        // VT switch is (see `tty::change_vt`) -- it is refused, not an
        // error, but it is never routine.
        tracing::warn!(
            uid = peer,
            expected_uid = own,
            "refused an ipc connection from another user",
        );
        return Ok(());
    }

    // Non-blocking is what every read and write in `connection` is written
    // for, and it is not inherited: on Linux `accept(2)` gives a fresh socket
    // its own file status flags, and std asks for `SOCK_CLOEXEC` only.
    // Verified on the dev VM rather than assumed -- with the listener itself
    // already non-blocking, `fcntl(F_GETFL) & O_NONBLOCK` on the accepted fd
    // reads false. Set explicitly either way: this is load-bearing enough that
    // it should not depend on what the listener happens to be.
    stream.set_nonblocking(true)?;
    state
        .loop_handle
        .insert_source(
            connection::source(stream)?,
            |_readiness, connection, state: &mut State| connection.step(state),
        )
        .map_err(std::io::Error::other)?;
    Ok(())
}

impl State {
    pub fn handle_request(&mut self, request: Request) -> Response {
        match request {
            Request::Version => Response::Version {
                version: env!("CARGO_PKG_VERSION").to_string(),
                protocol: PROTOCOL_VERSION,
            },
            Request::Outputs => Response::Outputs {
                outputs: self.output_snapshots(),
            },
            Request::Windows => Response::Windows {
                windows: self.window_snapshots(),
            },
            Request::Action(action) => {
                self.act(Action::from(action));
                Response::Ok
            }
            Request::Screenshot { .. } => match self.screenshot() {
                Ok(screenshot) => Response::Screenshot(screenshot),
                Err(error) => Response::error(error),
            },
            Request::PointerMove { x, y } => {
                self.pointer_move(x, y);
                Response::Ok
            }
            Request::PointerButton { button, pressed } => {
                self.pointer_button(button, pressed);
                Response::Ok
            }
            Request::Click { x, y, button } => {
                self.pointer_move(x, y);
                self.pointer_button(button, true);
                self.pointer_button(button, false);
                Response::Ok
            }
            Request::Scroll { dx, dy } => {
                self.scroll(dx, dy);
                Response::Ok
            }
            // Three of `press()`'s `Ok(Some(VtSwitchOutcome))` cases need
            // something other than a bare `Ok` -- each for a different
            // reason, so each gets its own arm rather than folding them
            // together:
            //
            // - `Requested`: this exact call just issued a real VT_ACTIVATE
            //   that libseat didn't reject outright, so this IPC connection
            //   -- if it's the caller's only input path -- may be about to
            //   lose the one channel that could switch back (see
            //   `tty::VtSwitchOutcome`'s doc and the backlog item this
            //   closes in `ROADMAP.md`). Worded as "requested"/"if it takes
            //   effect," not "this switched" -- libseat's own docs say a
            //   successful switch_session call doesn't guarantee a switch
            //   happens (confirmed on real hardware: requesting the VT
            //   already showing is also `Ok(())`, with no pause at all), so
            //   claiming a definite pause here would overclaim on that
            //   no-op case.
            // - `IgnoredPaused`: the request went nowhere (libseat was never
            //   asked), but *why* matters to an IPC caller specifically --
            //   this is the one-way-door scenario itself: an agent retrying
            //   its switch-back combo over IPC while paused needs to hear
            //   "this can't work from here," not a bare `Ok` indistinguishable
            //   from a real switch-back actually working (see 5b/5c in
            //   `ROADMAP.md` for why that ambiguity is exactly the defect).
            // - `Failed`: libseat itself returned an error -- a request that
            //   did not succeed, so unlike `Requested` this isn't "success
            //   with a side effect," it's a plain failure and gets
            //   `Response::error` rather than `Warning`.
            //
            // Plain `Ignored` (no `--tty` backend at all -- this request
            // means nothing on this backend, nothing to warn about) and
            // `None` (no VT binding matched at all) stay a plain `Ok`.
            Request::Key { keys } => match self.press(&keys) {
                Ok(Some(VtSwitchOutcome::Requested)) => Response::Warning {
                    message: "requested a VT switch away from --tty (libseat \
                              does not guarantee this actually happens, e.g. \
                              it is a no-op if already on the target VT); if \
                              it takes effect, the compositor will be paused, \
                              and only a VT switch from outside this \
                              compositor -- a physical Ctrl+Alt+Fn, or \
                              `sudo chvt N` from a shell on this machine -- \
                              not IPC, can reactivate it"
                        .to_string(),
                },
                Ok(Some(VtSwitchOutcome::IgnoredPaused)) => Response::Warning {
                    message: "ignored: this session is already paused; a VT \
                              switch from outside this compositor -- a \
                              physical Ctrl+Alt+Fn, or `sudo chvt N` from a \
                              shell on this machine -- is needed before a VT \
                              switch can succeed from here; other IPC \
                              requests still work"
                        .to_string(),
                },
                Ok(Some(VtSwitchOutcome::Failed)) => Response::error(
                    "requested a VT switch away from --tty, but libseat \
                     rejected the request (see the compositor log for why)",
                ),
                // Spelled out rather than `Ok(_)`: a collapsed catch-all is
                // exactly the shape of bug that made the paused-retry case
                // silently indistinguishable from success above (see
                // `VtSwitchOutcome::Ignored`'s doc) -- keeping this
                // exhaustive means a future fifth `VtSwitchOutcome` variant
                // fails to compile here instead of silently becoming `Ok`.
                Ok(None) | Ok(Some(VtSwitchOutcome::Ignored)) => Response::Ok,
                Err(error) => Response::error(error),
            },
            Request::Type { text } => match self.type_text(&text) {
                Ok(()) => Response::Ok,
                Err(error) => Response::error(error),
            },
            // Connection::step() intercepts and answers this variant itself
            // (see above) before handle_request is ever called, since a
            // reply here has to wait on pending_idle instead of being
            // returned immediately like every other request. If this ever
            // fires, that interception was bypassed -- a bug worth a loud
            // failure, not a request that quietly appears to work while
            // answering wrong.
            Request::WaitIdle { .. } => unreachable!("WaitIdle is answered by Connection::step"),
        }
    }

    /// Answers `wait-idle` requests whose quiet period has passed, or timed
    /// out, and pushes out the ones whose answer didn't fit in one write.
    pub fn settle_idle_waiters(&mut self) {
        if self.pending_idle.is_empty() {
            return;
        }
        let now = Instant::now();
        let last_commit = self.last_commit;
        let mut waiting = std::mem::take(&mut self.pending_idle);
        waiting.retain_mut(|wait| wait.advance(now, last_commit));
        self.pending_idle = waiting;
    }

    fn window_snapshots(&self) -> Vec<WindowSnapshot> {
        let arrangement = self.world.arrange();
        arrangement
            .placements
            .iter()
            .map(|placement| {
                let info = self
                    .world
                    .window_info(placement.id)
                    .cloned()
                    .unwrap_or_default();
                WindowSnapshot {
                    id: placement.id.0,
                    app_id: info.app_id,
                    title: info.title,
                    output: placement.output.0,
                    rect: wire(placement.rect),
                    visible: placement.visible,
                    focused: arrangement.focused == Some(placement.id),
                }
            })
            .collect()
    }

    fn output_snapshots(&self) -> Vec<OutputSnapshot> {
        let name = self
            .output
            .as_ref()
            .map(|output| output.name())
            .unwrap_or_default();
        self.world
            .outputs()
            .into_iter()
            .map(|(id, area)| OutputSnapshot {
                id: id.0,
                name: name.clone(),
                rect: wire(area),
            })
            .collect()
    }
}

fn wire(rect: flexwm_core::Rect) -> WireRect {
    WireRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    }
}

/// Whether a screenshot request arriving at `now` should be refused because
/// this connection was already handed one less than a frame ago.
///
/// `last` is when the previous capture *finished* (see the call site), so the
/// window this enforces is a gap *between* captures: one connection can cost
/// the event loop a capture no more often than the compositor already spends
/// a frame, and everything else gets that gap to be served in. A refused
/// caller waits at most one [`FRAME_INTERVAL`] and asks again.
///
/// Deliberately not justified as "the pixels cannot have changed yet" --
/// that would be a stronger claim than the code makes good on. `render()`
/// runs on demand (`needs_render`), not on frame boundaries, and this window
/// starts whenever the last capture happened to finish, so two captures a
/// frame apart can legitimately differ. Bounding the *cost* is the point.
///
/// Deliberately not a sleep: this runs on the event-loop thread, so waiting
/// here would block every other client in order to slow one down. A refusal
/// is answered in microseconds, so a client that ignores it and hammers
/// anyway costs the compositor a JSON reply per attempt, not a render.
///
/// Per connection, not global, and so bypassable by reconnecting for every
/// capture -- capping concurrent connections is the audit's separate finding
/// and deliberately not in scope here. What this does close is the case the
/// finding described: one connection issuing back-to-back captures.
///
/// `duration_since` saturates to zero rather than panicking when `last` is
/// somehow later than `now`, so a clock that fails to be monotonic makes this
/// throttle (harmlessly) rather than abort the compositor.
fn screenshot_throttled(last: Option<Instant>, now: Instant) -> bool {
    last.is_some_and(|last| now.duration_since(last) < FRAME_INTERVAL)
}

impl PendingIdle {
    /// Moves this waiter along by one frame tick: `true` to keep waiting,
    /// `false` once it is finished with (answered and written, or given up on).
    fn advance(&mut self, now: Instant, last_commit: Instant) -> bool {
        // Anything queued goes first, for the same reason the connection did
        // it first: this waiter may have inherited the tail of an earlier
        // reply, and the answer below must not overtake it.
        if !self.push(now) {
            return false;
        }
        if !self.answered {
            let response = match idle_outcome(now, last_commit, self) {
                IdleOutcome::StillWaiting => return true,
                IdleOutcome::Idle { waited_ms } => Response::Idle { waited_ms },
                IdleOutcome::TimedOut => {
                    Response::error("timed out waiting for the screen to settle")
                }
            };
            let Ok(line) = encode(&response) else {
                // `Response::Idle` and `Response::Error` are a `u64` and a
                // `String`; serde cannot fail on either. Nothing to answer
                // with if it somehow did.
                return false;
            };
            self.answered = true;
            let PendingIdle {
                stream, outbound, ..
            } = self;
            if outbound.send(&mut &*stream, line).is_err() {
                return false;
            }
        }
        // Kept only while there is still something to write.
        !self.outbound.is_empty()
    }

    /// Pushes out as much of the queue as the socket will take. `false` when
    /// this waiter is finished with: the peer is gone, or it has stopped making
    /// room for long enough to count as gone.
    ///
    /// The socket is non-blocking (it is a `try_clone` of the connection's,
    /// which shares its file status flags -- verified, not assumed), so a
    /// write here can come up short even for an answer this small: a client
    /// that pipelined requests and never read the replies has its own receive
    /// buffer full. Retried on the next frame tick instead of dropped, because
    /// dropping it would leave the client waiting on an answer that was
    /// decided and then thrown away -- and, when there is an inherited tail
    /// ahead of it, would truncate a response mid-way.
    ///
    /// Giving up is bounded by lack of *progress*, not by total time: a client
    /// draining a multi-megabyte screenshot reply slowly is making progress and
    /// is never given up on, however long it takes. The window is the same
    /// `timeout_ms` the client itself asked for -- it said how long it was
    /// prepared to wait on this request, and a peer that has not taken a single
    /// byte in that long is not reading at all.
    fn push(&mut self, now: Instant) -> bool {
        if self.outbound.is_empty() {
            return true;
        }
        let before = self.outbound.pending();
        let PendingIdle {
            stream, outbound, ..
        } = self;
        if outbound.flush(&mut &*stream).is_err() {
            return false;
        }
        if self.outbound.pending() < before {
            self.last_progress = now;
            return true;
        }
        if now.duration_since(self.last_progress) >= self.timeout {
            tracing::warn!(
                pending = self.outbound.pending(),
                "gave up writing a wait-idle reply: the client stopped reading"
            );
            return false;
        }
        true
    }
}

enum IdleOutcome {
    StillWaiting,
    Idle { waited_ms: u64 },
    TimedOut,
}

/// Whether a `wait-idle` request should be answered yet.
///
/// "Idle" means `quiet` has elapsed with no commit *since the request was
/// made* -- not merely since whenever the last commit happened to be. Using
/// `last_commit` alone would let a request reply idle on the very first tick
/// whenever the client was already commit-idle when it arrived, before the
/// client has had any chance to react to input sent moments earlier: the
/// same "stale screenshot" race the flush bugs produced, but from a missing
/// baseline instead of a missing flush. So the quiet window is measured from
/// `last_commit.max(wait.started)`, which only equals `last_commit` once a
/// commit has actually happened after the request began.
///
/// The timeout is a duration since `started` rather than a precomputed
/// deadline `Instant`: `Instant + Duration` panics on overflow, and
/// `timeout_ms` is a client-chosen `u64` that goes straight into a `Duration`.
/// Comparing durations instead means there is no addition to overflow.
fn idle_outcome(now: Instant, last_commit: Instant, wait: &PendingIdle) -> IdleOutcome {
    let baseline = last_commit.max(wait.started);
    if now.duration_since(baseline) >= wait.quiet {
        let waited_ms = now.duration_since(wait.started).as_millis() as u64;
        IdleOutcome::Idle { waited_ms }
    } else if now.duration_since(wait.started) >= wait.timeout {
        IdleOutcome::TimedOut
    } else {
        IdleOutcome::StillWaiting
    }
}
