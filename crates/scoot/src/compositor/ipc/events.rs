//! Event subscriptions: push notifications over the control socket.
//!
//! A subscription dedicates one connection to events (see `scoot_ipc::event`
//! for the protocol half): `Request::Subscribe` names the [`EventKind`]s,
//! the reply is `Response::Subscribed`, and afterwards the connection
//! carries [`Response::OutputRemoved`]/[`Response::OutputRestored`]/
//! [`Response::OutputChanged`] unasked as outputs come, go and change size.
//! This module is the server side of that: who is subscribed, getting events
//! to them, and dropping subscribers that stop reading.
//!
//! The shape mirrors the socket's two existing hand-offs (`PendingIdle`,
//! `PendingShot`): a subscriber is a clone of its connection's socket --
//! which shares the file status flags, so it is non-blocking like the
//! original -- plus the same [`Outbound`] queue a connection uses. Events
//! are emitted on the event-loop thread from the hotplug paths
//! (`State::remove_output`, `State::restore_displaced`) and the resize path
//! (`State::resize_output_of`), which are cold
//! (a monitor plug cycle or mode change, never per frame), so one encode
//! per event plus
//! one clone per subscriber is the whole cost, and subscribing costs
//! nothing per message or frame: the connection loop answers requests
//! exactly as before, with one boolean check for the dedicated-connection
//! rule.
//!
//! The backpressure policy, stated once here because a test pins it:
//!
//! - Emitting never blocks the compositor. The socket is non-blocking, so
//!   [`Outbound::send`] either takes the event or queues it; a subscriber
//!   that never reads is *disconnected*, not buffered without bound: once
//!   its queue passes the 1 MiB high-water mark a connection's own replies
//!   observe, the record is dropped and the socket shut down, so the peer
//!   sees the end of the stream. Output removal never waits for a
//!   subscriber.
//! - A tail the socket would not take in one write drains on the frame
//!   tick ([`State::settle_subscribers`]), with the same progress-based
//!   give-up the screenshot replies use: a client draining slowly is never
//!   given up on, however long it takes; a peer that has taken nothing at
//!   all for [`SUBSCRIBER_STALL_TIMEOUT`] is shut down and dropped.
//! - A subscriber record dies with its connection: the accept loop calls
//!   [`State::drop_subscriber`] whenever a connection source leaves the
//!   event loop, so a client that subscribes and disconnects leaves no
//!   record (and no fd) behind even if no event ever fires to reap it.
//!
//! The socket's credential story is unchanged: there is no second socket.
//! A subscriber connects to the same `0600`, same-user control socket every
//! other client uses, and is refused the same way when it is not this
//! compositor's own user.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use scoot_ipc::{EventKind, OutputChanged, OutputRemoved, OutputRestored, Response, encode};

use super::{Outbound, State};

/// How long a subscriber's queued events may go without a byte leaving
/// before the subscription is given up on.
///
/// The same window as `connection.rs`'s `WRITE_STALL_TIMEOUT`, for the same
/// reason: a client draining slowly is making progress and is never given
/// up on, however long it takes -- only a peer that has taken nothing at
/// all in this long is treated as gone. A separate constant rather than a
/// shared one because the test-shortening `Limits` does not reach here
/// (`State` holds no `Limits`); the suites drive `settle_subscribers_at`
/// against an explicit clock instead.
const SUBSCRIBER_STALL_TIMEOUT: Duration = Duration::from_secs(10);

/// One subscribed connection, as `State` sees it.
///
/// The connection itself stays in the event loop (it still needs its read
/// half served, if only to refuse further requests); this is the write
/// half the events go out through.
pub(crate) struct Subscriber {
    /// What `State` files this subscription under: its connection's id, so
    /// the accept loop can drop it when that connection leaves.
    conn: u64,
    /// A clone of the connection's socket: non-blocking through the shared
    /// file status flags, written here, drained here and on the tick.
    stream: UnixStream,
    /// Events the socket has not taken yet. Empty almost always: a client
    /// that reads its events never fills its own receive buffer.
    outbound: Outbound,
    /// The kinds this connection subscribed to.
    events: Vec<EventKind>,
    /// How many bytes have gone out since the last check, for the
    /// progress-based give-up. See [`Outbound::total_sent`] for why the
    /// stall check watches this rather than the queue size.
    progress: u64,
    /// When a byte last went out -- or, if none has, when the subscriber
    /// was created. The no-progress window runs from here.
    last_progress: Instant,
}

impl State {
    /// Subscribes `conn` to `events`, dedicating its connection to them.
    ///
    /// An empty list is refused with an error rather than held: a
    /// subscription to nothing would pin a connection slot (and an fd)
    /// for events that can never arrive.
    pub fn subscribe(&mut self, conn: u64, stream: UnixStream, events: Vec<EventKind>) -> Response {
        if events.is_empty() {
            return Response::error(
                "subscribe names no event kinds; name at least one (\"output\"), \
                 or open a connection for requests instead",
            );
        }
        let now = Instant::now();
        remove_subscriber(conn, &mut self.subscribers);
        self.subscribers.push(Subscriber {
            conn,
            stream,
            outbound: Outbound::default(),
            events: events.clone(),
            progress: 0,
            last_progress: now,
        });
        Response::Subscribed { events }
    }

