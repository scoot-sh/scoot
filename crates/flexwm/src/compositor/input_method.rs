//! `text-input-v3` and `input-method-v2`: composing text that a keyboard
//! cannot type directly.
//!
//! Two halves of one feature, and neither is useful alone:
//!
//! - **`zwp_text_input_v3`** is what an *application* binds. It tells the
//!   compositor "there is a text field here, this is where the cursor is,
//!   this is the surrounding text", and receives composed text back. A
//!   terminal or editor with no input method attached simply never hears
//!   anything -- which is exactly what `foot` means by "text input interface
//!   not implemented by compositor; IME will be disabled": without the global
//!   it does not even set up the path.
//! - **`zwp_input_method_v2`** is what an *input method* binds -- fcitx5,
//!   ibus, or an on-screen keyboard. It is told which text field is focused,
//!   and sends composed strings and preedit back through the compositor to
//!   that field.
//!
//! So `text-input-v3` on its own removes the warning and lets applications
//! set the path up, and `input-method-v2` is what ever puts anything through
//! it. They land together here for that reason.
//!
//! # What this module actually has to do
//!
//! Very little, and that is worth stating plainly rather than looking like an
//! omission. Smithay owns the whole middle of this: which text field is
//! focused follows keyboard focus automatically (`wayland/seat/keyboard.rs`
//! calls `text_input.set_focus`/`enter`/`leave` from the same `set_focus`
//! `shell.rs::refresh_keyboard_focus` already calls), the preedit and commit
//! traffic is forwarded between the two protocols inside `InputMethodHandle`,
//! and the IME's optional keyboard grab is an ordinary Smithay grab.
//!
//! What is left for the compositor is the *popup*: the candidate window an
//! IME shows next to the text cursor (the list of Chinese characters matching
//! what has been typed so far, the emoji picker, the on-screen keyboard's own
//! surface). That is a real surface that has to be tracked, positioned and
//! drawn, and it is what the four handler methods below are for.
//!
//! # How the popup gets drawn
//!
//! Through [`PopupManager`], not through a list of this module's own. An
//! input-method popup is a [`PopupKind::InputMethod`], and both of the things
//! this compositor renders surfaces from already draw their own popups: a
//! `Window`'s render elements include `PopupManager::popups_for_surface`, and
//! so do a `LayerSurface`'s (checked in the pinned rev's
//! `desktop/space/wayland/{window,layer}.rs`). So tracking the popup against
//! its parent is the whole of "make it appear" -- and, just as importantly,
//! frame callbacks and output enter/leave follow the same path, so an
//! animated IME popup is not stalled by this module forgetting to do
//! something the window path already does.
//!
//! The parent matters and is not always a window. Text-input focus follows
//! *keyboard* focus, and in flexwm the keyboard can be on a layer surface --
//! a launcher with a search field is exactly that, and exactly the case an
//! IME user would notice (see `layer_shell.rs`). [`State::parent_geometry`]
//! therefore answers for both, rather than returning an empty rectangle for
//! half of the sessions where an IME matters.

use smithay::desktop::{PopupKind, PopupManager, WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface};

use super::State;

#[cfg(test)]
mod tests;

impl InputMethodHandler for State {
    /// An input method created a popup, or an existing one was re-parented
    /// because focus moved to a different text field.
    ///
    /// Tracking it against its parent is what makes it render (see the module
    /// doc). `track_popup` fails only on a dead parent surface, which is
    /// reachable -- the client the IME is composing for can quit between the
    /// activation and this call -- so it is logged rather than unwrapped
    /// (anvil unwraps; a panic here takes every client's session with it).
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(error) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!(%error, "could not track an input-method popup");
            return;
        }
        // Nothing commits on this path -- the popup's surface may have had
        // its buffer for a while and simply moved to a new parent -- so the
        // frame that shows it has to be asked for here.
        self.request_render();
    }

    /// The input method's popup is going away, or its parent is: focus moved
    /// off the text field, or the IME deactivated.
    ///
    /// Untracked through the parent it was tracked against, which is the only
    /// key `PopupManager` holds it under. A popup with no parent was never
    /// tracked (`InputMethodHandle` only calls `new_popup` once it has one),
    /// so there is nothing to remove for it.
    fn dismiss_popup(&mut self, surface: PopupSurface) {
        let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) else {
            return;
        };
        let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        self.request_render();
    }

    /// The text cursor moved within its field, so the popup follows it.
    ///
    /// Smithay has already written the new rectangle into the popup's own
    /// state by the time this runs (`InputMethodHandle::set_text_input_rectangle`),
    /// and the render path reads that per frame -- so the only thing missing
    /// is a frame to read it in.
    fn popup_repositioned(&mut self, _surface: PopupSurface) {
        self.request_render();
    }

    /// Where the surface holding the focused text field is, so the IME can
    /// place its candidate window against it rather than at the origin.
    ///
    /// **This must be the parent's *surface-local* geometry, not its position
    /// on the output**, and that is the whole subtlety of this function.
    /// Smithay stores what it returns as `PopupParent::location`, and the
    /// render path reads it straight back as `popup.geometry().loc` and
    /// *subtracts* it from the popup's own offset -- where that offset is the
    /// client's `set_cursor_rectangle`, which the text-input protocol defines
    /// as surface-local. The two render paths in the pinned rev then differ:
    ///
    /// - `space/wayland/window.rs` computes `self.geometry().loc +
    ///   popup_offset - popup.geometry().loc`, so returning `window.geometry()`
    ///   makes the two cancel and leaves the surface-local offset.
    /// - `space/wayland/layer.rs` computes `popup_offset -
    ///   popup.geometry().loc` and adds *nothing* back -- because
    ///   `headless.rs::layer_elements` has already placed the surface at
    ///   `LayerMap::layer_geometry(..).loc`.
    ///
    /// So for a layer surface this returns [`LayerSurface::geometry`] (the
    /// client's own window geometry, normally at the origin) rather than
    /// `LayerMap::layer_geometry`, which is that same rectangle *plus the
    /// surface's position on the output*. Returning the latter cancels
    /// against the position the render path already applied, leaving the
    /// popup at the raw surface-local caret interpreted as output
    /// coordinates: a launcher anchored centre on a 1600x1000 output puts its
    /// candidate window in the top-left corner of the screen instead of
    /// beside its search field. It is invisible only for a surface that
    /// happens to sit at the origin, i.e. a top-left-anchored bar -- which is
    /// why `layer_parent_geometry_is_surface_local` below tests it at a
    /// non-zero position specifically.
    ///
    /// Takes the layer map's guard, reads, and drops it without calling back
    /// into Smithay, per `layer_shell.rs`'s guard-discipline note.
    ///
    /// An unknown surface gets the default (empty) rectangle, which is what
    /// the protocol's own callers do with no parent at all: the popup is
    /// tracked and positioned at the origin. That covers a text field on a
    /// surface this compositor does not lay out -- a session-lock surface's
    /// password box, most concretely, where the popup is in fact never drawn
    /// at all (the locked render path replaces the element list wholesale
    /// rather than gathering popups; see `session_lock.rs` and
    /// `docs/backlog/protocols/ime-popup-over-lock-screen.md`).
    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        if let Some(window) = self.id_of(parent).and_then(|id| self.window(id)) {
            return window.geometry();
        }
        let Some(output) = self.output.as_ref() else {
            return Rectangle::default();
        };
        let map = layer_map_for_output(output);
        map.layer_for_surface(parent, WindowSurfaceType::TOPLEVEL)
            .map(|layer| layer.geometry())
            .unwrap_or_default()
    }
}
