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
//! scoot's. Smithay keeps a `LockStatus` (`Unlocked` / `Locked(lock)` /
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
//! - **Rendering** (`render/elements.rs`'s `gather_elements`): the element list is built from
//!   this module alone -- an opaque full-output backdrop, the mapped lock
//!   surfaces, and the popups parented to those surfaces (see "Popups over
//!   the lock screen") -- so a window, a bar, a wallpaper or a focus ring is
//!   never gathered at all. A screenshot over IPC reads that same
//!   framebuffer, so it shows what the lock screen shows and nothing behind
//!   it.
//! - **Frame callbacks** (`headless.rs::render`): lock surfaces and their
//!   popups get them, so ordinary clients stop drawing while the session is
//!   locked -- the protocol's "the compositor must stop rendering ...
//!   normal clients".
//! - **Keyboard focus** (`shell.rs::refresh_keyboard_focus`): a lock surface,
//!   or nobody. Never a window, never a layer surface. With more than one
//!   output, the pointer's output picks the surface (see
//!   [`SessionLock::keyboard_focus`).
//! - **Pointer focus** (`state.rs::surface_under`, and the explicit
//!   [`State::refresh_pointer_focus`] at every lock transition): the hit test
//!   only ever sees lock surfaces, each against the output it was admitted
//!   for. The refresh matters as much as the hit
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
//! The honest consequence, which `docs/protocols.md` states too: while a lock
//! is abandoned, any client that can reach this compositor's wayland socket
//! can take it over and unlock. That is the same-uid trust boundary scoot's
//! IPC socket already has, and it is the price of having a recovery path at
//! all.
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
//! ## Every lock transition does the same four things
//!
//! Re-deriving both focuses is only two of them. The third is dropping a
//! pointer grab, and the fourth is deactivating a held pointer constraint
//! -- and either is the one a call site can silently forget: a grab
//! outlives focus changes *by design*, so a transition that re-derived focus
//! and stopped there would leave the grabbing client -- a drag-and-drop
//! started before the transition -- receiving exactly the pointer events the
//! focus change was meant to take away from it; and a held pointer lock
//! freezes the focus refresh itself (a zero-delta move resolves to holding),
//! so a transition that forgot it would leave pointer focus, deltas, buttons
//! and axis on the locking client across the lock. [`State::lock_transition`] is
//! the four together, and every transition calls it rather than repeating
//! them: [`SessionLockHandler::lock`], [`State::refresh_lock_state`],
//! [`State::lock_post_frame`]'s caller in the render loop, and both
//! destruction hooks (`handlers.rs` for a destroyed `wl_surface`,
//! `dispatch.rs` for a destroyed lock-surface *role object* --
//! [`State::lock_surface_destroyed`]). [`SessionLockHandler::unlock`] is the
//! single deliberate exception, documented there.
//!
//! A keyboard grab exists today -- an `xdg_popup.grab` (see `popup.rs`) --
//! and [`State::drop_input_grabs`] is where it, and any future non-pointer
//! grab, has to be dropped too: `PopupKeyboardGrab` ignores `set_focus`
//! while it is live, so leaving one installed across a lock would route the
//! user's password into whatever had a menu open, the keystroke twin of the
//! pointer leak above.
//!
//! ## Popups over the lock screen
//!
//! A lock surface can hold a text field -- that is what a password box is --
//! so an input method can be active against it, and its candidate window is
//! a popup parented to the lock surface. The locked render path therefore
//! gathers [`PopupManager::popups_for_surface`] for every current lock
//! surface, and sends those popups frame callbacks alongside the lock
//! surfaces' own. Without both halves the candidate window is tracked and
//! positioned but never drawn, and an animated one stalls for want of frame
//! callbacks: someone whose passphrase needs an IME could not see what they
//! are composing.
//!
//! This is the one deliberate exception to "never gathered", and it stays an
//! exception rather than a hole because of *who can parent a popup to a lock
//! surface*:
//!
//! - An `xdg_popup` names its parent by object id, and object ids live in
//!   per-connection namespaces: a client can only ever name its own
//!   surfaces. A popup whose root is a lock surface is therefore the lock
//!   client's own, through the whole parent chain -- and the lock client
//!   already draws fullscreen and receives the password, so its own menu
//!   grants it nothing it does not have.
//! - An input-method popup's parent is assigned by the compositor, never by
//!   client naming: Smithay parents it to the focused text field on creation
//!   and re-parents it on activation. While locked, keyboard (and with it
//!   text-input) focus is a current lock surface or nobody, so an IME popup
//!   in a lock surface's tree is there because the compositor put it there
//!   in service of the focused password field -- never because a background
//!   client asked for it. A background window's own IME popup stays parented
//!   to that window (and is dismissed outright when focus leaves it), which
//!   the locked path never gathers.
//!
//! The remaining trust is explicit and narrow: the IME client is trusted
//! with pixels over the lock screen *for the focused field's candidate
//! window*, because composition already routes every composed keystroke
//! through it -- the candidate window shows it nothing it does not already
//! know. Nothing else of any other client is gathered: no windows, no layer
//! surfaces, no xdg popups from background clients. Locking still dismisses
//! an open xdg grab and refuses new ones (see `popup.rs`), so a menu left
//! open at lock time can neither draw nor receive the password.
//!
//! The gather loop asks no focus question of its own, and needs none: the
//! two parenting constraints above hold regardless of how many lock surfaces
//! exist. One live surface per output is enforced, not assumed:
//! [`SessionLockHandler::new_surface`] refuses a second live surface for an
//! already-covered output with `duplicate_output`, so each output's frame
//! holds at most its own current surface -- and an xdg popup on it can only
//! be the lock client's own, while IME popups stay pinned to the focused
//! field by the compositor-assigned parenting above.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use scoot_core::OutputId;

