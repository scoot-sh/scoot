//! The media module's scripted session bus: the daemon and every player
//! driven by the test, so a hostile player can say what a real daemon
//! would never route (a string that is not UTF-8, a signal forged from
//! another sender's name, a reply that never comes). The real daemon's
//! side of it is `daemon_tests.rs`. Test-only.
//!
//! Synchronous and deterministic, as the tray's fake is: one end of a
//! socketpair goes to the module, and the test drives this end with
//! [`Fake::pump`] between the module's own turns (the blocking `Hello` is
//! served by a short-lived thread). Every read waits ten seconds at most.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::dbus::mpris::PATH;
use crate::dbus::proto::{Kind, Message, Reader, Writer, frame_at};

/// The unique name the daemon gives the module (`serve_setup`'s).
pub const MODULE: &str = ":1.7";
const BUS: &str = "org.freedesktop.DBus";
const TIMEOUT: Duration = Duration::from_secs(10);

/// A frame the module sent that the fake did not answer: a control, or a
/// call to something it does not script.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub destination: String,
    pub path: String,
    pub interface: String,
    pub member: String,
    pub flags: u8,
}

/// What a player does with `GetAll`.
#[derive(Debug, Clone)]
pub enum Answer {
    /// The `a{sv}` body.
    Props(Vec<u8>),
    /// Never answers (the call is recorded, with its serial).
    Silent,
    /// An error reply.
    Error(&'static str),
    /// A reply with this signature and these bytes, however wrong.
    Raw(&'static str, Vec<u8>),
}

struct Script {
    name: String,
    owner: String,
    listed: bool,
    answer: Answer,
}

pub struct Fake {
    /// Kept to shut the connection down from the test's side.
    send: UnixStream,
    bus: UnixStream,
    staged: Vec<u8>,
    outbox: Vec<u8>,
    setup_thread: Option<std::thread::JoinHandle<()>>,
    players: Vec<Script>,
    /// The match rules the module added.
    pub matches: Vec<String>,
    /// A policy that refuses every `AddMatch`.
    pub refuse_matches: bool,
    /// `GetAll` calls per destination, answered or not.
    getalls: HashMap<String, usize>,
    /// Serials of `GetAll`s a silent player left unanswered, by owner.
    unanswered: HashMap<String, Vec<u32>>,
    calls: Vec<Recorded>,
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
            players: Vec::new(),
            matches: Vec::new(),
            refuse_matches: false,
            getalls: HashMap::new(),
            unanswered: HashMap::new(),
            calls: Vec::new(),
            serial: 1000,
        };
        (module, fake)
    }

    /// A player: listed, owned by `owner`, answering `GetAll` with `props`.
    pub fn add_player(&mut self, name: &str, owner: &str, props: Vec<u8>) {
        self.script(name, owner, true, Answer::Props(props));
    }

    /// A player that is owned but not in `ListNames` (it appears later, by
    /// `NameOwnerChanged`).
    pub fn add_unlisted(&mut self, name: &str, owner: &str, props: Vec<u8>) {
        self.script(name, owner, false, Answer::Props(props));
    }

    pub fn script(&mut self, name: &str, owner: &str, listed: bool, answer: Answer) {
        self.players.retain(|p| p.name != name);
        self.players.push(Script {
            name: name.to_owned(),
            owner: owner.to_owned(),
            listed,
            answer,
        });
    }

    /// What the player owned by `owner` answers `GetAll` with from now on.
    pub fn set_answer(&mut self, owner: &str, answer: Answer) {
        for player in self.players.iter_mut().filter(|p| p.owner == owner) {
            player.answer = answer.clone();
        }
    }

    /// How many `GetAll` calls reached `destination`.
    pub fn getalls(&self, destination: &str) -> usize {
        self.getalls.get(destination).copied().unwrap_or(0)
    }

