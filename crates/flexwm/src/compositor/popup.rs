//! `xdg_popup.grab`: routing input into a menu, and taking it back.
//!
//! A popup already configures, maps and draws without anything here (see
//! `handlers.rs`'s `send_popup_initial_configure`). What this module owns is
//! the *input* half: when a client opens a menu it asks for an explicit
//! grab, and the protocol's answer is that keyboard and pointer go to the
//! popup tree until it is dismissed -- which is also the only thing that
//! ever sends `popup_done`, so Escape-to-close and click-outside-to-close
//! both live here rather than anywhere else.
//!
//! Smithay supplies the machinery ([`PopupGrab`], [`PopupKeyboardGrab`],
//! [`PopupPointerGrab`]). What flexwm has to decide is how that machinery
//! sits inside a focus model that already had three answers in it.
//!
//! ## Precedence, highest first
//!
//! 1. **The session lock.** While the session is locked nothing but a lock
//!    surface may receive a keystroke (see `session_lock.rs`). A grab is not
//!    an exception: Smithay's [`PopupKeyboardGrab`] *ignores* a
//!    `set_focus` while it is live, so leaving one installed across a lock
//!    would route the user's password into whatever had the menu open.
//! 2. **An `exclusive` layer surface on `top`/`overlay`** -- a launcher, a
//!    layer-shell lock screen. `layer_shell.rs` documents this as "takes the
//!    keyboard the moment it maps and keeps it until it unmaps", and that
//!    rule wins here rather than fighting: a launcher opened over a menu
//!    must be typeable.
//! 3. **The popup grab**, over the focused window and over a layer surface
//!    that only got the keyboard from a click. That is the whole point of
//!    the request.
//!
//! Losing is spelled `popup_done`, not "unset the seat grab": the protocol
//! lets a compositor dismiss a popup whenever it likes, and a menu left
//! mapped with its input taken away is a menu the user cannot get rid of.
//! So both the refusal path ([`State::grab_popup`]) and the pre-emption path
//! ([`State::dismiss_popup_grab`]) dismiss the popup tree outright.
//!
//! ## Who gets the last word when a grab ends
//!
//! Smithay restores focus to the grab's *root* -- the toplevel or layer
//! surface the popup chain hangs off. That is right for a window and wrong
//! for a bar that asked for `keyboard_interactivity: none`, which would end
//! up holding the keyboard it never wanted. [`State::settle_popup_grab`] is
//! what makes flexwm's own derivation final: it notices the grab has ended
//! and re-runs [`State::refresh_keyboard_focus`], so the answer comes from
//! the same policy every other focus change uses.
//!
//! ## What is deliberately *not* here
//!
//! Keyboard focus never moves onto a popup that did not grab. The backlog
//! entry asked for that too; it is unsafe as stated, because a tooltip is an
//! ordinary `xdg_popup` (GTK's are) and handing one the keyboard takes it
//! away from the window the user is typing into. niri, sway and mutter all
//! draw the same line. See the resolution doc for the full reasoning.

use flexwm_core::WindowId;
use smithay::desktop::{
    PopupGrab, PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy,
    find_popup_root_surface,
};
use smithay::input::Seat;
use smithay::input::pointer::Focus;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::utils::{SERIAL_COUNTER, Serial};
use smithay::wayland::shell::xdg::PopupSurface;

use super::State;

// Tests live in `layer_shell/tests/popup.rs`, not beside this file: every
// question here ("did the client get `wl_keyboard.enter` on the popup",
// "did it get `popup_done`") is a claim about the wire, and the real-client
// harness that can ask it already exists there -- along with the layer
// surfaces half these cases need. A second copy of that harness is exactly
// what `docs/backlog/testing/large-test-file-organization.md` exists to
// stop.

