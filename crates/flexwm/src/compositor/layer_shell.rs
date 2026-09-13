//! `wlr-layer-shell-unstable-v1`: the surfaces that aren't windows.
//!
//! A bar, a dock, a wallpaper, a launcher or a notification popup is not an
//! `xdg_toplevel` -- it doesn't belong in the scrolling layout, it anchors
//! itself to screen edges, and it decides whether it sits above or below
//! ordinary windows. This module is everything flexwm needs for those:
//! creating them, keeping them arranged, letting the pointer reach them, and
//! taking whatever they reserve off the area [`flexwm_core`] arranges windows
//! within.
//!
//! ## What does the arranging
//!
//! Smithay's [`LayerMap`] (one per [`Output`], reached with
//! [`layer_map_for_output`]) already implements the protocol's geometry
//! rules -- anchors, margins, the `-1` "don't push me around" exclusive zone,
//! the implied exclusive edge when a surface is anchored to three sides -- and
//! hands back both each surface's rectangle and
//! [`LayerMap::non_exclusive_zone`], the part of the output nothing reserved.
//! flexwm does not reimplement any of that; what it owns is *when* to
//! re-arrange, where the results sit in the render stack, and how that zone
//! reaches the core.
//!
//! ## The zone, and why it can't just be `OutputChanged`
//!
//! `flexwm_core` keeps two rectangles per output now: `area` (the whole
//! screen, which is what `flexwm msg outputs` reports and what a client sees
//! through `wl_output`) and `usable` (what windows are arranged within). A
//! bar shrinks the second, never the first -- pushing the shrunken rectangle
//! through `OutputChanged` instead would have made the compositor report a
//! 1600x970 screen to an agent that asked how big the display is. See
//! [`flexwm_core::Event::OutputUsableAreaChanged`].
//!
//! ## Keyboard focus, deliberately not here yet
//!
//! The protocol's `keyboard_interactivity` (`none`/`on_demand`/`exclusive`)
//! is parsed by Smithay and *ignored* by flexwm in this first pass: a layer
//! surface never takes keyboard focus, so keys always go to the focused
//! window. That is correct for bars, docks, wallpapers and notification
//! daemons (none of which ask for keyboard focus) and is the reason a
//! launcher like `wofi` or `fuzzel` will map and draw here but not yet accept
//! typing. Implementing it means an override in `shell.rs`'s `set_focus`,
//! which owns the compositor's most safety-critical path, so it is its own
//! item -- see `ROADMAP.md`'s backlog entry for the model to implement.
//!
//! ## Guard discipline
//!
//! [`layer_map_for_output`] hands out a `MutexGuard` over per-output state,
//! and its own documentation is explicit that holding two for the same output
//! deadlocks. Every function here therefore takes one, uses it, and drops it
//! before calling anything that could take another -- in particular before
//! [`State::apply`], which re-enters Smithay through configures and focus
//! changes.

use flexwm_core::{Event as CoreEvent, Rect};
use smithay::desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};

use super::State;
use super::headless::OUTPUT_ID;

#[cfg(test)]
mod tests;

/// The layers drawn in front of ordinary windows, front-most first -- the
/// order both the render stack and pointer hit-testing walk them in.
pub(super) const ABOVE_WINDOWS: [Layer; 2] = [Layer::Overlay, Layer::Top];
/// ...and the ones drawn behind them, again front-most first.
pub(super) const BELOW_WINDOWS: [Layer; 2] = [Layer::Bottom, Layer::Background];

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    /// A client asked for a layer surface. It has no size, no buffer and no
    /// committed state yet: everything that decides where it goes arrives
    /// with its first commit, which is also when it gets its first configure
    /// (see [`State::commit_layer_surface`]).
    ///
    /// `output` is the client's request for which screen to appear on. It is
    /// allowed to be `None`, which the protocol defines as "the compositor
    /// chooses"; flexwm has exactly one output, so both cases land on it.
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let requested = output.as_ref().and_then(Output::from_resource);
        let Some(output) = requested.or_else(|| self.output.clone()) else {
            // No output at all. Unreachable in this compositor (`headless::init`
            // creates one before the event loop starts and nothing removes
            // it), but it is a client-facing path, so it closes the surface
            // rather than panicking or leaving it mapped nowhere: `closed` is
            // exactly what the protocol says a compositor sends when a layer
            // surface can no longer be shown, and every layer-shell client
            // already handles it (that is how they survive an unplugged
            // monitor).
            tracing::warn!(%namespace, "no output for a layer surface; closing it");
            surface.send_close();
            return;
        };
        tracing::debug!(%namespace, ?layer, "layer surface created");
        let layer_surface = LayerSurface::new(surface, namespace);
        if let Err(error) = layer_map_for_output(&output).map_layer(&layer_surface) {
            // `AlreadyMapped` is the only variant, and it cannot happen for a
            // surface this handler just created: Smithay refuses a second
            // role on the same `wl_surface` before ever reaching here. Logged
            // rather than unwrapped (anvil unwraps) because a panic in a
            // compositor takes every client's session with it.
            tracing::warn!(%error, "could not map a layer surface");
        }
        // Nothing is drawn yet -- there is no buffer -- but `map_layer` has
        // already arranged, and an arrangement is what the first configure
        // will be built from.
        self.refresh_layer_zone();
    }

    /// The client destroyed its layer surface, or disconnected.
    ///
    /// Smithay calls this from the object's own destructor, so it runs for an
    /// implicit disconnect too. `LayerMap::cleanup` in the render loop is the
    /// second line of defence for the one case this cannot cover: an
    /// implicit destruction whose callback order leaves the `wl_surface`
    /// already dead.
    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(output) = self.output.clone() else {
            return;
        };
        {
            let mut map = layer_map_for_output(&output);
            let Some(layer) = map
                .layers()
                .find(|layer| layer.layer_surface() == &surface)
                .cloned()
            else {
                return;
            };
            map.unmap_layer(&layer);
        }
        // Whatever it was reserving is free again, and the screen still shows
        // it until something redraws.
        self.refresh_layer_zone();
        self.request_render();
    }
}

