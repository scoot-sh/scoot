//! The live half of the bluetooth module: the BlueZ conversation, with
//! no drawing in it.
//!
//! One connection to the system bus, four match rules, no polling.
//! `NameOwnerChanged` for `org.bluez` says when BlueZ appears, restarts
//! or leaves (tracked by owner: a release by a dead BlueZ is the bus's own
//! word); `InterfacesAdded` and `InterfacesRemoved` say when an adapter
//! or a device comes or goes (hotplug, rfkill, a pairing); and
//! `PropertiesChanged` on an adapter, device or battery object says when
//! power, connection, name or charge moves. The BlueZ rules are filtered
//! by the bus to `/org/bluez`, and every one of them is believed only from
//! the current owner of `org.bluez`: anything on the bus can send these
//! signals. Replies are matched by serial and sender — a reply from anyone
//! but the callee is refused whenever the callee is known — except calls
//! made of the well-known `org.bluez` itself, whose holder the client
//! cannot know: a forged answer to one of those is accepted, like any
//! peer's own claim to the name.
//!
//! The whole set starts from one `GetManagedObjects`; a `GetAll` re-reads
//! a single interface whose signal invalidated a shown property. A
//! `GetManagedObjects` answer past what the client reads (1 MiB, realistic
//! on a machine with many known devices) is kept as the last shown state
//! instead: the module marks itself stale and reads again at the next
//! signal, rather than clearing what it shows or asking at once for the
//! same oversized answer.
//!
//! ## What a peer can do
//!
//! Anything on the system bus can own `org.bluez` when BlueZ itself is
//! absent and say anything. It cannot crash or hang the bar or grow it
//! without bound: strings are cleaned and cut where they are stored, a
//! reply that does not parse leaves the last state, one that errors or
//! never comes is asked again at the next signal, and `NameOwnerChanged`
//! is believed only from the bus. At most [`MAX_ADAPTERS`] adapters and
//! [`MAX_DEVICES`] devices are held; a newcomer to a full room is ignored,
//! said once. A signal is applied only from the tracked owner, only for a
//! held path, and a `PropertiesChanged` for an unknown path re-reads the
//! set once instead of trusting it. What a hostile peer can still do is
//! bounded loss of visibility: full tables hide a later object until a
//! slot frees, and a forged owner (on a bus without BlueZ) is shown as
//! BlueZ.

use std::time::Instant;

use super::timer::OneShot;
use super::{FLIGHT_TTL, MAX_ADAPTERS, MAX_DEVICES, MIN_REFRESH_GAP};
use crate::dbus::bluez::{self, ADAPTER, BATTERY, DEVICE, MANAGER, NAME, PROPERTIES, ROOT};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Session;
use crate::dbus::proto::{self, Writer};

/// The match rules, filtered by the bus: the owner of `org.bluez`, and
/// the object and property signals under `/org/bluez` only. Anything
/// elsewhere never reaches the bar (a test against a real `dbus-daemon`
/// sends both, and fails when the namespace filter is removed).
pub(super) const MATCH_RULES: [&str; 4] = [
    "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
     member='NameOwnerChanged',path='/org/freedesktop/DBus',arg0='org.bluez'",
    "type='signal',interface='org.freedesktop.DBus.ObjectManager',\
     member='InterfacesAdded',path_namespace='/org/bluez'",
    "type='signal',interface='org.freedesktop.DBus.ObjectManager',\
     member='InterfacesRemoved',path_namespace='/org/bluez'",
    "type='signal',interface='org.freedesktop.DBus.Properties',\
     member='PropertiesChanged',path_namespace='/org/bluez'",
];

/// A call in flight: what its reply is for.
struct Flight {
    op: Op,
}

enum Op {
    /// An `AddMatch`: only its refusal matters.
    Match,
    /// `GetNameOwner("org.bluez")`: who BlueZ is, if anyone.
    Owner,
    /// `GetManagedObjects`: the whole set.
    Managed,
    /// `GetAll` for one object and interface.
    Props(Refresh),
}

