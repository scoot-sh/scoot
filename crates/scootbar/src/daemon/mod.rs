//! `scootbar daemon`: the Wayland client, the modules and the control
//! socket, in one thread's `poll` loop.
//!
//! One `poll` over the Wayland fd, the control socket's listener and its
//! clients, and every module's sources (the clock's timerfd), with no
//! timeout: between events the daemon makes no system call at all. No
//! async runtime, no frame callbacks (nothing is animated). Each turn
//! dispatches whatever arrived, serves the control clients (`query`,
//! `reload`, `version`, `kill`, `set`), hands each ready source to its
//! module, then asks every output what to draw ([`draw`]), then flushes
//! and sleeps. Asking after the whole batch means several events that change
//! one output (a scale change arrives as a `wl_output.scale`, a `done`, a
//! `preferred_scale` and a `configure`) are one draw, not four, and a
//! module's change is one draw on every output.
//!
//! ## First frame first
//!
//! Before connecting, every placed module is started and the font loaded
//! (or the daemon refuses to start, saying how to give it one); a module's
//! `init` must not block (`crate::modules`), so nothing slow stands between
//! the start and the first frame.
//!
//! Zero outputs is a normal state (a headless session before its first
//! output, a laptop with the lid shut): nothing to draw, the same poll, no
//! wakeups, until an output's global arrives.
//!
//! The compositor going away or a protocol error end the daemon with exit
//! status 1. **Signals keep their default action**: SIGTERM or SIGINT end it
//! on the spot, which is harmless, since it keeps no state and the
//! compositor removes its surfaces with the connection. SIGHUP is not
//! caught either: catching it would need `unsafe` signal registration
//! (rustix has no `signalfd`), which `#![forbid(unsafe_code)]` forbids, so
//! a reload is `scootbar msg reload` (scootbg documents the same reasoning).

#[cfg(feature = "workspaces")]
mod binds;
mod canvas;
mod listen;
mod respond;
mod surfaces;
mod wayland;
#[cfg(feature = "workspaces")]
mod workspaces;

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Instant;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use wayland_client::backend::WaylandError as BackendError;

use crate::cli::Given;
use crate::config::Config;
use crate::control::paths::{self, PathError};
use crate::control::{Claim, ClaimError, MAX_CONNECTIONS, Server};
use crate::font;
use crate::modules::{self, MAX_POLL, OutputView, Placed, Sources};
use crate::outputs::Plan;
use crate::print::warn;
use crate::render::Style;
use crate::text::Text;
use canvas::Drew;
use listen::Listening;
use respond::Responder;
use wayland::{State, Wayland, WaylandError};

/// What the bars show.
pub struct Content {
    /// The started modules, left to right.
    pub modules: Vec<Placed>,
    /// The font, with its glyph cache; `None` when no module is placed.
    pub text: Option<Text>,
    pub style: Style,
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("modules", &self.modules.len())
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Error {
    Font(font::Error),
    Wayland(WaylandError),
    /// The control socket's paths, claim or server.
    ControlPaths(PathError),
    Claim(ClaimError),
    Control(io::Error),
    /// The compositor closed the connection with nothing left to read.
    CompositorGone,
    /// The connection broke, or the compositor sent a last message (a
    /// protocol error) before closing it.
    Disconnected(BackendError),
    /// A protocol error, or a dispatch that failed.
    Dispatch(wayland_client::DispatchError),
    Poll(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Font(error) => write!(f, "{error}"),
            Self::Wayland(error) => write!(f, "{error}"),
            Self::ControlPaths(error) => write!(f, "{error}"),
            Self::Claim(error) => write!(f, "{error}"),
            Self::Control(error) => write!(f, "control socket: {error}"),
            Self::CompositorGone => write!(
                f,
                "lost the connection to the compositor: it closed the connection"
            ),
            Self::Disconnected(error) => {
                write!(f, "lost the connection to the compositor: {error}")
            }
            Self::Dispatch(error) => write!(f, "Wayland error: {error}"),
            Self::Poll(error) => write!(f, "poll failed: {error}"),
        }
    }
}

/// The poll set, all on the stack: the Wayland connection and the modules'
/// sources ([`MAX_POLL`]), then the control listener and its clients.
const MAX_FDS: usize = MAX_POLL + 1 + MAX_CONNECTIONS;

/// Everything a start needs before it touches the system: the placed modules
/// started and the font loaded, or the reason it cannot go on. Shared with
/// [`check`] so `--check` gives exactly the errors a start does.
fn prepare(config: &Config) -> Result<Content, Error> {
    let modules = modules::start(
        &config.outputs.to_start(&config.layout),
        &config.modules,
        &mut |id, why| {
            warn(format_args!(
                "scootbar: note: the {id} module is unavailable, and left out: {why}"
            ));
        },
    );
    // A font only if something will draw text: with none usable, refuse to
    // start (the error says how to give one).
    let text = if modules.is_empty() {
        None
    } else {
        let text =
            font::text(config.font.as_deref(), &config.fallback_fonts).map_err(Error::Font)?;
        Some(text)
    };
    Ok(Content {
        modules,
        text,
        style: config.style(),
    })
}

