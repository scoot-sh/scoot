//! `zwp_relative_pointer_manager_v1` plus the `zwp_pointer_constraints_v1`
//! global it pairs with: raw, unaccelerated pointer deltas for games and 3D
//! apps.
//!
//! Both globals are Smithay's at the pinned rev (`RelativePointerManagerState`
//! under `src/wayland/relative_pointer.rs`, `PointerConstraintsState` under
//! `src/wayland/pointer_constraints.rs` -- verified in source, not assumed),
//! so scoot's side is the PR #88 shape: two hold-alive fields on [`State`]
//! (see [`State::new`](super::State::new)), this module's
//! [`PointerConstraintsHandler`] impl, and the constraint-aware motion core in
//! `input.rs`. No Smithay patch vendored; everything stays in scoot's
//! handler layer.
//!
//! ## Gating: focus, not lock state
//!
//! `relative_motion` events are **focus-gated, not lock-gated**. The protocol
//! XML says it outright (`relative-pointer-unstable-v1.xml`,
//! `zwp_relative_pointer_v1`): "It shares the same focus as wl_pointer
//! objects of the same seat and will only emit events when it has focus."
//! Nothing in the protocol ties delivery to an active lock or confinement,
//! and Smithay's routing matches: `WlSurface::relative_motion` (pinned rev,
//! `src/wayland/seat/pointer.rs`) forwards to every `ZwpRelativePointerV1`
//! whose client is the focused surface's client, with no constraint check
//! anywhere on that path. A client with pointer focus gets relative deltas
//! whether or not it locked; a client without focus gets nothing, locked or
//! not. Gating on lock state instead would be a bespoke deviation from the
//! standard -- and would break real clients that read relative motion without
//! locking -- so this compositor does not do it. `input.rs` therefore emits
//! relative motion on every focused motion, and the tests pin both halves:
//! focused-without-lock receives, unfocused receives nothing.
//!
//! "Focused" means **pre-move focus**: the seat's focus as the motion
//! arrives. That is forced by Smithay's own dispatch --
//! `PointerInnerHandle::relative_motion` (pinned rev,
//! `src/input/pointer/mod.rs`) ignores the focus the caller passes and
//! routes by the seat's current focus, i.e. whoever the last absolute motion
//! focused -- so the emission runs *before* the absolute motion (anvil's
//! order at the pinned rev), and a motion that changes focus credits the
//! surface the pointer is leaving, not the one it lands on. For continuous
//! device motion the two agree on every event; they differ only on teleports
//! (an IPC jump, or the single device event that crosses a surface
//! boundary), where crediting the surface the motion started from is the
//! faithful answer -- the motion happened while the pointer was there. A
//! teleport *onto* a surface from bare desktop accordingly reports nothing:
//! nobody had focus when the motion began.
//!
//! ## What "unaccelerated" means on each motion source
//!
//! Only one source in this stack ever accelerates: libinput, on the `--tty`
//! relative path. Its `PointerMotionEvent` carries both the accelerated
//! `delta_x`/`delta_y` (adaptive profile by default) and the pre-accel
//! `delta_x_unaccel`/`delta_y_unaccel`, and `pointer_move_relative` threads
//! both pairs through so the relative event's `dx_unaccel`/`dy_unaccel` are
//! the pre-accel device values, not the accelerated ones the absolute
//! position moves by. Every other source is absolute and applies no
//! acceleration of its own -- IPC injection, `--nested` host-forwarded
//! motion, absolute tablets -- so there `delta` and `delta_unaccel` are the
//! same number: position change, honestly reported. (Nested motion was
//! already accelerated once, by the host compositor; nothing here can undo
//! that, and claiming otherwise would be the lie.)
//!
//! Relative deltas are **unclipped**: the spec's own example is motion
//! clipped by a monitor edge still reporting the unclipped vector. The tty
//! path therefore emits the raw device delta even when the absolute position
//! clamps at the output edge, and a locked pointer -- whose absolute position
//! does not move at all -- still reports every device delta.
//!
//! ## What lock and confinement do to absolute motion
//!
//! Advertising the constraints global without honouring it would be the
//! protocol lie in the other direction: a client told `locked` while its
//! cursor keeps teleporting. So the motion core resolves every move through
//! [`absolute_target`], against the constraint on the **pre-move focus**
//! surface (the same surface the relative event is credited to -- a
//! teleport's target has no say until focus actually lands there):
//!
//! - an **active lock** on that surface holds the absolute position
//!   (relative still flows, per above);
//! - an **active confinement** clamps the move per axis to its region (the
//!   anvil shape: each axis is zeroed independently when stepping out) and
//!   refuses a move that would leave the surface at all;
//! - a constraint whose region does not contain the pointer's *current*
//!   position does not apply (anvil's gate, verbatim: the check is against
//!   where the pointer is, not where it is going).
//!
//! Activation is anvil's, in two halves: [`new_constraint`](PointerConstraintsHandler)
//! activates immediately when the surface already has pointer focus, and a
//! focus-changing move engages a still-inactive constraint on the surface it
//! lands on (`engage_pending_constraint` in `input.rs`, region-gated like
//! anvil's own post-motion check). Without the second half, a lock taken
//! before first focus -- a game arming its mouse mode at startup -- would
//! sit inactive forever, since Smithay reports `locked`/`confined` only
//! from an explicit `activate`.
//!
//! Deactivation is Smithay's own (`WlSurface::leave` deactivates an active
//! constraint and reports `PointerLeave`/`unlocked`/`unconfined`), and it
//! keeps the persistent entry: only `Oneshot` entries are removed, so a
//! disarmed persistent constraint re-arms through the same engage path on
//! re-entry into its region. A held pointer (lock, or a confine-escape)
//! freezes focus as well as position, so no leave ever fires while one is
//! active -- which is why a session lock deactivates first and re-derives
//! focus second: [`State::lock_transition`](super::State::lock_transition)
//! deactivates the focus surface's held constraint (the client sees
//! `unlocked`/`unconfined`) so the transition's focus refresh behaves like
//! the unconstrained case, and the still-registered persistent entry
//! re-arms through the engage path when focus returns on unlock. A gated-out
//! regional confinement needs no such help: it can still be teleported off
//! its surface, which deactivates it, and re-entering inside the region
//! re-arms it with no new request from the client -- both pinned by tests.
//!
//! Because a locked pointer never moves absolute, there is no position to
//! restore on unlock, so `cursor_position_hint` keeps its default
//! (ignored) rather than tracked state nobody reads.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as every other
//! advertisement here: scoot has no security-context support, so an
//! allow-list would be theatre (see `docs/protocols.md`'s trust note). A lock
//! only ever pins the locking client's own focused surface, and relative
//! deltas only ever reach the focused client.
//!
//! Lock activation additionally carries no interaction-serial requirement,
//! deliberately, and unlike the activation/popup/drag gates (PRs #42, #56)
//! this is an accepted risk rather than an oversight -- weighed, not
//! hand-waved:
//!
//! - The protocol gives nothing to gate on. `lock_pointer` carries no
//!   serial, only (surface, pointer, region, lifetime), so any gate would
//!   be a heuristic recency check ("did this client recently interact"),
//!   not the exact serial match those gates enforce -- with false refusals
//!   on legitimate flows (a game locking on hover-enter minutes after its
//!   last keypress) and no protocol error to refuse with honestly.
//! - Anvil and wlroots both activate freely; a gate would be a bespoke
//!   deviation from the ecosystem, not the standard.
//! - The blast radius is the pointer position only. Constraint state is
//!   consulted on exactly one path -- absolute pointer motion (the three
//!   call sites in this module and `input.rs`) -- so the keyboard never
//!   freezes: keybindings still fire, so closing the offending window
//!   (whose surface destruction removes its constraints) frees the pointer
//!   with one chord, no VT switch needed. No pixels cross either: what is
//!   on screen is unchanged by a lock. That chord argument covers the
//!   unlocked session only: while the session is locked every binding but
//!   `ChangeVt` is forwarded to the locker instead of executed, which is
//!   why a held lock does not survive a lock transition -- the transition
//!   deactivates it first (see above) rather than relying on a recovery
//!   chord that cannot run.
//! - The threat needs a malicious client already running as the user --
//!   the same trust domain as keylogging through the input-method and
//!   data-control globals this compositor already advertises without a
//!   filter.
//!
//! The residual exposure is self-DoS of the cursor by a client the user
//! ran: a pre-armed persistent lock plus an arrival freezes the pointer
//! until that window closes. That is what the sentence above says is
//! acceptable, and why.
//!
//! ## Cost
//!
//! Unfocused motion pays one `current_focus` read. Focused motion pays one
//! `current_location` read, one constraint-map lookup, and -- only with a
//! live relative pointer for that client -- the per-object socket writes.
//! The empty-list mutex inside Smithay's dispatch is the only per-event cost
//! with no relative pointers at all. Measured before/after on the dev VM
//! (200k events x 5 reps, temporary bench since removed): debug unfocused
//! 3129-3426 ns/event after vs 3415-3623 before (overlapping, no
//! regression), debug focused 10688-11501 vs 8555-9090 before; release
//! unfocused 336-388 vs 354-362 before (noise), release focused 950-998 vs
//! 757-823 before -- a ~190ns residual, ~190us/s at 1000Hz, ~1.2% of one
//! 16ms frame per second. See `move_absolute`'s doc for the full table.

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{
    PointerConstraint, PointerConstraintsHandler, with_pointer_constraint,
};

