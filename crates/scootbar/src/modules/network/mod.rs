//! The network module: link state, WiFi name and signal, click to pick.
//!
//! Two netlink sockets on the bar's `poll` loop, no daemon and no child
//! process: `NETLINK_ROUTE` for links, addresses and the default route,
//! and `NETLINK_GENERIC` for nl80211 (which interfaces are wireless, the
//! associated SSID, the signal, the cached scan). Decided by measurement
//! against the network daemon's D-Bus (see the ticket's resolution): the
//! D-Bus wakes on every background scan's property storm for every visible
//! access point, needs a client the bar will not have until M6's shared
//! one, and is absent wherever NetworkManager is; nl80211 answers under
//! every daemon and none, and its multicast is silent unless the radio
//! roams or disconnects.
//!
//! ## States
//!
//! `Offline` shows `offline` (class `warn`); `Eth` shows the interface
//! name; `Wifi` shows the SSID and bars (`Wimbly ▂▄▆`); `Vpn` shows `VPN`.
//! A second VPN up beside the shown interface appends `· VPN`. Which
//! interface is shown is the config's `interface`, else the default
//! route's (v4 before v6), tracked by index so a rename keeps it; with
//! neither, the module is `Offline`.
//!
//! ## Signal
//!
//! Signal strength has no event where CQM thresholds are unsupported
//! (measured: brcmfmac refuses `SET_CQM`, see the live test), so a
//! `timerfd` re-asks the station's signal every [`SIGNAL_SECS`] seconds,
//! armed only while a WiFi network is shown. Connect, disconnect and roam
//! are events (rtnetlink for the first two, the `mlme` multicast where the
//! kernel lets the socket join it, else the next signal tick), so the
//! timer never decides them.
//!
//! ## The picker
//!
//! Click (the `menu` action) spawns `menu-command` with the cached scan's
//! SSIDs on stdin — a dmenu-style launcher, fed from the scan list — and
//! reaps it by pidfd. Connecting is the command's own business (the docs
//! show the `nmcli` wrapping); native popups replace it in M6, which is
//! why the bar never reads the choice back.
//!
//! ## Shape
//!
//! [`Nets`] is pure state: every notice is applied as it arrives and any
//! owed request is queued into an outbox, with no I/O. [`Network`] owns
//! the fds and flushes the outboxes after each drain. Dumps are idempotent
//! state, so no reply sequence is matched; an overrun or any socket error
//! resyncs from fresh dumps.
//!
//! ## Fixed bounds, bounded allocation
//!
//! At most [`MAX_IFACES`] interfaces and [`MAX_SCAN`] scan entries, in
//! fixed arrays; SSIDs and names are sanitized once, at parse time, and
//! the view only reads. The outboxes hold a few hundred bytes of dump
//! requests; an association re-spells one short `String`, and the one
//! larger allocation on the invoke path is the scan list the menu is
//! spawned with.

use std::fmt::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};

use rustix::event::PollFlags;
use rustix::io::Errno;
use rustix::net::netlink::{GENERIC, SocketAddrNetlink};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, bind, recv, send, socket_with,
    socketpair,
};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};

pub mod netlink;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod fuzz;
#[cfg(test)]
mod tests;

use netlink::{
    AddrEvent, Bss, Family, Link, READ_LEN, RTMGRP_IPV4_IFADDR, RTMGRP_IPV4_ROUTE,
    RTMGRP_IPV6_IFADDR, RTMGRP_IPV6_ROUTE, RTMGRP_LINK, RouteEvent, Wireless,
};

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "network";

/// The one action a binding or an agent may name: open the picker.
pub const ACTIONS: &[ActionSpec] = &[ActionSpec {
    name: "menu",
    arg: ArgKind::None,
}];

/// Seconds between signal re-reads while a WiFi network is shown. The
/// published rate: one wakeup per interval then, none otherwise.
pub const SIGNAL_SECS: u64 = 10;

/// Seconds between reopen attempts after both sockets failed. Only armed
/// in the failure state: the steady state has no timer for this.
const RETRY_SECS: u64 = 5;

/// The most interfaces tracked and the most scan entries kept. A machine
/// with more interfaces still shows one of them; a denser street still
/// fills the picker.
const MAX_IFACES: usize = 32;
const MAX_SCAN: usize = 32;

/// Consecutive failures of the in-flight dump after which it is dropped
/// rather than re-queued: a dump that fails this often is not transient
/// (a busy kernel refuses transiently), and re-queueing it on every error
/// reply would spin at the kernel's answer rate. Later events re-queue
/// fresh dumps, so the drop heals.
const MAX_DUMP_FAILURES: u8 = 3;

/// The module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The interface to show, by name. None is the default route's.
    pub interface: Option<String>,
    /// Whether the SSID is shown (and reported to `query`). The bar is
    /// visible in screenshots and to an agent's `query`; `false` shows
    /// `WiFi` and the bars instead.
    pub show_ssid: bool,
    /// The picker: spawned with the scan's SSIDs on stdin. Empty runs
    /// nothing, and the click is refused saying so.
    pub menu_command: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interface: None,
            show_ssid: true,
            menu_command: Vec::new(),
        }
    }
}

#[cfg(feature = "network")]
pub fn init(settings: &super::Settings) -> Init {
    match open(&settings.network) {
        Ok(network) => {
            if network.nets.has_usable() {
                Init::Available(Box::new(network))
            } else {
                Init::Unavailable("no network interfaces".to_owned())
            }
        }
        Err(why) => Init::Unavailable(why),
    }
}

/// Starts the module as if its probe had found interfaces: the contract
/// test's way in on a machine without any (there is always at least `lo`
/// where the tests run, so this is the same module, unrefused).
#[cfg(test)]
pub fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    match open(&settings.network) {
        Ok(network) => Box::new(network),
        Err(_) => {
            let (rt, genl) = fake::pair();
            start_with(&settings.network, rt, genl, 0)
        }
    }
}

/// A dump request waiting for its turn: the kernel runs one dump per
/// socket, answering any other with `-EBUSY`, so dumps queue here and go
/// out one at a time, each on its predecessor's `DONE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dump {
    Links,
    Addrs(u8),
    Routes(u8),
}

/// A generic-netlink dump waiting for its turn (same one-at-a-time rule
/// as above). Single queries (one interface, one station) are not dumps
/// and go out at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenlDump {
    Interfaces,
    Scan(u32),
    Station(u32),
}

