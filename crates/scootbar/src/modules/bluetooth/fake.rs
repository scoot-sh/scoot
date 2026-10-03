//! The bluetooth module's scripted system bus: the daemon and BlueZ
//! driven by the test, so a hostile BlueZ can say what a real daemon
//! would never route (a signal forged from another sender's name, a reply
//! that never comes, an oversize answer). The real daemon's half is
//! `daemon_tests.rs`. Test-only.
//!
//! Synchronous and deterministic, as the media module's fake is: one end
//! of a socketpair goes to the module, and the test drives this end with
//! [`Fake::pump`] between the module's own turns (the blocking `Hello` is
//! served by a short-lived thread). Every read waits ten seconds at most.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::dbus::bluez::{MANAGER, NAME, PROPERTIES};
use crate::dbus::proto::{Kind, Message, Reader, Writer, frame_at};

/// The owner BlueZ answers from, unless the test says otherwise.
pub const BLUEZ: &str = ":1.bluez";
const BUS: &str = "org.freedesktop.DBus";
const TIMEOUT: Duration = Duration::from_secs(10);

/// A `Set(Adapter1.Powered, _)` the module sent: the adapter path and the
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetPowered {
    pub path: String,
    pub powered: bool,
}

/// What one object answers `GetAll` with, by interface.
#[derive(Debug, Clone)]
pub struct ObjectAnswer {
    pub path: &'static str,
    pub adapter: Option<Vec<u8>>,
    pub device: Option<Vec<u8>>,
    pub battery: Option<Vec<u8>>,
}

pub struct Fake {
    /// Kept to shut the connection down from the test's side.
    send: UnixStream,
    bus: UnixStream,
    staged: Vec<u8>,
    outbox: Vec<u8>,
    setup_thread: Option<std::thread::JoinHandle<()>>,
    /// The owner `GetNameOwner("org.bluez")` answers with; `None` errors
    /// it (BlueZ is not running).
    pub owner: Option<String>,
    /// What `GetManagedObjects` answers with.
    pub managed: Vec<u8>,
    pub objects: Vec<ObjectAnswer>,
    /// How many `GetManagedObjects` calls arrived.
    pub managed_calls: usize,
    /// The match rules the module added.
    pub matches: Vec<String>,
    /// `Set` calls the module sent, drained by the test.
    pub sets: Vec<SetPowered>,
    serial: u32,
}

impl Fake {
    /// The module's end of the socketpair, and the fake serving the other.
    pub fn pair() -> (UnixStream, Fake) {
        let (module, bus) = UnixStream::pair().unwrap();
        module.set_read_timeout(Some(TIMEOUT)).unwrap();
        module.set_write_timeout(Some(TIMEOUT)).unwrap();
        bus.set_write_timeout(Some(TIMEOUT)).unwrap();
        let send = bus.try_clone().unwrap();
        let mut serving = bus.try_clone().unwrap();
        let thread = std::thread::spawn(move || {
            crate::dbus::testdaemon::serve_setup(&mut serving);
        });
        let fake = Fake {
            send,
            bus,
            staged: Vec::new(),
            outbox: Vec::new(),
            setup_thread: Some(thread),
            owner: Some(BLUEZ.to_owned()),
            managed: Vec::new(),
            objects: Vec::new(),
            managed_calls: 0,
            matches: Vec::new(),
            sets: Vec::new(),
            serial: 1000,
        };
        (module, fake)
    }

    /// The `Set` calls the module sent, drained.
    pub fn take_sets(&mut self) -> Vec<SetPowered> {
        std::mem::take(&mut self.sets)
    }

    /// `NameOwnerChanged(org.bluez, old, new)` from the bus.
    pub fn name_owner_changed(&mut self, old: &str, new: &str) {
        self.name_owner_changed_from(BUS, NAME, old, new);
    }