/// A single interface re-read: the object (by id, so an answer meant for
/// an object that left finds nothing) and which interface.
struct Refresh {
    id: u64,
    iface: RefreshIface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RefreshIface {
    Adapter,
    Device,
    Battery,
}

/// One adapter: its path and whether it is powered.
pub(super) struct Adapter {
    pub(super) id: u64,
    pub(super) path: String,
    pub(super) powered: bool,
    /// A `GetAll` in flight since, if one is.
    pub(super) asked: Option<Instant>,
    pub(super) last_asked: Option<Instant>,
    /// A signal arrived while one was in flight, or the last read
    /// errored: read again.
    pub(super) stale: bool,
}

/// One device: its path, whether it is connected, the name shown for it
/// (`Name`, else `Alias`, else the path's last element, cleaned at
/// store), and its charge when BlueZ reports one.
pub(super) struct Device {
    pub(super) id: u64,
    pub(super) path: String,
    pub(super) connected: bool,
    pub(super) name: String,
    pub(super) has_battery: bool,
    pub(super) battery: Option<u8>,
    asked: Option<Instant>,
    last_asked: Option<Instant>,
    stale: bool,
}

/// The connection and what the module knows of BlueZ on it.
pub(super) struct Live {
    conn: Conn,
    /// The current owner of `org.bluez`: signals come only from it, and
    /// only while it is `Some`.
    owner: Option<String>,
    pub(super) adapters: Vec<Adapter>,
    pub(super) devices: Vec<Device>,
    flights: Vec<Option<Flight>>,
    next_id: u64,
    /// Bumped on every state change the view reads: what the module
    /// compares before and after each turn.
    pub(super) rev: u64,
    /// Reused for every cleaned string, so a rename allocates nothing in
    /// steady state.
    scratch: String,
    /// A `GetManagedObjects` awaits its answer.
    pub(super) managed_in_flight: bool,
    /// Its answer was dropped or errored, or a signal arrived while one
    /// was in flight: read again, showing the last state meanwhile. A
    /// failed read retries once, on the coalesce timer; past that it
    /// waits for the next signal, never asking at once for the same
    /// oversized answer in a loop.
    pub(super) stale_managed: bool,
    /// The last `GetManagedObjects` completion succeeded: a stale set
    /// may be re-read at once. After a failure it may not (that loops),
    /// and waits for the timer or the next signal instead.
    managed_clean: bool,
    /// The re-read the coalesce timer fires is the one retry a failure
    /// gets: its own failure does not arm another.
    managed_retry: bool,
    /// A one-shot timer, armed only while an object waits out
    /// [`MIN_REFRESH_GAP`] to be read again: none waiting, no timer, no
    /// wakeups.
    pub(super) coalesce: Option<OneShot>,
    said_full: bool,
    said_match: bool,
}

impl Session for Live {
    fn conn(&self) -> &Conn {
        &self.conn
    }

    fn conn_mut(&mut self) -> &mut Conn {
        &mut self.conn
    }
}

/// Starts the session on a fresh connection: the match rules first, then
/// `GetNameOwner`, so a BlueZ that appears between the two is in one of
/// them. Nothing blocks past the `Hello` inside [`conn::setup`]. With no
/// owner there is nothing to enumerate, and `NameOwnerChanged` says when
/// one arrives.
pub(super) fn start(conn: Conn) -> Live {
    let mut live = Live {
        conn,
        owner: None,
        adapters: Vec::new(),
        devices: Vec::new(),
        flights: Vec::new(),
        next_id: 0,
        rev: 0,
        scratch: String::new(),
        managed_in_flight: false,
        stale_managed: false,
        managed_clean: true,
        managed_retry: false,
        coalesce: None,
        said_full: false,
        said_match: false,
    };
    for rule in MATCH_RULES {
        if let Some(body) = one_string(rule) {
            live.issue(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "AddMatch",
                "s",
                &body,
                Op::Match,
            );
        }
    }
    live.ask_owner();
    live
}

/// A body of one string.
fn one_string(text: &str) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.str(text);
    body.take_body()
}

/// Whether `path` is a BlueZ object: under `/org/bluez`, never the root
/// itself (which carries only the manager).
fn is_object(path: &str) -> bool {
    path.len() > ROOT.len() && path.starts_with(ROOT) && path.as_bytes()[ROOT.len()] == b'/'
}

impl Live {
    /// Works one bus event.
    pub(super) fn apply(&mut self, event: Event) {
        self.work(event);
        // A signal arrived while the enumeration was in flight: read
        // again now that it completed. Never after a failure (that
        // loops asking for the same answer); the timer or the next
        // signal carries those.
        if self.stale_managed
            && self.managed_clean
            && !self.managed_in_flight
            && self.owner.is_some()
        {
            self.stale_managed = false;
            self.read_managed();
        }
    }

    fn work(&mut self, event: Event) {
        match event {
            Event::Reply {
                token,
                signature,
                body,
            } => self.on_reply(token, &signature, &body),
            Event::CallError { token, .. } => self.on_error(token),
            Event::Dropped { token } => self.on_dropped(token),
            Event::Signal {
                sender,
                path,
                interface,
                member,
                signature,
                body,
            } => self.on_signal(&sender, &path, &interface, &member, &signature, &body),
            // Nothing is served here: a call to the bar is a peer's
            // mistake, and an unanswered one is the caller's to time out.
            Event::MethodCall { .. } => {}
        }
    }

