//! Withholding `wl_surface.frame` from layer surfaces nobody can see.
//!
//! After each rendered frame [`State::render`](super::super::State::render)
//! tells every mapped layer surface on the output to draw again. A client
//! that paces animation by frame callbacks -- an animated wallpaper, the
//! protocol's intended way to stop drawing when unseen -- therefore never
//! learns it is covered: under a fullscreen video it keeps decoding and
//! blending at the video's frame rate, on the CPU under pixman.
//!
//! [`withhold_frame`] answers whether this frame may skip one layer
//! surface's callbacks. A skipped callback is not lost: it stays queued
//! server-side, and the first frame that serves the surface again completes
//! it (that is just what `send_frame` does with a pending callback).
//!
//! # The rule
//!
//! Withhold from a layer surface only when all three hold:
//!
//! 1. **It already committed a buffer.** A client may ask for a callback
//!    before its first attach, and withholding it would stall the very frame
//!    that unsticks it. A surface with no committed buffer is always served.
//! 2. **It is below the opaque cover.** `overlay` is above every window, so
//!    it is never covered and always served. `top` is drawn above windows --
//!    except under a covering fullscreen window, which hides it entirely
//!    (see [`above_windows`](super::above_windows)) -- so it is withheld
//!    only then. `bottom`/`background` are below every window, so any opaque
//!    window spanning the output covers them.
//! 3. **The cover is really opaque, over the whole output, and drawn on
//!    this output.** Either the core's covering fullscreen window, or (for
//!    the layers below windows) a square -- never rounded, whose clip would
//!    cut its corners back to the layer -- window placed on this output (a
//!    frame draws only the windows placed on it; see `output_clip.rs`),
//!    whose root surface carries one opaque region spanning the output and
//!    whose own alpha is exactly 1.0. "Opaque region" here is Smithay's own
//!    answer ([`RendererSurfaceState::opaque_regions`], which already folds
//!    an opaque-format buffer -- YUV included, whose fourccs carry no alpha
//!    -- into a whole-surface region), so this check cannot disagree with
//!    what the frame just drew.
//!
//! Everything else is served, conservatively:
//!
//! - A translucent cover (alpha-format buffer with no opaque region, a
//!   `wp_alpha_modifier_v1` multiplier under 1.0, an X window with
//!   `_NET_WM_WINDOW_OPACITY` under opaque): the layer shows through it.
//! - Opacity composed out of several rects, or out of a subsurface while
//!   the root stays transparent: no single root region spans the output, so
//!   the cover is not recognised. A missed optimisation, never a wrong
//!   pixel.
//! - A cover the compositor cannot see yet (a window `apply()` has not
//!   mapped, an X window XWayland has not paired, a surface whose state is
//!   gone): served.
//!
//! # Cost
//!
//! No heap allocation once the session is up: surface-state reads, one
//! output-geometry lookup, one element-location lookup, integer arithmetic
//! over the root's already-computed opaque rectangles. `window.wl_surface()`
//! is a borrow for an xdg toplevel and a reference-count bump for an X
//! window, never an allocation.
//!
//! # Locking
//!
//! Called from the frame-callback pass while the caller holds the output's
//! layer-map guard. The surface-state locks taken here follow the same order
//! that pass already establishes (`send_frame` takes them under the same
//! guard), so no new lock ordering is introduced.

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::desktop::{LayerSurface, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::alpha_modifier::AlphaModifierSurfaceCachedState;
use smithay::wayland::compositor::with_states;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::compositor::State;
use crate::compositor::output_clip::placed_on;

/// Whether this frame may skip `layer`'s frame callbacks: true only for a
/// surface that already committed a buffer and that no output pixel can
/// show, because opaque window content spans the whole output above it.
/// See the module doc for the rule and its deliberate misses.
pub(in crate::compositor) fn withhold_frame(
    state: &State,
    output: &Output,
    layer: &LayerSurface,
) -> bool {
    // A surface whose client is gone keeps the old unconditional path:
    // `send_frame` on it is a no-op, and the teardown sweep below the call
    // site is what actually collects it.
    if !layer.wl_surface().alive() {
        return false;
    }
    // First-attach unsticks itself through the very callback it asked for
    // before committing anything: only a surface that already committed a
    // buffer is a candidate at all.
    if !has_committed_buffer(layer.wl_surface()) {
        return false;
    }
    match layer.layer() {
        // Above every window, so never covered.
        Layer::Overlay => false,
        // Above windows, hence visible -- unless a fullscreen window covers
        // the output, which hides the whole layer (see `above_windows`).
        Layer::Top => {
            state.covered_by_fullscreen(output) && covering_window_is_opaque(state, output)
        }
        // Below every window: any opaque window spanning the output covers
        // them, fullscreen or not.
        Layer::Background | Layer::Bottom => opaque_window_spans_output(state, output),
    }
}

/// Whether `surface` currently holds a committed buffer: Smithay's own
/// "can be used to check if surface is mapped" answer
/// ([`RendererSurfaceState::buffer`]). A surface that never committed has no
/// renderer state at all; one that unmapped itself with a null commit has
/// state but no buffer. Both read as false, without allocating.
fn has_committed_buffer(surface: &WlSurface) -> bool {
    with_states(surface, |data| {
        data.data_map
            .get::<RendererSurfaceStateUserData>()
            .is_some_and(|user| {
                user.lock()
                    .expect("renderer surface state")
                    .buffer()
                    .is_some()
            })
    })
}

/// Whether the core's fullscreen window covering `output` hides everything
/// below it: its root surface is fully opaque (`alpha` exactly 1.0, its own
/// multiplier -- a translucent subsurface still blends over an opaque root)
/// and carries one opaque region spanning the whole output.
fn covering_window_is_opaque(state: &State, output: &Output) -> bool {
    let Some(id) = state.outputs.id_of(output) else {
        return false;
    };
    let Some(window) = state
        .world
        .fullscreen_on(id)
        .and_then(|id| state.windows.get(&id))
    else {
        return false;
    };
    // A covering fullscreen window is never rounded (the render stack pushes
    // it plain: a rounded clip would cut its corners back to the layer), so
    // unlike the spanning case below this needs no radius guard.
    root_covers_output(state, output, window)
}

/// Whether any square window hides everything below the windows on `output`:
/// the covering fullscreen window (as above), or an unrounded tiled window
/// placed on the output that spans it. Rounded windows are excluded: their
/// clip cuts the corners back to whatever is beneath.
fn opaque_window_spans_output(state: &State, output: &Output) -> bool {
    if state.covered_by_fullscreen(output) && covering_window_is_opaque(state, output) {
        return true;
    }
    if state.appearance.corner_radius > 0 {
        return false;
    }
    // The spanning window is below every `bottom`/`background` surface's
    // cover question, so any match covers all of them; translucent windows
    // above it cannot uncover what its opaque root hides.
    //
    // Only a window placed on this output can cover it. A frame draws the
    // windows placed on it and no other (see `output_clip.rs`), so a window
    // stamped elsewhere covers nothing here however far its surface spills
    // past its slot -- an oversized client buffer overhanging the shared
    // edge, a column scrolled part-way off. Without this a window on the
    // neighbour whose opaque region contains this output's frame would
    // withhold layers nobody drew over.
    let Some(id) = state.outputs.id_of(output) else {
        return false;
    };
    state.space.elements().any(|window| {
        // `element_location` is `None` for a window `apply()` has not
        // mapped: nothing drawn, nothing covered.
        placed_on(window) == Some(id)
            && state.space.element_location(window).is_some()
            && root_covers_output(state, output, window)
    })
}

/// Whether `window`'s root surface is opaque over the whole of `output`: its
/// own alpha is exactly 1.0 and one of Smithay's already-computed opaque
/// regions contains the output rectangle in surface-local coordinates.
///
/// Only the root is read. That is sound in the withholding direction: a
/// translucent subsurface blends over the opaque root, still hiding what is
/// below; content spilling past the root cannot exist where one root region
/// already spans the output. It misses opacity a subsurface composes while
/// the root stays transparent -- served, never starved.
///
///Positions are logical throughout (Smithay keeps opaque regions in
/// surface-logical coordinates, the space in output-logical ones), so scale
/// never enters. All integer arithmetic, no allocation.
fn root_covers_output(state: &State, output: &Output, window: &Window) -> bool {
    let Some(frame) = state.space.output_geometry(output) else {
        return false;
    };
    let Some(mapped) = state.space.element_location(window) else {
        return false;
    };
    // Where the root surface's own origin sits in space coordinates -- the
    // same `mapped - geometry.loc` the render stack draws it at, widened
    // like the comparison below.
    let geometry = window.geometry();
    let origin_x = mapped.x as i64 - geometry.loc.x as i64;
    let origin_y = mapped.y as i64 - geometry.loc.y as i64;
    // An xdg toplevel's own surface (borrowed), or the one XWayland
    // associated with a managed X window (a reference-count bump): the same
    // surface the frame drew. An X window XWayland has not paired yet draws
    // nothing and covers nothing.
    let Some(root) = window.wl_surface() else {
        return false;
    };
    // The root's own multiplier, read exactly the way the render element
    // does (`current().multiplier_f32().unwrap_or(1.0)`): unset or fully
    // opaque reads 1.0, anything under reads below it.
    let alpha = with_states(&root, |data| {
        data.cached_state
            .get::<AlphaModifierSurfaceCachedState>()
            .current()
            .multiplier_f32()
            .unwrap_or(1.0)
    });
    if alpha < 1.0 {
        return false;
    }
    // An X window's `_NET_WM_WINDOW_OPACITY`, as the render stack applies
    // it; 1.0 for an xdg window.
    #[cfg(feature = "xwayland")]
    if let Some(surface) = window.x11_surface() {
        let alpha = surface
            .opacity()
            .map_or(1.0, |opacity| opacity as f32 / u32::MAX as f32);
        if alpha < 1.0 {
            return false;
        }
    }
    with_states(&root, |data| {
        let Some(user) = data.data_map.get::<RendererSurfaceStateUserData>() else {
            return false;
        };
        let state = user.lock().expect("renderer surface state");
        // `None` while unmapped or with no opaque region at all (an
        // alpha-format buffer that declared none): served either way.
        let Some(regions) = state.opaque_regions() else {
            return false;
        };
        // One region spanning the whole output. A union of several never
        // matches: a missed optimisation, never a starved surface.
        //
        // `i64`: the addends mix layout coordinates with client-declared
        // region rectangles, and a wrap here could fake coverage out of a
        // crafted region near `i32::MAX` -- starving a visible surface.
        regions.iter().any(|region| {
            let x = origin_x + region.loc.x as i64;
            let y = origin_y + region.loc.y as i64;
            x <= frame.loc.x as i64
                && y <= frame.loc.y as i64
                && x + region.size.w as i64 >= frame.loc.x as i64 + frame.size.w as i64
                && y + region.size.h as i64 >= frame.loc.y as i64 + frame.size.h as i64
        })
    })
}
