//! `ext-session-lock-v1`: the screen lock the compositor itself enforces.
//!
//! A layer-shell "lock screen" is a client asking nicely for exclusive
//! keyboard focus: it can be covered, closed, or drawn over, and everything
//! behind it is still rendered and still reachable. This protocol is the
//! other trust model. Once a lock is accepted, *this compositor* stops
//! drawing and stops routing input to anything except the lock client's own
//! surfaces, and only that client's explicit `unlock_and_destroy` ends it.
//!
//! ## One field says whether the session is locked
//!
//! [`SessionLock::owner`] is that field: `owner.is_some()` **is** "the session
//! is locked", and this module keeps no second boolean beside it that could
//! disagree (see `docs/roadmap/05b-vt-switch-eperm.md` for why this project treats a field with
//! two meanings as a bug class of its own). It is written `Some` in exactly one
//! place -- [`SessionLockHandler::lock`], when a lock is accepted -- and `None`
//! in exactly one place -- [`SessionLockHandler::unlock`], when the owning
//! client unlocks. Nothing else, in any backend, in any error path, may write
//! it; in particular a VT switch, a session pause, a failed render and a dying
//! client all leave it exactly as it was, which is what makes the lock survive
//! them.
//!
//! Two derived questions come off that same field, so they cannot drift:
//! [`SessionLock::is_locked`] (`owner.is_some()`) and
//! [`SessionLock::abandoned`] (`owner` is `Some` but its client is gone).
//!
//! **That claim is about this module's own state, and does not extend to
//! Smithay's.** An earlier version of this doc said there was "no second
//! boolean that could disagree", full stop; there is one, it just is not
//! flexwm's. Smithay keeps a `LockStatus` (`Unlocked` / `Locked(lock)` /
//! `Defunct`) inside the [`SessionLockManagerState`] held by
//! [`SessionLock::manager`], and it has *different timing on purpose*: it
//! becomes `Locked` only when [`SessionLocker::lock`] runs -- i.e. when the
//! `locked` event goes out -- whereas `owner` is set the moment a lock is
//! *accepted*. Closing that gap is not an option: the protocol forbids sending
//! `locked` before a blanked frame exists. It is load-bearing rather than
//! harmless, because `LockStatus` is what Smithay gates
//! `ext_session_lock_v1.destroy` on (`lock.rs`: the request is refused only
//! while `lock_status.is_locked_by(lock)`). In the accepted-but-not-yet-
//! confirmed window it still reads `Unlocked`, so that `destroy` is accepted,
//! and a client can therefore give up a lock it has already been granted. See
//! "Which lock surfaces count" for what that does to the surfaces it leaves
//! behind, and why filtering them is a correctness requirement rather than
//! tidiness.
//!
//! ## What "locked" changes, and where
//!
//! Every one of these is a branch taken *before* the unlocked path, not an
//! ordering tweak on top of it -- drawing the lock surface last is not
//! exclusivity if anything else is still in the list:
//!
//! - **Rendering** (`headless.rs::render`): the element list is built from
//!   this module alone -- an opaque full-output backdrop plus the mapped lock
//!   surfaces -- so a window, a bar, a wallpaper or a focus ring is never
//!   gathered at all. A screenshot over IPC reads that same framebuffer, so
//!   it shows what the lock screen shows and nothing behind it.
//! - **Frame callbacks** (`headless.rs::render`): only lock surfaces get
//!   them, so ordinary clients stop drawing while the session is locked --
//!   the protocol's "the compositor must stop rendering ... normal clients".
//! - **Keyboard focus** (`shell.rs::refresh_keyboard_focus`): a lock surface,
//!   or nobody. Never a window, never a layer surface.
//! - **Pointer focus** (`state.rs::surface_under`, and the explicit
//!   [`State::refresh_pointer_focus`] at every lock transition): the hit test
//!   only ever sees lock surfaces. The refresh matters as much as the hit
//!   test -- `wl_pointer.button` goes to whatever the pointer last *entered*,
//!   so without moving focus at the moment of locking, the first click after
//!   a lock would still land in the window underneath.
//! - **Keybindings** (`input.rs::key`): only `Bound::ChangeVt` may still
//!   fire. A `spawn` bind reaching a terminal from a locked screen would be a
//!   complete bypass; VT switching is the deliberate exception, because the
//!   VT it switches to has its own login and this compositor's session stays
//!   locked behind it.
//! - **IPC** (`ipc.rs::handle_request`): `Request::Action` is refused, for
//!   the same reason as the `spawn` bind. Injected keyboard/pointer input is
//!   *not* refused -- it goes through the focus paths above, so it can only
//!   reach the lock surface, exactly like a real keyboard.
//!
//! ## When the lock client dies
//!
//! The session stays locked. That is the protocol's own rule ("If the client
//! dies while the session is locked, the compositor must not unlock the
//! session in response") and the whole point of the protocol: a dead locker
//! is not evidence that the user wants their screen unlocked.
//!
//! The screen turns solid red, so a user can tell "my locker crashed" from
//! "my locker is showing a black screen", and a new client may take the lock
//! over -- run a lock client again and it can unlock after authenticating.
//! Both behaviors match sway (`sway/lock.c`'s `handle_abandon` paints the same
//! red and `handle_session_lock` replaces an abandoned lock) and niri
//! (`Niri::lock` replaces a lock whose client is not alive). The alternative
//! -- no takeover at all -- would make a crashed locker a permanently
//! unusable session with only a compositor restart as the way out, which is
//! its own kind of failure.
//!
//! A takeover is confirmed *immediately* only when the lock it replaces had
//! already been confirmed, i.e. when a blanked frame has genuinely been drawn
//! ([`SessionLock::pending`] is `None`). "The outputs are already blanked" is
//! true then and only then: a lock that was accepted but never confirmed --
//! because its client died, or gave it up, in the window before the first
//! frame -- leaves the *unlocked* desktop on screen, and telling the
//! replacement `locked` there would hand it exactly the guarantee the event
//! exists to provide while the user's windows are still displayed. Such a
//! takeover goes through the same [`SessionLock::pending`] path a fresh lock
//! uses instead. niri's `Niri::lock` draws the same line (it fast-confirms
//! only from an already-`Locked` state, never from `Locking(_)`).
//!
//! The honest consequence, which `README.md` states too: while a lock is
//! abandoned, any client that can reach this compositor's wayland socket can
//! take it over and unlock. That is the same-uid trust boundary flexwm's IPC
//! socket already has, and it is the price of having a recovery path at all.
//!
//! One more way to reach the same state, legally and without dying: a client
//! may `destroy` its lock object *before* `locked` arrives (only
//! `unlock_and_destroy` is forbidden that early -- see the `LockStatus` note
//! above), which a locker that gives up waiting could do. The session is
//! already locked by then, so it stays locked and reads as abandoned -- a red
//! screen, recovered by running a lock client again. That is the safe
//! direction, and deliberately not special-cased into an unlock: "the client
//! changed its mind" and "the client was killed" are indistinguishable from
//! here, and only one of them is safe to guess at.
//!
//! ## Which lock surfaces count
//!
//! Unlike a dying client, a client that merely destroys its *lock* keeps
//! everything else: its connection, and the `wl_surface` under each lock
//! surface it created. So [`SessionLock::surfaces`] can hold a surface whose
//! `wl_surface.alive()` is perfectly true and which no longer belongs to any
//! lock at all. Liveness is therefore not the question a reader may ask.
//!
//! [`is_current`] is, and every read goes through [`SessionLock::current`],
//! which applies it. A surface counts only if all three hold: its own
//! `wl_surface` is alive, the lock that created it is the one in `owner`, and
//! that lock still exists. Dropping any one of them is exploitable rather than
//! untidy:
//!
//! - `== owner` alone still admits the surfaces of an *abandoned* lock, which
//!   is what the red backdrop is supposed to be replacing on screen.
//! - `owner.is_alive()` alone still admits a surface left over from a lock
//!   that has since been taken over by someone else.
//!
//! Together they close the one attack this module has to care about: `lock`,
//! `get_lock_surface`, `destroy`, sent as a single batch by any client that
//! can reach the wayland socket. It needs no race and no crash, and before
//! this filter existed its surface stayed registered forever -- so the *next*
//! locker's screen would be drawn from the attacker's pixels and every
//! keystroke the user typed into it, password included, went to the attacker's
//! still-connected surface while the real locker could never authenticate.
//!
//! Filtering is the guarantee; dropping is the follow-through. A surface that
//! has stopped counting is also removed, and both focuses re-derived, at the
//! first of [`State::refresh_lock_state`] (the dispatch that observed the
//! disconnect or the `destroy`), [`SessionLock::cleanup`] (the next locked
//! frame) and the `surfaces.clear()` in [`SessionLockHandler::lock`] (a
//! takeover). Re-deriving is not optional there: `wl_pointer.button` goes to
//! whatever the pointer last *entered*, so a filter that hid a zombie from the
//! hit test while leaving it holding pointer focus would still deliver it
//! every click.
//!
//! ## Every lock transition does the same three things
//!
//! Re-deriving both focuses is only two of them. The third is dropping a
//! pointer grab, and it is the one a call site can silently forget: a grab
//! outlives focus changes *by design*, so a transition that re-derived focus
//! and stopped there would leave the grabbing client -- a drag-and-drop
//! started before the transition -- receiving exactly the pointer events the
//! focus change was meant to take away from it. [`State::lock_transition`] is
//! the three together, and every transition calls it rather than repeating
//! them: [`SessionLockHandler::lock`], [`State::refresh_lock_state`],
//! [`State::lock_post_frame`]'s caller in the render loop, and both
//! destruction hooks (`handlers.rs` for a destroyed `wl_surface`,
//! `dispatch.rs` for a destroyed lock-surface *role object* --
//! [`State::lock_surface_destroyed`]). [`SessionLockHandler::unlock`] is the
//! single deliberate exception, documented there.
//!
//! No keyboard grab is dropped because nothing in this compositor installs
//! one; if one is ever added, [`State::drop_pointer_grab`] is where it has to
//! be dropped too, or the same asymmetry becomes a keystroke leak instead of
//! a pointer one.

