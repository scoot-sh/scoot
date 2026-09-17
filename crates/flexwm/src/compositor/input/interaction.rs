//! The recent input events that count as the user asking for something, and
//! *who* received each one.
//!
//! Two consumers, with deliberately different bars:
//!
//! - `xdg-activation-v1` (`activation.rs`) spends only key and button
//!   serials. The protocol lets a client attach the seat and serial of the
//!   input event that caused it to ask for a token
//!   (`xdg_activation_token_v1.set_serial`), and the compositor is meant to
//!   refuse a token whose serial does not name a real, recent interaction --
//!   that is the only thing standing between "a launcher hands focus to the app
//!   it just started" and "any background client takes the keyboard whenever it
//!   likes". See `activation.rs` for the policy this feeds.
//! - `xdg_popup.grab` (`popup.rs`) spends those *and* pointer/keyboard
//!   `enter` serials. Qt's `QWaylandInputDevice::serial()` -- which is what a
//!   Qt client passes to `grab` -- is updated on `pointer_enter` (verified
//!   against Qt 6.8's `qwaylandinputdevice.cpp`: `Pointer::pointer_enter`
//!   assigns `mParent->mSerial`; `keyboard_key`, `pointer_button` and
//!   `touch_down` do the same, while `keyboard_enter` and `modifiers`
//!   deliberately do not), so a strict key-and-button check refuses a
//!   legitimate Qt menu whenever the toolkit last saw an enter -- typically a
//!   menu opened by hovering, which has had no button or key event to draw a
//!   fresh serial from. A refused grab is silent (no protocol error), so that
//!   failure mode is "this app's menus don't take the keyboard and nobody
//!   knows why".
//!
//! # Why the client identity is stored too, and is not optional
//!
//! A serial on its own is just a number out of [`SERIAL_COUNTER`], which is
//! process-global and shared by far more than input: the pinned Smithay rev
//! draws `xdg_surface.configure` and `xdg_popup.configure` serials from the
//! very same counter (`src/wayland/shell/xdg/mod.rs`). So any client can read
//! the counter's live value for free and unobserved -- create an
//! `xdg_surface`, commit it with no buffer so it never maps, and read the
//! serial of the configure it gets back -- and then guess. Guessing costs
//! nothing: a refused token posts no protocol error and Smithay still sends
//! `done`, so a client can pipeline a token per candidate offset and simply
//! see which one works. With a value-only check, a couple of dozen guesses
//! spanning the counter while the user types anywhere would eventually land
//! on a real serial and steal focus.
//!
//! Recording *who the event was delivered to* closes that, and is what makes
//! this a check on interaction rather than on arithmetic: an entry is only
//! spendable by the client that genuinely received that event, so a client
//! that was never focused (or under the pointer) has nothing to guess *with*,
//! however many numbers it tries. The identity comes from
//! [`XdgActivationTokenData::client_id`], which Smithay fills in from the
//! client that actually sent the request and no client can forge.
//!
//! "Delivered" is meant literally, and the call sites in `input.rs` keep it
//! that way. Nothing is recorded for:
//!
//! - a key a keybinding swallowed -- its serial is one the focused client
//!   never saw but could guess from the forwarded modifier presses around it;
//! - an event with no recipient at all -- a key with nothing focused, a click
//!   on bare desktop;
//! - a key Smithay *absorbs* rather than delivers, which the pinned rev's
//!   `KbdInternal::key_input` does for a press of a key already held and for
//!   a release of a key it has no record of. The second is not theoretical:
//!   under `--nested`, `nested_dispatch` forwards key events but not the
//!   held-key array in `wl_keyboard.enter`, so a modifier held while entering
//!   flexwm's window arrives as a lone release; `--tty` produces the same
//!   shape when a press happens while the session is paused. `is_transition`
//!   is not exposed, so `key()` tracks held keycodes itself to tell the
//!   difference (see [`State::key`]).
//!
//! The one place the claim is not yet literal is an input method holding a
//! keyboard grab (`zwp_input_method_v2.grab_keyboard`): Smithay's grab sends
//! keys to the IME alone and leaves the seat's focus untouched, so the
//! focused client is credited for keys it did not receive. That is not an
//! escalation -- the window the user is typing into is still the one credited,
//! which is the gate's intent -- but it is a divergence, filed as
//! `docs/backlog/protocols/interaction-serial-ime-grab.md`.
//!
//! # Why a ring rather than "the last serial"
//!
//! A single remembered serial cannot answer the question correctly, in
//! either direction:
//!
//! - A keyboard-driven launcher acts on the *press* and mints its token
//!   from that serial -- but `State::press` sends the press and the matching
//!   release back to back within one call, with no dispatch in between, so
//!   by the time the client's `commit` is handled the last serial issued is
//!   the *release*'s. (`flexwm msg key Return` is exactly this, and it is
//!   how the real-launcher flow in
//!   `docs/backlog/resolved/activation-serial-validation-done.md` was
//!   driven: the compositor issued 6 and 7, and the launcher's token carried
//!   6.)
//! - A pointer-driven one (a GTK button activates on release, not press)
//!   mints from the release serial, which a press-only tracker never saw.
//!
//! So what has to be remembered is a short *history* of qualifying events,
//! and a token is honored if its serial and client are one of them, and it is
//! recent (see [`INTERACTION_WINDOW`]). Exact match against remembered
//! values, never a range: pointer-motion serials that delivered no `enter`
//! are drawn from the same global counter and fall between the remembered
//! ones, so "between the oldest and the newest" would quietly admit the
//! passive events this exists to exclude.
//!
//! # Why `enter` serials are recorded at all, and what that costs
//!
//! An `enter` is *not* a user action, and recording it weakens the check --
//! eyes open. What remains closed, and what does not:
//!
//! - A client that was never given anything -- never focused, never under
//!   the pointer -- still has nothing to spend, however many numbers it
//!   guesses. The client half of every entry is what closes the
//!   configure-serial guessing attack, and enters keep it: an enter names the
//!   client that actually received it, resolved at delivery time like every
//!   other entry.
//! - What an enter *does* buy is any client that merely received one: mapping
//!   a window earns a keyboard `enter` (flexwm focuses every new window
//!   itself), and moving the pointer across one earns a pointer `enter`. For
//!   up to [`INTERACTION_WINDOW`] afterwards that client can take the keyboard
//!   through a popup grab with no click or keystroke.
//! - That is accepted because the grab itself is bounded in ways activation
//!   is not: the session lock and an `exclusive` layer surface pre-empt and
//!   refuse it, clicking outside dismisses it, keybindings run before it, and
//!   a refusal is silent while a *grant* is visible (a menu appears, and
//!   `flexwm msg windows` reports `popup_grab` on its holder). Activation
//!   keeps the stricter bar -- a focus serial there spends nothing -- because
//!   a stolen activation is invisible and leaves no such trace.
//!
//! Motion serials stay excluded: motion is continuous (libinput reports it at
//! 500-1000Hz, and `refresh_pointer_focus` synthesizes it with no user
//! involvement), so recording them would hand every client a
//! permanently-refreshing serial. Only a motion that actually *changes* focus
//! -- i.e. delivers an `enter` -- is recorded, and only the enter's serial,
//! which is bounded by focus changes rather than by the event rate.
//!
//! Fixed-size and stored inline in [`State`], with no heap indirection of
//! its own: this is written from the input path, on every key and button
//! event, and must not allocate.
//!
//! [`SERIAL_COUNTER`]: smithay::utils::SERIAL_COUNTER
//! [`State`]: super::State
//! [`State::key`]: super::State::key
//! [`XdgActivationTokenData::client_id`]: smithay::wayland::xdg_activation::XdgActivationTokenData::client_id