/// One tracked interface: the kernel's link, address presence, and what
/// nl80211 said about it.
struct Iface {
    link: Link,
    has_v4: bool,
    has_v6: bool,
    /// Known wireless (an nl80211 interface notice or dump entry).
    wireless: bool,
    /// Already asked nl80211 about this index; reset by a resync, so a
    /// refusal is not re-asked on every link event.
    queried: bool,
    /// The associated SSID, sanitized at parse time; empty is unknown.
    ssid: String,
    /// The peer's signal in dBm, from the last station reply.
    signal: Option<i8>,
}

/// What is shown: one interface's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Offline,
    Eth,
    Wifi,
    Vpn,
}

/// What the view is drawn from: a roam changes the SSID without changing
/// the state, and the bar must still redraw, so `on_ready` compares one
/// of these, not the state alone. Fixed arrays, no allocation.
#[derive(PartialEq, Eq)]
struct Fingerprint {
    state: State,
    name: [u8; 16],
    name_len: u8,
    ssid: [u8; 32],
    ssid_len: u8,
    signal: Option<i8>,
    vpn: bool,
    seen: bool,
}

/// The pure state: interfaces, routes, WiFi, and the owed requests. No
/// fd, no I/O; [`Network`] moves the bytes.
#[derive(Default)]
struct Nets {
    ifaces: Vec<Iface>,
    default4: u32,
    default6: u32,
    family: Family,
    /// Dumps waiting for the rtnetlink socket, and the one in flight:
    /// the kernel runs one dump per socket.
    rt_dumps: std::collections::VecDeque<Dump>,
    rt_busy: Option<(u32, Dump)>,
    /// The same for the generic socket.
    genl_dumps: std::collections::VecDeque<GenlDump>,
    genl_busy: Option<(u32, GenlDump)>,
    /// Consecutive failures of the in-flight dump, per socket (see
    /// [`MAX_DUMP_FAILURES`]).
    rt_failures: u8,
    genl_failures: u8,
    /// Whether both the `scan` and `mlme` multicast groups joined (see
    /// [`join_mask`]): where either id did not fit the bind mask, roam
    /// arrives with the signal refresh instead.
    joined: bool,
    /// Whether any datagram arrived yet: before the first one the view
    /// is empty (like the volume module before its first read), not
    /// `offline`.
    seen: bool,
    scan: [Bss; MAX_SCAN],
    scan_n: usize,
    scan_of: u32,
    station_of: u32,
    seq: u32,
    out_rt: Vec<u8>,
    out_genl: Vec<u8>,
}

