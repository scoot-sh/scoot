//! The control socket: newline-delimited JSON, one connection per client.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use flexwm_core::Action;
use flexwm_ipc::{
    OutputSnapshot, PROTOCOL_VERSION, Rect as WireRect, Request, Response, WindowSnapshot, decode,
    encode, socket_path,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};

use super::State;

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
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;

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
    stream.set_nonblocking(false)?;
    let mut connection = Connection {
        reader: BufReader::new(stream.try_clone()?),
        writer: stream.try_clone()?,
        line: String::new(),
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
    /// fresh String per message -- an agent driving a session over one
    /// connection can send many.
    line: String,
}

impl Connection {
    fn step(&mut self, state: &mut State) -> Step {
        self.line.clear();
        match self.reader.read_line(&mut self.line) {
            Ok(0) | Err(_) => return Step::Close,
            Ok(_) => {}
        }
        let request: Request = match decode(&self.line) {
            Ok(request) => request,
            Err(error) => {
                let _ = self.reply(&Response::error(error.to_string()));
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

        let response = state.handle_request(request);
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
            Request::Key { keys } => match self.press(&keys) {
                Ok(()) => Response::Ok,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(now: Instant, quiet_ms: u64, timeout_ms: u64) -> PendingIdle {
        // A real stream is required by the struct but never touched by
        // idle_outcome; a socket pair is a cheap, sandboxed stand-in.
        let (a, _b) = UnixStream::pair().expect("socket pair");
        PendingIdle {
            stream: a,
            quiet: Duration::from_millis(quiet_ms),
            deadline: now + Duration::from_millis(timeout_ms),
            started: now,
        }
    }

    #[test]
    fn already_quiet_client_still_waits_out_the_quiet_period() {
        let started = Instant::now();
        let wait = pending(started, 200, 5_000);
        // last_commit is long before `started` -- the client was already
        // idle when the request arrived. Immediately after registering,
        // this must NOT report idle: that was the race.
        let long_ago = started - Duration::from_secs(10);
        assert!(matches!(
            idle_outcome(started, long_ago, &wait),
            IdleOutcome::StillWaiting
        ));
        // Not idle either, partway through the quiet window...
        let mid = started + Duration::from_millis(100);
        assert!(matches!(
            idle_outcome(mid, long_ago, &wait),
            IdleOutcome::StillWaiting
        ));
        // ...but idle once quiet_ms has actually elapsed since `started`.
        let after = started + Duration::from_millis(201);
        assert!(matches!(
            idle_outcome(after, long_ago, &wait),
            IdleOutcome::Idle { .. }
        ));
    }

    #[test]
    fn a_commit_during_the_wait_pushes_the_baseline_forward() {
        let started = Instant::now();
        let wait = pending(started, 200, 5_000);
        let commit_at = started + Duration::from_millis(150);
        // 200ms after start, but only 50ms after the commit: still waiting.
        let now = started + Duration::from_millis(200);
        assert!(matches!(
            idle_outcome(now, commit_at, &wait),
            IdleOutcome::StillWaiting
        ));
        let now = commit_at + Duration::from_millis(201);
        assert!(matches!(
            idle_outcome(now, commit_at, &wait),
            IdleOutcome::Idle { .. }
        ));
    }

    #[test]
    fn times_out_when_never_idle_before_the_deadline() {
        let started = Instant::now();
        let wait = pending(started, 200, 500);
        // A commit keeps landing just inside every quiet window, so it's
        // never idle -- but the deadline still fires.
        let now = started + Duration::from_millis(501);
        let last_commit = now - Duration::from_millis(10);
        assert!(matches!(
            idle_outcome(now, last_commit, &wait),
            IdleOutcome::TimedOut
        ));
    }

    #[test]
    fn waited_ms_is_measured_from_the_request_not_the_commit() {
        let started = Instant::now();
        let wait = pending(started, 50, 5_000);
        let now = started + Duration::from_millis(123);
        match idle_outcome(now, started, &wait) {
            IdleOutcome::Idle { waited_ms } => assert_eq!(waited_ms, 123),
            _ => panic!("expected idle"),
        }
    }
}
