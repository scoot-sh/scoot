//! One client connection: non-blocking, line in, line out, bounded.
//!
//! Replies queue in a reused output buffer. While it holds anything, the
//! connection is not read and no further request is handled: a client that
//! sends and never reads stops being served instead of making the daemon
//! buffer for it, and handling stops early once [`OUT_SOFT_LIMIT`] bytes
//! are queued, so one read's worth of tiny requests cannot queue more than
//! about that much either. What is still queued past [`STALL_DEADLINE`]
//! with nothing delivered drops the connection (the server sweeps once a
//! turn), so a peer that stopped reading holds its slot no longer than
//! that; a peer making even slow progress starts its deadline over.
//!
//! Every request is answered at once (no request waits on the compositor),
//! so unlike scootbg's connection there is nothing deferred: [`Handler`]
//! appends each reply as it handles the line.
//!
//! ## A subscribed connection
//!
//! `subscribe` turns a connection into an event stream ([`Kinds`]): it then
//! carries events only, and any request on it is refused with an error naming
//! the rule (open another connection for requests). **Nothing is buffered
//! for it**: an event is one nonblocking write, and a subscriber whose socket
//! cannot take the whole of it is disconnected on the spot, so a client that
//! stops reading costs the daemon nothing and cannot make it wait or grow.
//!
//! (The shape follows scootbg's `control/conn.rs`, minus its deferred
//! replies.)

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use rustix::event::PollFlags;

use super::framing::{Line, LineBuffer};
use super::protocol::{DROPPED, EventKind, MAX_REQUEST_LINE, Reply, write_reply};

/// Stop handling requests once this much output is queued; resume when the
/// client has read it.
pub const OUT_SOFT_LIMIT: usize = 4096;

/// How long a peer that stopped reading is held: past it the connection is
/// dropped ([`Conn::stalled`], swept by the server once a turn). Generous
/// next to the soft limit above (a slow reader on a loaded machine drains a
/// few kilobytes in much less), short next to forever: without it a client
/// that never reads holds its slot and its queued replies for the daemon's
/// whole life.
pub const STALL_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Reading requests.
    Open,
    /// The client finished sending: answer what arrived, then close.
    Draining,
    /// Close once the queued output is sent.
    Closing,
    /// Dedicated to events of these kinds: no more requests are served.
    Subscribed(Kinds),
}

/// The event kinds a subscribed connection carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Kinds(u8);

impl Kinds {
    pub fn of(kinds: &[EventKind]) -> Self {
        Self(
            kinds
                .iter()
                .fold(0, |bits, kind| bits | (1 << (*kind as u8))),
        )
    }

    pub fn wants(self, kind: EventKind) -> bool {
        self.0 & (1 << (kind as u8)) != 0
    }
}

/// What the connection needs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Keep,
    Close,
}

/// The daemon's side of a request: gets one line, and appends exactly
/// one reply to `out`.
pub trait Handler {
    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>);

    /// Whether the request just handled subscribed this connection, and to
    /// what: asked after every line. The reply to `subscribe` is already in
    /// `out`; afterwards the connection carries events only.
    fn take_subscription(&mut self) -> Option<Kinds> {
        None
    }
}

#[derive(Debug)]
pub struct Conn {
    stream: UnixStream,
    lines: LineBuffer,
    out: Vec<u8>,
    sent: usize,
    phase: Phase,
    /// A subscribed client sent a line past the limit: framing is lost.
    too_long: bool,
    /// When the queued output first stopped moving (or last moved): a peer
    /// that delivers nothing past [`STALL_DEADLINE`] is dropped. `None`
    /// while nothing is queued behind it.
    stalled_since: Option<Instant>,
}