use std::time::Duration;

use smithay::backend::input::InputTime;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::{Color32F, ImportAll, Renderer, Texture};
use smithay::desktop::WindowSurfaceType;
use smithay::desktop::utils::{send_frames_surface_tree, under_from_surface_tree};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use super::State;
use super::output_scale::logical_size;

#[cfg(test)]
mod tests;

/// What a locked output is painted with when the lock client is alive: opaque
/// black, the "blank all outputs with an opaque color" the protocol requires.
const BACKDROP: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);

/// ...and what it is painted with once the lock client has died without
/// unlocking. Deliberately unmistakable, and deliberately the same signal
/// sway uses for the same state: a black screen that never comes back is
/// indistinguishable from a lock screen that simply draws black, and a user
/// needs to be able to tell those apart to know that re-running their locker
/// is what fixes it.
const ABANDONED_BACKDROP: Color32F = Color32F::new(1.0, 0.0, 0.0, 1.0);

/// The compositor's whole `ext-session-lock-v1` state.
pub struct SessionLock {
    /// Keeps the `ext_session_lock_manager_v1` global alive, and holds
    /// Smithay's own lock bookkeeping -- which gates `unlock_and_destroy`
    /// and `destroy` on the object that actually owns the lock, and is
    /// reached only through [`SessionLockHandler::lock_state`].
    manager: SessionLockManagerState,
    /// The `ext_session_lock_v1` object holding the current lock, if the
    /// session is locked at all. See this module's doc: this being `Some` is
    /// the definition of "locked", and it has exactly one writer per value.
    owner: Option<ExtSessionLockV1>,
    /// A lock that has been accepted but whose `locked` event has not gone
    /// out yet, because no blanked frame has been drawn since. The protocol
    /// forbids sending `locked` before that ("must not be sent until a new
    /// 'locked' frame ... has been presented on all outputs"), which is what
    /// keeps a client that suspends the machine on `locked` from racing an
    /// unlocked frame onto the screen.
    ///
    /// **Invariant, which [`SessionLockHandler::lock`]'s fast-confirm depends
    /// on: while this is `Some`, its `ext_session_lock()` is the same object
    /// as [`SessionLock::owner`].** Both are written together, from the one
    /// `confirmation` a `lock` call is handed, and `confirm_lock`/`unlock`
    /// only ever clear this one. That is what makes "locked, with nothing
    /// pending" mean "a blanked frame has been drawn for the lock we hold"
    /// rather than "for some other lock": if the two could name different
    /// objects, the dead-`pending` sweep could clear a `pending` belonging to
    /// a lock that never blanked the screen and leave the next lock being
    /// fast-confirmed over a fully visible desktop.
    ///
    /// Dropping a [`SessionLocker`] sends `finished` instead, which is how
    /// every refusal below tells a client its lock did not take.
    pending: Option<SessionLocker>,
    /// Every lock surface this compositor has been handed and not yet dropped,
    /// in creation order -- the first *current* one holds the keyboard, which
    /// is the focus rule the protocol itself suggests. One per output per
    /// lock; flexwm has one output today (see `headless.rs`'s `OUTPUT_ID`).
    ///
    /// Not every entry is necessarily current: see this module's "Which lock
    /// surfaces count". Nothing may read this field directly -- reads go
    /// through [`SessionLock::current`], which is what applies [`is_current`].
    /// The complete set of writers, so that rule can be audited in one grep:
    /// the `push` in [`SessionLockHandler::new_surface`] (itself gated on the
    /// same predicate), the `clear` in [`SessionLockHandler::lock`] and
    /// [`SessionLockHandler::unlock`], and the `retain` in
    /// [`SessionLock::cleanup`] and [`State::forget_lock_surface`].
    surfaces: Vec<LockSurface>,
    /// The opaque full-output rectangle drawn behind the lock surfaces.
    ///
    /// A persistent buffer updated in place, not a fresh one per frame, for
    /// the reason `decorations.rs` documents: a new element `Id` every frame
    /// reads as new content to [`smithay::backend::renderer::damage::OutputDamageTracker`].
    /// It is also why the blanking is an *element* rather than only
    /// `render_output`'s `clear_color`: the clear color is not part of any
    /// element's damage, so under `--tty` (the one backend that passes a real
    /// buffer age) a frame whose elements did not change can legitimately
    /// report no damage and leave the previous pixels on the scanout buffer.
    /// This buffer's own commit counter moves when its color or size does, so
    /// locking, unlocking and the switch to [`ABANDONED_BACKDROP`] are all
    /// real damage. The clear color is set to the same color anyway, as a
    /// second line of defence.
    backdrop: SolidColorBuffer,
}

