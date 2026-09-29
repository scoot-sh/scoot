//! `scootbar daemon`: the Wayland client and the modules, in one thread's
//! `poll` loop.
//!
//! One `poll` over the Wayland fd and every module's sources (the clock's
//! timerfd), with no timeout: between events the daemon makes no system
//! call at all. No async runtime, no frame callbacks (nothing is animated).
//! Each turn dispatches whatever arrived, hands each ready source to its
//! module, then asks every output what to draw ([`draw`]), then flushes and
//! sleeps. Asking after the whole batch means several events that change
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
//! compositor removes its surfaces with the connection.

mod canvas;
mod surfaces;
mod wayland;

use std::fmt;
use std::io;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use wayland_client::backend::WaylandError as BackendError;

use crate::config::Config;
use crate::font;
use crate::modules::{self, OutputView, Placed, Sources};
use crate::outputs::Plan;
use crate::print::warn;
use crate::render::{Scene, Style};
use crate::text::Text;
use canvas::Drew;
use wayland::{State, Wayland, WaylandError};

/// The most fds polled in one turn: the Wayland connection and every
/// module's sources. The registry's modules add one each, so this is far
/// more than a layout of [`crate::layout::MAX_MODULES`] can use; past it a
/// source is not polled (`Sources::add` says so).
const MAX_POLL: usize = 64;

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

/// Runs the bar until the compositor goes away or it cannot go on.
pub fn run(config: Config) -> Result<(), Error> {
    let modules = modules::start(&config.layout, &config.modules, &mut |id, why| {
        warn(format_args!(
            "scootbar: note: the {id} module is unavailable, and left out: {why}"
        ));
    });
    // A font only if something will draw text: with none usable, refuse to
    // start (the error says how to give one).
    let text = if modules.is_empty() {
        None
    } else {
        let font = font::find(config.font.as_deref()).map_err(Error::Font)?;
        Some(Text::new(font.face))
    };
    let content = Content {
        modules,
        text,
        style: config.style(),
    };
    let (mut wayland, missing) = Wayland::connect(config.bar, content).map_err(Error::Wayland)?;
    for interface in missing {
        warn(format_args!(
            "scootbar: note: the compositor has no {interface}; the bar is drawn at the \
             integer scale and scaled down on fractionally scaled outputs"
        ));
    }
    let mut wants_write = false;
    loop {
        wayland
            .queue
            .dispatch_pending(&mut wayland.state)
            .map_err(Error::Dispatch)?;
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
        // The Wayland fd first, then each module's sources: all on the
        // stack, so a turn of the loop allocates nothing.
        let mut fds: [PollFd<'_>; MAX_POLL] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(connection, PollFlags::empty()));
        let mut owners = [(0usize, 0usize); MAX_POLL];
        fds[0] = PollFd::from_borrowed_fd(connection, events);
        let mut len = 1;
        for (index, placed) in wayland.state.content.modules.iter().enumerate() {
            let mut sources = Sources::new(&mut fds, &mut owners, &mut len, index);
            placed.module.sources(&mut sources);
        }
        let polled = fds.get_mut(..len).unwrap_or_default();
        match poll(polled, None) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(errno) => return Err(Error::Poll(errno.into())),
        }
        let mut ready = [PollFlags::empty(); MAX_POLL];
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
        for (flags, &(module, source)) in ready.iter().zip(&owners).take(len).skip(1) {
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
    let bar = &state.bar;
    let content = &mut state.content;
    for entry in state.outputs.iter_mut() {
        let stale = Scene::stale(&content.modules, entry.objects.canvas.shown());
        match entry.output.plan(bar, stale) {
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
                        warn(format_args!(
                            "scootbar: cannot draw the bar on {}: {error}",
                            entry.output.label()
                        ));
                        entry.output.draw_failed(frame);
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