impl State {
    /// Handles a commit on `surface` if it is a layer surface, and reports
    /// whether it was one.
    ///
    /// Three things happen here, in this order, and the order is the
    /// protocol's rather than a preference: re-arrange (the commit may have
    /// changed anchors, margins, size or exclusive zone), *then* send the
    /// initial configure if this is the first commit -- the spec requires the
    /// initial configure to answer the initial commit, and arranging first is
    /// what makes it carry a size that respects what the client just asked
    /// for -- and only then tell the core what is left over.
    pub(super) fn commit_layer_surface(&mut self, surface: &WlSurface) -> bool {
        let Some(output) = self.output.clone() else {
            return false;
        };
        let found = {
            let mut map = layer_map_for_output(&output);
            let Some(layer) = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned()
            else {
                return false;
            };
            map.arrange();
            if layer.layer_surface().has_pending_changes() {
                // Covers both the initial configure (which
                // `has_pending_changes` reports until it has been sent) and
                // any size `arrange` just changed. `send_pending_configure`
                // is a no-op otherwise, so a bar redrawing at its own frame
                // rate does not get configured on every frame.
                layer.layer_surface().send_pending_configure();
            }
            true
        };
        self.refresh_layer_zone();
        found
    }

    /// Recomputes what layer surfaces have left for windows and tells the
    /// core, if it changed.
    ///
    /// Cheap to call often, which is why every mutation above just calls it:
    /// the comparison against what the core already has means a bar that
    /// repeats the same exclusive zone on every frame costs one rectangle
    /// comparison, not a re-layout.
    pub(super) fn refresh_layer_zone(&mut self) {
        let Some(output) = self.output.clone() else {
            return;
        };
        // The zone is output-local; the core's rectangles are global. One
        // output at (0, 0) makes these identical today, but the translation
        // is what makes that a fact about the setup rather than an
        // assumption baked into the arithmetic.
        let origin = self
            .space
            .output_geometry(&output)
            .map(|geometry| geometry.loc)
            .unwrap_or_default();
        let zone = layer_map_for_output(&output).non_exclusive_zone();
        let area = Rect::new(
            origin.x.saturating_add(zone.loc.x),
            origin.y.saturating_add(zone.loc.y),
            zone.size.w,
            zone.size.h,
        );
        if self.world.usable_areas().first() == Some(&area) {
            return;
        }
        self.world.handle_event(CoreEvent::OutputUsableAreaChanged {
            id: OUTPUT_ID,
            area,
        });
        self.apply();
    }

    /// The layer surface under `position` on one of `layers`, if any, plus
    /// the specific (sub)surface within it and where that sits globally --
    /// the same shape [`State::surface_under`] returns for a window.
    ///
    /// Split by layer group rather than searched all at once because the two
    /// groups sit on opposite sides of the window stack: overlay and top win
    /// over any window, background and bottom lose to every one of them.
    pub(super) fn layer_surface_under(
        &self,
        layers: &[Layer],
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output.as_ref()?;
        let origin = self
            .space
            .output_geometry(output)
            .map(|geometry| geometry.loc)
            .unwrap_or_default();
        let local = position - origin.to_f64();
        let map = layer_map_for_output(output);
        for &layer in layers {
            let Some(found) = map.layer_under(layer, local) else {
                continue;
            };
            // A layer surface the map holds always has a geometry; the
            // `Option` is `layer_geometry`'s answer for one that was never
            // mapped here, which `layer_under` cannot return.
            let Some(geometry) = map.layer_geometry(found) else {
                continue;
            };
            if let Some((surface, surface_location)) =
                found.surface_under(local - geometry.loc.to_f64(), WindowSurfaceType::ALL)
            {
                return Some((surface, (surface_location + geometry.loc + origin).to_f64()));
            }
        }
        None
    }
}
