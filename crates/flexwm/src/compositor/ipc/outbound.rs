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
//!   anything queued stops being read at all until it drains, so a client
//!   cannot queue work faster than it reads the answers.

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
///   writability only, so it is not read again until it has drained (see
///   `Connection::interest`). That alone stops a client queueing more work
///   while it is behind.
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
#[derive(Default)]
pub(super) struct Outbound {
    /// The queued reply (or replies), of which the first `sent` bytes have
    /// already gone out. Empty whenever nothing is waiting.
    buffer: Vec<u8>,
    /// How much of `buffer` the socket has taken. Invariant: never greater
    /// than `buffer.len()`.
    sent: usize,
}

impl Outbound {
    /// How many bytes are still waiting to go out.
    pub(super) fn pending(&self) -> usize {
        // `sent <= buffer.len()` by construction -- both sites that advance
        // it clamp to what was actually written into `buffer`.
        self.buffer.len() - self.sent
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pending() == 0
    }

    /// How much the queue is holding on to, sent bytes included. Only a test
    /// cares: the point it checks is that a drained queue holds on to nothing.
    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Whether enough is queued that the caller should stop reading requests
    /// until it drains. See [`HIGH_WATER_BYTES`].
    pub(super) fn over_high_water(&self) -> bool {
        self.pending() > HIGH_WATER_BYTES
    }

    /// Writes `line` to `socket`, queueing whatever the socket would not take.
    ///
    /// `line` is taken by value so the queue can adopt its allocation instead
    /// of copying it: a screenshot reply is megabytes, and a memcpy of that on
    /// the event-loop thread is exactly the cost this module exists to avoid.
    pub(super) fn send<W: Write>(&mut self, socket: &mut W, line: String) -> io::Result<()> {
        if !self.is_empty() {
            // Something is still queued, so this reply has to go behind it --
            // writing it now would interleave two responses on the wire. Drop
            // the part already sent first, so a connection that queues reply
            // after reply doesn't carry a growing dead prefix around.
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
        self.buffer = Vec::new();
        let written = write_some(socket, line.as_bytes())?;
        if written < line.len() {
            self.sent = written;
            self.buffer = line.into_bytes();
        }
        Ok(())
    }

    /// Pushes out as much of the queue as the socket will take.
    pub(super) fn flush<W: Write>(&mut self, socket: &mut W) -> io::Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let written = write_some(socket, &self.buffer[self.sent..])?;
        self.sent += written;
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
