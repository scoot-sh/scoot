//! One multiplexed session-bus connection, on a poll-loop fd.
//!
//! The shape the spike decided (`docs/scootbar/spikes/dbus-client.md`):
//! a single connection per bar process, a pending-call table keyed by
//! serial (replies must be demultiplexed — server signals arrive between
//! a call and its reply), match-rule multiplexing, and central
//! `NameOwnerChanged` tracking, all with no thread and no timer. The
//! socket fd is a poll-loop source like the netlink ones; reads and
//! writes never block past what is ready.
//!
//! Two phases: `connect` does the blocking set-up (auth, `Hello`) with
//! timeouts, then hands over a nonblocking [`Conn`]. Everything after is
//!`call` (queue a call with a caller token) and [`Conn::pump`] (flush,
//!read, and report [`Event`]s with owned data). Replies correlate by the
//! token the call carried; signals and incoming method calls arrive as
//! events for the consumer (the tray) to dispatch.
//!
//! A header that is not a message, or an I/O error, kills the connection
//! ([`Conn::dead`]); recovery (reconnect, re-acquire, re-match) belongs to
//! the consumer, which owns the retry policy. A valid message this client
//! does not take (past [`proto::MAX_MESSAGE`], up to the spec's 128 MiB)
//! is skipped whole, and a flood is read only as far as
//! [`READ_WATERMARK`], the rest waiting in the socket: neither costs the
//! connection, since a peer, not the bus, decides what is sent. While such
//! a message is being discarded, one turn reads a bounded amount (a sender
//! that outruns the reader holds one turn, not the bar).

use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::proto::{self, Kind, Message, Writer, check_name, frame_at, frame_header};

/// The most messages one [`Conn::pump`] reports: a storm is bounded
/// reads and bounded work per call. The consumer re-pumps while capped,
/// so a legitimate burst (thirty items answering at once) still drains
/// in its turn; a sustained flood backs up in the socket ([`READ_WATERMARK`])
/// and is worked a turn at a time, never a lost connection.
pub const MAX_EVENTS_PER_TURN: usize = 64;

/// The most calls waiting for a reply: past this `call` is refused and
/// the consumer tries again after the next turn.
pub const MAX_PENDING: usize = 64;

/// Reads stop once this much is staged, leaving the rest in the kernel's
/// socket buffer (the sender backs up, not the bar): one whole capped
/// message and a read's worth, so the largest message still completes.
/// A flood is worked a turn at a time, never a lost connection.
const READ_WATERMARK: usize = proto::MAX_MESSAGE + 64 * 1024;

/// The most one [`Conn::pump`] reads while discarding a message past
/// [`proto::MAX_MESSAGE`]: a sender that outruns the reader holds one
/// turn this long, and the rest waits in the socket (the poll is woken
/// for it, see [`Conn::has_staged_work`]). Reads of ordinary messages
/// are still bounded by [`READ_WATERMARK`] alone.
const DISCARD_BUDGET_PER_PUMP: usize = 256 * 1024;

/// A message past [`proto::MAX_MESSAGE`] has its header fields read, to
/// learn what it answers, only when they are this small; a message with
/// larger fields is skipped unread.
const MAX_OVERSIZE_FIELDS: usize = 64 * 1024;

/// The whole blocking set-up (auth, `Hello`) waits this long at most, in
/// total, not per read: a daemon that is stopped, or a socket that
/// accepts and never answers, costs the bar this once per attempt and
/// never a reply at a time.
const SETUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Queued output past this is a dead connection: a bus that stopped
/// reading while we owe it replies (a peer flooding us with calls) must
/// not grow the outbox without bound.
const MAX_OUTBOX: usize = 2 * proto::MAX_MESSAGE;

/// The bus's well-known name, object and interface.
pub const BUS_NAME: &str = "org.freedesktop.DBus";
pub const BUS_PATH: &str = "/org/freedesktop/DBus";
pub const BUS_INTERFACE: &str = "org.freedesktop.DBus";

