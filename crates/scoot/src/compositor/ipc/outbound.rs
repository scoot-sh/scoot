//! Replies on their way out of a non-blocking socket.
//!
//! A reply used to go out with `write_all`, which on a blocking socket parks
//! the event-loop thread until the client has read enough to make room --
//! i.e. one client that stops reading stops the whole compositor. Here a
//! reply is written as far as the socket will take it and whatever is left
//! waits in [`Outbound`] until the event loop reports the socket writable
//! again.
//!
//! Two invariants this type exists to keep:
//!
//! - **One reply never overtakes another.** A response that cannot go out in
//!   one write leaves this struct holding its tail, and every later response
//!   queues *behind* that tail rather than being written straight to the
//!   socket. Two responses interleaving on the wire would be undecodable
//!   nonsense at the other end, not merely late.
//! - **The queue cannot grow without bound.** Not by refusing to buffer, and
//!   not by closing the connection -- see [`HIGH_WATER_BYTES`] for why both of
//!   those are the wrong answer here -- but by back-pressure: a connection with
//!   anything queued is not *woken for readability* until it drains, so a
//!   client that stops reading stops being asked what it wants next. (It is not
//!   that reading stops: a wakeup that does arrive, for writability, still
//!   serves whatever is in the read buffer, because nothing else can -- see
//!   `Connection::interest`.)

use std::io::{self, Write};

#[cfg(test)]
mod tests;

/// How many unsent bytes it takes before a connection stops being read even
/// within the wakeup that is already serving it.
///
/// This is the cap on the outbound queue, and it is deliberately *not*
/// `MAX_REQUEST_BYTES`'s number repurposed -- the two bound different things.
/// A request's size is chosen by the client and nothing legitimate needs a
/// megabyte, so that one is a hard refusal. A response's size is chosen by the
/// *compositor*: a `Response::Screenshot` is a whole base64'd PNG, several
/// megabytes for a large output, and a client reading it over a loaded machine
/// is being slow, not hostile. Refusing or truncating that reply would break
/// real screenshot delivery, and truncating it would corrupt the connection
/// besides. So nothing here ever refuses to queue a reply, and nothing closes a
/// connection for being slow.
///
/// What bounds the queue is two rules together, which is worth being precise
/// about because neither is sufficient alone:
///
/// - **Between wakeups**, a connection with anything queued is registered for
///   writability only, so nothing wakes it to ask for more work until it has
///   drained (see `Connection::interest`). That alone stops a client that has
///   stopped reading from queueing more.
/// - **Within one wakeup**, requests the connection has *already* read into its
///   own buffer must still be answered -- a level-triggered readiness source
///   will not report them again -- so replies can keep queueing after one has
///   stopped fitting. This mark is what bounds that.
///
/// Together: a connection's queue never exceeds this many bytes plus the one
/// response that crossed the mark. That response is one the compositor had
/// already built in memory to write, so buffering it costs no more than the
/// `write_all` it replaces, which held the same bytes for as long as it blocked.
///
/// A mebibyte is far more small replies than any real client has outstanding
/// (`Response::Ok` is 15 bytes, a `version` reply ~50), so this is not a figure
/// a legitimate agent's batch runs into; it is the ceiling on what one
/// connection that never reads can make the compositor hold.
pub(super) const HIGH_WATER_BYTES: usize = 1 << 20;

/// Bytes of replies written but not yet accepted by the socket.
///
/// `pub(crate)` rather than `pub(super)`: a screenshot reply finished on the
/// encode worker goes out through a clone of its connection's socket, not
/// through the connection itself, so `screenshot.rs` writes it with this
/// same queue rather than a second one (see `PendingShot`).
#[derive(Default)]
pub(crate) struct Outbound {
    /// The queued reply (or replies), of which the first `sent` bytes have
    /// already gone out. Empty whenever nothing is waiting.
    buffer: Vec<u8>,
    /// How much of `buffer` the socket has taken. Invariant: never greater
    /// than `buffer.len()`.
    sent: usize,
    /// How many bytes have gone out through this queue since the connection
    /// opened, across every reply. Only ever increases.
    ///
    /// Deliberately not derivable from [`Outbound::pending`], which is what
    /// the write-stall deadline in `connection.rs` tried first: a queue that
    /// is the same size a window later has not necessarily stalled -- it may
    /// have drained and been refilled, which is a client reading steadily
    /// while the compositor answers it. Only a count of what actually left
    /// can tell those apart.
    ///
    /// A `u64` because a `usize` would be one on 32-bit: a connection that
    /// moved four gigabytes of screenshots would wrap it, and a wrapped
    /// counter that happens to land on its previous value reads as "no
    /// progress" -- i.e. drops a connection that is working perfectly. At
    /// this width it cannot happen at any transfer rate a socket has.
    total_sent: u64,
}

