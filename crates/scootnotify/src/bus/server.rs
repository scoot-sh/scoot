//! The `org.freedesktop.Notifications` server: name ownership, the four
//! methods, the three signals.
//!
//! Ownership is `RequestName` with `DO_NOT_QUEUE`, never replacing and
//! never replaceable — without `ALLOW_REPLACEMENT` nobody takes the name
//! from us, without `REPLACE_EXISTING` we never take it from anyone. A
//! refusal names the owner (through `GetNameOwner`,
//! `GetConnectionUnixProcessID` and `/proc/PID/comm`) and exits 3, which
//! the unit turns into a stopped unit rather than a restart loop.
//!
//! Methods: `GetCapabilities` (body, body-markup, actions, icon-static —
//! not persistence: an expired popup's `NotificationClosed` invalidates
//! the id, and history is a local convenience that never keeps one
//! alive), `Notify` (all eight arguments, bounded), `CloseNotification`
//! (an unknown id gets the spec's empty D-Bus Error message),
//! `GetServerInformation` (`scootnotify`, `scoot`, the crate version,
//! `1.3`). Signals: `NotificationClosed` (reasons 1–4, the id
//! invalidated before the signal is sent), `ActionInvoked` (kept for the
//! control ticket's `msg invoke`; headless nothing sends it),
//! `ActivationToken` (pointer invocations only, in the popups ticket).
//! `org.freedesktop.DBus.Peer` and `Introspectable` answer as specified.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{self, Reader, Writer, request, request_reply};

use crate::bus::notify::{self, NOTIFY_SIGNATURE};
use crate::config::Config;
use crate::model::store::{Closure, IdFile, Store};

/// The bus name, object and interface: the spec's §2 service.
pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// The per-connection cap: 4 MiB fits a 1000x1000 RGBA `image-data`,
/// while scootbar stays at 1 MiB. Past it a call gets
/// `LimitsExceeded`, never silence.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// Sender apps remembered past `MAX_APPS`: unique names are
/// per-connection, so an unbounded map would be the flood's memory.
const MAX_APPS: usize = 1024;

/// How long one bus round trip waits: the daemon is local, and a bus
/// that never answers is a failed start, not a hang.
const ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(5);

/// The name is owned, by whom: printed, then exit 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    /// The owner's unique name, or the bus name when even that failed.
    pub owner: String,
    /// The owner's pid, 0 when the bus would not say.
    pub pid: u32,
    /// `/proc/PID/comm`, empty when unreadable.
    pub comm: String,
}

impl Denied {
    /// The refusal: naming the owner, never taking over.
    pub fn report(&self) -> String {
        let who = if self.comm.is_empty() {
            self.owner.clone()
        } else {
            format!("{} (pid {})", self.comm, self.pid)
        };
        format!(
            "{BUS_NAME} is owned by {who}; scootnotify will not take it over. \
             Stop that daemon first (systemctl --user stop mako) and remove its \
             D-Bus activation file, or it will come back on the next notification."
        )
    }
}

/// Requests the name with `DO_NOT_QUEUE`: `Ok` when primary owner,
/// `Err` naming the owner otherwise. `conn` is connected but unnamed.
pub fn request_name(conn: &mut Conn) -> Result<(), Denied> {
    let mut body = Writer::new();
    body.str(BUS_NAME);
    body.u32(request::DO_NOT_QUEUE);
    let Some(arg) = body.take_body() else {
        return Err(Denied {
            owner: BUS_NAME.to_owned(),
            pid: 0,
            comm: String::new(),
        });
    };
    if conn
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "RequestName",
            "su",
            &arg,
            0,
            1,
        )
        .is_err()
    {
        return Err(Denied {
            owner: BUS_NAME.to_owned(),
            pid: 0,
            comm: String::new(),
        });
    }
    let reply = wait_reply(conn, 1)?;
    if reply == request_reply::PRIMARY_OWNER {
        return Ok(());
    }
    Err(find_owner(conn))
}