use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::backend::ClientId;
use smithay::utils::Serial;

/// How many qualifying events are remembered.
///
/// Sized off the largest burst one user action can produce, so that the
/// serial a client legitimately captured cannot be evicted before the
/// client's own request makes it back to the compositor: a full four-modifier
/// combo through [`State::press`] is ten events (four modifier presses, the
/// key's press and release, four modifier releases), and `type_text` spends
/// two to six per character. Sixteen covers that with room to spare.
///
/// It is a bound on *count* and nothing else -- how long an entry may be
/// spent for is [`INTERACTION_WINDOW`]'s job. An idle session rotates nothing
/// out at all, which is exactly why the age bound has to exist separately.
///
/// `pub(crate)` only so `activation/tests.rs` can say "older than the whole
/// history" without hard-coding a number that would silently stop meaning
/// that if this changed.
///
/// [`State::press`]: super::State::press
pub(crate) const CAPACITY: usize = 16;

/// How long after the event itself a grab or token may still be minted
/// from it.
///
/// `pub(crate)` so the popup-grab grace (`popup.rs`) can state its bound
/// relative to this one rather than as a bare number.
///
/// The count bound above cannot do this job: it only evicts when *newer*
/// qualifying input arrives, and a session can go hours without any. That is
/// not a corner case here but flexwm's own computer-use path -- an agent
/// drives windows through `Request::Action`, which goes straight to
/// `State::act` and never through `key`/`pointer_button` -- so without an age
/// bound a click from this morning would still be spendable this evening, and
/// the app clicked then could yank focus off whatever the agent had arranged.
/// `TOKEN_LIFETIME` does not cover it: that bounds creation to redemption,
/// not interaction to creation.
///
/// Ten seconds is three orders of magnitude more than the legitimate case
/// needs -- a launcher mints its token inside its own input handler, within
/// milliseconds -- with room for a client that was descheduled or waiting on
/// something slow, while keeping "the user just did this" true.
pub(crate) const INTERACTION_WINDOW: Duration = Duration::from_secs(10);

