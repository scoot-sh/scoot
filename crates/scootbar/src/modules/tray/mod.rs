//! The tray module: the StatusNotifierItem watcher and host, over the
//! shared D-Bus client.
//!
//! The watcher is core infrastructure of the bar, not just a module: the
//! bar owns the `org.kde.StatusNotifierWatcher` name (and its
//! `org.freedesktop` twin), re-acquires it if it is lost, answers item
//! registrations itself, and picks up items that registered before the
//! bar started (the bus is enumerated at connect). Where another process
//! already owns the name, the bar runs as a host against it instead —
//! items still appear, clicks still work — and takes over when the owner
//! leaves. Item menus (`ContextMenu`, the DBusMenu protocol) wait on the
//! [popups entry](../../../../docs/scootbar/backlog/popups.md): the
//! `menu` action is refused naming that, loudly, not silently.
//!
//! ## States
//!
//! `Waiting` owns an inotify fd on the bus socket's directory and shows
//! nothing: no bus, no items, one watch. `Live` owns the bus socket and
//! shows what registered, in `service + path` order (the KDE watcher's
//! id format). A dead connection drops back to waiting with nothing
//! shown; a bus that never exists costs one watch fd at most, and none
//! where even the watch cannot be armed (a reload starts over, as the
//! volume module's does).
//!
//! ## Events, all asynchronous past connect
//!
//! The set-up (auth, `Hello`, `RequestName`, match rules, enumeration)
//! is a handful of blocking round trips on a local socket, like the
//! volume module's handshake. Past that nothing blocks: `GetAll` answers
//! arrive as pending-call replies on a later turn, `NewIcon` and friends
//! re-read the item then, and `NameOwnerChanged` (tracked centrally, one
//! map for items and watcher names alike) drops vanished items at once —
//! an app that crashes without unregistering disappears with its owner.
//!
//! ## Untrusted bytes, bounded icons
//!
//! Item strings are sanitized once, at parse time (controls stripped,
//! cut like view text). Pixmaps arrive as raw `ARGB32` over the bus:
//! entries past 64 pixels a side are left for the icon cache to scale
//! (at most one kept), at most eight entries an item, each converted to
//! premultiplied once per icon version and drawn from the shared icon
//! cache at the output's real size — icons at exact device pixels,
//! without a per-frame allocation. A malformed or hostile item loses
//! itself (its reply is dropped, its entry skipped), never the bar.

use std::fmt::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustix::event::PollFlags;
use rustix::fs::inotify::{CreateFlags, WatchFlags};

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::proto::{self, Pixmap, Reader, Writer, check_name, check_path, read_pixmaps, request_reply};
use crate::icon::tray::TrayIcon;
use crate::icon::Art;
use crate::text::Text;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod fake;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "tray";

/// The actions a binding or an agent may name. Every one takes the item
/// index (`activate 0`); a click, middle click or scroll supplies its own.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "activate",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "secondary",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "scroll-up",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "scroll-down",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu",
        arg: ArgKind::Required,
    },
];

/// The watcher's names, object and interfaces: the KDE one apps speak,
/// and the freedesktop twin the spec names. Both are owned when free;
/// the KDE one decides the mode.
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_KDE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_FDO: &str = "org.freedesktop.StatusNotifierWatcher";
/// The item interfaces, KDE first (what items send), then the twin.
const ITEM_KDE: &str = "org.kde.StatusNotifierItem";
const ITEM_FDO: &str = "org.freedesktop.StatusNotifierItem";
/// The item properties interface.
const ITEM_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The default item path, for a registration naming a service only.
const ITEM_DEFAULT_PATH: &str = "/StatusNotifierItem";
/// What the watcher reports as its protocol version (KDE answers 0).
const PROTOCOL_VERSION: u32 = 0;

/// The most items shown: past this a registration is answered and then
/// ignored, said once per newcomer on stderr, never polled again.
pub const MAX_ITEMS: usize = 32;
/// The longest side of a stored pixmap entry, in pixels: entries past it
/// are left for the icon cache to scale from the single smallest kept.
const MAX_STORED_SIDE: u32 = 64;
/// The most pixmap entries converted per item: an animation past this
/// still shows its first entries.
const MAX_STORED_ICONS: usize = 8;
/// Item strings kept this long at most, in bytes: titles and tooltip
/// text are cut like view text, once, at parse time.
const MAX_ITEM_TEXT: usize = 128;
/// A scroll's delta past this magnitude is clamped to it: a touchpad
/// flood is one bounded call, never an accumulated one.
const MAX_SCROLL_DELTA: u32 = 64;

/// The module's options: none of its own yet, so the table holds the
/// margin and the interaction keys like every module's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {}

#[cfg(feature = "tray")]
pub fn init(settings: &super::Settings) -> Init {
    let _ = settings;
    Init::Available(start_with(BusAddr::Path(conn::bus_path())))
}

/// Tests only: the module started as if the probe had found a bus — a
/// scripted one on a socketpair, so the contract test drives the connected
/// path on a machine without any bus. The fake's handle is leaked on
/// purpose (a thread and a socketpair per stand-in): the signature
/// cannot hand a guard back, and the contract calls it once.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    let _ = settings;
    let (stream, fake) = fake::Fake::pair();
    std::mem::forget(fake);
    start_connected(stream)
}

/// How to reach the bus: the filesystem path, or an already-open stream
/// (the tests' socketpair end).
#[derive(Debug)]
enum BusAddr {
    Path(PathBuf),
    #[cfg(test)]
    Stream(std::os::unix::net::UnixStream),
}

/// Starts the module: connects now when the bus is there, or waits on it
/// when it is not. Always available: a bus may appear at any time, and
/// waiting costs one inotify fd at most.
fn start_with(addr: BusAddr) -> Box<dyn Module> {
    let (path, stream) = match addr {
        BusAddr::Path(path) => (path, None),
        #[cfg(test)]
        BusAddr::Stream(stream) => (conn::bus_path(), Some(stream)),
    };
    let mut tray = Tray {
        path,
        bus: Bus::Waiting {
            notify: None,
            dir: PathBuf::new(),
        },
    };
    match stream {
        Some(stream) => tray.connected(stream),
        None => tray.wait(),
    }
    Box::new(tray)
}

/// Starts the module on an already-open stream: the fake bus's end.
#[cfg(test)]
fn start_connected(stream: std::os::unix::net::UnixStream) -> Box<dyn Module> {
    start_with(BusAddr::Stream(stream))
}

/// The module: the bus path, and the wait or the connection.
struct Tray {
    path: PathBuf,
    bus: Bus,
}

enum Bus {
    Waiting { notify: Option<Notify>, dir: PathBuf },
    Live(Box<Live>),
}

/// The inotify watch on the bus socket's directory, while waiting.
struct Notify {
    fd: OwnedFd,
}

/// A live bus: the connection, the mode, the items in id order, and the
/// calls in flight.
struct Live {
    conn: Conn,
    mode: Mode,
    /// The KDE watcher name's owner while in host mode: who item lists
    /// are read from.
    watcher_owner: Option<String>,
    items: Vec<Item>,
    flights: Vec<Option<Flight>>,
    /// Hosts registered with us (owner mode): our own host is first.
    hosts: Vec<String>,
    /// Whether the `too many items` warning was said for the current set.
    said_full: bool,
}

