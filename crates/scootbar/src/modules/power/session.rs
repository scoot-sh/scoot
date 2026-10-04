//! The live half of the power module: the logind conversation, with no
//! drawing in it.
//!
//! One connection to the system bus, no match rules, no polling. The
//! module asks logind's `CanSuspend`/`CanReboot`/`CanPowerOff` when its
//! popup opens (unknown until then, and shown: only an explicit `no` or
//! `na` hides a row), and calls `Suspend`/`Reboot`/`PowerOff` with
//! `interactive: true` when a confirmed row performs (polkit decides; a
//! refusal is kept as the module's last error, said on stderr, and shown
//! in its tooltip and `query`). Replies are matched by serial and sender
//! by the shared client, like every other call.
//!
//! ## What a peer can do
//!
//! logind is PID 1's own name on any real system; on a bus without it
//! anything can own `org.freedesktop.login1` and say anything. Like the
//! bluetooth module's BlueZ peer, it cannot crash or hang the bar: a
//! reply that does not parse leaves the last state, one that errors or
//! never comes keeps what was shown (and, for an action, records the
//! error). `Can*` answers other than `yes`/`challenge`/`no`/`na` are kept
//! as unknown (shown), and the error text kept for the tooltip is cut to
//! [`MAX_ERROR`] bytes.

use std::time::{Duration, Instant};

use super::FLIGHT_TTL;
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Session;
use crate::dbus::proto::Writer;

/// logind on the system bus.
pub(super) const NAME: &str = "org.freedesktop.login1";
/// How old a full `Can*` ask may be before the popup asks again:
/// inhibitors come and go (a backup tool, a video player), so an open
/// re-asks past this, never per refill. Always in tests, so scripted
/// answers replace each other within one run.
#[cfg(not(test))]
const CAN_STALE: Duration = Duration::from_secs(60);
#[cfg(test)]
const CAN_STALE: Duration = Duration::ZERO;
/// Its manager object.
pub(super) const PATH: &str = "/org/freedesktop/login1";
/// The manager's interface.
pub(super) const IFACE: &str = "org.freedesktop.login1.Manager";

/// The longest error text kept for the tooltip and `query`, in bytes.
pub(super) const MAX_ERROR: usize = 256;

/// What logind's `Can*` said for one action. Unknown (no answer yet, or
/// none usable) shows the row: only an explicit refusal hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Can {
    #[default]
    Unknown,
    Yes,
    No,
}

impl Can {
    /// Whether logind refused outright (`no`): only then is the row
    /// hidden. Every other answer — `yes`, `challenge`, unknown — shows
    /// it.
    pub(super) fn refused(self) -> bool {
        matches!(self, Self::No)
    }
}

/// A call in flight: what its reply is for.
struct Flight {
    op: Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    CanSuspend,
    CanReboot,
    CanPoweroff,
    Suspend,
    Reboot,
    Poweroff,
}

/// The logind session: what the `Can*` calls said, and the last action
/// error, if any.
pub(super) struct Live {
    conn: Conn,
    pub(super) can_suspend: Can,
    pub(super) can_reboot: Can,
    pub(super) can_poweroff: Can,
    flights: Vec<Option<Flight>>,
    /// The last suspend/reboot/shut-down refusal, bounded, for the
    /// tooltip and `query` (and said on stderr when it arrived).
    pub(super) last_error: Option<String>,
    /// When the `Can*` calls last went out: an open re-asks past
    /// [`CAN_STALE`], never per refill.
    asked_at: Option<Instant>,
    /// Whether a `Can*` answer was said already: once per connection,
    /// not per wake.
    said_malformed: bool,
}

impl Session for Live {
    fn conn(&self) -> &Conn {
        &self.conn
    }

    fn conn_mut(&mut self) -> &mut Conn {
        &mut self.conn
    }
}