    /// The same from `sender`: forged when it is not the bus.
    pub fn name_owner_changed_from(&mut self, sender: &str, name: &str, old: &str, new: &str) {
        let mut body = Writer::new();
        body.str(name);
        body.str(old);
        body.str(new);
        let body = body.take_body().unwrap();
        self.signal(
            sender,
            "/org/freedesktop/DBus",
            BUS,
            "NameOwnerChanged",
            "sss",
            &body,
        );
    }

    /// An `InterfacesAdded` from `sender` (the manager at `/org/bluez`).
    pub fn interfaces_added(&mut self, sender: &str, body: &[u8]) {
        self.signal(
            sender,
            "/org/bluez",
            MANAGER,
            "InterfacesAdded",
            "oa{sa{sv}}",
            body,
        );
    }

    /// An `InterfacesRemoved` from `sender` (the manager at `/org/bluez`).
    pub fn interfaces_removed(&mut self, sender: &str, body: &[u8]) {
        self.signal(
            sender,
            "/org/bluez",
            MANAGER,
            "InterfacesRemoved",
            "oas",
            body,
        );
    }

    /// A `PropertiesChanged` from `sender` on `path`.
    pub fn properties_changed(&mut self, sender: &str, path: &str, body: &[u8]) {
        self.signal(
            sender,
            path,
            PROPERTIES,
            "PropertiesChanged",
            "sa{sv}as",
            body,
        );
    }

    /// Any signal from `sender`: what a peer may send, to the bar if it
    /// matches.
    pub fn signal(
        &mut self,
        sender: &str,
        path: &str,
        interface: &str,
        member: &str,
        sig: &str,
        body: &[u8],
    ) {
        let serial = self.next_serial();
        let message = frame_with_sender(4, serial, path, interface, member, sender, sig, body);
        self.outbox.extend_from_slice(&message);
    }

    /// Whether queued bytes still wait for the module to read them.
    pub fn pending(&self) -> bool {
        !self.outbox.is_empty()
    }

