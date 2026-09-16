//! One client's connection, from the event loop's point of view.
//!
//! The whole of this module exists to answer requests without ever parking the
//! compositor's single thread: see `ipc.rs`'s module doc for what used to
//! happen instead, and [`super::line`]/[`super::outbound`] for the read and
//! write halves this drives.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use flexwm_ipc::{Request, Response, decode, encode};
use smithay::reexports::calloop;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{
    EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory,
};

use super::line::{LineRead, Lines, MAX_REQUEST_BYTES};
use super::outbound::Outbound;
use super::slots::Slot;
use super::{PendingIdle, screenshot_throttled};
use crate::compositor::State;
use crate::compositor::headless::FRAME_INTERVAL;

#[cfg(test)]
mod tests;

/// How many times one wakeup may read from one connection's socket.
///
/// This is the per-wakeup fairness bound, and it has to be counted in *reads*
/// rather than in requests or bytes served — see [`Lines::next`] for why
/// neither of those bounds anything, and why stopping with complete lines still
/// in the read buffer is not an option either (it strands them: a
/// level-triggered source does not report what has already left the kernel).
/// Stopping when the budget runs out is safe for the opposite reason: the
/// budget can only run out with the read buffer empty, so whatever is left is
/// still in the kernel, and the kernel reports it again.
///
/// One, so a connection serves roughly one [`line::DEFAULT_CAPACITY`] chunk
/// and then lets input, the frame timer, wayland dispatch and every other
/// connection have their turn. A flooding client therefore costs a few hundred
/// microseconds of other sources' latency per wakeup instead of tens of
/// milliseconds, at the price of one extra `epoll_wait` round trip per 8 KiB of
/// pipelined requests, which is not a cost anything can measure.
const READS_PER_WAKEUP: u32 = 1;

/// How long a connection may hold a reply its peer has not taken a byte of
/// before it is dropped.
///
/// This is what stops a half-closed client from pinning a connection slot and
/// two fds for the rest of the session. A client that does `shutdown(SHUT_WR)`
/// and then never reads raises `EPOLLIN`/`EPOLLRDHUP`, not `EPOLLHUP`, and a
/// connection with something queued is registered for *writability* only (see
/// [`Connection::interest`]) -- so nothing ever wakes it again to notice. The
/// fix that suggests itself, registering for reads as well, is the one
/// `docs/roadmap/10-ipc-connection-loop.md` deliberately rejected: readiness
/// here is level-triggered, an end of stream is reported on every turn, and
/// the queue cannot drain into a socket nobody is reading, so the loop would
/// spin at full speed on an event it can never act on. A deadline can act on
/// it.
///
/// Measured in *progress*, not in total time, exactly as `PendingIdle::push`
/// measures a `wait-idle` answer that will not go out: a client slowly
/// draining a multi-megabyte screenshot is making progress and is never given
/// up on, however long it takes. What is given up on is a peer that has not
/// taken a single byte in this long -- and the queue is only non-empty at all
/// once that peer's receive buffer is full, which on Linux is a couple of
/// hundred kilobytes it is already behind by. Ten seconds of that is not a
/// slow reader; it is a dead one.
///
/// The check runs on a deadline of its own rather than continuously, so the
/// real window is between one and two of these: a connection is dropped no
/// sooner than [`WRITE_STALL_TIMEOUT`] after its last byte went out, and no
/// later than twice that.
pub(super) const WRITE_STALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Wraps an accepted socket up as an event source ready to be inserted.
///
/// The socket must already be non-blocking; [`super::accept`] does that, and
/// says why there rather than here. `slot` is this connection's claim on one of
/// `slots::MAX_CONNECTIONS`, released when the source is dropped; `stall` is
/// how long it may hold an unread reply (see [`WRITE_STALL_TIMEOUT`], which is
/// what production passes).
pub(super) fn source(
    stream: UnixStream,
    slot: Slot,
    stall: Duration,
) -> std::io::Result<ConnectionSource> {
    Ok(ConnectionSource {
        // A second fd on the same socket: this one is polled, the one inside
        // `Lines` is read and written. `Generic` has to own what it polls, and
        // `BufReader` has to own what it reads, so two is the floor without
        // writing a buffered reader by hand.
        connection: Connection {
            lines: Lines::new(stream.try_clone()?),
            outbound: Outbound::default(),
            last_screenshot: None,
            closing: false,
            slot: Some(slot),
        },
        socket: Generic::new(stream, Interest::READ, Mode::Level),
        // The deadline this starts with is never the one that fires: arming
        // sets a fresh one (see `process_events`), and until then the timer is
        // not registered at all.
        stall: Timer::from_duration(stall),
        window: stall,
        armed: false,
        stall_progress: 0,
    })
}

