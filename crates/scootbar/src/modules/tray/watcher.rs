//! The StatusNotifierWatcher and the bus conversation: the live half of
//! the tray, with no drawing in it.
//!
//! The bar owns `org.kde.StatusNotifierWatcher` (and its `org.freedesktop`
//! twin) when free, answers registrations itself, and otherwise hosts
//! against whoever does. Every call is asynchronous past the set-up: a
//! flight table maps replies back to what asked, and a call nobody
//! answers is forgotten by age when its slot is wanted.

use std::os::fd::OwnedFd;
use std::time::Instant;

use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use super::Update;
use super::item::{Fingerprint, Item, fill, get_all_body, is_item_name};
use super::menu::{MENU_IFACE, MenuOpen, about_body, event_body, fill_menu, get_layout_body};
use super::timer::OneShot;
use super::{
    DRAW_GAP, FLIGHT_TTL, ITEM_DEFAULT_PATH, ITEM_FDO, ITEM_KDE, ITEM_PROPERTIES, MAX_ITEMS,
    MAX_PER_SERVICE, MIN_REFRESH_GAP, PROTOCOL_VERSION, WATCHER_FDO, WATCHER_KDE, WATCHER_PATH,
};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Session;
use crate::dbus::proto::{self, Reader, Writer, check_name, check_path, request_reply};

/// A live bus: the connection, the mode, the items in id order, and the
/// calls in flight.
pub(super) struct Live {
    /// A one-shot timer, armed only while an item waits out
    /// [`MIN_REFRESH_GAP`] to be read again: no item waiting, no timer,
    /// no wakeups.
    pub(super) coalesce: Option<OwnedFd>,
    /// When a content change was last reported to the bar.
    pub(super) drawn: Option<Instant>,
    /// Armed while a content change waits out [`DRAW_GAP`]: the view the
    /// bar asks for when it fires is the latest, whatever came meanwhile.
    pub(super) held: Option<OneShot>,
    pub(super) conn: Conn,
    pub(super) mode: Mode,
    /// The KDE watcher name's owner while in host mode: who item lists
    /// are read from.
    pub(super) watcher_owner: Option<String>,
    pub(super) items: Vec<Item>,
    pub(super) flights: Vec<Option<Flight>>,
    /// Hosts registered with us (owner mode): our own host is first.
    pub(super) hosts: Vec<String>,
    /// Whether the `too many items` warning was said for the current set.
    pub(super) said_full: bool,
    /// The item's menu that is open, if any: one at a time, transient.
    pub(super) menu: Option<MenuOpen>,
    /// A `menu` action asked for the popup surface: taken once by
    /// [`super::Tray::wants_popup`], which the daemon answers by opening
    /// it (an invoke cannot open a surface itself).
    #[cfg(feature = "popup")]
    pub(super) menu_popup: bool,
}

/// Whether we own the watcher name (answering registrations) or talk to
/// whoever does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Owner,
    Host,
}

/// A call in flight: what the reply correlates to.
pub(super) struct Flight {
    pub(super) op: Op,
}

pub(super) enum Op {
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
    /// A `GetLayout` for the menu of the item with this id, from this
    /// parent row (0 is the whole tree, any other a lazy submenu).
    MenuLayout { item: String, parent: i32 },
}

