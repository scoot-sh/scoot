//! One client connection: non-blocking, line in, line out, bounded.
//!
//! Replies queue in a reused output buffer. While it holds anything, the
//! connection is not read and no further request is handled: a client that
//! sends and never reads stops being served instead of making the daemon
//! buffer for it, and handling stops early once [`OUT_SOFT_LIMIT`] bytes
//! are queued, so one read's worth of tiny requests cannot queue more than
//! about that much either.
//!
//! **A reply can wait.** A handler may answer [`Answer::Later`] (a `set`
//! waits for the compositor). The connection then *awaits*: it is neither
//! read nor asked for readiness, and the requests behind that one stay
//! unanswered, in order, until [`Conn::complete`] delivers the reply. A
//! client that hangs up meanwhile shows as `POLLHUP` (reported even with no
//! events asked for) and is closed; the reply then finds no connection.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;

use rustix::event::PollFlags;

use crate::framing::{Line, LineBuffer};
use crate::protocol::{MAX_REQUEST_LINE, Reply, write_reply};

/// Stop handling requests once this much output is queued; resume when the
/// client has read it.
pub const OUT_SOFT_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Reading requests.
    Open,
    /// The client finished sending: answer what arrived, then close.
    Draining,
    /// Close once the queued output is sent.
    Closing,
}

/// What the connection needs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Keep,
    Close,
}

/// Identifies one connection for the daemon's whole life, never reused, so
/// a reply that arrives late cannot reach a newer client in the same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnId(pub(super) u64);

#[cfg(test)]
impl ConnId {
    pub fn for_test(id: u64) -> Self {
        Self(id)
    }
}

/// Whether a request was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Answer {
    /// The reply is in `out`.
    Now,
    /// Nothing written: the reply comes through [`Conn::complete`].
    Later,
}

/// The daemon's side of a request: gets one line from connection `conn`,
/// and either appends one reply or promises it for later.
pub trait Handler {
    fn handle(&mut self, conn: ConnId, line: &[u8], out: &mut Vec<u8>) -> Answer;
}

#[derive(Debug)]
pub struct Conn {
    id: ConnId,
    stream: UnixStream,
    lines: LineBuffer,
    out: Vec<u8>,
    sent: usize,
    phase: Phase,
    /// A request was answered [`Answer::Later`] and its reply has not
    /// come: nothing more is read or answered.
    awaiting: bool,
}

impl Conn {
    pub fn new(id: ConnId, stream: UnixStream) -> Self {
        Self {
            id,
            stream,
            lines: LineBuffer::new(MAX_REQUEST_LINE),
            out: Vec::new(),
            sent: 0,
            phase: Phase::Open,
            awaiting: false,
        }
    }

    pub fn id(&self) -> ConnId {
        self.id
    }

    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// The poll events to wait for.
    pub fn interest(&self) -> PollFlags {
        if self.sent < self.out.len() {
            PollFlags::OUT
        } else if self.awaiting {
            // Nothing to do until the reply comes; a hang-up is still
            // reported (`POLLHUP` needs no asking).
            PollFlags::empty()
        } else if self.phase == Phase::Open {
            PollFlags::IN
        } else {
            // Nothing to wait for: `service` closes it on its next call.
            PollFlags::empty()
        }
    }

    /// Handles readiness: reads if readable, then answers and sends as far
    /// as the socket allows.
    pub fn service<H: Handler>(
        &mut self,
        revents: PollFlags,
        scratch: &mut [u8],
        handler: &mut H,
    ) -> Status {
        let readable = revents.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR);
        if self.awaiting && self.sent >= self.out.len() {
            // Nothing asked for, so only a hang-up or an error wakes it:
            // the reply could not be delivered. Close, rather than be
            // reported again on every poll.
            return if revents.intersects(PollFlags::HUP | PollFlags::ERR) {
                Status::Close
            } else {
                Status::Keep
            };
        }
        if readable && self.phase == Phase::Open && self.sent >= self.out.len() && !self.awaiting {
            match self.lines.read_from(&mut self.stream, scratch) {
                Ok(0) => self.phase = Phase::Draining,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Status::Close,
            }
        }
        self.pump(handler)
    }

    /// The reply to the request that answered [`Answer::Later`]: `write`
    /// appends it. Then sends it and carries on with the requests behind
    /// it. Does nothing (and keeps the connection) if nothing was awaited.
    pub fn complete<H: Handler>(
        &mut self,
        write: impl FnOnce(&mut Vec<u8>),
        handler: &mut H,
    ) -> Status {
        if !self.awaiting {
            return Status::Keep;
        }
        self.awaiting = false;
        write(&mut self.out);
        self.pump(handler)
    }

    /// Sends queued output; when it is all sent, handles more requests.
    fn pump<H: Handler>(&mut self, handler: &mut H) -> Status {
        loop {
            if let Some(status) = self.flush() {
                return status;
            }
            if self.awaiting {
                return Status::Keep;
            }
            if self.phase == Phase::Closing {
                return Status::Close;
            }
            self.answer(handler);
            if self.awaiting {
                // Send what came before it; the rest waits for the reply.
                continue;
            }
            if self.out.is_empty() {
                return match self.phase {
                    Phase::Open => Status::Keep,
                    Phase::Draining | Phase::Closing => Status::Close,
                };
            }
        }
    }

    /// Writes queued output. `Some` when the caller must stop: the socket
    /// is full (`Keep`, wait for POLLOUT) or broken (`Close`). `None` once
    /// everything is sent, with the buffer emptied for reuse.
    fn flush(&mut self) -> Option<Status> {
        while let Some(pending) = self.out.get(self.sent..).filter(|p| !p.is_empty()) {
            match self.stream.write(pending) {
                Ok(0) => return Some(Status::Close),
                Ok(n) => self.sent += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Some(Status::Keep),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Some(Status::Close),
            }
        }
        self.out.clear();
        self.sent = 0;
        None
    }

    /// Answers complete requests until the output buffer passes the soft
    /// limit or none are left; at the end of the stream, the unterminated
    /// last one too.
    fn answer<H: Handler>(&mut self, handler: &mut H) {
        while self.out.len() < OUT_SOFT_LIMIT {
            match self.lines.next_line() {
                Some(Line::Complete(line)) => {
                    if handler.handle(self.id, line, &mut self.out) == Answer::Later {
                        self.awaiting = true;
                        return;
                    }
                }
                Some(Line::TooLong) => {
                    self.refuse_too_long();
                    return;
                }
                None => {
                    if self.phase == Phase::Draining {
                        match self.lines.take_rest() {
                            Some(Line::Complete(line)) => {
                                if handler.handle(self.id, line, &mut self.out) == Answer::Later {
                                    self.awaiting = true;
                                }
                            }
                            Some(Line::TooLong) => self.refuse_too_long(),
                            None => {}
                        }
                        // Anything awaited is still answered first: `pump`
                        // waits for it before it looks at the phase.
                        self.phase = Phase::Closing;
                    }
                    return;
                }
            }
        }
    }

    /// Framing is lost past an overlong line, so answer and close.
    fn refuse_too_long(&mut self) {
        write_reply(
            &mut self.out,
            &Reply::Error {
                message: &format_args!("request line longer than {MAX_REQUEST_LINE} bytes"),
            },
        );
        self.phase = Phase::Closing;
    }

    /// One non-blocking attempt to send what is queued, for shutdown.
    pub fn flush_once(&mut self) {
        let _ = self.flush();
    }
}
