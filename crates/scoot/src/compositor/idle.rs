//! Idle detection (`ext-idle-notify-v1`, version 2) and idle inhibition
//! (`idle-inhibit-unstable-v1`, version 1) -- the automatic trigger the
//! session lock has no other way to get.
//!
//! A `swayidle`-style daemon binds `ext_idle_notifier_v1`, asks for a
//! notification with a timeout in milliseconds, and gets `idled` when the
//! seat has been quiet that long and `resumed` on the next input -- which
//! is what lets it dim the screen, lock it (`ext-session-lock-v1`) or
//! suspend the machine without polling. A video player or presentation
//! app goes the other way: it creates an inhibitor on one of its surfaces
//! and the compositor holds `idled` back until the inhibitor is gone.
//!
//! The protocol objects live in Smithay (`IdleNotifierState`, which owns
//! the per-notification calloop timers, and `IdleInhibitManagerState`);
//! what lives here is the wiring between them and this compositor:
//!
//! - [`IdleNotifierHandler`] routes Smithay's state accessor to
//!   `State::idle_notifier`.
//! - [`IdleInhibitHandler`] records inhibiting surfaces in
//!   [`Inhibitors`] and pushes the aggregate into the notifier via
//!   `set_is_inhibited`.
//! - [`State::announce_activity`] is what the four input choke points in
//!   `input.rs` (`pointer_move`, `pointer_button`, `scroll`, `key`) call,
//!   so every input source -- libinput under `--tty`, the host under
//!   `--nested`, IPC injection -- resets the idle timers, including input
//!   a keybinding intercepted (an intercepted keypress is still a user at
//!   the keyboard).
//!
//! Two deliberate scopings, both documented rather than half-built:
//!
//! - **Inhibition counts surfaces that are alive, not surfaces that are
//!   visible.** Checking visibility would mean re-deriving "inhibited"
//!   on every map/unmap commit -- the hot path -- while liveness is
//!   settled exactly where surfaces already die (`destroyed`, below, plus
//!   the explicit `uninhibit`). A client inhibiting from a surface it
//!   never maps can hold off `idled` indefinitely; that client is local
//!   and unprivileged either way, and an allow-list without
//!   security-context support would be theatre (the same rationale as the
//!   session-lock and data-control globals -- see `README.md`'s trust
//!   note).
//! - **There is no built-in auto-locker.** scoot provides the protocol;
//!   the policy (which timeout locks, what dims first) belongs to the
//!   daemon the user runs, the way `swayidle` works everywhere else. No
//!   config key, no timer of our own.

use std::collections::HashSet;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};

use super::State;

#[cfg(test)]
mod tests;

/// The surfaces currently holding idle inhibition, by `wl_surface`.
///
/// A set, not a count: `create_inhibitor` on the same surface twice must
/// not need destroying twice to release, and a surface dying with two
/// inhibitors out must still release exactly once. Liveness is re-checked
/// on every recompute rather than trusted from insert time -- a client
/// that disconnects without destroying its inhibitors leaves dead
/// surfaces behind, and only the recompute can see that.
#[derive(Debug, Default)]
pub struct Inhibitors {
    surfaces: HashSet<WlSurface>,
}

impl Inhibitors {
    /// Records an inhibitor, reporting whether it changed anything. A
    /// duplicate insert is a no-op by construction of the set.
    fn insert(&mut self, surface: WlSurface) -> bool {
        self.surfaces.insert(surface)
    }

    /// Drops an inhibitor, reporting whether one was held. Destroying an
    /// inhibitor that was never created -- or twice -- changes nothing.
    fn remove(&mut self, surface: &WlSurface) -> bool {
        self.surfaces.remove(surface)
    }

    /// Drops surfaces whose client is gone, reporting whether any were.
    /// The only caller besides [`State::refresh_idle_inhibit`] is the
    /// surface-destroyed path, which names exactly one surface -- but a
    /// disconnect can kill several at once, and only a full sweep sees
    /// all of them.
    fn prune_dead(&mut self) -> bool {
        let before = self.surfaces.len();
        self.surfaces.retain(IsAlive::alive);
        self.surfaces.len() != before
    }

    /// Whether any *live* inhibitor is held. Dead surfaces never inhibit:
    /// without this, a client that disconnects mid-inhibition would hold
    /// the session awake until its inhibitors are explicitly destroyed,
    /// which a dead client can never do.
    fn inhibited(&self) -> bool {
        self.surfaces.iter().any(IsAlive::alive)
    }
}

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        if self.idle_inhibitors.insert(surface) {
            self.refresh_idle_inhibit();
        }
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        if self.idle_inhibitors.remove(&surface) {
            self.refresh_idle_inhibit();
        }
    }
}

impl State {
    /// Re-derives the notifier's inhibited flag from [`Inhibitors`],
    /// pruning dead surfaces first. Called after every change to the set
    /// (inhibit, uninhibit, surface destroyed) -- never on a timer or a
    /// commit, so nothing on a hot path pays for it.
    ///
    /// `set_is_inhibited` is itself a no-op when the flag doesn't change,
    /// so the common case (an unrelated surface dying while nothing is
    /// inhibited) costs a set removal and nothing else.
    pub fn refresh_idle_inhibit(&mut self) {
        self.idle_inhibitors.prune_dead();
        let inhibited = self.idle_inhibitors.inhibited();
        self.idle_notifier.set_is_inhibited(inhibited);
    }

    /// Forgets one surface's inhibitor, if it held one. Called from
    /// `CompositorHandler::destroyed` (see `handlers.rs`), which runs for
    /// every `wl_surface` that goes away however it goes -- explicit
    /// destroy, client disconnect, or output teardown.
    pub fn forget_idle_inhibitor(&mut self, surface: &WlSurface) {
        if self.idle_inhibitors.remove(surface) {
            self.refresh_idle_inhibit();
        }
    }

    /// Marks the seat as active, resetting every idle timer Smithay holds.
    /// The four input choke points call this; nothing else needs to,
    /// because every input source in the compositor already funnels
    /// through one of those four.
    ///
    /// Cheap when nobody watches: with no notifications registered,
    /// Smithay's per-seat lookup misses and this is a mutex lock plus a
    /// `HashMap::get` -- no allocation, no timer churn. With watchers it
    /// re-arms one calloop timer per notification, which is the protocol
    /// working as specified, not overhead to optimize away.
    pub fn announce_activity(&mut self) {
        self.idle_notifier.notify_activity(&self.seat);
    }
}