impl SessionLock {
    pub(super) fn new(display: &DisplayHandle) -> Self {
        Self {
            // No client filter: this compositor has no security-context
            // support to distinguish a privileged lock client from any other,
            // so restricting the global by client would only be theatre. See
            // `README.md` and
            // `docs/backlog/protocols/session-lock-global-restriction.md`.
            manager: SessionLockManagerState::new::<State, _>(display, |_| true),
            owner: None,
            pending: None,
            surfaces: Vec::new(),
            backdrop: SolidColorBuffer::default(),
        }
    }

    /// Whether the session is locked: nothing but lock surfaces may be drawn,
    /// and nothing but lock surfaces may receive input.
    pub(super) fn is_locked(&self) -> bool {
        self.owner.is_some()
    }

    /// Whether the session is locked and the client that locked it is gone.
    ///
    /// Never true while unlocked, because it is derived from the same field.
    pub(super) fn abandoned(&self) -> bool {
        self.owner.as_ref().is_some_and(|lock| !lock.is_alive())
    }

    /// The lock surfaces that may be drawn, focused and hit-tested right now:
    /// the current lock's own, in creation order.
    ///
    /// The single reader of [`SessionLock::surfaces`], so that a reader added
    /// to this module later cannot accidentally ask the weaker question (see
    /// this module's "Which lock surfaces count" for what the weaker questions
    /// let through).
    ///
    /// Costs one `is_alive` and one object-id comparison per surface per read,
    /// allocates nothing, and only runs at all while the session is locked --
    /// where the list is one surface per output.
    fn current(&self) -> impl Iterator<Item = &LockSurface> {
        let owner = self.owner.as_ref();
        self.surfaces
            .iter()
            .filter(move |surface| is_current(owner, surface))
    }