impl Conn {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            lines: LineBuffer::new(MAX_REQUEST_LINE),
            out: Vec::new(),
            sent: 0,
            phase: Phase::Open,
            too_long: false,
            stalled_since: None,
        }
    }

    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// The kinds this connection carries, if it is subscribed.
    pub fn subscription(&self) -> Option<Kinds> {
        match self.phase {
            Phase::Subscribed(kinds) => Some(kinds),
            _ => None,
        }
    }

    /// The poll events to wait for.
    pub fn interest(&self) -> PollFlags {
        if self.sent < self.out.len() {
            PollFlags::OUT
        } else if matches!(self.phase, Phase::Open | Phase::Subscribed(_)) {
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
        if matches!(self.phase, Phase::Subscribed(_)) {
            return self.serve_subscribed(readable, scratch);
        }
        if readable && self.phase == Phase::Open && self.sent >= self.out.len() {
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

    /// A subscribed connection: anything the client sends is refused (one
    /// error line, never queued: a reply that cannot be sent at once ends the
    /// connection), and its end of the stream ends it.
    fn serve_subscribed(&mut self, readable: bool, scratch: &mut [u8]) -> Status {
        if !readable {
            return Status::Keep;
        }
        match self.lines.read_from(&mut self.stream, scratch) {
            Ok(0) => return Status::Close,
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Status::Keep;
            }
            Err(_) => return Status::Close,
        }
        let mut refusal = Vec::new();
        let asked = self.refuse_requests(&mut refusal);
        if self.too_long {
            return Status::Close;
        }
        if !asked {
            return Status::Keep;
        }
        self.send_event(&refusal)
    }

    /// Drains every request that arrived on a subscribed connection and, if
    /// there were any, appends the one refusal that answers them all (a
    /// client that keeps asking is not owed a line each). Whether it did.
    fn refuse_requests(&mut self, out: &mut Vec<u8>) -> bool {
        let mut asked = false;
        loop {
            match self.lines.next_line() {
                Some(Line::Complete(_)) => asked = true,
                Some(Line::TooLong) => {
                    self.too_long = true;
                    break;
                }
                None => break,
            }
        }
        if asked {
            write_reply(
                out,
                &Reply::Error {
                    message: &"a subscribed connection serves no further requests: it \
                               carries events only (open another connection for requests)",
                },
            );
        }
        asked
    }

    /// Sends an event: one nonblocking write of all of `bytes`, or the
    /// connection is closed (`Status::Close`). Nothing is queued.
    pub fn send_event(&mut self, bytes: &[u8]) -> Status {
        match self.stream.write(bytes) {
            Ok(n) if n == bytes.len() => Status::Keep,
            // Partial, full or broken: the subscriber is not keeping up.
            _ => Status::Close,
        }
    }

    /// Whether the peer stopped reading past [`STALL_DEADLINE`]: output
    /// has been queued since `stalled_since` with nothing delivered.
    /// Subscribed connections never queue, so this never holds for them
    /// (they are dropped on the spot instead: [`Conn::send_event`]).
    pub fn stalled(&self, now: Instant) -> bool {
        self.stalled_since
            .is_some_and(|at| now.saturating_duration_since(at) >= STALL_DEADLINE)
    }

    /// How long until [`Conn::stalled`] holds for this connection, if its
    /// peer stays quiet: `None` while nothing is queued behind it.
    pub fn stall_in(&self, now: Instant) -> Option<Duration> {
        self.stalled_since.map(|at| {
            STALL_DEADLINE
                .checked_sub(now.saturating_duration_since(at))
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Tells a subscriber it is being dropped, if it can be told without
    /// waiting: one nonblocking write of [`DROPPED`], and whatever
    /// the socket does with it is the end of it. A subscriber is always at a
    /// line boundary here (an event is written whole or the connection is
    /// closed), so the line lands whole or not at all; a partial write of
    /// its 19 bytes (a socket with less room than that) leaves a prefix with
    /// no newline, which the client reads as a cut line.
    pub fn notify_dropped(&mut self) {
        if self.subscription().is_some() {
            let _ = self.stream.write(DROPPED);
        }
    }

    /// Sends queued output; when it is all sent, handles more requests.
    fn pump<H: Handler>(&mut self, handler: &mut H) -> Status {
        loop {
            if let Some(status) = self.flush() {
                return status;
            }
            if self.phase == Phase::Closing {
                return Status::Close;
            }
            self.answer(handler);
            if let Phase::Subscribed(_) = self.phase {
                // The `subscribed` reply goes out now, in one write, or the
                // subscriber is not worth keeping.
                // Requests pipelined behind it are refused in the same write.
                let mut reply = std::mem::take(&mut self.out);
                self.sent = 0;
                self.refuse_requests(&mut reply);
                if self.too_long {
                    return Status::Close;
                }
                return self.send_event(&reply);
            }
            if self.out.is_empty() {
                return match self.phase {
                    Phase::Open => Status::Keep,
                    Phase::Draining | Phase::Closing => Status::Close,
                    Phase::Subscribed(_) => Status::Keep,
                };
            }
        }
    }

    /// Writes queued output. `Some` when the caller must stop: the socket
    /// is full (`Keep`, wait for POLLOUT) or broken (`Close`). `None` once
    /// everything is sent, with the buffer emptied for reuse. Stamps when
    /// the output stops moving, refreshes the stamp on any delivery, and
    /// clears it once drained, for [`Conn::stalled`].
    fn flush(&mut self) -> Option<Status> {
        // Whether this call delivered anything: a peer making even slow
        // progress has not stopped, and its deadline starts over.
        let mut moved = false;
        while let Some(pending) = self.out.get(self.sent..).filter(|p| !p.is_empty()) {
            match self.stream.write(pending) {
                Ok(0) => return Some(Status::Close),
                Ok(n) => {
                    self.sent += n;
                    moved = true;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if moved {
                        self.stalled_since = Some(Instant::now());
                    } else {
                        self.stalled_since.get_or_insert_with(Instant::now);
                    }
                    return Some(Status::Keep);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Some(Status::Close),
            }
        }
        self.out.clear();
        self.sent = 0;
        self.stalled_since = None;
        None
    }

    /// Answers complete requests until the output buffer passes the soft
    /// limit or none are left; at the end of the stream, the unterminated
    /// last one too.
    fn answer<H: Handler>(&mut self, handler: &mut H) {
        while self.out.len() < OUT_SOFT_LIMIT {
            match self.lines.next_line() {
                Some(Line::Complete(line)) => {
                    handler.handle(line, &mut self.out);
                    if let Some(kinds) = handler.take_subscription() {
                        // The reply is queued; whatever else arrived behind
                        // the request is refused as any request now is.
                        self.phase = Phase::Subscribed(kinds);
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
                                handler.handle(line, &mut self.out);
                            }
                            Some(Line::TooLong) => self.refuse_too_long(),
                            None => {}
                        }
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
