//! The tray's scripted session bus: the daemon, every item and (where
//! scripted) the other watcher, driven by the test. Test-only: compiled
//! under `#[cfg(test)]`, never into the bar.
//!
//! Synchronous and deterministic: one end of a socketpair goes to the
//! module, and the test drives this end with [`Fake::pump`] between the
//! module's own turns — no threads past the set-up, no timing, no locks.
//! (The module's `Hello` is a blocking round trip, so a short-lived
//! thread serves SASL and that first reply; everything after is pumped.)
//! Blocking, with a timeout on every read, so a module that stops
//! talking fails the test instead of hanging it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::dbus::proto::{Kind, Message, Reader, Writer, frame_at};

/// Our assigned unique name: what the module's sender is.
pub const MODULE: &str = ":1.200";
/// The bus itself.
const BUS: &str = "org.freedesktop.DBus";
/// A read waits this long at most: failure, not patience.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A frame the module sent, recorded with its parsed shape: calls at
/// items and the bus, answers to the fake's own calls, and signals.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub kind: Kind,
    pub serial: u32,
    pub reply_to: Option<u32>,
    pub destination: String,
    pub path: String,
    pub member: String,
    pub signature: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Default)]
struct State {
    /// The `RequestName` word (primary owner by default; `Exists` with
    /// `watcher_owner` set means host mode).
    request_word: u32,
    /// The other watcher's unique name (`None` is no other watcher).
    watcher_owner: Option<String>,
    /// The other watcher's item ids (`service/path`, the KDE format).
    watcher_items: Vec<String>,
    /// Bus names listed past the bus and the module itself.
    names: Vec<String>,
    /// Well-known name to unique name.
    owners: HashMap<String, String>,
    /// Item service to its `GetAll` body (the `a{sv}` bytes).
    props: HashMap<String, Vec<u8>>,
    /// Host registrations the module made in host mode.
    hosts: Vec<String>,
}

/// The scripted bus: scripted through the handle, served by [`Fake::pump`].
pub struct Fake {
    /// Driver writes: item signals, registrations, owner changes.
    send: UnixStream,
    /// Serving reads and writes, nonblocking once the set-up hands over.
    bus: UnixStream,
    staged: Vec<u8>,
    outbox: Vec<u8>,
    state: State,
    calls: Vec<RecordedCall>,
    serial: u32,
    setup_thread: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    /// A bus on a socketpair: the module's end and the driver. A
    /// short-lived thread serves SASL and the `Hello` the module's
    /// blocking set-up waits on; the first [`Fake::pump`] joins it and
    /// serves the rest synchronously.
    pub fn pair() -> (UnixStream, Fake) {
        let (module, bus) = UnixStream::pair().unwrap();
        module.set_read_timeout(Some(TIMEOUT)).unwrap();
        module.set_write_timeout(Some(TIMEOUT)).unwrap();
        bus.set_read_timeout(Some(TIMEOUT)).unwrap();
        bus.set_write_timeout(Some(TIMEOUT)).unwrap();
        let send = bus.try_clone().unwrap();
        let serving = bus.try_clone().unwrap();
        let thread = std::thread::spawn(move || setup_server(serving));
        let mut fake = Fake {
            send,
            bus,
            staged: Vec::new(),
            outbox: Vec::new(),
            state: State {
                request_word: 1,
                ..State::default()
            },
            calls: Vec::new(),
            serial: 1000,
            setup_thread: Some(thread),
        };
        fake.nonblocking();
        (module, fake)
    }