/// One bus call to `reply`'s shape, or `Err` naming what failed.
fn wait_reply(conn: &mut Conn, token: u64) -> Result<u32, Denied> {
    let denied = || Denied {
        owner: BUS_NAME.to_owned(),
        pid: 0,
        comm: String::new(),
    };
    let start = Instant::now();
    loop {
        if conn.dead() || start.elapsed() > ROUNDTRIP_TIMEOUT {
            return Err(denied());
        }
        let (events, _) = conn.pump();
        for event in events {
            if let Event::Reply { token: got, signature, body } = event
                && got == token
            {
                return proto::read_request_reply(&signature, &body).map_err(|()| denied());
            }
        }
        poll_in(conn);
    }
}

/// Who owns the name: the owner, its pid, its comm — each step
/// best-effort, degrading to the bus name rather than failing.
fn find_owner(conn: &mut Conn) -> Denied {
    let owner = bus_string(conn, "GetNameOwner", BUS_NAME).unwrap_or_else(|| BUS_NAME.to_owned());
    let pid = bus_uint(conn, "GetConnectionUnixProcessID", &owner).unwrap_or(0);
    let comm = if pid != 0 {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|text| text.trim().to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    Denied { owner, pid, comm }
}

/// One bus string call (`GetNameOwner`): `None` when the bus errored.
fn bus_string(conn: &mut Conn, member: &str, arg: &str) -> Option<String> {
    let mut body = Writer::new();
    body.str(arg);
    let arg = body.take_body()?;
    let token = 100;
    conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        member,
        "s",
        &arg,
        0,
        token,
    )
    .ok()?;
    let start = Instant::now();
    loop {
        if conn.dead() || start.elapsed() > ROUNDTRIP_TIMEOUT {
            return None;
        }
        let (events, _) = conn.pump();
        for event in events {
            match event {
                Event::Reply { token: got, body, .. } if got == token => {
                    return Reader::le(&body).str().ok().map(str::to_owned);
                }
                Event::CallError { token: got, .. } if got == token => return None,
                _ => {}
            }
        }
        poll_in(conn);
    }
}

/// One bus uint call (`GetConnectionUnixProcessID`): `None` on error.
fn bus_uint(conn: &mut Conn, member: &str, arg: &str) -> Option<u32> {
    let mut body = Writer::new();
    body.str(arg);
    let arg = body.take_body()?;
    let token = 101;
    conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        member,
        "s",
        &arg,
        0,
        token,
    )
    .ok()?;
    let start = Instant::now();
    loop {
        if conn.dead() || start.elapsed() > ROUNDTRIP_TIMEOUT {
            return None;
        }
        let (events, _) = conn.pump();
        for event in events {
            match event {
                Event::Reply { token: got, body, .. } if got == token => {
                    return Reader::le(&body).u32().ok();
                }
                Event::CallError { token: got, .. } if got == token => return None,
                _ => {}
            }
        }
        poll_in(conn);
    }
}

/// Waits for the bus fd to read, briefly: the serve loop's patience
/// between pumps.
fn poll_in(conn: &Conn) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let fd = conn.as_fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 20_000_000,
    };
    let _ = poll(&mut fds, Some(&timeout));
}

/// The introspection XML: the four methods, the three signals, and the
/// Peer/Introspectable halves.
pub const INTROSPECT_XML: &str = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \
 \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n\
 <node>\n\
 <interface name=\"org.freedesktop.Notifications\">\n\
 <method name=\"GetCapabilities\"><arg direction=\"out\" type=\"as\"/></method>\n\
 <method name=\"Notify\"><arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"u\"/>\
 <arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"s\"/>\
 <arg direction=\"in\" type=\"as\"/><arg direction=\"in\" type=\"a{sv}\"/><arg direction=\"in\" type=\"i\"/>\
 <arg direction=\"out\" type=\"u\"/></method>\n\
 <method name=\"CloseNotification\"><arg direction=\"in\" type=\"u\"/></method>\n\
 <method name=\"GetServerInformation\">\
 <arg direction=\"out\" type=\"s\"/><arg direction=\"out\" type=\"s\"/>\
 <arg direction=\"out\" type=\"s\"/><arg direction=\"out\" type=\"s\"/></method>\n\
 <signal name=\"NotificationClosed\"><arg type=\"u\"/><arg type=\"u\"/></signal>\n\
 <signal name=\"ActionInvoked\"><arg type=\"u\"/><arg type=\"s\"/></signal>\n\
 <signal name=\"ActivationToken\"><arg type=\"u\"/><arg type=\"s\"/></signal>\n\
 </interface>\n\
 <interface name=\"org.freedesktop.DBus.Peer\">\n\
 <method name=\"Ping\"/>\n\
 <method name=\"GetMachineId\"><arg direction=\"out\" type=\"s\"/></method>\n\
 </interface>\n\
 <interface name=\"org.freedesktop.DBus.Introspectable\">\n\
 <method name=\"Introspect\"><arg direction=\"out\" type=\"s\"/></method>\n\
 </interface>\n\
 </node>\n";