/// One IPC connection, and the event-loop registration that drives it.
///
/// A plain `Generic` would do for reading, but not for writing: whether this
/// connection wants to hear about write-readiness changes as replies queue and
/// drain, and the callback a `Generic` hands out has no way to reach the
/// `Generic`'s own `interest` field to say so. A source that owns both can --
/// see [`ConnectionSource::process_events`].
///
/// It owns a second sub-source for the same reason the first one is here: the
/// deadline that evicts a peer which has stopped reading (see
/// [`WRITE_STALL_TIMEOUT`]) has to be able to say "this connection is
/// finished", and only the source itself can answer `PostAction::Remove`.
/// A `Timer` costs no fd and no syscall -- calloop tracks deadlines in the
/// loop's own heap -- so an armed one costs a heap entry and a wakeup per
/// window, and a connection whose peer is keeping up is never armed at all.
pub(super) struct ConnectionSource {
    socket: Generic<UnixStream>,
    /// Comes due when the peer may have stopped reading. Registered -- and so
    /// able to fire -- only while [`ConnectionSource::armed`].
    stall: Timer,
    /// How long that deadline is, held because a `Timer`'s own is set at
    /// arming time and there is no reading it back out.
    window: Duration,
    /// Whether [`ConnectionSource::stall`] currently has a live deadline in the
    /// event loop.
    ///
    /// True exactly while this connection is registered for writability, which
    /// is exactly while it has a queue its peer has not taken (see
    /// [`Connection::interest`]): a connection with nothing to write cannot be
    /// stalled, and must not be woken to be told so.
    armed: bool,
    /// How many bytes this connection had got out when the deadline last
    /// looked. Compared against the same count a window later, which is what
    /// makes this a bound on *no progress* rather than on how long a slow
    /// reader may take.
    ///
    /// Counted in bytes that left rather than in bytes still queued, which is
    /// not the same test: a queue the same size a window later may have
    /// drained and refilled in between -- a client reading steadily while the
    /// compositor answers it -- and dropping that connection would be exactly
    /// wrong. See [`Outbound::total_sent`].
    stall_progress: u64,
    connection: Connection,
}