use super::State;

#[cfg(test)]
mod tests;

/// Where an absolute move to `to` (from `from`) actually lands under pointer
/// constraints. No heap: the confined arm carries the clamped point and the
/// hit test behind it inline.
pub(super) enum AbsoluteTarget {
    /// No focus, no constraint on the focus surface, an inactive one, or one
    /// whose region does not contain the pointer's current position: move to
    /// `to`.
    Free,
    /// An active lock, or an active confinement the move would escape: move
    /// nothing absolute. (Relative deltas still flow; the caller emits them
    /// against pre-move focus before consulting this.)
    Held,
    /// An active confinement containing the move: move to `point`, whose hit
    /// test (`under`) is what the absolute motion is delivered with.
    Clamped {
        point: Point<f64, Logical>,
        under: (WlSurface, Point<f64, Logical>),
    },
}

/// Resolves an absolute move against the pointer constraint on the pre-move
/// focus surface, if any.
///
/// Read-only: this never activates anything (activation is
/// [`new_constraint`](PointerConstraintsHandler)'s job, at creation time).
/// The focus surface -- not the move's target -- is what carries the
/// constraint, for the same reason the relative event is credited there: a
/// teleport's target has no standing until focus lands on it. Concretely,
/// that means a move *off* a locked or confined surface is held, not freed
/// (the target's hit test says bare desktop, but the focus surface says
/// locked), while a move *onto* one is free (its constraint will engage, if
/// at all, at creation time via `new_constraint`).
///
/// The region gate mirrors anvil at the pinned rev: a constraint whose
/// region does not contain the pointer's *current* surface-local position
/// does not apply to this move at all.
///
/// The confined arm is two checks, also anvil's shape: each axis is zeroed
/// independently when that axis alone would step out of the region, and a
/// move that would still leave the focus surface -- a region-less confinement
/// being the degenerate case, where the per-axis clamp changes nothing -- is
/// refused outright. "Leave the surface" is answered by the compositor's own
/// hit test rather than geometry: what confines the pointer is staying on
/// the surface the constraint was made for, whatever covers it.
///
/// The focus surface's origin comes from the target hit test when the move
/// stays on it (the common case: no second hit test), and from a fresh hit
/// test at the current position otherwise -- which only runs with an active
/// confinement in play, never on the hot path. If even that finds nothing
/// (focus without a hit -- only reachable through server-side motion, which
/// no production path performs), the constraint is unenforceable without an
/// origin to measure in, so the move is free rather than frozen: a fail-open
/// that cannot wedge the pointer, documented here rather than hidden.
pub(super) fn absolute_target(
    state: &State,
    pointer: &PointerHandle<State>,
    focus: Option<&WlSurface>,
    from: Point<f64, Logical>,
    to: Point<f64, Logical>,
    to_under: &Option<(WlSurface, Point<f64, Logical>)>,
) -> AbsoluteTarget {
    let Some(focus) = focus else {
        return AbsoluteTarget::Free;
    };
    // Phase one: what kind of active constraint, if any, is on the focus
    // surface. Borrowed, never cloned: the pre-fix shape copied
    // `constraint.region()` (a `Vec` inside `RegionAttributes`) into an
    // owned `Option` on every focused move with an active constraint -- a
    // per-event heap allocation on an input path -- and the confined arm
    // only ever asked `contains` of it.
    //
    // The confine arm deliberately does NOT run inside this lookup. The
    // map lock is held for the closure's duration, while the origin and
    // clamp hit tests it needs run under Smithay's `with_states`, which
    // holds the surface's user-data mutex -- a non-reentrant `MutexGuard`
    // -- so re-entering it from in here deadlocks the event loop on any
    // surface the hit test touches, the focus surface first among them
    // (Smithay's own constraint commit hook carries the same warning).
    // Phase two below re-takes the lookup once the origin is known and
    // returns only `Copy` data.
    let locked = with_pointer_constraint(focus, pointer, |constraint| {
        let constraint = constraint.as_deref()?;
        if !constraint.is_active() {
            return None;
        }
        Some(matches!(constraint, PointerConstraint::Locked(_)))
    });
    let Some(locked) = locked else {
        return AbsoluteTarget::Free;
    };
    if locked {
        return AbsoluteTarget::Held;
    }
    // Confined from here. The origin the region is measured in: the target
    // hit test's, when the move stays on the focus surface, else a fresh one
    // at the current position (see the doc above for when each runs).
    let origin = match to_under {
        Some((surface, origin)) if surface == focus => *origin,
        _ => match state
            .surface_under(from)
            .filter(|(surface, _)| surface == focus)
        {
            Some((_, origin)) => origin,
            // Focus without a hit: fail open (see above).
            None => return AbsoluteTarget::Free,
        },
    };
    // Phase two: the region answers against the now-known origin, borrowed.
    // No allocation on any arm: the gate and the per-axis clamp only ask
    // `contains`, and the owned decision is built outside afterwards.
    //
    // The kind is re-checked rather than trusted from phase one. Nothing on
    // this thread can change it in between -- constraint state moves on
    // wayland dispatch (client requests, commit hooks) and this function
    // dispatches nothing -- so the fallbacks are unreachable; they fail in
    // the safe direction (a lock holds, anything else frees) rather than
    // wedging the pointer on an assumption.
    enum Clamp {
        /// Anvil's gate: the region does not contain the pointer's current
        /// position, so the constraint does not apply to this move.
        Free,
        /// The move resolves against a lock after all: hold.
        Held,
        /// The move with each out-of-region axis zeroed.
        Delta(Point<f64, Logical>),
    }
    let clamp = with_pointer_constraint(focus, pointer, |constraint| {
        let constraint = constraint.as_deref()?;
        if !constraint.is_active() {
            return Some(Clamp::Free);
        }
        if matches!(constraint, PointerConstraint::Locked(_)) {
            return Some(Clamp::Held);
        }
        let region = constraint.region();
        if !region.is_none_or(|region| region.contains((from - origin).to_i32_round())) {
            return Some(Clamp::Free);
        }
        let mut delta = to - from;
        if let Some(region) = region {
            if !region.contains((from + Point::from((delta.x, 0.0)) - origin).to_i32_round()) {
                delta.x = 0.0;
            }
            if !region.contains((from + Point::from((0.0, delta.y)) - origin).to_i32_round()) {
                delta.y = 0.0;
            }
        }
        Some(Clamp::Delta(delta))
    })
    .unwrap_or(Clamp::Free);
    let delta = match clamp {
        Clamp::Free => return AbsoluteTarget::Free,
        Clamp::Held => return AbsoluteTarget::Held,
        Clamp::Delta(delta) => delta,
    };
    let clamped = from + delta;
    match state
        .surface_under(clamped)
        .filter(|(found, _)| found == focus)
    {
        Some(under) => AbsoluteTarget::Clamped {
            point: clamped,
            under,
        },
        None => AbsoluteTarget::Held,
    }
}

