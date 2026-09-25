//! Phase 4: the clipboard and the primary selection across the X/Wayland
//! line, both ways.
//!
//! Smithay's window manager does the X half -- it watches the X
//! selections (XFixes), converts them, and speaks `INCR` for large
//! transfers (`xwm/selection.rs` at the pinned fork). This module decides
//! what crosses, and hooks it into the same Smithay selection state the
//! Wayland devices use, so a clipboard manager on `zwlr_data_control` or
//! `ext_data_control` sees an X selection like any other.
//!
//! # The gate
//!
//! A Wayland client may set the clipboard or the primary selection only
//! while it holds the keyboard (Smithay's `wl_data_device` and primary
//! device refuse anyone else), and only the focused client is offered them.
//! XWayland is one Wayland client standing for every X client, so the X
//! form of that rule is: **an X client may set or read a selection only
//! while an X window of this server holds the keyboard** -- the seat's
//! focus is [`KeyboardFocus::X11`] -- **and the session is not locked.**
//!
//! "Background" therefore means "any X client while the user is not in an
//! X window". The gate cannot tell X clients apart from each other, and
//! does not try to:
//!
//! - a read names no requestor at all (Smithay's `allow_selection_access`
//!   carries only the selection), so reads can only ever be gated per
//!   server;
//! - a clipboard tool has no window (`xclip`, `xsel`), so a rule keyed to
//!   the focused window's own client would refuse `xclip` run from the very
//!   terminal the user is typing in;
//! - and inside the X server there is no isolation to preserve: any X
//!   client can read and replace another's selection, or type into it,
//!   by design (see `docs/protocols.md`'s trust note).
//!
//! What the gate does hold is the Wayland half: while the user is in a
//! Wayland window (or the lock screen), no X client can read the Wayland
//! clipboard or put a new selection on it. It cannot fully protect a paste
//! of something copied *in X* -- see "The owner a paste reaches".
//!
//! # What crosses, and when
//!
//! - **Wayland -> X.** Every Wayland selection change (a focused client's
//!   `set_selection`, a data-control client's) is announced to the X server:
//!   the window manager takes the X selection with the Wayland types, and an
//!   X client's conversion request is served from the Wayland source --
//!   through [`State::x11_selection_allowed`] first.
//! - **X -> Wayland.** A new X owner is announced (after the window manager
//!   has read its types); when the gate allows, it becomes the Wayland
//!   selection as a compositor-provided one, tagged with its server's
//!   [`XwmId`], and a Wayland paste is converted from the X owner on demand.
//!
//! # The owner a paste reaches
//!
//! The window manager converts a Wayland paste from whoever owns the X
//! selection *at the time of the paste*, and it only tells scoot about a new
//! owner once that owner answers `TARGETS`. So an owner the gate let through
//! is not enough on its own: a background X client could take the selection
//! afterwards -- answering no `TARGETS`, so nothing is ever announced -- and
//! serve every later Wayland paste. Comparing owner *windows* does not stop
//! it either: `SetSelectionOwner` accepts any window id, so the client can
//! take the selection under the approved owner's own window (measured).
//! Instead the window manager counts ownership changes
//! (`X11Wm::selection_generation`, a scoot-sh fork addition): the count is
//! recorded when a selection crosses ([`CrossedOwners`]), and a paste is
//! served only while no change of hands has happened since. Otherwise the
//! paste reads nothing and the stale selection is taken off the Wayland side.
//!
//! **This raises the bar; it is not a guarantee.** A background X client
//! can still answer a paste in the owner's place without owning anything:
//! the window manager reads the answer from a property on a window of its
//! own, which any X client may write, after a `SelectionNotify` event, which
//! any X client may send -- and a real owner's `SelectionNotify` is a sent
//! event too, so a forged one cannot be told apart. That takes watching for
//! the window manager's paste windows and racing the owner, but X11 offers
//! nothing to close it.
//!
//! What happens to transfers in flight when the selection changes hands
//! (every ownership change, including an owner re-claiming it, the window
//! manager taking it for a Wayland selection, or the owner quitting): a
//! paste still waiting for the owner's answer is ended -- its reader reads
//! nothing; one answered by a previous owner and waiting on it for a chunk
//! is ended once it has been idle for over a second; one that is moving is
//! left to finish, since ending it would hand its reader part of the
//! selection as if it were all of it. The window manager also ends a
//! transfer whose reader has gone, or that has not moved for 30 seconds --
//! checked by a timer every second while any transfer is in flight, so a
//! paste orphaned by an X app quitting ends by itself. It caps what is in
//! flight: at most 8 pastes of one selection waiting on their owner and 8
//! under way, no more than 4 of those from one owner X client (so an owner
//! trickling a byte per chunk -- progress enough never to time out -- cannot
//! keep a different owner out), and at most 4 transfers out to one X client
//! and 16 in all. A paste past a cap reads nothing at once: it is refused,
//! not queued. X clients are counted by the client bits of a window id,
//! which a client can borrow by naming another client's window; that moves
//! whose share it spends, and the totals still hold. Each transfer buffers
//! at most one 64 KiB slice (two for a Wayland source feeding an X reader),
//! however large the selection.
//!
//! One more window, not a way around the gate: the ownership count is read
//! when the types arrive, not when they were asked for, so a client taking
//! the selection between an owner's `TARGETS` request and its answer is
//! recorded in its place. That can only happen while the gate is open -- an
//! X window focused, when any X client may set the selection anyway -- so it
//! grants nothing the gate does not.
//!
//! There is no loop: announcing an X selection to Wayland goes through
//! Smithay's compositor-side setters, which never call back into
//! `SelectionHandler::new_selection`, and the window manager ignores the
//! ownership change it made itself.
//!
//! # Hostile types
//!
//! An X selection's types are atom names, which an X client chooses freely:
//! up to 64 KiB long, and any bytes. A Wayland string carrying a NUL panics
//! the compositor (wayland-scanner's `CString::new(..).unwrap()`, the same
//! hole `manage::x11_text` closes for titles), and one longer than the wire
//! allows disconnects every client it is sent to. So only types that look
//! like mime types cross: no NUL, at most [`MAX_MIME_BYTES`], a `/` in them
//! -- which also drops the X-only target names (`MULTIPLE`,
//! `SAVE_TARGETS`, ...) no Wayland client can use -- and at most
//! [`MAX_MIME_TYPES`] of them. The same count cap applies the other way,
//! since the window manager interns every Wayland type for each X
//! `TARGETS` request.