/// Splits a register argument the KDE way: a leading `/` is a path on the
/// sender's service, else a service with the default path. Both halves
/// validated; `Err(())` refuses the registration, never the bar.
pub(super) fn split_service_path(
    sender: &str,
    service_or_path: &str,
) -> Result<(String, String), ()> {
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

/// Splits what a watcher announces in `StatusNotifierItemRegistered`: the
/// KDE id form (`service + path`: `:1.42/StatusNotifierItem`), which is
/// what KDE's watcher and ours both emit, or, failing a path in the
/// middle, what a registration may say (a bare service, or a bare path on
/// the sender's service). Both halves validated; `Err(())` ignores it.
pub(super) fn split_announced(sender: &str, announced: &str) -> Result<(String, String), ()> {
    match announced.find('/') {
        None | Some(0) => split_service_path(sender, announced),
        Some(_) => {
            let (service, path) = split_id(announced).ok_or(())?;
            check_name(service)?;
            check_path(path)?;
            Ok((service.to_owned(), path.to_owned()))
        }
    }
}

/// Splits a KDE item id (`service + path`) back into its halves: the
/// first `/` starts the path (a service never holds one).
pub(super) fn split_id(id: &str) -> Option<(&str, &str)> {
    let slash = id.find('/')?;
    Some((&id[..slash], &id[slash..]))
}

impl Session for Live {
    fn conn(&self) -> &Conn {
        &self.conn
    }

    fn conn_mut(&mut self) -> &mut Conn {
        &mut self.conn
    }
}

impl Live {
    /// Works one bus event; says whether the view moved.
    pub(super) fn apply(&mut self, event: Event) -> Update {
        match event {
            Event::Reply {
                token,
                signature,
                body,
            } => self.on_reply(token, &signature, &body),
            Event::CallError { token, name } => self.on_error(token, &name),
            Event::Dropped { token } => self.on_dropped(token),
            Event::Signal {
                sender,
                path,
                interface,
                member,
                signature,
                body,
            } => self.on_signal(&sender, &path, &interface, &member, &signature, &body),
            Event::MethodCall {
                sender,
                path,
                interface,
                member,
                serial,
                signature,
                body,
            } => self.on_call(
                &sender, &path, &interface, &member, serial, &signature, &body,
            ),
        }
    }

    /// Queues a call, tracking its flight: `true` when it was queued. A
    /// full table first forgets the calls nobody answered (see
    /// [`FLIGHT_TTL`]); still full, the call is dropped, and the next
    /// signal re-queues it (every refresh is signal-driven, so nothing is
    /// lost but a turn). Calls that want no reply take no flight. Eight
    /// arguments: a call names its destination, object, interface,
    /// member, body and reply wish, like the header it becomes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        body: &[u8],
        flags: u8,
        op: Op,
    ) -> bool {
        let wants_reply = flags & proto::flag::NO_REPLY_EXPECTED == 0;
        let slot = if wants_reply {
            match self.free_slot() {
                Some(slot) => slot,
                None => return false,
            }
        } else {
            usize::MAX
        };
        let token = slot as u64;
        let queued = self
            .conn
            .call(
                destination,
                path,
                interface,
                member,
                body_sig,
                body,
                flags,
                token,
            )
            .is_ok();
        if queued && wants_reply {
            self.flights[slot] = Some(Flight { op });
        }
        queued
    }

    /// A free slot in the flight table, reaping the unanswered when it
    /// is full.
    pub(super) fn free_slot(&mut self) -> Option<usize> {
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

    /// Forgets the calls the connection gave up on, freeing what they
    /// held: an item whose `GetAll` never answered can be asked again.
    /// Says whether an empty menu closed with its call.
    pub(super) fn reap(&mut self) -> bool {
        let mut closed = false;
        for token in self.conn.expire(FLIGHT_TTL) {
            let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
                continue;
            };
            match flight.op {
                Op::Props(id) => {
                    if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                        item.fetching = false;
                        item.stale = false;
                    }
                }
                Op::MenuLayout { item, .. } => {
                    self.finish_menu_fetch(&item);
                    closed |= self.close_menu_if_empty(&item);
                }
                _ => {}
            }
        }
        closed
    }

    /// Fires a call that wants no reply (activation): queued, never
    /// tracked, never reported. Eight arguments, as [`Live::issue`].
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fire(
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

    /// Refreshes the item's properties: one `GetAll`, answered later. One
    /// at a time per item: a signal while one is in flight only marks the
    /// item stale, and the answer asks again.
    pub(super) fn refresh(&mut self, id: &str) {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return;
        };
        if item.fetching {
            item.stale = true;
            return;
        }
        // An item that announces a change as fast as it is read would cost
        // a bus round trip and a redraw each time: read it again no sooner
        // than the gap, and say so with one timer for all such items.
        if item
            .last_asked
            .is_some_and(|at| at.elapsed() < MIN_REFRESH_GAP)
        {
            item.stale = true;
            self.arm_coalesce();
            return;
        }
        item.last_asked = Some(Instant::now());
        item.fetching = true;
        let (service, path) = (item.service.clone(), item.path.clone());
        let queued = self.issue(
            &service,
            &path,
            ITEM_PROPERTIES,
            "GetAll",
            "s",
            &get_all_body(),
            0,
            Op::Props(id.to_owned()),
        );
        if !queued {
            if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                item.fetching = false;
            }
        }
    }

    /// Arms the one-shot timer that re-reads the items waiting out the
    /// gap (a no-op while it is armed). Without a timer (the fd could not
    /// be made) the waiting items are read at their next signal, which
    /// is the slower, never the wrong, answer.
    fn arm_coalesce(&mut self) {
        if self.coalesce.is_some() {
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
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: 0,
                tv_nsec: MIN_REFRESH_GAP.as_nanos() as i64,
            },
        };
        if timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).is_ok() {
            self.coalesce = Some(fd);
        }
    }

    /// The gap passed: reads again the items that waited, and the menu
    /// that waited.
    pub(super) fn on_coalesce(&mut self) {
        if let Some(timer) = self.coalesce.take() {
            let mut expirations = [0u8; 8];
            let _ = rustix::io::read(&timer, &mut expirations);
        }
        let waiting: Vec<String> = self
            .items
            .iter()
            .filter(|item| item.stale && !item.fetching)
            .map(|item| item.id.clone())
            .collect();
        for id in waiting {
            if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                item.stale = false;
            }
            self.refresh(&id);
        }
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.stale && !menu.fetching)
        {
            if let Some(menu) = self.menu.as_mut() {
                menu.stale = false;
            }
            self.refresh_menu();
        }
    }

    /// Reports a content change to the bar now: the module appearing or
    /// emptying, the first change after a quiet spell, and the held change
    /// when its timer fires. Drawing draws the latest state, so whatever
    /// was held is shown with it.
    pub(super) fn draw_now(&mut self) -> Update {
        self.held = None;
        self.drawn = Some(Instant::now());
        Update::Changed
    }

    /// Reports a content change that is not the module appearing or
    /// emptying: at once after a quiet spell, else held for one timer
    /// ([`DRAW_GAP`] since the last report), however many follow.
    pub(super) fn changed(&mut self) -> Update {
        if self.held.is_some() {
            return Update::Unchanged;
        }
        let wait = self
            .drawn
            .map(|at| DRAW_GAP.saturating_sub(at.elapsed()))
            .filter(|wait| !wait.is_zero());
        match wait.and_then(OneShot::after) {
            Some(timer) => {
                self.held = Some(timer);
                Update::Unchanged
            }
            // Past the gap, or no timer to wait with: now.
            None => self.draw_now(),
        }
    }

    /// An item's `GetAll` came back (answered or errored): it may be
    /// asked again, and whether a signal arrived meanwhile.
    pub(super) fn finish_fetch(&mut self, id: &str) -> bool {
        match self.items.iter_mut().find(|item| item.id == id) {
            Some(item) => {
                item.fetching = false;
                core::mem::take(&mut item.stale)
            }
            None => false,
        }
    }

    /// Opens the item's menu: drops whatever menu was open (one at a
    /// time), sends `AboutToShow(0)` and the first `GetLayout`, and says
    /// whether a menu opened (the caller then asks the daemon for the
    /// popup surface). Refused when the item has no menu path (the
    /// caller falls back to `ContextMenu`), names nothing, or is not
    /// verified yet (no owner to match update signals against).
    #[cfg(feature = "popup")]
    pub(super) fn open_menu(&mut self, id: &str) -> bool {
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return false;
        };
        if item.menu.is_empty() || item.owner.is_none() {
            return false;
        }
        let menu = MenuOpen::new(
            &item.id,
            &item.service,
            &item.menu,
            item.owner.as_deref().unwrap_or(""),
        );
        self.menu = Some(menu);
        // `AboutToShow` wants no reply (the layout follows either way,
        // answered later through `on_menu_layout`).
        if let Some(bytes) = about_body(0) {
            let (service, path) = (item.service.clone(), item.menu.clone());
            self.fire(&service, &path, MENU_IFACE, "AboutToShow", "i", &bytes);
        }
        self.ask_layout(id, 0);
        true
    }

    /// Queues the item's menu read from `parent` (0 is the whole tree,
    /// any other a lazy submenu): one at a time, answered later.
    fn ask_layout(&mut self, id: &str, parent: i32) {
        let Some(menu) = self.menu.as_mut().filter(|menu| menu.item == id) else {
            return;
        };
        menu.last_asked = Some(Instant::now());
        menu.fetching = true;
        let (service, path) = (menu.service.clone(), menu.path.clone());
        let Some(bytes) = get_layout_body(parent) else {
            menu.fetching = false;
            return;
        };
        let queued = self.issue(
            &service,
            &path,
            MENU_IFACE,
            "GetLayout",
            "iias",
            &bytes,
            0,
            Op::MenuLayout {
                item: id.to_owned(),
                parent,
            },
        );
        if !queued {
            if let Some(menu) = self.menu.as_mut().filter(|menu| menu.item == id) {
                menu.fetching = false;
            }
        }
    }

    /// Re-reads the open menu, no oftener than the items' floor: one
    /// that floods `LayoutUpdated` costs a round trip per gap, like a
    /// runaway icon, and the waiting re-read shares the items' timer.
    pub(super) fn refresh_menu(&mut self) {
        let Some(item) = self.menu.as_ref().map(|menu| menu.item.clone()) else {
            return;
        };
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        if menu.fetching {
            menu.stale = true;
            return;
        }
        if menu
            .last_asked
            .is_some_and(|at| at.elapsed() < MIN_REFRESH_GAP)
        {
            menu.stale = true;
            self.arm_coalesce();
            return;
        }
        self.ask_layout(&item, 0);
    }

    /// A `GetLayout` came back: answered or errored, the item may be
    /// asked again, and whether a signal arrived meanwhile.
    pub(super) fn finish_menu_fetch(&mut self, id: &str) -> bool {
        match self.menu.as_mut().filter(|menu| menu.item == id) {
            Some(menu) => {
                menu.fetching = false;
                core::mem::take(&mut menu.stale)
            }
            None => false,
        }
    }

    /// Applies a `GetLayout` answer to the open menu and says whether
    /// anything shown moved. A misshapen answer is dropped: a first
    /// load that never parses loses the menu, a later one keeps its
    /// last state.
    pub(super) fn on_menu_layout(
        &mut self,
        id: &str,
        parent: i32,
        signature: &str,
        body: &[u8],
    ) -> Update {
        let stale = self.finish_menu_fetch(id);
        if self.menu.as_ref().is_none_or(|menu| menu.item != id) {
            return Update::Unchanged;
        }
        let was_empty = self.menu.as_ref().is_some_and(|menu| menu.root.is_empty());
        let parsed = proto::read_menu_layout(signature, body).ok();
        let moved = match parsed {
            None => {
                let first = self
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.root.is_empty() && parent == 0);
                if first {
                    self.menu = None;
                    true
                } else {
                    false
                }
            }
            Some((revision, parsed)) => {
                let Some(menu) = self.menu.as_mut().filter(|menu| menu.item == id) else {
                    return Update::Unchanged;
                };
                let before = (menu.root.clone(), menu.trail.clone());
                fill_menu(menu, parent, revision, &parsed);
                (menu.root.clone(), menu.trail.clone()) != before
            }
        };
        if stale {
            self.refresh_menu();
        }
        if moved {
            // The menu newly filled or newly lost opens or closes its
            // popup: at once. New rows on one already shown wait out the
            // draw gap, like item icons.
            if was_empty {
                self.draw_now()
            } else {
                self.changed()
            }
        } else {
            Update::Unchanged
        }
    }

    /// Works the menu's own signals, accepted only from the item that
    /// owns the open menu: anything else (another item, a peer's
    /// forgery) moves nothing. Either update re-reads the layout behind
    /// the refresh floor; anything else is ignored.
    pub(super) fn on_menu_signal(
        &mut self,
        sender: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Update {
        if !self.menu.as_ref().is_some_and(|menu| menu.owner == sender) {
            return Update::Unchanged;
        }
        match member {
            "LayoutUpdated" => {
                if proto::read_layout_updated(signature, body).is_err() {
                    return Update::Unchanged;
                }
                self.refresh_menu();
                Update::Unchanged
            }
            "ItemsPropertiesUpdated" => {
                if signature != "a(ia{sv})a(ias)" {
                    return Update::Unchanged;
                }
                self.refresh_menu();
                Update::Unchanged
            }
            _ => Update::Unchanged,
        }
    }

    /// Clicks the row with dbusmenu `id`: sends `Event clicked` and
    /// closes the menu. Refused when no menu is open, the row is gone,
    /// or it is not a live row (a separator, a disabled row, or one
    /// that opens a submenu drills instead of clicking).
    pub(super) fn menu_select(&mut self, id: i32) -> bool {
        let live = self.menu.as_ref().is_some_and(|menu| {
            menu.find(id)
                .is_some_and(|node| node.enabled && !node.separator && !node.opens())
        });
        if !live {
            return false;
        }
        let Some(menu) = self.menu.as_ref() else {
            return false;
        };
        let (service, path) = (menu.service.clone(), menu.path.clone());
        let Some(bytes) = event_body(id) else {
            return false;
        };
        self.fire(&service, &path, MENU_IFACE, "Event", "isvu", &bytes);
        self.menu = None;
        true
    }

    /// Drills into the submenu row with dbusmenu `id`: rows that
    /// arrived show at once, a lazy one (asked for submenu display with
    /// no children yet) is asked for first. Refused for anything else.
    pub(super) fn menu_drill(&mut self, id: i32) -> bool {
        let Some(item) = self.menu.as_ref().map(|menu| menu.item.clone()) else {
            return false;
        };
        let drill = self.menu.as_ref().and_then(|menu| {
            let node = menu.find(id)?;
            if !node.enabled || node.separator || !node.opens() {
                return None;
            }
            Some((node.children.is_empty(), menu.fetching))
        });
        let Some((lazy, fetching)) = drill else {
            return false;
        };
        if !lazy {
            if let Some(menu) = self.menu.as_mut().filter(|menu| menu.item == item) {
                menu.trail.push(id);
            }
            return true;
        }
        if fetching {
            return false;
        }
        if let Some(bytes) = about_body(id) {
            let (service, path) = self
                .menu
                .as_ref()
                .map(|menu| (menu.service.clone(), menu.path.clone()))
                .unwrap_or_default();
            if service.is_empty() {
                return false;
            }
            self.fire(&service, &path, MENU_IFACE, "AboutToShow", "i", &bytes);
        }
        self.ask_layout(&item, id);
        true
    }

    /// Backs out one level, or closes the menu past the root. Always a
    /// change when a menu is open.
    pub(super) fn menu_back(&mut self) -> bool {
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        if menu.trail.pop().is_none() {
            self.menu = None;
        }
        true
    }

    /// Drops the open menu when its item is gone: called after every
    /// path that retains the items (a crash without unregistering
    /// retains inline, past [`Live::remove`]).
    /// Drops the open menu when its item is gone: called after every
    /// path that retains the items (a crash without unregistering
    /// retains inline, past [`Live::remove`]).
    fn close_menu_if_vanished(&mut self) {
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| !self.items.iter().any(|item| item.id == menu.item))
        {
            self.menu = None;
        }
    }

    /// Closes the item's menu when its first load never landed (an
    /// empty tree): later failures keep the last state instead.
    /// Whether one closed.
    pub(super) fn close_menu_if_empty(&mut self, id: &str) -> bool {
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.item == id && menu.root.is_empty())
        {
            self.menu = None;
            true
        } else {
            false
        }
    }

    pub(super) fn on_reply(&mut self, token: u64, signature: &str, body: &[u8]) -> Update {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return Update::Unchanged;
        };
        match flight.op {
            Op::Props(id) => {
                let stale = self.finish_fetch(&id);
                let update = self.on_props(&id, signature, body);
                if stale {
                    self.refresh(&id);
                }
                update
            }
            Op::Owner(id) => self.on_owner(&id, signature, body),
            Op::Names => self.on_names(signature, body),
            Op::WatcherItems => self.on_watcher_items(signature, body),
            Op::RequestKde => self.on_request_reply(true, signature, body),
            Op::RequestFdo => self.on_request_reply(false, signature, body),
            Op::WatcherOwner => self.on_watcher_owner_reply(signature, body),
            Op::MenuLayout { item, parent } => self.on_menu_layout(&item, parent, signature, body),
        }
    }

    /// A reply past the size this client reads was skipped: the call is
    /// answered with nothing usable. An item that sent one loses that
    /// update only (it keeps its last state, and is read again at its
    /// next signal); a bus question that did is simply unanswered.
    /// [`conn::DROPPED_UNKNOWN`] instead of a token: the skipped reply's
    /// header was not read, so no call can be named. Some call lost its
    /// answer: free what expired by age, and let every item waiting on
    /// an answer ask again at its next signal (one more `GetAll` each,
    /// behind the refresh floor), instead of staying stuck till a reap.
    pub(super) fn on_dropped(&mut self, token: u64) -> Update {
        if token == conn::DROPPED_UNKNOWN {
            let closed = self.reap();
            for item in &mut self.items {
                if item.fetching {
                    item.fetching = false;
                    item.stale = true;
                }
            }
            if let Some(menu) = self.menu.as_mut() {
                menu.fetching = false;
                menu.stale = true;
            }
            if closed {
                return self.draw_now();
            }
            return Update::Unchanged;
        }
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return Update::Unchanged;
        };
        match flight.op {
            Op::Props(id) => {
                // `stale` is not honored: re-reading at once would fetch the
                // same oversized answer; the next signal asks again.
                self.finish_fetch(&id);
            }
            Op::MenuLayout { item, .. } => {
                // The same, for a menu: a later signal reads it again, and a
                // first load that never parses loses the menu (see
                // `on_menu_layout`).
                self.finish_menu_fetch(&item);
                if self.close_menu_if_empty(&item) {
                    return self.draw_now();
                }
            }
            _ => {}
        }
        Update::Unchanged
    }

    pub(super) fn on_error(&mut self, token: u64, name: &str) -> Update {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return Update::Unchanged;
        };
        match flight.op {
            // A `GetAll` that errors is an item gone (or never there): a
            // bogus registration loses itself, silently.
            Op::Props(id) => self.remove(&id),
            // A `GetLayout` that errors is a menu losing its update: a
            // first load that never lands loses the menu, a later one
            // keeps its last state.
            Op::MenuLayout { item, .. } => {
                self.finish_menu_fetch(&item);
                if self.close_menu_if_empty(&item) {
                    self.draw_now()
                } else {
                    Update::Unchanged
                }
            }
            // `NameHasNoOwner` is an item already gone; anything else
            // leaves the owner unknown (signals still match by sender).
            Op::Owner(id) if name.ends_with("NameHasNoOwner") => self.remove(&id),
            // The bus refused the watcher name (a policy that denies
            // `own`): say so once, and host against whoever has it, as
            // when it is owned elsewhere, so the tray is not silently dead.
            Op::RequestKde => {
                crate::print::warn(format_args!(
                    "scootbar: tray: the bus refused the watcher name ({name}); \
                     hosting against another watcher if there is one"
                ));
                self.match_rules();
                self.mode = Mode::Host;
                self.read_watcher_owner();
                Update::Unchanged
            }
            Op::Owner(_) | Op::Names | Op::WatcherItems | Op::RequestFdo | Op::WatcherOwner => {
                Update::Unchanged
            }
        }
    }

    /// Applies a `GetAll` answer to the item, through [`fill`], and
    /// reports whether anything shown moved. A misshapen answer is
    /// dropped (the item keeps its last state).
    pub(super) fn on_props(&mut self, id: &str, signature: &str, body: &[u8]) -> Update {
        let index = match self.items.iter().position(|item| item.id == id) {
            Some(index) => index,
            None => return Update::Unchanged,
        };
        if signature != "a{sv}" {
            return Update::Unchanged;
        }
        let before = Fingerprint::of(&self.items[index]);
        let was = self.items[index].shown();
        if !fill(&mut self.items[index], body) {
            return Update::Unchanged;
        }
        if Fingerprint::of(&self.items[index]) != before {
            // An item newly shown or hidden moves the bar's layout: at
            // once, like an item arriving or leaving. A new icon or title
            // on one already shown waits out the draw gap.
            if self.items[index].shown() != was {
                self.draw_now()
            } else {
                self.changed()
            }
        } else {
            Update::Unchanged
        }
    }

    pub(super) fn on_owner(&mut self, id: &str, signature: &str, body: &[u8]) -> Update {
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
    pub(super) fn on_names(&mut self, signature: &str, body: &[u8]) -> Update {
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
            if !self.has_room(&name, &name) {
                self.say_full(&id);
                continue;
            }
            self.items.push(Item::new(
                id.clone(),
                name.clone(),
                ITEM_DEFAULT_PATH.to_owned(),
                name.clone(),
            ));
            self.sort();
            added = true;
            if name.starts_with(':') {
                self.set_owner(&id, name);
                self.refresh(&id);
            } else {
                let mut get = Writer::new();
                get.str(&name);
                let Some(bytes) = get.take_body() else {
                    continue;
                };
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
        if added {
            self.draw_now()
        } else {
            Update::Unchanged
        }
    }

    /// Works the existing watcher's item list (host mode): adds what is
    /// new, drops what left.
    pub(super) fn on_watcher_items(&mut self, _signature: &str, body: &[u8]) -> Update {
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
            let (service, path) = split_id(id).unwrap_or(("", ""));
            if !self.has_room(service, service) {
                self.say_full(id);
                continue;
            }
            self.items.push(Item::new(
                id.clone(),
                service.to_owned(),
                path.to_owned(),
                service.to_owned(),
            ));
            self.sort();
            moved = true;
            let id = id.clone();
            self.refresh(&id);
        }
        let before = self.items.len();
        self.items.retain(|item| seen.contains(&item.id));
        if self.items.len() != before {
            self.close_menu_if_vanished();
            moved = true;
        }
        if moved {
            self.draw_now()
        } else {
            Update::Unchanged
        }
    }

    pub(super) fn on_signal(
        &mut self,
        sender: &str,
        _path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Update {
        if interface == conn::BUS_INTERFACE && member == "NameOwnerChanged" {
            // Only the bus says who owns what: a peer can send this signal
            // to us, addressed, and it is not believed.
            if sender != conn::BUS_NAME {
                return Update::Unchanged;
            }
            return self.on_name_owner_changed(signature, body);
        }
        if interface == WATCHER_KDE || interface == WATCHER_FDO {
            return self.on_watcher_signal(sender, member, signature, body);
        }
        if interface == ITEM_KDE || interface == ITEM_FDO {
            return self.on_item_signal(sender, member);
        }
        if interface == MENU_IFACE {
            return self.on_menu_signal(sender, member, signature, body);
        }
        Update::Unchanged
    }

    /// Central owner tracking: items vanish with their owner, the watcher
    /// name switches the mode, and our own echoes are ignored.
    pub(super) fn on_name_owner_changed(&mut self, signature: &str, body: &[u8]) -> Update {
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
        if new.is_none() {
            // A host that went away is not a host anymore (our own entry
            // dies with the connection, which drops the whole session, so
            // only a peer's can be pruned here).
            self.hosts.retain(|host| host != &name);
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
                            refresh.push((item.id.clone(), owner.to_owned()));
                        }
                    }
                }
            }
        }
        if gone {
            self.items.retain(|item| {
                item.service != name && item.owner.as_deref() != Some(name.as_str())
            });
            self.close_menu_if_vanished();
            if self.mode == Mode::Owner {
                self.emit_unregistered(&name);
            }
            return self.draw_now();
        }
        for (id, owner) in refresh {
            // The name changed hands: signals now come from the new
            // owner, and the old one vanishing later must not take the
            // item with it. An open menu follows the owner too, or its
            // updates stop arriving.
            self.set_owner(&id, owner.clone());
            if self.menu.as_ref().is_some_and(|menu| menu.item == id) {
                if let Some(menu) = self.menu.as_mut() {
                    menu.owner = owner;
                }
                self.refresh_menu();
            }
            self.refresh(&id);
        }
        Update::Unchanged
    }

    /// The KDE name's owner moved: ours means we answer registrations
    /// (re-enumerating, since a new watcher starts empty); anyone else's
    /// means we host against them; none means we take it back.
    pub(super) fn on_watcher_owner(&mut self, new: Option<&str>) -> Update {
        let ours = new == Some(self.conn.unique());
        self.watcher_owner = new.map(str::to_owned);
        if ours && self.mode == Mode::Owner {
            return Update::Unchanged;
        }
        if ours {
            self.mode = Mode::Owner;
            self.enumerate();
            return self.draw_now();
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
            return self.draw_now();
        }
        self.mode = Mode::Host;
        self.refresh_watcher();
        self.draw_now()
    }

    /// Works a watcher-name answer: ours means owner mode (matches,
    /// then enumeration); anyone else's means host mode against them
    /// (matches, then their list). The freedesktop twin is best effort:
    /// owned is nice, owned elsewhere changes nothing.
    pub(super) fn on_request_reply(&mut self, kde: bool, signature: &str, body: &[u8]) -> Update {
        let Ok(word) = proto::read_request_reply(signature, body) else {
            return Update::Unchanged;
        };
        let owned = matches!(
            word,
            request_reply::PRIMARY_OWNER | request_reply::ALREADY_OWNER
        );
        if kde {
            self.match_rules();
            if owned {
                self.mode = Mode::Owner;
                self.enumerate();
            } else {
                self.mode = Mode::Host;
                self.read_watcher_owner();
            }
            return self.draw_now();
        }
        Update::Unchanged
    }

    /// Reads who owns the KDE watcher name (host mode): their list and
    /// our host registration follow.
    pub(super) fn on_watcher_owner_reply(&mut self, signature: &str, body: &[u8]) -> Update {
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
    pub(super) fn on_watcher_signal(
        &mut self,
        sender: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Update {
        if self.mode != Mode::Host {
            return Update::Unchanged;
        }
        // Only the watcher we host against speaks for it (which also
        // leaves out our own echoes, emitted in owner mode).
        if self.watcher_owner.as_deref() != Some(sender) || sender == self.conn.unique() {
            return Update::Unchanged;
        }
        match member {
            "StatusNotifierItemRegistered" => {
                let Ok(service) = proto::read_string(signature, body) else {
                    return Update::Unchanged;
                };
                let Ok((service, path)) = split_announced(sender, &service) else {
                    return Update::Unchanged;
                };
                // The registrant is not known here (the other watcher
                // announces, the app registered with it): the service.
                let registrant = service.clone();
                self.add(service, path, &registrant)
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
    pub(super) fn on_item_signal(&mut self, sender: &str, member: &str) -> Update {
        let Some(id) = self
            .items
            .iter()
            .find(|item| item.owner.as_deref() == Some(sender))
            .map(|item| item.id.clone())
        else {
            return Update::Unchanged;
        };
        match member {
            "NewTitle" | "NewIcon" | "NewAttentionIcon" | "NewOverlayIcon" | "NewToolTip"
            | "NewStatus" => {
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
    pub(super) fn on_call(
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
            self.conn
                .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownObject");
            return Update::Unchanged;
        }
        if interface == ITEM_PROPERTIES {
            return self.on_properties(sender, member, serial, signature, body);
        }
        if interface == "org.freedesktop.DBus.Introspectable" && member == "Introspect" {
            self.conn
                .reply_return(sender, serial, "s", &introspect_body());
            return Update::Unchanged;
        }
        if interface == "org.freedesktop.DBus.Peer" {
            if member == "Ping" {
                self.conn.reply_return(sender, serial, "", &[]);
                return Update::Unchanged;
            }
            self.conn
                .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        if interface != WATCHER_KDE && interface != WATCHER_FDO {
            self.conn
                .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        match member {
            "RegisterStatusNotifierItem" => {
                let Ok(service) = proto::read_string(signature, body) else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.InvalidArgs");
                    return Update::Unchanged;
                };
                let Ok((service, path)) = split_service_path(sender, &service) else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.InvalidArgs");
                    return Update::Unchanged;
                };
                self.conn.reply_return(sender, serial, "", &[]);
                self.add(service, path, sender)
            }
            "RegisterStatusNotifierHost" => {
                self.conn.reply_return(sender, serial, "", &[]);
                if !self.hosts.contains(&sender.to_owned()) {
                    self.hosts.push(sender.to_owned());
                    self.emit_host_registered();
                }
                Update::Unchanged
            }
            _ => {
                self.conn
                    .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
                Update::Unchanged
            }
        }
    }

    /// Answers `Properties.Get`/`GetAll` on the watcher object.
    pub(super) fn on_properties(
        &mut self,
        sender: &str,
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
            self.conn
                .reply_error(sender, serial, "org.freedesktop.DBus.Error.InvalidArgs");
            return Update::Unchanged;
        };
        if interface != WATCHER_KDE && interface != WATCHER_FDO {
            self.conn
                .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
            return Update::Unchanged;
        }
        let mut out = Writer::new();
        match (member, property.as_str()) {
            ("GetAll", _) => {
                let Some(cookie) = out.open_array(8) else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                write_watcher_props(self, &mut out);
                out.close_array(cookie);
                let Some(bytes) = out.take_body() else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(sender, serial, "a{sv}", &bytes);
            }
            ("Get", "RegisteredStatusNotifierItems") => {
                out.variant("as");
                let Some(cookie) = out.open_array(4) else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                for item in &self.items {
                    out.str(&item.id);
                }
                out.close_array(cookie);
                let Some(bytes) = out.take_body() else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(sender, serial, "v", &bytes);
            }
            ("Get", "IsStatusNotifierHostRegistered") => {
                out.variant("b");
                out.boolean(!self.hosts.is_empty());
                let Some(bytes) = out.take_body() else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(sender, serial, "v", &bytes);
            }
            ("Get", "ProtocolVersion") => {
                out.variant("i");
                out.i32(PROTOCOL_VERSION as i32);
                let Some(bytes) = out.take_body() else {
                    self.conn
                        .reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
                    return Update::Unchanged;
                };
                self.conn.reply_return(sender, serial, "v", &bytes);
            }
            _ => {
                self.conn
                    .reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
            }
        }
        Update::Unchanged
    }

    /// Adds an item (or re-reads one already shown), verifying it with a
    /// `GetNameOwner` first: a registration for a name nobody owns is
    /// answered and dropped.
    pub(super) fn add(&mut self, service: String, path: String, registrant: &str) -> Update {
        let id = format!("{service}{path}");
        if self.items.iter().any(|item| item.id == id) {
            self.refresh(&id);
            return Update::Unchanged;
        }
        if !self.has_room(&service, registrant) {
            self.say_full(&id);
            return Update::Unchanged;
        }
        self.items.push(Item::new(
            id.clone(),
            service.clone(),
            path,
            registrant.to_owned(),
        ));
        self.sort();
        if service.starts_with(':') {
            self.set_owner(&id, service);
            self.refresh(&id);
        } else {
            let mut get = Writer::new();
            get.str(&service);
            let Some(bytes) = get.take_body() else {
                return self.draw_now();
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
        self.draw_now()
    }

    /// Drops the item with `id` (or every id under a vanished service),
    /// saying so to other hosts in owner mode. An open menu of a dropped
    /// item closes with it.
    pub(super) fn remove(&mut self, id: &str) -> Update {
        let before = self.items.len();
        self.items
            .retain(|item| item.id != id && item.service != id);
        if self.items.len() == before {
            return Update::Unchanged;
        }
        if self.items.len() < MAX_ITEMS {
            self.said_full = false;
        }
        self.close_menu_if_vanished();
        if self.mode == Mode::Owner {
            self.emit_unregistered(id);
        }
        self.draw_now()
    }

    pub(super) fn set_owner(&mut self, id: &str, owner: String) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.owner = Some(owner);
        }
    }

    pub(super) fn sort(&mut self) {
        self.items.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// Queues a `RequestName` for the KDE name or its twin.
    pub(super) fn request(&mut self, name: &str, op: Op) {
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
    pub(super) fn match_rules(&mut self) {
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
    pub(super) fn read_watcher_owner(&mut self) {
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
    pub(super) fn register_host(&mut self) {
        let Some(owner) = self.watcher_owner.clone() else {
            return;
        };
        let mut body = Writer::new();
        body.str(self.conn.unique());
        let Some(bytes) = body.take_body() else {
            return;
        };
        self.fire(
            &owner,
            WATCHER_PATH,
            WATCHER_KDE,
            "RegisterStatusNotifierHost",
            "s",
            &bytes,
        );
    }

    /// Queues a `ListNames` to pick up what registered before us.
    pub(super) fn enumerate(&mut self) {
        if self
            .flights
            .iter()
            .any(|flight| matches!(flight, Some(Flight { op: Op::Names })))
        {
            return;
        }
        self.issue(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "ListNames",
            "",
            &[],
            0,
            Op::Names,
        );
    }

    /// Queues a read of the existing watcher's item list (host mode).
    pub(super) fn refresh_watcher(&mut self) {
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
        self.issue(
            &owner,
            WATCHER_PATH,
            ITEM_PROPERTIES,
            "Get",
            "ss",
            &bytes,
            0,
            Op::WatcherItems,
        );
    }

    pub(super) fn emit_registered(&mut self, id: &str) {
        let mut body = Writer::new();
        body.str(id);
        if let Some(bytes) = body.take_body() {
            self.conn.signal(
                WATCHER_PATH,
                WATCHER_KDE,
                "StatusNotifierItemRegistered",
                "s",
                &bytes,
            );
        }
    }

    pub(super) fn emit_unregistered(&mut self, id: &str) {
        let mut body = Writer::new();
        body.str(id);
        if let Some(bytes) = body.take_body() {
            self.conn.signal(
                WATCHER_PATH,
                WATCHER_KDE,
                "StatusNotifierItemUnregistered",
                "s",
                &bytes,
            );
        }
    }

    pub(super) fn emit_host_registered(&mut self) {
        self.conn.signal(
            WATCHER_PATH,
            WATCHER_KDE,
            "StatusNotifierHostRegistered",
            "",
            &[],
        );
    }

    /// Whether one more item is taken: under the total cap, and under the
    /// per-service one, counted by the service's name and by whoever
    /// registered it (one peer owning many names is still one peer).
    ///
    /// Decided, not changed (`tray-review-hardening`): a peer registering
    /// items under another app's service name crowds that name out — the
    /// count is by service OR registrant, and the victim's own later
    /// registrations count against the squatter's 8. That is the
    /// pre-existing semantics, and the per-registrant half is what bounds
    /// it: a peer spraying many names still holds 8 items at most, so the
    /// damage is one name's slots, never the tray's 32. Counting by AND
    /// instead would let one peer hold 8 under every name on the bus.
    pub(super) fn has_room(&self, service: &str, registrant: &str) -> bool {
        self.items.len() < MAX_ITEMS
            && self
                .items
                .iter()
                .filter(|item| item.service == service || item.registrant == registrant)
                .count()
                < MAX_PER_SERVICE
    }

    /// The `too many items` warning: once per full set, on stderr, never
    /// per wake.
    pub(super) fn say_full(&mut self, id: &str) {
        if !self.said_full {
            self.said_full = true;
            crate::print::warn(format_args!(
                "scootbar: tray: ignoring `{id}`: no room (at most {MAX_ITEMS} items, \
                 {MAX_PER_SERVICE} from one service)"
            ));
        }
    }

    /// How many items the bar draws.
    pub(super) fn shown_count(&self) -> usize {
        self.items.iter().filter(|item| item.shown()).count()
    }

    /// The item index (in id order) of the `n`th item the bar draws.
    pub(super) fn nth_shown(&self, n: usize) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.shown())
            .nth(n)
            .map(|(index, _)| index)
    }

    /// The item index `at` in id order, or `None` past the end.
    pub(super) fn at(&self, index: i32) -> Option<&Item> {
        (index >= 0)
            .then(|| self.items.get(index as usize))
            .flatten()
    }
}

/// Starts the bus live: queues both watcher names and returns at once.
/// Nothing here blocks past `Hello` (inside [`conn::setup`]): the names
/// are answered later, through [`Live::on_request_reply`], which installs
/// the match rules and enumerates. The module joins the bar on the first
/// frame either way, filling in as the bus answers.
pub(super) fn setup(conn: Conn) -> Live {
    let mut live = Live {
        conn,
        // Host until the KDE name answers otherwise: registrations reach
        // whoever owns the name, so nothing is answered prematurely.
        mode: Mode::Host,
        watcher_owner: None,
        items: Vec::new(),
        flights: Vec::new(),
        coalesce: None,
        drawn: None,
        held: None,
        // Our own host is first: `IsStatusNotifierHostRegistered` holds
        // from the start in owner mode.
        hosts: Vec::new(),
        said_full: false,
        // No menu open, and none asked for: both start shut.
        menu: None,
        #[cfg(feature = "popup")]
        menu_popup: false,
    };
    live.hosts.push(live.conn.unique().to_owned());
    live.request(WATCHER_KDE, Op::RequestKde);
    live.request(WATCHER_FDO, Op::RequestFdo);
    live
}

/// The match rules: owner changes, item signals on both interfaces,
/// menu updates on the DBusMenu one, and the other watcher's item
/// signals for host mode. Installed once the names answer (and again on
/// every re-acquire: a new connection has no rules, and re-adding to the
/// same one errors silently into the kept rule).
pub(super) const MATCH_RULES: [&str; 6] = [
    "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
     member='NameOwnerChanged',path='/org/freedesktop/DBus'",
    "type='signal',interface='org.kde.StatusNotifierItem'",
    "type='signal',interface='org.freedesktop.StatusNotifierItem'",
    "type='signal',interface='org.kde.StatusNotifierWatcher'",
    "type='signal',interface='org.freedesktop.StatusNotifierWatcher'",
    "type='signal',interface='com.canonical.dbusmenu'",
];

/// The introspection XML of the watcher object: the two watcher
/// interfaces, properties, introspection and ping — nothing else.
pub(super) fn introspect_body() -> Vec<u8> {
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
pub(super) fn write_watcher_props(live: &Live, out: &mut Writer) {
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