/// `scootbar daemon --check`: what a start does before it connects, and
/// nothing after: no compositor, no control socket.
pub fn check(config: &Config) -> Result<(), Error> {
    prepare(config).map(drop)
}

/// Runs the bar until the compositor goes away, it cannot go on, or
/// `scootbar msg kill` stops it. `file` is the config file a `reload`
/// re-reads; `given` are the daemon flags, overlaid onto every reload as
/// at start-up, so the precedence (defaults, then the file, then the
/// flags) holds for the running bar at all times.
pub fn run(config: Config, file: Option<PathBuf>, given: Given) -> Result<(), Error> {
    let content = prepare(&config)?;
    // The control socket first, so a second daemon refuses before it
    // touches the compositor.
    let paths = paths::from_env().map_err(Error::ControlPaths)?;
    let mut claim = Claim::acquire(&paths).map_err(Error::Claim)?;
    let mut server = Server::new(claim.listener()).map_err(Error::Control)?;
    let mut listening = Listening::default();
    let (mut wayland, missing) = Wayland::connect(
        crate::policy::Placement {
            bar: config.bar,
            layout: config.layout.clone(),
            policy: config.outputs.clone(),
        },
        content,
        #[cfg(feature = "workspaces")]
        config.modules.workspaces.link.clone(),
    )
    .map_err(Error::Wayland)?;
    for interface in missing {
        warn(format_args!(
            "scootbar: note: the compositor has no {interface}; the bar is drawn at the \
             integer scale and scaled down on fractionally scaled outputs"
        ));
    }
    #[cfg(feature = "workspaces")]
    if config
        .outputs
        .to_start(&config.layout)
        .placed()
        .any(|(_, id)| id == crate::modules::workspaces::ID)
    {
        if !wayland.state.workspaces.0.borrow().has_manager() {
            warn(format_args!(
                "scootbar: note: the compositor has no ext_workspace_manager_v1; the \
                 workspaces module is left out"
            ));
        }
        if wayland.state.pointer.is_none() && wayland.state.globals.seat.is_none() {
            warn(format_args!(
                "scootbar: note: the compositor has no wl_seat; clicks on the workspaces \
                 module do nothing"
            ));
        }
    }
    let mut wants_write = false;
    loop {
        wayland
            .queue
            .dispatch_pending(&mut wayland.state)
            .map_err(Error::Dispatch)?;
        // What the dispatch brought (workspace batches, pointer presses
        // are routed in their own dispatch): each module reports, then the
        // loop draws what moved.
        for placed in wayland.state.content.modules.iter_mut() {
            placed.dispatch();
        }
        // The control clients: drain the listener, then serve each client
        // what the last poll reported. A failed accept rests the listener
        // rather than spins or exits (`listen`).
        let (listen, timeout) = listening.poll_plan(Instant::now);
        if listen {
            if let Err(error) = server.accept(claim.listener()) {
                warn(format_args!(
                    "scootbar: cannot accept control clients: {error}"
                ));
                listening.rest(Instant::now());
            }
        }
        draw(&mut wayland.state, &wayland.qh);
        flush(&wayland, &mut wants_write)?;
        let Some(guard) = wayland.queue.prepare_read() else {
            // Events arrived for our queue meanwhile: dispatch them.
            continue;
        };
        let mut events = PollFlags::IN;
        if wants_write {
            events |= PollFlags::OUT;
        }
        let connection = guard.connection_fd();
        // The Wayland fd first, then each module's sources, then the
        // control listener and its clients: all on the stack, so a turn
        // of the loop allocates nothing.
        let mut fds: [PollFd<'_>; MAX_FDS] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(connection, PollFlags::empty()));
        let mut owners = [(0usize, 0usize); MAX_POLL];
        fds[0] = PollFd::from_borrowed_fd(connection, events);
        let mut len = 1;
        for (index, placed) in wayland.state.content.modules.iter().enumerate() {
            let mut sources = Sources::new(&mut fds, &mut owners, &mut len, index);
            placed.module.sources(&mut sources);
        }
        // The modules' poll slots end here; the ready flags below must
        // not read past them (the control slots reuse no owners).
        let sources = len;
        if listen {
            if let Some(slot) = fds.get_mut(len) {
                *slot = PollFd::new(claim.listener(), PollFlags::IN);
                len += 1;
            }
        }
        let conns_at = len;
        for conn in server.conns() {
            let Some(slot) = fds.get_mut(len) else {
                break;
            };
            *slot = PollFd::new(conn.stream(), conn.interest());
            len += 1;
        }
        let polled = fds.get_mut(..len).unwrap_or_default();
        let timeout = timeout.map(timespec);
        match poll(polled, timeout.as_ref()) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(errno) => return Err(Error::Poll(errno.into())),
        }
        let mut ready = [PollFlags::empty(); MAX_FDS];
        for (flags, fd) in ready.iter_mut().zip(&fds[..len]) {
            *flags = fd.revents();
        }
        let revents = ready[0];
        if revents.intersects(PollFlags::OUT) {
            wants_write = false;
        }
        if revents.intersects(PollFlags::HUP | PollFlags::ERR)
            && rustix::io::ioctl_fionread(guard.connection_fd()).unwrap_or(0) == 0
        {
            // The compositor is gone and sent nothing more. Stop here
            // rather than let the backend read the EOF, which without its
            // `log` feature it reports with `eprintln!` (scootbg's loop
            // explains). Anything still readable (a protocol error) is read
            // below instead.
            return Err(Error::CompositorGone);
        }
        // The control clients: a `reload` applies before the modules are
        // asked, and a `kill` stops before anything is drawn.
        {
            let mut responder = Responder::new(
                &mut wayland.state,
                &wayland.conn,
                &wayland.qh,
                file.as_ref(),
                &given,
            );
            let mut index = 0;
            while index < server.conns().len() {
                let revents = ready
                    .get(conns_at + index)
                    .copied()
                    .unwrap_or(PollFlags::empty());
                if server.service(index, revents, &mut responder) {
                    index += 1;
                }
            }
            if responder.stop {
                // Answer owed (`kill`'s reply), then close: by the time the
                // client sees the close, the socket file is gone and the
                // lock released, so a new daemon starts straight away.
                server.close_all();
                claim.release();
                return Ok(());
            }
        }
        // Whatever the clients asked of the bars' visibility: once, with
        // the net result of the whole batch.
        wayland.state.apply_visibility(&wayland.conn, &wayland.qh);
        if revents.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            match guard.read() {
                Ok(_) => {}
                Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(Error::Disconnected(error)),
            }
        } else {
            drop(guard);
        }
        // Then the modules, each ready source once; a changed view is
        // drawn at the top of the next turn.
        let modules = &mut wayland.state.content.modules;
        for (flags, &(module, source)) in ready.iter().zip(&owners).take(sources).skip(1) {
            if flags.is_empty() {
                continue;
            }
            if let Some(placed) = modules.get_mut(module) {
                placed.ready(source, *flags);
            }
        }
    }
}