impl Nets {
    /// Request sequence numbers: the kernel echoes them, but nothing
    /// matches them (dumps are idempotent state); they only need to be
    /// nonzero.
    fn next_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.seq = 1;
        }
        self.seq
    }

    fn find(&mut self, index: u32) -> Option<&mut Iface> {
        self.ifaces
            .iter_mut()
            .find(|iface| iface.link.index == index)
    }

    fn has_usable(&self) -> bool {
        self.ifaces
            .iter()
            .any(|iface| !iface.link.is_loopback() && !iface.link.is_empty())
    }

    /// The interface the view shows: the config's by name, else the
    /// default route's by index (v4 before v6).
    fn selected(&self, interface: Option<&str>) -> Option<&Iface> {
        if let Some(name) = interface {
            return self
                .ifaces
                .iter()
                .find(|iface| iface.link.name_str() == name);
        }
        [self.default4, self.default6]
            .into_iter()
            .find_map(|index| self.ifaces.iter().find(|iface| iface.link.index == index))
    }

    fn state(&self, interface: Option<&str>) -> State {
        let Some(iface) = self.selected(interface) else {
            return State::Offline;
        };
        if !iface.link.is_up() || iface.link.is_loopback() || !(iface.has_v4 || iface.has_v6) {
            return State::Offline;
        }
        if iface.wireless {
            if iface.ssid.is_empty() {
                return State::Offline;
            }
            return State::Wifi;
        }
        if iface.link.is_tunnel() {
            return State::Vpn;
        }
        State::Eth
    }

    /// Whether any interface but `except` is an up tunnel with an
    /// address: the `· VPN` marker.
    fn other_vpn(&self, except: u32) -> bool {
        self.ifaces.iter().any(|iface| {
            iface.link.index != except
                && iface.link.is_up()
                && !iface.link.is_loopback()
                && (iface.has_v4 || iface.has_v6)
                && iface.link.is_tunnel()
        })
    }

    /// Applies a link notice: upserts by index (a rename is the same
    /// index with a new name), or drops the entry.
    fn on_link(&mut self, link: &Link, present: bool) {
        if !present {
            self.ifaces.retain(|iface| iface.link.index != link.index);
            if self.scan_of == link.index {
                self.scan_n = 0;
            }
            return;
        }
        if link.is_empty() || link.is_loopback() {
            return;
        }
        match self.find(link.index) {
            Some(iface) => {
                iface.link = link.clone();
            }
            None => {
                if self.ifaces.len() >= MAX_IFACES {
                    return;
                }
                self.ifaces.push(Iface {
                    link: link.clone(),
                    has_v4: false,
                    has_v6: false,
                    wireless: false,
                    queried: false,
                    ssid: String::new(),
                    signal: None,
                });
            }
        }
        self.query_wifi(link.index);
    }

    /// Queues a scan dump for `index`, forgetting the previous list: the
    /// replies append to an empty list, whatever order the test feeds
    /// them in, and on the wire the request always precedes its replies.
    fn queue_scan(&mut self, index: u32) {
        self.scan_of = index;
        self.scan_n = 0;
        self.genl_dumps.push_back(GenlDump::Scan(index));
    }

    /// Re-asks nl80211 about one interface: a single query, not a dump,
    /// so it goes out at once. Roam and associate carry no SSID, and the
    /// scan cache may predate them; the interface info names the current
    /// network authoritatively.
    fn refresh_interface(&mut self, index: u32) {
        if self.family.id == 0 || index == 0 {
            return;
        }
        let seq = self.next_seq();
        netlink::interface_request(&mut self.out_genl, self.family.id, seq, index);
    }

    /// Queues an nl80211 interface query for `index`, once per resync: a
    /// dump at sync covers the interfaces already there, so this is only
    /// for newcomers.
    fn query_wifi(&mut self, index: u32) {
        if self.family.id == 0 {
            return;
        }
        let fresh = self.find(index).is_some_and(|iface| !iface.queried);
        if !fresh {
            return;
        }
        if let Some(iface) = self.find(index) {
            iface.queried = true;
        }
        let seq = self.next_seq();
        netlink::interface_request(&mut self.out_genl, self.family.id, seq, index);
    }

    fn on_addr(&mut self, event: &AddrEvent, present: bool) {
        let Some(iface) = self.find(event.index) else {
            return;
        };
        match event.family {
            2 => iface.has_v4 = present,
            10 => iface.has_v6 = present,
            _ => {}
        }
    }

    fn on_route(&mut self, event: &RouteEvent) {
        let slot = match event.family {
            2 => &mut self.default4,
            10 => &mut self.default6,
            _ => return,
        };
        if event.present {
            *slot = event.oif;
        } else if *slot == event.oif {
            *slot = 0;
        }
    }

    fn on_wireless(&mut self, wifi: &Wireless, present: bool) {
        if wifi.index == 0 {
            return;
        }
        if !present {
            if let Some(iface) = self.find(wifi.index) {
                iface.wireless = false;
                iface.ssid.clear();
                iface.signal = None;
            }
            if self.scan_of == wifi.index {
                self.scan_n = 0;
            }
            return;
        }
        let fresh = self.find(wifi.index).is_some_and(|iface| !iface.wireless);
        if let Some(iface) = self.find(wifi.index) {
            iface.queried = true;
            iface.wireless = true;
            // The interface info carries the current SSID while
            // associated (absent on a fresh interface, which leaves what
            // the scan said).
            if wifi.ssid_len > 0 {
                iface.ssid = netlink::sanitize(&wifi.ssid[..wifi.ssid_len as usize]);
            }
        } else {
            return;
        }
        if fresh {
            self.station_of = wifi.index;
            self.queue_scan(wifi.index);
            self.genl_dumps.push_back(GenlDump::Station(wifi.index));
        }
    }
    /// Applies one rtnetlink datagram's messages. Dumps are idempotent
    /// state, so no reply sequence is matched except the in-flight
    /// dump's, which its `DONE` releases for the next one in line.
    fn on_rt_datagram(&mut self, bytes: &[u8]) {
        for msg in netlink::messages(bytes) {
            if msg.kind == netlink::NLMSG_DONE {
                self.done_rt(msg.seq);
                continue;
            }
            if msg.kind == netlink::NLMSG_ERROR {
                match netlink::error_of(msg.body) {
                    None => {}
                    Some(_) => self.redump_rt(msg.seq),
                }
                continue;
            }
            match msg.kind {
                netlink::RTM_NEWLINK | netlink::RTM_DELLINK => {
                    let mut link = Link::default();
                    netlink::parse_link(msg.body, &mut link);
                    if link.index != 0 {
                        self.on_link(&link, msg.kind == netlink::RTM_NEWLINK);
                    }
                }
                netlink::RTM_NEWADDR | netlink::RTM_DELADDR => {
                    if let Some(event) = netlink::parse_addr(msg.body) {
                        self.on_addr(&event, msg.kind == netlink::RTM_NEWADDR);
                    }
                }
                netlink::RTM_NEWROUTE | netlink::RTM_DELROUTE => {
                    if let Some(event) =
                        netlink::parse_route(msg.body, msg.kind == netlink::RTM_NEWROUTE)
                    {
                        self.on_route(&event);
                    }
                }
                _ => {}
            }
        }
    }

    /// Applies one generic-netlink datagram's messages. Notices and dump
    /// replies are applied as they arrive; a failed dump goes back to the
    /// queue's end (a busy kernel refuses dumps transiently; the replays
    /// are idempotent, so nothing is lost or doubled), except a refused
    /// one, which would fail again.
    fn on_genl_datagram(&mut self, bytes: &[u8]) {
        for msg in netlink::messages(bytes) {
            if msg.kind == netlink::NLMSG_ERROR {
                match netlink::error_of(msg.body) {
                    None => {}
                    Some(-1) => self.drop_genl(msg.seq),
                    Some(_) => self.redump_genl(msg.seq),
                }
                continue;
            }
            if msg.kind == netlink::NLMSG_DONE {
                self.done_genl(msg.seq);
                continue;
            }
            if msg.kind != self.family.id || self.family.id == 0 {
                continue;
            }
            let Some((command, attrs)) = netlink::genl_of(msg.body) else {
                continue;
            };
            match command {
                netlink::NL80211_CMD_NEW_INTERFACE => {
                    let wifi = netlink::parse_interface(msg.body);
                    self.on_wireless(&wifi, true);
                }
                netlink::NL80211_CMD_DEL_INTERFACE => {
                    let wifi = netlink::parse_interface(msg.body);
                    self.on_wireless(&wifi, false);
                }
                netlink::NL80211_CMD_ASSOCIATE | netlink::NL80211_CMD_ROAM => {
                    let index = netlink::find_u32(attrs, netlink::ATTR_IFINDEX);
                    if index != 0 {
                        self.station_of = index;
                        self.refresh_interface(index);
                        self.queue_scan(index);
                        self.genl_dumps.push_back(GenlDump::Station(index));
                    }
                }
                netlink::NL80211_CMD_DEAUTHENTICATE | netlink::NL80211_CMD_DISASSOCIATE => {
                    let index = netlink::find_u32(attrs, netlink::ATTR_IFINDEX);
                    if let Some(iface) = self.find(index) {
                        iface.ssid.clear();
                        iface.signal = None;
                    }
                }
                netlink::NL80211_CMD_NEW_SCAN_RESULTS => {
                    // A bare notice (sequence zero) says a scan finished:
                    // re-dump. A dump reply (our own sequence) carries the
                    // entries instead, and must not re-queue, or every
                    // reply schedules another dump without end.
                    if msg.seq == 0 {
                        if self.scan_of != 0 {
                            let of = self.scan_of;
                            self.queue_scan(of);
                        }
                    } else {
                        // Pages append; the next dump resets (see
                        // `pump_genl`).
                        let mut fresh = [Bss::default(); MAX_SCAN];
                        let n = netlink::fold_scan(msg.body, &mut fresh);
                        for bss in fresh.iter().take(n) {
                            if self.scan_n < MAX_SCAN {
                                self.scan[self.scan_n] = *bss;
                                self.scan_n += 1;
                            }
                        }
                        self.apply_scan();
                    }
                }
                netlink::NL80211_CMD_NEW_STATION => {
                    if let Some(signal) = netlink::parse_station(msg.body) {
                        let of = self.station_of;
                        if let Some(iface) = self.find(of) {
                            iface.signal = Some(signal);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Folds the scan cache into the interfaces: the associated BSS names
    /// the SSID of the scanned interface, and lends its signal until the
    /// station reply (queued beside the scan) confirms it.
    fn apply_scan(&mut self) {
        let of = self.scan_of;
        let (ssid, signal) = match self.scan[..self.scan_n]
            .iter()
            .find(|bss| bss.associated && bss.ssid_len > 0)
        {
            Some(bss) => (
                netlink::sanitize(&bss.ssid[..bss.ssid_len as usize]),
                bss.signal,
            ),
            None => return,
        };
        if let Some(iface) = self.find(of) {
            iface.ssid = ssid;
            // The scan's signal for the associated BSS, until the station
            // reply (queued beside the scan) confirms it.
            if signal.is_some() {
                iface.signal = signal;
            }
        }
    }

    /// The in-flight rtnetlink dump finished: the next queued one may go.
    fn done_rt(&mut self, seq: u32) {
        if self.rt_busy.is_some_and(|(busy, _)| busy == seq) {
            self.rt_busy = None;
            self.rt_failures = 0;
        }
    }

    /// The in-flight rtnetlink dump failed: back to the queue's end,
    /// except a refused one, which would fail again — and except one
    /// that has failed too often, which is dropped (see
    /// [`MAX_DUMP_FAILURES`]).
    fn redump_rt(&mut self, seq: u32) {
        if let Some((busy, dump)) = self.rt_busy {
            if busy == seq {
                self.rt_busy = None;
                self.rt_failures += 1;
                if self.rt_failures >= MAX_DUMP_FAILURES {
                    self.rt_failures = 0;
                } else {
                    self.rt_dumps.push_back(dump);
                }
            }
        }
    }

    /// The in-flight generic dump finished.
    fn done_genl(&mut self, seq: u32) {
        if self.genl_busy.is_some_and(|(busy, _)| busy == seq) {
            self.genl_busy = None;
            self.genl_failures = 0;
        }
    }

    /// The in-flight generic dump failed: refused ones are dropped (they
    /// would fail again), the rest re-queued — until they fail too often
    /// (see [`MAX_DUMP_FAILURES`]).
    fn redump_genl(&mut self, seq: u32) {
        if let Some((busy, dump)) = self.genl_busy {
            if busy == seq {
                self.genl_busy = None;
                self.genl_failures += 1;
                if self.genl_failures >= MAX_DUMP_FAILURES {
                    self.genl_failures = 0;
                } else {
                    self.genl_dumps.push_back(dump);
                }
            }
        }
    }

    /// A refused generic dump is dropped, not re-queued.
    fn drop_genl(&mut self, seq: u32) {
        if self.genl_busy.is_some_and(|(busy, _)| busy == seq) {
            self.genl_busy = None;
            self.genl_failures = 0;
        }
    }

    /// The scan list for the picker's stdin: one sanitized SSID per line,
    /// unnamed networks skipped, capped at what fits a pipe write.
    fn scan_list(&self) -> Vec<u8> {
        let mut list = Vec::new();
        for bss in &self.scan[..self.scan_n] {
            if bss.ssid_len == 0 || list.len() >= 2048 {
                continue;
            }
            let line = netlink::sanitize(&bss.ssid[..bss.ssid_len as usize]);
            if line.is_empty() {
                continue;
            }
            list.extend_from_slice(line.as_bytes());
            list.push(b'\n');
        }
        list
    }
}

/// One polled socket: the fd and its read buffer.
struct Sock {
    fd: OwnedFd,
    buf: [u8; READ_LEN],
}

/// A running menu picker, for the reap.
struct Menu {
    child: Child,
    pidfd: Option<OwnedFd>,
}

/// Which source `sources` added at `index`: the two sockets, then the
/// signal timer while armed, then the menu's pidfd while one runs, then
/// the reopen timer while the sockets are gone.
enum Source {
    Rt,
    Genl,
    Timer,
    Menu,
    Retry,
}

pub struct Network {
    interface: Option<String>,
    show_ssid: bool,
    menu_command: Vec<String>,
    nets: Nets,
    rt: Option<Sock>,
    genl: Option<Sock>,
    timer: Option<OwnedFd>,
    timer_for: u32,
    retry: Option<OwnedFd>,
    menu: Option<Menu>,
    /// Whether the degraded-WiFi warning was said: once per process, not
    /// per resync.
    said_degraded: bool,
}

impl Network {
    fn source_at(&self, index: usize) -> Option<Source> {
        let mut at = index;
        if self.rt.is_some() {
            if at == 0 {
                return Some(Source::Rt);
            }
            at -= 1;
        }
        if self.genl.is_some() {
            if at == 0 {
                return Some(Source::Genl);
            }
            at -= 1;
        }
        if self.timer.is_some() {
            if at == 0 {
                return Some(Source::Timer);
            }
            at -= 1;
        }
        if self.menu.as_ref().is_some_and(|menu| menu.pidfd.is_some()) {
            if at == 0 {
                return Some(Source::Menu);
            }
            at -= 1;
        }
        if self.retry.is_some() && at == 0 {
            return Some(Source::Retry);
        }
        None
    }

    fn state(&self) -> State {
        self.nets.state(self.interface.as_deref())
    }

    /// What [`Module::view`] would draw: compared before and after each
    /// ready turn.
    fn fingerprint(&self) -> Fingerprint {
        let mut print = Fingerprint {
            state: self.state(),
            name: [0; 16],
            name_len: 0,
            ssid: [0; 32],
            ssid_len: 0,
            signal: None,
            vpn: false,
            seen: self.nets.seen,
        };
        if let Some(iface) = self.nets.selected(self.interface.as_deref()) {
            let name = iface.link.name_str().as_bytes();
            let take = name.len().min(print.name.len());
            print.name[..take].copy_from_slice(&name[..take]);
            print.name_len = take as u8;
            let ssid = iface.ssid.as_bytes();
            let take = ssid.len().min(print.ssid.len());
            print.ssid[..take].copy_from_slice(&ssid[..take]);
            print.ssid_len = take as u8;
            print.signal = iface.signal;
            print.vpn = self.nets.other_vpn(iface.link.index);
        }
        print
    }

    /// Sends what the state owes: the single queries first, then one dump
    /// per socket while none is in flight. A failed send resyncs; the
    /// dumps it carried are re-queued there.
    fn flush(&mut self) {
        if let Some(sock) = self.rt.as_ref() {
            if !flush_out(&sock.fd, &mut self.nets.out_rt) {
                self.resync();
                return;
            }
        }
        if let Some(sock) = self.genl.as_ref() {
            if !flush_out(&sock.fd, &mut self.nets.out_genl) {
                self.resync();
                return;
            }
        }
        self.pump_rt();
        self.pump_genl();
    }

    /// Sends the next queued rtnetlink dump, if none is in flight. A
    /// send that does not go re-queues without resyncing: the next flush
    /// retries, and the retry timer is the backstop.
    fn pump_rt(&mut self) {
        if self.nets.rt_busy.is_some() {
            return;
        }
        let Some(dump) = self.nets.rt_dumps.pop_front() else {
            return;
        };
        let Some(sock) = self.rt.as_ref() else {
            self.nets.rt_dumps.push_front(dump);
            return;
        };
        let seq = self.nets.next_seq();
        let mut out = Vec::new();
        match dump {
            Dump::Links => netlink::link_dump(&mut out, seq),
            Dump::Addrs(family) => netlink::addr_dump(&mut out, seq, family),
            Dump::Routes(family) => netlink::route_dump(&mut out, seq, family),
        }
        match send(&sock.fd, &out, SendFlags::empty()) {
            Ok(_) => self.nets.rt_busy = Some((seq, dump)),
            Err(_) => self.nets.rt_dumps.push_front(dump),
        }
    }

    /// Sends the next queued generic dump, if none is in flight.
    fn pump_genl(&mut self) {
        if self.nets.genl_busy.is_some() {
            return;
        }
        let Some(dump) = self.nets.genl_dumps.pop_front() else {
            return;
        };
        let (Some(sock), id) = (self.genl.as_ref(), self.nets.family.id) else {
            self.nets.genl_dumps.push_front(dump);
            return;
        };
        if id == 0 {
            self.nets.genl_dumps.push_front(dump);
            return;
        }
        let seq = self.nets.next_seq();
        let mut out = Vec::new();
        match dump {
            GenlDump::Interfaces => netlink::interface_request(&mut out, id, seq, 0),
            GenlDump::Scan(index) => netlink::scan_request(&mut out, id, seq, index),
            GenlDump::Station(index) => netlink::station_request(&mut out, id, seq, index),
        }
        match send(&sock.fd, &out, SendFlags::empty()) {
            Ok(_) => self.nets.genl_busy = Some((seq, dump)),
            Err(_) => self.nets.genl_dumps.push_front(dump),
        }
    }

    /// Drains one socket: at most [`netlink::MAX_DATAGRAMS`] datagrams a
    /// turn, so a flapping link coalesces into one derivation. An overrun
    /// or any error resyncs.
    fn drain_rt(&mut self) {
        for _ in 0..netlink::MAX_DATAGRAMS {
            let n = match self.rt.as_mut() {
                Some(sock) => match recv(&sock.fd, &mut sock.buf, RecvFlags::empty()) {
                    Ok((_, n)) => n,
                    Err(Errno::AGAIN) => break,
                    Err(_) => {
                        self.resync();
                        break;
                    }
                },
                None => break,
            };
            if n == 0 {
                self.resync();
                break;
            }
            self.nets.seen = true;
            // The borrow of the socket ends before the state is touched:
            // `Nets` and the sockets are disjoint fields.
            let Some(sock) = self.rt.as_ref() else {
                break;
            };
            self.nets.on_rt_datagram(&sock.buf[..n]);
        }
        self.flush();
    }

    fn drain_genl(&mut self) {
        for _ in 0..netlink::MAX_DATAGRAMS {
            let n = match self.genl.as_mut() {
                Some(sock) => match recv(&sock.fd, &mut sock.buf, RecvFlags::empty()) {
                    Ok((_, n)) => n,
                    Err(Errno::AGAIN) => break,
                    Err(_) => {
                        self.resync();
                        break;
                    }
                },
                None => break,
            };
            if n == 0 {
                self.resync();
                break;
            }
            self.nets.seen = true;
            let Some(sock) = self.genl.as_ref() else {
                break;
            };
            self.nets.on_genl_datagram(&sock.buf[..n]);
        }
        self.flush();
    }

    /// Recreates both sockets and re-queues the starting dumps: after an
    /// error, a missed burst, or a suspend the tables may be stale, and
    /// the dumps are authoritative. What was shown is cleared first: a
    /// dead network's last SSID is not shown as if live. A failed reopen
    /// arms the retry timer instead of stranding the module; a successful
    /// one does too, as a backstop — the queued dumps go out on the next
    /// flush, and the timer fires one if no event flushes first.
    fn resync(&mut self) {
        let menu = self.menu.take();
        let said = self.said_degraded;
        let settings = Settings {
            interface: self.interface.clone(),
            show_ssid: self.show_ssid,
            menu_command: self.menu_command.clone(),
        };
        match open_inner(&settings) {
            Ok(mut fresh) => {
                fresh.menu = menu;
                fresh.said_degraded = said;
                fresh.warn_degraded();
                initial_dumps(&mut fresh.nets);
                *self = fresh;
                // `open_inner` leaves the sockets blocking (for `open`'s
                // synchronous dump); the loop needs them nonblocking.
                // A socket that will not go nonblocking is dropped: the
                // retry timer reopens it.
                if set_nonblock(self.rt.as_ref()).is_err() {
                    self.rt = None;
                }
                if set_nonblock(self.genl.as_ref()).is_err() {
                    self.genl = None;
                }
                if self.rt.is_none() {
                    self.nets = Nets::default();
                    self.timer = None;
                }
                self.arm_retry();
            }
            Err(_) => {
                self.rt = None;
                self.genl = None;
                self.nets = Nets::default();
                self.timer = None;
                self.menu = menu;
                self.arm_retry();
            }
        }
    }

    /// Warns once per process when WiFi is degraded: no nl80211 at all,
    /// or the multicast join did not fit and roam arrives late.
    fn warn_degraded(&mut self) {
        if self.said_degraded {
            return;
        }
        self.said_degraded = true;
        if self.genl.is_none() {
            crate::print::warn(format_args!(
                "scootbar: network: no nl80211, WiFi state is unavailable"
            ));
        } else if !self.nets.joined {
            crate::print::warn(format_args!(
                "scootbar: network: the nl80211 multicast did not fit the join, roaming shows at the next signal refresh"
            ));
        }
    }

    fn arm_retry(&mut self) {
        self.retry = None;
        let Ok(fd) = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        ) else {
            return;
        };
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: RETRY_SECS as i64,
                tv_nsec: 0,
            },
        };
        if timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).is_ok() {
            self.retry = Some(fd);
        }
    }

    /// Arms the signal timer while a WiFi network is shown, disarms it
    /// otherwise. The timerfd is created and dropped with the need: no
    /// connection, no timer, no wakeups.
    fn arm_timer(&mut self) {
        let want = match self.state() {
            State::Wifi => self
                .nets
                .selected(self.interface.as_deref())
                .map(|iface| iface.link.index)
                .unwrap_or(0),
            _ => 0,
        };
        if want != 0 && self.timer_for == want && self.timer.is_some() {
            return;
        }
        self.timer = None;
        self.timer_for = 0;
        if want == 0 {
            return;
        }
        let Ok(fd) = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        ) else {
            return;
        };
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: SIGNAL_SECS as i64,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: SIGNAL_SECS as i64,
                tv_nsec: 0,
            },
        };
        if timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).is_ok() {
            self.timer = Some(fd);
            self.timer_for = want;
            self.nets.station_of = want;
        }
    }

    fn on_timer(&mut self) {
        let Some(timer) = self.timer.as_ref() else {
            return;
        };
        let mut expirations = [0u8; 8];
        match rustix::io::read(timer, &mut expirations) {
            Ok(_) | Err(Errno::AGAIN) => {}
            Err(_) => {
                self.timer = None;
                self.timer_for = 0;
                return;
            }
        }
        // The association may have changed under us; re-asking is cheap
        // and self-healing. Queued behind any scan, so dumps never collide.
        let of = self.timer_for;
        self.nets.station_of = of;
        self.nets.genl_dumps.push_back(GenlDump::Station(of));
        self.flush();
    }

    /// Opens the picker: spawns `menu-command` with the scan list on
    /// stdin. Fire and forget (the M6 popups read the choice back; this
    /// does not), reaped by pidfd when it exits. Idempotent while one is
    /// already open.
    fn open_menu(&mut self) -> Result<Update, InvokeError> {
        if self.menu.is_some() {
            return Ok(Update::Unchanged);
        }
        if self.menu_command.is_empty() {
            return Err(InvokeError::Refused("no menu command configured"));
        }
        let list = self.nets.scan_list();
        if list.is_empty() {
            return Err(InvokeError::Refused("no networks seen yet"));
        }
        let (read, write) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .map_err(|_| InvokeError::Refused("cannot start the menu command"))?;
        let mut command = Command::new(&self.menu_command[0]);
        command.args(&self.menu_command[1..]);
        command.stdin(Stdio::from(read));
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::inherit());
        let child = command
            .spawn()
            .map_err(|_| InvokeError::Refused("cannot start the menu command"))?;
        // The list is a couple of kilobytes against a 64 KiB pipe: one
        // write carries it, even when the menu never reads. A short write
        // still leaves a usable prefix; the menu, not the bar, owns the
        // choice.
        let mut input: std::fs::File = write.into();
        use std::io::Write as _;
        let _ = input.write_all(&list);
        drop(input);
        let pidfd = rustix::process::pidfd_open(
            rustix::process::Pid::from_child(&child),
            rustix::process::PidfdFlags::empty(),
        )
        .ok();
        // Without pidfds (pre-5.3 kernels) the child is reaped
        // opportunistically on every ready turn instead.
        self.menu = Some(Menu { child, pidfd });
        self.reap_menu();
        // A fresh scan for the next opening.
        if self.nets.scan_of != 0 {
            let of = self.nets.scan_of;
            self.nets.queue_scan(of);
            self.nets
                .genl_dumps
                .push_back(GenlDump::Station(self.nets.scan_of));
            self.flush();
        }
        Ok(Update::Unchanged)
    }

    /// Reaps the menu when it exited: by pidfd where there is one, by
    /// `try_wait` everywhere. Returns whether a menu is still running.
    fn reap_menu(&mut self) -> bool {
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        if let Some(pidfd) = menu.pidfd.as_ref() {
            let mut expirations = [0u8; 8];
            match rustix::io::read(pidfd, &mut expirations) {
                Ok(_) => {}
                Err(Errno::AGAIN) => {
                    let _ = menu.child.try_wait();
                    return true;
                }
                Err(_) => {}
            }
        }
        match menu.child.try_wait() {
            Ok(Some(_)) | Err(_) => {
                self.menu = None;
                false
            }
            Ok(None) => true,
        }
    }
}