impl EventSource for ConnectionSource {
    type Event = Readiness;
    type Metadata = Connection;
    type Ret = Step;
    type Error = std::io::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> std::io::Result<PostAction>
    where
        F: FnMut(Readiness, &mut Connection) -> Step,
    {
        let Self {
            socket,
            stall,
            window,
            armed,
            stall_progress,
            connection,
        } = self;
        // The deadline first: if this wakeup is it coming due, whether there is
        // anything left to serve depends on what it finds. Each sub-source
        // compares the token against its own registration and does nothing if
        // it is not theirs, so exactly one of the two runs per wakeup.
        let mut evicted = false;
        stall.process_events(readiness, token, |_deadline, ()| {
            let progress = connection.total_sent();
            // Two conditions, and the queue check is not redundant: a
            // connection with nothing to write has made no progress either,
            // and is not stalled -- it is idle. (It also cannot be armed, so
            // this is belt and braces; written this way round so that a state
            // that cannot happen could only ever cost a spurious re-arm,
            // never a live connection.)
            if !connection.is_drained() && progress == *stall_progress {
                evicted = true;
                // Nothing to reschedule for: this source is leaving the loop.
                return TimeoutAction::Drop;
            }
            *stall_progress = progress;
            // Rescheduled from here rather than by re-registering: calloop
            // puts the new deadline straight back in its heap, so a peer
            // draining a large reply slowly costs one wakeup per window and no
            // registration churn at all.
            TimeoutAction::ToDuration(*window)
        })?;
        if evicted {
            tracing::debug!(
                pending = connection.pending(),
                stall_ms = window.as_millis(),
                "dropped an ipc connection whose peer stopped reading its reply"
            );
            return Ok(PostAction::Remove);
        }
        let mut step = None;
        // Delegated to `Generic` rather than calling `callback` straight away
        // so its stale-token check still runs -- the token it compares against
        // is private to it, so there is no way to repeat that check here.
        socket.process_events(readiness, token, |readiness, _file| {
            step = Some(callback(readiness, connection));
            Ok(PostAction::Continue)
        })?;
        match step {
            // Removing the source drops it, which closes both fds and releases
            // its slot. An armed deadline is left behind in the loop's heap
            // (calloop drops a removed source without unregistering it, so
            // there is nothing to cancel it from); it comes due once, finds no
            // source to dispatch to, and is discarded.
            Some(Step::Close) => Ok(PostAction::Remove),
            // Neither sub-source's token, or the deadline's -- which has
            // already rescheduled itself above. Either way nothing about this
            // connection has changed and there is nothing to re-register.
            None => Ok(PostAction::Continue),
            Some(Step::Continue) => {
                let wanted = connection.interest();
                if same_interest(wanted, socket.interest) {
                    return Ok(PostAction::Continue);
                }
                // Both halves are needed, in this order: the field is what
                // `Generic::reregister` reads, and `PostAction::Reregister` is
                // what gets `reregister` called at all -- the poller was told
                // the old interest when the fd was registered and does not
                // consult the field again on its own. The loop applies this
                // right after this callback returns, re-deriving the same
                // token from the same registration (`TokenFactory::new` then
                // one `token()` is deterministic), so no event can be lost to
                // a token change in between. Checked against calloop 0.14.4's
                // `loop_logic.rs`, `generic.rs` and `token.rs`.
                socket.interest = wanted;
                // The stall deadline runs exactly while this connection waits
                // for writability, so this is also where it is armed and
                // disarmed. Both halves of an arming happen here together and
                // unconditionally: a `Timer` keeps whatever deadline it was
                // last given, and re-registering one whose deadline has since
                // gone by would fire immediately against a fresh baseline --
                // evicting a healthy connection for the first byte it queued.
                *armed = wanted.writable;
                if *armed {
                    stall.set_duration(*window);
                    *stall_progress = connection.total_sent();
                }
                Ok(PostAction::Reregister)
            }
        }
    }

    // Registration order is load-bearing and the same in all three: the socket
    // takes the first sub-token, the deadline the second, so the socket's token
    // does not shift with whether the deadline is armed. calloop re-derives
    // both from the same registration on every re-registration (a fresh
    // `TokenFactory` and one `token()` per sub-source, in order), so nothing
    // has to be remembered between them.

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> calloop::Result<()> {
        self.socket.register(poll, token_factory)?;
        if self.armed {
            self.stall.register(poll, token_factory)?;
        }
        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> calloop::Result<()> {
        self.socket.reregister(poll, token_factory)?;
        // A `Timer` hands its deadline to the event loop when it registers, so
        // a deadline set above only takes effect here -- and one that is no
        // longer wanted only leaves the loop here. Unregistered first either
        // way: registering a timer that already holds a registration would
        // leave two deadlines racing for one connection.
        self.stall.unregister(poll)?;
        if self.armed {
            self.stall.register(poll, token_factory)?;
        }
        Ok(())
    }

    fn unregister(&mut self, poll: &mut Poll) -> calloop::Result<()> {
        self.socket.unregister(poll)?;
        self.stall.unregister(poll)
    }
}

/// `Interest` is two `bool`s with no `PartialEq`, so they get compared by hand.
fn same_interest(a: Interest, b: Interest) -> bool {
    a.readable == b.readable && a.writable == b.writable
}

pub(super) enum Step {
    Continue,
    Close,
}

pub(super) struct Connection {
    /// The socket, the buffer between it and request lines, and the line being
    /// assembled -- which on a non-blocking socket may be half of one, waiting
    /// for the rest to arrive.
    ///
    /// Also how replies get out: writing through `lines.socket()` rather than a
    /// second duplicated fd costs one fewer fd per connection, of the two each
    /// of `slots::MAX_CONNECTIONS` may hold.
    lines: Lines<UnixStream>,
    /// Replies the socket has not taken yet. Empty almost always: a client
    /// that reads its answers never fills its own receive buffer.
    outbound: Outbound,
    /// When this connection last had a screenshot captured for it, or `None`
    /// if it never has. Per connection, not global: one client hammering the
    /// request must not make another client's first one fail.
    last_screenshot: Option<Instant>,
    /// Whether no further request will be read from this connection: its peer
    /// has closed its write half, or it sent something that cannot be
    /// recovered from. The connection stays in the event loop only until
    /// [`Connection::outbound`] has drained, so a reply to a client that asked
    /// and then shut down its write half is still delivered rather than
    /// truncated, and then it closes.
    ///
    /// Both write sites set it for that same meaning. For the unrecoverable
    /// case (a request line past the limit) it is also the end of reading in a
    /// stronger sense: the rest of that line is still arriving with no way to
    /// tell where it ends, so leaving it unread -- the client's own writes
    /// filling the socket and then failing -- is the intended outcome.
    closing: bool,
    /// This connection's claim on one of `slots::MAX_CONNECTIONS`, released
    /// when the connection is dropped.
    ///
    /// `None` only after a `wait-idle` hand-off has moved it into the waiter
    /// (see [`Connection::serve`]), which happens immediately before this
    /// connection leaves the event loop.
    slot: Option<Slot>,
}

impl Connection {
    /// How many bytes of reply this connection's peer has not taken yet.
    fn pending(&self) -> usize {
        self.outbound.pending()
    }