    /// Serves every complete frame waiting: answers calls, records the
    /// rest. Never blocks. Joins the set-up thread first (already past
    /// `Hello` whenever the module connected).
    pub fn pump(&mut self) {
        if let Some(thread) = self.setup_thread.take() {
            let _ = thread.join();
        }
        self.flush();
        loop {
            let mut chunk = [0u8; 8192];
            match self.bus.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => self.staged.extend_from_slice(&chunk[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(_) => return,
            }
            while let Ok(Some(len)) = frame_at(&self.staged) {
                let frame: Vec<u8> = self.staged.drain(..len).collect();
                if self.serve_frame(&frame).is_err() {
                    return;
                }
            }
            if self.staged.len() > 2 * 1024 * 1024 {
                return;
            }
        }
    }

    /// Scripts an item: listed, owned, and answering `GetAll` with
    /// `props` (the `a{sv}` body bytes; see `item_body`).
    pub fn add_item(&mut self, service: &str, owner: &str, props: Vec<u8>) {
        if !self.state.names.contains(&service.to_owned()) {
            self.state.names.push(service.to_owned());
        }
        self.state.owners.insert(service.to_owned(), owner.to_owned());
        self.state.props.insert(service.to_owned(), props);
    }

    /// Scripts the other watcher owning the KDE name (host mode): the
    /// request word becomes `Exists`, and its item list is `ids`
    /// (`service/path` each).
    pub fn set_watcher(&mut self, owner: &str, ids: &[&str]) {
        self.state.request_word = 3;
        self.state.watcher_owner = Some(owner.to_owned());
        self.state.watcher_items = ids.iter().map(|id| id.to_string()).collect();
        if !self.state.names.contains(&owner.to_owned()) {
            self.state.names.push(owner.to_owned());
        }
    }

    /// Sends `NameOwnerChanged(name, old, new)` from the bus.
    pub fn send_name_owner_changed(&mut self, name: &str, old: &str, new: &str) {
        let mut body = Writer::new();
        body.str(name);
        body.str(old);
        body.str(new);
        let bytes = body.take_body().unwrap();
        self.send_signal(BUS, "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged", "sss", &bytes);
    }

    /// Sends an item signal (`NewIcon` etc.) from `sender`.
    pub fn send_item_signal(&mut self, sender: &str, member: &str) {
        self.send_signal(sender, "/StatusNotifierItem", "org.kde.StatusNotifierItem", member, "", &[]);
    }

    /// Sends a `NewStatus` with its argument.
    pub fn send_new_status(&mut self, sender: &str, status: &str) {
        let mut body = Writer::new();
        body.str(status);
        let bytes = body.take_body().unwrap();
        self.send_signal(sender, "/StatusNotifierItem", "org.kde.StatusNotifierItem", "NewStatus", "s", &bytes);
    }

    /// Sends a `RegisterStatusNotifierItem(service)` call addressed to
    /// the module, as the daemon would deliver it from `sender`. Returns
    /// the call's serial, which the module's answer quotes.
    pub fn send_register(&mut self, sender: &str, service: &str) -> u32 {
        let mut body = Writer::new();
        body.str(service);
        let bytes = body.take_body().unwrap();
        let serial = self.next_serial();
        let message = frame_with_sender(
            1,
            serial,
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
            "RegisterStatusNotifierItem",
            Some(MODULE),
            sender,
            "s",
            &bytes,
        );
        self.send.write_all(&message).unwrap();
        serial
    }

    /// Sends the other watcher's `StatusNotifierItemRegistered(service)`.
    pub fn send_watcher_registered(&mut self, service: &str) {
        let owner = self.state.watcher_owner.clone().unwrap_or_default();
        let mut body = Writer::new();
        body.str(service);
        let bytes = body.take_body().unwrap();
        self.send_signal(&owner, "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher", "StatusNotifierItemRegistered", "s", &bytes);
    }

    /// Frames the module sent so far, without draining.
    pub fn calls_len(&self) -> usize {
        self.calls.len()
    }

    /// The frames the module sent, drained.
    pub fn calls(&mut self) -> Vec<RecordedCall> {
        core::mem::take(&mut self.calls)
    }

    /// The host registrations the module made, drained.
    pub fn hosts(&mut self) -> Vec<String> {
        core::mem::take(&mut self.state.hosts)
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    fn nonblocking(&mut self) {
        self.bus.set_read_timeout(None).unwrap();
        self.bus.set_write_timeout(None).unwrap();
        rustix::fs::fcntl_setfl(&self.bus, rustix::fs::OFlags::NONBLOCK).unwrap();
    }

    /// Flushes queued reply bytes; leftovers wait for the next turn.
    fn flush(&mut self) {
        while !self.outbox.is_empty() {
            match self.bus.write(&self.outbox) {
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(_) => return,
            }
        }
    }

    fn send_signal(&mut self, sender: &str, path: &str, interface: &str, member: &str, sig: &str, body: &[u8]) {
        let serial = self.next_serial();
        let message = frame_with_sender(4, serial, path, interface, member, None, sender, sig, body);
        self.send.write_all(&message).unwrap();
    }

    /// Answers one frame. `Err` stops the pump (the peer hung up or broke
    /// the protocol); unknown calls are error replies, never silence.
    fn serve_frame(&mut self, frame: &[u8]) -> Result<(), ()> {
        let message = Message::parse(frame).map_err(|_| ())?;
        if message.kind != Kind::MethodCall {
            self.calls.push(record(&message));
            return Ok(());
        }
        let serial = self.next_serial();
        let (member, destination, path) = (
            message.member.unwrap_or(""),
            message.destination.unwrap_or(""),
            message.path.unwrap_or(""),
        );
        let body = message.body.rest();
        if destination == BUS || destination.is_empty() {
            return self.serve_bus(message.serial, serial, member, body);
        }
        if path == "/StatusNotifierWatcher" {
            // Calls at the module's own object come here only in
            // host-mode tests driving both ends; recorded like item calls.
            self.calls.push(record(&message));
            return Ok(());
        }
        // An item call: `GetAll`/`Get` are answered from the script;
        // anything else is recorded (activation wants no reply).
        if member == "GetAll" {
            if let Some(props) = self.state.props.get(destination).cloned() {
                return self.reply(message.serial, serial, "a{sv}", &props);
            }
        }
        if member == "Get" {
            // A single property, variant-wrapped: answer from the
            // scripted dictionary where it has one.
            let mut reader = Reader::le(body);
            let (Ok(interface), Ok(property)) = (reader.str(), reader.str()) else {
                return self.error(message.serial, serial, "org.freedesktop.DBus.Error.InvalidArgs");
            };
            let _ = interface;
            if let Some(props) = self.state.props.get(destination) {
                if let Some(variant) = find_prop(props, property) {
                    return self.reply(message.serial, serial, "v", &variant);
                }
            }
            return self.error(message.serial, serial, "org.freedesktop.DBus.Error.UnknownMethod");
        }
        self.calls.push(record(&message));
        Ok(())
    }

    fn reply(&mut self, to: u32, serial: u32, sig: &str, body: &[u8]) -> Result<(), ()> {
        let mut writer = Writer::new();
        writer.begin_return(serial, to, sig);
        writer.raw(body);
        let message = writer.finish().ok_or(())?;
        self.outbox.extend_from_slice(&message);
        Ok(())
    }

    fn error(&mut self, to: u32, serial: u32, name: &str) -> Result<(), ()> {
        let mut writer = Writer::new();
        writer.begin_error(serial, to, name, "");
        let message = writer.finish().ok_or(())?;
        self.outbox.extend_from_slice(&message);
        Ok(())
    }

    /// Serves the bus's own methods.
    fn serve_bus(&mut self, to: u32, serial: u32, member: &str, body: &[u8]) -> Result<(), ()> {
        match member {
            "Hello" => {
                let mut out = Writer::new();
                out.str(MODULE);
                let bytes = out.take_body().ok_or(())?;
                self.reply(to, serial, "s", &bytes)
            }
            "RequestName" => {
                let word = self.state.request_word;
                let mut out = Writer::new();
                out.u32(word);
                let bytes = out.take_body().ok_or(())?;
                self.reply(to, serial, "u", &bytes)
            }
            "AddMatch" => Ok(()),
            "ListNames" => {
                let mut out = Writer::new();
                let Some(cookie) = out.open_array(4) else {
                    return self.error(to, serial, "org.freedesktop.DBus.Error.Failed");
                };
                out.str(BUS);
                out.str(MODULE);
                for name in &self.state.names {
                    out.str(name);
                }
                if let Some(owner) = &self.state.watcher_owner {
                    out.str(owner);
                }
                out.close_array(cookie);
                let Some(bytes) = out.take_body() else {
                    return self.error(to, serial, "org.freedesktop.DBus.Error.Failed");
                };
                self.reply(to, serial, "as", &bytes)
            }
            "GetNameOwner" => {
                let mut reader = Reader::le(body);
                let Ok(name) = reader.str() else {
                    return self.error(to, serial, "org.freedesktop.DBus.Error.InvalidArgs");
                };
                let owner = if name == BUS || name == MODULE {
                    Some(name.to_owned())
                } else if name == "org.kde.StatusNotifierWatcher"
                    || name == "org.freedesktop.StatusNotifierWatcher"
                {
                    self.state.watcher_owner.clone()
                } else {
                    self.state.owners.get(name).cloned()
                };
                match owner {
                    Some(owner) => {
                        let mut out = Writer::new();
                        out.str(&owner);
                        let bytes = out.take_body().ok_or(())?;
                        self.reply(to, serial, "s", &bytes)
                    }
                    None => self.error(to, serial, "org.freedesktop.DBus.Error.NameHasNoOwner"),
                }
            }
            _ => self.error(to, serial, "org.freedesktop.DBus.Error.UnknownMethod"),
        }
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.send.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.setup_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Serves SASL and the first call (`Hello`) on a short-lived thread: the
/// module's blocking set-up waits on exactly this, and the first
/// [`Fake::pump`] joins the thread before serving the rest. Blocking, so
/// no scheduling race; bounded by timeouts, so a module that never
/// speaks fails the test instead of hanging it.
fn setup_server(mut stream: UnixStream) {
    if sasl(&mut stream).is_err() {
        return;
    }
    let mut staged = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut serial = 1000u32;
    loop {
        match frame_at(&staged) {
            Ok(Some(len)) => {
                let frame: Vec<u8> = staged.drain(..len).collect();
                let Ok(message) = Message::parse(&frame) else {
                    return;
                };
                if message.kind != Kind::MethodCall || message.member != Some("Hello") {
                    return;
                }
                serial = serial.wrapping_add(1).max(1);
                let mut out = Writer::new();
                out.str(MODULE);
                let Ok(bytes) = out.take_body().ok_or(()) else {
                    return;
                };
                let mut writer = Writer::new();
                writer.begin_return(serial, message.serial, "s");
                writer.raw(&bytes);
                let Some(answer) = writer.finish() else {
                    return;
                };
                if stream.write_all(&answer).is_err() {
                    return;
                }
                return;
            }
            Ok(None) => {}
            Err(()) => return,
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => staged.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }
    }
}

/// The SASL opening: the empty `EXTERNAL`, exactly as the spike traced
/// `busctl` sending it.
fn sasl(stream: &mut UnixStream) -> Result<(), ()> {
    let mut first = [0u8; 1];
    read_exact(stream, &mut first)?;
    if first != [0] {
        return Err(());
    }
    let auth = read_line(stream)?;
    if auth != "AUTH EXTERNAL" {
        return Err(());
    }
    stream.write_all(b"DATA\r\n").map_err(|_| ())?;
    let data = read_line(stream)?;
    if data != "DATA" && !data.starts_with("DATA ") {
        return Err(());
    }
    stream.write_all(b"OK 8d Expedition fake bus guid\r\n").map_err(|_| ())?;
    let begin = read_line(stream)?;
    if begin != "BEGIN" {
        return Err(());
    }
    Ok(())
}

fn read_exact(stream: &mut UnixStream, mut buf: &mut [u8]) -> Result<(), ()> {
    while !buf.is_empty() {
        match stream.read(buf) {
            Ok(0) => return Err(()),
            Ok(n) => buf = &mut buf[n..],
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

fn read_line(stream: &mut UnixStream) -> Result<String, ()> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        read_exact(stream, &mut byte)?;
        line.push(byte[0]);
        if line.len() > 512 {
            return Err(());
        }
        if line.len() >= 2 && line[line.len() - 2] == b'\r' && line[line.len() - 1] == b'\n' {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).map_err(|_| ());
        }
    }
}

/// Sets the sender header of an outgoing frame: builds the message
/// header by hand (fixed part plus the fields array with the code-7
/// entry), the way the daemon does for every message it routes.
fn frame_with_sender(
    kind: u8,
    serial: u32,
    path: &str,
    interface: &str,
    member: &str,
    destination: Option<&str>,
    sender: &str,
    signature: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut fields = Writer::new();
    let mut entry = |code: u8, sig: &str, write: &dyn Fn(&mut Writer)| {
        fields.open_struct();
        fields.u8(code);
        fields.signature(sig);
        write(&mut fields);
        fields.close_struct();
    };
    entry(1, "o", &|w| w.str(path));
    entry(2, "s", &|w| w.str(interface));
    entry(3, "s", &|w| w.str(member));
    if let Some(destination) = destination {
        entry(6, "s", &|w| w.str(destination));
    }
    entry(7, "s", &|w| w.str(sender));
    entry(8, "g", &|w| w.signature(signature));
    let fields = fields.take_body().unwrap_or_default();
    let mut out = Vec::new();
    out.extend_from_slice(&[b'l', kind, 0, 1]);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    out.extend_from_slice(&fields);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    out.extend_from_slice(body);
    out
}

/// One frame the module sent, as a record.
fn record(message: &Message<'_>) -> RecordedCall {
    RecordedCall {
        kind: message.kind,
        serial: message.serial,
        reply_to: message.reply_serial,
        destination: message.destination.unwrap_or("").to_owned(),
        path: message.path.unwrap_or("").to_owned(),
        member: message.member.unwrap_or("").to_owned(),
        signature: message.signature.to_owned(),
        body: message.body.rest().to_vec(),
    }
}

/// Finds one property's value bytes in a scripted `a{sv}` body: the
/// signature and the raw value, rebuilt as a variant.
fn find_prop(props: &[u8], property: &str) -> Option<Vec<u8>> {
    let mut reader = Reader::le(props);
    let raw = reader.array_raw(8).ok()?;
    let mut entries = Reader::le(raw);
    while !entries.exhausted() {
        entries.enter_struct().ok()?;
        let key = entries.str().ok()?;
        let sig = entries.signature().ok()?;
        if key == property {
            let rest = entries.rest();
            let mut scoped = Reader::le(rest);
            scoped.skip(sig).ok()?;
            let consumed = rest.len() - scoped.rest().len();
            let mut out = Writer::new();
            out.variant(sig);
            out.raw(&rest[..consumed]);
            entries.leave_struct();
            return out.take_body();
        }
        if entries.skip(sig).is_err() {
            return None;
        }
        entries.leave_struct();
    }
    None
}

/// Builds an item's `GetAll` body: status, title, one pixmap, tooltip,
/// menu and `ItemIsMenu`. `argb` is `width × height` `ARGB32` in
/// network order.
pub fn item_body(title: &str, status: &str, width: u32, height: u32, argb: &[u8]) -> Vec<u8> {
    let mut body = Writer::new();
    let Some(cookie) = body.open_array(8) else {
        return Vec::new();
    };
    entry(&mut body, "Status", "s", &|w| w.str(status));
    entry(&mut body, "Title", "s", &|w| w.str(title));
    entry(&mut body, "IconPixmap", "a(iiay)", &|w| {
        if let Some(cookie) = w.open_array(8) {
            w.open_struct();
            w.u32(width);
            w.u32(height);
            if let Some(cookie) = w.open_array(1) {
                w.raw(argb);
                w.close_array(cookie);
            }
            w.close_struct();
            w.close_array(cookie);
        }
    });
    entry(&mut body, "ToolTip", "(sa(iiay)ss)", &|w| {
        w.open_struct();
        w.str("");
        if let Some(cookie) = w.open_array(8) {
            w.close_array(cookie);
        }
        w.str(title);
        w.str("");
        w.close_struct();
    });
    entry(&mut body, "Menu", "o", &|w| w.str("/Menu"));
    entry(&mut body, "ItemIsMenu", "b", &|w| w.boolean(false));
    body.close_array(cookie);
    body.take_body().unwrap_or_default()
}

fn entry(body: &mut Writer, key: &str, sig: &str, write: &dyn Fn(&mut Writer)) {
    body.open_struct();
    body.str(key);
    body.variant(sig);
    write(body);
    body.close_struct();
}

/// Solid `ARGB32` in network order: one `a` then `r`, `g`, `b` a pixel.
pub fn solid(w: u32, h: u32, a: u8, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for _ in 0..w as usize * h as usize {
        pixels.extend_from_slice(&[a, r, g, b]);
    }
    pixels
}