/// Sends the whole outbox, or as much as goes without blocking. `false`
/// when the socket is gone: the caller resyncs.
fn flush_out(fd: &OwnedFd, out: &mut Vec<u8>) -> bool {
    while !out.is_empty() {
        match send(fd, out, SendFlags::empty()) {
            Ok(n) => out.drain(..n),
            Err(Errno::AGAIN) => break,
            Err(_) => return false,
        };
    }
    true
}

/// Opens both sockets, joins the multicast, and queues the starting
/// dumps. Blocking, at init only: the kernel answers on loopback in
/// microseconds, and `init` must know whether there is anything to show.
fn open(settings: &Settings) -> Result<Network, String> {
    let mut network = open_inner(settings)?;
    // The starting state, synchronously: without it the module cannot say
    // `Unavailable` where there is no hardware.
    let mut out = Vec::new();
    netlink::link_dump(&mut out, network.nets.next_seq());
    send_blocking(network.rt.as_ref(), &out)?;
    let mut buf = [0u8; READ_LEN];
    for _ in 0..64 {
        let Some(sock) = network.rt.as_ref() else {
            break;
        };
        match recv(&sock.fd, &mut buf, RecvFlags::empty()) {
            Ok((_, 0)) | Err(_) => break,
            Ok((_, n)) => {
                network.nets.seen = true;
                network.nets.on_rt_datagram(&buf[..n]);
                if netlink::messages(&buf[..n])
                    .any(|msg| msg.kind == netlink::NLMSG_DONE || msg.kind == netlink::NLMSG_ERROR)
                {
                    break;
                }
            }
        }
    }
    set_nonblock(network.rt.as_ref())?;
    set_nonblock(network.genl.as_ref())?;
    initial_dumps(&mut network.nets);
    network.flush();
    network.warn_degraded();
    network.arm_timer();
    Ok(network)
}