impl Outbound {
    /// How many bytes are still waiting to go out.
    pub(crate) fn pending(&self) -> usize {
        // Saturating, not a plain subtraction, even though `sent` is never
        // greater than `buffer.len()` at any of the three sites that touch
        // either. That is an invariant spread across methods, and the cost of
        // it being broken by some future edit is not a wrong number: it is a
        // wrapped `usize` here, a `self.buffer[self.sent..]` slice panic in
        // `flush`, and -- because `panic = "abort"` -- every connected client's
        // unsaved work. A `debug_assert` keeps the tests honest about it
        // without making release builds crash over it.
        debug_assert!(self.sent <= self.buffer.len());
        self.buffer.len().saturating_sub(self.sent)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending() == 0
    }

    /// How many bytes this queue has ever got out. See [`Outbound::total_sent`]
    /// for why the write-stall deadline watches this rather than `pending`.
    pub(crate) fn total_sent(&self) -> u64 {
        self.total_sent
    }

    /// How much the queue is holding on to, sent bytes included. Only a test
    /// cares: the point it checks is that a drained queue holds on to nothing.
    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Whether enough is queued that the caller should stop reading requests
    /// until it drains. See [`HIGH_WATER_BYTES`].
    pub(crate) fn over_high_water(&self) -> bool {
        self.pending() > HIGH_WATER_BYTES
    }

    /// Writes `line` to `socket`, queueing whatever the socket would not take.
    ///
    /// `line` is taken by value so the queue can adopt its allocation instead
    /// of copying it: a screenshot reply is megabytes, and a memcpy of that on
    /// the event-loop thread is exactly the cost this module exists to avoid.
    pub(crate) fn send<W: Write>(&mut self, socket: &mut W, line: String) -> io::Result<()> {
        if !self.is_empty() {
            // Something is still queued, so this reply has to go behind it --
            // writing it now would interleave two responses on the wire. Drop
            // the part already sent first, so a connection that queues reply
            // after reply doesn't carry a growing dead prefix around. The
            // memmove that costs is proportional to what is still *owed*
            // (bounded by `HIGH_WATER_BYTES`), never to what has already gone
            // out, and it only happens at all once the kernel has taken part
            // of the queue -- never on the common path, where the queue is
            // empty and this branch is not taken.
            self.buffer.drain(..self.sent);
            self.sent = 0;
            self.buffer.extend_from_slice(line.as_bytes());
            return self.flush(socket);
        }
        // Nothing queued: write straight from the encoded line, and free the
        // previous reply's buffer rather than keeping its capacity. A drained
        // screenshot would otherwise leave megabytes held per connection for
        // the rest of its life, and the only thing reuse would save is the
        // one allocation a *newly blocked* reply needs -- which the move
        // below hands over for free anyway.
        //
        // `sent` goes with it. Emptying the buffer without it would be correct
        // only as long as nothing can reach here with `sent > 0` -- true today
        // (`is_empty()` means `sent == buffer.len()`, and `flush` clears both
        // together), but an invariant held in another method is not one to lean
        // on for memory safety: it would make `pending()` underflow.
        self.buffer = Vec::new();
        self.sent = 0;
        let written = write_some(socket, line.as_bytes())?;
        self.total_sent += written as u64;
        if written < line.len() {
            self.sent = written;
            self.buffer = line.into_bytes();
        }
        Ok(())
    }

    /// Pushes out as much of the queue as the socket will take.
    pub(crate) fn flush<W: Write>(&mut self, socket: &mut W) -> io::Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let written = write_some(socket, &self.buffer[self.sent..])?;
        self.sent += written;
        self.total_sent += written as u64;
        if self.sent == self.buffer.len() {
            // Fully out. Freed rather than cleared, for the reason in `send`.
            self.buffer = Vec::new();
            self.sent = 0;
        }
        Ok(())
    }
}

/// Writes as much of `bytes` as `socket` accepts without blocking, returning
/// how much that was.
///
/// A short write is the normal case here, not an error: the socket's buffer is
/// full and the rest has to wait for the client to read. `WouldBlock` is the
/// same answer with nothing accepted at all.
fn write_some<W: Write>(socket: &mut W, bytes: &[u8]) -> io::Result<usize> {
    let mut written = 0;
    while written < bytes.len() {
        match socket.write(&bytes[written..]) {
            // A write that accepts nothing while there is something to accept
            // is not something a socket does; treated as an error rather than
            // retried, so a reader that somehow does it cannot spin this loop
            // forever on the event-loop thread.
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error),
        }
    }
    Ok(written)
}