    /// The serials of the `GetAll`s `owner` left unanswered.
    pub fn unanswered(&self, owner: &str) -> Vec<u32> {
        self.unanswered.get(owner).cloned().unwrap_or_default()
    }

    /// The frames the module sent that were not answered, drained.
    pub fn calls(&mut self) -> Vec<Recorded> {
        std::mem::take(&mut self.calls)
    }

    /// `NameOwnerChanged(name, old, new)` from the bus.
    pub fn name_owner_changed(&mut self, name: &str, old: &str, new: &str) {
        self.name_owner_changed_from(BUS, name, old, new);
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

    /// A `PropertiesChanged` from `sender` on the MPRIS object.
    pub fn properties_changed(&mut self, sender: &str, body: &[u8]) {
        self.signal(
            sender,
            PATH,
            "org.freedesktop.DBus.Properties",
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

    /// Queues bytes for the module as they are: what a hostile bus or peer
    /// would send.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.outbox.extend_from_slice(bytes);
    }

    /// Whether queued bytes still wait for the module to read them: a
    /// nonblocking socket takes what it takes, and the rest goes on the
    /// next [`Fake::pump`].
    pub fn pending(&self) -> bool {
        !self.outbox.is_empty()
    }

    /// A reply to `to` (a serial the module's call carried) from `sender`,
    /// whenever the test chooses: a late answer.
    pub fn reply_late(&mut self, to: u32, sig: &str, body: &[u8]) {
        let serial = self.next_serial();
        let mut writer = Writer::new();
        writer.begin_return(serial, to, sig);
        writer.raw(body);
        let message = writer.finish().unwrap();
        self.outbox.extend_from_slice(&message);
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
        let body = message.body.rest();
        if destination == BUS {
            return self.serve_bus(to, member, body);
        }
        if member == "GetAll" {
            *self.getalls.entry(destination.to_owned()).or_default() += 1;
            let answer = self
                .players
                .iter()
                .find(|p| p.owner == destination || p.name == destination)
                .map(|p| (p.owner.clone(), p.answer.clone()));
            return match answer {
                Some((_, Answer::Props(props))) => self.reply(to, "a{sv}", &props),
                Some((_, Answer::Error(name))) => self.error(to, name),
                Some((_, Answer::Raw(sig, bytes))) => self.reply(to, sig, &bytes),
                Some((owner, Answer::Silent)) => {
                    self.unanswered.entry(owner).or_default().push(to);
                    Ok(())
                }
                None => self.error(to, "org.freedesktop.DBus.Error.ServiceUnknown"),
            };
        }
        self.calls.push(Recorded {
            destination: destination.to_owned(),
            path: message.path.unwrap_or("").to_owned(),
            interface: message.interface.unwrap_or("").to_owned(),
            member: member.to_owned(),
            flags: frame[2],
        });
        Ok(())
    }

    fn serve_bus(&mut self, to: u32, member: &str, body: &[u8]) -> Result<(), ()> {
        match member {
            "AddMatch" => {
                let mut reader = Reader::le(body);
                if let Ok(rule) = reader.str() {
                    self.matches.push(rule.to_owned());
                }
                if self.refuse_matches {
                    self.error(to, "org.freedesktop.DBus.Error.MatchRuleInvalid")
                } else {
                    self.reply(to, "", &[])
                }
            }
            "ListNames" => {
                let mut out = Writer::new();
                let cookie = out.open_array(4).ok_or(())?;
                out.str(BUS);
                out.str(MODULE);
                for player in self.players.iter().filter(|p| p.listed) {
                    out.str(&player.name);
                }
                out.close_array(cookie);
                let bytes = out.take_body().ok_or(())?;
                self.reply(to, "as", &bytes)
            }
            "GetNameOwner" => {
                let mut reader = Reader::le(body);
                let Ok(name) = reader.str() else {
                    return self.error(to, "org.freedesktop.DBus.Error.InvalidArgs");
                };
                let owner = self
                    .players
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| p.owner.clone());
                match owner {
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