/// Whether we own the watcher name (answering registrations) or talk to
/// whoever does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Owner,
    Host,
}

/// A call in flight: what the reply correlates to.
struct Flight {
    op: Op,
}

enum Op {
    /// `GetAll` for the item with this id.
    Props(String),
    /// `GetNameOwner` for the item with this id (verifying a newcomer).
    Owner(String),
    /// `ListNames`: pick up what registered before us.
    Names,
    /// The existing watcher's `RegisteredStatusNotifierItems` (host
    /// mode): diff it against what is shown.
    WatcherItems,
    /// The KDE watcher name's answer.
    RequestKde,
    /// The freedesktop twin's answer (best effort, ignored past owning).
    RequestFdo,
    /// The KDE watcher name's owner (host mode): read its list next.
    WatcherOwner,
}

/// One item: who it is, what it shows, and its converted icons.
struct Item {
    /// `service + path`, the KDE watcher's id format and the sort key.
    id: String,
    /// The bus name calls go to.
    service: String,
    /// The object path calls go to.
    path: String,
    /// The service's unique name: signal matching and vanish tracking.
    /// `None` until the first `GetNameOwner` answers.
    owner: Option<String>,
    status: Status,
    title: String,
    tooltip_title: String,
    tooltip_text: String,
    menu: String,
    item_is_menu: bool,
    /// The converted icons, smallest first, at most [`MAX_STORED_ICONS`].
    /// Each id hashes its pixels (see `convert`), so an unchanged icon
    /// compares equal and the icon cache hits every steady frame.
    icons: Vec<Arc<TrayIcon>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Passive,
    Active,
    NeedsAttention,
}

impl Status {
    fn parse(text: &str) -> Self {
        match text {
            "Active" => Self::Active,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Passive,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Passive => "Passive",
            Self::Active => "Active",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

/// Item strings sanitized once, at parse time: controls stripped (the
/// contract test's rule), cut at [`MAX_ITEM_TEXT`] bytes on a character
/// boundary.
fn clean(text: &str) -> String {
    let stripped: String = text.chars().filter(|c| !c.is_control()).collect();
    if stripped.len() <= MAX_ITEM_TEXT {
        return stripped;
    }
    let mut cut = MAX_ITEM_TEXT;
    while !stripped.is_char_boundary(cut) {
        cut -= 1;
    }
    stripped[..cut].to_owned()
}

/// Splits a register argument the KDE way: a leading `/` is a path on the
/// sender's service, else a service with the default path. Both halves
/// validated; `Err(())` refuses the registration, never the bar.
fn split_service_path(sender: &str, service_or_path: &str) -> Result<(String, String), ()> {
    if let Some(path) = service_or_path.strip_prefix('/') {
        let path = format!("/{path}");
        check_path(&path)?;
        check_name(sender)?;
        Ok((sender.to_owned(), path))
    } else {
        check_name(service_or_path)?;
        Ok((service_or_path.to_owned(), ITEM_DEFAULT_PATH.to_owned()))
    }
}

/// Splits a KDE item id (`service + path`) back into its halves: the
/// first `/` starts the path (a service never holds one).
fn split_id(id: &str) -> Option<(&str, &str)> {
    let slash = id.find('/')?;
    Some((&id[..slash], &id[slash..]))
}

impl Tray {
    /// Waits on the bus socket's directory: the socket's own directory
    /// when there is one, else the runtime directory itself. A watch that
    /// cannot be armed is nothing polled: the socket is re-probed on
    /// every wake that does arrive, and a reload starts over (the volume
    /// module's rule).
    fn wait(&mut self) {
        let dir = self
            .path
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(runtime_dir);
        let notify = rustix::fs::inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .ok()
            .and_then(|fd| {
                rustix::fs::inotify::add_watch(
                    &fd,
                    &dir,
                    WatchFlags::CREATE
                        | WatchFlags::MOVED_TO
                        | WatchFlags::DELETE_SELF
                        | WatchFlags::MOVE_SELF,
                )
                .ok()
                .map(|_| Notify { fd })
            });
        self.bus = Bus::Waiting { notify, dir };
    }

    /// Drops the connection and waits again, with nothing shown: a dead
    /// bus's last icons are not icons.
    fn drop_live(&mut self) -> Update {
        let had = matches!(self.bus, Bus::Live(_));
        self.wait();
        if had { Update::Changed } else { Update::Unchanged }
    }

    /// Connects a stream that is already open (the tests' socketpair end,
    /// or a dialled bus): `Hello` is the only blocking call, and the
    /// names, matches and enumeration follow ready-driven.
    fn connected(&mut self, stream: std::os::unix::net::UnixStream) {
        match conn::setup(stream) {
            Ok(conn) => {
                self.bus = Bus::Live(Box::new(setup(conn)));
            }
            Err(_) => self.wait(),
        }
    }

    /// Dials the bus and starts the set-up; failures wait. Nothing is
    /// shown until the names answer.
    fn connect(&mut self) -> Update {
        match conn::connect(&self.path) {
            Ok(conn) => {
                self.bus = Bus::Live(Box::new(setup(conn)));
                Update::Unchanged
            }
            Err(_) => Update::Unchanged,
        }
    }

    /// Handles the directory watch: drains it (else it stays ready) and
    /// probes the socket when its name arrived. Events for other names
    /// cost one scan of the read bytes, never a connect.
    fn on_notify(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            self.wait();
            return Update::Unchanged;
        }
        let mut bus = false;
        let Bus::Waiting { notify, dir } = &mut self.bus else {
            return Update::Unchanged;
        };
        if let Some(notify) = notify.as_ref() {
            let fd = notify.fd.as_fd();
            let mut buf = [0u8; 4096];
            loop {
                match rustix::io::read(fd, &mut buf) {
                    Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
                    Ok(n) => {
                        if scan_names(&buf[..n], bus_name(&self.path)) {
                            bus = true;
                        }
                    }
                    Err(_) => {
                        self.wait();
                        return Update::Unchanged;
                    }
                }
            }
        }
        let _ = dir;
        if bus {
            self.connect()
        } else {
            Update::Unchanged
        }
    }

    /// One turn on the bus: pumps the connection and works each event,
    /// re-pumping while capped so a burst drains in its turn.
    fn on_bus(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return self.drop_live();
        }
        let Bus::Live(live) = &mut self.bus else {
            return Update::Unchanged;
        };
        if live.conn.dead() {
            return self.drop_live();
        }
        let mut changed = Update::Unchanged;
        loop {
            let (events, capped) = live.conn.pump();
            for event in events {
                if live.apply(event) == Update::Changed {
                    changed = Update::Changed;
                }
                if live.conn.dead() {
                    return self.drop_live();
                }
            }
            if !capped {
                break;
            }
        }
        changed
    }

    /// The item index `x` (device pixels from the span's left, `padding`
    /// inside) lands on at `em`, or `None` in a gap or past the end.
    fn hit(x: u32, padding: u32, em: f32, count: usize) -> Option<usize> {
        let side = Text::art_side(em) as usize;
        let gap = gap(side);
        let at = (x as usize).saturating_sub(padding as usize);
        let stride = side + gap;
        if stride == 0 {
            return None;
        }
        let index = at / stride;
        if index < count && at % stride < side {
            Some(index)
        } else {
            None
        }
    }
}

/// The gap between neighbouring icons: an eighth of the slot, at least
/// one device pixel.
fn gap(side: usize) -> usize {
    (side / 8).max(1)
}

/// The bus socket's file name, for the watch scan.
fn bus_name(path: &Path) -> &[u8] {
    path.file_name().map(|name| name.as_encoded_bytes()).unwrap_or(b"bus")
}

/// The runtime directory, or its conventional fallback.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()))
}

