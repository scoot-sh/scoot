//! The control socket: newline-delimited JSON, one connection per client.

mod line;
mod listener;
#[cfg(test)]
mod tests;

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use flexwm_core::Action;
use flexwm_ipc::{
    OutputSnapshot, PROTOCOL_VERSION, Rect as WireRect, Request, Response, WindowSnapshot, decode,
    encode, socket_path,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};

use self::line::{LineRead, MAX_REQUEST_BYTES, read_line_bounded};
use super::State;
use super::headless::FRAME_INTERVAL;
use super::tty::VtSwitchOutcome;

/// A `wait-idle` request that hasn't been answered yet.
pub struct PendingIdle {
    pub stream: UnixStream,
    pub quiet: Duration,
    pub deadline: Instant,
    pub started: Instant,
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

    stream.set_nonblocking(false)?;
    let mut connection = Connection {
        reader: BufReader::new(stream.try_clone()?),
        writer: stream.try_clone()?,
        line: Vec::new(),
        last_screenshot: None,
    };
    let source = Generic::new(stream, Interest::READ, Mode::Level);
    state
        .loop_handle
        .insert_source(source, move |_, _, state: &mut State| {
            Ok(match connection.step(state) {
                Step::Continue => PostAction::Continue,
                Step::Close => PostAction::Remove,
            })
        })
        .map_err(std::io::Error::other)?;
    Ok(())
}

enum Step {
    Continue,
    Close,
}

struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// Reused across requests on this connection instead of allocating a
    /// fresh buffer per message -- an agent driving a session over one
    /// connection can send many.
    line: Vec<u8>,
    /// When this connection last had a screenshot captured for it, or `None`
    /// if it never has. Per connection, not global: one client hammering the
    /// request must not make another client's first one fail.
    last_screenshot: Option<Instant>,
}

impl Connection {
    fn step(&mut self, state: &mut State) -> Step {
        match read_line_bounded(&mut self.reader, &mut self.line, MAX_REQUEST_BYTES) {
            LineRead::Line => {}
            LineRead::Eof | LineRead::Failed => return Step::Close,
            LineRead::TooLong => {
                // Nothing to resynchronize to: the rest of this line is
                // still coming and there is no way to tell where it ends.
                // Best-effort reply so a legitimately over-long request gets
                // a reason rather than a bare disconnection, then done.
                let _ = self.reply(&Response::error(format!(
                    "request line exceeds the {MAX_REQUEST_BYTES}-byte limit; connection closed"
                )));
                tracing::debug!("closed an ipc connection whose request line ran past the limit");
                return Step::Close;
            }
        }
        // Borrowed only until the request (or an owned error message) is
        // out; `self.line` is needed mutably again the moment either is.
        let decoded = match std::str::from_utf8(&self.line) {
            Ok(text) => decode::<Request>(text).map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        let request = match decoded {
            Ok(request) => request,
            Err(message) => {
                let _ = self.reply(&Response::error(message));
                return Step::Continue;
            }
        };

        // Waiting is answered later, from the render loop, so the connection
        // leaves this source and lives on in `pending_idle`.
        if let Request::WaitIdle {
            quiet_ms,
            timeout_ms,
        } = request
        {
            let Ok(stream) = self.writer.try_clone() else {
                return Step::Close;
            };
            let now = Instant::now();
            state.pending_idle.push(PendingIdle {
                stream,
                quiet: Duration::from_millis(quiet_ms),
                deadline: now + Duration::from_millis(timeout_ms),
                started: now,
            });
            // The frame timer answers this, but it drops itself when there's
            // nothing to do -- if the compositor was already idle, it needs
            // waking back up or this request would wait forever.
            state.ensure_ticking();
            return Step::Close;
        }

        // A screenshot is the one request that costs a full render, a
        // framebuffer read-back and a PNG encode, all on the single thread
        // that also runs wayland dispatch, input and every other IPC
        // connection (see `screenshot.rs`). Served back-to-back it starves
        // everything else, so a connection that was handed one less than a
        // frame ago is told to come back rather than served a second one at
        // that price.
        let screenshot = matches!(request, Request::Screenshot { .. });
        if screenshot && screenshot_throttled(self.last_screenshot, Instant::now()) {
            let _ = self.reply(&Response::error(format!(
                "screenshots are limited to one per connection per {}ms, the \
                 compositor's own frame interval: capturing costs a full \
                 render and encode on the thread that serves every other \
                 client. Retry after that long",
                FRAME_INTERVAL.as_millis()
            )));
            return Step::Continue;
        }

        let response = state.handle_request(request);
        if screenshot {
            // Stamped once the capture is done, not when the request
            // arrived: the whole point is to leave the event loop a frame's
            // worth of room for everything else *after* a capture, and a
            // capture can easily take longer than a frame itself, which
            // would leave a start-stamped window already expired by the time
            // it mattered (measured: ~170ms for 800x600 in a debug build --
            // the throttle never once fired that way). Recorded whether or
            // not the capture succeeded: `screenshot()` renders before it
            // can fail, so the expensive part was spent either way.
            self.last_screenshot = Some(Instant::now());
        }
        // Synthetic input (key/pointer) and action-driven configures queue
        // wayland messages on the client's connection; nothing else flushes
        // them until the next render tick, which only runs when something
        // already marked the screen dirty. Flush explicitly so an injected
        // keystroke reaches the client the moment it's sent, not whenever a
        // later, unrelated redraw happens to piggyback it out.
        let _ = state.display_handle.flush_clients();
        if self.reply(&response).is_err() {
            Step::Close
        } else {
            Step::Continue
        }
    }

    fn reply(&mut self, response: &Response) -> std::io::Result<()> {
        self.writer.write_all(encode(response)?.as_bytes())?;
        self.writer.flush()
    }
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

    /// Answers `wait-idle` requests whose quiet period has passed, or timed out.
    pub fn settle_idle_waiters(&mut self) {
        if self.pending_idle.is_empty() {
            return;
        }
        let now = Instant::now();
        let last_commit = self.last_commit;
        let mut waiting = std::mem::take(&mut self.pending_idle);
        waiting.retain_mut(|wait| {
            let response = match idle_outcome(now, last_commit, wait) {
                IdleOutcome::StillWaiting => return true,
                IdleOutcome::Idle { waited_ms } => Response::Idle { waited_ms },
                IdleOutcome::TimedOut => {
                    Response::error("timed out waiting for the screen to settle")
                }
            };
            if let Ok(line) = encode(&response) {
                let _ = wait.stream.write_all(line.as_bytes());
                let _ = wait.stream.flush();
            }
            false
        });
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
fn idle_outcome(now: Instant, last_commit: Instant, wait: &PendingIdle) -> IdleOutcome {
    let baseline = last_commit.max(wait.started);
    if now.duration_since(baseline) >= wait.quiet {
        let waited_ms = now.duration_since(wait.started).as_millis() as u64;
        IdleOutcome::Idle { waited_ms }
    } else if now >= wait.deadline {
        IdleOutcome::TimedOut
    } else {
        IdleOutcome::StillWaiting
    }
}