impl State {
    /// Deactivates the active pointer constraint on the current
    /// pointer-focus surface, if any.
    ///
    /// Called from [`State::lock_transition`](super::State::lock_transition)
    /// before the focus refresh, so the refresh behaves like the
    /// unconstrained case. Without this, a zero-delta refresh against a
    /// held lock resolves to [`AbsoluteTarget::Held`], which delivers no
    /// `leave` and no `enter` -- so seat pointer focus never leaves the
    /// locking client, device deltas, buttons and axis keep streaming to
    /// it across the session lock, and the lock surface gets nothing (the
    /// password-leak class `session_lock.rs` rejects for the keyboard).
    ///
    /// Deactivating -- rather than bypassing `Held` for one move -- is
    /// Smithay's own story for losing a lock: the client sees
    /// `unlocked`/`unconfined`, the persistent entry stays registered, and
    /// the ordinary arrival path (`engage_pending_constraint` in
    /// `input.rs`) re-arms it on unlock with no new request from the
    /// client -- the same leave/re-enter story as a pointer physically
    /// leaving the surface.
    ///
    /// Only the focus surface's constraint is deactivated, and that is
    /// exhaustive rather than approximate: activation is focus-gated both
    /// at creation ([`new_constraint`](PointerConstraintsHandler)) and on
    /// arrival (`engage_pending_constraint`), and Smithay deactivates on
    /// pointer-leave, so an active constraint implies its surface holds
    /// the seat's single pointer focus. Inactive constraints elsewhere are
    /// untouched by the lookup (deactivating one is a no-op that sends
    /// nothing and keeps the entry), which is correct: they hold no focus
    /// and stream nothing, and a lock requested while locked must stay
    /// inactive until unlock engages it.
    ///
    /// Read-only on scoot state: the one seat lookup plus one
    /// constraint-map lookup on a transition that has already decided to
    /// re-derive focus and redraw; nothing on any per-event path.
    pub(super) fn deactivate_pointer_constraint(&self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(focus) = pointer.current_focus() else {
            return;
        };
        with_pointer_constraint(&focus, &pointer, |constraint| {
            if let Some(constraint) = constraint {
                constraint.deactivate();
            }
        });
    }
}

impl PointerConstraintsHandler for State {
    /// Activate at creation when the surface already has pointer focus --
    /// anvil's policy at the pinned rev. A lock taken while unfocused stays
    /// inactive until focus arrives, at which point `engage_pending_constraint`
    /// activates it (see below) — and once Smithay tears it down on
    /// pointer-leave, nothing re-arms it afterwards (see the module doc). The `with_pointer_constraint` lookup cannot miss: Smithay
    /// calls this because the constraint was just created.
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        let focused = pointer.current_focus().as_ref() == Some(surface);
        if focused {
            with_pointer_constraint(surface, pointer, |constraint| {
                constraint.unwrap().activate();
            });
        }
    }
}