/// Whether an inotify buffer holds an event for `want`: whole-field
/// matching on the file name, so a longer name cannot false-positive (the
/// volume module's scan).
fn scan_names(buf: &[u8], want: &[u8]) -> bool {
    let mut rest = buf;
    if rest.is_empty() {
        return false;
    }
    loop {
        if rest.len() < 16 {
            return true;
        }
        let len = u32::from_ne_bytes([rest[12], rest[13], rest[14], rest[15]]) as usize;
        rest = &rest[16..];
        if rest.len() < len {
            return true;
        }
        let (name, tail) = rest.split_at(len);
        // The name is NUL-terminated in the event: compare without it.
        let name = name.split(|byte| *byte == 0).next().unwrap_or(&[]);
        if name == want {
            return true;
        }
        rest = tail;
    }
}

impl Live {
    /// Works one bus event; says whether the view moved.
    fn apply(&mut self, event: Event) -> Update {
        match event {
            Event::Reply { token, signature, body } => self.on_reply(token, &signature, &body),
            Event::CallError { token, name } => self.on_error(token, &name),
            Event::Signal { sender, path, interface, member, signature, body } => {
                self.on_signal(&sender, &path, &interface, &member, &signature, &body)
            }
            Event::MethodCall { sender, path, interface, member, serial, signature, body } => {
                self.on_call(&sender, &path, &interface, &member, serial, &signature, &body)
            }
        }
    }

    /// Queues a call, tracking its flight. A full table drops the call:
    /// the next signal re-queues it (every refresh is signal-driven, so
    /// nothing is lost but a turn). Calls that want no reply take no
    /// flight. Eight arguments: a call names its destination, object,
    /// interface, member, body and reply wish, like the header it becomes.
    #[allow(clippy::too_many_arguments)]
    fn issue(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
        flags: u8,
        op: Op,
    ) {
        let wants_reply = flags & proto::flag::NO_REPLY_EXPECTED == 0;
        let slot = if wants_reply {
            match self.flights.iter().position(|flight| flight.is_none()) {
                Some(slot) => slot,
                None => {
                    if self.flights.len() >= conn::MAX_PENDING {
                        return;
                    }
                    self.flights.push(None);
                    self.flights.len() - 1
                }
            }
        } else {
            usize::MAX
        };
        let token = slot as u64;
        if self
            .conn
            .call(destination, path, interface, member, body_sig, body, flags, token)
            .is_ok()
            && wants_reply
        {
            self.flights[slot] = Some(Flight { op });
        }
    }

