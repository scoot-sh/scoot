//! Windows mode: the open windows, listed and activated through
//! `wlr-foreign-toplevel-management-unstable-v1`.
//!
//! `--windows` opens the panel over the compositor's toplevel list, one
//! row per window, and Enter (or a click) asks the compositor to
//! activate the chosen one. The protocol does both halves: each
//! `zwlr_foreign_toplevel_handle_v1` carries `title`, `app_id` and
//! `state`, and its `activate(seat)` hands the keyboard on as a click on
//! a taskbar would. `ext-foreign-toplevel-list-v1` is not used: its
//! handles share nothing with wlr's, so a window listed from it could
//! not be activated from it. Both scoot and sway serve the wlr protocol.
//!
//! The list is a snapshot taken at open, on a connection and event
//! queue of its own: the manager is bound only here, so apps, run and
//! dmenu never receive a window event. The panel's connection is the
//! same socket (a second queue on it): events for these handles that
//! arrive while the panel is open wait in this queue, in memory, and
//! are read when the pick is activated. A title that changes while the
//! panel is open is not redrawn (a snapshot is the list the user
//! opened), but a window that closes is known to be gone before
//! `activate` is sent, so the pick is refused instead of sent to a
//! window that no longer exists.
//!
//! The pick is activated after the panel is gone: the layer surface
//! holds an exclusive keyboard, and a compositor may refuse to move
//! focus while it does. Nothing but `activate` is ever sent on a
//! handle (no close, no minimize, no fullscreen).

mod dispatch;
mod entries;

#[cfg(test)]
mod tests;

use std::fmt;
use std::time::{Duration, Instant};

use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;

pub use entries::{to_entries, window_number};

/// At most this many windows are kept: a hostile or runaway compositor
/// cannot grow the list without bound. Past it the extra handles are
/// destroyed unseen.
pub const MAX_WINDOWS: usize = 1024;

/// The highest protocol version bound (`parent` and the later events are
/// never read, so the version only has to carry what is).
const MAX_VERSION: u32 = 3;

/// How long one round trip to the compositor may take before windows
/// mode gives up: a stalled compositor is an error, never a hang.
const PATIENCE: Duration = Duration::from_secs(2);

/// One window as the compositor announced it.
pub struct Item {
    handle: ZwlrForeignToplevelHandleV1,
    pub title: String,
    pub app_id: String,
    /// The compositor sent `closed`: the handle is dead and nothing may
    /// be sent on it.
    closed: bool,
}

/// The event state the dispatch fills.
#[derive(Default)]
pub(crate) struct State {
    items: Vec<Item>,
    /// The manager sent `finished`: no request may follow on its
    /// handles.
    finished: bool,
    /// The globals the registry announced that this mode binds, as
    /// `(name, version)`.
    manager_global: Option<(u32, u32)>,
    seat_global: Option<(u32, u32)>,
    /// The last `wl_display.sync` came back.
    synced: bool,
}

/// Why the windows could not be listed or activated.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The compositor does not serve `zwlr_foreign_toplevel_manager_v1`.
    NoManager,
    /// No `wl_seat`, which `activate` needs.
    NoSeat,
    /// The connection failed.
    Connection(String),
    /// The compositor did not answer within [`PATIENCE`].
    Stalled,
    /// The chosen window closed while the panel was open.
    Closed,
    /// The compositor withdrew the manager: it can no longer be asked
    /// to activate.
    Finished,
    /// The pick is not a window the list held.
    Unknown,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoManager => write!(
                f,
                "the compositor does not offer zwlr_foreign_toplevel_manager_v1, which windows mode needs"
            ),
            Error::NoSeat => write!(
                f,
                "the compositor offers no wl_seat to activate a window on"
            ),
            Error::Connection(why) => write!(f, "the Wayland connection failed: {why}"),
            Error::Stalled => write!(f, "the compositor did not answer in time"),
            Error::Closed => write!(f, "that window closed before it could be activated"),
            Error::Finished => write!(
                f,
                "the compositor stopped serving the window list before the window could be activated"
            ),
            Error::Unknown => write!(f, "that window is not in the list"),
        }
    }
}

