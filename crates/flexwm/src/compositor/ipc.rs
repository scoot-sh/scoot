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
}

impl Connection {
    fn step(&mut self, state: &mut State) -> Step {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => return Step::Close,
            Ok(_) => {}
        }
        let request: Request = match decode(&line) {
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
            Request::WaitIdle { .. } => Response::error("wait-idle is answered by the render loop"),
        }
    }

    /// Answers `wait-idle` requests whose quiet period has passed, or timed out.
    pub fn settle_idle_waiters(&mut self) {
        if self.pending_idle.is_empty() {
            return;
        }
        let now = Instant::now();
        let quiet_for = now.duration_since(self.last_commit);
        let mut waiting = std::mem::take(&mut self.pending_idle);
        waiting.retain_mut(|wait| {
            let idle = quiet_for >= wait.quiet;
            if !idle && now < wait.deadline {
                return true;
            }
            let waited_ms = now.duration_since(wait.started).as_millis() as u64;
            let response = if idle {
                Response::Idle { waited_ms }
            } else {
                Response::error("timed out waiting for the screen to settle")
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