    /// The surface the keyboard goes to while locked: the current lock's first
    /// surface, or nobody.
    ///
    /// Liveness, not mapped-ness, deliberately -- the opposite of
    /// `layer_shell.rs`'s rule, and for the opposite reason. There, a surface
    /// holding every keystroke while showing nothing was the hazard; here
    /// *nothing else may have the keyboard at all*, so handing it to a lock
    /// surface that has not drawn yet is strictly better than handing it to
    /// nobody: the client is mid-startup and its first keystrokes are the
    /// user's password. That argument only holds for a surface whose lock is
    /// the live one, which is why this asks [`SessionLock::current`] rather
    /// than `alive()`: handing the keyboard to a *former* locker's surface is
    /// not "better than nobody", it is the whole password going to whoever
    /// left it there.
    fn keyboard_focus(&self) -> Option<WlSurface> {
        self.current()
            .next()
            .map(|surface| surface.wl_surface().clone())
    }

    /// This frame's backdrop element, sized to the output and coloured by
    /// whether the lock has been abandoned.
    fn backdrop_element(
        &mut self,
        origin: Point<i32, Physical>,
        size: (i32, i32),
    ) -> SolidColorRenderElement {
        let color = self.backdrop_color();
        // Negative dimensions are impossible here (the caller passes the
        // render target's own size, built from an output mode), and a zero
        // one simply draws nothing.
        self.backdrop.update(size, color);
        SolidColorRenderElement::from_buffer(&self.backdrop, origin, 1.0, 1.0, Kind::Unspecified)
    }

    /// The colour of both the backdrop element and the frame's clear colour
    /// while locked, so the two can never disagree.
    fn backdrop_color(&self) -> Color32F {
        if self.abandoned() {
            ABANDONED_BACKDROP
        } else {
            BACKDROP
        }
    }