    /// Whether everything queued for this connection has gone out.
    fn is_drained(&self) -> bool {
        self.outbound.is_empty()
    }

    /// How many bytes of reply have gone out since this connection opened.
    fn total_sent(&self) -> u64 {
        self.outbound.total_sent()
    }

    /// What this connection needs to hear about next.
    ///
    /// Never both at once, and that is the whole of the flow control between
    /// wakeups: with nothing queued there is nothing to write, and while
    /// something *is* queued the read side has to be off. Registering for both
    /// would mean either spinning the event loop at full speed (readiness here
    /// is level-triggered, so unread bytes are reported again on every turn) or
    /// reading requests this connection is already behind on answering.
    ///
    /// Precisely: this drops *wakeups for readability*, not reading. A wakeup
    /// that does arrive -- for writability -- still runs the whole serve loop
    /// after flushing, which is deliberate: lines already pulled into the read
    /// buffer have to be answered wherever that wakeup came from, or they are
    /// stranded. What a client with a full queue loses is only the readable
    /// wakeup, so if it never reads its replies the socket never becomes
    /// writable, it is never woken, and its requests are never read. It stalls
    /// -- itself only, which is the point, and exactly as it already would have
    /// against the blocking write this replaced.
    fn interest(&self) -> Interest {
        if self.outbound.is_empty() {
            Interest::READ
        } else {
            Interest::WRITE
        }
    }

