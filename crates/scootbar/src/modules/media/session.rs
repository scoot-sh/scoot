//! The live half of the media module: the bus conversation, with no
//! drawing in it.
//!
//! One connection, two match rules, no polling. `NameOwnerChanged` for the
//! `org.mpris.MediaPlayer2` namespace says when a player appears and
//! vanishes (the bus releases a dead player's names itself, so a player
//! that crashes without saying goodbye is gone with its connection), and
//! `PropertiesChanged` on the one MPRIS object says when its track or state
//! moves. Both are filtered by the bus: an unrelated app coming or going
//! never wakes the bar, and neither does a player's position (which has no
//! change signal and is deliberately not read).
//!
//! Every call is asynchronous past the set-up: a flight table maps replies
//! back to what asked, and a player that never answers is forgotten by age
//! when its slot is wanted (no bus times a call out by default: see
//! `Conn::expire`). A reply names the player it was asked of by an id, not
//! a name, so an answer meant for a player that left and came back is
//! dropped.
//!
//! ## What a peer can do
//!
//! Anything on the session bus can claim an MPRIS name and say anything. It
//! cannot crash or hang the bar, grow it without bound, or make it believe
//! another player's state: strings are cleaned and cut where they are stored,
//! a reply that does not parse leaves the player's last state, one that errors
//! leaves the player held and unshown (read again at its next signal), one
//! that never comes is forgotten, and `NameOwnerChanged` is believed only from
//! the bus. At most [`MAX_PLAYERS`] are held, one name per connection; a
//! newcomer to a full room takes the place of the oldest stopped player that
//! has been read, else waits in a list of [`MAX_WAITING`] (the oldest
//! forgotten first). What a hostile peer can still do is bounded loss of
//! visibility: live players filling the table and the list hide a later one
//! until a slot frees. A signal is applied
//! only from the connection that owns a held name.

use std::collections::VecDeque;
use std::time::Instant;

use super::player::{Player, Status, select};
use super::timer::OneShot;
use super::{FLIGHT_TTL, LOOKUP_WINDOW, MAX_PLAYERS, MAX_WAITING, MIN_REFRESH_GAP};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Session;
use crate::dbus::mpris::{self, NAME_PREFIX, PATH, PLAYER};
use crate::dbus::proto::{self, Writer};