    /// The render elements of every *mapped* lock surface, front-most first.
    ///
    /// Mapped-ness here is `last_acked`, which Smithay's own pre-commit hook
    /// maintains as "has a buffer" (`session_lock/surface.rs`), the same test
    /// `layer_shell.rs` uses for a layer surface. An unmapped one produces no
    /// elements and the backdrop shows through, which is exactly what the
    /// protocol asks for while a lock client is still starting up.
    fn surface_elements<R>(
        &self,
        renderer: &mut R,
        origin: Point<i32, Physical>,
        scale: f64,
    ) -> Vec<WaylandSurfaceRenderElement<R>>
    where
        R: Renderer + ImportAll,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let mut elements = Vec::new();
        for surface in self.current() {
            if !is_mapped(surface) {
                continue;
            }
            elements.extend(render_elements_from_surface_tree(
                renderer,
                surface.wl_surface(),
                origin,
                scale,
                1.0,
                Kind::Unspecified,
            ));
        }
        elements
    }

    /// Sends this frame's callbacks to every current lock surface.
    ///
    /// Sent to all of them rather than only the ones that produced an
    /// element, matching `render()`'s window, cursor and layer-surface loops
    /// and for the same reason: a client may legitimately ask for a callback
    /// before its first attach, and withholding it would stall the very frame
    /// that unsticks it -- here, the first frame of the lock screen itself.
    fn send_frames(&self, output: &Output, time: Duration) {
        for surface in self.current() {
            send_frames_surface_tree(
                surface.wl_surface(),
                output,
                time,
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
    }

    /// Drops every lock surface that has stopped counting, reporting whether
    /// any went.
    ///
    /// The same predicate the readers use, so this can only ever remove
    /// surfaces they were already ignoring -- it exists to stop this
    /// compositor *holding* them (a `wl_surface` handle, and through it an
    /// `Arc` on client state), and so that the callers can re-derive focus off
    /// its return value.
    ///
    /// The second line of defence behind `handlers.rs`'s `destroyed` hook, in
    /// the same spirit as `LayerMap::cleanup` in the render loop: a client
    /// whose teardown ran in an order that left a dead surface here must not
    /// leave it holding the keyboard or an `Arc` on a destroyed client's
    /// state.
    fn cleanup(&mut self) -> bool {
        let before = self.surfaces.len();
        // Split borrow: `owner` is read, `surfaces` is written, and they are
        // different fields of the same struct.
        let owner = self.owner.as_ref();
        self.surfaces.retain(|surface| is_current(owner, surface));
        before != self.surfaces.len()
    }

    /// Configures every current lock surface to `size`.
    ///
    /// Called when the output's mode changes: a lock surface's size is an
    /// exact requirement (committing a buffer of any other size is a protocol
    /// error), so a resized output has to reconfigure them or the next commit
    /// kills the lock client.
    fn configure_all(&self, size: (i32, i32)) {
        for surface in self.current() {
            configure(surface, size);
        }
    }

    /// The current lock surface under `position`, if any.
    fn surface_under(
        &self,
        position: Point<f64, Logical>,
        origin: Point<i32, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.current()
            .find_map(|surface| {
                under_from_surface_tree(
                    surface.wl_surface(),
                    position,
                    origin,
                    WindowSurfaceType::ALL,
                )
            })
            .map(|(surface, location)| (surface, location.to_f64()))
    }
}

/// Whether `surface` is one of the surfaces the session's *current* lock put
/// up: all three of "its `wl_surface` still exists", "the lock that created it
/// is the one that owns the session" and "that lock still exists".
///
/// The one definition of what counts, taken as a free function rather than a
/// method so [`SessionLock::cleanup`] can apply it while holding `surfaces`
/// mutably. See this module's "Which lock surfaces count" for why each of the
/// three is load-bearing.
///
/// The `is_alive` is asked of `owner` rather than of `surface.ext_session_lock()`
/// only because it reads as the question being asked ("is this session
/// abandoned"); the equality above it means they are the same object, so the
/// two spellings cannot disagree.
fn is_current(owner: Option<&ExtSessionLockV1>, surface: &LockSurface) -> bool {
    surface.alive()
        && owner.is_some_and(|owner| owner.is_alive() && owner == surface.ext_session_lock())
}

/// Sets a lock surface's pending size and sends the configure carrying it.
///
/// `u32` because that is what the protocol's `configure` event carries;
/// negatives are impossible (an output mode's dimensions) and clamped to zero
/// rather than wrapping if one ever were.
fn configure(surface: &LockSurface, (width, height): (i32, i32)) {
    let size = (width.max(0) as u32, height.max(0) as u32);
    surface.with_pending_state(|state| state.size = Some(size.into()));
    surface.send_configure();
}

/// Whether a lock surface has actually committed a buffer.
fn is_mapped(surface: &LockSurface) -> bool {
    surface.with_cached_state(|state| state.last_acked.is_some())
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock.manager
    }

    /// A client asked to lock the session.
    ///
    /// Three answers, in the order they are decided:
    ///
    /// 1. Another client's lock is still taking effect (accepted, not yet
    ///    confirmed): refused. Two clients racing to lock must not both end
    ///    up believing they own the session.
    /// 2. The session is already locked by a client that is still alive:
    ///    refused. This is the ordinary "swaylock is already running" case.
    /// 3. Otherwise accepted -- either a fresh lock, or a takeover of a lock
    ///    whose client is gone (see this module's doc). Confirmation is
    ///    immediate only when a blanked frame has genuinely already been drawn
    ///    (`already_blanked` below); otherwise, fresh lock or takeover alike,
    ///    it waits for one ([`State::confirm_lock`]).
    ///
    /// A refusal is a `finished` event, which is what dropping the
    /// [`SessionLocker`] sends. It is never a protocol error: asking to lock
    /// an already-locked session is legal, and `finished` is the protocol's
    /// own answer for "the compositor decided not to".
    fn lock(&mut self, confirmation: SessionLocker) {
        // Whether the screen this lock is arriving onto is *already* blanked,
        // which is the entire justification for confirming a takeover without
        // drawing a frame first. `pending` is cleared by `confirm_lock` when a
        // locked frame has actually been drawn, so "locked, with nothing
        // pending" is exactly that state -- and "locked, still pending" means
        // the previous lock never got its blanked frame, so the user's own
        // desktop is still what is on the display.
        //
        // Read here, before the dead-`pending` sweep below clears the evidence:
        // that sweep exists to stop a dead locker blocking a replacement, and
        // reading after it would make the never-confirmed case indistinguishable
        // from the confirmed one -- i.e. would hand the replacement `locked`
        // with the unlocked session still on screen, the precise race the event
        // exists to prevent.
        let already_blanked = self.session_lock.is_locked() && self.session_lock.pending.is_none();

        // A confirmation whose own client died before its first frame can
        // never be confirmed into anything meaningful, and must not block a
        // replacement locker from starting. Dropping it sends `finished` to
        // an object that no longer exists, which is a no-op.
        if self
            .session_lock
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.ext_session_lock().is_alive())
        {
            self.session_lock.pending = None;
        }
        if self.session_lock.pending.is_some() {
            tracing::info!("refusing a session lock: another client's lock is still taking effect");
            return;
        }
        if self
            .session_lock
            .owner
            .as_ref()
            .is_some_and(|owner| owner.is_alive())
        {
            tracing::info!("refusing a session lock: the session is already locked");
            return;
        }

        if self.session_lock.is_locked() {
            tracing::info!(
                already_blanked,
                "a new client is taking over an abandoned session lock"
            );
        } else {
            tracing::info!("locking the session");
        }
        self.session_lock.owner = Some(confirmation.ext_session_lock().clone());
        // Whatever is left belongs to the lock just replaced -- and the
        // replaced client may still be connected, holding a live `wl_surface`,
        // if it gave its lock up rather than died. None of it may be drawn or
        // focused again, so it goes now rather than being filtered forever.
        // (Empty already on the fresh-lock path: `unlock` clears it too.)
        self.session_lock.surfaces.clear();
        if already_blanked {
            // The outputs are blank and stay blank across this handover, so
            // the protocol's reason for waiting is satisfied by construction
            // and the new client can put its own surfaces up straight away.
            confirmation.lock();
        } else {
            self.session_lock.pending = Some(confirmation);
        }

        // Whatever cursor image a client asked for before the lock is that
        // client's own pixels, and the protocol says only lock surfaces are
        // rendered. Back to the compositor's own shape; from here only the
        // lock client can change it, because only it has pointer focus.
        //
        // Before the transition below rather than after, and the order does
        // not matter: the only cursor-status write `lock_transition` can
        // cause is `PointerTarget::replace`'s own reset -- `unset_grab`
        // restores focus with a motion, and a motion that replaces one focus
        // with another calls `cursor_image(default_named())` -- which is the
        // same value this line sets. A client's `wl_pointer.set_cursor` in
        // answer to the enter is a later request, not a synchronous callback,
        // and by then only the lock client can send one.
        self.cursor.set_status(CursorImageStatus::default_named());
        self.lock_transition();
    }

    /// The owning client unlocked. Smithay has already checked that the
    /// request came from the object that actually holds the lock (and posted
    /// `invalid_unlock` if it did not), so this is unconditional.
    fn unlock(&mut self) {
        tracing::info!("unlocking the session");
        self.session_lock.owner = None;
        // Unreachable while `pending` is set -- Smithay only routes
        // `unlock_and_destroy` here once `locked` has been sent, which is
        // what clears `pending` -- but leaving a stale confirmation behind
        // would keep an unlocked session waiting to confirm a lock.
        self.session_lock.pending = None;
        self.session_lock.surfaces.clear();
        // Deliberately not [`State::lock_transition`]: this is the one lock
        // transition that hands the session *back*, so a pointer grab has to
        // survive it exactly as it survives any other focus change. Only the
        // unlocking client itself can hold one by now -- nothing else has had
        // pointer focus since the lock -- so dropping it here would break a
        // drag that client is entitled to finish, and would protect nobody.
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
    }

    /// The lock client created a surface for an output.
    ///
    /// Smithay sends this surface's first configure the moment this returns,
    /// so the size has to be set here: the configure is an exact requirement
    /// the client's first buffer must match.
    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        // Smithay may call this for a lock this compositor refused, if that
        // refusal and this request cross on the wire; its own docs say to
        // match surfaces to lockers by their `ext_session_lock_v1`. Anything
        // else must not end up on screen or holding the keyboard.
        //
        // `owner` is the right thing to compare against, including while a
        // lock is still pending confirmation: it is set the moment a lock is
        // *accepted*, not when it is confirmed (see `lock` above), so there is
        // no window in which an accepted lock's surfaces would fail this check.
        //
        // Asked through the same [`is_current`] every reader uses, so the set
        // of surfaces that may enter this list is exactly the set that may be
        // read out of it -- one predicate, not two that could drift.
        if !is_current(self.session_lock.owner.as_ref(), &surface) {
            tracing::warn!("ignoring a lock surface from a lock this compositor did not accept");
            return;
        }
        // The output the client named, falling back to this compositor's own
        // -- there is exactly one, so the two are the same object today (see
        // `headless.rs`'s `OUTPUT_ID` for what multi-output has to revisit).
        let Some(output) = Output::from_resource(&output).or_else(|| self.output.clone()) else {
            tracing::warn!("no output for a lock surface");
            return;
        };
        let size = logical_size(&output);
        configure(&surface, size);
        self.session_lock.surfaces.push(surface);
        // The first surface takes the keyboard off nobody, and the pointer
        // has to enter it rather than wait for the user to move the mouse.
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
    }
}