    pub(super) fn step(&mut self, state: &mut State) -> Step {
        // Whatever is left over goes first: a writable wakeup means the socket
        // has room, and a readable one cannot be served before this anyway --
        // a reply must never overtake one still going out.
        if let Err(error) = self.flush() {
            tracing::debug!(%error, "dropped an ipc connection whose reply could not be written");
            return Step::Close;
        }
        if self.closing {
            return self.close_when_drained();
        }
        // How much of the socket this wakeup may still take. See
        // `READS_PER_WAKEUP`; running out is what ends the loop on a flood.
        let mut reads = READS_PER_WAKEUP;
        let mut served = false;
        // Set instead of returning, so the one flush below is not skipped on
        // the way out. See there.
        //
        // Distinct from `self.closing`, which is this connection's own state:
        // "read no further requests, then close once the queue has drained".
        // This one is local to the wakeup and means "this call returns
        // `Step::Close`, as soon as it has flushed" -- no draining, because
        // these are the two cases where there is nothing left to drain to (a
        // read that failed, and the `wait-idle` hand-off, which takes the queue
        // with it).
        let mut leaving = false;
        loop {
            // Back-pressure, not a refusal: this connection stops being read
            // until its client catches up, and nothing is dropped or closed.
            // Checked here and not only through `interest()` because the
            // requests this loop is working through have already been read --
            // nothing will report them again -- so this is the only place that
            // can bound how much one wakeup queues up. See
            // `outbound::HIGH_WATER_BYTES`.
            if self.outbound.over_high_water() {
                break;
            }
            match self.lines.next(MAX_REQUEST_BYTES, &mut reads) {
                LineRead::Line => {}
                // Either half a request line with nothing more arrived yet, or
                // this wakeup's read budget spent mid-flood. Both mean the same
                // thing here: the thread goes back to the event loop, the bytes
                // stay put, and what is still in the kernel is reported again.
                LineRead::Incomplete => break,
                LineRead::Eof => {
                    self.closing = true;
                    break;
                }
                LineRead::Failed => {
                    leaving = true;
                    break;
                }
                LineRead::TooLong => {
                    // Nothing to resynchronize to: the rest of this line is
                    // still coming and there is no way to tell where it ends.
                    // Best-effort reply so a legitimately over-long request
                    // gets a reason rather than a bare disconnection, then
                    // done.
                    let _ = self.reply(&Response::error(format!(
                        "request line exceeds the {MAX_REQUEST_BYTES}-byte limit; connection closed"
                    )));
                    tracing::debug!(
                        "closed an ipc connection whose request line ran past the limit"
                    );
                    self.closing = true;
                    break;
                }
            }
            // Marked before serving, not after: by the time `serve` returns,
            // this request's wayland-side messages are already queued, and
            // that's true even on the exits that close the connection right
            // after -- e.g. a Type/Key/Click/Scroll request queues its
            // synthetic input, then `reply()` fails to write back (the
            // client already closed its read side) and this loop exits via
            // `Step::Close`. If `served` were set after `serve` instead,
            // that request's queued input would never reach the flush below.
            // (A `wait-idle` hand-off queues nothing wayland-side in
            // production and doesn't depend on this ordering itself, but the
            // test harness's synthetic case does, which is what caught this.)
            served = true;
            if let Step::Close = self.serve(state) {
                leaving = true;
                break;
            }
            // Everything already read has to be answered before yielding: it
            // has left the kernel, so a level-triggered source will never
            // report it again, and leaving one line here is the pipelining bug
            // this module exists to fix. So this is not a fairness bound (that
            // is `reads`, above) -- it is the common-path exit, and the reason
            // the common case (one request, one reply) costs one `read` per
            // wakeup rather than two. Reading again only to discover nothing is
            // buffered is an `EAGAIN` syscall per request, which is measurable:
            // ~2 jiffies per 50,000 round-trips.
            if self.lines.buffered().is_empty() {
                break;
            }
        }
        if served {
            // Synthetic input (key/pointer) and action-driven configures queue
            // wayland messages on the clients' connections. `mod.rs`'s
            // `post_dispatch` also flushes every client once per dispatch
            // cycle now, strictly after this runs, so this flush is no longer
            // the only thing standing between a queued message and the wire --
            // but flushing here too costs nothing when there's nothing left to
            // flush (see `post_dispatch`'s doc), and doing it here as well
            // means an injected keystroke reaches the client the instant it's
            // sent rather than waiting for this wakeup to fully unwind first.
            // `wait-idle` is why this can't just be left to `post_dispatch`
            // alone without re-checking: an unflushed keystroke is one the
            // client cannot have redrawn for, so the wait would see a quiet
            // compositor and answer `idle` immediately if anything here ever
            // skipped it.
            //
            // The invariant, which every exit above is written to preserve (by
            // breaking rather than returning): if this wakeup served any
            // request at all, the wayland side is flushed before it returns --
            // no exception for the exits that close the connection.
            //
            // Once per wakeup rather than once per served request: the IPC
            // client can see its own reply land microseconds before this runs,
            // but nothing it does in response can reach the compositor until
            // this returns to the event loop, while a pipelined batch would
            // otherwise flush every wayland client once per line in it.
            let _ = state.display_handle.flush_clients();
        }
        if leaving {
            return Step::Close;
        }
        if self.closing {
            self.close_when_drained()
        } else {
            Step::Continue
        }
    }

    /// Closes once there is nothing left to write, and waits for writability
    /// until then.
    fn close_when_drained(&mut self) -> Step {
        if self.outbound.is_empty() {
            Step::Close
        } else {
            Step::Continue
        }
    }

