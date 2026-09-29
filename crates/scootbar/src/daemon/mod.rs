//! `scootbar daemon`: the Wayland client, in one thread's `poll` loop.
//!
//! One `poll` over the Wayland fd, with no timeout: when nothing happens the
//! daemon makes no system call at all. No async runtime, no timers, no frame
//! callbacks (a solid bar is drawn once per change, never animated). Each
//! turn dispatches whatever arrived, then asks every output what to draw
//! ([`draw`]), then flushes and sleeps. Asking after the whole batch means
//! several events that change one output (a scale change arrives as a
//! `wl_output.scale`, a `done`, a `preferred_scale` and a `configure`) are
//! one draw, not four.
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

use crate::bar::Bar;
use crate::outputs::Plan;
use crate::print::warn;
use canvas::Drew;
use wayland::{State, Wayland, WaylandError};

#[derive(Debug)]
pub enum Error {
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
pub fn run(bar: Bar) -> Result<(), Error> {
    let (mut wayland, missing) = Wayland::connect(bar).map_err(Error::Wayland)?;
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
        let fd = guard.connection_fd();
        // One fd, on the stack: the loop allocates nothing.
        let mut fds = [PollFd::new(&fd, events)];
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(errno) => return Err(Error::Poll(errno.into())),
        }
        let revents = fds[0].revents();
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
    }
}

/// Carries out every output's [`Plan`]: a draw, a bare commit, or nothing.
/// Walks the outputs and allocates nothing unless a buffer must be made
/// (the first draw, or a new size).
fn draw(state: &mut State, qh: &wayland_client::QueueHandle<State>) {
    let globals = &state.globals;
    let bar = &state.bar;
    for entry in state.outputs.iter_mut() {
        match entry.output.plan(bar) {
            Plan::Nothing => {}
            Plan::Commit => {
                if let Some(layer) = &entry.objects.layer {
                    layer.surface.commit();
                }
                entry.output.committed();
            }
            Plan::Draw(frame) => {
                let Some(layer) = entry.objects.layer.as_mut() else {
                    continue;
                };
                let id = entry.output.id();
                match entry
                    .objects
                    .canvas
                    .draw(globals, qh, id, layer, frame, bar.background)
                {
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
