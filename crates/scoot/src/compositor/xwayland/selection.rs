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
//! What the gate does guarantee is the Wayland half: while the user is in a
//! Wayland window (or the lock screen), no X client can put anything on
//! the Wayland clipboard or read it.
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
//! is not enough: a background X client could take the selection afterwards
//! -- answering no `TARGETS`, so nothing is ever announced -- and serve every
//! later Wayland paste. The owner that crossed is recorded
//! ([`CrossedOwners`]), and a paste is served only while it still owns the
//! selection (`X11Wm::selection_owner`, a scoot-sh fork addition); otherwise
//! the paste reads nothing and the stale selection is taken off the Wayland
//! side.
//!
//! Two windows are left, neither a way around the gate. A conversion already
//! sent when ownership changes is answered by the owner it was sent to --
//! the approved one -- as ICCCM asks. And the owner is read when the types
//! arrive, not when they were asked for: a client taking the selection
//! between an owner's `TARGETS` request and its answer is recorded in its
//! place, with the earlier owner's types. That can only happen while the
//! gate is open -- an X window focused, when any X client may set the
//! selection anyway -- so it grants nothing the gate does not.
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
use smithay::xwayland::xwm::{SelectionError, X11Window, XwmId};

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

/// The X window whose selection is the Wayland one, per selection (see the
/// module doc's "The owner a paste reaches"). `None` when that selection is
/// not an X one.
#[derive(Debug, Default)]
pub struct CrossedOwners {
    clipboard: Option<X11Window>,
    primary: Option<X11Window>,
}

impl CrossedOwners {
    fn slot(&mut self, target: SelectionTarget) -> &mut Option<X11Window> {
        match target {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        }
    }

    fn get(&self, target: SelectionTarget) -> Option<X11Window> {
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
        // The owner the window manager tracks now is the one whose types it
        // just read: it updates the owner before asking for `TARGETS`, and a
        // newer owner would be announced in turn.
        let owner = self
            .xwm
            .as_ref()
            .filter(|wm| wm.id() == xwm)
            .map(|wm| wm.selection_owner(target))
            .filter(|&owner| owner != smithay::reexports::x11rb::NONE);
        let Some(owner) = owner else {
            // Released between the types and now: nothing to cross.
            self.drop_x11_selection_from_wayland(xwm, target);
            return;
        };
        *self.x11_selection_owners.slot(target) = Some(owner);
        tracing::debug!(
            ?target,
            owner,
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
        let owner = xwm.selection_owner(target);
        if self.x11_selection_owners.get(target) != Some(owner) {
            tracing::debug!(
                ?target,
                owner,
                "refusing a Wayland paste of an X selection: its owner is not the one the gate let through"
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