use std::os::fd::OwnedFd;

use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata,
    request_data_device_client_selection, set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
    set_primary_selection,
};
use smithay::wayland::selection::{SelectionSource, SelectionTarget};
use smithay::xwayland::xwm::{SelectionError, XwmId};

use super::super::State;
use super::super::keyboard_focus::KeyboardFocus;

/// The longest selection type that crosses from X to Wayland. RFC 6838
/// bounds a mime type's name at 127 + 1 + 127 bytes; parameters make real
/// ones a little longer, never near a kilobyte. Far below the Wayland
/// wire's ~4 KiB string bound, so no type that passes can disconnect the
/// clients it is sent to.
pub(in crate::compositor) const MAX_MIME_BYTES: usize = 255;

/// The most selection types that cross, either way. Real applications
/// offer a few dozen at most (an office suite's clipboard is the large
/// case); each one is a Wayland event per receiving client one way and an
/// X round trip per `TARGETS` request the other.
pub(in crate::compositor) const MAX_MIME_TYPES: usize = 64;

/// Whether `mime` may cross from X to Wayland (see the module doc).
pub(in crate::compositor) fn mime_crosses(mime: &str) -> bool {
    !mime.is_empty() && mime.len() <= MAX_MIME_BYTES && !mime.contains('\0') && mime.contains('/')
}

/// The X types that cross to Wayland, in the owner's order.
pub(in crate::compositor) fn crossing_mime_types(mime_types: Vec<String>) -> Vec<String> {
    let mut crossing = mime_types;
    crossing.retain(|mime| mime_crosses(mime));
    crossing.truncate(MAX_MIME_TYPES);
    crossing
}