/// Where the session bus listens: `DBUS_SESSION_BUS_ADDRESS` when it
/// names a filesystem path (`unix:path=...`, `%xx` escapes decoded,
/// parameters after `,` and further `;`-separated addresses ignored),
/// else the runtime directory's `bus` when no address is set at all.
/// An address that is set but has no path (`unix:abstract=...`, `tcp:`,
/// `autolaunch:`) is `Err(())`: refused, never silently replaced by
/// another bus that happens to exist at the default place. The argument
/// is the address (the env lookup is the caller's), so tests never touch
/// the environment.
pub fn bus_path_for(address: Option<&std::ffi::OsStr>) -> Result<PathBuf, ()> {
    use std::os::unix::ffi::OsStrExt;
    let Some(address) = address.filter(|address| !address.is_empty()) else {
        return Ok(runtime_dir().join("bus"));
    };
    for part in address.as_encoded_bytes().split(|byte| *byte == b';') {
        let Some(unix) = part.strip_prefix(b"unix:") else {
            continue;
        };
        for param in unix.split(|byte| *byte == b',') {
            if let Some(path) = param.strip_prefix(b"path=") {
                let path = unescape(path);
                if !path.is_empty() {
                    return Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&path)));
                }
            }
        }
    }
    Err(())
}

/// A D-Bus address value with its `%xx` escapes decoded (a malformed
/// escape is kept as it is).
fn unescape(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    let mut at = 0;
    while at < value.len() {
        let hex = |byte: u8| char::from(byte).to_digit(16);
        if value[at] == b'%' && at + 2 < value.len() {
            if let (Some(high), Some(low)) = (hex(value[at + 1]), hex(value[at + 2])) {
                out.push((high * 16 + low) as u8);
                at += 3;
                continue;
            }
        }
        out.push(value[at]);
        at += 1;
    }
    out
}

/// The session bus's path from the environment.
pub fn bus_path() -> Result<PathBuf, ()> {
    bus_path_for(std::env::var_os("DBUS_SESSION_BUS_ADDRESS").as_deref())
}

/// The runtime directory, or its conventional fallback.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(format!("/run/user/{}", rustix_uid()))
}

fn rustix_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Why the blocking set-up failed: `Refused` is the bus saying no (or
/// speaking out of turn), with what it said.
#[derive(Debug)]
pub enum SetupError {
    Io(std::io::Error),
    Refused(&'static str),
}

impl core::fmt::Display for SetupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "bus {error}"),
            Self::Refused(why) => write!(f, "bus refused: {why}"),
        }
    }
}

impl From<std::io::Error> for SetupError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            // The socket's timeout firing: the bus did not answer, which
            // is a refusal in all but name.
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                Self::Refused("the bus did not answer in time")
            }
            _ => Self::Io(error),
        }
    }
}

/// Bounds the next blocking read or write on `stream` by what is left of
/// `deadline`: the set-up's timeout is a total, so a bus that answers one
/// byte at a time cannot stretch it.
fn bound(stream: &UnixStream, deadline: Instant) -> Result<(), SetupError> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or(SetupError::Refused("the bus did not answer in time"))?;
    stream.set_read_timeout(Some(left))?;
    stream.set_write_timeout(Some(left))?;
    Ok(())
}

/// What `call` could not queue: the pending table is full, or the
/// message did not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    Full,
    TooLarge,
}

/// One turn's report: replies correlated by the caller's token, signals
/// and incoming method calls with owned data. Bodies are cloned (bus
/// traffic is rare — registrations and icon changes — never per frame),
/// header strings only where the consumer matches on them.
#[derive(Debug)]
pub enum Event {
    /// A method return for the call that carried `token`.
    Reply {
        token: u64,
        signature: String,
        body: Vec<u8>,
    },
    /// An error reply for the call that carried `token`.
    CallError { token: u64, name: String },
    /// The reply to the call that carried `token` was past
    /// [`proto::MAX_MESSAGE`], and was skipped unread: the call is
    /// answered, with nothing the client can use. The connection lives.
    Dropped { token: u64 },
    /// A signal the match rules asked for.
    Signal {
        sender: String,
        path: String,
        interface: String,
        member: String,
        signature: String,
        body: Vec<u8>,
    },
    /// A method call addressed to us (the watcher object): the consumer
    /// answers with [`Conn::reply_return`] or [`Conn::reply_error`].
    MethodCall {
        sender: String,
        path: String,
        interface: String,
        member: String,
        serial: u32,
        signature: String,
        body: Vec<u8>,
    },
}