impl State {
    /// Everything the compositor has to catch up when the set of lock
    /// surfaces that may be drawn and focused has just changed: any input
    /// grab dropped, both focuses re-derived, the screen marked dirty.
    ///
    /// One function rather than the same three calls repeated, because the
    /// three are not independent and the *asymmetry* is what goes wrong: a
    /// transition that re-derived focus but left a grab installed would leave
    /// the grabbing client receiving pointer events the focus change was
    /// supposed to take away from it, and that mistake is invisible at the
    /// call site that forgot it. The callers are
    /// [`SessionLockHandler::lock`] (a fresh lock or a takeover),
    /// [`State::refresh_lock_state`] (a `destroy`/disconnect observed on the
    /// wayland connection), [`State::lock_post_frame`]'s caller (the render
    /// loop's own cleanup pass) and the two destruction hooks --
    /// `handlers.rs`'s `CompositorHandler::destroyed` (the `wl_surface` went)
    /// and `dispatch.rs`'s lock-surface `destroyed` (only the role object
    /// went).
    ///
    /// Not used by [`SessionLockHandler::unlock`], which is the one
    /// transition that gives the session *back* -- see the note there.
    pub(super) fn lock_transition(&mut self) {
        // Before the focus refreshes, not after: `unset_grab` restores focus
        // to whatever the pointer had pending, so re-deriving afterwards is
        // what gets the final word.
        self.drop_pointer_grab();
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
    }