/// The server: the store, the sender-app memory for the close policy,
/// and the method routing.
pub struct Server {
    store: Store,
    /// Unique sender to its last `app_name`: `CloseNotification` carries
    /// only an id, so the same-app half of the policy reads here.
    apps: HashMap<String, String>,
}

impl Server {
    /// A server over `config`, reserving ids through `ids`.
    pub fn new(config: Config, ids: IdFile) -> Self {
        Self {
            store: Store::new(config, ids),
            apps: HashMap::new(),
        }
    }

    /// Routes one method call: replies, and queues the signals its
    /// closes emit. Never panics; a misshapen call is `InvalidArgs`, an
    /// unknown one `UnknownMethod`/`UnknownObject`. Nine arguments: a
    /// call names its sender, object, interface, member, body and time,
    /// like the protocol's own header.
    #[allow(clippy::too_many_arguments)]
    pub fn handle(
        &mut self,
        conn: &mut Conn,
        sender: &str,
        path: &str,
        interface: &str,
        member: &str,
        serial: u32,
        signature: &str,
        body: &[u8],
        now_ms: u64,
    ) {
        if path != OBJECT_PATH {
            conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownObject");
            return;
        }
        match interface {
            INTERFACE => self.handle_notify(conn, sender, member, serial, signature, body, now_ms),
            "org.freedesktop.DBus.Peer" => {
                self.handle_peer(conn, sender, member, serial, signature);
            }
            "org.freedesktop.DBus.Introspectable" => {
                if member == "Introspect" && signature.is_empty() {
                    self.answer_introspect(conn, sender, serial);
                } else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
                }
            }
            _ => {
                conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
            }
        }
    }

    /// Answers `Introspect` with the interface XML.
    fn answer_introspect(&self, conn: &mut Conn, sender: &str, serial: u32) {
        let mut reply = Writer::new();
        reply.str(INTROSPECT_XML);
        let Some(arg) = reply.take_body() else {
            conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
            return;
        };
        conn.reply_return(sender, serial, "s", &arg);
    }

    /// Routes the notification interface's four methods.
    #[allow(clippy::too_many_arguments)]
    fn handle_notify(
        &mut self,
        conn: &mut Conn,
        sender: &str,
        member: &str,
        serial: u32,
        signature: &str,
        body: &[u8],
        now_ms: u64,
    ) {
        match member {
            "GetCapabilities" if signature.is_empty() => {
                let mut reply = Writer::new();
                let Some(cookie) = reply.open_array(4) else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return;
                };
                for capability in ["body", "body-markup", "actions", "icon-static"] {
                    reply.str(capability);
                }
                reply.close_array(cookie);
                let Some(arg) = reply.take_body() else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return;
                };
                conn.reply_return(sender, serial, "as", &arg);
            }
            "Notify" if signature == NOTIFY_SIGNATURE => {
                let mut reader = Reader::le(body);
                let Ok(incoming) = notify::parse(&mut reader, sender) else {
                    invalid(conn, sender, serial, "Notify needs (susssasa{sv}i)");
                    return;
                };
                self.remember_app(sender, &incoming.app_name);
                let (arrival, evicted) =
                    self.store.notify(incoming, Instant::now(), now_ms);
                // Headless arrival is display: the clock starts at once.
                // The popups ticket moves this to the frame callback.
                if let crate::model::store::Arrival::Shown { id, .. } = arrival {
                    self.store.presented(id, Instant::now(), now_ms);
                }
                for closure in evicted.into_iter().chain(self.store.expired(Instant::now())) {
                    self.signal_closed(conn, closure);
                }
                let id = match arrival {
                    crate::model::store::Arrival::Shown { id, .. } => id,
                    crate::model::store::Arrival::Suppressed { id } => id,
                };
                let mut reply = Writer::new();
                reply.u32(id);
                let Some(arg) = reply.take_body() else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return;
                };
                conn.reply_return(sender, serial, "u", &arg);
            }
            "CloseNotification" if signature == "u" => {
                let mut reader = Reader::le(body);
                let Ok(id) = reader.u32() else {
                    invalid(conn, sender, serial, "CloseNotification needs (u)");
                    return;
                };
                if !reader.exhausted() {
                    invalid(conn, sender, serial, "CloseNotification needs (u)");
                    return;
                }
                let app = self.apps.get(sender).map(String::as_str).unwrap_or("");
                match self.store.close(id, sender, app) {
                    Some(closure) => {
                        conn.reply_return(sender, serial, "", &[]);
                        self.signal_closed(conn, closure);
                    }
                    // The spec's empty D-Bus Error message for an id
                    // that no longer exists: a name, no body.
                    None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed"),
                }
            }
            "GetServerInformation" if signature.is_empty() => {
                let mut reply = Writer::new();
                reply.str("scootnotify");
                reply.str("scoot");
                reply.str(env!("CARGO_PKG_VERSION"));
                reply.str("1.3");
                let Some(arg) = reply.take_body() else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return;
                };
                conn.reply_return(sender, serial, "ssss", &arg);
            }
            _ => {
                if matches!(member, "GetCapabilities" | "Notify" | "CloseNotification" | "GetServerInformation") {
                    invalid(conn, sender, serial, "the arguments do not match the method");
                } else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
                }
            }
        }
    }

    /// Routes the Peer half: `Ping` answers empty, `GetMachineId`
    /// answers the host's id.
    fn handle_peer(
        &mut self,
        conn: &mut Conn,
        sender: &str,
        member: &str,
        serial: u32,
        signature: &str,
    ) {
        match (member, signature) {
            ("Ping", "") => conn.reply_return(sender, serial, "", &[]),
            ("GetMachineId", "") => match machine_id() {
                Some(id) => {
                    let mut reply = Writer::new();
                    reply.str(&id);
                    match reply.take_body() {
                        Some(arg) => conn.reply_return(sender, serial, "s", &arg),
                        None => {
                            conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed")
                        }
                    }
                }
                None => invalid(conn, sender, serial, "no machine id on this host"),
            },
            _ => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod"),
        }
    }

    /// Fires due expiries: invalidated first, signalled after.
    pub fn fire_expiries(&mut self, conn: &mut Conn) {
        for closure in self.store.expired(Instant::now()) {
            self.signal_closed(conn, closure);
        }
    }

    /// The earliest running deadline: the timerfd arms to this.
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.store.earliest_deadline()
    }

    /// Remembers a sender's app for the close policy, pruning idle
    /// senders past the bound.
    fn remember_app(&mut self, sender: &str, app: &str) {
        if self.apps.len() > MAX_APPS {
            self.apps.clear();
        }
        self.apps.insert(sender.to_owned(), app.to_owned());
    }

    /// Queues `NotificationClosed(id, reason)`: the id is already
    /// invalidated, as the spec requires.
    fn signal_closed(&self, conn: &mut Conn, closure: Closure) {
        let mut body = Writer::new();
        body.u32(closure.id);
        body.u32(closure.reason.code());
        if let Some(arg) = body.take_body() {
            conn.signal(OBJECT_PATH, INTERFACE, "NotificationClosed", "uu", &arg);
        }
    }
}

/// An `InvalidArgs` error with a body: what the argument was, for the
/// client's user.
fn invalid(conn: &mut Conn, sender: &str, serial: u32, what: &str) {
    let mut body = Writer::new();
    body.str(what);
    match body.take_body() {
        Some(arg) => conn.reply_error_body(
            sender,
            serial,
            "org.freedesktop.DBus.Error.InvalidArgs",
            "s",
            &arg,
        ),
        None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.InvalidArgs"),
    }
}

/// The host's machine id: `/etc/machine-id`, else the D-Bus one.
fn machine_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let id = text.lines().next().unwrap_or("").trim().to_owned();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