/// Queues the starting dumps: links (already known at init, refilled on
/// a resync), addresses, routes and the wireless interfaces. They go out
/// one per socket at a time (see [`Dump`]); the replies refill the tables
/// as they arrive.
fn initial_dumps(nets: &mut Nets) {
    nets.rt_dumps.push_back(Dump::Links);
    nets.rt_dumps.push_back(Dump::Addrs(2));
    nets.rt_dumps.push_back(Dump::Addrs(10));
    nets.rt_dumps.push_back(Dump::Routes(2));
    nets.rt_dumps.push_back(Dump::Routes(10));
    if nets.family.id != 0 {
        nets.genl_dumps.push_back(GenlDump::Interfaces);
    }
}

fn open_inner(settings: &Settings) -> Result<Network, String> {
    let rt = route_socket()?;
    let (genl, family, joined) = genl_socket()?;
    let nets = Nets {
        family,
        joined,
        ..Nets::default()
    };
    Ok(Network {
        interface: settings.interface.clone(),
        show_ssid: settings.show_ssid,
        menu_command: settings.menu_command.clone(),
        nets,
        rt: Some(Sock {
            fd: rt,
            buf: [0; READ_LEN],
        }),
        genl: genl.map(|fd| Sock {
            fd,
            buf: [0; READ_LEN],
        }),
        timer: None,
        timer_for: 0,
        retry: None,
        menu: None,
        said_degraded: false,
    })
}

