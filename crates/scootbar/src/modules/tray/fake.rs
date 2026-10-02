//! The tray's scripted session bus: the daemon, every item and (where
//! scripted) the other watcher, in one test thread. Test-only: compiled
//! under `#[cfg(test)]`, never into the bar.
//!
//! One end of a socketpair goes to the module; this end serves the bus
//! protocol on a thread: SASL, `Hello` (assigning `:1.200`), `RequestName`
//! (the scripted word), `AddMatch` (recorded, never replied — the module
//! asks for no reply), `ListNames`, `GetNameOwner`, item `GetAll`s, and
//! the existing watcher's `Get` in host mode. Calls at items
//! (`Activate`, `Scroll`, ...) are recorded, never answered (the module
//! wants no reply). Blocking, with a timeout on every read, so a module
//! that stops talking fails the test instead of hanging it.
//!
//! The driver half offers what a bus would do on its own: item signals,
//! registrations addressed to the module, and owner changes — plus the
//! scripted state (names, owners, properties, the other watcher).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::dbus::proto::{Kind, Message, Reader, Writer, frame_at};

/// Our assigned unique name: what the module's sender is.
pub const MODULE: &str = ":1.200";
/// The bus itself.
const BUS: &str = "org.freedesktop.DBus";
/// A read waits this long at most: failure, not patience.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

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