    /// Drops `conn`'s subscription, if it has one. Called whenever a
    /// connection source leaves the event loop, so a disconnected
    /// subscriber leaves no record behind.
    pub(crate) fn drop_subscriber(&mut self, conn: u64) {
        remove_subscriber(conn, &mut self.subscribers);
    }

    /// Sends an output-removed event to every `Output` subscriber.
    /// See the module doc for what happens to one that stops reading.
    pub fn emit_output_removed(&mut self, event: OutputRemoved) {
        let Ok(line) = encode(&Response::OutputRemoved(event)) else {
            return;
        };
        self.emit(EventKind::Output, &line);
    }

    /// Sends an output-restored event to every `Output` subscriber.
    pub fn emit_output_restored(&mut self, event: OutputRestored) {
        let Ok(line) = encode(&Response::OutputRestored(event)) else {
            return;
        };
        self.emit(EventKind::Output, &line);
    }

    /// Sends an output-changed event to every `Output` subscriber.
    /// See the module doc for what happens to one that stops reading.
    pub fn emit_output_changed(&mut self, event: OutputChanged) {
        let Ok(line) = encode(&Response::OutputChanged(event)) else {
            return;
        };
        self.emit(EventKind::Output, &line);
    }

    /// The shared tail of all three emitters: one encoded line to every
    /// subscriber of `kind`.
    ///
    /// `line` is encoded once, outside, and cloned per subscriber -- one
    /// small allocation each, on a hotplug path, never per frame.
    fn emit(&mut self, kind: EventKind, line: &str) {
        if self.subscribers.is_empty() {
            return;
        }
        let mut draining = false;
        self.subscribers.retain_mut(|subscriber| {
            if !subscriber.events.contains(&kind) {
                return true;
            }
            let Subscriber {
                stream, outbound, ..
            } = subscriber;
            match outbound.send(&mut &*stream, line.to_string()) {
                Err(error) => {
                    tracing::debug!(%error, conn = subscriber.conn, "dropped a subscriber its events could not be written to");
                    shut(subscriber);
                    false
                }
                Ok(()) => {
                    if outbound.over_high_water() {
                        tracing::warn!(
                            conn = subscriber.conn,
                            pending = outbound.pending(),
                            "dropped a subscriber that stopped reading its events"
                        );
                        shut(subscriber);
                        false
                    } else {
                        draining = draining || !outbound.is_empty();
                        true
                    }
                }
            }
        });
        if draining {
            // Part-written tails retry on the frame tick, which may have
            // dropped itself on a quiet screen.
            self.ensure_ticking();
        }
    }

    /// Pushes out subscriber tails the socket would not take in one write.
    pub fn settle_subscribers(&mut self) {
        self.settle_subscribers_at(Instant::now());
    }

    /// [`State::settle_subscribers`] against an explicit clock, so the
    /// suites can wait out the give-up window without waiting it out.
    pub fn settle_subscribers_at(&mut self, now: Instant) {
        if self.subscribers.is_empty() {
            return;
        }
        self.subscribers.retain_mut(|subscriber| {
            if subscriber.outbound.is_empty() {
                return true;
            }
            let Subscriber {
                stream,
                outbound,
                progress,
                last_progress,
                ..
            } = subscriber;
            match outbound.flush(&mut &*stream) {
                Err(error) => {
                    tracing::debug!(%error, conn = subscriber.conn, "dropped a subscriber its events could not be written to");
                    shut(subscriber);
                    false
                }
                Ok(()) => {
                    if outbound.is_empty() {
                        // Fully out -- and the subscription stays: an empty
                        // queue means done for now, not done forever.
                        true
                    } else {
                        let sent = outbound.total_sent();
                        if sent != *progress {
                            *progress = sent;
                            *last_progress = now;
                            true
                        } else if now.duration_since(*last_progress)
                            >= SUBSCRIBER_STALL_TIMEOUT
                        {
                            tracing::warn!(
                                conn = subscriber.conn,
                                pending = outbound.pending(),
                                "gave up writing a subscriber's events: the client stopped reading"
                            );
                            shut(subscriber);
                            false
                        } else {
                            true
                        }
                    }
                }
            }
        });
    }

    /// Whether any subscriber still has event bytes to push out -- which is
    /// what keeps the frame timer alive until they are gone (see
    /// `frame_tick`). A subscriber with nothing queued needs nothing from
    /// the tick; the next emission wakes it.
    pub fn subscribers_draining(&self) -> bool {
        self.subscribers
            .iter()
            .any(|subscriber| !subscriber.outbound.is_empty())
    }
}

/// Removes every subscription filed under `conn`, for [`State::subscribe`]
/// (a second subscribe from one connection is impossible -- the first
/// dedicates it -- but the record key must stay unique regardless) and
/// [`State::drop_subscriber`] (a disconnected subscriber leaves nothing).
fn remove_subscriber(conn: u64, subscribers: &mut Vec<Subscriber>) {
    if let Some(index) = subscribers.iter().position(|s| s.conn == conn) {
        subscribers.remove(index);
    }
}

/// Shuts a dropped subscriber's socket down, so its peer sees the end of
/// the stream rather than a connection gone quiet. Best-effort: there is
/// nothing to do about a shutdown that fails, and the record is dropped
/// either way.
fn shut(subscriber: &Subscriber) {
    use std::net::Shutdown;
    let _ = subscriber.stream.shutdown(Shutdown::Both);
}