/// One qualifying event: which serial it carried, who it was delivered to,
/// and when.
#[derive(Clone, Debug)]
struct Delivered {
    serial: Serial,
    client: ClientId,
    kind: Kind,
    at: Instant,
}

/// What kind of delivery an entry records: a real user action, or a focus
/// change the client legitimately saw but the user never made.
///
/// The distinction is the two consumers' different bars (see the module
/// doc): activation spends [`Kind::Input`] only, while a popup grab spends
/// either. There is deliberately no third kind for pointer motion without a
/// focus change -- that is continuous and passive, and recording it would
/// hand every client a permanently-refreshing serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A key or button press or release delivered to the client.
    Input,
    /// A pointer or keyboard `enter` delivered to the client.
    Focus,
}

/// The most recent qualifying input events, newest overwriting oldest.
#[derive(Debug, Default)]
pub(crate) struct Recent {
    /// `None` only before the ring has been filled once -- a fresh session
    /// that has seen no input at all matches nothing, which is the case the
    /// activation gate exists for.
    events: [Option<Delivered>; CAPACITY],
    /// Where the next event goes. Always `< CAPACITY` (see [`Self::record`]),
    /// so indexing with it cannot panic.
    next: usize,
}

impl Recent {
    /// Remembers one event, evicting the oldest once full.
    ///
    /// `client` is whoever the event is being delivered to, resolved at the
    /// call site from the seat's current focus -- not the client that later
    /// asks about it.
    ///
    /// The timestamp is an [`Instant`] rather than the millisecond counter
    /// the call sites already have for `InputTime`: that one is a `u32` and
    /// wraps every 49 days, which would make a very old entry look fresh
    /// rather than stale -- the wrong way round for a security check. One
    /// clock read per key or button event pays for not having to reason about
    /// that.
    pub(crate) fn record(&mut self, serial: Serial, client: ClientId) {
        self.push(serial, client, Kind::Input);
    }

    /// Remembers a focus `enter` delivered to `client` carrying `serial`.
    ///
    /// The popup-grab half of this history (see the module doc): a toolkit
    /// opens a menu with the last serial it saw, which for a hover-opened
    /// menu is an `enter`. Never spent by the activation gate -- see
    /// [`Self::contains`].
    ///
    /// The call sites guarantee this names an `enter` that was actually
    /// sent: `shell.rs` records only when the keyboard focus really moves to
    /// a new surface, and `input.rs` only when the pointer really enters one
    /// -- both skip redundant derivations (which mint a serial but send
    /// nothing) and anything delivered while a grab holds the seat (whose
    /// recipient the call site cannot name).
    pub(crate) fn record_focus(&mut self, serial: Serial, client: ClientId) {
        self.push(serial, client, Kind::Focus);
    }

    fn push(&mut self, serial: Serial, client: ClientId, kind: Kind) {
        self.events[self.next] = Some(Delivered {
            serial,
            client,
            kind,
            at: Instant::now(),
        });
        self.next = (self.next + 1) % CAPACITY;
    }

    /// Whether `client` was given a key or button event carrying `serial`,
    /// recently enough to still be worth something.
    ///
    /// The activation gate, and deliberately only it: a focus `enter` spends
    /// nothing here (see the module doc for why the popup grab differs).
    ///
    /// A linear scan of sixteen entries, on a path that runs once per
    /// activation token (a user action), not per input event.
    pub(crate) fn contains(&self, serial: Serial, client: &ClientId) -> bool {
        self.events.iter().flatten().any(|known| {
            known.kind == Kind::Input
                && known.serial == serial
                && known.client == *client
                && known.at.elapsed() < INTERACTION_WINDOW
        })
    }

