//! Client fullscreen: the Wayland half of the core's fullscreen state.
//!
//! What fullscreen *means* -- which window covers what, what ends it, how
//! leaving restores the layout -- is layout, and lives in `scoot-core`
//! (`scoot_core::Action::ToggleFullscreen` has the rules). This module is the
//! wire around it:
//!
//! - **Requests in.** A client's own `xdg_toplevel.set_fullscreen` /
//!   `unset_fullscreen` reach the core as
//!   `scoot_core::Event::FullscreenRequested`; a taskbar's wlr
//!   foreign-toplevel request, an IPC `toggle-fullscreen`/`set-fullscreen`
//!   and the `Super+f` bind go through `State::act` as actions (see
//!   `foreign_toplevel_management.rs` for the first).
//! - **State out.** `shell.rs`'s `apply()` sets the `fullscreen` state bit
//!   and the output-sized frame on every visible window's configure, from
//!   `Placement::fullscreen`; [`State::answer_fullscreen_request`] covers the
//!   window the arrangement did not configure (an invisible one) and the
//!   request that changed nothing.
//! - **What stays above.** [`State::covered_by_fullscreen`] is the one
//!   question the render stack, pointer hit-testing and keyboard focus ask
//!   (through `layer_shell::above_windows`) to hide the top layer under a
//!   covering window.
//!
//! ## While the session is locked
//!
//! A client's *own* request is honoured -- the same line `shell.rs` draws for
//! a window mapping or closing while locked: it changes that window's own
//! state, nothing is drawn behind the lock, and the session is as the client
//! left it at unlock. A request made *on the user's behalf* (a taskbar, IPC,
//! the bind) is refused, like every other action behind the lock.
//!
//! ## The output hint
//!
//! `set_fullscreen` may name an output. It is honoured only when the
//! requesting window is the focused window and the session is unlocked: then
//! it is carried there first with the existing move-to-output action (focus
//! follows it, as it does for that action), and goes fullscreen on arrival.
//! Otherwise the hint is ignored and the window goes fullscreen on the output
//! it is on -- moving a window the user is not looking at to another screen
//! on a client's say-so is not something a hint should be able to do. The
//! protocol leaves the choice of output to the compositor either way.

use scoot_core::{Action, Event, WindowId};
use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::shell::xdg::{ToplevelState, ToplevelSurface};

use super::State;

#[cfg(test)]
mod tests;

/// Sets or clears the `fullscreen` state bit in a toplevel's pending state.
///
/// Shared by `apply()`'s per-placement configure and the request answer
/// below, so the two cannot disagree about what "fullscreen" puts on the
/// wire. `ToplevelStateSet::set`/`unset` are no-ops when the bit is already
/// what is asked, so this never manufactures a pending change on its own.
pub(super) fn set_fullscreen_state(state: &mut ToplevelState, fullscreen: bool) {
    if fullscreen {
        state.states.set(xdg_toplevel::State::Fullscreen);
    } else {
        state.states.unset(xdg_toplevel::State::Fullscreen);
    }
}

impl State {
    /// Whether a fullscreen window covers `output` right now -- the core's
    /// [`World::fullscreen_on`](scoot_core::World::fullscreen_on), for an
    /// output in hand.
    ///
    /// Allocation-free (a scan of the output list and one map lookup),
    /// because pointer motion asks it at libinput's rate through
    /// `layer_hit`, and every frame asks it once per output.
    pub(super) fn covered_by_fullscreen(&self, output: &Output) -> bool {
        self.outputs
            .id_of(output)
            .and_then(|id| self.world.fullscreen_on(id))
            .is_some()
    }

    /// Re-derives pointer focus when what covers any output changed since
    /// the last `apply()`.
    ///
    /// `wl_pointer.button` goes to whatever surface the pointer last
    /// *entered*, not whatever is under it now (see
    /// `State::refresh_pointer_focus`, which the lock transitions call for
    /// the same reason). Entering fullscreen under a pointer resting on the
    /// bar, or on a window the fullscreen one now covers, would otherwise
    /// send the next click to a surface that is no longer drawn -- until
    /// the mouse happened to move. Leaving is the mirror image.
    ///
    /// Called from every `apply()`; costs one `fullscreen_on` per output and
    /// no allocation when nothing changed, which is nearly always, and the
    /// synthesized motion only when something did.
    pub(super) fn refresh_fullscreen_cover(&mut self) {
        let count = self.outputs.len();
        // A slot only counts as changed when what covers it did: an output
        // appearing with nothing on it, which is every output at startup,
        // must not synthesize a pointer motion nothing asked for. An output
        // that went away while covered did change what is on screen.
        let mut changed = self
            .fullscreen_covers
            .get(count..)
            .is_some_and(|gone| gone.iter().any(Option::is_some));
        self.fullscreen_covers.resize(count, None);
        for (index, (id, _)) in self.outputs.iter_with_ids().enumerate() {
            let cover = self.world.fullscreen_on(id);
            if let Some(slot) = self.fullscreen_covers.get_mut(index)
                && *slot != cover
            {
                *slot = cover;
                changed = true;
            }
        }
        if changed {
            self.refresh_pointer_focus();
        }
    }