use smithay::backend::input::InputTime;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::{Color32F, ImportAll, Renderer, Texture};
use smithay::desktop::utils::{
    OutputPresentationFeedback, send_frames_surface_tree, take_presentation_feedback_surface_tree,
    under_from_surface_tree,
};
use smithay::desktop::{PopupManager, WindowSurfaceType};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::{
    Error as LockError, ExtSessionLockV1,
};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER, Scale};
use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, with_states};
use smithay::wayland::session_lock::{
    LockSurface, LockSurfaceConfigure, LockSurfaceData, SessionLockHandler,
    SessionLockManagerState, SessionLocker,
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

/// How long a lock waits for the vblank confirming its blanked frame before
/// giving up and confirming anyway.
///
/// Only `--tty` ever waits (see [`SessionLock::await_vblank`]): headless and
/// nested have no scanout, so the framebuffer a screenshot reads *is* the
/// frame and there is nothing further to wait for. Under `--tty` the ordinary
/// case confirms within a vblank or two of the present -- one 16.7ms period
/// at 60Hz -- so a full second is ~60 periods of grace for a loaded system
/// while still far short of any locker's give-up timescale (this suite's own
/// `Lock` step waits five). The failure mode past the bound is the old
/// one-vblank gap, bounded and logged, rather than a locker hung forever --
/// which is strictly worse, since a locker waiting for `locked` may be
/// waiting to prompt for the password at all.
pub(super) const LOCK_VBLANK_TIMEOUT: Duration = Duration::from_secs(1);

/// The configure a lock surface last acked while its role was alive, plus
/// whether that role has since been destroyed.
///
/// Smithay's role-destruction handler resets the role attributes -- including
/// `last_acked` -- so after a destroy the surface on the wire is
/// indistinguishable from one that never acked anything. This is the record
/// that tells them apart: an entry means "this surface acked this configure",
/// and `role_destroyed` means "its role object has since gone away", which
/// [`State::prepare_post_destroy_lock_commit`] sets the first time it sees
/// the reset. See that method for why both halves are needed.
struct AckedLockSurface {
    configure: LockSurfaceConfigure,
    role_destroyed: bool,
}

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
    /// The outputs whose blanked frame has been recorded for the pending
    /// lock. `locked` goes out only once *every* output is in here (see
    /// [`SessionLock::note_blanked`]) -- confirming on the first output's
    /// frame would expose a live desktop on the screens that haven't blanked
    /// yet, which is the bug the vblank-confirmation work exists to prevent.
    ///
    /// Empty unless [`SessionLock::pending`] is `Some`: cleared at every site
    /// that writes `pending` ([`SessionLockHandler::lock`]'s sweep and fresh
    /// install, [`SessionLockHandler::unlock`], [`State::confirm_lock`]).
    /// A `Vec`, not a set: a session has a handful of outputs, and this only
    /// ever grows on a frame drawn while a lock awaits confirmation -- a cold
    /// path, never a hot one.
    confirmed: Vec<OutputId>,
    /// The in-flight flip carrying each output's blanked frame, where
    /// confirmation is waiting on a vblank for it rather than going out with
    /// the render -- one entry per output with a flip out, keyed by the
    /// output's core id.
    ///
    /// Only `--tty` ever writes this (see [`SessionLock::await_vblank`]): the
    /// other backends have no scanout, so the rendered frame *is* the shown
    /// one. The value is that output's head's flip sequence number for the
    /// flip `present` issued for the blanked frame -- not "a flip is in
    /// flight", which is the presenter's own bookkeeping and a different
    /// question. Keyed per output because each `--tty` head numbers its own
    /// flips from zero: a bare number would let one screen's vblank confirm
    /// another screen's blank. Matching the *completed* flip's output and
    /// number (see [`SessionLock::confirm_on_vblank`]) is what keeps a vblank
    /// for the *previous* frame -- the ordinary case when the lock raced a
    /// flip already in flight, since `present` skips then -- and a stale
    /// vblank for a flip the scanout bookkeeping has since discarded (VT
    /// switch, hotplug modeset) from confirming a lock whose pixels never
    /// scanned out.
    ///
    /// A matched output moves into [`SessionLock::confirmed`], and `locked`
    /// goes out once every output is there -- the same completion rule the
    /// render-confirmed backends use.
    ///
    /// **Invariant: non-empty only while [`SessionLock::pending`] is
    /// `Some`.** Installed with it and cleared with it, at every site that
    /// writes `pending` -- [`SessionLockHandler::lock`] (fresh wait or
    /// sweep), [`SessionLockHandler::unlock`] (cancel), [`State::confirm_lock`]'s
    /// callers (confirm) -- so a wait can never outlive the lock it was
    /// recorded for and confirm a later one. A `Vec`: a handful of outputs,
    /// written only on frames drawn while a lock awaits confirmation.
    blank_flips: Vec<(OutputId, u64)>,
    /// The outputs that have *rendered* a blanked frame for the pending lock
    /// under `--tty` -- every output [`SessionLock::await_vblank`] was
    /// called for, whether or not a flip carrying it was issued. What the
    /// fallback deadline may record in place of a vblank that never came
    /// ([`State::note_blank_timeout`]): the timeout stands in for a missing
    /// *completion*, never for a frame that was never drawn -- an output
    /// whose render failed still shows the pre-lock desktop, and recording
    /// it would send `locked` over it.
    ///
    /// Same lifetime as [`SessionLock::blank_flips`]: cleared at every site
    /// that writes `pending` (through [`SessionLock::cancel_blank_wait`]),
    /// and an output that goes away is dropped from it
    /// ([`SessionLock::forget_output`]). A `Vec` for the same reason.
    drawn: Vec<OutputId>,
    /// When the wait above stops waiting: [`LOCK_VBLANK_TIMEOUT`] after the
    /// blanked frame rendered. Armed together with the wait (and when a
    /// blanked frame rendered under `--tty` without any flip issued at all),
    /// so a vblank that can never arrive -- switched away, a discarded flip,
    /// completions a driver never delivers -- confirms anyway instead of
    /// hanging the locker. `Some` exactly while a `--tty` frame is
    /// unconfirmed, whether or not `blank_flips` names a flip.
    blank_deadline: Option<Instant>,
    /// Every lock surface this compositor has been handed and not yet dropped,
    /// in creation order. At most one *live* surface per output per lock (see
    /// [`SessionLock::surface_outputs`]): a second `get_lock_surface` for an
    /// already-covered output is refused in
    /// [`SessionLockHandler::new_surface`]. The keyboard goes to the surface
    /// on the pointer's output (see [`SessionLock::keyboard_focus`]).
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
    /// The physical output each lock surface was admitted for, keyed by its
    /// `wl_surface`'s object id -- the key [`SessionLockHandler::new_surface`]
    /// refuses a second live surface on.
    ///
    /// Keyed on the resolved [`Output`], not the `WlOutput` resource the
    /// client named: one global bound twice is two resources for one output,
    /// and Smithay's own `locked_outputs` guard compares resource identity,
    /// so it admits what this map refuses. [`Output`]'s equality is the
    /// physical output (`Arc::ptr_eq` in `0ff0098/src/output.rs`, the rev
    /// this project pins), which is exactly the granularity the protocol's
    /// `duplicate_output` error names.
    ///
    /// Live-only, deliberately: entries are consulted solely through
    /// [`SessionLock::current`], so a surface that stopped counting -- role
    /// destroyed, client gone, lock taken over -- frees its output for a
    /// rebuild without any removal here. Stronger than a lifecycle claim:
    /// [`SessionLock::surface_for_output`] answers with a live
    /// `&LockSurface`, so a sticky refusal (an entry with no live surface
    /// behind it) is unrepresentable -- the removal in
    /// [`State::forget_lock_surface`] is bounded-hygiene, not load-bearing.
    /// Like [`SessionLock::acked`] this outlives [`SessionLock::surfaces`]
    /// entries rather than shadowing their lifecycle, and for the same
    /// reason: the writers are the `insert` in
    /// [`SessionLockHandler::new_surface`], the `clear` in
    /// [`SessionLockHandler::lock`] and [`SessionLockHandler::unlock`], and
    /// the `remove` in [`State::forget_lock_surface`] -- the same three
    /// sites that write `surfaces`, minus the `retain` passes that need no
    /// map change because a stale entry is never consulted.
    surface_outputs: HashMap<ObjectId, Output>,
    /// The last configure every lock surface acked while its role was alive,
    /// keyed by its `wl_surface`'s object id.
    ///
    /// This outlives [`SessionLock::surfaces`] on purpose: the real unlock
    /// teardown (`unlock_and_destroy`, which clears `surfaces`, *then* the
    /// role destroy, *then* the trailing null commit) arrives after the
    /// surface has been forgotten, and the trailing commit still needs the
    /// ack it dropped. Entries are pruned when their `wl_surface` dies
    /// ([`State::forget_lock_surface`]), so this is bounded by live surfaces.
    /// The complete set of writers: the `insert` in
    /// [`SessionLockHandler::ack_configure`], the flag write in
    /// [`State::prepare_post_destroy_lock_commit`], and the `remove` in
    /// [`State::forget_lock_surface`].
    acked: HashMap<ObjectId, AckedLockSurface>,
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
            // No client filter: this compositor has no privilege primitive
            // to distinguish a locker from any other client (peer creds
            // are spoofable and same-uid anyway; security-context marks
            // sandboxes, not lockers), so restricting the global by
            // client would only be theatre. See `docs/protocols.md` and
            // `docs/backlog/resolved/session-lock-global-restriction-done.md`.
            manager: SessionLockManagerState::new::<State, _>(display, |_| true),
            owner: None,
            pending: None,
            confirmed: Vec::new(),
            blank_flips: Vec::new(),
            drawn: Vec::new(),
            blank_deadline: None,
            surfaces: Vec::new(),
            surface_outputs: HashMap::new(),
            acked: HashMap::new(),
            backdrop: SolidColorBuffer::default(),
        }
    }

    /// Whether the session is locked: nothing but lock surfaces may be drawn,
    /// and nothing but lock surfaces may receive input.
    pub(super) fn is_locked(&self) -> bool {
        self.owner.is_some()
    }

    /// Whether a lock has been accepted but the blanked frame confirming it
    /// has not been drawn yet -- the one window in which
    /// [`SessionLock::is_locked`] already answers `true` while the framebuffer
    /// still holds the unlocked desktop.
    ///
    /// Never true while unlocked: [`SessionLock::pending`]'s invariant is that
    /// while it is `Some` it names the same lock [`SessionLock::owner`] does,
    /// and both are written together.
    ///
    /// Read by `screencopy.rs`, which must not hand a client the desktop
    /// pixels a locked session is about to paint over. Nothing else needs it:
    /// the render path decides what to draw from `is_locked` alone, and
    /// drawing the lock screen is precisely what clears this.
    pub(super) fn awaiting_blank(&self) -> bool {
        self.pending.is_some()
    }

    /// Records that `output` presented a locked frame for the pending lock,
    /// reporting whether the wait is now complete -- every output recorded --
    /// so the caller can send `locked`.
    ///
    /// `id` is the output's core id, `expected` how many outputs must record
    /// before the lock confirms (the render loop's output count, read once up
    /// front -- outputs change only at startup and in the `--tty` hotplug
    /// handler, never mid-frame, so it cannot go stale).
    ///
    /// An output records when it drew a locked frame, with two qualifications:
    ///
    /// - An output with no current surface records on its backdrop frame:
    ///   the solid-colour fallback *is* that output's locked frame, so a
    ///   locker covering only some outputs still gets `locked`. With zero
    ///   surfaces anywhere that is every output, which is the long-pinned
    ///   zero-surface confirmation, unchanged.
    /// - An output whose admitted surface is mapped records; one whose
    ///   surface is admitted but undrawn -- acked, no buffer yet -- does
    ///   not, while its role is still alive. The locker is mid-startup on
    ///   that screen and the backdrop is a placeholder, not its blank, so
    ///   confirming would hand it the guarantee while one screen still shows
    ///   nothing of its. A surface whose role is gone never blocks, mapped
    ///   or not: its screen shows the fallback finally (see
    ///   [`State::lock_surface_destroyed`]), and waiting for a teardown to
    ///   draw would hang the locker.
    ///
    /// The undrawn-surface half applies only with more than one output. With
    /// exactly one, the first blanked frame confirms whatever the surface
    /// state -- the single-output pin `per_output.rs` records -- so the
    /// whole existing single-output suite reads this function as "record and
    /// complete".
    ///
    /// No allocation past the first record per output per lock; runs only on
    /// frames drawn while a lock awaits confirmation.
    pub(super) fn note_blanked(&mut self, id: OutputId, output: &Output, expected: usize) -> bool {
        if self.pending.is_none() {
            return false;
        }
        if expected > 1 && self.output_blocks_confirm(output) {
            return false;
        }
        if !self.confirmed.contains(&id) {
            self.confirmed.push(id);
        }
        self.confirmed.len() >= expected
    }

    /// Whether `output`'s screen is not its locked blank yet: a current
    /// surface admitted for it that has drawn nothing and may still do so.
    fn output_blocks_confirm(&self, output: &Output) -> bool {
        self.current()
            .filter(|surface| self.is_admitted_for(surface, output))
            .any(|surface| self.surface_blocks_confirm(surface))
    }

    /// Whether an admitted surface's screen is still a placeholder: mapped
    /// surfaces never block (their pixels are on screen), and neither do
    /// torn-down ones (their backdrop is final) -- only a live role that has
    /// not drawn yet.
    fn surface_blocks_confirm(&self, surface: &LockSurface) -> bool {
        if is_mapped(surface) {
            return false;
        }
        if self
            .acked
            .get(&surface.wl_surface().id())
            .is_some_and(|acked| acked.role_destroyed)
        {
            // Destroyed and committed since (see
            // `prepare_post_destroy_lock_commit`): the teardown the naive
            // "unmapped blocks" rule would wait on forever.
            return false;
        }
        // A role destroyed without any commit since leaves no flag behind,
        // but Smithay's own `destroyed` hook reset the role attributes
        // synchronously -- last ack, pending configures and server state all
        // gone -- which a live role never reads as: a configure the client
        // has not acked yet is still sitting in `pending_configures`, and an
        // acked one in `last_acked`. All three empty can only mean the reset
        // ran, i.e. the role is gone and the backdrop is final.
        !with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<LockSurfaceData>()
                .is_some_and(|data| {
                    let attributes = data.lock().expect("lock surface attributes");
                    attributes.last_acked.is_none()
                        && attributes.pending_configures.is_empty()
                        && attributes.server_pending.is_none()
                })
        })
    }

    /// Whether `surface` was admitted for `output` -- the render, callback
    /// and feedback scoping predicate, and the per-output half of the
    /// duplicate check [`SessionLock::surface_for_output`] answers.
    fn is_admitted_for(&self, surface: &LockSurface, output: &Output) -> bool {
        self.surface_outputs
            .get(&surface.wl_surface().id())
            .is_some_and(|admitted| admitted == output)
    }

    /// A blanked frame was rendered for output `id` under the pending lock
    /// but, under `--tty`, that output's confirmation must wait for scanout
    /// rather than going out now.
    ///
    /// `issued` is the flip carrying it -- [`Tty::present`](super::tty::Tty)'s
    /// return for that output -- or `None` when the frame never reached the
    /// presenter at all: the session is inactive, the size disagrees, a
    /// previous flip is still in flight, or nothing was damaged. A tracked
    /// flip records its number against the output in
    /// [`SessionLock::blank_flips`]; a skipped frame leaves whatever is there
    /// (the still-in-flight flip already carries the blank pixels, or nothing
    /// does yet and the re-render the skip armed will record its own). Either
    /// way the fallback deadline is armed if it is not already, so a vblank
    /// that can never arrive confirms anyway after [`LOCK_VBLANK_TIMEOUT`]
    /// instead of hanging the locker.
    ///
    /// Records that `id` drew its blank (see [`SessionLock::drawn`]), which
    /// is what lets the fallback confirm it later without a vblank.
    ///
    /// Returns whether the caller should arm the one-shot timer watching
    /// the deadline: yes exactly when this call armed it. **The deadline is
    /// armed once per wait and never extended**, whichever output presents
    /// next and however often. Two livelocks are what that closes, both
    /// found in review of the multi-output change: a screen already
    /// confirmed that keeps flipping (cursor motion, an animated locker)
    /// used to push the shared bound out on every frame, so a second screen
    /// whose completion never arrived held `locked` back indefinitely (and
    /// every such frame inserted another timer, an allocation on the render
    /// path); and a scanout tier that keeps queueing frames behind a lost
    /// completion would do the same on its own. With one bound the fallback
    /// fires [`LOCK_VBLANK_TIMEOUT`] after the first blanked frame drew,
    /// whatever happens after. The cost is that a blank re-presented after a
    /// discard (VT switch, hotplug modeset) may confirm by the fallback up to
    /// one bound after the *first* one drew rather than on its own vblank --
    /// late, never early, for a frame that has drawn. A wait the fallback
    /// took without completing the set (see [`State::note_blank_timeout`])
    /// leaves no deadline, so the next blanked frame arms a fresh one.
    ///
    /// A flip on an output already recorded blanked is not waited on: that
    /// screen's blank is already on scanout.
    ///
    /// Only `--tty` calls this -- headless and nested confirm on render, as
    /// before. No allocation past the first record per output per lock, on a
    /// path that runs only while a lock awaits confirmation.
    pub(super) fn await_vblank(&mut self, id: OutputId, issued: Option<u64>, now: Instant) -> bool {
        if !self.drawn.contains(&id) {
            self.drawn.push(id);
        }
        if let Some(seq) = issued
            && !self.confirmed.contains(&id)
        {
            match self
                .blank_flips
                .iter_mut()
                .find(|(output, _)| *output == id)
            {
                Some(entry) => entry.1 = seq,
                None => self.blank_flips.push((id, seq)),
            }
        }
        if self.blank_deadline.is_some() {
            return false;
        }
        self.blank_deadline = Some(now + LOCK_VBLANK_TIMEOUT);
        true
    }

    /// A flip completed under `--tty` on output `id`: whether the pending
    /// lock is now confirmed.
    ///
    /// `completed` is the finished flip's sequence number, or `None` when the
    /// completion is untrackable -- a stale vblank for a flip the scanout
    /// bookkeeping discarded, or a `DrmEvent::Error` -- in which case nothing
    /// is recorded and the fallback deadline owns the wait. A completion that
    /// matches the flip recorded for *this* output records the output as
    /// blanked; `locked` is owed once `expected` outputs (the output count)
    /// are recorded, exactly [`SessionLock::note_blanked`]'s completion rule.
    ///
    /// `blocked` is whether the output's screen is still a placeholder (see
    /// [`SessionLock::output_blocks_confirm`], which the caller evaluates
    /// only with more than one output): a matched flip then carried the
    /// backdrop, not the locker's blank, so its entry is spent without
    /// recording, and the frame that draws the surface records its own.
    fn confirm_on_vblank(
        &mut self,
        id: OutputId,
        completed: Option<u64>,
        blocked: bool,
        expected: usize,
    ) -> bool {
        let Some(got) = completed else {
            return false;
        };
        let Some(position) = self
            .blank_flips
            .iter()
            .position(|&(output, seq)| output == id && seq == got)
        else {
            return false;
        };
        self.blank_flips.swap_remove(position);
        if blocked {
            return false;
        }
        if !self.confirmed.contains(&id) {
            self.confirmed.push(id);
        }
        if self.confirmed.len() >= expected {
            self.blank_flips.clear();
            self.blank_deadline = None;
            true
        } else {
            false
        }
    }

    /// Whether the fallback bound has passed. Takes the wait exactly once --
    /// every output's recorded flip with it; the caller records whatever is
    /// not a placeholder and sends `locked` if that completes the set, and
    /// logs that it did so without a vblank.
    fn poll_blank_timeout(&mut self, now: Instant) -> bool {
        if self.blank_deadline.is_some_and(|deadline| now >= deadline) {
            self.blank_flips.clear();
            self.blank_deadline = None;
            true
        } else {
            false
        }
    }

    /// Forgets output `id` from the pending confirmation: its recorded blank
    /// and its outstanding flip. Called when a `--tty` hotplug removes the
    /// output. Without it, an output recorded blanked before it went away
    /// would keep counting towards `expected` -- which has just shrunk by one
    /// -- and could complete the set for a remaining output that never
    /// blanked. Answers whether the set is now complete (`expected` being the
    /// output count after the removal), so the caller can send `locked` for
    /// a wait that only the removed output was holding up.
    pub(super) fn forget_output(&mut self, id: OutputId, expected: usize) -> bool {
        self.confirmed.retain(|&output| output != id);
        self.blank_flips.retain(|&(output, _)| output != id);
        self.drawn.retain(|&output| output != id);
        self.pending.is_some() && expected > 0 && self.confirmed.len() >= expected
    }

    /// Forgets a wait that can no longer confirm anything: the lock it was
    /// recorded for is gone (unlock) or superseded (a fresh `lock`
    /// installing its own `pending`, or the dead-`pending` sweep clearing the
    /// way for one). Without this a late vblank for the old flip could
    /// confirm a lock whose blanked frame was never presented.
    fn cancel_blank_wait(&mut self) {
        self.blank_flips.clear();
        self.drawn.clear();
        self.blank_deadline = None;
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

    /// Every current lock surface's `wl_surface`, cloned, for the config
    /// reload's scale re-send (see `output_scale.rs`'s
    /// `resend_output_scale`): the one reader outside this module's own
    /// render/focus paths, asking the same "which surfaces count" question
    /// through [`SessionLock::current`] rather than the weaker ones. Empty
    /// while unlocked. Allocates one small `Vec`, on the cold reload path
    /// only.
    pub(super) fn live_surfaces(&self) -> Vec<WlSurface> {
        self.current()
            .map(|surface| surface.wl_surface().clone())
            .collect()
    }

    /// The current surface admitted for `output`, if any -- the duplicate
    /// check [`SessionLockHandler::new_surface`] refuses on.
    ///
    /// Asked through [`SessionLock::current`], so only live surfaces count:
    /// destroying one frees its output for a rebuild. The output half comes
    /// from [`SessionLock::surface_outputs`]; a current surface with no entry
    /// there simply never matches, which is the safe direction for a lookup
    /// that only ever refuses.
    fn surface_for_output(&self, output: &Output) -> Option<&LockSurface> {
        self.current().find(|surface| {
            self.surface_outputs
                .get(&surface.wl_surface().id())
                .is_some_and(|admitted| admitted == output)
        })
    }

    /// The surface the keyboard goes to while locked: the current surface on
    /// the pointer's output, or the first current surface when the pointer is
    /// over no output (or its output has no surface).
    ///
    /// The pointer picks the output -- the milestone-19 focus decision, the
    /// same rule `layer_keyboard_focus` applies -- and the existing
    /// derivation applies within it. With one output the preferred surface
    /// *is* the first, so the single-output answer is unchanged.
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
    fn keyboard_focus(&self, preferred: Option<&Output>) -> Option<WlSurface> {
        preferred
            .and_then(|output| self.surface_for_output(output))
            .or_else(|| self.current().next())
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

    /// The render elements of every *mapped* lock surface admitted for
    /// `output`, front-most first, each preceded by the popups parented to
    /// it.
    ///
    /// Scoped to the output being drawn: each output's framebuffer is that
    /// output's size, in that output's local coordinates (see
    /// `render/elements.rs`'s `window_elements`, which translates the
    /// unlocked path the same way), so a surface drawn at another output's global
    /// origin would land off-target -- and a surface drawn onto another
    /// output's screen would put one screen's lock pixels where they do not
    /// belong. At most one live surface per output (see
    /// [`SessionLockHandler::new_surface`]), so this is one surface's
    /// elements plus its popups, or nothing but the backdrop.
    ///
    /// Mapped-ness here is `last_acked`, which Smithay's own pre-commit hook
    /// maintains as "has a buffer" (`session_lock/surface.rs`), the same test
    /// `layer_shell.rs` uses for a layer surface. An unmapped one produces no
    /// elements and the backdrop shows through, which is exactly what the
    /// protocol asks for while a lock client is still starting up.
    ///
    /// The popups are the input method's candidate window over the password
    /// field, or the lock client's own popups -- and nothing else can be
    /// (see this module's "Popups over the lock screen" for why each
    /// parentage there is safe and what can never appear). They come before
    /// their parent's own elements in the list, which is what puts them in
    /// front of it: the damage tracker draws the list back-to-front, so the
    /// first entry ends up on top (the same order `Window`'s own render
    /// elements use for its popups).
    ///
    /// Costs what every window already pays per frame on the unlocked path:
    /// one [`PopupManager::popups_for_surface`] walk per lock surface (one
    /// surface per output), plus Smithay's own per-popup element `Vec`. No
    /// new allocation shape -- with no popup the walk finds an empty tree
    /// and appends nothing, which is the byte-identical no-IME behaviour the
    /// blanking tests pin.
    fn surface_elements<R>(
        &self,
        renderer: &mut R,
        output: &Output,
        scale: f64,
    ) -> Vec<WaylandSurfaceRenderElement<R>>
    where
        R: Renderer + ImportAll,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        // Framebuffer-local, not global: the target is this output's size,
        // so the global origin another output sits at would push its surface
        // out of its own frame (see `lock_elements`).
        let origin = Point::<i32, Physical>::default();
        let mut elements = Vec::new();
        for surface in self
            .current()
            .filter(|surface| self.is_admitted_for(surface, output))
        {
            if !is_mapped(surface) {
                continue;
            }
            for (popup, popup_offset) in PopupManager::popups_for_surface(surface.wl_surface()) {
                // The layer-surface shape (`space/wayland/layer.rs`), not
                // the window one: a lock surface is drawn at `origin`
                // directly, so there is no parent geometry to add back --
                // and for an IME popup the two coincide anyway, because
                // `parent_geometry` answers the default rectangle for a
                // lock surface (see `input_method.rs`).
                let offset = (popup_offset - popup.geometry().loc)
                    .to_f64()
                    .to_physical(Scale::from(scale))
                    .to_i32_round();
                elements.extend(render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    origin + offset,
                    scale,
                    1.0,
                    Kind::Unspecified,
                ));
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

    /// Sends this frame's callbacks to every current lock surface admitted
    /// for `output`, and the popups parented to it.
    ///
    /// Sent to all of them rather than only the ones that produced an
    /// element, matching `render()`'s window, cursor and layer-surface loops
    /// and for the same reason: a client may legitimately ask for a callback
    /// before its first attach, and withholding it would stall the very frame
    /// that unsticks it -- here, the first frame of the lock screen itself,
    /// or of the candidate window. The popup set is the same trees
    /// [`SessionLock::surface_elements`] gathers, so a popup with elements
    /// is never stalled -- except popups of not-yet-mapped lock surfaces,
    /// which are woken but not drawn: `surface_elements` skips a lock
    /// surface with no buffer yet, while this sends to every current
    /// surface, so a popup that commits before its lock surface's first
    /// buffer gets callbacks with no elements yet. Harmless -- the same
    /// client's own unseen buffer, redrawn the same way `window.send_frame`
    /// (see `headless.rs::render`) wakes every window unconditionally.
    fn send_frames(&self, output: &Output, time: Duration) {
        for surface in self
            .current()
            .filter(|surface| self.is_admitted_for(surface, output))
        {
            send_frames_surface_tree(
                surface.wl_surface(),
                output,
                time,
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
            for (popup, _) in PopupManager::popups_for_surface(surface.wl_surface()) {
                send_frames_surface_tree(
                    popup.wl_surface(),
                    output,
                    time,
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
        }
    }

    /// Takes every current lock surface admitted for `output` (and its
    /// popups') committed presentation feedback into `output_feedback` -- the
    /// take half of what [`SessionLock::send_frames`] is the frame-callback
    /// half of, over the same surface set: while locked these are the only
    /// client surfaces any frame shows, so they are the only ones any locked
    /// frame may stamp. `flags` is the presenting frame's flags, applied to
    /// every surface alike (there is no per-surface zero-copy path behind a
    /// pixman copy).
    pub(super) fn take_presentation_feedback(
        &self,
        output: &Output,
        output_feedback: &mut OutputPresentationFeedback,
        flags: wp_presentation_feedback::Kind,
    ) {
        for surface in self
            .current()
            .filter(|surface| self.is_admitted_for(surface, output))
        {
            take_presentation_feedback_surface_tree(
                surface.wl_surface(),
                output_feedback,
                |_, _| Some(output.clone()),
                |_, _| flags,
            );
            for (popup, _) in PopupManager::popups_for_surface(surface.wl_surface()) {
                take_presentation_feedback_surface_tree(
                    popup.wl_surface(),
                    output_feedback,
                    |_, _| Some(output.clone()),
                    |_, _| flags,
                );
            }
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

    /// Configures every current lock surface admitted for `output` to `size`.
    ///
    /// Called when that output's mode changes: a lock surface's size is an
    /// exact requirement (committing a buffer of any other size is a protocol
    /// error), so a resized output has to reconfigure its own surfaces or the
    /// next commit kills the lock client. Scoped to the output that moved --
    /// reconfiguring every surface to one output's size would kill the lock
    /// clients of all the others the moment outputs differ. A no-op when the
    /// session isn't locked (there are none).
    fn configure_output(&self, output: &Output, size: (i32, i32)) {
        for surface in self
            .current()
            .filter(|surface| self.is_admitted_for(surface, output))
        {
            configure(surface, size);
        }
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
            // The wait belonged to the dead lock: a late vblank for its flip
            // must not confirm whatever lock comes next. Its recorded blanks
            // go with it: they are evidence about the dead lock's frames, not
            // about the replacement's.
            self.session_lock.cancel_blank_wait();
            self.session_lock.confirmed.clear();
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
        // The admitted-output map goes with the surfaces: a takeover starts
        // with no output covered.
        self.session_lock.surfaces.clear();
        self.session_lock.surface_outputs.clear();
        if already_blanked {
            // The outputs are blank and stay blank across this handover, so
            // the protocol's reason for waiting is satisfied by construction
            // and the new client can put its own surfaces up straight away.
            confirmation.lock();
        } else {
            // Any wait still recorded here belongs to the lock just replaced
            // (or to nothing, on the fresh-lock path) -- the new lock
            // records its own once its blanked frames present.
            self.session_lock.cancel_blank_wait();
            self.session_lock.confirmed.clear();
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
        // A cursor change like any other (see `State::cursor_changed`); the
        // transition below redraws the whole screen anyway.
        self.cursor_changed();
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
        // The wait belonged to the lock just ended: a late vblank for its
        // flip must not confirm anything afterwards -- and recorded blanks
        // for it must not complete a later lock's wait.
        self.session_lock.cancel_blank_wait();
        self.session_lock.confirmed.clear();
        self.session_lock.surfaces.clear();
        self.session_lock.surface_outputs.clear();
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
    ///
    /// At most one live surface per physical output: a second
    /// `get_lock_surface` for an output the lock already covers is refused
    /// with the protocol's own `duplicate_output` error, which kills the
    /// offending client. Smithay already refuses the same `wl_output`
    /// resource twice before this is ever called; what lands here is the
    /// shape its resource-identity guard admits, the same physical output
    /// named through a second bind of the global. The check is against live
    /// surfaces only (see [`SessionLock::surface_for_output`]), so replacing
    /// a destroyed surface is still admitted.
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
        // The output the client named. No fallback: with more than one
        // output a surface that names nothing resolvable has no size to be
        // configured to and no screen to be drawn on, so it is ignored rather
        // than shown somewhere the locker did not ask for. Reachable since
        // `--tty` hotplug: a locker naming the `wl_output` of a monitor that
        // was just unplugged (its resource outlives the output) -- and
        // written as a fallthrough rather than an `expect` either way,
        // because a panic on the commit path would take every client's
        // unsaved state with it.
        let Some(output) = Output::from_resource(&output) else {
            tracing::warn!("no output for a lock surface");
            return;
        };
        if self.session_lock.surface_for_output(&output).is_some() {
            tracing::warn!("refusing a second lock surface for an already-locked output");
            surface.ext_session_lock().post_error(
                LockError::DuplicateOutput,
                "Output already has a lock surface.",
            );
            return;
        }
        let size = logical_size(&output);
        configure(&surface, size);
        self.session_lock
            .surface_outputs
            .insert(surface.wl_surface().id(), output);
        self.session_lock.surfaces.push(surface);
        // The first surface takes the keyboard off nobody, and the pointer
        // has to enter it rather than wait for the user to move the mouse.
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
    }

    /// A lock surface acked a configure: record it.
    ///
    /// This is the "before" half of the post-destroy commit fix (see
    /// [`State::prepare_post_destroy_lock_commit`]): Smithay's
    /// role-destruction reset drops exactly this value, and only a copy kept
    /// outside the role attributes can tell a surface whose role was destroyed
    /// from one that never acked anything. Overwrites unconditionally -- a
    /// dead role object can never ack again, so any ack names a live role.
    fn ack_configure(&mut self, surface: WlSurface, configure: LockSurfaceConfigure) {
        self.session_lock.acked.insert(
            surface.id(),
            AckedLockSurface {
                configure,
                role_destroyed: false,
            },
        );
    }
}

impl State {
    /// Everything the compositor has to catch up when the set of lock
    /// surfaces that may be drawn and focused has just changed: a held
    /// pointer constraint deactivated, any input grab dropped, both focuses
    /// re-derived, the screen marked dirty.
    ///
    /// One function rather than the same four calls repeated, because the
    /// four are not independent and the *asymmetry* is what goes wrong: a
    /// transition that re-derived focus but left a grab installed would leave
    /// the grabbing client receiving pointer events the focus change was
    /// supposed to take away from it, and a transition that re-derived focus
    /// but left a held pointer lock active would never move focus at all --
    /// and both mistakes are invisible at the call site that forgot them.
    /// The callers are
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
        self.drop_input_grabs();
        // A floating window's drag was one of those grabs: the arrangement
        // it asked for, before focus is re-derived against it.
        self.settle_floating_grab();
        // Deactivate a held pointer lock or confinement before the refresh
        // below: a zero-delta refresh against either resolves to holding
        // focus in place, so without this the lock transition would leave
        // pointer focus -- and with it deltas, buttons and axis -- on the
        // locking client across the lock. See
        // `relative_pointer.rs::deactivate_pointer_constraint` for why only
        // the focus surface's constraint needs it.
        self.deactivate_pointer_constraint();
        self.refresh_keyboard_focus();
        self.refresh_pointer_focus();
        self.request_render();
    }

    /// Drops every input grab so none can outlive a lock transition.
    ///
    /// A grab deliberately outlives focus changes -- that is what a grab *is*
    /// -- so re-deriving focus is not enough on its own: the grabbing client
    /// goes on receiving every motion and button until the grab ends by
    /// itself. Three kinds can be active here, and all three matter:
    ///
    /// - **A popup grab** (`xdg_popup.grab`, see `popup.rs`), which holds
    ///   the *keyboard* as well as the pointer, and whose keyboard half
    ///   swallows the very `set_focus` the refresh below is about to make --
    ///   so without dropping it the lock client would never be told it has
    ///   the keyboard, and every keystroke of the user's password would go
    ///   to whatever had a menu open. Dropped first, so the pointer unset
    ///   underneath cannot restore keyboard focus to the popup's root on its
    ///   way out (`PopupPointerGrab::unset` does exactly that while the
    ///   keyboard is still grabbed).
    ///
    /// - **A drag-and-drop**, installed by `handlers.rs`'s
    ///   `WaylandDndGrabHandler` at a client's own request. It ends only when
    ///   the drag does, and while it lasts it routes pointer events through
    ///   `DnDGrab` rather than through focus at all.
    /// - **A floating window's move or resize** (`floating/grab.rs`), which
    ///   would otherwise keep dragging a window behind the lock screen.
    /// - **The implicit click grab**, which is not scoot's code but is
    ///   nonetheless installed on *every* button press: Smithay's
    ///   `DefaultGrab::button` calls `SeatHandler::click_grab` (scoot takes
    ///   the default `ClickGrab`) and sets it. It releases itself once every
    ///   button is up -- so this only ever finds one with a button still
    ///   held, which is exactly the case that must not survive a lock
    ///   transition: a press delivered to a surface before the transition
    ///   would otherwise keep steering the pointer afterwards.
    ///
    /// **If a touch grab is ever added to this compositor, it has to be
    /// dropped here too.** It does not exist today -- nothing calls
    /// `Seat::add_touch`, so Smithay's `TouchDownGrab` (the touch twin of
    /// the click grab above) is never reached. The keyboard half of that
    /// warning has since come true and is handled: popup grabs are the
    /// keyboard grab this function was told to expect.
    ///
    /// Costs one `Option` check and one mutex-guarded enum check on a
    /// transition that has already decided to re-derive focus and redraw;
    /// nothing on any per-event path.
    fn drop_input_grabs(&mut self) {
        self.dismiss_popup_grab();
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
    /// It costs nothing to leave: a later commit on it survives as a no-op
    /// now (see [`State::prepare_post_destroy_lock_commit`]) rather than a
    /// protocol error, and the keyboard it may still hold is the lock
    /// client's own -- the same client that owns the session -- which cannot
    /// put a replacement surface up for that output anyway, because Smithay's
    /// `locked_outputs` list never shrinks. It goes for real when that
    /// `wl_surface` is destroyed, when the client disconnects, or at the next
    /// takeover.
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

    /// Prepares a `wl_surface.commit` on a lock surface whose role object has
    /// already been destroyed, so Smithay's role validation does not kill the
    /// client for it.
    ///
    /// Called from `dispatch.rs`'s blanket request impl *before* the commit
    /// is delegated -- the only seam scoot owns ahead of Smithay's
    /// pre-commit hooks, which is where the kill happens. The commit itself
    /// is still delegated afterwards, so frame callbacks, buffer release and
    /// damage all flow exactly as they would have.
    ///
    /// Two facets, matching the two errors the trailing commit would trip:
    ///
    /// - **Bare commit.** Smithay's `destroyed` reset `last_acked` to `None`,
    ///   and the pre-commit hook's first check posts `CommitBeforeFirstAck`
    ///   without it. Restoring the acked configure recorded by
    ///   [`SessionLockHandler::ack_configure`] passes that check, and the
    ///   hook then finds no new buffer and nothing previously mapped, so it
    ///   commits nothing: a no-op.
    /// - **Null commit** (the real quickshell teardown: `destroy` then
    ///   `attach(nil)` then `commit`). The same restore is not enough on its
    ///   own -- a null attach on a surface the hook believes was mapped is
    ///   the *by-design* `NullBuffer` error. But the surface is not mapped:
    ///   the reset cleared the cached state too. Clearing the pending
    ///   `Removed` turns it into the bare commit above, which is what the
    ///   surface going unmapped means now that its role is gone.
    ///
    /// The carve-out applies *only* to destroyed-role surfaces, and the two
    /// halves of the gate are both load-bearing:
    ///
    /// - An entry in [`SessionLock::acked`] means this surface acked a
    ///   configure while its role was alive. A surface that never acked --
    ///   the by-design `CommitBeforeFirstAck` case -- has no entry and is
    ///   left alone to die loudly.
    /// - `last_acked` still `None` in the role attributes (or the
    ///   `role_destroyed` flag from a previous restore) means the reset ran.
    ///   A surface whose role is still alive keeps its `last_acked`, so its
    ///   commits -- including the by-design `NullBuffer` for a mapped lock
    ///   surface -- validate exactly as before.
    ///
    /// `reset` is the only writer that clears `last_acked` after an ack, so
    /// "acked plus currently `None`" can only mean the role was destroyed;
    /// the flag then keeps later commits covered after the first restore put
    /// a value back. A same-size buffer committed after the destroy maps the
    /// surface again -- the hook cannot tell the restored ack from a live
    /// one -- but only its own locker's pixels on its own lock screen (every
    /// read still goes through [`SessionLock::current`]), and a wrong-size
    /// one still dies with `DimensionsMismatch`.
    ///
    /// Costs one typemap probe per `wl_surface.commit` for surfaces that never
    /// had a lock role (the `get` misses, nothing is allocated), and a map
    /// lookup only for ones that did -- and neither at all while no lock
    /// surface has ever acked a configure (the `is_empty` check below), which
    /// is every commit of an unlocked session that never locked. No allocation
    /// on any path.
    pub(super) fn prepare_post_destroy_lock_commit(
        &mut self,
        id: ObjectId,
        dhandle: &DisplayHandle,
    ) {
        if self.session_lock.acked.is_empty() {
            return;
        }
        // The request's interface is `wl_surface`, so this id names the
        // committed surface itself.
        let Ok(surface) = WlSurface::from_id(dhandle, id) else {
            return;
        };
        let Some(acked) = self.session_lock.acked.get_mut(&surface.id()) else {
            return;
        };
        with_states(&surface, |states| {
            let Some(attributes) = states.data_map.get::<LockSurfaceData>() else {
                return;
            };
            let mut attributes = attributes.lock().expect("lock surface attributes");
            if attributes.last_acked.is_some() && !acked.role_destroyed {
                // The role is still alive: normal validation, untouched.
                return;
            }
            attributes.last_acked = Some(acked.configure);
            acked.role_destroyed = true;
            let mut cached = states.cached_state.get::<SurfaceAttributes>();
            if matches!(cached.pending().buffer, Some(BufferAssignment::Removed)) {
                cached.pending().buffer = None;
            }
        });
    }

    /// Re-derives pointer focus on a lock surface's commit, if it was one.
    ///
    /// Called from `CompositorHandler::commit` for every commit that is
    /// neither a window's nor a layer surface's. `new_surface` already
    /// re-derives, but it runs while the lock surface is still unmapped, so
    /// the hit test finds nothing; the commit that maps it is what makes it
    /// hit-testable, and without this the lock surface gets `wl_pointer.enter`
    /// only on the first mouse move -- with the click before that reaching
    /// nobody. Pointer focus only: the keyboard goes to a lock surface on
    /// liveness, not mapped-ness, so `new_surface`'s keyboard refresh already
    /// covered it, and a commit is not a lock transition, so no grab is
    /// dropped (a grab outlives focus changes by design).
    ///
    /// The recognition is deliberately cheap, because this sits on the
    /// commit path: while unlocked it is one `Option::is_some` and nothing
    /// else. While locked it adds one `with_states` typemap probe for a
    /// surface that never had a lock role (the `get` misses, nothing is
    /// allocated), and the refresh itself runs only for surfaces that did.
    /// No allocation on any path. Ordinary window commits never reach here
    /// at all (`id_of` claims them first), and layer-surface commits never
    /// reach the probe (`commit_layer_surface` claims them first).
    ///
    /// Precedence needs no special case: the refresh runs the same locked
    /// hit test every other derivation runs, which sees lock surfaces only --
    /// so an exclusive layer surface or a popup grab cannot steal it, and
    /// nothing behind the lock can receive it. An unmapped lock surface is
    /// skipped by that hit test exactly like everywhere else, so a commit
    /// that attaches no buffer takes no focus.
    pub(super) fn refresh_lock_pointer_focus(&mut self, surface: &WlSurface) {
        if !self.session_lock.is_locked() {
            return;
        }
        let is_lock_surface = with_states(surface, |states| {
            states.data_map.get::<LockSurfaceData>().is_some()
        });
        if !is_lock_surface {
            return;
        }
        self.refresh_pointer_focus();
    }

    /// Confirms a pending lock now that every output's blanked frame has been
    /// drawn.
    ///
    /// Called from `headless.rs::render` after a frame whose blanks complete
    /// the set (see [`SessionLock::note_blanked`]) -- but only where there is
    /// no scanout to wait for. `--headless`/`--nested` have none, and the
    /// framebuffer a screenshot reads *is* this frame, so this is exact
    /// there. Under `--tty` the render loop calls
    /// [`SessionLock::await_vblank`] instead (see it for which flip is
    /// tracked and what bounds the wait), and confirmation arrives through
    /// [`State::note_flip_completed`] (the flip's vblank) or
    /// [`State::note_blank_timeout`] (the fallback), never here.
    ///
    /// Costs one `Option` check on every frame that is not locking.
    ///
    /// Taking a wait also re-arms the frame ticker, and that arm lives here
    /// rather than at any of the three call sites on purpose: the tick that
    /// rendered the blank drops the timer -- a parked screencopy frame is
    /// none of `frame_tick`'s re-arm conditions -- so the tick that clears
    /// the wait has to be followed by one more, whichever path clears it
    /// (the render tail, the vblank, the fallback, or any future one calling
    /// this function), or nothing runs `service_captures` again and a
    /// capture parked across the wait sits undelivered (see `screencopy.rs`).
    /// A new wait-clearing path gets the re-arm by construction by calling
    /// this function; a call that finds no wait arms nothing. Inside the
    /// render tail the arm is a no-op branch -- `frame_tick` only runs while
    /// the timer is armed. The only caller outside a tick is an
    /// out-of-tick `render()` that confirms (today: only the IPC-screenshot
    /// path), which buys one extra tick that finds nothing and drops itself.
    pub(super) fn confirm_lock(&mut self) {
        if let Some(confirmation) = self.session_lock.pending.take() {
            tracing::debug!("session lock confirmed: a blanked frame has been drawn");
            // The recorded per-output blanks belonged to the wait just taken:
            // without this a later lock could start one output already
            // "confirmed" from frames it never drew -- and the same for the
            // `--tty` flips, drawn set and deadline, whichever path
            // confirmed (a render, a vblank or the fallback).
            self.session_lock.confirmed.clear();
            self.session_lock.cancel_blank_wait();
            // A no-op if the client died in the meantime: the generated event
            // sender discards the send error for a destroyed object. The
            // session stays locked either way -- `owner` is untouched here --
            // and reads as abandoned from the next frame on.
            confirmation.lock();
            self.ensure_ticking();
        }
    }

    /// Whether `output`'s screen is still a placeholder for the pending lock:
    /// an admitted lock surface there has not drawn yet (see
    /// [`SessionLock::output_blocks_confirm`]). The render tail's `--tty`
    /// branch asks this before recording a flip to wait on.
    pub(super) fn session_lock_output_blocks(&self, output: &Output) -> bool {
        self.session_lock.output_blocks_confirm(output)
    }

    /// A DRM flip completed under `--tty` on output `id`: confirm the pending
    /// lock if that output's blanked frame was aboard and it completes the
    /// set.
    ///
    /// The whole of the vblank-confirmation wiring in one place, so the DRM
    /// event handler and the tests call the same code: the handler passes
    /// the output whose CRTC completed and what its presenter reports for the
    /// finished flip (a sequence number, or `None` for a completion that
    /// names no flip), and a match on the last unconfirmed output sends
    /// `locked`. Anything else -- a vblank for the previous frame, a stale
    /// one for a discarded flip, another output's flip with the same number,
    /// an untrackable error -- leaves the wait alone for the fallback timer.
    ///
    /// A confirm schedules the follow-up tick through [`State::confirm_lock`],
    /// which owns the re-arm; a vblank that matches nothing arms nothing.
    pub(super) fn note_flip_completed(&mut self, id: OutputId, completed: Option<u64>) {
        if !self.session_lock.awaiting_blank() {
            return;
        }
        let expected = self.outputs.len();
        let blocked = expected > 1
            && self
                .outputs
                .get(id)
                .is_some_and(|output| self.session_lock.output_blocks_confirm(output));
        if self
            .session_lock
            .confirm_on_vblank(id, completed, blocked, expected)
        {
            tracing::debug!("session lock confirmed: every output's blanked frame reached scanout");
            self.confirm_lock();
        }
    }

    /// The vblank fallback bound passed: confirm the pending lock anyway
    /// rather than hang the locker.
    ///
    /// Called by the one-shot timer the render loop arms when it starts the
    /// wait. warn!, not debug!: this means a blanked frame went out without
    /// the scanout confirmation the protocol wants -- while switched away,
    /// after a discarded flip, or on hardware whose completions never arrive
    /// -- and that is exactly the event an operator debugging a lock screen
    /// needs at the default log level.
    ///
    /// The timeout stands in for the missing *vblanks*, never for a missing
    /// frame or a missing lock surface: an output that has not rendered a
    /// blank for this lock (a failed render -- its screen still shows the
    /// pre-lock desktop) is not recorded, and neither, with more than one
    /// output, is one whose admitted surface has not drawn yet (see
    /// [`SessionLock::output_blocks_confirm`]), exactly as on the
    /// render-confirmed backends. The next frame that does draw one arms a
    /// fresh wait. With one output the deadline is only ever armed by that
    /// output's own drawn blank, so every timeout confirms, as it always has.
    ///
    /// Like the vblank path, the follow-up tick is scheduled by
    /// [`State::confirm_lock`], which owns the re-arm.
    pub(super) fn note_blank_timeout(&mut self, now: Instant) {
        if !self.session_lock.poll_blank_timeout(now) {
            return;
        }
        let expected = self.outputs.len();
        for (id, output) in self.outputs.iter_with_ids() {
            // Only an output that drew its blank for this lock: the timeout
            // stands in for a lost completion, never for a render that never
            // happened (see `SessionLock::drawn`).
            if !self.session_lock.drawn.contains(&id) {
                continue;
            }
            let blocked = expected > 1 && self.session_lock.output_blocks_confirm(output);
            if !blocked && !self.session_lock.confirmed.contains(&id) {
                self.session_lock.confirmed.push(id);
            }
        }
        if self.session_lock.confirmed.len() >= expected {
            tracing::warn!(
                "confirming a session lock without its vblank: no completion \
                 arrived within {TIMEOUT:?}",
                TIMEOUT = LOCK_VBLANK_TIMEOUT,
            );
            self.confirm_lock();
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
            self.drop_input_grabs();
            self.request_render();
        }
    }

    /// This frame's lock elements, front-most first: the mapped lock
    /// surfaces each preceded by the popups parented to it, then the opaque
    /// backdrop behind them.
    ///
    /// The whole element list while locked, by construction -- the caller
    /// adds only the cursor in front of it. Nothing else is gathered, so
    /// there is no ordering mistake that could put a window behind the
    /// backdrop instead of out of the frame entirely.
    pub(super) fn lock_elements<R>(
        &mut self,
        renderer: &mut R,
        output: &Output,
        scale: f64,
        size: (i32, i32),
    ) -> (Vec<WaylandSurfaceRenderElement<R>>, SolidColorRenderElement)
    where
        R: Renderer + ImportAll,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let surfaces = self.session_lock.surface_elements(renderer, output, scale);
        let backdrop = self.session_lock.backdrop_element(Point::default(), size);
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
    ///
    /// The pointer's output picks the surface (see
    /// [`SessionLock::keyboard_focus`]); with one output that is the first
    /// current surface, exactly as before.
    pub(super) fn lock_keyboard_focus(&self) -> Option<WlSurface> {
        // Owned, so no borrow of `self.outputs` outlives the pointer read --
        // and an `Output` clone is an `Arc` bump. `None` where the pointer is
        // over no output, which simply skips the preferred-output pass.
        let preferred = self.seat.get_pointer().and_then(|pointer| {
            self.output_under(pointer.current_location())
                .map(|(output, _)| output)
        });
        self.session_lock.keyboard_focus(preferred.as_ref())
    }

    /// The lock surface under `position`, if any -- the whole of pointer
    /// hit-testing while locked.
    ///
    /// Each surface is hit-tested against the output it was admitted for, so
    /// a position over one output can only ever enter that output's surface.
    /// Asks the surface tree, so a client's own `set_input_region` is
    /// honoured exactly as it is for a window; a lock surface that excludes
    /// a point simply gets no pointer there, and nothing behind it is
    /// consulted, because nothing behind it is reachable.
    pub(super) fn lock_surface_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.session_lock
            .current()
            .find_map(|surface| {
                let output = self
                    .session_lock
                    .surface_outputs
                    .get(&surface.wl_surface().id())?;
                let origin = self
                    .space
                    .output_geometry(output)
                    .map(|geometry| geometry.loc)
                    .unwrap_or_default();
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
    ///
    /// Also prunes [`SessionLock::acked`]'s record of the surface, which lives
    /// past the surface list on purpose (see that field) and so needs its own
    /// removal here, and [`SessionLock::surface_outputs`]' admission entry --
    /// bounded-hygiene rather than load-bearing (a forgotten surface is out
    /// of [`SessionLock::current`], so its entry is never consulted again),
    /// but without it the map would grow with every surface the session ever
    /// admitted. Silent -- none of the three affects the return value: a
    /// surface that was already filtered out of the list needs no focus or
    /// redraw catch-up when its `wl_surface` finally goes.
    pub(super) fn forget_lock_surface(&mut self, surface: &WlSurface) -> bool {
        let before = self.session_lock.surfaces.len();
        self.session_lock
            .surfaces
            .retain(|lock| lock.wl_surface() != surface);
        self.session_lock.acked.remove(&surface.id());
        self.session_lock.surface_outputs.remove(&surface.id());
        before != self.session_lock.surfaces.len()
    }

    /// Reconfigures the resized output's lock surfaces for its new size.
    pub(super) fn resize_lock_surfaces(&mut self, output: &Output, size: (i32, i32)) {
        self.session_lock.configure_output(output, size);
    }
}