    /// Fires a call that wants no reply (activation): queued, never
    /// tracked, never reported. Eight arguments, as [`Live::issue`].
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
    ) {
        let _ = self.conn.call(
            destination,
            path,
            interface,
            member,
            body_sig,
            body,
            proto::flag::NO_REPLY_EXPECTED,
            0,
        );
    }

    /// Refreshes the item's properties: one `GetAll`, answered later.
    fn refresh(&mut self, id: &str) {
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return;
        };
        let (service, path) = (item.service.clone(), item.path.clone());
        self.issue(&service, &path, ITEM_PROPERTIES, "GetAll", "s", &get_all_body(), 0, Op::Props(id.to_owned()));
    }

    fn on_reply(&mut self, token: u64, signature: &str, body: &[u8]) -> Update {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return Update::Unchanged;
        };
        match flight.op {
            Op::Props(id) => self.on_props(&id, signature, body),
            Op::Owner(id) => self.on_owner(&id, signature, body),
            Op::Names => self.on_names(signature, body),
            Op::WatcherItems => self.on_watcher_items(signature, body),
            Op::RequestKde => self.on_request_reply(true, signature, body),
            Op::RequestFdo => self.on_request_reply(false, signature, body),
            Op::WatcherOwner => self.on_watcher_owner_reply(signature, body),
        }
    }

    fn on_error(&mut self, token: u64, name: &str) -> Update {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return Update::Unchanged;
        };
        match flight.op {
            // A `GetAll` that errors is an item gone (or never there): a
            // bogus registration loses itself, silently.
            Op::Props(id) => self.remove(&id),
            // `NameHasNoOwner` is an item already gone; anything else
            // leaves the owner unknown (signals still match by sender).
            Op::Owner(id) if name.ends_with("NameHasNoOwner") => self.remove(&id),
            Op::Owner(_) | Op::Names | Op::WatcherItems | Op::RequestKde | Op::RequestFdo | Op::WatcherOwner => {
                Update::Unchanged
            }
        }
    }

    /// Applies a `GetAll` answer to the item, through [`fill`], and
    /// reports whether anything shown moved. A misshapen answer is
    /// dropped (the item keeps its last state).
    fn on_props(&mut self, id: &str, signature: &str, body: &[u8]) -> Update {
        let index = match self.items.iter().position(|item| item.id == id) {
            Some(index) => index,
            None => return Update::Unchanged,
        };
        if signature != "a{sv}" {
            return Update::Unchanged;
        }
        let before = Fingerprint::of(&self.items[index]);
        if !fill(&mut self.items[index], body) {
            return Update::Unchanged;
        }
        if Fingerprint::of(&self.items[index]) != before {
            Update::Changed
        } else {
            Update::Unchanged
        }
    }

        fn on_owner(&mut self, id: &str, signature: &str, body: &[u8]) -> Update {
        let Ok(owner) = proto::read_owner(signature, body) else {
            return Update::Unchanged;
        };
        let known = self.items.iter().any(|item| item.id == id);
        if !known {
            return Update::Unchanged;
        }
        self.set_owner(id, owner);
        // The verification the registration waited on: now read it.
        self.refresh(id);
        Update::Unchanged
    }

    /// Works a `ListNames` answer: registers what matches the item
    /// prefixes and is not shown (cap [`MAX_ITEMS`]), verifying each
    /// newcomer with a `GetNameOwner` first.
    fn on_names(&mut self, signature: &str, body: &[u8]) -> Update {
        let Ok(names) = proto::read_names(signature, body) else {
            return Update::Unchanged;
        };
        let mut added = false;
        for name in names {
            if !is_item_name(&name) {
                continue;
            }
            let id = format!("{name}{ITEM_DEFAULT_PATH}");
            if self.items.iter().any(|item| item.id == id) {
                continue;
            }
            if self.items.len() >= MAX_ITEMS {
                self.say_full(&id);
                continue;
            }
            self.items.push(Item::new(id.clone(), name.clone(), ITEM_DEFAULT_PATH.to_owned()));
            self.sort();
            added = true;
            if name.starts_with(':') {
                self.set_owner(&id, name);
                self.refresh(&id);
            } else {
                let mut get = Writer::new();
                get.str(&name);
                let Some(bytes) = get.take_body() else { continue };
                self.issue(
                    conn::BUS_NAME,
                    conn::BUS_PATH,
                    conn::BUS_INTERFACE,
                    "GetNameOwner",
                    "s",
                    &bytes,
                    0,
                    Op::Owner(id),
                );
            }
        }
        // Newcomers arrive empty and fill in from their `GetAll`: the
        // view moves when one was added, and again when each answers.
        if added { Update::Changed } else { Update::Unchanged }
    }

    /// Works the existing watcher's item list (host mode): adds what is
    /// new, drops what left.
    fn on_watcher_items(&mut self, _signature: &str, body: &[u8]) -> Update {
        // A `Get` of `RegisteredStatusNotifierItems` answers a variant
        // holding `as`.
        let mut reader = Reader::le(body);
        let Ok(value) = reader.variant_raw() else {
            return Update::Unchanged;
        };
        if value.signature() != "as" {
            return Update::Unchanged;
        }
        let mut list = value.read();
        let Ok(raw) = list.array_raw(4) else {
            return Update::Unchanged;
        };
        let mut ids = Reader::le(raw);
        let mut seen = Vec::new();
        while !ids.exhausted() {
            let Ok(entry) = ids.str() else { break };
            let Some((service, path)) = entry.split_once('/') else {
                continue;
            };
            let path = format!("/{path}");
            if check_name(service).is_err() || check_path(&path).is_err() {
                continue;
            }
            seen.push(format!("{service}{path}"));
        }
        let mut moved = false;
        for id in &seen {
            if self.items.iter().any(|item| &item.id == id) {
                continue;
            }
            if self.items.len() >= MAX_ITEMS {
                self.say_full(id);
                continue;
            }
            let (service, path) = split_id(id).unwrap_or(("", ""));
            self.items.push(Item::new(id.clone(), service.to_owned(), path.to_owned()));
            self.sort();
            moved = true;
            let id = id.clone();
            self.refresh(&id);
        }
        let before = self.items.len();
        self.items.retain(|item| seen.contains(&item.id));
        if self.items.len() != before {
            moved = true;
        }
        if moved { Update::Changed } else { Update::Unchanged }
    }

    fn on_signal(
        &mut self,
        sender: &str,
        _path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Update {
        if interface == conn::BUS_INTERFACE && member == "NameOwnerChanged" {
            return self.on_name_owner_changed(signature, body);
        }
        if interface == WATCHER_KDE || interface == WATCHER_FDO {
            return self.on_watcher_signal(sender, member, signature, body);
        }
        if interface == ITEM_KDE || interface == ITEM_FDO {
            return self.on_item_signal(sender, member);
        }
        Update::Unchanged
    }

    /// Central owner tracking: items vanish with their owner, the watcher
    /// name switches the mode, and our own echoes are ignored.
    fn on_name_owner_changed(&mut self, signature: &str, body: &[u8]) -> Update {
        if signature != "sss" {
            return Update::Unchanged;
        }
        let Ok((name, _old, new)) = proto::read_name_owner_changed(body) else {
            return Update::Unchanged;
        };
        if name == WATCHER_KDE {
            return self.on_watcher_owner(new.as_deref());
        }
        if name == WATCHER_FDO {
            return Update::Unchanged;
        }
        // An item's service or owner changed hands: gone means dropped
        // (a crash without unregistering), new means re-read.
        let mut gone = false;
        let mut refresh = Vec::new();
        for item in &self.items {
            if item.service == name || item.owner.as_deref() == Some(name.as_str()) {
                match new.as_deref() {
                    None => gone = true,
                    Some(owner) => {
                        if item.owner.as_deref() != Some(owner) {
                            refresh.push(item.id.clone());
                        }
                    }
                }
            }
        }
        if gone {
            self.items.retain(|item| item.service != name && item.owner.as_deref() != Some(name.as_str()));
            if self.mode == Mode::Owner {
                self.emit_unregistered(&name);
            }
            return Update::Changed;
        }
        for id in refresh {
            self.refresh(&id);
        }
        Update::Unchanged
    }

    /// The KDE name's owner moved: ours means we answer registrations
    /// (re-enumerating, since a new watcher starts empty); anyone else's
    /// means we host against them; none means we take it back.
    fn on_watcher_owner(&mut self, new: Option<&str>) -> Update {
        let ours = new == Some(self.conn.unique());
        self.watcher_owner = new.map(str::to_owned);
        if ours && self.mode == Mode::Owner {
            return Update::Unchanged;
        }
        if ours {
            self.mode = Mode::Owner;
            self.enumerate();
            return Update::Changed;
        }
        if new.is_none() {
            // Nobody owns it: take it back (answered later).
            let mut body = Writer::new();
            body.str(WATCHER_KDE);
            body.u32(proto::request::ALLOW_REPLACEMENT | proto::request::DO_NOT_QUEUE);
            if let Some(bytes) = body.take_body() {
                self.issue(
                    conn::BUS_NAME,
                    conn::BUS_PATH,
                    conn::BUS_INTERFACE,
                    "RequestName",
                    "su",
                    &bytes,
                    0,
                    Op::RequestKde,
                );
            }
            return Update::Changed;
        }
        self.mode = Mode::Host;
        self.refresh_watcher();
        Update::Changed
    }

    /// Works a watcher-name answer: ours means owner mode (matches,
    /// then enumeration); anyone else's means host mode against them
    /// (matches, then their list). The freedesktop twin is best effort:
    /// owned is nice, owned elsewhere changes nothing.
    fn on_request_reply(&mut self, kde: bool, signature: &str, body: &[u8]) -> Update {
        let Ok(word) = proto::read_request_reply(signature, body) else {
            return Update::Unchanged;
        };
        let owned = matches!(word, request_reply::PRIMARY_OWNER | request_reply::ALREADY_OWNER);
        if kde {
            self.match_rules();
            if owned {
                self.mode = Mode::Owner;
                self.enumerate();
            } else {
                self.mode = Mode::Host;
                self.read_watcher_owner();
            }
            return Update::Changed;
        }
        Update::Unchanged
    }

    /// Reads who owns the KDE watcher name (host mode): their list and
    /// our host registration follow.
    fn on_watcher_owner_reply(&mut self, signature: &str, body: &[u8]) -> Update {
        let Ok(owner) = proto::read_owner(signature, body) else {
            return Update::Unchanged;
        };
        self.watcher_owner = Some(owner);
        self.register_host();
        self.refresh_watcher();
        Update::Unchanged
    }

    /// Another watcher's item signals (host mode): a registration we did
    /// not see is picked up; an unregistration drops at once.
    fn on_watcher_signal(&mut self, sender: &str, member: &str, signature: &str, body: &[u8]) -> Update {
        if self.mode != Mode::Host {
            return Update::Unchanged;
        }
        // Our own echoes (we emit these in owner mode) are ignored.
        if sender == self.conn.unique() {
            return Update::Unchanged;
        }
        match member {
            "StatusNotifierItemRegistered" => {
                let Ok(service) = proto::read_string(signature, body) else {
                    return Update::Unchanged;
                };
                let Ok((service, path)) = split_service_path(sender, &service) else {
                    return Update::Unchanged;
                };
                self.add(service, path)
            }
            "StatusNotifierItemUnregistered" => {
                let Ok(service) = proto::read_string(signature, body) else {
                    return Update::Unchanged;
                };
                self.remove(&service)
            }
            _ => Update::Unchanged,
        }
    }

    /// An item's own signals: re-read what changed. `NewStatus` carries
    /// the status itself — taken at once, with a `GetAll` behind it for
    /// the rest (a lying signal loses nothing).
    fn on_item_signal(&mut self, sender: &str, member: &str) -> Update {
        let Some(id) = self.items.iter().find(|item| item.owner.as_deref() == Some(sender)).map(|item| item.id.clone()) else {
            return Update::Unchanged;
        };
        match member {
            "NewTitle" | "NewIcon" | "NewAttentionIcon" | "NewOverlayIcon" | "NewToolTip" | "NewStatus" => {
                self.refresh(&id);
                Update::Unchanged
            }
            _ => Update::Unchanged,
        }
    }

    /// Answers a call on the watcher object (owner mode): registrations,
    /// properties, introspection and ping. Anything else is an error, not
    /// silence (a caller waiting on no reply is the caller's bug, not
    /// ours to hang on). Eight arguments, as [`Live::issue`].
    #[allow(clippy::too_many_arguments)]
    fn on_call(
        &mut self,
        sender: &str,
        path: &str,
        interface: &str,
        member: &str,
        serial: u32,
        signature: &str,
        body: &[u8],
    ) -> Update {
        if path != WATCHER_PATH {
            self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownObject");
            return Update::Unchanged;
        }
        if interface == ITEM_PROPERTIES {
            return self.on_properties(sender, member, serial, signature, body);
        }
        if interface == "org.freedesktop.DBus.Introspectable" && member == "Introspect" {
            self.conn.reply_return(serial, "s", &introspect_body());
            return Update::Unchanged;
        }
        if interface == "org.freedesktop.DBus.Peer" {
            if member == "Ping" {
                self.conn.reply_return(serial, "", &[]);
                return Update::Unchanged;
            }
            self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        if interface != WATCHER_KDE && interface != WATCHER_FDO {
            self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        match member {
            "RegisterStatusNotifierItem" => {
                let Ok(service) = proto::read_string(signature, body) else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.InvalidArgs");
                    return Update::Unchanged;
                };
                let Ok((service, path)) = split_service_path(sender, &service) else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.InvalidArgs");
                    return Update::Unchanged;
                };
                self.conn.reply_return(serial, "", &[]);
                self.add(service, path)
            }
            "RegisterStatusNotifierHost" => {
                self.conn.reply_return(serial, "", &[]);
                if !self.hosts.contains(&sender.to_owned()) {
                    self.hosts.push(sender.to_owned());
                    self.emit_host_registered();
                }
                Update::Unchanged
            }
            _ => {
                self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownMethod");
                Update::Unchanged
            }
        }
    }

    /// Answers `Properties.Get`/`GetAll` on the watcher object.
    fn on_properties(
        &mut self,
        _sender: &str,
        member: &str,
        serial: u32,
        signature: &str,
        body: &[u8],
    ) -> Update {
        let mut reader = Reader::le(body);
        let read = |reader: &mut Reader<'_>| {
            let interface = reader.str().ok()?;
            if member == "GetAll" {
                Some((interface.to_owned(), String::new()))
            } else {
                let property = reader.str().ok()?;
                Some((interface.to_owned(), property.to_owned()))
            }
        };
        let _ = signature;
        let Some((interface, property)) = read(&mut reader) else {
            self.conn.reply_error(serial, "org.freedesktop.DBus.Error.InvalidArgs");
            return Update::Unchanged;
        };
        if interface != WATCHER_KDE && interface != WATCHER_FDO {
            self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        let mut out = Writer::new();
        match (member, property.as_str()) {
            ("GetAll", _) => {
                out.variant("a{sv}");
                let Some(cookie) = out.open_array(8) else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                write_watcher_props(self, &mut out);
                out.close_array(cookie);
                let Some(bytes) = out.take_body() else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(serial, "a{sv}", &bytes);
            }
            ("Get", "RegisteredStatusNotifierItems") => {
                out.variant("as");
                let Some(cookie) = out.open_array(4) else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                for item in &self.items {
                    out.str(&item.id);
                }
                out.close_array(cookie);
                let Some(bytes) = out.take_body() else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(serial, "v", &bytes);
            }
            ("Get", "IsStatusNotifierHostRegistered") => {
                out.variant("b");
                out.boolean(!self.hosts.is_empty());
                let Some(bytes) = out.take_body() else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(serial, "v", &bytes);
            }
            ("Get", "ProtocolVersion") => {
                out.variant("i");
                out.i32(PROTOCOL_VERSION as i32);
                let Some(bytes) = out.take_body() else {
                    self.conn.reply_error(serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(serial, "v", &bytes);
            }
            _ => {
                self.conn.reply_error(serial, "org.freedesktop.DBus.Error.UnknownMethod");
            }
        }
        Update::Unchanged
    }

    /// Adds an item (or re-reads one already shown), verifying it with a
    /// `GetNameOwner` first: a registration for a name nobody owns is
    /// answered and dropped.
    fn add(&mut self, service: String, path: String) -> Update {
        let id = format!("{service}{path}");
        if self.items.iter().any(|item| item.id == id) {
            self.refresh(&id);
            return Update::Unchanged;
        }
        if self.items.len() >= MAX_ITEMS {
            self.say_full(&id);
            return Update::Unchanged;
        }
        self.items.push(Item::new(id.clone(), service.clone(), path));
        self.sort();
        if service.starts_with(':') {
            self.set_owner(&id, service);
            self.refresh(&id);
        } else {
            let mut get = Writer::new();
            get.str(&service);
            let Some(bytes) = get.take_body() else {
                return Update::Changed;
            };
            self.issue(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "GetNameOwner",
                "s",
                &bytes,
                0,
                Op::Owner(id.clone()),
            );
        }
        if self.mode == Mode::Owner {
            self.emit_registered(&id);
        }
        Update::Changed
    }

    /// Drops the item with `id` (or every id under a vanished service),
    /// saying so to other hosts in owner mode.
    fn remove(&mut self, id: &str) -> Update {
        let before = self.items.len();
        self.items.retain(|item| item.id != id && item.service != id);
        if self.items.len() == before {
            return Update::Unchanged;
        }
        if self.items.len() < MAX_ITEMS {
            self.said_full = false;
        }
        if self.mode == Mode::Owner {
            self.emit_unregistered(id);
        }
        Update::Changed
    }

    fn set_owner(&mut self, id: &str, owner: String) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.owner = Some(owner);
        }
    }

    fn sort(&mut self) {
        self.items.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// Queues a `RequestName` for the KDE name or its twin.
    fn request(&mut self, name: &str, op: Op) {
        let mut body = Writer::new();
        body.str(name);
        body.u32(proto::request::ALLOW_REPLACEMENT | proto::request::DO_NOT_QUEUE);
        let Some(bytes) = body.take_body() else {
            return;
        };
        self.issue(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "RequestName",
            "su",
            &bytes,
            0,
            op,
        );
    }

    /// Installs the match rules: owner changes, item signals on both
    /// interfaces, and the other watcher's item signals for host mode.
    /// Answered to no one (duplicates error silently into the kept rule).
    fn match_rules(&mut self) {
        for rule in MATCH_RULES {
            let mut body = Writer::new();
            body.str(rule);
            let Some(bytes) = body.take_body() else {
                continue;
            };
            self.fire(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "AddMatch",
                "s",
                &bytes,
            );
        }
    }

    /// Queues a read of the KDE watcher name's owner (host mode).
    fn read_watcher_owner(&mut self) {
        let mut body = Writer::new();
        body.str(WATCHER_KDE);
        let Some(bytes) = body.take_body() else {
            return;
        };
        self.issue(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "GetNameOwner",
            "s",
            &bytes,
            0,
            Op::WatcherOwner,
        );
    }

    /// Registers our host with the existing watcher (host mode): no
    /// reply wanted, flushed with the rest.
    fn register_host(&mut self) {
        let Some(owner) = self.watcher_owner.clone() else {
            return;
        };
        let mut body = Writer::new();
        body.str(self.conn.unique());
        let Some(bytes) = body.take_body() else {
            return;
        };
        self.fire(&owner, WATCHER_PATH, WATCHER_KDE, "RegisterStatusNotifierHost", "s", &bytes);
    }

    /// Queues a `ListNames` to pick up what registered before us.
    fn enumerate(&mut self) {
        if self.flights.iter().any(|flight| {
            matches!(flight, Some(Flight { op: Op::Names }))
        }) {
            return;
        }
        self.issue(conn::BUS_NAME, conn::BUS_PATH, conn::BUS_INTERFACE, "ListNames", "", &[], 0, Op::Names);
    }

    /// Queues a read of the existing watcher's item list (host mode).
    fn refresh_watcher(&mut self) {
        let mut get = Writer::new();
        get.str(WATCHER_KDE);
        get.str("RegisteredStatusNotifierItems");
        let Some(bytes) = get.take_body() else {
            return;
        };
        // The watcher's owner: read fresh (it may have moved).
        let owner = self.watcher_owner.clone().unwrap_or_default();
        if owner.is_empty() {
            return;
        }
        self.issue(&owner, WATCHER_PATH, ITEM_PROPERTIES, "Get", "ss", &bytes, 0, Op::WatcherItems);
    }

    fn emit_registered(&mut self, id: &str) {
        let mut body = Writer::new();
        body.str(id);
        if let Some(bytes) = body.take_body() {
            self.conn.signal(WATCHER_PATH, WATCHER_KDE, "StatusNotifierItemRegistered", "s", &bytes);
        }
    }

    fn emit_unregistered(&mut self, id: &str) {
        let mut body = Writer::new();
        body.str(id);
        if let Some(bytes) = body.take_body() {
            self.conn.signal(WATCHER_PATH, WATCHER_KDE, "StatusNotifierItemUnregistered", "s", &bytes);
        }
    }

    fn emit_host_registered(&mut self) {
        self.conn.signal(WATCHER_PATH, WATCHER_KDE, "StatusNotifierHostRegistered", "", &[]);
    }

    /// The `too many items` warning: once per full set, on stderr, never
    /// per wake.
    fn say_full(&mut self, id: &str) {
        if !self.said_full {
            self.said_full = true;
            crate::print::warn(format_args!(
                "scootbar: tray: ignoring `{id}`: more than {MAX_ITEMS} items are not shown"
            ));
        }
    }

    /// The item index `at` in id order, or `None` past the end.
    fn at(&self, index: i32) -> Option<&Item> {
        (index >= 0).then(|| self.items.get(index as usize)).flatten()
    }
}

impl Item {
    fn new(id: String, service: String, path: String) -> Self {
        Self {
            id,
            service,
            path,
            owner: None,
            status: Status::Passive,
            title: String::new(),
            tooltip_title: String::new(),
            tooltip_text: String::new(),
            menu: String::new(),
            item_is_menu: false,
            icons: Vec::new(),
        }
    }

    /// Picks the icon for `side` device pixels: the smallest kept entry
    /// at or past it, else the largest kept — exact device pixels where
    /// one matches, a smooth scale otherwise. Icons are stored smallest
    /// first (see `convert`).
    fn icon_for(&self, side: u32) -> Option<&Arc<TrayIcon>> {
        let mut largest = None;
        for icon in &self.icons {
            largest = Some(icon);
            if icon.side() >= side {
                return Some(icon);
            }
        }
        largest
    }
}

/// What the view is drawn from: everything `fill` sets that the screen
/// shows. Compared before and after each answer, so the bar redraws on a
/// real change and nothing else.
#[derive(PartialEq, Eq)]
struct Fingerprint {
    status: Status,
    title: String,
    tooltip_title: String,
    tooltip_text: String,
    menu: String,
    item_is_menu: bool,
    icons: Vec<Arc<TrayIcon>>,
}

impl Fingerprint {
    fn of(item: &Item) -> Self {
        Self {
            status: item.status,
            title: item.title.clone(),
            tooltip_title: item.tooltip_title.clone(),
            tooltip_text: item.tooltip_text.clone(),
            menu: item.menu.clone(),
            item_is_menu: item.item_is_menu,
            icons: item.icons.clone(),
        }
    }
}

/// Starts the bus live: queues both watcher names and returns at once.
/// Nothing here blocks past `Hello` (inside [`conn::setup`]): the names
/// are answered later, through [`Live::on_request_reply`], which installs
/// the match rules and enumerates. The module joins the bar on the first
/// frame either way, filling in as the bus answers.
fn setup(conn: Conn) -> Live {
    let mut live = Live {
        conn,
        // Host until the KDE name answers otherwise: registrations reach
        // whoever owns the name, so nothing is answered prematurely.
        mode: Mode::Host,
        watcher_owner: None,
        items: Vec::new(),
        flights: Vec::new(),
        // Our own host is first: `IsStatusNotifierHostRegistered` holds
        // from the start in owner mode.
        hosts: Vec::new(),
        said_full: false,
    };
    live.hosts.push(live.conn.unique().to_owned());
    live.request(WATCHER_KDE, Op::RequestKde);
    live.request(WATCHER_FDO, Op::RequestFdo);
    live
}

/// The match rules: owner changes, item signals on both interfaces, and
/// the other watcher's item signals for host mode. Installed once the
/// names answer (and again on every re-acquire: a new connection has no
/// rules, and re-adding to the same one errors silently into the kept
/// rule).
const MATCH_RULES: [&str; 5] = [
    "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
     member='NameOwnerChanged',path='/org/freedesktop/DBus'",
    "type='signal',interface='org.kde.StatusNotifierItem'",
    "type='signal',interface='org.freedesktop.StatusNotifierItem'",
    "type='signal',interface='org.kde.StatusNotifierWatcher'",
    "type='signal',interface='org.freedesktop.StatusNotifierWatcher'",
];

impl Live {
}

/// The `Properties.GetAll` body for the item interface.
fn get_all_body() -> Vec<u8> {
    let mut body = Writer::new();
    body.str(ITEM_KDE);
    body.take_body().unwrap_or_default()
}

/// Whether a bus name is an item's well-known name (the KDE or
/// freedesktop prefix): what the start-up enumeration picks up. Items
/// behind plain unique names register explicitly and cannot be listed —
/// the KDE watcher's own limitation.
fn is_item_name(name: &str) -> bool {
    name.starts_with("org.kde.StatusNotifierItem") || name.starts_with("org.freedesktop.StatusNotifierItem")
}

/// Applies a `GetAll` body to the item: sanitized strings and converted
/// icons. `false` drops the answer (the item keeps its last state).
fn fill(item: &mut Item, body: &[u8]) -> bool {
    let mut props = Reader::le(body);
    let Ok(raw) = props.array_raw(8) else {
        return false;
    };
    let mut entries = Reader::le(raw);
    let mut status = None;
    let mut title = None;
    let mut tooltip: Option<(String, String, String)> = None;
    let mut menu = None;
    let mut item_is_menu = None;
    let mut pixmaps: Option<Vec<Pixmap<'_>>> = None;
    while !entries.exhausted() {
        if entries.enter_struct().is_err() {
            return false;
        }
        let Ok(key) = entries.str() else {
            return false;
        };
        let Ok(sig) = entries.signature() else {
            return false;
        };
        match (key, sig) {
            ("Status", "s") => status = entries.str().ok(),
            ("Title", "s") => title = entries.str().ok(),
            ("Menu", "o") => menu = entries.str().ok(),
            ("ItemIsMenu", "b") => item_is_menu = entries.boolean().ok(),
            ("ToolTip", "(sa(iiay)ss)") => match read_tooltip_shape(&mut entries) {
                Ok(shown) => tooltip = Some(shown),
                Err(()) => return false,
            },
            ("IconPixmap", "a(iiay)") => match entries.array_raw(8).ok().and_then(|elements| read_pixmaps(elements).ok()) {
                Some(list) => pixmaps = Some(list),
                None => return false,
            },
            _ => {
                if entries.skip(sig).is_err() {
                    return false;
                }
            }
        }
        entries.leave_struct();
    }
    if !props.exhausted() {
        return false;
    }
    if let Some(status) = status {
        item.status = Status::parse(status);
    }
    if let Some(title) = title {
        item.title = clean(title);
    }
    if let Some((_name, title, text)) = tooltip {
        item.tooltip_title = clean(&title);
        item.tooltip_text = clean(&text);
    }
    if let Some(menu) = menu {
        item.menu = menu.to_owned();
    }
    if let Some(item_is_menu) = item_is_menu {
        item.item_is_menu = item_is_menu;
    }
    if let Some(pixmaps) = pixmaps {
        item.icons = convert(&item.id, &pixmaps);
    }
    true
}

/// Reads a `(sa(iiay)ss)` tooltip value off the cursor: the icon name,
/// the title and the text (the pixmaps are validated and dropped — the
/// tooltip shows no icon until the tooltips entry lands).
fn read_tooltip_shape(entries: &mut Reader<'_>) -> Result<(String, String, String), ()> {
    entries.enter_struct().map_err(|_| {
    })?;
    let name = entries.str().map_err(|_| {
    })?.to_owned();
    let elements = entries.array_raw(8).map_err(|_| {
    })?;
    read_pixmaps(elements).map_err(|_| {
    })?;
    let title = entries.str()?.to_owned();
    let text = entries.str()?.to_owned();
    entries.leave_struct();
    Ok((name, title, text))
}

/// Converts the kept pixmap entries to premultiplied icons, smallest
/// first: entries past [`MAX_STORED_SIDE`] are left for the icon cache
/// to scale from the single smallest kept, at most [`MAX_STORED_ICONS`]
/// entries. Each id hashes its pixels, so an unchanged icon compares
/// equal and the cache hits.
fn convert(id: &str, pixmaps: &[Pixmap<'_>]) -> Vec<Arc<TrayIcon>> {
    let mut kept: Vec<&Pixmap<'_>> = pixmaps
        .iter()
        .filter(|pixmap| pixmap.width <= MAX_STORED_SIDE && pixmap.height <= MAX_STORED_SIDE)
        .collect();
    kept.sort_by_key(|pixmap| pixmap.width.max(pixmap.height));
    if kept.is_empty() {
        kept = pixmaps.iter().min_by_key(|pixmap| pixmap.width.max(pixmap.height)).into_iter().collect();
    }
    kept.truncate(MAX_STORED_ICONS);
    kept
        .into_iter()
        .filter_map(|pixmap| {
            let key = fnv(id, pixmap.width, pixmap.height, pixmap.pixels);
            TrayIcon::take(key, pixmap.width, pixmap.height, pixmap.pixels).map(Arc::new)
        })
        .collect()
}

/// FNV-1a over the icon's identity and pixels: the cache key. Collisions
/// show a stale icon until the next `NewIcon`; 64 bits over a handful of
/// icons makes that a non-event.
fn fnv(id: &str, width: u32, height: u32, pixels: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in id.bytes().chain(width.to_le_bytes()).chain(height.to_le_bytes()).chain(pixels.iter().copied()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The introspection XML of the watcher object: the two watcher
/// interfaces, properties, introspection and ping — nothing else.
fn introspect_body() -> Vec<u8> {
    const XML: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg name="service" type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg name="service" type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
  <interface name="org.freedesktop.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg name="service" type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg name="service" type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get"><arg name="interface" type="s" direction="in"/><arg name="property" type="s" direction="in"/><arg name="value" type="v" direction="out"/></method>
    <method name="GetAll"><arg name="interface" type="s" direction="in"/><arg name="properties" type="a{sv}" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect"><arg name="data" type="s" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
  </interface>
</node>"#;
    let mut body = Writer::new();
    body.str(XML);
    body.take_body().unwrap_or_default()
}

/// Writes the watcher properties into the open `a{sv}`: the item ids in
/// shown order, whether a host is registered, and the version.
fn write_watcher_props(live: &Live, out: &mut Writer) {
    if out.open_struct() {
        out.str("RegisteredStatusNotifierItems");
        out.variant("as");
        if let Some(cookie) = out.open_array(4) {
            for item in &live.items {
                out.str(&item.id);
            }
            out.close_array(cookie);
        }
        out.close_struct();
    }
    if out.open_struct() {
        out.str("IsStatusNotifierHostRegistered");
        out.variant("b");
        out.boolean(!live.hosts.is_empty());
        out.close_struct();
    }
    if out.open_struct() {
        out.str("ProtocolVersion");
        out.variant("i");
        out.i32(PROTOCOL_VERSION as i32);
        out.close_struct();
    }
}

impl Module for Tray {
    /// The bus socket while live (with `OUT` while answers wait), then
    /// the directory watch while waiting. No timer, ever: every refresh
    /// is bus-driven, and the wait is an inotify watch.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        match &self.bus {
            Bus::Live(live) => {
                let mut flags = PollFlags::IN;
                if live.conn.want_write() {
                    flags |= PollFlags::OUT;
                }
                sources.add(live.conn.as_fd(), flags);
            }
            Bus::Waiting { notify, .. } => {
                if let Some(notify) = notify {
                    sources.add(notify.fd.as_fd(), PollFlags::IN);
                }
            }
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        match &self.bus {
            Bus::Live(_) => {
                // Source 0 is the bus socket: the only source a live
                // module adds. Anything else is unreachable (the harness
                // holds it to that); drained as the socket rather than
                // panicking the bar.
                let _ = source;
                self.on_bus(events)
            }
            Bus::Waiting { .. } => {
                let _ = source;
                self.on_notify(events)
            }
        }
    }

    /// Nothing textual: the icons are drawn by [`Module::custom_draw`]
    /// and sized by [`Module::span_extra`], so an empty view with icons
    /// still takes no text. The tooltip lists the shown titles that fit.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Bus::Live(live) = &self.bus else {
            return;
        };
        for item in &live.items {
            let title = if item.title.is_empty() {
                &item.tooltip_title
            } else {
                &item.title
            };
            if title.is_empty() {
                continue;
            }
            let sep = if view.tooltip().is_empty() { "" } else { ", " };
            if view.tooltip().len() + sep.len() + title.len() > super::MAX_TEXT {
                break;
            }
            let _ = write!(view.tooltip_mut(), "{sep}{title}");
        }
    }

    /// The icons' width past the (empty) text: one slot a shown item, and
    /// a gap between neighbours. Zero with no items, so the module hides
    /// like any empty one.
    fn span_extra(&self, measure: &super::Measure<'_>) -> u32 {
        let Bus::Live(live) = &self.bus else {
            return 0;
        };
        let count = live.items.len();
        if count == 0 {
            return 0;
        }
        let side = Text::art_side(measure.em) as usize;
        (count * side + (count - 1) * gap(side)) as u32
    }

    /// Draws each item's icon at the output's device size, from the
    /// shared icon cache (a `NewIcon` misses once; steady frames hit).
    /// `true`: the loop draws nothing more for this span.
    fn custom_draw(&self, ctx: &mut super::CustomDraw<'_, '_>) -> bool {
        let Some(live) = self.bus_live() else {
            return false;
        };
        if live.items.is_empty() {
            return false;
        }
        let side = Text::art_side(ctx.em);
        let stride = side as usize + gap(side as usize);
        let top = (i64::from(ctx.canvas.height()) - i64::from(side)) / 2;
        let mut x = i64::from(ctx.span.x) + i64::from(ctx.padding);
        for item in &live.items {
            if let Some(icon) = item.icon_for(side) {
                let art = Art::Tray(icon.clone());
                if let Some(crate::icon::Bitmap::Premultiplied(pixels)) = ctx.text.bitmap(&art, side) {
                    let pixels: &[u8] = pixels;
                    let edge = side as usize;
                    for (gy, row) in pixels.chunks_exact(edge * 4).enumerate() {
                        for (gx, pixel) in row.chunks_exact(4).enumerate() {
                            let pixel = [pixel[0], pixel[1], pixel[2], pixel[3]];
                            ctx.canvas.blend_premultiplied(
                                x + gx as i64,
                                top + gy as i64,
                                pixel,
                                ctx.span,
                            );
                        }
                    }
                }
            }
            x += stride as i64;
        }
        true
    }

    /// A click activates, a middle click secondarily, a scroll scrolls —
    /// each on the item under the pointer, with no binding at all. A
    /// right click means nothing by default: the menu waits on popups,
    /// and silence beats a refusal on every click.
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        let live = self.bus_live()?;
        let index = Self::hit(input.at.x, input.at.padding, input.at.em, live.items.len())?;
        let name = match input.trigger {
            Trigger::Click => "activate",
            Trigger::MiddleClick => "secondary",
            Trigger::ScrollUp => "scroll-up",
            Trigger::ScrollDown => "scroll-down",
            Trigger::RightClick => return None,
        };
        Some(crate::action::Action::Module(ModuleAction::new(name, Some(index as i32))))
    }

    /// Carries out the item actions: `activate`, `secondary` and the two
    /// scrolls call the item (never blocking the bar); `menu` is refused
    /// naming the popups entry. Every one takes the item index.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let index = action.arg.ok_or(InvokeError::NeedsArg)?;
        match &*action.name {
            "activate" | "secondary" | "scroll-up" | "scroll-down" | "menu" => {}
            _ => return Err(InvokeError::Unknown),
        }
        let Bus::Live(live) = &mut self.bus else {
            return Err(InvokeError::Refused("no bus to call on"));
        };
        if live.at(index).is_none() {
            return Err(InvokeError::Refused("no such tray item"));
        }
        match &*action.name {
            "activate" => {
                let mut body = Writer::new();
                body.i32(0);
                body.i32(0);
                let Some(bytes) = body.take_body() else {
                    return Err(InvokeError::Refused("the call does not fit"));
                };
                let item = live.at(index).expect("checked");
                let (service, path) = (item.service.clone(), item.path.clone());
                live.fire(&service, &path, ITEM_KDE, "Activate", "ii", &bytes);
                Ok(Update::Unchanged)
            }
            "secondary" => {
                let mut body = Writer::new();
                body.i32(0);
                body.i32(0);
                let Some(bytes) = body.take_body() else {
                    return Err(InvokeError::Refused("the call does not fit"));
                };
                let item = live.at(index).expect("checked");
                let (service, path) = (item.service.clone(), item.path.clone());
                live.fire(&service, &path, ITEM_KDE, "SecondaryActivate", "ii", &bytes);
                Ok(Update::Unchanged)
            }
            "scroll-up" | "scroll-down" => {
                if steps == 0 {
                    return Ok(Update::Unchanged);
                }
                let delta = steps.min(MAX_SCROLL_DELTA) as i32;
                let mut body = Writer::new();
                body.i32(delta);
                body.str("vertical");
                let Some(bytes) = body.take_body() else {
                    return Err(InvokeError::Refused("the call does not fit"));
                };
                let item = live.at(index).expect("checked");
                let (service, path) = (item.service.clone(), item.path.clone());
                live.fire(&service, &path, ITEM_KDE, "Scroll", "is", &bytes);
                Ok(Update::Unchanged)
            }
            _ => Err(InvokeError::Refused("tray menus wait on the popups entry")),
        }
    }

    /// A click activates with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// What `query` reports: the mode and the shown items, or nothing
    /// while nothing is shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let live = self.bus_live()?;
        if live.items.is_empty() {
            return None;
        }
        let mode = match live.mode {
            Mode::Owner => "owner",
            Mode::Host => "host",
        };
        Some(serde_json::json!({
            "watcher": mode,
            "items": live.items.iter().map(|item| serde_json::json!({
                "id": item.id,
                "title": item.title,
                "status": item.status.name(),
            })).collect::<Vec<_>>(),
        }))
    }
}

impl Tray {
    /// The live bus, if there is one.
    fn bus_live(&self) -> Option<&Live> {
        match &self.bus {
            Bus::Live(live) => Some(live),
            Bus::Waiting { .. } => None,
        }
    }
}