    /// `xdg_toplevel.set_fullscreen` (`fullscreen: true`, with the client's
    /// optional output hint) or `unset_fullscreen` (`false`).
    ///
    /// Always answered with a configure, as the protocol requires of both
    /// requests ("the compositor will respond by emitting a configure
    /// event") -- including when nothing changed, and including before the
    /// client's first commit, where the answer is what its initial configure
    /// carries.
    pub(super) fn client_fullscreen_request(
        &mut self,
        surface: &ToplevelSurface,
        fullscreen: bool,
        output: Option<WlOutput>,
    ) {
        let Some(id) = self.id_of(surface.wl_surface()) else {
            return;
        };
        if fullscreen {
            self.honour_output_hint(id, output.as_ref());
        }
        let before = self.world.is_fullscreen(id);
        self.world
            .handle_event(Event::FullscreenRequested { id, fullscreen });
        // The already-there fast path the IPC focus actions have: a client
        // repeating a request that changes nothing (already in that state,
        // or refused) would otherwise drive a full `apply` -- an arrange, a
        // configure per window, a render -- as fast as it can write to its
        // socket. The core changed nothing in that case (`set_fullscreen`
        // returns before touching anything), so there is nothing to apply;
        // the configure the protocol requires still goes out below.
        if self.world.is_fullscreen(id) != before {
            self.apply();
        }
        self.answer_fullscreen_request(surface, id, Some(before));
    }

    /// Makes sure the window has been told the core's current answer about
    /// its fullscreen state, after `apply()` has run.
    ///
    /// `apply()` configures visible windows only, so two cases are left for
    /// here: a window that is not visible (on an inactive workspace, scrolled
    /// away, stacked under a fullscreen sibling) gets its state bit set and
    /// sent, and a request the core did not act on at all (`before` equals
    /// the state now: already in that state, or refused) is still answered
    /// with a configure when the client asked for one. `before` is `None`
    /// when nobody asked -- a taskbar changed the window's state -- and then
    /// only a real change is sent.
    pub(super) fn answer_fullscreen_request(
        &self,
        surface: &ToplevelSurface,
        id: WindowId,
        before: Option<bool>,
    ) {
        let now = self.world.is_fullscreen(id);
        // The size moves with the bit, as `apply()` pairs them: an
        // invisible window told it is fullscreen is also told the output's
        // size (and its tiled size when it leaves), so it never renders one
        // state at the other's size. Only when the bit actually flips: a
        // refused request leaves the size alone -- which matters for a
        // window stacked under a fullscreen sibling, whose placement is the
        // sibling's frame, not a size it should ever be configured to. An
        // unplaced window (no output yet) has no placement and keeps its
        // size.
        let size = self
            .world
            .arrange()
            .get(id)
            .map(|placed| placed.rect.size());
        surface.with_pending_state(|state| {
            let flips = state.states.contains(xdg_toplevel::State::Fullscreen) != now;
            if flips && let Some(size) = size {
                state.size = Some((size.w, size.h).into());
            }
            set_fullscreen_state(state, now);
        });
        let sent = surface.send_pending_configure().is_some();
        if !sent && before == Some(now) {
            surface.send_configure();
        }
    }

    /// [`State::answer_fullscreen_request`] for a window nobody on the wire
    /// asked about -- after a taskbar or an IPC `set-fullscreen` changed it,
    /// which may have been while it was invisible, where `apply()` does not
    /// reach. Sends only a real change. Unknown ids are ignored.
    pub(super) fn tell_fullscreen(&self, id: WindowId) {
        if let Some(toplevel) = self.window(id).and_then(Window::toplevel) {
            self.answer_fullscreen_request(toplevel, id, None);
        }
    }

    /// Carries the focused window to the output a fullscreen request named,
    /// when the module doc's conditions hold; does nothing otherwise.
    pub(super) fn honour_output_hint(&mut self, id: WindowId, output: Option<&WlOutput>) {
        let Some(target) = output
            .and_then(Output::from_resource)
            .and_then(|output| self.outputs.id_of(&output))
        else {
            return;
        };
        // The focused window is always on the focused output, so this is the
        // "already there" check as well as the "is it focused" one.
        if self.session_lock.is_locked()
            || self.world.focused_window() != Some(id)
            || self.world.focused_output() == Some(target)
        {
            return;
        }
        self.act(Action::MoveFocusedWindowToOutput(target));
    }

    /// Drops a window's fullscreen when Smithay has discarded its toplevel
    /// state -- which it does when the window unmaps (commits a null
    /// buffer), exactly as xdg-shell says: "all attributes (e.g. title,
    /// state, stacking, ...) are discarded ... the xdg_toplevel returns to
    /// the state it had right after xdg_surface.get_toplevel".
    ///
    /// scoot has no unmap boundary of its own (a window is in the layout from
    /// creation to destruction), so the reset is read off its one visible
    /// trace: the role's `initial_configure_sent` going back to `false`.
    /// While a window is fullscreen that flag cannot be `false` for any other
    /// reason -- entering fullscreen always answers with a configure (see
    /// [`State::client_fullscreen_request`]), and a taskbar or bind can only
    /// reach a window the core has placed, which `apply()` has configured.
    ///
    /// Called from the commit path, before that path sends the initial
    /// configure the reset calls for, so the re-map's configure goes out
    /// already unfullscreened. The fullscreen check comes first: it is a map
    /// lookup, false for nearly every commit, and only a fullscreen window's
    /// commit pays for the role-state lock.
    pub(super) fn discard_fullscreen_if_unmapped(&mut self, id: WindowId) {
        if !self.world.is_fullscreen(id) {
            return;
        }
        let reset = self
            .window(id)
            .and_then(Window::toplevel)
            .is_some_and(|toplevel| !toplevel.is_initial_configure_sent());
        if reset {
            self.world.handle_event(Event::FullscreenRequested {
                id,
                fullscreen: false,
            });
            self.apply();
        }
    }
}