    /// Answers the line [`Lines::next`] just produced.
    fn serve(&mut self, state: &mut State) -> Step {
        // Borrowed only until the request (or an owned error message) is out;
        // the line buffer is needed mutably again the moment either is.
        let decoded = match std::str::from_utf8(self.lines.line()) {
            Ok(text) => decode::<Request>(text).map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        let request = match decoded {
            Ok(request) => request,
            Err(message) => {
                return self.answer(&Response::error(message));
            }
        };

        // Waiting is answered later, from the render loop, so the connection
        // leaves this source and lives on in `pending_idle`.
        if let Request::WaitIdle {
            quiet_ms,
            timeout_ms,
        } = request
        {
            let Ok(stream) = self.lines.socket().try_clone() else {
                return Step::Close;
            };
            let now = Instant::now();
            state.pending_idle.push(PendingIdle {
                stream,
                quiet: Duration::from_millis(quiet_ms),
                timeout: Duration::from_millis(timeout_ms),
                started: now,
                last_progress: now,
                answered: false,
                // Taken, not left behind. This connection is about to leave
                // the event loop, so anything still queued here would have
                // nothing left to write it -- and the idle reply, written
                // later through a clone of the same socket, would land in the
                // middle of a half-written response. `take` also leaves this
                // connection's own queue provably empty, which is what makes
                // the `Step::Close` below an immediate close rather than a
                // drain.
                outbound: std::mem::take(&mut self.outbound),
                // The slot moves with the socket rather than being released
                // here: a waiter still holds an fd, for as long as the
                // `quiet_ms` and `timeout_ms` it chose. Released instead, a
                // client could shed the cap entirely -- park every connection
                // it opens in a `wait-idle` that never comes due, and each one
                // frees its slot on the way into a list nothing bounds.
                _slot: self.slot.take(),
            });
            // The frame timer answers this, but it drops itself when there's
            // nothing to do -- if the compositor was already idle, it needs
            // waking back up or this request would wait forever.
            state.ensure_ticking();
            return Step::Close;
        }

        // A screenshot is the one request that costs a full render, a
        // framebuffer read-back and a PNG encode, all on the single thread
        // that also runs wayland dispatch, input and every other IPC
        // connection (see `screenshot.rs`). Served back-to-back it starves
        // everything else, so a connection that was handed one less than a
        // frame ago is told to come back rather than served a second one at
        // that price. Per request, not per wakeup: several screenshot requests
        // arriving in one write are throttled exactly as if they had arrived
        // one at a time.
        let screenshot = matches!(request, Request::Screenshot { .. });
        if screenshot && screenshot_throttled(self.last_screenshot, Instant::now()) {
            return self.answer(&Response::error(format!(
                "screenshots are limited to one per connection per {}ms, the \
                 compositor's own frame interval: capturing costs a full \
                 render and encode on the thread that serves every other \
                 client. Retry after that long",
                FRAME_INTERVAL.as_millis()
            )));
        }

        let response = state.handle_request(request);
        if screenshot {
            // Stamped once the capture is done, not when the request
            // arrived: the whole point is to leave the event loop a frame's
            // worth of room for everything else *after* a capture, and a
            // capture can easily take longer than a frame itself, which
            // would leave a start-stamped window already expired by the time
            // it mattered (measured: ~170ms for 800x600 in a debug build --
            // the throttle never once fired that way). Recorded whether or
            // not the capture succeeded: `screenshot()` renders before it
            // can fail, so the expensive part was spent either way.
            self.last_screenshot = Some(Instant::now());
        }
        // The wayland-side flush this used to do per request is now done once
        // per wakeup, by `step`, for every request it served.
        self.answer(&response)
    }

    /// Sends one reply, closing the connection if even queueing it failed.
    fn answer(&mut self, response: &Response) -> Step {
        match self.reply(response) {
            Ok(()) => Step::Continue,
            Err(error) => {
                tracing::debug!(%error, "dropped an ipc connection that could not be replied to");
                Step::Close
            }
        }
    }

    /// Writes `response` out, or queues whatever of it the socket won't take.
    fn reply(&mut self, response: &Response) -> std::io::Result<()> {
        let line = encode(response)?;
        // Disjoint fields: the socket is borrowed from `lines`, the queue is
        // mutated. `&UnixStream` is itself a `Write`, so no extra fd is needed
        // to write through.
        let Connection {
            lines, outbound, ..
        } = self;
        outbound.send(&mut lines.socket(), line)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let Connection {
            lines, outbound, ..
        } = self;
        outbound.flush(&mut lines.socket())
    }
}