/// The scripted bus: serves on a thread, driven through the handle.
pub struct Fake {
    write: UnixStream,
    state: Arc<Mutex<State>>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    serial: Arc<Mutex<u32>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    /// A bus on a socketpair: the module's end and the driver.
    pub fn pair() -> (UnixStream, Fake) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let _id = NEXT.fetch_add(1, Ordering::Relaxed);
        let (module, bus) = UnixStream::pair().unwrap();
        module.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
        module.set_write_timeout(Some(READ_TIMEOUT)).unwrap();
        bus.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        bus.set_write_timeout(Some(READ_TIMEOUT)).unwrap();
        let state = Arc::new(Mutex::new(State {
            request_word: 1,
            ..State::default()
        }));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let serial = Arc::new(Mutex::new(1000u32));
        let write = bus.try_clone().unwrap();
        let thread = std::thread::spawn({
            let state = Arc::clone(&state);
            let calls = Arc::clone(&calls);
            let serial = Arc::clone(&serial);
            move || serve(bus, state, calls, serial)
        });
        (
            module,
            Fake {
                write,
                state,
                calls,
                serial,
                thread: Some(thread),
            },
        )
    }

    /// Scripts an item: listed, owned, and answering `GetAll` with
    /// `props` (the `a{sv}` body bytes; see `item_body`).
    pub fn add_item(&self, service: &str, owner: &str, props: Vec<u8>) {
        let mut state = self.state.lock().unwrap();
        if !state.names.contains(&service.to_owned()) {
            state.names.push(service.to_owned());
        }
        state.owners.insert(service.to_owned(), owner.to_owned());
        state.props.insert(service.to_owned(), props);
    }

    /// Scripts the other watcher owning the KDE name (host mode): the
    /// request word becomes `Exists`, and its item list is `ids`
    /// (`service/path` each).
    pub fn set_watcher(&self, owner: &str, ids: &[&str]) {
        let mut state = self.state.lock().unwrap();
        state.request_word = 3;
        state.watcher_owner = Some(owner.to_owned());
        state.watcher_items = ids.iter().map(|id| id.to_string()).collect();
        if !state.names.contains(&owner.to_owned()) {
            state.names.push(owner.to_owned());
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
        self.write.write_all(&message).unwrap();
        serial
    }

    /// Sends the other watcher's `StatusNotifierItemRegistered(service)`.
    pub fn send_watcher_registered(&mut self, service: &str) {
        let owner = self.state.lock().unwrap().watcher_owner.clone().unwrap_or_default();
        let mut body = Writer::new();
        body.str(service);
        let bytes = body.take_body().unwrap();
        self.send_signal(&owner, "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher", "StatusNotifierItemRegistered", "s", &bytes);
    }

    /// The item calls the module made, drained.
    pub fn calls(&self) -> Vec<RecordedCall> {
        core::mem::take(&mut *self.calls.lock().unwrap())
    }

    /// The host registrations the module made, drained.
    pub fn hosts(&self) -> Vec<String> {
        core::mem::take(&mut *self.state.lock().unwrap()).hosts
    }

    fn next_serial(&self) -> u32 {
        let mut serial = self.serial.lock().unwrap();
        *serial = serial.wrapping_add(1).max(1);
        *serial
    }

    fn send_signal(&mut self, sender: &str, path: &str, interface: &str, member: &str, sig: &str, body: &[u8]) {
        let serial = self.next_serial();
        let message = frame_with_sender(4, serial, path, interface, member, None, sender, sig, body);
        self.write.write_all(&message).unwrap();
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.write.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
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
        fields.pad(8);
        fields.u8(code);
        fields.signature(sig);
        write(&mut fields);
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

/// Serves one connection: SASL, then frames until the peer goes away.
fn serve(
    mut stream: UnixStream,
    state: Arc<Mutex<State>>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    serial: Arc<Mutex<u32>>,
) {
    if sasl(&mut stream).is_err() {
        return;
    }
    let mut staged = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match frame_at(&staged) {
            Ok(Some(len)) => {
                let frame: Vec<u8> = staged.drain(..len).collect();
                eprintln!("FAKEDBG {:?}: serving {len} bytes", std::time::Instant::now());
                if serve_frame(&mut stream, &frame, &state, &calls, &serial).is_err() {
                    eprintln!("FAKEDBG: serve failed on {}", frame.iter().map(|b| format!("{b:02x}")).collect::<String>());
                    return;
                }
            }
            Ok(None) => {}
            Err(()) => {
                eprintln!("FAKEDBG: bad frame, exiting");
                return;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                staged.extend_from_slice(&chunk[..n]);
                if staged.len() > 2 * 1024 * 1024 {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut => {
                continue;
            }
            Err(_) => return,
        }
    }
}

/// The SASL opening: the empty `EXTERNAL`, exactly as the spike traced
/// `busctl` sending it.
fn sasl(stream: &mut UnixStream) -> Result<(), ()> {
    let mut first = [0u8; 1];
    read_retry(stream, &mut first)?;
    if first != [0] {
        return Err(());
    }
    let auth = read_sasl_line(stream)?;
    if auth != "AUTH EXTERNAL" {
        return Err(());
    }
    stream.write_all(b"DATA\r\n").map_err(|_| ())?;
    let data = read_sasl_line(stream)?;
    if data != "DATA" && !data.starts_with("DATA ") {
        return Err(());
    }
    stream.write_all(b"OK 8d Expedition fake bus guid\r\n").map_err(|_| ())?;
    let begin = read_sasl_line(stream)?;
    if begin != "BEGIN" {
        return Err(());
    }
    Ok(())
}

fn read_sasl_line(stream: &mut UnixStream) -> Result<String, ()> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        read_retry(stream, &mut byte)?;
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

/// Reads exactly `buf.len()` bytes, retrying timeouts: the server
/// thread may not be scheduled for a while on a loaded box, and its
/// 2-second read timeout expires first. Bounded (about a minute), so a
/// peer that never speaks still fails the test instead of hanging it.
fn read_retry(stream: &mut UnixStream, mut buf: &mut [u8]) -> Result<(), ()> {
    for _ in 0..30 {
        match stream.read(buf) {
            Ok(0) => return Err(()),
            Ok(n) => {
                buf = &mut buf[n..];
                if buf.is_empty() {
                    return Ok(());
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return Err(()),
        }
    }
    Err(())
}

fn next_serial(serial: &Arc<Mutex<u32>>) -> u32 {
    let mut serial = serial.lock().unwrap();
    *serial = serial.wrapping_add(1).max(1);
    *serial
}

fn reply(stream: &mut UnixStream, to: u32, serial: u32, sig: &str, body: &[u8]) -> Result<(), ()> {
    let mut writer = Writer::new();
    writer.begin_return(serial, to, sig);
    writer.raw(body);
    let message = writer.finish().ok_or(())?;
    stream.write_all(&message).map_err(|_| ())
}

fn error(stream: &mut UnixStream, to: u32, serial: u32, name: &str) -> Result<(), ()> {
    let mut writer = Writer::new();
    writer.begin_error(serial, to, name, "");
    let message = writer.finish().ok_or(())?;
    stream.write_all(&message).map_err(|_| ())
}

/// Answers one frame. `Err` ends the session (the peer hung up or broke
/// the protocol); unknown calls are error replies, never silence. Every
/// frame past the set-up is recorded, so tests assert answers as well as
/// calls.
fn serve_frame(
    stream: &mut UnixStream,
    frame: &[u8],
    state: &Arc<Mutex<State>>,
    calls: &Arc<Mutex<Vec<RecordedCall>>>,
    serial: &Arc<Mutex<u32>>,
) -> Result<(), ()> {
    let message = match Message::parse(frame) {
        Ok(message) => message,
        Err(()) => {
            eprintln!("FAKEDBG: parse refused");
            return Err(());
        }
    };
    if message.kind != Kind::MethodCall {
        calls.lock().unwrap().push(record(&message));
        return Ok(());
    }
    let serial = next_serial(serial);
    let (member, destination, path) = (
        message.member.unwrap_or(""),
        message.destination.unwrap_or(""),
        message.path.unwrap_or(""),
    );
    let body = message.body.rest();
    if destination == BUS || destination.is_empty() {
        return serve_bus(stream, message.serial, serial, member, body, state);
    }
    if path == "/StatusNotifierWatcher" {
        // Calls at the module's own object come here only in host-mode
        // tests driving both ends; recorded like item calls.
        calls.lock().unwrap().push(record(&message));
        return Ok(());
    }
    // An item call: `GetAll`/`Get` are answered from the script; anything
    // else is recorded (activation wants no reply).
    let state_guard = state.lock().unwrap();
    if member == "GetAll" && state_guard.props.contains_key(destination) {
        let props = state_guard.props[destination].clone();
        drop(state_guard);
        return reply(stream, message.serial, serial, "a{sv}", &props);
    }
    if member == "Get" {
        // A single property, variant-wrapped: answer from the scripted
        // dictionary where it has one.
        let mut reader = Reader::le(body);
        let (Ok(interface), Ok(property)) = (reader.str(), reader.str()) else {
            return error(stream, message.serial, serial, "org.freedesktop.DBus.Error.InvalidArgs");
        };
        let _ = interface;
        if let Some(props) = state_guard.props.get(destination) {
            if let Some(variant) = find_prop(props, property) {
                drop(state_guard);
                return reply(stream, message.serial, serial, "v", &variant);
            }
        }
        drop(state_guard);
        return error(stream, message.serial, serial, "org.freedesktop.DBus.Error.UnknownMethod");
    }
    drop(state_guard);
    calls.lock().unwrap().push(record(&message));
    Ok(())
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

/// Serves the bus's own methods.
fn serve_bus(
    stream: &mut UnixStream,
    to: u32,
    serial: u32,
    member: &str,
    body: &[u8],
    state: &Arc<Mutex<State>>,
) -> Result<(), ()> {
    match member {
        "Hello" => {
            let mut out = Writer::new();
            out.str(MODULE);
            let bytes = out.take_body().ok_or(())?;
            reply(stream, to, serial, "s", &bytes)
        }
        "RequestName" => {
            let word = state.lock().unwrap().request_word;
            let mut out = Writer::new();
            out.u32(word);
            let bytes = out.take_body().ok_or(())?;
            reply(stream, to, serial, "u", &bytes)
        }
        "AddMatch" => Ok(()),
        "ListNames" => {
            let state = state.lock().unwrap();
            let mut out = Writer::new();
            let Some(cookie) = out.open_array(4) else {
                return error(stream, to, serial, "org.freedesktop.DBus.Error.Failed");
            };
            out.str(BUS);
            out.str(MODULE);
            for name in &state.names {
                out.str(name);
            }
            if let Some(owner) = &state.watcher_owner {
                out.str(owner);
            }
            out.close_array(cookie);
            let Some(bytes) = out.take_body() else {
                return error(stream, to, serial, "org.freedesktop.DBus.Error.Failed");
            };
            reply(stream, to, serial, "as", &bytes)
        }
        "GetNameOwner" => {
            let mut reader = Reader::le(body);
            let Ok(name) = reader.str() else {
                return error(stream, to, serial, "org.freedesktop.DBus.Error.InvalidArgs");
            };
            let state = state.lock().unwrap();
            let owner = if name == BUS || name == MODULE {
                Some(name.to_owned())
            } else if name == "org.kde.StatusNotifierWatcher" || name == "org.freedesktop.StatusNotifierWatcher" {
                state.watcher_owner.clone()
            } else {
                state.owners.get(name).cloned()
            };
            match owner {
                Some(owner) => {
                    drop(state);
                    let mut out = Writer::new();
                    out.str(&owner);
                    let bytes = out.take_body().ok_or(())?;
                    reply(stream, to, serial, "s", &bytes)
                }
                None => {
                    drop(state);
                    error(stream, to, serial, "org.freedesktop.DBus.Error.NameHasNoOwner")
                }
            }
        }
        _ => error(stream, to, serial, "org.freedesktop.DBus.Error.UnknownMethod"),
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