/// The Properties interface: `GetAll` and the signal.
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The match rules, filtered by the bus (both daemons implement
/// `arg0namespace` and `arg0`): owner changes of MPRIS names only, and the
/// Player interface's property signals on the MPRIS object only. A
/// player's `Seeked` and its other interfaces never reach the bar.
const MATCH_RULES: [&str; 2] = [
    "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
     member='NameOwnerChanged',path='/org/freedesktop/DBus',\
     arg0namespace='org.mpris.MediaPlayer2'",
    "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',\
     path='/org/mpris/MediaPlayer2',arg0='org.mpris.MediaPlayer2.Player'",
];

/// A call in flight: what its reply is for.
struct Flight {
    op: Op,
}

enum Op {
    /// An `AddMatch`: only its refusal matters.
    Match,
    /// `ListNames`: the players already there.
    Names,
    /// `GetNameOwner` for a listed name.
    Owner(String),
    /// `GetAll` for the player with this id.
    Props(u64),
}

/// The connection and what the module knows of the players on it.
pub(super) struct Live {
    conn: Conn,
    pub(super) players: Vec<Player>,
    flights: Vec<Option<Flight>>,
    /// Listed MPRIS names whose owner is not asked yet: at most
    /// [`LOOKUP_WINDOW`] questions are in flight, so a connection owning
    /// hundreds of names costs a window of calls at a time and never
    /// starves a real player listed behind them.
    lookups: VecDeque<String>,
    /// `(name, owner, evicted)` of players that could not be held (no room,
    /// or the second name of a connection), at most [`MAX_WAITING`]: held
    /// when a slot frees, or (unless `evicted`, which made room itself by
    /// being stopped) when a held player stops.
    waiting: VecDeque<(String, String, bool)>,
    next_id: u64,
    /// The activity clock: ticks each time a player starts playing.
    tick: u64,
    /// Reused for every cleaned string, so a track change allocates
    /// nothing in steady state.
    scratch: String,
    /// A one-shot timer, armed only while a player waits out
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
/// `ListNames`, so a player that appears between the two is in one of
/// them. Nothing blocks past the `Hello` inside [`conn::setup`].
pub(super) fn start(conn: Conn) -> Live {
    let mut live = Live {
        conn,
        players: Vec::new(),
        flights: Vec::new(),
        lookups: VecDeque::new(),
        waiting: VecDeque::new(),
        next_id: 0,
        tick: 0,
        scratch: String::new(),
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
    live.issue(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        "ListNames",
        "",
        &[],
        Op::Names,
    );
    live
}

/// A body of one string.
fn one_string(text: &str) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.str(text);
    body.take_body()
}

impl Live {
    /// Works one bus event.
    pub(super) fn apply(&mut self, event: Event) {
        self.work(event);
        // Whatever the event freed (a lookup answered, a flight forgotten)
        // lets the next name be asked.
        if !self.lookups.is_empty() {
            self.pump_lookups();
        }
    }

    fn work(&mut self, event: Event) {
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
            // Nothing is served here: a call to the bar is a peer's
            // mistake, and an unanswered one is the caller's to time out.
            Event::MethodCall { .. } => {}
        }
    }

    /// Queues a call that wants a reply, tracking its flight: `false`
    /// when it was not queued (the table is full of live calls). Seven
    /// arguments: a call names its destination, object, interface, member
    /// and body, like the header it becomes.
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
    /// freeing what they held: a player whose `GetAll` never answered can
    /// be asked again.
    fn reap(&mut self) {
        for token in self.conn.expire(FLIGHT_TTL) {
            let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
                continue;
            };
            if let Op::Props(id) = flight.op {
                if let Some(player) = self.players.iter_mut().find(|p| p.id == id) {
                    player.asked = None;
                    player.stale = false;
                }
            }
        }
    }

    /// Fires a call that wants no reply (a control): queued, never tracked.
    pub(super) fn control(&mut self, index: usize, member: &str) {
        let Some(player) = self.players.get(index) else {
            return;
        };
        let _ = self.conn.call(
            &player.owner,
            PATH,
            PLAYER,
            member,
            "",
            &[],
            proto::flag::NO_REPLY_EXPECTED,
            0,
        );
    }

    /// The index of the player to show (see [`select`]).
    pub(super) fn selected(&self, preferred: Option<&str>) -> Option<usize> {
        select(&self.players, preferred)
    }

    /// Reads a player's properties: one `GetAll` at a time, no oftener
    /// than [`MIN_REFRESH_GAP`]. A request while one is in flight, or too
    /// soon, marks the player stale and the answer (or the timer) asks
    /// again, so a player that announces as fast as it is read costs 20
    /// round trips a second, not a thousand.
    fn refresh(&mut self, id: u64) {
        let Some(player) = self.players.iter().find(|p| p.id == id) else {
            return;
        };
        if player.asked.is_some_and(|at| at.elapsed() > FLIGHT_TTL) {
            // Asked long ago and never answered: forgotten, and asked
            // again below (a player that is merely slow answers the new
            // call too).
            self.reap();
        }
        let Some(player) = self.players.iter_mut().find(|p| p.id == id) else {
            return;
        };
        if player.asked.is_some() {
            player.stale = true;
            return;
        }
        if player
            .last_asked
            .is_some_and(|at| at.elapsed() < MIN_REFRESH_GAP)
        {
            player.stale = true;
            self.arm_coalesce();
            return;
        }
        let owner = player.owner.clone();
        player.last_asked = Some(Instant::now());
        player.asked = Some(Instant::now());
        let Some(body) = one_string(PLAYER) else {
            return;
        };
        let queued = self.issue(
            &owner,
            PATH,
            PROPERTIES,
            "GetAll",
            "s",
            &body,
            Op::Props(id),
        );
        if !queued {
            if let Some(player) = self.players.iter_mut().find(|p| p.id == id) {
                player.asked = None;
            }
        }
    }

    /// Arms the one-shot timer that reads again the players waiting out
    /// the gap (a no-op while armed). Without a timer (the fd could not be
    /// made) they are read at their next signal: the slower answer, never
    /// the wrong one.
    fn arm_coalesce(&mut self) {
        if self.coalesce.is_none() {
            self.coalesce = OneShot::after(MIN_REFRESH_GAP);
        }
    }

    /// The gap passed: reads again the players that waited.
    pub(super) fn on_coalesce(&mut self) {
        if let Some(timer) = self.coalesce.take() {
            timer.drain();
        }
        let waiting: Vec<u64> = self
            .players
            .iter()
            .filter(|p| p.stale && p.asked.is_none())
            .map(|p| p.id)
            .collect();
        for id in waiting {
            if let Some(player) = self.players.iter_mut().find(|p| p.id == id) {
                player.stale = false;
            }
            self.refresh(id);
        }
    }

    fn on_reply(&mut self, token: u64, signature: &str, body: &[u8]) {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        match flight.op {
            Op::Match => {}
            Op::Names => self.on_names(signature, body),
            Op::Owner(name) => {
                if let Ok(owner) = proto::read_owner(signature, body) {
                    self.add(name, owner);
                }
            }
            Op::Props(id) => {
                let stale = self.finish_fetch(id);
                self.on_props(id, signature, body);
                if stale {
                    self.refresh(id);
                }
            }
        }
    }

    /// A reply past the size this client reads was skipped: the call is
    /// answered with nothing usable. The player keeps its last state and is
    /// read again at its next signal (`stale` is not honored: asking at
    /// once would fetch the same oversized answer).
    fn on_dropped(&mut self, token: u64) {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        if let Op::Props(id) = flight.op {
            self.finish_fetch(id);
        }
    }

    fn on_error(&mut self, token: u64, name: &str) {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return;
        };
        match flight.op {
            // An error, of any kind (a busy player's timeout, a player that
            // has the name before it exports the object): the player stays
            // held but unread, shows nothing, and is read again at its next
            // signal. Bounded by the reads' own floor, and the oldest stopped
            // player is the first to go when the room is wanted.
            Op::Props(id) => {
                self.finish_fetch(id);
                if let Some(player) = self.players.iter_mut().find(|p| p.id == id) {
                    player.unread = true;
                }
            }
            Op::Match => {
                if !self.said_match {
                    self.said_match = true;
                    crate::print::warn(format_args!(
                        "scootbar: media: the bus refused a match rule ({name}); \
                         players may not show"
                    ));
                }
            }
            // The name left between the listing and the question.
            Op::Owner(_) | Op::Names => {}
        }
    }

    /// An answer to a player's `GetAll`: applied, or dropped whole when it
    /// does not parse (the player keeps its last state). A reply for a
    /// player that left finds nothing.
    fn on_props(&mut self, id: u64, signature: &str, body: &[u8]) {
        if signature != "a{sv}" {
            return;
        }
        let Ok(props) = mpris::read_player_props(body) else {
            return;
        };
        let Some(player) = self.players.iter_mut().find(|p| p.id == id) else {
            return;
        };
        player.unread = false;
        player.read = true;
        player.apply(&props, &mut self.scratch, &mut self.tick);
        self.admit_waiting();
    }

    /// The call is answered: it may be asked again, and whether a signal
    /// arrived meanwhile.
    fn finish_fetch(&mut self, id: u64) -> bool {
        match self.players.iter_mut().find(|p| p.id == id) {
            Some(player) => {
                player.asked = None;
                std::mem::take(&mut player.stale)
            }
            None => false,
        }
    }

    /// Works a `ListNames` answer: queues each MPRIS name that is not held,
    /// to have its owner asked ([`Live::pump_lookups`]), so the player
    /// behind it can be told from a name that left.
    fn on_names(&mut self, signature: &str, body: &[u8]) {
        let Ok(names) = proto::read_names(signature, body) else {
            return;
        };
        for name in names {
            // `ListNames` lists each name once, so no name is queued twice.
            if is_player_name(&name) && !self.players.iter().any(|p| p.name == name) {
                self.lookups.push_back(name);
            }
        }
        self.pump_lookups();
    }

    /// Asks the owner of queued names, [`LOOKUP_WINDOW`] at a time. Full
    /// or not, every listed name is asked (the queue is bounded by what
    /// `ListNames` returns): what [`Live::add`] does with the answer when no
    /// slot is free is its rule, so a name queued behind the held ones is
    /// not lost to them.
    fn pump_lookups(&mut self) {
        loop {
            let asking = self
                .flights
                .iter()
                .flatten()
                .filter(|flight| matches!(flight.op, Op::Owner(_)))
                .count();
            if asking >= LOOKUP_WINDOW {
                return;
            }
            let Some(name) = self.lookups.pop_front() else {
                return;
            };
            let Some(body) = one_string(&name) else {
                continue;
            };
            let queued = self.issue(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "GetNameOwner",
                "s",
                &body,
                Op::Owner(name.clone()),
            );
            if !queued {
                // No slot free (the table is full of live calls): the name
                // waits for the next event to free one.
                self.lookups.push_front(name);
                return;
            }
        }
    }

    /// Holds the player `name` owned by `owner`, and reads it. A name
    /// already held changes hands (the new process's state is not the old
    /// one's). A connection already holding another name is left to it (one
    /// player a connection), and with [`MAX_PLAYERS`] held a newcomer takes
    /// the place of the oldest stopped player, if there is one; with none, or
    /// for the second name of a connection, it waits in a short list
    /// ([`MAX_WAITING`], the oldest forgotten first) and is held when a slot
    /// frees: a player never has to restart to be seen because eight others
    /// were there first.
    fn add(&mut self, name: String, owner: String) {
        self.waiting.retain(|(waiting, _, _)| *waiting != name);
        if let Some(player) = self.players.iter_mut().find(|p| p.name == name) {
            if player.owner != owner {
                // A new process: not the old one's state, and not the old
                // one's answers either (a new id, so a reply still in flight
                // from the old owner finds nothing to land on).
                self.next_id += 1;
                player.id = self.next_id;
                player.owner = owner;
                player.reset();
                player.asked = None;
                player.stale = false;
                let id = player.id;
                self.refresh(id);
            }
            return;
        }
        if self.players.iter().any(|p| p.owner == owner) {
            self.wait(name, owner, false);
            return;
        }
        if self.players.len() >= MAX_PLAYERS {
            // The oldest stopped player (one that has been read: an unread
            // one is not known to be stopped) shows nothing and is the
            // cheapest to lose; it waits for a slot like any other.
            let oldest = self
                .players
                .iter()
                .filter(|p| p.status == Status::Stopped && (p.read || p.unread))
                .map(|p| p.id)
                .min();
            match oldest {
                Some(id) => {
                    // Not lost: it waits for a slot like any other.
                    if let Some(gone) = self.players.iter().find(|p| p.id == id) {
                        let (name, owner) = (gone.name.clone(), gone.owner.clone());
                        self.wait(name, owner, true);
                    }
                    self.players.retain(|p| p.id != id);
                }
                None => {
                    self.say_full(&name);
                    self.wait(name, owner, false);
                    return;
                }
            }
        }
        self.next_id += 1;
        let id = self.next_id;
        self.players.push(Player::new(id, name, owner));
        self.refresh(id);
    }

    /// Remembers a name that cannot be held now, bounded: past
    /// [`MAX_WAITING`] the oldest is forgotten.
    fn wait(&mut self, name: String, owner: String, evicted: bool) {
        if self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_front();
        }
        self.waiting.push_back((name, owner, evicted));
    }

    /// A held player stopped: if names wait for room and it (or another
    /// read, stopped one) can make it, the first that did not itself make
    /// room by stopping takes the place of the oldest. One name a stop, so a
    /// run of stops is a run of one-for-one swaps, each consuming a waiting
    /// name for good.
    fn admit_waiting(&mut self) {
        let room_to_make = self
            .players
            .iter()
            .any(|p| p.status == Status::Stopped && (p.read || p.unread));
        if !room_to_make || self.players.len() < MAX_PLAYERS {
            return;
        }
        if let Some(at) = self.waiting.iter().position(|(_, _, evicted)| !evicted) {
            if let Some((name, owner, _)) = self.waiting.remove(at) {
                self.add(name, owner);
            }
        }
    }

    fn remove_id(&mut self, id: u64) {
        self.players.retain(|p| p.id != id);
        if self.players.len() < MAX_PLAYERS {
            self.said_full = false;
        }
        // Each name that was waiting gets one try at the room.
        for _ in 0..self.waiting.len() {
            let Some((name, owner, _)) = self.waiting.pop_front() else {
                break;
            };
            self.add(name, owner);
        }
    }

    fn say_full(&mut self, name: &str) {
        if !self.said_full {
            self.said_full = true;
            crate::print::warn(format_args!(
                "scootbar: media: ignoring `{name}`: no room (at most {MAX_PLAYERS} players, \
                 one a connection)"
            ));
        }
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
            // Only the bus says who owns what: a peer can send this signal
            // to the bar, addressed, and it is not believed.
            if sender == conn::BUS_NAME {
                self.on_name_owner_changed(signature, body);
            }
        } else if interface == PROPERTIES && member == "PropertiesChanged" {
            self.on_properties_changed(sender, path, signature, body);
        }
    }

    /// A player's state moved: applied from the connection that owns a
    /// held name, and only from it (any peer can send this signal).
    fn on_properties_changed(&mut self, sender: &str, path: &str, signature: &str, body: &[u8]) {
        if path != PATH || signature != "sa{sv}as" {
            return;
        }
        let Some(index) = self.players.iter().position(|p| p.owner == sender) else {
            return;
        };
        let Ok(changed) = mpris::read_properties_changed(body) else {
            return;
        };
        if changed.interface != PLAYER {
            return;
        }
        let id = self.players[index].id;
        self.players[index].apply(&changed.props, &mut self.scratch, &mut self.tick);
        self.admit_waiting();
        if changed.invalidated || self.players[index].unread {
            self.refresh(id);
        }
    }

    /// Owner tracking: an MPRIS name gained an owner (a player to hold),
    /// changed hands, or lost its owner (the player is gone, whether it
    /// said goodbye or died: the bus releases a dead connection's names).
    fn on_name_owner_changed(&mut self, signature: &str, body: &[u8]) {
        if signature != "sss" {
            return;
        }
        let Ok((name, _old, new)) = proto::read_name_owner_changed(body) else {
            return;
        };
        if !is_player_name(&name) {
            return;
        }
        match new {
            None => {
                self.waiting.retain(|(waiting, _, _)| *waiting != name);
                if let Some(id) = self.players.iter().find(|p| p.name == name).map(|p| p.id) {
                    self.remove_id(id);
                }
            }
            Some(owner) => self.add(name, owner),
        }
    }
}

/// Whether `name` is a player's well-known name: the MPRIS prefix and
/// something after it.
fn is_player_name(name: &str) -> bool {
    name.len() > NAME_PREFIX.len() && name.starts_with(NAME_PREFIX)
}