fn set_nonblock(sock: Option<&Sock>) -> Result<(), String> {
    let Some(sock) = sock else {
        return Ok(());
    };
    rustix::fs::fcntl_setfl(&sock.fd, rustix::fs::OFlags::NONBLOCK)
        .map_err(|errno| format!("cannot set nonblocking: {errno}"))
}

fn send_blocking(sock: Option<&Sock>, out: &[u8]) -> Result<(), String> {
    let Some(sock) = sock else {
        return Ok(());
    };
    send(&sock.fd, out, SendFlags::empty())
        .map_err(|errno| format!("cannot dump the links: {errno}"))?;
    Ok(())
}

fn route_socket() -> Result<OwnedFd, String> {
    let fd = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|errno| format!("cannot open the rtnetlink socket: {errno}"))?;
    let groups = RTMGRP_LINK
        | RTMGRP_IPV4_IFADDR
        | RTMGRP_IPV6_IFADDR
        | RTMGRP_IPV4_ROUTE
        | RTMGRP_IPV6_ROUTE;
    bind(&fd, &SocketAddrNetlink::new(0, groups))
        .map_err(|errno| format!("cannot join the rtnetlink groups: {errno}"))?;
    Ok(fd)
}

/// The multicast mask for the nl80211 `scan`/`mlme` groups: whatever ids
/// fit the bind mask's 32 bits join (rustix offers no `ADD_MEMBERSHIP`),
/// and `both` says whether roam and scan notices both arrive promptly.
/// A group past bit 31, or absent (id 0), simply does not join; the other
/// still does, and the signal timer covers what is missed.
fn join_mask(scan: u32, mlme: u32) -> (u32, bool) {
    fn bit(id: u32) -> u32 {
        if id != 0 && id < 32 { 1u32 << id } else { 0 }
    }
    let mask = bit(scan) | bit(mlme);
    (mask, mask != 0 && bit(scan) != 0 && bit(mlme) != 0)
}

