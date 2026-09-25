//! Floating windows: the Wayland half of the core's floating layer.
//!
//! What floating *means* -- where a floating window goes, what focus does,
//! what un-floating puts back -- is layout, and lives in `scoot-core`
//! (`scoot_core::Action::ToggleFloating` has the rules). This module is the
//! wire around it:
//!
//! - **The map-time decision.** A toplevel enters the core as a column the
//!   moment it is created (`add_window`, at `xdg_surface.get_toplevel`),
//!   before the client has said anything about itself. By its first commit
//!   it has: its app id and title, its parent (`set_parent`), its size
//!   limits (`set_min_size`/`set_max_size` are double-buffered, applied by
//!   that commit) and, through `xdg-dialog-v1`, whether it is a dialog. So
//!   [`State::decide_floating_at_map`] runs there, once per window, and asks
//!   `window_rules.rs` for the answer; a window that floats reaches the core
//!   as `scoot_core::Event::FloatingRequested`, which takes it back out of
//!   the strip and restores the strip's focus and scroll exactly (see the
//!   core's `World::float_window`). Nothing re-decides later: a title or a
//!   dialog hint changing after the first commit floats nothing, and a
//!   re-map (a null buffer, then a new first commit) keeps the window where
//!   it is, the way it keeps its column. The toggle is how a user changes it.
//!
//!   The window was configured tiled at its column's size when it was
//!   created (`add_window`'s `apply()` sends the first configure then), so
//!   the decision is followed by a second configure: no size (the client
//!   chooses, the protocol's 0x0) or a rule's size, and no `tiled_*` states.
//!   Both go out in the same flush the client's first commit is answered in;
//!   a client that handles its configures before drawing (every toolkit
//!   does) draws its first frame at the size it chose.
//!
//! - **State out.** `shell.rs`'s `apply()` sends each floating placement its
//!   `requested` size (`None` is 0x0) and the floating layout states (no
//!   `tiled_*`, see `fullscreen::set_layout_states`), whether or not it is
//!   visible: a floating window that has not drawn yet is invisible
//!   *because* it is waiting for exactly that configure.
//!
//! - **Frames in.** A floating window is placed at the size it last drew, so
//!   `observe_frame` reports its frames whatever size it was asked for, and
//!   re-applies the arrangement when one moved or resized it -- only then,
//!   since a floating video commits at its frame rate.
//!
//! - **Moving and resizing** by pointer (a modifier drag, or the client's
//!   own `xdg_toplevel.move`/`.resize`): `floating/grab.rs`.
//!
//! - **The pointer.** A dialog appears where the user just clicked, and
//!   `wl_pointer.button` goes to the surface the pointer last *entered*, not
//!   the one under it. [`State::refresh_floating_cover`] re-derives pointer
//!   focus whenever what floats on screen changed, as
//!   `fullscreen.rs`'s `refresh_fullscreen_cover` does for fullscreen.

use scoot_core::{Arrangement, Event, WindowId};
use smithay::desktop::Window;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogHandler};
use smithay::wayland::shell::xdg::{SurfaceCachedState, ToplevelSurface, XdgToplevelSurfaceData};

use super::State;
use super::window_rules::{Decision, MapSignals};

pub(super) mod grab;
#[cfg(test)]
mod tests;

/// `xdg-dialog-v1`. The dialog hint is read once, at the window's first
/// commit (see the module doc); a hint set or changed after that floats
/// nothing, so there is nothing to do when it changes. Smithay keeps the
/// hint in the toplevel's role data either way.
impl XdgDialogHandler for State {
    fn dialog_hint_changed(&mut self, _toplevel: ToplevelSurface, _hint: ToplevelDialogHint) {}
}

impl State {
    /// Decides at a window's first commit whether it floats, and floats it.
    /// A no-op for every later commit: the window is no longer in
    /// `awaiting_map`, and checking costs an empty-`Vec` test in the steady
    /// state.
    pub(super) fn decide_floating_at_map(&mut self, id: WindowId) {
        let Some(index) = self.awaiting_map.iter().position(|&w| w == id) else {
            return;
        };
        self.awaiting_map.swap_remove(index);
        let Some(decision) = self
            .window(id)
            .and_then(Window::toplevel)
            .and_then(|toplevel| self.map_decision(toplevel))
        else {
            return;
        };
        tracing::debug!(
            ?id,
            float = decision.float,
            reason = ?decision.reason,
            size = ?decision.size,
            "decided whether a window floats"
        );
        if decision.float {
            self.world.handle_event(Event::FloatingRequested {
                id,
                floating: true,
                size: decision.size,
            });
            self.apply();
        }
    }

    /// What `window_rules.rs` answers for this toplevel, from what it has
    /// committed. `None` for a surface whose role data is gone (destroyed
    /// between the lookup and here).
    fn map_decision(&self, toplevel: &ToplevelSurface) -> Option<Decision> {
        let rules = &self.floating_rules;
        with_states(toplevel.wl_surface(), |states| {
            let data = states.data_map.get::<XdgToplevelSurfaceData>()?;
            let attributes = data.lock().ok()?;
            let mut limits = states.cached_state.get::<SurfaceCachedState>();
            let (min, max) = (limits.current().min_size, limits.current().max_size);
            let fixed_size = min.w > 0 && min.h > 0 && min == max;
            let signals = MapSignals {
                app_id: attributes.app_id.as_deref().unwrap_or_default(),
                title: attributes.title.as_deref().unwrap_or_default(),
                dialog: attributes.dialog_hint != ToplevelDialogHint::Unknown,
                parent: attributes.parent.is_some(),
                fixed_size,
            };
            Some(rules.decide(&signals))
        })
    }

    /// Whether what floats on screen changed since the last `apply()`: a
    /// floating window appearing, going, moving, resizing or being raised.
    /// Folds every visible floating placement's id and rect, in stacking
    /// order, into one number -- allocation-free, and a single pass over a
    /// list `apply()` already walks.
    pub(super) fn refresh_floating_cover(&mut self, arrangement: &Arrangement) -> bool {
        let mut cover: u64 = 0;
        for placement in arrangement
            .placements
            .iter()
            .filter(|p| p.floating && p.visible)
        {
            let rect = placement.rect;
            for word in [
                placement.id.0,
                placement.output.0,
                u64::from(rect.x as u32),
                u64::from(rect.y as u32),
                u64::from(rect.w as u32),
                u64::from(rect.h as u32),
            ] {
                // FNV-1a-style mixing, word at a time: order-sensitive, so a
                // raise changes it, and cheap.
                cover = (cover ^ word).wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
        let changed = cover != self.floating_cover;
        self.floating_cover = cover;
        changed
    }
}