/// A call waiting for its reply: the serial it went out with, the
/// destination it was made of (a reply from anyone else is refused),
/// the consumer's token, and when it was sent (for [`Conn::expire`]).
#[derive(Debug)]
struct Pending {
    serial: u32,
    callee: String,
    token: u64,
    sent: Instant,
}

/// Whether a reply from `sender` may answer the call made of `callee`:
/// the bus answers its own calls, a unique name answers calls made of
/// it, and anything may answer a call made of a well-known name (who
/// holds it now is the bus's business, not this client's). No sender
/// header at all (a scripted peer; never a daemon, which always sets
/// one) is nothing to check against.
fn sender_matches(callee: &str, sender: Option<&str>) -> bool {
    let Some(sender) = sender else {
        return true;
    };
    if callee == BUS_NAME {
        return sender == BUS_NAME;
    }
    if callee.starts_with(':') {
        return sender == callee;
    }
    true
}

/// The connection: the socket, the staging buffers, the pending-call
/// table. All reads and writes past `connect` are nonblocking.
#[derive(Debug)]
pub struct Conn {
    stream: UnixStream,
    unique: String,
    serial: u32,
    staged: Vec<u8>,
    outbox: Vec<u8>,
    pending: Vec<Pending>,
    dead: bool,
    /// Set while the blocking set-up runs: every read and write is
    /// bounded by what is left of it.
    deadline: Option<Instant>,
    /// Bytes of a message past [`proto::MAX_MESSAGE`] still to be
    /// discarded as they arrive.
    discard: usize,
    /// Reads stopped at [`READ_WATERMARK`] with more waiting in the
    /// socket: the consumer pumps again at once.
    backlog: bool,
    /// An over-cap message was said once, not per message.
    said_oversize: bool,
    /// A refused forged reply was said once, not per message.
    said_forged: bool,
    /// Signals that arrived during the blocking set-up, delivered on the
    /// first [`Conn::pump`].
    stashed: Vec<Event>,
}

impl Conn {
    /// The daemon's unique name for us (`:1.42`).
    pub fn unique(&self) -> &str {
        &self.unique
    }

    /// Whether a refusal or an I/O error killed the connection: the
    /// consumer reconnects.
    pub fn dead(&self) -> bool {
        self.dead
    }

    /// Whether the outbox holds bytes: the consumer polls for `OUT` too.
    pub fn want_write(&self) -> bool {
        !self.outbox.is_empty()
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }

    /// Queues a method call, returning its serial. `body` is the already
    /// marshalled body with signature `body_sig`. `token` comes back on
    /// the reply event. Calls with [`proto::flag::NO_REPLY_EXPECTED`]
    /// take no pending slot (no reply arrives) and report no event.
    /// Nine arguments: a call names its destination, object, interface,
    /// member, body and reply wish, like the protocol's own header.
    #[allow(clippy::too_many_arguments)]
    pub fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
        flags: u8,
        token: u64,
    ) -> Result<u32, CallError> {
        if !self.pending_can_wait(flags) {
            return Err(CallError::Full);
        }
        let serial = self.next_serial();
        let mut writer = Writer::new();
        writer.begin_call(
            serial,
            destination,
            path,
            interface,
            member,
            body_sig,
            flags,
        );
        writer.raw(body);
        let message = writer.finish().ok_or(CallError::TooLarge)?;
        if self.waits_for_reply(flags) {
            self.pending.push(Pending {
                serial,
                callee: destination.to_owned(),
                token,
                sent: Instant::now(),
            });
        }
        self.outbox.extend_from_slice(&message);
        Ok(serial)
    }

    fn waits_for_reply(&self, flags: u8) -> bool {
        flags & proto::flag::NO_REPLY_EXPECTED == 0
    }

    fn pending_can_wait(&self, flags: u8) -> bool {
        !self.waits_for_reply(flags) || self.pending.len() < MAX_PENDING
    }

    /// Forgets the calls sent more than `max_age` ago and returns their
    /// tokens, so the consumer frees what it tracked under them. A peer
    /// that never answers would otherwise hold a slot for the life of
    /// the connection, and enough of them would starve every other call:
    /// no bus answers for it by default (measured: a stock dbus-daemon
    /// session.conf has no reply timeout, dbus-broker has none, and only a
    /// configured daemon limit does), and this client has no timeout of
    /// its own to fall back on.
    /// A reply that arrives late is dropped like any unknown serial.
    /// Called when the table is full, never on a timer: an idle bar
    /// stays at zero wakeups.
    pub fn expire(&mut self, max_age: Duration) -> Vec<u64> {
        let now = Instant::now();
        let mut tokens = Vec::new();
        self.pending.retain(|pending| {
            if now.saturating_duration_since(pending.sent) > max_age {
                tokens.push(pending.token);
                false
            } else {
                true
            }
        });
        tokens
    }

    /// Queues a method return for `to_serial` with a pre-marshalled body,
    /// addressed to `dest` (the call's sender): dbus-daemon 1.16.2 drops
    /// a reply with no destination silently (measured 2026-10-02: the
    /// bytes leave, nothing arrives, no error, no kick), while an
    /// addressed one arrives. An empty destination is omitted, as before.
    pub fn reply_return(&mut self, dest: &str, to_serial: u32, body_sig: &str, body: &[u8]) {
        let serial = self.next_serial();
        let mut writer = Writer::new();
        if dest.is_empty() {
            writer.begin_return(serial, to_serial, body_sig);
        } else {
            writer.begin_return_to(serial, dest, to_serial, body_sig);
        }
        writer.raw(body);
        if let Some(message) = writer.finish() {
            self.outbox.extend_from_slice(&message);
        }
    }

    /// Queues an error reply for `to_serial`, addressed to `dest` (see
    /// [`Conn::reply_return`]).
    pub fn reply_error(&mut self, dest: &str, to_serial: u32, name: &str) {
        let serial = self.next_serial();
        let mut writer = Writer::new();
        if dest.is_empty() {
            writer.begin_error(serial, to_serial, name, "");
        } else {
            writer.begin_error_to(serial, dest, to_serial, name, "");
        }
        if let Some(message) = writer.finish() {
            self.outbox.extend_from_slice(&message);
        }
    }

    /// Bytes staged, unprocessed: a flood must never grow it past the
    /// read watermark.
    #[cfg(test)]
    pub fn staged_len(&self) -> usize {
        self.staged.len()
    }

    /// Bytes of the over-cap message still to discard: while it is
    /// nonzero the connection is mid-skip.
    #[cfg(test)]
    pub fn discard_pending(&self) -> usize {
        self.discard
    }

    /// The per-pump discard budget [`Conn::pump`] stops at.
    #[cfg(test)]
    pub const DISCARD_BUDGET: usize = DISCARD_BUDGET_PER_PUMP;

    /// The watermark reads stop at.
    #[cfg(test)]
    pub const WATERMARK: usize = READ_WATERMARK;

    /// Queues bytes as they are, with no framing or cap of this client's:
    /// the tests' way to send what a hostile or merely different peer
    /// would (a valid message past [`proto::MAX_MESSAGE`]).
    #[cfg(test)]
    pub fn queue_raw(&mut self, bytes: &[u8]) {
        self.outbox.extend_from_slice(bytes);
    }

    /// Queues a broadcast signal with a pre-marshalled body.
    pub fn signal(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
    ) {
        let serial = self.next_serial();
        let mut writer = Writer::new();
        writer.begin_signal(serial, path, interface, member, body_sig);
        writer.raw(body);
        if let Some(message) = writer.finish() {
            self.outbox.extend_from_slice(&message);
        }
    }

    /// One turn on the fd: flushes the outbox, reads what is ready, and
    /// reports up to [`MAX_EVENTS_PER_TURN`] events with whether it
    /// stopped capped. Leftovers stay staged. Any refusal or I/O error
    /// past `WouldBlock` kills the connection instead of panicking it.
    pub fn pump(&mut self) -> (Vec<Event>, bool) {
        let mut events = core::mem::take(&mut self.stashed);
        if self.dead {
            return (events, false);
        }
        self.flush();
        if self.dead {
            return (events, false);
        }
        self.read_ready();
        if self.dead {
            return (events, false);
        }
        // The staged bytes are worked through a cursor and cut once at the
        // end: draining the front per message made a flood of small ones
        // quadratic (a megabyte moved for every hundred bytes read).
        let mut staged = core::mem::take(&mut self.staged);
        let mut at = 0;
        let dead = self.work_staged(&staged, &mut at, &mut events);
        staged.drain(..at);
        self.staged = staged;
        if dead {
            return (events, false);
        }
        // Capped with whole frames still staged, or reads stopped short of
        // the socket's end: the consumer re-pumps at once, so a burst
        // drains in its turn instead of stranding past what the next poll
        // wakes for.
        let capped = (events.len() >= MAX_EVENTS_PER_TURN
            && frame_at(&self.staged).ok().flatten().is_some())
            || self.backlog;
        (events, capped)
    }

    /// Turns the staged bytes from `at` on into events, up to
    /// [`MAX_EVENTS_PER_TURN`]; `at` moves past what was used. `true`
    /// when a header that is no message killed the connection.
    fn work_staged(&mut self, staged: &[u8], at: &mut usize, events: &mut Vec<Event>) -> bool {
        while events.len() < MAX_EVENTS_PER_TURN {
            if self.discard > 0 {
                let n = self.discard.min(staged.len() - *at);
                *at += n;
                self.discard -= n;
                if self.discard > 0 {
                    break;
                }
                continue;
            }
            let rest = &staged[*at..];
            let total = match frame_header(rest) {
                Ok(Some(total)) => total,
                Ok(None) => break,
                Err(()) => {
                    crate::print::warn(format_args!(
                        "scootbar: dbus: dropping a bus frame that is not a message"
                    ));
                    self.dead = true;
                    return true;
                }
            };
            if total > proto::MAX_MESSAGE {
                // Valid, and more than this client takes: skipped whole as
                // its bytes arrive. A reply says which call lost its answer.
                let fields = u32::from_le_bytes([rest[12], rest[13], rest[14], rest[15]]) as usize;
                let prefix = 16 + fields + (8 - (16 + fields) % 8) % 8;
                let known = fields <= MAX_OVERSIZE_FIELDS;
                if known && rest.len() < prefix {
                    break;
                }
                if !self.said_oversize {
                    self.said_oversize = true;
                    crate::print::warn(format_args!(
                        "scootbar: dbus: skipping a {total}-byte message (more than {} are not read)",
                        proto::MAX_MESSAGE
                    ));
                }
                let dropped = if known {
                    Self::oversize_reply(&mut self.pending, &rest[..prefix])
                } else {
                    None
                };
                self.discard = total;
                if let Some(token) = dropped {
                    events.push(Event::Dropped { token });
                }
                continue;
            }
            if rest.len() < total {
                break;
            }
            *at += total;
            if let Some(event) = self.dispatch(&rest[..total]) {
                events.push(event);
            }
        }
        false
    }

    /// Whether `pump` has more to do without the socket saying so: a whole
    /// message is staged past the events one turn takes, or reads stopped
    /// at the watermark. The consumer asks for `OUT` meanwhile, so the poll
    /// returns at once (a socket is nearly always writable) and the work
    /// goes on a turn at a time, the bar's other sources between.
    pub fn has_staged_work(&self) -> bool {
        if self.backlog {
            return true;
        }
        match frame_header(&self.staged) {
            Ok(Some(total)) => total > proto::MAX_MESSAGE || self.staged.len() >= total,
            _ => false,
        }
    }

    /// Drops as much of the message being skipped as is staged.
    fn apply_discard(&mut self) {
        let n = self.discard.min(self.staged.len());
        self.staged.drain(..n);
        self.discard -= n;
    }

    /// The token of the call an over-cap reply (header only) answers, and
    /// the call leaves the pending table. A reply from anyone but the
    /// callee answers nothing (see [`Conn::dispatch`]).
    fn oversize_reply(pending: &mut Vec<Pending>, prefix: &[u8]) -> Option<u64> {
        let message = Message::parse_header(prefix).ok()?;
        if !matches!(message.kind, Kind::MethodReturn | Kind::Error) {
            return None;
        }
        let reply_to = message.reply_serial?;
        let at = pending
            .iter()
            .position(|waiting| waiting.serial == reply_to)?;
        if !sender_matches(&pending[at].callee, message.sender) {
            return None;
        }
        Some(pending.remove(at).token)
    }

    /// Writes the outbox until it is empty or the socket would block. A
    /// real I/O error kills the connection.
    fn flush(&mut self) {
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => {
                    crate::print::warn(format_args!(
                        "scootbar: dbus: the bus closed under a write"
                    ));
                    self.dead = true;
                    return;
                }
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if self.outbox.len() > MAX_OUTBOX {
                        crate::print::warn(format_args!(
                            "scootbar: dbus: the bus stopped reading; dropping the connection"
                        ));
                        self.dead = true;
                    }
                    return;
                }
                Err(error) => {
                    crate::print::warn(format_args!("scootbar: dbus: bus write failed: {error}"));
                    self.dead = true;
                    return;
                }
            }
        }
    }

    /// Reads what is ready into staging, up to [`READ_WATERMARK`], killing
    /// the connection only on a real I/O error. While a message past
    /// [`proto::MAX_MESSAGE`] is being discarded, one pump reads at most
    /// [`DISCARD_BUDGET_PER_PUMP`] more (a sender that outruns the reader
    /// holds one turn this long); the rest waits in the socket with
    /// `backlog` set, so the poll returns at once for it.
    fn read_ready(&mut self) {
        let mut chunk = [0u8; 8192];
        self.backlog = false;
        let mut read = 0;
        loop {
            if self.staged.len() >= READ_WATERMARK {
                // Enough for this turn: the rest waits in the socket.
                self.backlog = true;
                return;
            }
            if self.discard > 0 && read >= DISCARD_BUDGET_PER_PUMP {
                // Enough discarding for this turn.
                self.backlog = true;
                return;
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    crate::print::warn(format_args!("scootbar: dbus: the bus went away"));
                    self.dead = true;
                    return;
                }
                Ok(n) => {
                    self.staged.extend_from_slice(&chunk[..n]);
                    self.apply_discard();
                    read += n;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    crate::print::warn(format_args!("scootbar: dbus: bus read failed: {error}"));
                    self.dead = true;
                    return;
                }
            }
        }
    }

    /// Sorts one frame into an event: replies by the pending table (and
    /// refused when their sender is not the callee), signals and incoming
    /// calls owned, anything else dropped. A refused frame kills the
    /// connection.
    fn dispatch(&mut self, frame: &[u8]) -> Option<Event> {
        let message = Message::parse(frame).ok()?;
        match message.kind {
            Kind::MethodReturn | Kind::Error => {
                let reply_to = message.reply_serial?;
                let at = self
                    .pending
                    .iter()
                    .position(|pending| pending.serial == reply_to)?;
                if !sender_matches(&self.pending[at].callee, message.sender) {
                    // A reply from anyone but the callee (dbus-daemon
                    // delivers an unsolicited one to the destination;
                    // dbus-broker does not): refused, and the flight stays
                    // for the real answer.
                    if !self.said_forged {
                        self.said_forged = true;
                        crate::print::warn(format_args!(
                            "scootbar: dbus: refusing a reply from a peer that was not called"
                        ));
                    }
                    return None;
                }
                let token = self.pending.remove(at).token;
                if message.kind == Kind::Error {
                    return Some(Event::CallError {
                        token,
                        name: message
                            .error
                            .unwrap_or("org.freedesktop.DBus.Error.Failed")
                            .to_owned(),
                    });
                }
                Some(Event::Reply {
                    token,
                    signature: message.signature.to_owned(),
                    body: message.body.rest().to_vec(),
                })
            }
            Kind::Signal => Some(Event::Signal {
                sender: message.sender.unwrap_or("").to_owned(),
                path: message.path.unwrap_or("").to_owned(),
                interface: message.interface.unwrap_or("").to_owned(),
                member: message.member.unwrap_or("").to_owned(),
                signature: message.signature.to_owned(),
                body: message.body.rest().to_vec(),
            }),
            Kind::MethodCall => {
                // Addressed to us (the daemon only delivers what is ours:
                // our unique name or a well-known one we own, which it
                // does not rewrite). The consumer answers what is its
                // object and errors the rest.
                Some(Event::MethodCall {
                    sender: message.sender.unwrap_or("").to_owned(),
                    path: message.path.unwrap_or("").to_owned(),
                    interface: message.interface.unwrap_or("").to_owned(),
                    member: message.member.unwrap_or("").to_owned(),
                    serial: message.serial,
                    signature: message.signature.to_owned(),
                    body: message.body.rest().to_vec(),
                })
            }
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    /// A blocking call for the set-up phase: queues the call and reads
    /// until its reply arrives, stashing signals for the first pump.
    /// `Err` is the error reply's name, or the set-up failure.
    pub fn roundtrip(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
    ) -> Result<(String, Vec<u8>), SetupError> {
        let serial = self
            .call(destination, path, interface, member, body_sig, body, 0, 0)
            .map_err(|error| match error {
                CallError::Full => SetupError::Refused("the bus never answers"),
                CallError::TooLarge => SetupError::Refused("the set-up call does not fit"),
            })?;
        // The pending table's token is unused here (the serial is known);
        // drop the entry the blocking wait replaces.
        self.pending.pop();
        self.flush_blocking()?;
        loop {
            let frame = self.read_frame_blocking()?;
            let message =
                Message::parse(&frame).map_err(|_| SetupError::Refused("a bad set-up reply"))?;
            match message.kind {
                Kind::MethodReturn
                    if message.reply_serial == Some(serial)
                        && sender_matches(destination, message.sender) =>
                {
                    return Ok((message.signature.to_owned(), message.body.rest().to_vec()));
                }
                Kind::Error
                    if message.reply_serial == Some(serial)
                        && sender_matches(destination, message.sender) =>
                {
                    return Err(SetupError::Refused("the bus errored the set-up call"));
                }
                Kind::Signal => self.stashed.push(Event::Signal {
                    sender: message.sender.unwrap_or("").to_owned(),
                    path: message.path.unwrap_or("").to_owned(),
                    interface: message.interface.unwrap_or("").to_owned(),
                    member: message.member.unwrap_or("").to_owned(),
                    signature: message.signature.to_owned(),
                    body: message.body.rest().to_vec(),
                }),
                _ => {}
            }
        }
    }

    /// The set-up's deadline; an instant already past once it is over, so
    /// a blocking call made after it is refused, not unbounded.
    fn setup_deadline(&self) -> Instant {
        self.deadline.unwrap_or_else(Instant::now)
    }

    fn flush_blocking(&mut self) -> Result<(), SetupError> {
        while !self.outbox.is_empty() {
            bound(&self.stream, self.setup_deadline())?;
            match self.stream.write(&self.outbox) {
                Ok(0) => return Err(SetupError::Refused("the bus closed the set-up")),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn read_frame_blocking(&mut self) -> Result<Vec<u8>, SetupError> {
        loop {
            match frame_at(&self.staged) {
                Ok(Some(len)) => return Ok(self.staged.drain(..len).collect()),
                Ok(None) => {}
                Err(()) => return Err(SetupError::Refused("a bad set-up frame")),
            }
            let mut chunk = [0u8; 8192];
            bound(&self.stream, self.setup_deadline())?;
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(SetupError::Refused("the bus closed the set-up")),
                Ok(n) => {
                    self.staged.extend_from_slice(&chunk[..n]);
                    if self.staged.len() > READ_WATERMARK {
                        return Err(SetupError::Refused("the set-up never framed"));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// Connects the session bus at `path`: dials, then runs [`setup`].
pub fn connect(path: &Path) -> Result<Conn, SetupError> {
    setup(UnixStream::connect(path)?)
}

/// Runs the blocking set-up on an open stream: authenticates (`EXTERNAL`
/// with the empty initial response — the exact bytes `busctl` sends, per
/// the spike), says `Hello`, and returns the nonblocking connection with
/// our unique name. Blocking, bounded by timeouts: the set-up is a
/// handful of round trips on a local socket.
pub fn setup(stream: UnixStream) -> Result<Conn, SetupError> {
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let mut stream = stream;
    bound(&stream, deadline)?;
    // The NUL byte starts SASL; the empty `AUTH EXTERNAL` (no hex uid)
    // is what both daemons accept.
    stream.write_all(&[0])?;
    stream.write_all(b"AUTH EXTERNAL\r\n")?;
    expect_line(&mut stream, deadline, "DATA")?;
    bound(&stream, deadline)?;
    stream.write_all(b"DATA\r\n")?;
    let ok = read_line(&mut stream, deadline)?;
    if !ok.starts_with("OK ") {
        return Err(SetupError::Refused("no OK to EXTERNAL"));
    }
    bound(&stream, deadline)?;
    stream.write_all(b"BEGIN\r\n")?;
    let mut conn = Conn {
        stream,
        unique: String::new(),
        serial: 0,
        staged: Vec::new(),
        outbox: Vec::new(),
        pending: Vec::new(),
        dead: false,
        deadline: Some(deadline),
        discard: 0,
        backlog: false,
        said_oversize: false,
        said_forged: false,
        stashed: Vec::new(),
    };
    let (signature, body) = conn.roundtrip(BUS_NAME, BUS_PATH, BUS_INTERFACE, "Hello", "", &[])?;
    if signature != "s" {
        return Err(SetupError::Refused("Hello answered out of shape"));
    }
    let mut reader = proto::Reader::le(&body);
    let unique = reader
        .str()
        .map_err(|_| SetupError::Refused("Hello named nothing"))?;
    check_name(unique).map_err(|_| SetupError::Refused("Hello named badly"))?;
    conn.unique = unique.to_owned();
    conn.deadline = None;
    conn.stream.set_read_timeout(None)?;
    conn.stream.set_write_timeout(None)?;
    conn.stream.set_nonblocking(true)?;
    Ok(conn)
}

/// The bus's next line (to `\r\n`), bounded: a peer that never finishes
/// a line fails the set-up instead of hanging it.
fn read_line(stream: &mut UnixStream, deadline: Instant) -> Result<String, SetupError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        bound(stream, deadline)?;
        match stream.read(&mut byte) {
            Ok(0) => return Err(SetupError::Refused("the bus went quiet")),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
        line.push(byte[0]);
        if line.len() > 512 {
            return Err(SetupError::Refused("a set-up line too long"));
        }
        if line.len() >= 2 && line[line.len() - 2] == b'\r' && line[line.len() - 1] == b'\n' {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).map_err(|_| SetupError::Refused("a bad set-up line"));
        }
    }
}

fn expect_line(stream: &mut UnixStream, deadline: Instant, prefix: &str) -> Result<(), SetupError> {
    let line = read_line(stream, deadline)?;
    if line == prefix || line.starts_with(&format!("{prefix} ")) {
        Ok(())
    } else {
        Err(SetupError::Refused("the bus answered out of turn"))
    }
}