/// The bound manager, its windows and the seat to activate on.
pub struct Windows {
    link: Link,
    seat: WlSeat,
    /// Kept so the manager is not dropped silently; nothing is sent on
    /// it (`stop` would only add events).
    _manager: ZwlrForeignToplevelManagerV1,
}

/// The queue and state this mode's events fill, with the bounded round
/// trip that drives them.
struct Link {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
}

impl Link {
    /// A `wl_display.sync` and the events up to its answer, for at most
    /// [`PATIENCE`]. Never blocks past it, so a stalled compositor cannot
    /// hang the launcher (before the panel or after it is gone).
    fn roundtrip(&mut self) -> Result<(), Error> {
        let fail = |error: &dyn fmt::Display| Error::Connection(error.to_string());
        self.state.synced = false;
        self.conn.display().sync(&self.qh, ());
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.queue
                .dispatch_pending(&mut self.state)
                .map_err(|error| fail(&error))?;
            if self.state.synced {
                return Ok(());
            }
            self.queue.flush().map_err(|error| fail(&error))?;
            // `None`: events arrived meanwhile; dispatch them first.
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Stalled);
            }
            let timeout = rustix::time::Timespec {
                tv_sec: remaining.as_secs() as _,
                tv_nsec: remaining.subsec_nanos() as _,
            };
            let mut polls = [rustix::event::PollFd::from_borrowed_fd(
                guard.connection_fd(),
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut polls, Some(&timeout)) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(fail(&error)),
            }
            if polls[0].revents().is_empty() {
                // Nothing yet: the read guard drops unread and the loop
                // checks the deadline.
                continue;
            }
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(fail(&error)),
            }
        }
    }
}

impl Windows {
    /// Binds the manager and the seat on `conn` (a queue of their own)
    /// and reads the windows that exist now: the announcement comes in
    /// one burst on bind, ended by the sync, so every window is in the
    /// list before the first draw.
    pub fn connect(conn: &Connection) -> Result<Self, Error> {
        let queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut link = Link {
            conn: conn.clone(),
            queue,
            qh: qh.clone(),
            state: State::default(),
        };
        link.roundtrip()?;
        let (manager_name, manager_version) = link.state.manager_global.ok_or(Error::NoManager)?;
        let (seat_name, seat_version) = link.state.seat_global.ok_or(Error::NoSeat)?;
        let manager: ZwlrForeignToplevelManagerV1 =
            registry.bind(manager_name, manager_version.min(MAX_VERSION), &qh, ());
        let seat: WlSeat = registry.bind(seat_name, seat_version.min(1), &qh, ());
        link.roundtrip()?;
        Ok(Self {
            link,
            seat,
            _manager: manager,
        })
    }

    /// The windows that are open, with the index `activate` takes.
    pub fn open_windows(&self) -> impl Iterator<Item = (usize, &Item)> {
        self.link
            .state
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.closed)
    }

    /// Activates window `index` (as `open_windows` numbered it). Pending
    /// events are read first, so a window that closed meanwhile is
    /// reported as [`Error::Closed`] and never sent a request.
    pub fn activate(&mut self, index: usize) -> Result<(), Error> {
        // A round trip reads whatever the compositor sent while the panel
        // was open, so a close that happened then is known now.
        self.link.roundtrip()?;
        let item = self.link.state.items.get(index).ok_or(Error::Unknown)?;
        if item.closed {
            return Err(Error::Closed);
        }
        if self.link.state.finished {
            return Err(Error::Finished);
        }
        item.handle.activate(&self.seat);
        // The request is on the wire once the sync returns, so the
        // process may exit without the compositor missing it.
        self.link.roundtrip()
    }
}