/// Starts the session on a fresh connection: nothing is asked yet (the
/// `Can*` calls go out when the popup opens, so a bar whose menu is never
/// opened costs no round trips). Nothing blocks past the `Hello` inside
/// [`crate::dbus::conn::setup`].
pub(super) fn start(conn: Conn) -> Live {
    Live {
        conn,
        can_suspend: Can::Unknown,
        can_reboot: Can::Unknown,
        can_poweroff: Can::Unknown,
        flights: Vec::new(),
        last_error: None,
        asked_at: None,
        said_malformed: false,
    }
}

impl Live {
    /// Works one bus event. Says whether what is shown changed (a `Can*`
    /// answer, or a new action error).
    pub(super) fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Reply {
                token,
                signature,
                body,
            } => self.on_reply(token, &signature, &body),
            Event::CallError { token, name } => self.on_error(token, &name),
            Event::Dropped { token } => self.on_dropped(token),
            // Nothing is served here and no signals are asked for: a call
            // to the bar is a peer's mistake, and signals from logind
            // (such as `PrepareForShutdown`) carry nothing shown.
            Event::Signal { .. } | Event::MethodCall { .. } => false,
        }
    }

    /// Asks the `Can*` calls: unknown ones always, the rest when the
    /// last full ask is past [`CAN_STALE`]. Already in flight counts as
    /// asked, so a slow bus is not asked twice — and a refill while open
    /// does not re-ask what just went out.
    pub(super) fn refresh(&mut self) {
        let now = Instant::now();
        let full = self
            .asked_at
            .is_none_or(|at| now.duration_since(at) > CAN_STALE);
        let cans = [
            ("CanSuspend", Op::CanSuspend, self.can_suspend),
            ("CanReboot", Op::CanReboot, self.can_reboot),
            ("CanPowerOff", Op::CanPoweroff, self.can_poweroff),
        ];
        for (member, op, known) in cans {
            if (matches!(known, Can::Unknown) || full) && !self.in_flight(op) {
                self.ask_can(member, op);
            }
        }
        if full {
            self.asked_at = Some(now);
        }
    }

    /// Whether a call for `want` is already in flight.
    fn in_flight(&self, want: Op) -> bool {
        self.flights
            .iter()
            .flatten()
            .any(|flight| flight.op == want)
    }

    /// Calls `Suspend`/`Reboot`/`PowerOff` with `interactive: true`
    /// (polkit may ask, through its own agent). `false` when the call was
    /// not queued (the table is full of live calls). The caller has
    /// already disarmed the row, so the next click arms it again: never a
    /// half-performed action.
    pub(super) fn perform(&mut self, row: super::Row) -> bool {
        let member = match row {
            super::Row::Suspend => "Suspend",
            super::Row::Reboot => "Reboot",
            super::Row::Poweroff => "PowerOff",
            super::Row::Lock | super::Row::Logout => return false,
        };
        let op = match row {
            super::Row::Suspend => Op::Suspend,
            super::Row::Reboot => Op::Reboot,
            super::Row::Poweroff => Op::Poweroff,
            super::Row::Lock | super::Row::Logout => return false,
        };
        let Some(body) = bool_body(true) else {
            return false;
        };
        self.issue(NAME, PATH, IFACE, member, "b", &body, op)
    }

    fn ask_can(&mut self, member: &str, op: Op) {
        self.issue(NAME, PATH, IFACE, member, "", &[], op);
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
    /// freeing what they held: a `Can*` never answered is asked again
    /// when the popup opens, and an action never answered keeps what was
    /// shown.
    fn reap(&mut self) {
        for token in self.conn.expire(FLIGHT_TTL) {
            let slot = token as usize;
            if self.flights.get_mut(slot).and_then(Option::take).is_some() {
                // The flight's answer is gone with it; nothing shown
                // changes (unknown stays shown, an action stays unrecorded).
            }
        }
    }

    fn on_reply(&mut self, token: u64, signature: &str, body: &[u8]) -> bool {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return false;
        };
        match flight.op {
            Op::CanSuspend | Op::CanReboot | Op::CanPoweroff => {
                let Some(answer) = read_can(signature, body) else {
                    self.say_malformed_once();
                    return false;
                };
                let can = match answer.as_str() {
                    // `challenge` needs authentication, which the action
                    // call (with `interactive: true`) drives: the row is
                    // shown, like an allowed one.
                    "yes" | "challenge" => Can::Yes,
                    "no" | "na" => Can::No,
                    // logind documents only these four; anything else is
                    // kept as unknown (shown), never hidden on a guess.
                    _ => {
                        self.say_malformed_once();
                        return false;
                    }
                };
                let slot = match flight.op {
                    Op::CanSuspend => &mut self.can_suspend,
                    Op::CanReboot => &mut self.can_reboot,
                    _ => &mut self.can_poweroff,
                };
                if *slot == can {
                    return false;
                }
                *slot = can;
                true
            }
            Op::Suspend | Op::Reboot | Op::Poweroff => {
                if !signature.is_empty() || !body.is_empty() {
                    self.say_malformed_once();
                    return false;
                }
                // Answered: a refusal would have errored instead. A past
                // error showing is cleared.
                if self.last_error.take().is_some() {
                    return true;
                }
                false
            }
        }
    }

    fn on_error(&mut self, token: u64, name: &str) -> bool {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return false;
        };
        match flight.op {
            // A `Can*` that errored (an old logind without it, a denied
            // introspection) keeps what was shown: unknown shows the row,
            // and the action's own refusal, if one follows, says why.
            Op::CanSuspend | Op::CanReboot | Op::CanPoweroff => false,
            Op::Suspend | Op::Reboot | Op::Poweroff => {
                let what = match flight.op {
                    Op::Suspend => "suspend",
                    Op::Reboot => "reboot",
                    _ => "shut down",
                };
                self.fail(format_args!("logind refused {what}: {name}"))
            }
        }
    }

    fn on_dropped(&mut self, token: u64) -> bool {
        let Some(flight) = self.flights.get_mut(token as usize).and_then(Option::take) else {
            return false;
        };
        match flight.op {
            Op::CanSuspend | Op::CanReboot | Op::CanPoweroff => false,
            Op::Suspend | Op::Reboot | Op::Poweroff => {
                let what = match flight.op {
                    Op::Suspend => "suspend",
                    Op::Reboot => "reboot",
                    _ => "shut down",
                };
                self.fail(format_args!(
                    "logind gave no usable answer to {what} (its reply was too large to read)"
                ))
            }
        }
    }

    /// Records an action failure: kept for the tooltip and `query`, and
    /// said on stderr now, never silently. Always `true`: a new error is
    /// always shown.
    fn fail(&mut self, message: std::fmt::Arguments<'_>) -> bool {
        use std::fmt::Write as _;
        let mut text = String::new();
        let _ = text.write_fmt(message);
        if text.len() > MAX_ERROR {
            let mut cut = MAX_ERROR;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        crate::print::warn(format_args!("scootbar: power: {text}"));
        self.last_error = Some(text);
        true
    }

    /// A reply that does not parse, said once per connection: a hostile
    /// peer answering shapes the bar refuses is routine, not per-wake
    /// news.
    fn say_malformed_once(&mut self) {
        if self.said_malformed {
            return;
        }
        self.said_malformed = true;
        crate::print::warn(format_args!(
            "scootbar: power: logind answered what was not asked; keeping what is shown"
        ));
    }
}

/// A body of one boolean (logind's `interactive`).
fn bool_body(value: bool) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.boolean(value);
    body.take_body()
}

/// Reads a `Can*` answer: one string, and nothing after it.
fn read_can(signature: &str, body: &[u8]) -> Option<String> {
    if signature != "s" {
        return None;
    }
    let mut reader = crate::dbus::proto::Reader::le(body);
    let answer = reader.str().ok()?;
    if !reader.exhausted() {
        return None;
    }
    Some(answer.to_owned())
}