    /// Queues a call that wants a reply, tracking its flight: `false`
    /// when it was not queued (the table is full of live calls).
    #[allow(clippy::too_many_arguments)]
    fn issue(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
        op: Op,
    ) -> bool {
        let Some(slot) = self.free_slot() else {
            return false;
        };
        let queued = self
            .conn
            .call(
                destination,
                path,
                interface,
                member,
                body_sig,
                body,
                0,
                slot as u64,
            )
            .is_ok();
        if queued {
            self.flights[slot] = Some(Flight { op });
        }
        queued
    }

    /// A free slot in the flight table, reaping the unanswered when it is
    /// full.
    fn free_slot(&mut self) -> Option<usize> {
        let free = |flights: &[Option<Flight>]| flights.iter().position(Option::is_none);
        if let Some(slot) = free(&self.flights) {
            return Some(slot);
        }
        if self.flights.len() < conn::MAX_PENDING {
            self.flights.push(None);
            return Some(self.flights.len() - 1);
        }
        self.reap();
        free(&self.flights)
    }

    /// Forgets the calls the connection gave up on ([`FLIGHT_TTL`]),
    /// freeing what they held: an object whose `GetAll` never answered
    /// can be asked again.
    fn reap(&mut self) {
        for token in self.conn.expire(FLIGHT_TTL) {
            let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
                continue;
            };
            match flight.op {
                Op::Props(refresh) => {
                    self.finish_fetch(&refresh);
                }
                Op::Managed => self.managed_in_flight = false,
                Op::Owner | Op::Match => {}
            }
        }
    }

    /// Asks who `org.bluez` is: its owner starts the enumeration, and its
    /// absence (an error) clears what is shown.
    fn ask_owner(&mut self) {
        if let Some(body) = one_string(NAME) {
            self.issue(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "GetNameOwner",
                "s",
                &body,
                Op::Owner,
            );
        }
    }

    /// Reads the whole set: one `GetManagedObjects` at a time.
    fn read_managed(&mut self) {
        if self.managed_in_flight || self.owner.is_none() {
            return;
        }
        let queued = self.issue(
            NAME,
            "/",
            MANAGER,
            "GetManagedObjects",
            "",
            &[],
            Op::Managed,
        );
        self.managed_in_flight = queued;
        if !queued {
            self.stale_managed = true;
        }
    }

    /// Sends `Set(Adapter1.Powered, powered)` to the first adapter: the
    /// click's toggle. Fire and forget, like the media module's controls:
    /// the state converges when BlueZ says so, as a signal. A device that
    /// vanished meanwhile is nothing: there is no reply to miss.
    pub(super) fn set_powered(&mut self, powered: bool) {
        let Some(path) = self.first_adapter_path() else {
            return;
        };
        let mut body = Writer::new();
        body.str(ADAPTER);
        body.str("Powered");
        body.variant("b");
        body.boolean(powered);
        if let Some(body) = body.take_body() {
            let _ = self.conn.call(
                NAME,
                &path,
                PROPERTIES,
                "Set",
                "ssv",
                &body,
                proto::flag::NO_REPLY_EXPECTED,
                0,
            );
        }
    }

    /// The first adapter's path, for the toggle's target.
    fn first_adapter_path(&self) -> Option<String> {
        self.adapters
            .iter()
            .min_by(|a, b| a.path.cmp(&b.path))
            .map(|adapter| adapter.path.clone())
    }

    /// Reads one object's interface again: one `GetAll` in flight per
    /// object, no oftener than [`MIN_REFRESH_GAP`]. A request while one
    /// is in flight, or too soon, marks the object stale and the answer
    /// (or the timer) asks again.
    pub(super) fn refresh(&mut self, id: u64, iface: RefreshIface) {
        if !self.exists(id, iface) {
            return;
        }
        if self
            .fetch_times(id, iface)
            .is_some_and(|(asked, _)| asked.is_some_and(|at| at.elapsed() > FLIGHT_TTL))
        {
            self.reap();
        }
        let (asked, last) = self.fetch_times(id, iface).unwrap_or((None, None));
        if asked.is_some() {
            self.set_stale(id, iface, true);
            return;
        }
        if last.is_some_and(|at| at.elapsed() < MIN_REFRESH_GAP) {
            self.set_stale(id, iface, true);
            self.arm_coalesce();
            return;
        }
        let (path, member_iface) = match iface {
            RefreshIface::Adapter => match self.adapters.iter().find(|a| a.id == id) {
                Some(adapter) => (adapter.path.clone(), ADAPTER),
                None => return,
            },
            RefreshIface::Device => match self.devices.iter().find(|d| d.id == id) {
                Some(device) => (device.path.clone(), DEVICE),
                None => return,
            },
            RefreshIface::Battery => match self.devices.iter().find(|d| d.id == id) {
                Some(device) => (device.path.clone(), BATTERY),
                None => return,
            },
        };
        let now = Instant::now();
        self.set_fetch_times(id, iface, Some(now), Some(now));
        let Some(body) = one_string(member_iface) else {
            self.set_fetch_times(id, iface, None, self.last_asked(id, iface));
            return;
        };
        let queued = self.issue(
            NAME,
            &path,
            PROPERTIES,
            "GetAll",
            "s",
            &body,
            Op::Props(Refresh { id, iface }),
        );
        if !queued {
            self.set_fetch_times(id, iface, None, self.last_asked(id, iface));
        }
    }

    /// Arms the one-shot timer that reads again the objects waiting out
    /// the gap (a no-op while armed). Without a timer they are read at
    /// their next signal: the slower answer, never the wrong one.
    fn arm_coalesce(&mut self) {
        if self.coalesce.is_none() {
            self.coalesce = OneShot::after(MIN_REFRESH_GAP);
        }
    }

    /// The gap passed: reads again the objects that waited, and the set
    /// when its re-read is owed one.
    pub(super) fn on_coalesce(&mut self) {
        if let Some(timer) = self.coalesce.take() {
            timer.drain();
        }
        if self.stale_managed && !self.managed_in_flight && self.owner.is_some() {
            self.stale_managed = false;
            self.managed_retry = true;
            self.read_managed();
            if !self.managed_in_flight {
                // Nothing was queued (the table is full): not a read,
                // so its failure must still arm one.
                self.managed_retry = false;
            }
        }
        let mut waiting = Vec::new();
        for adapter in &self.adapters {
            if adapter.stale && adapter.asked.is_none() {
                waiting.push((adapter.id, RefreshIface::Adapter));
            }
        }
        for device in &self.devices {
            if device.stale && device.asked.is_none() {
                waiting.push((device.id, RefreshIface::Device));
                waiting.push((device.id, RefreshIface::Battery));
            }
        }
        for (id, iface) in waiting {
            // A device without a battery answers its `GetAll` with an
            // empty dictionary: harmless, and rarer than the signal that
            // marks it.
            self.set_stale(id, iface, false);
            self.refresh(id, iface);
        }
    }

    fn on_reply(&mut self, token: u64, signature: &str, body: &[u8]) {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        match flight.op {
            Op::Match => {}
            Op::Owner => {
                if signature == "s" {
                    if let Ok(owner) = proto::read_owner(signature, body) {
                        self.set_owner(Some(owner));
                        return;
                    }
                }
                // An unparsable owner is no owner.
                self.set_owner(None);
            }
            Op::Managed => {
                self.managed_in_flight = false;
                if signature != "a{oa{sa{sv}}}" {
                    self.fail_managed();
                    return;
                }
                match bluez::read_managed_objects(body) {
                    Ok(objects) => {
                        self.stale_managed = false;
                        self.managed_clean = true;
                        self.managed_retry = false;
                        self.replace(objects);
                    }
                    // A misshapen set leaves the last state: one retry
                    // on the timer, then the next signal.
                    Err(()) => self.fail_managed(),
                }
            }
            Op::Props(refresh) => {
                let stale = self.finish_fetch(&refresh);
                self.on_props(&refresh, signature, body);
                if stale {
                    self.refresh(refresh.id, refresh.iface);
                }
            }
        }
    }

    /// A reply past the size this client reads was skipped: the call is
    /// answered with nothing usable. The set keeps its last state and is
    /// read again at its next signal (asking at once would fetch the same
    /// oversized answer, so `stale` waits for one).
    /// [`conn::DROPPED_UNKNOWN`] instead of a token: the skipped reply's
    /// header was not read, so no call can be named. Some call lost its
    /// answer: every in-flight read fails the way its own drop would (the
    /// tray and the media module do the same). Without this a skipped
    /// `GetManagedObjects` leaves `managed_in_flight` set with nothing
    /// stale, and the set is never read again.
    fn on_dropped(&mut self, token: u64) {
        if token == conn::DROPPED_UNKNOWN {
            if self.managed_in_flight {
                self.fail_managed();
            }
            let mut stale = Vec::new();
            for flight in self.flights.iter().flatten() {
                if let Op::Props(refresh) = &flight.op {
                    stale.push((refresh.id, refresh.iface));
                }
            }
            for (id, iface) in stale {
                self.finish_fetch(&Refresh { id, iface });
                self.set_stale(id, iface, true);
            }
            return;
        }
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        match flight.op {
            Op::Managed => self.fail_managed(),
            Op::Props(refresh) => {
                self.finish_fetch(&refresh);
                self.set_stale(refresh.id, refresh.iface, true);
            }
            Op::Owner | Op::Match => {}
        }
    }

    /// A `GetManagedObjects` completion that cannot be used (dropped,
    /// errored, or misshapen): the last state stays, retried once on
    /// the timer (unless this was the retry), then left for the next
    /// signal.
    fn fail_managed(&mut self) {
        self.managed_in_flight = false;
        self.managed_clean = false;
        self.stale_managed = true;
        if !self.managed_retry {
            self.arm_coalesce();
        } else {
            self.managed_retry = false;
        }
    }

    fn on_error(&mut self, token: u64) {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        match flight.op {
            // No owner (BlueZ is not running), or one that left between
            // the question and the answer: nothing is shown.
            Op::Owner => self.set_owner(None),
            Op::Managed => self.fail_managed(),
            Op::Props(refresh) => {
                self.finish_fetch(&refresh);
                self.set_stale(refresh.id, refresh.iface, true);
            }
            Op::Match => {
                if !self.said_match {
                    self.said_match = true;
                    crate::print::warn(format_args!(
                        "scootbar: bluetooth: the bus refused a match rule; devices may not show"
                    ));
                }
            }
        }
    }

    /// A new owner (or none): not the old one's state. Drops everything
    /// and enumerates again; in-flight answers land on nothing (ids are
    /// never reused within a connection).
    fn set_owner(&mut self, owner: Option<String>) {
        if self.owner == owner {
            return;
        }
        self.owner = owner;
        self.adapters.clear();
        self.devices.clear();
        self.managed_in_flight = false;
        self.stale_managed = false;
        self.managed_clean = true;
        self.managed_retry = false;
        self.rev = self.rev.wrapping_add(1);
        if self.owner.is_some() {
            self.read_managed();
        }
    }

    /// Replaces the set from a `GetManagedObjects` answer, bounded: past
    /// [`MAX_ADAPTERS`] adapters or [`MAX_DEVICES`] devices the rest is
    /// ignored, said once. Paths already held keep their ids (a reply in
    /// flight still lands); newcomers take new ones.
    fn replace(&mut self, objects: Vec<bluez::Object<'_>>) {
        let mut adapters = Vec::new();
        let mut devices = Vec::new();
        for object in &objects {
            if object.adapter.is_some() {
                let id = self
                    .adapters
                    .iter()
                    .find(|a| a.path == object.path)
                    .map(|a| a.id)
                    .unwrap_or_else(|| self.alloc_id());
                let powered = object
                    .adapter
                    .as_ref()
                    .and_then(|p| p.powered)
                    .unwrap_or(false);
                adapters.push(Adapter {
                    id,
                    path: object.path.to_owned(),
                    powered,
                    asked: None,
                    last_asked: None,
                    stale: false,
                });
                if adapters.len() >= MAX_ADAPTERS {
                    self.say_full("adapters");
                    break;
                }
            }
            if object.device.is_some() || object.battery.is_some() {
                if devices.len() >= MAX_DEVICES {
                    self.say_full("devices");
                    break;
                }
                let id = self
                    .devices
                    .iter()
                    .find(|d| d.path == object.path)
                    .map(|d| d.id)
                    .unwrap_or_else(|| self.alloc_id());
                let mut device = Device {
                    id,
                    path: object.path.to_owned(),
                    connected: false,
                    name: String::new(),
                    has_battery: false,
                    battery: None,
                    asked: None,
                    last_asked: None,
                    stale: false,
                };
                if let Some(props) = &object.device {
                    device.connected = props.connected.unwrap_or(false);
                    rename(&mut device.name, &mut self.scratch, props, &device.path);
                }
                if let Some(props) = &object.battery {
                    device.has_battery = true;
                    device.battery = props.percentage;
                }
                devices.push(device);
            }
        }
        self.adapters = adapters;
        self.devices = devices;
        self.rev = self.rev.wrapping_add(1);
    }

    /// Applies one added object: upserts its adapter and device halves,
    /// bounded like [`Live::replace`]. Says whether anything shown moved.
    fn upsert(&mut self, object: &bluez::Object<'_>) -> bool {
        let mut changed = false;
        if let Some(props) = &object.adapter {
            match self.adapters.iter_mut().find(|a| a.path == object.path) {
                Some(adapter) => {
                    if let Some(powered) = props.powered {
                        if adapter.powered != powered {
                            adapter.powered = powered;
                            changed = true;
                        }
                    }
                }
                None => {
                    if self.adapters.len() < MAX_ADAPTERS {
                        self.next_id += 1;
                        let id = self.next_id;
                        self.adapters.push(Adapter {
                            id,
                            path: object.path.to_owned(),
                            powered: props.powered.unwrap_or(false),
                            asked: None,
                            last_asked: None,
                            stale: false,
                        });
                        changed = true;
                    } else {
                        self.say_full("adapters");
                    }
                }
            }
        }
        if object.device.is_some() || object.battery.is_some() {
            match self.devices.iter_mut().find(|d| d.path == object.path) {
                Some(device) => {
                    if apply_device(device, &mut self.scratch, object) {
                        changed = true;
                    }
                }
                None => {
                    if self.devices.len() < MAX_DEVICES {
                        self.next_id += 1;
                        let id = self.next_id;
                        let mut device = Device {
                            id,
                            path: object.path.to_owned(),
                            connected: false,
                            name: String::new(),
                            has_battery: false,
                            battery: None,
                            asked: None,
                            last_asked: None,
                            stale: false,
                        };
                        apply_device(&mut device, &mut self.scratch, object);
                        self.devices.push(device);
                        changed = true;
                    } else {
                        self.say_full("devices");
                    }
                }
            }
        }
        if changed {
            self.rev = self.rev.wrapping_add(1);
        }
        changed
    }

    fn say_full(&mut self, what: &str) {
        if !self.said_full {
            self.said_full = true;
            crate::print::warn(format_args!(
                "scootbar: bluetooth: ignoring further {what}: no room (at most {MAX_ADAPTERS} \
                 adapters, {MAX_DEVICES} devices)"
            ));
        }
    }

    fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn on_signal(
        &mut self,
        sender: &str,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) {
        if interface == conn::BUS_INTERFACE && member == "NameOwnerChanged" {
            // Only the bus says who owns what: a peer can send this
            // signal to the bar, addressed, and it is not believed.
            if sender == conn::BUS_NAME && signature == "sss" {
                self.on_name_owner_changed(body);
            }
            return;
        }
        // Only the tracked owner speaks for BlueZ: a peer can send
        // these signals too. Property signals arrive on the object's own
        // path; the manager's arrive on `/org/bluez` with the object in
        // the body.
        if self.owner.as_deref() != Some(sender) {
            return;
        }
        if interface == MANAGER
            && member == "InterfacesAdded"
            && path == ROOT
            && signature == "oa{sa{sv}}"
        {
            if let Ok(object) = bluez::read_interfaces_added(body) {
                if is_object(object.path) {
                    self.upsert(&object);
                }
            }
        } else if interface == MANAGER
            && member == "InterfacesRemoved"
            && path == ROOT
            && signature == "oas"
        {
            if let Ok((removed, names)) = bluez::read_interfaces_removed(body) {
                if is_object(removed) {
                    self.remove(removed, &names);
                }
            }
        } else if interface == PROPERTIES
            && member == "PropertiesChanged"
            && is_object(path)
            && signature == "sa{sv}as"
        {
            self.on_properties_changed(path, body);
        }
    }

    /// Owner tracking: `org.bluez` gained an owner (enumerate it),
    /// changed hands (not the old one's state), or lost its owner (BlueZ
    /// is gone, whether it said goodbye or died: the bus releases a dead
    /// connection's names).
    fn on_name_owner_changed(&mut self, body: &[u8]) {
        let Ok((name, _, new)) = proto::read_name_owner_changed(body) else {
            return;
        };
        if name != NAME {
            return;
        }
        self.set_owner(new);
    }

    /// Drops whatever `path` lost: an adapter, a device, or a device's
    /// battery. A path that was never held is not an error (a device that
    /// vanishes mid-connect).
    fn remove(&mut self, path: &str, names: &[&str]) {
        let mut changed = false;
        if names.contains(&ADAPTER) {
            let before = self.adapters.len();
            self.adapters.retain(|a| a.path != path);
            changed |= self.adapters.len() != before;
        }
        if names.contains(&DEVICE) {
            let before = self.devices.len();
            self.devices.retain(|d| d.path != path);
            changed |= self.devices.len() != before;
        } else if names.contains(&BATTERY) {
            if let Some(device) = self.devices.iter_mut().find(|d| d.path == path) {
                if device.has_battery || device.battery.is_some() {
                    device.has_battery = false;
                    device.battery = None;
                    changed = true;
                }
            }
        }
        if changed {
            self.rev = self.rev.wrapping_add(1);
        }
    }

    /// A device's or adapter's properties moved: applied from the tracked
    /// owner, and only from it. A signal for a path never held re-reads
    /// the set once (a change that raced the enumeration), instead of
    /// trusting it.
    fn on_properties_changed(&mut self, path: &str, body: &[u8]) {
        let Ok(changed) = bluez::read_properties_changed(body) else {
            return;
        };
        match changed.iface {
            bluez::Iface::Adapter => {
                let Some(index) = self.adapters.iter().position(|a| a.path == path) else {
                    self.restale();
                    return;
                };
                if let Some(powered) = changed.adapter.powered {
                    if self.adapters[index].powered != powered {
                        self.adapters[index].powered = powered;
                        self.rev = self.rev.wrapping_add(1);
                    }
                }
                if changed.invalidated {
                    let id = self.adapters[index].id;
                    self.refresh(id, RefreshIface::Adapter);
                }
            }
            bluez::Iface::Device => {
                let Some(index) = self.devices.iter().position(|d| d.path == path) else {
                    self.restale();
                    return;
                };
                let id = self.devices[index].id;
                let mut moved = false;
                if let Some(connected) = changed.device.connected {
                    if self.devices[index].connected != connected {
                        self.devices[index].connected = connected;
                        moved = true;
                    }
                }
                if changed.device.name.is_some() || changed.device.alias.is_some() {
                    let path = self.devices[index].path.clone();
                    moved |= rename(
                        &mut self.devices[index].name,
                        &mut self.scratch,
                        &changed.device,
                        &path,
                    );
                }
                if changed.invalidated {
                    self.refresh(id, RefreshIface::Device);
                }
                if moved {
                    self.rev = self.rev.wrapping_add(1);
                }
            }
            bluez::Iface::Battery => {
                let Some(index) = self.devices.iter().position(|d| d.path == path) else {
                    self.restale();
                    return;
                };
                let id = self.devices[index].id;
                let device = &mut self.devices[index];
                let mut moved = false;
                if !device.has_battery {
                    device.has_battery = true;
                    moved = true;
                }
                if let Some(percentage) = changed.battery.percentage {
                    if device.battery != Some(percentage) {
                        device.battery = Some(percentage);
                        moved = true;
                    }
                }
                if changed.invalidated {
                    self.refresh(id, RefreshIface::Battery);
                }
                if moved {
                    self.rev = self.rev.wrapping_add(1);
                }
            }
            bluez::Iface::Other => {}
        }
    }

    /// A signal for a path never held: the enumeration is behind, so
    /// read it again on the coalesce timer (one re-read however many
    /// such signals arrive), or in [`Live::apply`] when one is already
    /// in flight and completes.
    fn restale(&mut self) {
        self.stale_managed = true;
        if !self.managed_in_flight {
            self.arm_coalesce();
        }
    }

    /// An answer to an object's `GetAll`: applied, or dropped whole when
    /// it does not parse (the object keeps its last state). A reply for
    /// an object that left finds nothing.
    fn on_props(&mut self, refresh: &Refresh, signature: &str, body: &[u8]) {
        if signature != "a{sv}" {
            return;
        }
        match refresh.iface {
            RefreshIface::Adapter => {
                let Ok(props) = bluez::read_adapter_all(body) else {
                    return;
                };
                if let Some(adapter) = self.adapters.iter_mut().find(|a| a.id == refresh.id) {
                    if let Some(powered) = props.powered {
                        if adapter.powered != powered {
                            adapter.powered = powered;
                            self.rev = self.rev.wrapping_add(1);
                        }
                    }
                }
            }
            RefreshIface::Device => {
                let Ok(props) = bluez::read_device_all(body) else {
                    return;
                };
                let Some(index) = self.devices.iter().position(|d| d.id == refresh.id) else {
                    return;
                };
                let mut moved = false;
                if let Some(connected) = props.connected {
                    if self.devices[index].connected != connected {
                        self.devices[index].connected = connected;
                        moved = true;
                    }
                }
                if props.name.is_some() || props.alias.is_some() {
                    let path = self.devices[index].path.clone();
                    let props = bluez::DeviceProps {
                        connected: None,
                        name: props.name,
                        alias: props.alias,
                    };
                    moved |= rename(
                        &mut self.devices[index].name,
                        &mut self.scratch,
                        &props,
                        &path,
                    );
                }
                if moved {
                    self.rev = self.rev.wrapping_add(1);
                }
            }
            RefreshIface::Battery => {
                let Ok(props) = bluez::read_battery_all(body) else {
                    return;
                };
                if let Some(device) = self.devices.iter_mut().find(|d| d.id == refresh.id) {
                    device.has_battery = true;
                    if device.battery != props.percentage {
                        device.battery = props.percentage;
                        self.rev = self.rev.wrapping_add(1);
                    }
                }
            }
        }
    }

    /// The call is answered: it may be asked again, and whether a signal
    /// arrived meanwhile.
    fn finish_fetch(&mut self, refresh: &Refresh) -> bool {
        let stale = match refresh.iface {
            RefreshIface::Adapter => self
                .adapters
                .iter_mut()
                .find(|a| a.id == refresh.id)
                .map(|a| (a.asked.take().is_some(), std::mem::take(&mut a.stale))),
            RefreshIface::Device | RefreshIface::Battery => self
                .devices
                .iter_mut()
                .find(|d| d.id == refresh.id)
                .map(|d| (d.asked.take().is_some(), std::mem::take(&mut d.stale))),
        };
        stale.is_some_and(|(was_asked, stale)| was_asked && stale)
    }

    fn exists(&self, id: u64, iface: RefreshIface) -> bool {
        match iface {
            RefreshIface::Adapter => self.adapters.iter().any(|a| a.id == id),
            RefreshIface::Device | RefreshIface::Battery => self.devices.iter().any(|d| d.id == id),
        }
    }

    fn fetch_times(
        &self,
        id: u64,
        iface: RefreshIface,
    ) -> Option<(Option<Instant>, Option<Instant>)> {
        match iface {
            RefreshIface::Adapter => self
                .adapters
                .iter()
                .find(|a| a.id == id)
                .map(|a| (a.asked, a.last_asked)),
            RefreshIface::Device | RefreshIface::Battery => self
                .devices
                .iter()
                .find(|d| d.id == id)
                .map(|d| (d.asked, d.last_asked)),
        }
    }

    fn last_asked(&self, id: u64, iface: RefreshIface) -> Option<Instant> {
        self.fetch_times(id, iface).and_then(|(_, last)| last)
    }

    fn set_stale(&mut self, id: u64, iface: RefreshIface, stale: bool) {
        match iface {
            RefreshIface::Adapter => {
                if let Some(adapter) = self.adapters.iter_mut().find(|a| a.id == id) {
                    adapter.stale = stale;
                }
            }
            RefreshIface::Device | RefreshIface::Battery => {
                if let Some(device) = self.devices.iter_mut().find(|d| d.id == id) {
                    device.stale = stale;
                }
            }
        }
    }

    fn set_fetch_times(
        &mut self,
        id: u64,
        iface: RefreshIface,
        asked: Option<Instant>,
        last: Option<Instant>,
    ) {
        match iface {
            RefreshIface::Adapter => {
                if let Some(adapter) = self.adapters.iter_mut().find(|a| a.id == id) {
                    adapter.asked = asked;
                    adapter.last_asked = last;
                }
            }
            RefreshIface::Device | RefreshIface::Battery => {
                if let Some(device) = self.devices.iter_mut().find(|d| d.id == id) {
                    device.asked = asked;
                    device.last_asked = last;
                }
            }
        }
    }
}