    /// Drops a pointer grab so it cannot outlive a lock transition.
    ///
    /// A grab deliberately outlives focus changes -- that is what a grab *is*
    /// -- so re-deriving focus is not enough on its own: the grabbing client
    /// goes on receiving every motion and button until the grab ends by
    /// itself. Two kinds can be active here, and both matter:
    ///
    /// - **A drag-and-drop**, installed by `handlers.rs`'s
    ///   `WaylandDndGrabHandler` at a client's own request. It ends only when
    ///   the drag does, and while it lasts it routes pointer events through
    ///   `DnDGrab` rather than through focus at all.
    /// - **The implicit click grab**, which is not flexwm's code but is
    ///   nonetheless installed on *every* button press: Smithay's
    ///   `DefaultGrab::button` calls `SeatHandler::click_grab` (flexwm takes
    ///   the default `ClickGrab`) and sets it. It releases itself once every
    ///   button is up -- so this only ever finds one with a button still
    ///   held, which is exactly the case that must not survive a lock
    ///   transition: a press delivered to a surface before the transition
    ///   would otherwise keep steering the pointer afterwards.
    ///
    /// **If a keyboard or touch grab is ever added to this compositor, it has
    /// to be dropped here too.** Neither exists today -- nothing installs a
    /// keyboard grab, and nothing calls `Seat::add_touch`, so Smithay's
    /// `TouchDownGrab` (the touch twin of the click grab above) is never
    /// reached -- which is the only reason this is a pointer-only function
    /// and the only reason the same asymmetry was not already a keystroke
    /// leak.
    ///
    /// Costs one mutex-guarded enum check on a transition that has already
    /// decided to re-derive focus and redraw; nothing on any per-event path.
    fn drop_pointer_grab(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if !pointer.is_grabbed() {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        pointer.unset_grab(self, serial, time);
    }

    /// A lock surface's *role object* was destroyed, with its `wl_surface`,
    /// its lock and its client all still alive.
    ///
    /// This is the protocol's "If a lock surface on an active output is
    /// destroyed before the `ext_session_lock_v1.unlock_and_destroy` event is
    /// sent, the compositor must fall back to rendering a solid color" -- and
    /// it is legal, ordinary behaviour: it is what a locker does when an
    /// output goes away under it.
    ///
    /// Smithay's `ExtLockSurfaceUserData::destroyed` resets the surface's
    /// `last_acked`, so [`is_mapped`] goes false and the surface stops
    /// producing render elements from the very next frame -- but *nothing
    /// asks for that frame*. Without this hook the destroyed surface's last
    /// pixels stay on the display indefinitely, until something unrelated
    /// (a pointer motion, the backdrop turning red) happens to mark the
    /// screen dirty. Same shape as the abandoned-backdrop bug
    /// [`State::refresh_lock_state`] documents, on the one teardown path
    /// that reaches no other hook: no `wl_surface` is destroyed, so
    /// `handlers.rs`'s `CompositorHandler::destroyed` never runs, and no lock
    /// object is destroyed, so `refresh_lock_state`'s two questions both
    /// still answer "nothing changed".
    ///
    /// What this deliberately does *not* do is drop the orphaned
    /// [`LockSurface`] from [`SessionLock::surfaces`]: it still passes
    /// [`is_current`] (its `wl_surface` is alive and its lock is still the
    /// owner), and there is no public way to tell *which* stored surface the
    /// destroyed role object belonged to -- both `ExtLockSurfaceUserData`'s
    /// and `LockSurfaceAttributes`' handle on it is `pub(crate)` in Smithay.
    /// It costs nothing to leave: it can never be drawn again (`last_acked`
    /// is reset for good, and a later commit on it is a protocol error), and
    /// the keyboard it may still hold is the lock client's own -- the same
    /// client that owns the session -- which cannot put a replacement surface
    /// up for that output anyway, because Smithay's `locked_outputs` list
    /// never shrinks. It goes for real when that `wl_surface` is destroyed,
    /// when the client disconnects, or at the next takeover.
    ///
    /// The visible consequence, seen on real `--tty` hardware rather than
    /// inferred: the pointer refresh below can *enter* that orphan, because
    /// its `wl_surface` still carries the buffer and input region the hit
    /// test reads. Same client, nothing else is reachable while locked, and
    /// it is no longer drawn -- so this is a cosmetic consequence of leaving
    /// the orphan registered, not a route anywhere.
    ///
    /// Runs only while the session is locked: with the session unlocked no
    /// lock surface is drawn, focused or hit-tested at all, so a late
    /// teardown after an unlock has nothing to catch up (and
    /// [`SessionLockHandler::unlock`] has already asked for its own redraw).
    /// That is the only question asked, deliberately -- the destroyed role
    /// object carries no handle this side can read back to a `LockSurface`
    /// (see above), so a role object belonging to a lock this compositor
    /// *refused* costs one redundant redraw while someone else holds the
    /// lock. That is bounded by the frame timer and is no more than the
    /// `request_render` any client's own `wl_surface.commit` already asks
    /// for, so it does not need a guard of its own.
    pub(super) fn lock_surface_destroyed(&mut self) {
        if !self.session_lock.is_locked() {
            return;
        }
        self.lock_transition();
    }

    /// Confirms a pending lock now that a blanked frame has been drawn.
    ///
    /// Called from `headless.rs::render` after a successful frame, which is
    /// the closest this compositor gets to the protocol's "presented on all
    /// outputs" -- and closer on some backends than others:
    ///
    /// - `--headless`/`--nested` have no scanout at all, and the framebuffer a
    ///   screenshot reads *is* this frame, so this is exact.
    /// - `--tty` renders into the pixman image here; `Tty::present` then copies
    ///   it into a dumb buffer and asks for a page flip -- but only if it can.
    ///   It returns without doing either when the session is paused/inactive,
    ///   or when a previous flip has not been confirmed by a `VBlank` yet
    ///   (`flip_pending`, which its own comment describes as an ordinary,
    ///   frequent, harmless throttle, not an edge case). So under contention
    ///   the previous -- possibly unlocked -- frame can still be on the scanout
    ///   buffer for up to one more vblank after `locked` has gone out.
    ///
    /// `README.md` states that weaker guarantee in those terms rather than
    /// claiming vblank accuracy, and
    /// `docs/backlog/protocols/session-lock-vblank-confirm.md` carries closing it
    /// (confirming from the `DrmEvent::VBlank` handler instead) as its own
    /// item -- it means tracking which in-flight flip carries the blanked frame
    /// through a path that also has to not hang a locker when the session is
    /// switched away, which is more than a doc fix's worth of presentation-path
    /// change.
    ///
    /// Costs one `Option` check on every frame that is not locking.
    pub(super) fn confirm_lock(&mut self) {
        if let Some(confirmation) = self.session_lock.pending.take() {
            tracing::debug!("session lock confirmed: a blanked frame has been drawn");
            // A no-op if the client died in the meantime: the generated event
            // sender discards the send error for a destroyed object. The
            // session stays locked either way -- `owner` is untouched here --
            // and reads as abandoned from the next frame on.
            confirmation.lock();
        }
    }

    /// Catches up the two things that change when a lock stops being the
    /// current one and *nothing else notices*: the lock surfaces it left
    /// behind, and the backdrop's colour.
    ///
    /// A client disconnecting -- or destroying its lock object while keeping
    /// the connection -- destroys protocol objects; it does not commit a
    /// surface, move a pointer or press a key, so no existing path marks the
    /// screen dirty or re-derives focus, and the last frame drawn stays on the
    /// display with the last focus still pointing wherever it pointed.
    ///
    /// **Surfaces.** [`SessionLock::cleanup`] drops whatever has stopped
    /// counting, and both focuses are re-derived if anything did. Both halves
    /// matter, and pointer focus is the one that is easy to miss:
    /// `wl_pointer.button` goes to whatever the pointer last *entered*, never
    /// to whatever the hit test would return now, so a former locker's surface
    /// that keeps pointer focus keeps receiving clicks no matter what the hit
    /// test says. Doing it here rather than only at the next frame means it
    /// happens in the same dispatch cycle that observed the disconnect -- i.e.
    /// before any later cycle could route input.
    ///
    /// **Backdrop.** Found on real `--tty` hardware rather than by inspection:
    /// `kill -9` on a lock client that had already destroyed its lock surface
    /// left a black screen (the previous frame) instead of the red one that
    /// tells a user their locker crashed. The same-looking case where a lock
    /// surface *was* still up happened to work, because destroying that surface
    /// goes through `handlers.rs`'s `destroyed` hook -- i.e. the signal was
    /// correct only by accident of teardown order, which is exactly the kind of
    /// implicit dependency worth removing rather than documenting. Compares
    /// against the backdrop buffer's *own* colour rather than a remembered
    /// flag, so there is no second piece of state to fall out of step with what
    /// was actually drawn.
    ///
    /// Called from the place a disconnect is observed: the wayland display
    /// source in `state.rs`. That is where the *lock object* going away is
    /// caught; the two surface-level teardowns have their own explicit hooks
    /// (`handlers.rs`'s `CompositorHandler::destroyed` for the `wl_surface`,
    /// `dispatch.rs`'s for the role object alone), so none of the three
    /// depends on another one's ordering to be noticed. Costs one
    /// `Option::is_some` on every wayland dispatch cycle with the session
    /// unlocked, which is every cycle in ordinary use; while locked it adds a
    /// `retain` over one surface per output and one colour compare, on a
    /// connection carrying only the lock client's own traffic. No allocation
    /// either way.
    pub(super) fn refresh_lock_state(&mut self) {
        if !self.session_lock.is_locked() {
            return;
        }
        if self.session_lock.cleanup() {
            self.lock_transition();
        }
        if self.session_lock.backdrop.color() != self.session_lock.backdrop_color() {
            // The lock has just been abandoned with no surface of its own to
            // drop above -- the `kill -9` case in this doc. A grab the dead
            // client started still outlives that by design, and until some
            // later takeover happens to drop it, it keeps taking pointer
            // events away from the lock screen, so it goes here too. Only
            // reachable on the edge: the next locked frame repaints the
            // backdrop in the new colour, and the comparison is false again.
            self.drop_pointer_grab();
            self.request_render();
        }
    }

    /// This frame's lock elements, front-most first: the mapped lock
    /// surfaces, then the opaque backdrop behind them.
    ///
    /// The whole element list while locked, by construction -- the caller
    /// adds only the cursor in front of it. Nothing else is gathered, so
    /// there is no ordering mistake that could put a window behind the
    /// backdrop instead of out of the frame entirely.
    pub(super) fn lock_elements<R>(
        &mut self,
        renderer: &mut R,
        origin: Point<i32, Physical>,
        scale: f64,
        size: (i32, i32),
    ) -> (Vec<WaylandSurfaceRenderElement<R>>, SolidColorRenderElement)
    where
        R: Renderer + ImportAll,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let surfaces = self.session_lock.surface_elements(renderer, origin, scale);
        let backdrop = self.session_lock.backdrop_element(origin, size);
        (surfaces, backdrop)
    }