/// Opens the generic socket and resolves nl80211: first unbound to ask
/// the controller, then bound with the [`join_mask`] mask where any id
/// fits it. `joined` is false where neither does (see the module docs).
fn genl_socket() -> Result<(Option<OwnedFd>, Family, bool), String> {
    let probe = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        Some(GENERIC),
    )
    .map_err(|errno| format!("cannot open the generic netlink socket: {errno}"))?;
    bind(&probe, &SocketAddrNetlink::new(0, 0))
        .map_err(|errno| format!("cannot bind the generic netlink socket: {errno}"))?;
    let mut out = Vec::new();
    netlink::family_request(&mut out, 1, b"nl80211");
    send(&probe, &out, SendFlags::empty())
        .map_err(|errno| format!("cannot ask for nl80211: {errno}"))?;
    let mut buf = [0u8; READ_LEN];
    let mut family = Family::default();
    'resolve: for _ in 0..16 {
        match recv(&probe, &mut buf, RecvFlags::empty()) {
            Ok((_, 0)) | Err(_) => break,
            Ok((_, n)) => {
                for msg in netlink::messages(&buf[..n]) {
                    if msg.kind == netlink::GENL_ID_CTRL && msg.seq == 1 {
                        let found = netlink::parse_family(msg.body);
                        if found.id != 0 {
                            family = found;
                        }
                    }
                    if msg.kind == netlink::NLMSG_ERROR && netlink::error_of(msg.body).is_some() {
                        // Answered, negatively (no such family): waiting
                        // for more would hang the init. A zero errno is
                        // the request's own ack, not an answer.
                        break 'resolve;
                    }
                }
                if family.id != 0 {
                    break;
                }
            }
        }
    }
    if family.id == 0 {
        // No nl80211: link state still works, WiFi does not.
        return Ok((None, family, false));
    }
    let (mask, both) = join_mask(family.scan, family.mlme);
    if mask != 0 {
        let joined = socket_with(
            AddressFamily::NETLINK,
            SocketType::RAW,
            SocketFlags::CLOEXEC,
            Some(GENERIC),
        )
        .and_then(|fd| bind(&fd, &SocketAddrNetlink::new(0, mask)).map(|()| fd));
        if let Ok(fd) = joined {
            return Ok((Some(fd), family, both));
        }
    }
    Ok((Some(probe), family, false))
}

