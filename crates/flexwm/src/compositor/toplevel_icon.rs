//! `xdg-toplevel-icon-v1`: the icon a client wants shown for its window.
//!
//! A client builds an `xdg_toplevel_icon_v1` out of an icon *name* (a
//! freedesktop icon-theme name, which is what nearly every real toolkit
//! sends -- usually the same string as its `.desktop` file) and/or one or
//! more square `wl_shm` buffers of pixels, then attaches it to a toplevel.
//! The attachment is double-buffered like everything else on a surface: it
//! becomes current on that surface's next commit.
//!
//! # What flexwm does with it
//!
//! Stores nothing, and draws nothing. flexwm has no titlebars, no taskbar and
//! no window-switcher of its own (see `decorations.rs`), so there is no place
//! in this compositor an icon would appear. What it is for here is the two
//! consumers *outside* it: a bar or dock showing a window list, and an agent
//! driving the session over IPC -- and both read it through
//! [`State::icon_name_of`], which resolves the surface's own current cached
//! state at the moment it is asked.
//!
//! That "at the moment it is asked" is the whole design: the icon lives in
//! the `wl_surface`'s double-buffered state, which Smithay already keeps
//! correct across commits, unmaps and destruction. Mirroring it into a
//! `HashMap<WindowId, String>` here would add a second copy that has to be
//! invalidated on every one of those transitions -- the class of bug
//! `CLAUDE.md`'s worked example is about -- to save a lookup on a path that
//! runs when an agent asks for a window list, not per frame and not per
//! event.
//!
//! # The gap this leaves
//!
//! Only the *name* is exposed. A client may instead (or additionally) supply
//! raw pixel buffers, which `ToplevelIconCachedState::buffers` holds and
//! nothing here reads: handing those to an IPC client would mean re-encoding
//! shm buffers to PNG per query, the way `screenshot.rs` does for the screen,
//! and no consumer has asked for it yet. A client that supplies only buffers
//! therefore reads as having no icon over IPC. Closed as the honest scope,
//! with the reasoning verified rather than assumed: see
//! `docs/backlog/resolved/toplevel-icon-buffers-done.md`. Two facts worth
//! keeping next to the code: the buffers are ordinary `wl_buffer`s under
//! the per-client live budget (`wl_buffers.rs` -- creating them past it is
//! refused, destroying them early kills that client with `NoBuffer`), and
//! the protocol defines no `release` for them (`add_buffer`'s own XML says
//! the event is unused), so there is no release to forget.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_protocols::xdg::toplevel_icon::v1::server::xdg_toplevel_icon_v1;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::xdg_toplevel_icon::{ToplevelIconCachedState, XdgToplevelIconHandler};

use flexwm_core::WindowId;

use super::State;

#[cfg(test)]
mod tests;

impl XdgToplevelIconHandler for State {
    /// A client attached (or cleared) an icon on a toplevel.
    ///
    /// Nothing to do but say so: the icon itself is already in the surface's
    /// pending cached state, and [`State::icon_name_of`] reads the *current*
    /// one, which the client's next commit will make this. Implemented rather
    /// than left to the trait's default body so the log line exists -- an
    /// icon that never shows up in a bar is otherwise indistinguishable from
    /// a client that never set one.
    fn set_icon(&mut self, _toplevel: XdgToplevel, wl_surface: WlSurface) {
        tracing::debug!(
            window = ?self.id_of(&wl_surface),
            "a toplevel icon was attached, pending its next commit"
        );
    }
}

impl State {
    /// Records that `icon` has been handed to a toplevel, so a later request
    /// on it can be refused before it reaches Smithay.
    ///
    /// Called from `dispatch.rs` for every
    /// `xdg_toplevel_icon_manager_v1.set_icon` that names an icon -- which is
    /// exactly when upstream freezes that icon's contents
    /// (`XdgToplevelIconUserData::freeze`). `set_icon(toplevel, None)` freezes
    /// nothing and is not recorded.
    ///
    /// Re-recording an icon already in the set is a no-op, which is correct:
    /// assigning the same icon to a second toplevel leaves it just as frozen.
    pub(super) fn note_toplevel_icon_assigned(&mut self, icon: ObjectId) {
        self.frozen_icons.insert(icon);
    }

    /// Drops an icon from the frozen set once its protocol object is gone.
    ///
    /// This is what bounds the set: an entry exists only while the client's
    /// own `xdg_toplevel_icon_v1` does, so it is bounded by the same thing
    /// wayland-backend already bounds -- how many live objects a client
    /// holds. Nothing else removes entries, deliberately: an icon does not
    /// become mutable again.
    pub(super) fn forget_toplevel_icon(&mut self, icon: &ObjectId) {
        self.frozen_icons.remove(icon);
    }

    /// Whether a request on `icon` must be refused because the icon has
    /// already been assigned, and posts the protocol error if so.
    ///
    /// See `dispatch.rs`'s module doc for the upstream fall-through this
    /// exists to get in front of. The error code and message are upstream's
    /// own, for the same reason the `wl_shm` guards match theirs: a client
    /// sees exactly what it would have seen, minus the compositor dying.
    pub(super) fn refuse_frozen_toplevel_icon(
        &self,
        icon: &xdg_toplevel_icon_v1::XdgToplevelIconV1,
    ) -> bool {
        if !self.frozen_icons.contains(&icon.id()) {
            return false;
        }
        icon.post_error(
            xdg_toplevel_icon_v1::Error::Immutable,
            "Request made after the icon has been assigned to a toplevel via 'set_icon'"
                .to_string(),
        );
        true
    }

    /// The freedesktop icon name a window's client has committed for it, if
    /// any.
    ///
    /// `None` covers every way there can be no name to report, and they are
    /// deliberately not distinguished: no such window, a window whose client
    /// never bound `xdg_toplevel_icon_manager_v1`, one that attached an icon
    /// but has not committed it yet, one that cleared its icon, and one that
    /// supplied pixel buffers but no name (see the module doc).
    ///
    /// Reads the surface's own current cached state each time rather than a
    /// cached copy -- see the module doc for why. The `with_states` closure
    /// does nothing but clone a short string, for the same reason every other
    /// `with_states` call in this codebase stays small: it holds a plain,
    /// non-reentrant mutex on that surface's data.
    pub fn icon_name_of(&self, id: WindowId) -> Option<String> {
        let surface = self.window(id)?.toplevel()?.wl_surface().clone();
        with_states(&surface, |states| {
            states
                .cached_state
                .get::<ToplevelIconCachedState>()
                .current()
                .icon_name()
                .map(str::to_owned)
        })
    }
}