/// A cleaned device name is at most this many bytes: the view bounds the
/// line it makes of it again ([`crate::modules::MAX_TEXT`]).
pub(super) const MAX_FIELD: usize = 120;

/// Cleans `props` (`Name`, else `Alias`, else the path's last element)
/// into `name` (reusing its buffer): says whether anything shown moved.
fn rename(
    name: &mut String,
    scratch: &mut String,
    props: &bluez::DeviceProps<'_>,
    path: &str,
) -> bool {
    scratch.clear();
    let text = props
        .name
        .or(props.alias)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(""));
    push_clean(scratch, text, MAX_FIELD);
    if *scratch == *name {
        scratch.clear();
        return false;
    }
    std::mem::swap(scratch, name);
    scratch.clear();
    true
}

/// Applies an added object's device half to `device`: says whether
/// anything shown moved.
fn apply_device(device: &mut Device, scratch: &mut String, object: &bluez::Object<'_>) -> bool {
    let mut moved = false;
    if let Some(props) = &object.device {
        if let Some(connected) = props.connected {
            if device.connected != connected {
                device.connected = connected;
                moved = true;
            }
        }
        // A `Name` that BlueZ stops sending falls back to the `Alias`,
        // and one with neither to the path: recompute whenever either is
        // present (or nothing is shown yet), so a removal is not stuck
        // showing the old one.
        if props.name.is_some() || props.alias.is_some() || device.name.is_empty() {
            let path = device.path.clone();
            moved |= rename(&mut device.name, scratch, props, &path);
        }
    }
    if let Some(props) = &object.battery {
        if !device.has_battery {
            device.has_battery = true;
            moved = true;
        }
        if device.battery != props.percentage {
            device.battery = props.percentage;
            moved = true;
        }
    }
    moved
}

/// Appends `text` without control characters, at most `room` bytes, cut
/// on a character boundary. Reads no further than it keeps (and the
/// controls it drops), so a kilobyte name costs a bounded walk, not a
/// kilobyte of copy.
fn push_clean(out: &mut String, text: &str, room: usize) {
    let end = out.len() + room;
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > end {
            break;
        }
        out.push(c);
    }
}