/// Starts the module on already-open sockets: the tests' way in, with a
/// socketpair standing in for the kernel. `family` is the fake kernel's
/// nl80211 id (0 for none, as where there is no WiFi).
#[cfg(test)]
fn start_with(settings: &Settings, rt: OwnedFd, genl: OwnedFd, family: u16) -> Box<dyn Module> {
    let _ = rustix::fs::fcntl_setfl(&rt, rustix::fs::OFlags::NONBLOCK);
    let _ = rustix::fs::fcntl_setfl(&genl, rustix::fs::OFlags::NONBLOCK);
    let mut nets = Nets::default();
    nets.family.id = family;
    Box::new(Network {
        interface: settings.interface.clone(),
        show_ssid: settings.show_ssid,
        menu_command: settings.menu_command.clone(),
        nets,
        rt: Some(Sock {
            fd: rt,
            buf: [0; READ_LEN],
        }),
        genl: Some(Sock {
            fd: genl,
            buf: [0; READ_LEN],
        }),
        timer: None,
        timer_for: 0,
        retry: None,
        menu: None,
        said_degraded: true,
    })
}

impl Module for Network {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        if let Some(sock) = self.rt.as_ref() {
            sources.add(sock.fd.as_fd(), PollFlags::IN);
        }
        if let Some(sock) = self.genl.as_ref() {
            sources.add(sock.fd.as_fd(), PollFlags::IN);
        }
        if let Some(timer) = self.timer.as_ref() {
            sources.add(timer.as_fd(), PollFlags::IN);
        }
        if let Some(menu) = self.menu.as_ref() {
            if let Some(pidfd) = menu.pidfd.as_ref() {
                sources.add(pidfd.as_fd(), PollFlags::IN);
            }
        }
        if let Some(retry) = self.retry.as_ref() {
            sources.add(retry.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        // Without pidfds the menu is reaped on every turn instead.
        if self.menu.as_ref().is_some_and(|menu| menu.pidfd.is_none()) {
            self.reap_menu();
        }
        let before = self.fingerprint();
        match self.source_at(source) {
            Some(Source::Rt) => {
                if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
                    self.resync();
                } else if events.intersects(PollFlags::IN) {
                    self.drain_rt();
                }
            }
            Some(Source::Genl) => {
                if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
                    self.resync();
                } else if events.intersects(PollFlags::IN) {
                    self.drain_genl();
                }
            }
            Some(Source::Timer) => {
                self.on_timer();
            }
            Some(Source::Menu) => {
                self.reap_menu();
            }
            Some(Source::Retry) => {
                if let Some(retry) = self.retry.as_ref() {
                    let mut expirations = [0u8; 8];
                    let _ = rustix::io::read(retry, &mut expirations);
                }
                self.retry = None;
                // The backstop: queued dumps go out even where no event
                // flushed first. Dead sockets reopen instead.
                if self.rt.is_some() {
                    self.flush();
                } else {
                    self.resync();
                }
            }
            None => {}
        }
        self.flush();
        self.arm_timer();
        if self.fingerprint() != before {
            Update::Changed
        } else {
            Update::Unchanged
        }
    }

    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        if !self.nets.seen {
            return;
        }
        let selected = self.nets.selected(self.interface.as_deref());
        let marker = selected.is_some_and(|iface| self.nets.other_vpn(iface.link.index));
        match self.state() {
            State::Offline => {
                let _ = write!(view.text_mut(), "offline");
                let _ = write!(view.tooltip_mut(), "No network");
                view.set_class(crate::modules::Class::Warn);
            }
            State::Eth => {
                let name = selected.map(|iface| iface.link.name_str()).unwrap_or("");
                let _ = write!(view.text_mut(), "{name}");
                let _ = write!(view.tooltip_mut(), "{name}");
            }
            State::Wifi => {
                let Some(iface) = selected else {
                    return;
                };
                let bars = netlink::bars_for(iface.signal.unwrap_or(-100));
                let glyphs = &netlink::BAR_GLYPHS[..bars as usize];
                if self.show_ssid {
                    let _ = write!(view.text_mut(), "{} ", iface.ssid);
                } else {
                    let _ = write!(view.text_mut(), "WiFi ");
                }
                for glyph in glyphs {
                    let _ = write!(view.text_mut(), "{glyph}");
                }
                let _ = write!(
                    view.tooltip_mut(),
                    "{} · ",
                    if self.show_ssid {
                        iface.ssid.as_str()
                    } else {
                        "WiFi"
                    },
                );
                match iface.signal {
                    Some(dbm) => {
                        let _ = write!(view.tooltip_mut(), "{dbm} dBm on ");
                    }
                    None => {
                        let _ = write!(view.tooltip_mut(), "no signal on ");
                    }
                }
                let _ = write!(view.tooltip_mut(), "{}", iface.link.name_str());
            }
            State::Vpn => {
                let name = selected.map(|iface| iface.link.name_str()).unwrap_or("");
                let _ = write!(view.text_mut(), "VPN");
                let _ = write!(view.tooltip_mut(), "VPN on {name}");
            }
        }
        if marker {
            let _ = write!(view.text_mut(), " · VPN");
        }
    }

    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        if !self.nets.seen {
            return None;
        }
        let state = self.state();
        let Some(selected) = self.nets.selected(self.interface.as_deref()) else {
            return Some(serde_json::json!({"state": "disconnected"}));
        };
        if state == State::Offline {
            return Some(serde_json::json!({"state": "disconnected"}));
        }
        let mut value = serde_json::json!({
            "interface": selected.link.name_str(),
            "vpn": self.nets.other_vpn(selected.link.index) || state == State::Vpn,
        });
        match state {
            State::Wifi => {
                value["state"] = serde_json::Value::String("wifi".to_owned());
                if self.show_ssid {
                    value["ssid"] = serde_json::Value::String(selected.ssid.clone());
                }
                if let Some(signal) = selected.signal {
                    value["signal"] = serde_json::Value::from(signal);
                    value["bars"] = serde_json::Value::from(netlink::bars_for(signal));
                }
            }
            State::Eth => {
                value["state"] = serde_json::Value::String("ethernet".to_owned());
            }
            State::Vpn => {
                value["state"] = serde_json::Value::String("vpn".to_owned());
            }
            State::Offline => {}
        }
        Some(value)
    }

    /// A click opens the picker; anything else means nothing.
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        match input.trigger {
            Trigger::Click => Some(crate::action::Action::Module(ModuleAction::new(
                "menu", None,
            ))),
            _ => None,
        }
    }

    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = steps;
        if action.arg.is_some() {
            return Err(InvokeError::NoArg);
        }
        match &*action.name {
            "menu" => self.open_menu(),
            _ => Err(InvokeError::Unknown),
        }
    }

    /// A click opens the picker with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }
}