    /// What a locked frame clears to: the same colour as the backdrop element
    /// that covers it.
    pub(super) fn lock_clear_color(&self) -> Color32F {
        self.session_lock.backdrop_color()
    }

    /// Frame callbacks and stale-surface cleanup for a locked frame.
    ///
    /// Returns whether a lock surface was dropped, which is a reason to
    /// re-derive both focuses: the surface that went may have held either.
    pub(super) fn lock_post_frame(&mut self, output: &Output, time: Duration) -> bool {
        self.session_lock.send_frames(output, time);
        self.session_lock.cleanup()
    }

    /// The keyboard focus while locked, for `shell.rs::refresh_keyboard_focus`.
    pub(super) fn lock_keyboard_focus(&self) -> Option<WlSurface> {
        self.session_lock.keyboard_focus()
    }

    /// The lock surface under `position`, if any -- the whole of pointer
    /// hit-testing while locked.
    ///
    /// Asks the surface tree, so a client's own `set_input_region` is
    /// honoured exactly as it is for a window; a lock surface that excludes
    /// a point simply gets no pointer there, and nothing behind it is
    /// consulted, because nothing behind it is reachable.
    pub(super) fn lock_surface_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let origin = self
            .output
            .as_ref()
            .and_then(|output| self.space.output_geometry(output))
            .map(|geometry| geometry.loc)
            .unwrap_or_default();
        self.session_lock.surface_under(position, origin)
    }

    /// Drops `surface` if it is one of the lock surfaces, reporting whether
    /// it was.
    ///
    /// Called from `CompositorHandler::destroyed`, so a lock client tearing
    /// down one surface (or disconnecting entirely) stops that surface being
    /// drawn or focused on the very next frame rather than at the next
    /// cleanup pass.
    pub(super) fn forget_lock_surface(&mut self, surface: &WlSurface) -> bool {
        let before = self.session_lock.surfaces.len();
        self.session_lock
            .surfaces
            .retain(|lock| lock.wl_surface() != surface);
        before != self.session_lock.surfaces.len()
    }

    /// Reconfigures every lock surface for a new output size.
    pub(super) fn resize_lock_surfaces(&mut self, size: (i32, i32)) {
        self.session_lock.configure_all(size);
    }
}