    /// The bus going away: the module's connection reads end of file.
    pub fn hang_up(&mut self) {
        let _ = self.send.shutdown(std::net::Shutdown::Both);
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    /// Serves every complete frame waiting; never blocks. Joins the
    /// set-up thread first. Returns frames served.
    pub fn pump(&mut self) -> usize {
        if let Some(thread) = self.setup_thread.take() {
            let _ = thread.join();
            self.bus.set_read_timeout(None).unwrap();
            self.bus.set_write_timeout(None).unwrap();
            rustix::fs::fcntl_setfl(&self.bus, rustix::fs::OFlags::NONBLOCK).unwrap();
        }
        self.flush();
        let mut served = 0;
        loop {
            let mut chunk = [0u8; 8192];
            match std::io::Read::read(&mut self.bus, &mut chunk) {
                Ok(0) => return served,
                Ok(n) => self.staged.extend_from_slice(&chunk[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.flush();
                    return served;
                }
                Err(_) => return served,
            }
            while let Ok(Some(len)) = frame_at(&self.staged) {
                let frame: Vec<u8> = self.staged.drain(..len).collect();
                served += 1;
                if self.serve(&frame).is_err() {
                    return served;
                }
            }
        }
    }

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

    fn reply(&mut self, to: u32, sig: &str, body: &[u8]) -> Result<(), ()> {
        let serial = self.next_serial();
        // Capped past what the module reads, so a reply it must skip can
        // be built.
        let mut writer = Writer::with_cap(8 << 20);
        writer.begin_return(serial, to, sig);
        writer.raw(body);
        let message = writer.finish().ok_or(())?;
        self.outbox.extend_from_slice(&message);
        Ok(())
    }

    fn error(&mut self, to: u32, name: &str) -> Result<(), ()> {
        let serial = self.next_serial();
        let mut writer = Writer::new();
        writer.begin_error(serial, to, name, "");
        let message = writer.finish().ok_or(())?;
        self.outbox.extend_from_slice(&message);
        Ok(())
    }

    fn serve(&mut self, frame: &[u8]) -> Result<(), ()> {
        let message = Message::parse(frame).map_err(|_| ())?;
        if message.kind != Kind::MethodCall {
            return Ok(());
        }
        let (member, destination) = (
            message.member.unwrap_or(""),
            message.destination.unwrap_or(""),
        );
        let to = message.serial;
        let path = message.path.unwrap_or("");
        let interface = message.interface.unwrap_or("");
        let body = message.body.rest().to_vec();
        if destination == BUS {
            return self.serve_bus(to, member, &body);
        }
        if destination == NAME && interface == MANAGER && member == "GetManagedObjects" {
            self.managed_calls += 1;
            let managed = self.managed.clone();
            return self.reply(to, "a{oa{sa{sv}}}", &managed);
        }
        if destination == NAME && interface == PROPERTIES && member == "GetAll" {
            let mut reader = Reader::le(&body);
            let Ok(iface) = reader.str() else {
                return self.error(to, "org.freedesktop.DBus.Error.InvalidArgs");
            };
            let answer = self
                .objects
                .iter()
                .find(|o| o.path == path)
                .and_then(|o| match iface {
                    "org.bluez.Adapter1" => o.adapter.clone(),
                    "org.bluez.Device1" => o.device.clone(),
                    "org.bluez.Battery1" => o.battery.clone(),
                    _ => None,
                });
            return match answer {
                Some(props) => self.reply(to, "a{sv}", &props),
                None => self.error(to, "org.freedesktop.DBus.Error.UnknownMethod"),
            };
        }
        if destination == NAME && interface == PROPERTIES && member == "Set" {
            let mut reader = Reader::le(&body);
            let (Ok(iface), Ok(prop)) = (reader.str(), reader.str()) else {
                return Ok(());
            };
            // `Set` wants no reply: record it, answer nothing.
            if iface == "org.bluez.Adapter1"
                && prop == "Powered"
                && let Ok(value) = reader.variant(|sig, reader| {
                    if sig == "b" {
                        reader.boolean()
                    } else {
                        Err(())
                    }
                })
            {
                self.sets.push(SetPowered {
                    path: path.to_owned(),
                    powered: value,
                });
            }
            return Ok(());
        }
        // Anything else (a `Set` with no reply expected takes no slot and
        // is answered never): nothing.
        Ok(())
    }

    fn serve_bus(&mut self, to: u32, member: &str, body: &[u8]) -> Result<(), ()> {
        match member {
            "AddMatch" => {
                let mut reader = Reader::le(body);
                if let Ok(rule) = reader.str() {
                    self.matches.push(rule.to_owned());
                }
                self.reply(to, "", &[])
            }
            "GetNameOwner" => {
                let mut reader = Reader::le(body);
                let Ok(name) = reader.str() else {
                    return self.error(to, "org.freedesktop.DBus.Error.InvalidArgs");
                };
                match (name == NAME).then(|| self.owner.clone()).flatten() {
                    Some(owner) => {
                        let mut out = Writer::new();
                        out.str(&owner);
                        let bytes = out.take_body().ok_or(())?;
                        self.reply(to, "s", &bytes)
                    }
                    None => self.error(to, "org.freedesktop.DBus.Error.NameHasNoOwner"),
                }
            }
            _ => self.error(to, "org.freedesktop.DBus.Error.UnknownMethod"),
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

/// A frame with the sender header set, as the daemon does for everything
/// it routes: built by hand (fixed part plus the fields array).
#[allow(clippy::too_many_arguments)]
fn frame_with_sender(
    kind: u8,
    serial: u32,
    path: &str,
    interface: &str,
    member: &str,
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
    entry(7, "s", &|w| w.str(sender));
    entry(8, "g", &|w| w.signature(signature));
    let fields = fields.take_body().unwrap_or_default();
    let mut out = vec![b'l', kind, 0, 1];
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
