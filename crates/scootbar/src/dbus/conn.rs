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
//! A refused frame or an I/O error kills the connection ([`Conn::dead`]);
//! recovery (reconnect, re-acquire, re-match) belongs to the consumer,
//! which owns the retry policy.

use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::proto::{self, Kind, Message, Writer, check_name, frame_at};

/// The most messages one [`Conn::pump`] reports: a storm is bounded
/// reads and bounded work per call. The consumer re-pumps while capped,
/// so a legitimate burst (thirty items answering at once) still drains
/// in its turn; a sustained flood grows staging into [`MAX_STAGING`],
/// which kills the connection instead.
pub const MAX_EVENTS_PER_TURN: usize = 64;

/// The most calls waiting for a reply: past this `call` is refused and
/// the consumer tries again after the next turn.
pub const MAX_PENDING: usize = 64;

/// Staging past this is a dead connection: two whole capped messages the
/// turns never drained, or garbage no frame accepts.
const MAX_STAGING: usize = 2 * proto::MAX_MESSAGE;

/// A round trip of the blocking set-up waits this long at most.
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The bus's well-known name, object and interface.
pub const BUS_NAME: &str = "org.freedesktop.DBus";
pub const BUS_PATH: &str = "/org/freedesktop/DBus";
pub const BUS_INTERFACE: &str = "org.freedesktop.DBus";

/// Where the session bus listens: `DBUS_SESSION_BUS_ADDRESS` when it
/// names a filesystem path (`unix:path=...`, with parameters after `;`
/// ignored), else the runtime directory's `bus`, else its conventional
/// fallback. Abstract sockets are not dialled (nothing the bar runs on
/// serves the session bus on one); an address without a path falls
/// through to the default. The argument is the address (the env lookup
/// is the caller's), so tests never touch the environment.
pub fn bus_path_for(address: Option<&std::ffi::OsStr>) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    if let Some(address) = address {
        let bytes = address.as_encoded_bytes();
        for part in bytes.split(|byte| *byte == b';') {
            let unix = part.strip_prefix(b"unix:").unwrap_or(part);
            for param in unix.split(|byte| *byte == b',') {
                if let Some(path) = param.strip_prefix(b"path=") {
                    if !path.is_empty() {
                        return PathBuf::from(std::ffi::OsStr::from_bytes(path));
                    }
                }
            }
        }
    }
    runtime_dir().join("bus")
}

/// The session bus's path from the environment.
pub fn bus_path() -> PathBuf {
    bus_path_for(std::env::var_os("DBUS_SESSION_BUS_ADDRESS").as_deref())
}

fn runtime_dir() -> PathBuf {
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
        Self::Io(error)
    }
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

/// The connection: the socket, the staging buffers, the pending-call
/// table. All reads and writes past `connect` are nonblocking.
#[derive(Debug)]
pub struct Conn {
    stream: UnixStream,
    unique: String,
    serial: u32,
    staged: Vec<u8>,
    outbox: Vec<u8>,
    pending: Vec<(u32, u64)>,
    dead: bool,
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

    /// Queued reply bytes, for tests.
    pub fn outbox_len(&self) -> usize {
        self.outbox.len()
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
            self.pending.push((serial, token));
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
        while events.len() < MAX_EVENTS_PER_TURN {
            let consumed = match frame_at(&self.staged) {
                Ok(Some(len)) => len,
                Ok(None) => break,
                Err(()) => {
                    crate::print::warn(format_args!(
                        "scootbar: tray: dropping a refused bus frame"
                    ));
                    self.dead = true;
                    return (events, false);
                }
            };
            let frame: Vec<u8> = self.staged.drain(..consumed).collect();
            if let Some(event) = self.dispatch(&frame) {
                events.push(event);
                continue;
            }
            if self.dead {
                return (events, false);
            }
        }
        // Capped with whole frames still staged: the consumer re-pumps
        // at once, so a burst drains in its turn instead of stranding
        // past what the next poll wakes for.
        let capped =
            events.len() >= MAX_EVENTS_PER_TURN && frame_at(&self.staged).ok().flatten().is_some();
        (events, capped)
    }