/// Carries out every output's [`Plan`]: a draw, a bare commit, or nothing.
/// Walks the outputs and allocates nothing unless a buffer must be made
/// (the first draw, or a new size) or a glyph is drawn for the first time.
fn draw(state: &mut State, qh: &wayland_client::QueueHandle<State>) {
    let globals = &state.globals;
    let content = &mut state.content;
    for entry in state.outputs.iter_mut() {
        let stale = entry
            .objects
            .scene
            .stale(&content.modules, entry.objects.canvas.shown());
        match entry.output.plan(&entry.objects.bar, stale) {
            Plan::Nothing => {}
            Plan::Commit => {
                if let Some(layer) = &entry.objects.layer {
                    layer.surface.commit();
                }
                entry.output.committed();
            }
            Plan::Draw(frame) => {
                let objects = &mut entry.objects;
                let Some(layer) = objects.layer.as_mut() else {
                    continue;
                };
                let id = entry.output.id();
                let output = OutputView {
                    name: entry.output.info().name.as_deref(),
                };
                match objects.canvas.draw(
                    globals,
                    qh,
                    id,
                    layer,
                    frame,
                    &mut objects.scene,
                    &output,
                    content,
                ) {
                    Ok(Drew::Committed) => entry.output.drew(frame),
                    // A release wakes the loop, and this plan is asked
                    // again then.
                    Ok(Drew::Stalled) => {}
                    Err(error) => {
                        if entry.output.draw_failed(frame) {
                            warn(format_args!(
                                "scootbar: cannot draw the bar on {}: {error}",
                                entry.output.label()
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn flush(wayland: &Wayland, wants_write: &mut bool) -> Result<(), Error> {
    match wayland.conn.flush() {
        Ok(()) => Ok(()),
        Err(BackendError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
            *wants_write = true;
            Ok(())
        }
        Err(error) => Err(Error::Disconnected(error)),
    }
}

/// A poll timeout. At most `listen::REST`, so it always fits; a second
/// is the fallback all the same, never a timeout of zero (a spin).
fn timespec(duration: std::time::Duration) -> rustix::event::Timespec {
    rustix::event::Timespec::try_from(duration).unwrap_or(rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    })
}