/// The window manager's ownership count (`X11Wm::selection_generation`) at
/// the moment each selection crossed to Wayland (see the module doc's "The
/// owner a paste reaches"). `None` when that selection is not an X one.
#[derive(Debug, Default)]
pub struct CrossedOwners {
    clipboard: Option<u64>,
    primary: Option<u64>,
}

impl CrossedOwners {
    fn slot(&mut self, target: SelectionTarget) -> &mut Option<u64> {
        match target {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        }
    }

    fn get(&self, target: SelectionTarget) -> Option<u64> {
        match target {
            SelectionTarget::Clipboard => self.clipboard,
            SelectionTarget::Primary => self.primary,
        }
    }
}

impl State {
    /// The module doc's gate: an X window of server `xwm` holds the
    /// keyboard, and the session is not locked.
    ///
    /// The lock is checked first and on its own. Today it is redundant --
    /// every lock transition re-derives the keyboard onto the lock surface,
    /// or onto nothing (`State::refresh_keyboard_focus`), so no X window
    /// holds it while locked -- but the rule the ticket states is "never
    /// while locked", and it should not rest on every future lock path
    /// remembering to move the keyboard first.
    pub(super) fn x11_selection_allowed(&self, xwm: XwmId) -> bool {
        if self.session_lock.is_locked() {
            return false;
        }
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| match focus {
                KeyboardFocus::X11 { window, .. } => window.xwm_id() == Some(xwm),
                KeyboardFocus::Surface(_) => false,
            })
    }

    /// An X client took `target` and the window manager has read its types.
    /// It becomes the Wayland selection if the gate allows; otherwise the X
    /// selection stays X-side only.
    pub(super) fn x11_new_selection(
        &mut self,
        xwm: XwmId,
        target: SelectionTarget,
        mime_types: Vec<String>,
    ) {
        let mime_types = crossing_mime_types(mime_types);
        if !self.x11_selection_allowed(xwm) || mime_types.is_empty() {
            tracing::debug!(
                ?target,
                "an X selection stays X-side: no X window holds the keyboard, the session is locked, or it offers no usable type"
            );
            // The X owner changed either way, so an X selection Wayland was
            // given earlier no longer exists -- and a paste of it would now
            // be converted from whoever owns it instead.
            self.drop_x11_selection_from_wayland(xwm, target);
            return;
        }
        // The ownership the window manager tracks now is the one whose types
        // it just read: it counts the change before asking for `TARGETS`, and
        // a newer one would be announced in turn.
        let crossed = self
            .xwm
            .as_ref()
            .filter(|wm| wm.id() == xwm)
            .filter(|wm| wm.selection_owner(target) != smithay::reexports::x11rb::NONE)
            .map(|wm| (wm.selection_owner(target), wm.selection_generation(target)));
        let Some((owner, generation)) = crossed else {
            // Released between the types and now: nothing to cross.
            self.drop_x11_selection_from_wayland(xwm, target);
            return;
        };
        *self.x11_selection_owners.slot(target) = Some(generation);
        tracing::debug!(
            ?target,
            owner,
            generation,
            types = mime_types.len(),
            "an X selection crosses to Wayland"
        );
        let handle = self.display_handle.clone();
        match target {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&handle, &self.seat, mime_types, xwm);
            }
            SelectionTarget::Primary => set_primary_selection(&handle, &self.seat, mime_types, xwm),
        }
    }

    /// The X selection `target` lost its owner (the client cleared it, or
    /// quit). If Wayland was holding it, it goes; a Wayland client's own
    /// selection is untouched.
    pub(super) fn x11_cleared_selection(&mut self, xwm: XwmId, target: SelectionTarget) {
        self.drop_x11_selection_from_wayland(xwm, target);
    }

    /// Clears the Wayland `target` selection if it is the one server `xwm`
    /// provided -- never a Wayland client's.
    fn drop_x11_selection_from_wayland(&mut self, xwm: XwmId, target: SelectionTarget) {
        *self.x11_selection_owners.slot(target) = None;
        let ours = match target {
            SelectionTarget::Clipboard => {
                current_data_device_selection_userdata(&self.seat).is_some_and(|data| *data == xwm)
            }
            SelectionTarget::Primary => {
                current_primary_selection_userdata(&self.seat).is_some_and(|data| *data == xwm)
            }
        };
        if !ours {
            return;
        }
        let handle = self.display_handle.clone();
        match target {
            SelectionTarget::Clipboard => clear_data_device_selection(&handle, &self.seat),
            SelectionTarget::Primary => clear_primary_selection(&handle, &self.seat),
        }
    }

    /// Server `xwm` is gone: whatever it put on the Wayland clipboard or
    /// primary selection goes with it (a paste would have nothing to
    /// convert from).
    pub(super) fn forget_x11_selections(&mut self, xwm: XwmId) {
        self.drop_x11_selection_from_wayland(xwm, SelectionTarget::Clipboard);
        self.drop_x11_selection_from_wayland(xwm, SelectionTarget::Primary);
    }

    /// An X client, through the gate, asked for the Wayland `target`
    /// selection as `mime`: the Wayland source writes it into `fd`, which
    /// the window manager relays to the X client.
    pub(super) fn x11_reads_wayland_selection(
        &mut self,
        target: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        // An error means the selection changed between the X client's
        // request and now, or it asked for a type the source never offered:
        // the X client reads an empty transfer. Per request, never per frame.
        // (The two errors are distinct types, one per protocol.)
        match target {
            SelectionTarget::Clipboard => {
                if let Err(error) = request_data_device_client_selection(&self.seat, mime_type, fd)
                {
                    tracing::debug!(%error, "an X read of the Wayland clipboard found nothing to serve");
                }
            }
            SelectionTarget::Primary => {
                if let Err(error) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    tracing::debug!(%error, "an X read of the Wayland primary selection found nothing to serve");
                }
            }
        }
    }

    /// `SelectionHandler::new_selection`: a Wayland selection changed. The X
    /// server hears about every one, so an X client pasting sees the same
    /// clipboard -- the read itself is gated when it happens.
    pub(in crate::compositor) fn wayland_selection_to_x11(
        &mut self,
        target: SelectionTarget,
        source: Option<SelectionSource>,
    ) {
        // Whatever X selection Wayland held is replaced (or cleared).
        *self.x11_selection_owners.slot(target) = None;
        let Some(xwm) = self.xwm.as_mut() else {
            return;
        };
        let mime_types = source.map(|source| {
            let mut mime_types = source.mime_types();
            mime_types.truncate(MAX_MIME_TYPES);
            mime_types
        });
        if let Err(error) = xwm.new_selection(target, mime_types) {
            tracing::warn!(?target, %error, "could not announce a Wayland selection to the X server");
        }
    }

    /// `SelectionHandler::send_selection`: a Wayland client pastes the
    /// selection server `xwm` provided, as `mime`. The window manager
    /// converts it from the X owner into `fd`.
    pub(in crate::compositor) fn wayland_reads_x11_selection(
        &mut self,
        target: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        provider: XwmId,
    ) {
        // Every refusal below drops `fd`: the reader's end of file, an empty
        // paste.
        let Some(xwm) = self.xwm.as_mut() else {
            // The server that provided it is gone.
            return;
        };
        if xwm.id() != provider {
            return;
        }
        let generation = xwm.selection_generation(target);
        if self.x11_selection_owners.get(target) != Some(generation) {
            tracing::debug!(
                ?target,
                generation,
                "refusing a Wayland paste of an X selection: it has changed hands since the gate let it through"
            );
            self.drop_x11_selection_from_wayland(provider, target);
            return;
        }
        match xwm.send_selection(target, mime_type, fd) {
            Ok(()) => {}
            // The window manager's bound on pastes in flight (a scoot-sh fork
            // addition): a client can hit it at will, so not at `warn`.
            Err(SelectionError::TooManyTransfers) => tracing::debug!(
                ?target,
                "refusing a Wayland paste of an X selection: too many already in flight"
            ),
            Err(error) => {
                tracing::warn!(?target, %error, "could not convert an X selection for a Wayland paste");
            }
        }
    }
}