    /// Whether `client` was given an event carrying `serial` -- a key,
    /// button, or focus `enter` -- recently enough to still be worth
    /// something.
    ///
    /// The popup-grab gate: strictly looser than [`Self::contains`], for the
    /// toolkit shape the module doc describes. Same bounds otherwise (exact
    /// match, same count cap, same age window, same per-client binding).
    ///
    /// A linear scan of sixteen entries, on a path that runs once per menu
    /// opened, not per input event or per frame.
    pub(crate) fn contains_seen(&self, serial: Serial, client: &ClientId) -> bool {
        self.events.iter().flatten().any(|known| {
            known.serial == serial
                && known.client == *client
                && known.at.elapsed() < INTERACTION_WINDOW
        })
    }

    /// The serial and recipient of the event recorded most recently, if any.
    ///
    /// Tests only: the compositor itself never asks "what was the last one",
    /// precisely because that question has no single right answer (see the
    /// module doc). Tests need it to learn the serial a simulated press was
    /// sent with, which is otherwise unobservable -- [`SERIAL_COUNTER`] is
    /// process-global and shared with every other test running in parallel.
    ///
    /// [`SERIAL_COUNTER`]: smithay::utils::SERIAL_COUNTER
    #[cfg(test)]
    pub(crate) fn latest(&self) -> Option<(Serial, ClientId)> {
        let last = (self.next + CAPACITY - 1) % CAPACITY;
        self.events[last]
            .as_ref()
            .map(|event| (event.serial, event.client.clone()))
    }

