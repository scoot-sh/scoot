//! How many connections the control socket serves at once.
//!
//! Everything else in this module is written so that one client cannot cost
//! the others anything: a request line is capped, an outbound queue is capped,
//! a wakeup's reads are capped, and a screenshot is capped to one per
//! connection per frame. All of those are *per connection*, which leaves the
//! obvious way around every one of them -- open another connection. A client
//! reconnecting for each capture pays the same accept it always did and gets a
//! fresh screenshot allowance every time.
//!
//! So the number of connections is bounded too, by the same shape the rest of
//! the compositor's bounds use (`wl_shm` pool sizes in `dispatch.rs`,
//! `MAX_TOKENS` in `activation.rs`, `CAPACITY` in `input/interaction.rs`): a
//! fixed ceiling, refused rather than queued once full, logged at `debug`.
//! Queueing would only move the unbounded thing from a table of connections
//! into a table of waiters, and a refusal costs the client a millisecond and a
//! retry.
//!
//! A slot is claimed in [`super::accept`] and released by dropping the [`Slot`]
//! -- which happens wherever the connection itself goes away, including the
//! paths that never run a line of this module's code (an event-loop source
//! removed on close, an evicted stalled connection, the whole loop being torn
//! down). The one place a slot outlives its connection source is a `wait-idle`
//! hand-off, which moves it into the waiter rather than releasing it; see
//! `Connection::serve`.

use std::cell::Cell;
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// How many IPC connections may be live at once, across every client.
///
/// A connection is two fds and a few kilobytes of buffer, so like
/// `activation::MAX_TOKENS` this is not a memory bound worth tuning -- it is a
/// bound at all, which is the point. It is far above any real workload: a
/// session runs a bar, a notifier and an agent or two, each holding one
/// connection open, and a one-shot `flexwm msg` holds one for the millisecond
/// it takes to ask and be answered (requests pipeline, so even a busy agent
/// needs exactly one). A session that ever holds 64 at once has a client in a
/// loop, not a busy desktop.
pub(super) const MAX_CONNECTIONS: usize = 64;

/// The count of live connections, shared between the accept loop and every
/// connection that has claimed a slot.
///
/// `Rc<Cell<_>>` rather than a field on `State`: nothing outside this module
/// has any business with the number, and a [`Slot`] has to be able to release
/// itself from a `Drop` that cannot reach `State`. Single-threaded by
/// construction -- the compositor has one thread, and `State` is already
/// `!Send` for the same reason.
pub(super) struct Slots {
    live: Rc<Cell<usize>>,
}

impl Slots {
    pub(super) fn new() -> Self {
        Self {
            live: Rc::new(Cell::new(0)),
        }
    }

    /// Takes one of the [`MAX_CONNECTIONS`] slots, or `None` when every one of
    /// them is already taken.
    pub(super) fn claim(&self) -> Option<Slot> {
        let live = self.live.get();
        if live >= MAX_CONNECTIONS {
            return None;
        }
        self.live.set(live + 1);
        Some(Slot {
            live: Rc::clone(&self.live),
        })
    }

    /// How many slots are taken. Only a test asks: the accept path cares
    /// whether it can claim one, not how many are out.
    #[cfg(test)]
    pub(super) fn live(&self) -> usize {
        self.live.get()
    }
}

/// One live connection's claim on a slot, released when it is dropped.
///
/// Held, never read. Its whole job is to outlive exactly what the slot is
/// counting -- so it lives inside the connection (and moves into a `wait-idle`
/// waiter when the connection hands over to one), and no code path has to
/// remember to give it back.
pub(super) struct Slot {
    live: Rc<Cell<usize>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        // Saturating, though a `Slot` is created only by `claim` (one
        // increment each) and cannot be cloned, so the count cannot reach
        // zero with one still alive. The cost of that invariant being broken
        // by a future edit is not an off-by-one: a wrapped `usize` here is a
        // count permanently past `MAX_CONNECTIONS`, i.e. a control socket
        // that refuses every connection for the rest of the session. A
        // `debug_assert` keeps the tests honest about it instead.
        let live = self.live.get();
        debug_assert!(live > 0, "a connection slot was released twice");
        self.live.set(live.saturating_sub(1));
    }
}