    /// Writes the outbox until it is empty or the socket would block. A
    /// real I/O error kills the connection.
    fn flush(&mut self) {
        if !self.outbox.is_empty() {
            eprintln!("DBUGBUS flush {} bytes: {}", self.outbox.len(), self.outbox.iter().map(|b| format!("{b:02x}")).collect::<String>());
        }
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => {
                    crate::print::warn(format_args!(
                        "scootbar: tray: the bus closed under a write"
                    ));
                    self.dead = true;
                    return;
                }
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(error) => {
                    crate::print::warn(format_args!("scootbar: tray: bus write failed: {error}"));
                    self.dead = true;
                    return;
                }
            }
        }
    }

    /// Reads what is ready into staging, killing the connection past
    /// [`MAX_STAGING`] or on a real I/O error.
    fn read_ready(&mut self) {
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    crate::print::warn(format_args!("scootbar: tray: the bus went away"));
                    self.dead = true;
                    return;
                }
                Ok(n) => {
                    self.staged.extend_from_slice(&chunk[..n]);
                    if self.staged.len() > MAX_STAGING {
                        self.dead = true;
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(error) => {
                    crate::print::warn(format_args!("scootbar: tray: bus read failed: {error}"));
                    self.dead = true;
                    return;
                }
            }
        }
    }

    /// Sorts one frame into an event: replies by the pending table,
    /// signals and incoming calls owned, anything else dropped. A refused
    /// frame kills the connection.
    fn dispatch(&mut self, frame: &[u8]) -> Option<Event> {
        let message = Message::parse(frame).ok()?;
        match message.kind {
            Kind::MethodReturn | Kind::Error => {
                let reply_to = message.reply_serial?;
                let at = self
                    .pending
                    .iter()
                    .position(|(serial, _)| *serial == reply_to)?;
                let (_, token) = self.pending.remove(at);
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
                eprintln!("DBUGBUS call {} bytes: {}", frame.len(), frame.iter().map(|b| format!("{b:02x}")).collect::<String>());
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
                Kind::MethodReturn if message.reply_serial == Some(serial) => {
                    return Ok((message.signature.to_owned(), message.body.rest().to_vec()));
                }
                Kind::Error if message.reply_serial == Some(serial) => {
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

    fn flush_blocking(&mut self) -> Result<(), SetupError> {
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => return Err(SetupError::Refused("the bus closed the set-up")),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(error) => return Err(SetupError::Io(error)),
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
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(SetupError::Refused("the bus closed the set-up")),
                Ok(n) => {
                    self.staged.extend_from_slice(&chunk[..n]);
                    if self.staged.len() > MAX_STAGING {
                        return Err(SetupError::Refused("the set-up never framed"));
                    }
                }
                Err(error) => return Err(SetupError::Io(error)),
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
    stream.set_read_timeout(Some(SETUP_TIMEOUT))?;
    stream.set_write_timeout(Some(SETUP_TIMEOUT))?;
    let mut stream = stream;
    // The NUL byte starts SASL; the empty `AUTH EXTERNAL` (no hex uid)
    // is what both daemons accept.
    stream.write_all(&[0])?;
    stream.write_all(b"AUTH EXTERNAL\r\n")?;
    expect_line(&mut stream, "DATA")?;
    stream.write_all(b"DATA\r\n")?;
    let ok = read_line(&mut stream)?;
    if !ok.starts_with("OK ") {
        return Err(SetupError::Refused("no OK to EXTERNAL"));
    }
    stream.write_all(b"BEGIN\r\n")?;
    let mut conn = Conn {
        stream,
        unique: String::new(),
        serial: 0,
        staged: Vec::new(),
        outbox: Vec::new(),
        pending: Vec::new(),
        dead: false,
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
    conn.stream.set_read_timeout(None)?;
    conn.stream.set_write_timeout(None)?;
    conn.stream.set_nonblocking(true)?;
    Ok(conn)
}

/// The bus's next line (to `\r\n`), bounded: a peer that never finishes
/// a line fails the set-up instead of hanging it.
fn read_line(stream: &mut dyn Read) -> Result<String, SetupError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err(SetupError::Refused("the bus went quiet")),
            Ok(_) => {}
            Err(error) => return Err(SetupError::Io(error)),
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

fn expect_line(stream: &mut dyn Read, prefix: &str) -> Result<(), SetupError> {
    let line = read_line(stream)?;
    if line == prefix || line.starts_with(&format!("{prefix} ")) {
        Ok(())
    } else {
        Err(SetupError::Refused("the bus answered out of turn"))
    }
}
