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
//! [`SessionLock::owner`] is that field, and it is deliberately the *only*
//! one: `owner.is_some()` **is** "the session is locked", so there is no
//! second boolean that could disagree with it (see `ROADMAP.md` item 5b for
//! why this project treats a field with two meanings as a bug class of its
//! own). It is written `Some` in exactly one place -- [`SessionLockHandler::lock`],
//! when a lock is accepted -- and `None` in exactly one place --
//! [`SessionLockHandler::unlock`], when the owning client unlocks. Nothing
//! else, in any backend, in any error path, may write it; in particular a
//! VT switch, a session pause, a failed render and a dying client all leave
//! it exactly as it was, which is what makes the lock survive them.
//!
//! Two derived questions come off that same field, so they cannot drift:
//! [`SessionLock::is_locked`] (`owner.is_some()`) and
//! [`SessionLock::abandoned`] (`owner` is `Some` but its client is gone).
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
//! over -- run a lock client again and it gets `locked` immediately (the
//! outputs are already blanked) and can unlock after authenticating. Both
//! behaviors match sway (`sway/lock.c`'s `handle_abandon` paints the same
//! red and `handle_session_lock` replaces an abandoned lock) and niri
//! (`Niri::lock` replaces a lock whose client is not alive). The alternative
//! -- no takeover at all -- would make a crashed locker a permanently
//! unusable session with only a compositor restart as the way out, which is
//! its own kind of failure.
//!
//! The honest consequence, which `README.md` states too: while a lock is
//! abandoned, any client that can reach this compositor's wayland socket can
//! take it over and unlock. That is the same-uid trust boundary flexwm's IPC
//! socket already has, and it is the price of having a recovery path at all.

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
    /// Dropping a [`SessionLocker`] sends `finished` instead, which is how
    /// every refusal below tells a client its lock did not take.
    pending: Option<SessionLocker>,
    /// The lock surfaces of the owning lock, in creation order -- the first
    /// live one holds the keyboard, which is the focus rule the protocol
    /// itself suggests. One per output; flexwm has one output today (see
    /// `headless.rs`'s `OUTPUT_ID`).
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
            // `README.md` and the backlog entry in `ROADMAP.md`.
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

    /// The surface the keyboard goes to while locked: the first lock surface
    /// still alive, or nobody.
    ///
    /// Liveness, not mapped-ness, deliberately -- the opposite of
    /// `layer_shell.rs`'s rule, and for the opposite reason. There, a surface
    /// holding every keystroke while showing nothing was the hazard; here
    /// *nothing else may have the keyboard at all*, so handing it to a lock
    /// surface that has not drawn yet is strictly better than handing it to
    /// nobody: the client is mid-startup and its first keystrokes are the
    /// user's password.
    fn keyboard_focus(&self) -> Option<WlSurface> {
        self.surfaces
            .iter()
            .find(|surface| surface.alive())
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
        for surface in &self.surfaces {
            if !surface.alive() || !is_mapped(surface) {
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

    /// Sends this frame's callbacks to every live lock surface.
    ///
    /// Sent to all of them rather than only the ones that produced an
    /// element, matching `render()`'s window, cursor and layer-surface loops
    /// and for the same reason: a client may legitimately ask for a callback
    /// before its first attach, and withholding it would stall the very frame
    /// that unsticks it -- here, the first frame of the lock screen itself.
    fn send_frames(&self, output: &Output, time: Duration) {
        for surface in &self.surfaces {
            if !surface.alive() {
                continue;
            }
            send_frames_surface_tree(
                surface.wl_surface(),
                output,
                time,
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
    }

    /// Drops lock surfaces whose client has gone, reporting whether any went.
    ///
    /// The second line of defence behind `handlers.rs`'s `destroyed` hook, in
    /// the same spirit as `LayerMap::cleanup` in the render loop: a client
    /// whose teardown ran in an order that left a dead surface here must not
    /// leave it holding the keyboard or an `Arc` on a destroyed client's
    /// state.
    fn cleanup(&mut self) -> bool {
        let before = self.surfaces.len();
        self.surfaces.retain(LockSurface::alive);
        before != self.surfaces.len()
    }

    /// Configures every lock surface to `size`.
    ///
    /// Called when the output's mode changes: a lock surface's size is an
    /// exact requirement (committing a buffer of any other size is a protocol
    /// error), so a resized output has to reconfigure them or the next commit
    /// kills the lock client.
    fn configure_all(&self, size: (i32, i32)) {
        for surface in &self.surfaces {
            if !surface.alive() {
                continue;
            }
            configure(surface, size);
        }
    }
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
    ///    whose client died (see this module's doc). A takeover is confirmed
    ///    immediately, because the outputs are already blanked and the
    ///    protocol's reason for waiting is satisfied by construction; a fresh
    ///    lock waits for the first blanked frame ([`State::confirm_lock`]).
    ///
    /// A refusal is a `finished` event, which is what dropping the
    /// [`SessionLocker`] sends. It is never a protocol error: asking to lock
    /// an already-locked session is legal, and `finished` is the protocol's
    /// own answer for "the compositor decided not to".
    fn lock(&mut self, confirmation: SessionLocker) {
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
            // Takeover of an abandoned lock. Nothing about what is on screen
            // changes -- it is already blanked -- so this needs no frame
            // before `locked` goes out, and the new client can put its own
            // surfaces up straight away.
            tracing::info!("a new client is taking over an abandoned session lock");
            self.session_lock.owner = Some(confirmation.ext_session_lock().clone());
            // The previous client's surfaces died with it; anything still
            // here would be drawn from a destroyed client's state.
            self.session_lock.surfaces.retain(LockSurface::alive);
            confirmation.lock();
        } else {
            tracing::info!("locking the session");
            self.session_lock.owner = Some(confirmation.ext_session_lock().clone());
            self.session_lock.pending = Some(confirmation);
        }

        // A pointer grab (a drag-and-drop in flight, say) outlives focus
        // changes by design -- that is what a grab is -- so it has to be
        // dropped explicitly, or the client that started it keeps receiving
        // pointer events through the lock.
        if let Some(pointer) = self.seat.get_pointer()
            && pointer.is_grabbed()
        {
            let serial = SERIAL_COUNTER.next_serial();
            let time = InputTime::from_millis(self.millis());
            pointer.unset_grab(self, serial, time);
        }
        // Whatever cursor image a client asked for before the lock is that
        // client's own pixels, and the protocol says only lock surfaces are
        // rendered. Back to the compositor's own shape; from here only the
        // lock client can change it, because only it has pointer focus.
        self.cursor.set_status(CursorImageStatus::default_named());
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
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
        // `owner` alone is the right thing to compare against, including
        // while a lock is still pending confirmation: it is set the moment a
        // lock is *accepted*, not when it is confirmed (see `lock` above), so
        // there is no window in which an accepted lock's surfaces would fail
        // this check.
        let owns = self
            .session_lock
            .owner
            .as_ref()
            .is_some_and(|lock| lock == surface.ext_session_lock());
        if !owns {
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
        let size = output
            .current_mode()
            .map(|mode| (mode.size.w, mode.size.h))
            .unwrap_or((0, 0));
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
    /// Confirms a pending lock now that a blanked frame has been drawn.
    ///
    /// Called from `headless.rs::render` after a successful frame, which is
    /// the closest this compositor gets to the protocol's "presented on all
    /// outputs": under `--tty` that frame has been copied into the dumb
    /// buffer and a page flip asked for, so the very next scanout shows it;
    /// under `--headless`/`--nested` there is no scanout at all and the
    /// framebuffer a screenshot reads is already this frame. See `README.md`,
    /// which states the difference rather than claiming vblank accuracy.
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

    /// Marks the screen dirty if the backdrop's colour no longer matches what
    /// the lock's state says it should be -- i.e. if the lock client has died
    /// since the last frame was drawn.
    ///
    /// This exists because *nothing else notices*. A client disconnecting
    /// destroys protocol objects; it does not commit a surface, move a
    /// pointer or press a key, so no existing path marks the screen dirty,
    /// and the last frame drawn stays on the display. Found on real `--tty`
    /// hardware rather than by inspection: `kill -9` on a lock client that
    /// had already destroyed its lock surface left a black screen (the
    /// previous frame) instead of the red one that tells a user their locker
    /// crashed. The same-looking case where a lock surface *was* still up
    /// happened to work, because destroying that surface goes through
    /// `handlers.rs`'s `destroyed` hook -- i.e. the signal was correct only
    /// by accident of teardown order, which is exactly the kind of implicit
    /// dependency worth removing rather than documenting.
    ///
    /// Called from the one place a disconnect is observed: the wayland
    /// display source in `state.rs`. Costs one `Option::is_some` on every
    /// wayland dispatch cycle with the session unlocked, which is every cycle
    /// in ordinary use; while locked it costs one liveness check on an object
    /// id, on a connection carrying only the lock client's own traffic.
    ///
    /// Compares against the backdrop buffer's *own* colour rather than a
    /// remembered flag, so there is no second piece of state to fall out of
    /// step with what was actually drawn.
    pub(super) fn refresh_lock_backdrop(&mut self) {
        if !self.session_lock.is_locked() {
            return;
        }
        if self.session_lock.backdrop.color() != self.session_lock.backdrop_color() {
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

    /// Frame callbacks and dead-surface cleanup for a locked frame.
    ///
    /// Returns whether a dead lock surface was dropped, which is a reason to
    /// re-derive keyboard focus: the surface that died may have held it.
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
        self.session_lock
            .surfaces
            .iter()
            .filter(|surface| surface.alive())
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
