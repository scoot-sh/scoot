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
//! ## Keyboard focus
//!
//! The protocol's `keyboard_interactivity` is honoured here, and
//! [`LayerFocus`] is the whole policy: `none` never takes the keyboard,
//! `exclusive` on `top`/`overlay` takes it the moment the surface maps and
//! keeps it until it unmaps (a launcher, a lock screen), and everything else
//! that wants keys at all is click-to-focus like a window. That last bucket
//! deliberately includes `exclusive` on `bottom`/`background`, which the
//! spec explicitly allows ("for the bottom and background layers, the
//! compositor is allowed to use normal focus semantics") and which is the
//! only sane answer to a wallpaper asking to swallow every keystroke.
//!
//! Two things about *where* that decision is applied, both load-bearing:
//!
//! - It is **derived, not stored**. [`State::layer_keyboard_focus`] re-reads
//!   the layer map every time keyboard focus is refreshed, so "top-most
//!   exclusive surface wins", "focus comes back when it unmaps", and "focus
//!   falls to the next exclusive surface when this one dies" all fall out
//!   rather than needing their own bookkeeping. The one thing stored is
//!   which surface a *click* focused, because nothing else records that --
//!   and it is forgotten again the moment that surface stops wanting the
//!   keyboard, so a click focuses a surface exactly once rather than
//!   outliving the state it was made against.
//! - It overrides **only the keyboard**. `arrangement.focused`, the focus
//!   ring, `set_activated` and `flexwm msg windows`' `focused` flag all keep
//!   tracking the window -- which is the one focus will return to, and is
//!   what an agent driving this compositor is asking about. A layer surface
//!   holding the keyboard is deliberately not a window and does not pretend
//!   to be one.
//!
//! Keybindings are matched in `input.rs`'s `key()` *before* anything is
//! forwarded to the focused client, so they keep working while an exclusive
//! layer surface holds the keyboard. That is what makes it safe to let a
//! full-screen client take every keystroke: the VT-switch bindings, and
//! `quit`, are still reachable if it wedges.
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
use smithay::utils::{IsAlive, Logical, Point};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
    WlrLayerShellHandler, WlrLayerShellState,
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

/// What a layer surface may do with the keyboard right now: the protocol's
/// `keyboard_interactivity`, resolved against the layer it sits on and
/// whether it is actually mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LayerFocus {
    /// Never takes keyboard focus: `none` (the protocol's default, and what
    /// every bar, wallpaper and notification daemon asks for), or a surface
    /// that isn't mapped yet -- or isn't any more.
    Never,
    /// Takes keyboard focus when clicked, and loses it when something else
    /// is clicked: `on_demand` anywhere, and `exclusive` on the two layers
    /// the spec lets a compositor apply normal focus semantics to.
    OnDemand,
    /// Takes keyboard focus as soon as it maps and holds it until it
    /// unmaps: `exclusive` on `top`/`overlay`.
    Exclusive,
}