impl State {
    /// A client asked for an explicit grab on `surface`
    /// (`XdgShellHandler::grab`).
    ///
    /// Reached from Smithay's `PopupSurface::pre_commit_hook`, i.e. during
    /// the popup's own commit and before it has a buffer -- the pinned rev
    /// answers a grab requested after the popup is mapped with
    /// `invalid_grab` and never calls this at all, so there is no
    /// already-mapped case to handle here.
    pub(super) fn grab_popup(
        &mut self,
        surface: PopupSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
    ) {
        // `None` only for a `wl_seat` that is already dead or was never this
        // compositor's -- a client that disconnected mid-request. Nothing to
        // dismiss either, since the popup goes with it.
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        let popup = PopupKind::Xdg(surface);
        // `Err` means the popup has no parent, which the role's own
        // pre-commit check has already refused with `not_constructed` -- so
        // this is a torn-down client, not a reachable protocol state.
        let Ok(root) = find_popup_root_surface(&popup) else {
            return;
        };

        if self.popup_grab_outranked() {
            // Checked *here* and not only on the next focus refresh: a
            // client can ask for a grab at any moment, including while the
            // screen is locked or a launcher is up, and a grant followed by
            // an immediate revoke would flicker the keyboard through the
            // menu on its way back.
            tracing::debug!("refusing a popup grab: something outranks it for the keyboard");
            let _ = PopupManager::dismiss_popup(&root, &popup);
            return;
        }

        // `root` is exactly what `grab_popup` recomputes for its own
        // debug assertion, from the same parent chain and in the same
        // synchronous call, so that assertion cannot fire from here.
        let mut grab = match self.popups.grab_popup(root, popup, &seat, serial) {
            Ok(grab) => grab,
            Err(error) => {
                // Every variant is the client's own doing and Smithay has
                // already answered the one that is a protocol error
                // (`NotTheTopmostPopup`); the rest are races with a dying
                // parent. Logged, never fatal.
                tracing::debug!(%error, "a popup grab was denied");
                return;
            }
        };

        // Both devices are checked before either is touched, so a refusal
        // can never leave half a grab installed. A grab already on the seat
        // that this request is not nested inside belongs to someone else --
        // an input method's `zwp_input_method_v2.grab_keyboard`, or the
        // implicit click grab from a button that is still held -- and
        // stealing it is exactly the fight this module exists to avoid.
        let keyboard = seat.get_keyboard();
        let pointer = seat.get_pointer();
        let nested = grab.previous_serial().unwrap_or(serial);
        let taken = keyboard
            .as_ref()
            .is_some_and(|k| k.is_grabbed() && !(k.has_grab(serial) || k.has_grab(nested)))
            || pointer
                .as_ref()
                .is_some_and(|p| p.is_grabbed() && !(p.has_grab(serial) || p.has_grab(nested)));
        if taken {
            tracing::debug!("refusing a popup grab: another grab already holds the seat");
            grab.ungrab(PopupUngrabStrategy::All);
            self.request_render();
            return;
        }

        // The install order is keyboard-down, pointer, keyboard-up, and all
        // three steps are here because of what happens when this grab
        // *replaces* a live one -- a submenu opening on top of the menu it
        // came from.
        //
        // `PointerHandle::set_grab` runs the outgoing grab's `unset`, and
        // `PopupPointerGrab::unset` hands keyboard focus back to the chain's
        // root whenever the keyboard is still grabbed with a serial matching
        // the *outgoing* grab. A client is free to reuse one input serial for
        // both grabs -- a submenu opened by hovering has had no new input
        // event to draw a fresh one from -- and then that condition is true
        // of the grab being installed right now, so the outgoing pointer
        // grab's teardown silently removes the incoming keyboard grab and
        // drops focus back to the toplevel with the submenu still on screen.
        // Measured, with a real client, before this order was chosen.
        //
        // Taking the keyboard grab down first makes that teardown a no-op.
        // It cannot take down a grab belonging to anyone else: the check
        // above has already established that any keyboard grab still
        // installed here is one of this popup chain's own.
        if let Some(keyboard) = &keyboard {
            keyboard.unset_grab(self);
        }
        if let Some(pointer) = &pointer {
            // `Focus::Keep`: the pointer is wherever the click that opened
            // the menu left it, and the grab decides for itself which
            // surface may have it from here.
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
        if let Some(keyboard) = &keyboard {
            // Focus before grab: `PopupKeyboardGrab` only lets a `set_focus`
            // through when it already names the current grab, so setting it
            // afterwards would be swallowed by the grab just installed.
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        // Held so flexwm can ask whether the grab is over (`settle_popup_grab`)
        // and end it on its own terms (`dismiss_popup_grab`); neither is
        // answerable through the seat, which hands back only a `&dyn` grab.
        self.popup_grab = Some(grab);
    }

    /// Whether something outranks a popup grab for the keyboard right now --
    /// the session lock, or an `exclusive` layer surface.
    ///
    /// Reads the same two sources [`State::refresh_keyboard_focus`] does, so
    /// the grant-time answer and the pre-emption answer cannot drift apart.
    /// Costs a layer-map walk, which is why only the two grab paths call it
    /// and not the per-event ones.
    fn popup_grab_outranked(&self) -> bool {
        self.session_lock.is_locked()
            || self
                .layer_keyboard_focus()
                .is_some_and(|found| found.exclusive)
    }

    /// Which window's popup tree currently holds the keyboard through an
    /// explicit `xdg_popup.grab`, if any -- the answer `flexwm msg windows`
    /// reports per window as `popup_grab` (see `ipc.rs`).
    ///
    /// Read off the held grab's own root surface, so there is no second copy
    /// of "who grabbed" to keep in sync and nothing to clear when the grab
    /// ends: a dead root simply maps to no window. Three cases report
    /// nothing, each honestly:
    ///
    /// - no grab is held at all;
    /// - the held grab has ended but [`State::settle_popup_grab`] has not
    ///   reaped it yet -- defensive only: destroy processing and the reap
    ///   run in the same display-source callback on the thread that also
    ///   serves IPC, so no request can land between them, and `has_ended`
    ///   is only answerable after `cleanup()` anyway. The keyboard is
    ///   already back where Smithay's teardown put it, not in any menu;
    /// - the grab is rooted at a layer surface (a bar's own dropdown), which
    ///   [`State::id_of`] maps to no window;
    /// - the session is locked: keys go to the lock surface, so no window
    ///   is reported as holding them.
    ///
    /// Costs a grab-state read and one walk of the window map, no
    /// allocation, and the one caller resolves it once per `windows`
    /// request -- not per window, and not on any per-frame path.
    pub(super) fn popup_grab_holder(&self) -> Option<WindowId> {
        let grab = self.popup_grab.as_ref()?;
        if grab.has_ended() {
            return None;
        }
        // The start data names the grab's root -- the toplevel or layer
        // surface the popup chain hangs off -- for the grab's whole life,
        // unlike `current_grab`, which names the topmost popup itself (and
        // the root once the grab has ended, which the check above already
        // excluded). Only the root maps onto a window.
        let root = grab.keyboard_grab_start_data().focus.as_ref()?;
        self.id_of(root)
    }

    /// Ends the active popup grab, if any, dismissing its popups.
    ///
    /// The unsets are serial-guarded because the seat's grab may no longer
    /// be this one: an input method can take the keyboard afterwards, and a
    /// button press installs its own pointer grab. Ripping out whichever
    /// grab happens to be installed would break a drag or an IME mid-compose
    /// for no reason.
    ///
    /// Keyboard first, pointer second, and the order is load-bearing:
    /// `PopupPointerGrab::unset` restores keyboard focus to the popup's root
    /// if the keyboard is still grabbed, which would put a `wl_keyboard.enter`
    /// on the root in between the caller's own decision and its effect.
    pub(super) fn dismiss_popup_grab(&mut self) {
        let Some(mut grab) = self.popup_grab.take() else {
            return;
        };
        // `popup_done` for the whole chain, innermost first. This also takes
        // the popups out of the parent's `PopupTree`, so they stop being
        // drawn on the very next frame rather than when the client gets
        // around to destroying them -- hence the redraw below.
        grab.ungrab(PopupUngrabStrategy::All);
        let serial = grab.serial();
        if let Some(keyboard) = self.seat.get_keyboard()
            && keyboard.has_grab(serial)
        {
            keyboard.unset_grab(self);
        }
        if let Some(pointer) = self.seat.get_pointer()
            && pointer.has_grab(serial)
        {
            let time = smithay::backend::input::InputTime::from_millis(self.millis());
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
        self.request_render();
    }

    /// Notices that the active grab has ended -- the client destroyed its
    /// popup, its parent went away, or a click outside dismissed it -- and
    /// gives flexwm's own focus policy the last word.
    ///
    /// Without this the keyboard stays where Smithay's grab put it back: on
    /// the popup's *root*, which is right for a window and wrong for a bar
    /// that never asked for keys.
    ///
    /// Called from the wayland display source (a client's destroy is seen
    /// there and nowhere else) and after a pointer button (which is what
    /// dismisses a menu by clicking outside it). Both are gated on there
    /// being a grab at all, so an ordinary session pays one `Option` check.
    pub(super) fn settle_popup_grab(&mut self) {
        if self.popup_grab.is_none() {
            return;
        }
        // A popup the client destroyed stays in the grab's own list until
        // the manager reaps it (`PopupGrabInner::cleanup`), so "has it
        // ended" is only answerable after this.
        self.popups.cleanup();
        if !self
            .popup_grab
            .as_ref()
            .is_some_and(|grab| grab.has_ended())
        {
            return;
        }
        self.popup_grab = None;
        self.refresh_keyboard_focus();
        // The dismissed popup's pixels are gone from the tree; nothing else
        // marks the screen dirty for a destroy.
        self.request_render();
    }
}

/// The type [`State::popup_grab`](super::State::popup_grab) holds, named so
/// the field's declaration reads without a turbofish.
pub(super) type ActivePopupGrab = PopupGrab<State>;