    /// Ages every remembered event by `by`, as if the session had been idle
    /// that long.
    ///
    /// Tests only, and the only way to reach [`INTERACTION_WINDOW`] without
    /// a test that really sleeps for it.
    #[cfg(test)]
    pub(crate) fn backdate(&mut self, by: Duration) {
        for event in self.events.iter_mut().flatten() {
            event.at -= by;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use smithay::reexports::wayland_server::Display;

    use super::*;
    use crate::compositor::State;
    use crate::compositor::state::ClientState;

    /// Two real clients on a display that never listens on a socket: a
    /// [`ClientId`] cannot be built by hand, and its identity is the whole
    /// point of what is being tested.
    ///
    /// The `UnixStream`s are handed back so the clients stay connected --
    /// dropping the other end would disconnect them mid-test.
    fn two_clients() -> (Display<State>, [ClientId; 2], [UnixStream; 2]) {
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut handle = display.handle();
        let mut connect = || {
            let (server, ours) = UnixStream::pair().expect("a socket pair");
            let id = handle
                .insert_client(server, Arc::new(ClientState::default()))
                .expect("an inserted client")
                .id();
            (id, ours)
        };
        let (first, first_end) = connect();
        let (second, second_end) = connect();
        (display, [first, second], [first_end, second_end])
    }

    fn serial(raw: u32) -> Serial {
        Serial::from(raw)
    }

    #[test]
    fn a_fresh_ring_remembers_nothing() {
        let (_display, [one, _two], _kept) = two_clients();
        let recent = Recent::default();
        assert!(!recent.contains(serial(1), &one));
        assert_eq!(recent.latest(), None);
    }

    #[test]
    fn a_recorded_event_is_remembered_for_the_client_it_went_to() {
        let (_display, [one, two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(7), one.clone());

        assert!(recent.contains(serial(7), &one));
        assert!(
            !recent.contains(serial(8), &one),
            "a serial nothing carried"
        );
        assert!(
            !recent.contains(serial(7), &two),
            "a client that never received this event can spend it"
        );
        assert_eq!(recent.latest(), Some((serial(7), one)));
    }

    #[test]
    fn the_same_serial_for_two_clients_stays_two_entries() {
        // Not a case the compositor can produce -- one event has one
        // recipient -- but the check must be on the pair, not on either half
        // of it, and this is what says so.
        let (_display, [one, two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(3), one.clone());
        assert!(!recent.contains(serial(3), &two));
        recent.record(serial(3), two.clone());
        assert!(recent.contains(serial(3), &one));
        assert!(recent.contains(serial(3), &two));
    }

    #[test]
    fn every_event_in_a_full_ring_still_matches() {
        // The press/release pairs of one burst must all stay valid: which of
        // them a given client minted its token from is the client's choice,
        // not something this can predict.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        for raw in 1..=CAPACITY as u32 {
            recent.record(serial(raw), one.clone());
        }
        for raw in 1..=CAPACITY as u32 {
            assert!(
                recent.contains(serial(raw), &one),
                "{raw} was forgotten early"
            );
        }
    }

    #[test]
    fn the_oldest_event_is_evicted_once_the_ring_wraps() {
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        for raw in 1..=CAPACITY as u32 + 1 {
            recent.record(serial(raw), one.clone());
        }
        assert!(
            !recent.contains(serial(1), &one),
            "the oldest event survived"
        );
        assert!(recent.contains(serial(2), &one));
        assert!(recent.contains(serial(CAPACITY as u32 + 1), &one));
        assert_eq!(
            recent.latest(),
            Some((serial(CAPACITY as u32 + 1), one.clone()))
        );
    }

    #[test]
    fn recording_far_past_the_capacity_keeps_exactly_the_last_capacity_events() {
        // The index arithmetic wraps rather than growing, so a long session
        // is the same memory and the same cost as a fresh one.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        for raw in 1..=1_000u32 {
            recent.record(serial(raw), one.clone());
        }
        assert!(!recent.contains(serial(1_000 - CAPACITY as u32), &one));
        assert!(recent.contains(serial(1_000 - CAPACITY as u32 + 1), &one));
        assert!(recent.contains(serial(1_000), &one));
    }

    #[test]
    fn an_event_older_than_the_window_is_no_longer_recent() {
        // Nothing evicts an entry in an idle session -- the ring only rotates
        // when newer qualifying input arrives -- so this is the only thing
        // stopping this morning's click from being spendable tonight.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(5), one.clone());
        assert!(recent.contains(serial(5), &one));

        recent.backdate(INTERACTION_WINDOW);
        assert!(
            !recent.contains(serial(5), &one),
            "an event exactly at the window is still spendable"
        );
    }

    #[test]
    fn an_event_just_inside_the_window_still_counts() {
        // The other side of the bound, so the test above is checking an edge
        // rather than a check that refuses everything.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(5), one.clone());

        recent.backdate(INTERACTION_WINDOW - Duration::from_secs(1));
        assert!(recent.contains(serial(5), &one));
    }

    #[test]
    fn ageing_out_one_event_leaves_a_newer_one_alone() {
        // `contains` filters per entry rather than treating the whole ring as
        // one age, so a burst that straddles the window keeps its recent half.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(5), one.clone());
        recent.backdate(INTERACTION_WINDOW);
        recent.record(serial(6), one.clone());

        assert!(!recent.contains(serial(5), &one), "the old one survived");
        assert!(recent.contains(serial(6), &one), "the new one aged out too");
    }

    #[test]
    fn a_focus_serial_is_seen_but_not_a_user_action() {
        // The two gates' different bars, in one place: a popup grab may
        // spend an `enter` (the Qt hover-menu shape), while an activation
        // token may not.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record_focus(serial(9), one.clone());

        assert!(
            recent.contains_seen(serial(9), &one),
            "a delivered enter should be spendable for a grab"
        );
        assert!(
            !recent.contains(serial(9), &one),
            "a delivered enter must not be spendable for activation"
        );
    }

    #[test]
    fn a_focus_serial_is_still_bound_to_its_recipient() {
        // Widening to enters must not reopen the guessing attack the client
        // half of each entry exists to close: knowing the number is not
        // enough without having received it.
        let (_display, [one, two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record_focus(serial(9), one.clone());

        assert!(!recent.contains_seen(serial(9), &two));
    }

    #[test]
    fn a_focus_serial_ages_out_like_any_other_entry() {
        // The weakening an enter buys is bounded by the same window: this
        // morning's hover is not tonight's keyboard.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record_focus(serial(9), one.clone());
        assert!(recent.contains_seen(serial(9), &one));

        recent.backdate(INTERACTION_WINDOW);
        assert!(
            !recent.contains_seen(serial(9), &one),
            "an enter exactly at the window is still spendable"
        );
    }

    #[test]
    fn a_wrapped_around_counter_is_matched_by_value() {
        // `Serial`'s own ordering is wrap-aware, but this only ever asks for
        // equality, so a counter that has wrapped past `u32::MAX` matches the
        // same way any other value does.
        let (_display, [one, _two], _kept) = two_clients();
        let mut recent = Recent::default();
        recent.record(serial(u32::MAX), one.clone());
        recent.record(serial(0), one.clone());
        recent.record(serial(1), one.clone());
        assert!(recent.contains(serial(u32::MAX), &one));
        assert!(recent.contains(serial(0), &one));
        assert!(!recent.contains(serial(2), &one));
    }
}