/// Reads [`LayerFocus`] off a layer surface's committed state.
///
/// One `with_states` lock (Smithay's `cached_state()` returns a `Copy`
/// snapshot), and it reads the *current* state rather than the pending one:
/// `keyboard_interactivity` is double-buffered, so what the client asked for
/// only counts once it has committed.
///
/// `last_acked` is what says "mapped". Smithay's own `pre_commit_hook`
/// maintains it exactly that way (`wlr_layer/mod.rs`: it is set from the
/// acked configure when a buffer is attached and cleared again when one is
/// removed, with its doc comment "Reset to `None` when the surface
/// unmaps"), which is stronger than "is in the layer map": a surface that
/// committed but never attached a buffer, or that unmapped itself by
/// attaching a null buffer, is still in the map. Neither should be able to
/// hold the keyboard while nothing of it is on screen.
fn layer_focus(layer: &LayerSurface) -> LayerFocus {
    let state = layer.cached_state();
    if state.last_acked.is_none() {
        return LayerFocus::Never;
    }
    match state.keyboard_interactivity {
        KeyboardInteractivity::None => LayerFocus::Never,
        KeyboardInteractivity::OnDemand => LayerFocus::OnDemand,
        KeyboardInteractivity::Exclusive => match state.layer {
            Layer::Overlay | Layer::Top => LayerFocus::Exclusive,
            Layer::Bottom | Layer::Background => LayerFocus::OnDemand,
        },
    }
}

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
        // First, before anything that can return early: the surface outlives
        // its role (the client keeps the `wl_surface` -- it may even commit
        // to it again), and Smithay answers that later commit from the reset
        // state its own destruction handler leaves behind. See
        // [`State::neutralize_destroyed_layers`].
        self.layers_awaiting_neutralize
            .push(surface.wl_surface().clone());
        self.mapped_layers.remove(surface.wl_surface());
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
            if self.clicked_layer.as_ref() == Some(&layer) {
                self.clicked_layer = None;
            }
        }
        // Whatever it was reserving is free again, and the screen still shows
        // it until something redraws.
        self.refresh_layer_zone();
        // Explicitly, not via `refresh_layer_zone`: that one returns early
        // when the zone did not move, which is exactly what a launcher
        // reserving nothing does on its way out -- and it is the surface
        // most likely to have been holding the keyboard.
        self.refresh_keyboard_focus();
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
        let (found, touches_keyboard, mapped) = {
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
            let focus = layer_focus(&layer);
            // A click is spent the moment the surface it landed on stops
            // wanting the keyboard -- it committed `none`, or attached a
            // null buffer to unmap itself. Forgotten here rather than left
            // for [`State::layer_keyboard_focus`] to keep ignoring, because
            // that check is re-derived on every refresh while this field is
            // remembered: a surface going `on_demand` -> `none` ->
            // `on_demand` (a bar collapsing and re-opening a search field, a
            // notification daemon closing and re-opening an inline reply)
            // would otherwise take the keyboard straight back on the second
            // transition, with no new click, while the focus ring and
            // `flexwm msg windows` still name the window.
            //
            // This commit is the only place that transition can be seen:
            // `layer_focus` reads committed state, so it changes only when
            // this surface commits -- destruction is `layer_destroyed`'s job
            // and a dead client is `forget_dead_clicked_layer`'s. The
            // `Exclusive` -> `OnDemand` relaxation deliberately *keeps* the
            // click (see [`State::click_layer`]); it never passes through
            // `Never`, so it does not come through here.
            if focus == LayerFocus::Never && self.clicked_layer.as_ref() == Some(&layer) {
                self.clicked_layer = None;
            }
            // Both halves are needed, and the second is the easy one to
            // miss: a surface that *stops* wanting the keyboard reads as
            // `Never` here, and without `keyboard_on_layer` nothing would
            // ever take the focus back off it. That second half is also what
            // makes the clear above reach a refresh: whenever
            // `clicked_layer` is set, the last refresh found *something* on
            // a layer (that surface, or an exclusive one in front of it), so
            // `keyboard_on_layer` is true.
            let mapped = layer.cached_state().last_acked.is_some();
            (
                true,
                focus != LayerFocus::Never || self.keyboard_on_layer,
                mapped,
            )
        };
        if mapped {
            self.mapped_layers.insert(surface.clone());
        } else if self.mapped_layers.remove(surface) {
            // Was mapped, now isn't: Smithay's unmap reset has just put the
            // default back into pending, and whatever commits next -- a
            // second null commit on an already-hidden surface, which wlroots
            // treats as a no-op -- would trip the size validation on it.
            // Neutralized now, while this commit's own validation already
            // passed on the real description. Never fires for a surface that
            // was never mapped, so an initial commit with no description at
            // all still fails loudly with `invalid_size`.
            neutralize_pending_anchor(surface);
        }
        self.refresh_layer_zone();
        if touches_keyboard {
            // Gated so the overwhelmingly common commit -- a bar with
            // `keyboard_interactivity: none` redrawing its clock -- costs
            // one already-loaded `Copy` snapshot and a bool, not a second
            // walk of the layer map.
            self.refresh_keyboard_focus();
        }
        found
    }

    /// Neutralizes every surface whose layer role has just been destroyed.
    ///
    /// Runs from `dispatch.rs`'s post-destruction hook, which is the only
    /// thing that runs *after* Smithay's own destruction handler -- and the
    /// neutralize has to run after it, because that handler resets the
    /// surface's layer state to its default (no anchors, zero size) once
    /// [`WlrLayerShellHandler::layer_destroyed`] has already returned.
    ///
    /// Without this, the client's next commit on the surviving `wl_surface`
    /// trips the role's commit-time size validation on that default ("width
    /// 0 requested without setting left and right anchors") and the client
    /// is killed with `invalid_size`. That is exactly what a launcher does
    /// when it is dismissed -- destroy the role, attach a null buffer,
    /// commit -- so every overlay dismissal killed its whole client
    /// (measured 4/4 with DankMaterialShell; see
    /// `docs/backlog/protocols/dms-enablement-gaps.md`, gap 1).
    ///
    /// Anchoring everything is the neutral choice, not a lie about geometry:
    /// a zero size with both anchors set is the protocol's own "the
    /// compositor chooses" shape, so validation passes, and nothing reads
    /// this state again afterwards -- the surface is in no layer map, so no
    /// `arrange` and no configure ever consults it. Dead surfaces (a client
    /// that died with its role) are skipped: there is no one left to commit.
    pub(super) fn neutralize_destroyed_layers(&mut self) {
        for surface in std::mem::take(&mut self.layers_awaiting_neutralize) {
            neutralize_pending_anchor(&surface);
        }
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
        // Nothing to do when it hasn't moved -- and this is the common case,
        // since every commit a bar makes comes through here while its
        // exclusive zone stays exactly the same.
        if self.world.usable_area(OUTPUT_ID) == Some(area) {
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
        self.layer_hit(layers, position)
            .map(|hit| (hit.surface, hit.location))
    }

    /// The layer surface itself under `position`, for deciding what a click
    /// focuses. Same hit test as [`State::layer_surface_under`] -- the
    /// client's input region, not the bounding box.
    pub(super) fn layer_under(
        &self,
        layers: &[Layer],
        position: Point<f64, Logical>,
    ) -> Option<LayerSurface> {
        self.layer_hit(layers, position).map(|hit| hit.layer)
    }

    /// The one hit test behind both of the above.
    ///
    /// Returns the owning layer surface as well as the (sub)surface actually
    /// under the pointer, because a click needs the former (what takes
    /// keyboard focus is the layer surface, not whichever subsurface of it
    /// was clicked) and pointer motion needs the latter. The extra
    /// [`LayerSurface`] is an `Arc` clone -- two atomics -- against a surface
    /// tree walk that already takes a lock per node, which is why this is one
    /// function rather than two loops: pointer motion runs this at libinput's
    /// rate, and a second walk would have cost far more than the clone.
    fn layer_hit(&self, layers: &[Layer], position: Point<f64, Logical>) -> Option<LayerHit> {
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
                return Some(LayerHit {
                    layer: found.clone(),
                    surface,
                    location: (surface_location + geometry.loc + origin).to_f64(),
                });
            }
        }
        None
    }

    /// The layer surface that should hold the keyboard right now, if one
    /// should at all -- the whole of the focus policy, re-derived rather than
    /// remembered (see this module's doc).
    ///
    /// Walks `overlay` then `top` for an `exclusive` surface, most recently
    /// mapped first, which is the same front-to-back order the render stack
    /// and pointer hit-testing use: the spec leaves "top-most" within one
    /// layer implementation-defined, and answering it the same way everywhere
    /// means the surface that takes the keyboard is the one actually drawn in
    /// front. Failing that, whatever a click focused, if it is still there
    /// and still wants keys.
    pub(super) fn layer_keyboard_focus(&self) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for &layer in &ABOVE_WINDOWS {
            let exclusive = map
                .layers_on(layer)
                .rev()
                .find(|found| layer_focus(found) == LayerFocus::Exclusive);
            if let Some(found) = exclusive {
                return Some(found.wl_surface().clone());
            }
        }
        let clicked = self.clicked_layer.as_ref()?;
        // Membership in the map, not just liveness: `layer_destroyed`
        // unmaps, and a surface that is no longer arranged is no longer on
        // screen to be typed into.
        //
        // The `Never` half is the derived safety net, not the mechanism:
        // `commit_layer_surface` forgets the click on the commit that stops
        // wanting the keyboard, so this only has to answer for the window
        // between that commit and this call. Keeping it means this function
        // still states the whole policy on its own rather than depending on
        // that clear having run first.
        let still_mapped = map.layers().any(|found| found == clicked);
        (still_mapped && layer_focus(clicked) != LayerFocus::Never)
            .then(|| clicked.wl_surface().clone())
    }

    /// Applies a click that landed on `layer`.
    ///
    /// A surface that wants the keyboard takes it; one that does not -- a
    /// plain bar -- takes it away from whatever had it, which is the
    /// protocol's "the user should be able to unfocus this surface" read the
    /// other way round. Neither case touches *window* focus: see
    /// `input.rs`'s `focus_under_pointer`.
    pub(super) fn click_layer(&mut self, layer: &LayerSurface) {
        self.clicked_layer = match layer_focus(layer) {
            LayerFocus::Never => None,
            // Recorded for `exclusive` too, harmlessly: that case is already
            // answered before `clicked_layer` is consulted, but a surface
            // that later relaxes to `on_demand` then keeps the focus its
            // click earned rather than silently dropping it.
            LayerFocus::OnDemand | LayerFocus::Exclusive => Some(layer.clone()),
        };
        self.refresh_keyboard_focus();
    }

    /// Drops a remembered click target whose client has gone.
    ///
    /// Cheap enough to call on every focus refresh (one atomic read), and it
    /// is what keeps a dead client's surface from being held alive by this
    /// field after the compositor has forgotten it everywhere else.
    pub(super) fn forget_dead_clicked_layer(&mut self) {
        if self.clicked_layer.as_ref().is_some_and(|l| !l.alive()) {
            self.clicked_layer = None;
        }
    }
}

/// What [`State::layer_hit`] found: the layer surface, the (sub)surface
/// within it under the pointer, and where that surface sits in the same
/// global coordinates the pointer moves in.
struct LayerHit {
    layer: LayerSurface,
    surface: WlSurface,
    location: Point<f64, Logical>,
}

/// Rewrites a surface's pending layer anchors to all four edges, so a later
/// commit with no new role requests passes the role's commit-time size
/// validation instead of tripping it on the anchorless default Smithay
/// leaves behind (on role destruction, and on unmapping).
///
/// A zero size with both anchors set is the protocol's own "the compositor
/// chooses" shape, so validation passes, and the write is neutral: the
/// surface is unmapped or role-less, in no arrange or configure path, so
/// nothing reads these anchors again until the client describes the surface
/// afresh (which overwrites them) or destroys it. Skips dead surfaces, for
/// the teardown orders where the `wl_surface` is already gone too.
fn neutralize_pending_anchor(surface: &WlSurface) {
    if !surface.alive() {
        return;
    }
    with_states(surface, |states| {
        states
            .cached_state
            .get::<LayerSurfaceCachedState>()
            .pending()
            .anchor = Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
    });
}
