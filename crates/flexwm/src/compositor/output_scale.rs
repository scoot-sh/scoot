//! Output scaling: `[output] scale`, `wp_fractional_scale_v1` and
//! `wp_viewporter`.
//!
//! # Why this exists
//!
//! A compositor that advertises scale 1.0 on a HiDPI panel tells every client
//! to render at native pixels, so text and widgets come out far smaller than
//! on a compositor that advertises the real scale (niri on the same hardware).
//! This module is what sets the real scale and gives clients the two protocols
//! they need to act on it.
//!
//! # The three moving parts
//!
//! - **`wl_output.scale`** is Smithay's own: an output whose scale is
//!   [`Scale::Fractional`] advertises `ceil(scale)` on `wl_output`, re-sent on
//!   every `change_current_state` and on every client bind. Nothing here
//!   hand-writes that event. `headless.rs`'s `set_mode` is the one caller that
//!   actually applies the configured value.
//! - **`wp_fractional_scale_v1`** is the protocol that carries the *fractional*
//!   value (`1.5`, not just `2`). A client creates a fractional-scale object
//!   per surface and is told a `preferred_scale`; [`FractionalScaleHandler`] is
//!   what sends it.
//! - **`wp_viewporter`** is the protocol a client needs to render a
//!   fractionally-scaled buffer (it sets a destination size in logical
//!   coordinates and lets the compositor scale the buffer). The global is kept
//!   alive by storing [`ViewporterState`] on [`State`]; the
//!   render path already honours its per-surface state because
//!   `on_commit_buffer_handler` calls `ensure_viewport_valid`, so there is no
//!   handler trait to implement here.
//!
//! # Start-up only, and only one output
//!
//! [`State::output_scale`](super::State) is resolved once from
//! the config and never changes. That is why the preferred scale is set in
//! [`FractionalScaleHandler::new_fractional_scale`] -- the one moment a
//! surface's fractional-scale object appears -- rather than re-asserted on
//! every commit the way a compositor that supports live scale changes must.
//! There is exactly one output (see `headless.rs`'s `OUTPUT_ID`), so there is
//! one scale for every surface.
//!
//! `--nested` is explicitly scale-1-only: the host compositor owns the scale
//! of the window flexwm is drawn in, and forward-scaled output would only
//! double-count it. `compositor::run` warns and forces 1.0 there.

use smithay::output::{Output, Scale};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::fractional_scale::{FractionalScaleHandler, with_fractional_scale};
use smithay::wayland::viewporter::ViewporterState;

use super::State;

#[cfg(test)]
mod tests;

/// The smallest `[output] scale` a config may ask for.
///
/// Below 1.0 the output has *more* logical pixels than physical ones, so
/// everything gets smaller -- an unusual but coherent preference (a
/// small-screened machine wanting more content). 0.5 is the common floor
/// (GNOME's "zoom out" scale); anything smaller is a typo or a probe.
pub(super) const MIN_SCALE: f64 = 0.5;

/// The largest `[output] scale` a config may ask for.
///
/// 4.0 covers every panel a user could plausibly have (a 15" 4K panel is 2x,
/// a phone-class 6" 4K panel is 3x) with room to spare. The bound matters
/// beyond taste: the logical output size is `physical / scale` rounded up, so
/// an unbounded scale shrinks the core's usable rectangle toward zero, and
/// `physical / 0` would be infinite. 4.0 keeps a 7680-wide 8K panel at a
/// still-coherent 1920 logical pixels.
pub(super) const MAX_SCALE: f64 = 4.0;

/// The scale a config value actually gets. Pure and separately tested, the
/// same shape as [`Appearance::clamped`](super::decorations::Appearance::clamped)
/// and [`Config::clamp_gap`](flexwm_core::Config::clamp_gap): out-of-range
/// values are brought into range rather than failing startup (on `--tty` this
/// compositor *is* the session -- see `config.rs`'s module doc).
///
/// A non-finite value (TOML can spell `nan` and `inf`) has no meaningful
/// clamp -- `NaN.clamp(..)` is `NaN` -- so it resolves to 1.0, the same value
/// its absence would.
pub(super) fn clamp_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

/// The Smithay scale variant for a resolved scale.
///
/// Exactly 1.0 is `Scale::Integer(1)`, not `Scale::Fractional(1.0)`, so a
/// scale-1 session is indistinguishable from the one this compositor ran
/// before output scaling existed -- same `current_scale()` variant, same
/// `wl_output.scale`, same `fractional_scale()`. Anything else is fractional,
/// which is what lets `wl_output.scale` carry `ceil(scale)` and lets the
/// render path thread the real number through.
pub(super) fn smithay_scale(scale: f64) -> Scale {
    if scale == 1.0 {
        Scale::Integer(1)
    } else {
        Scale::Fractional(scale)
    }
}

/// The output's size in logical pixels: physical mode size divided by the
/// output's scale, rounded *up* -- exactly what Smithay's
/// `Space::output_geometry` returns (verified against the pinned rev's
/// `desktop/space/mod.rs`), so the core and the `Space` can never disagree.
///
/// `ceil`, not truncate or round: a 2560-wide panel at 1.5 is 1706.67 logical
/// pixels, and Smithay rounds that up to 1707. The core must be told the same
/// number the `Space` will lay windows out in, or a window placed at the
/// right edge would be clipped. A 2880-wide panel at 1.5 is exactly 1920 and
/// rounds to itself, so both directions are covered.
///
/// `(0, 0)` when the output has no mode yet, matching the `unwrap_or((0, 0))`
/// every caller already used for a missing mode.
pub(super) fn logical_size(output: &Output) -> (i32, i32) {
    let Some(mode) = output.current_mode() else {
        return (0, 0);
    };
    let size: Size<i32, Logical> = output
        .current_transform()
        .transform_size(mode.size)
        .to_f64()
        .to_logical(output.current_scale().fractional_scale())
        .to_i32_ceil();
    (size.w, size.h)
}

/// Tells `surface`'s `wp_fractional_scale_v1` object, if it has one, what
/// scale to render at.
///
/// A no-op for a surface that never created a fractional-scale object (the
/// common case for clients that only understand `wl_output.scale`): the state
/// is recorded either way, and `with_fractional_scale`'s `set_preferred_scale`
/// only emits when an object exists.
fn set_preferred_scale(surface: &WlSurface, scale: f64) {
    with_states(surface, |states| {
        with_fractional_scale(states, |fractional_scale| {
            fractional_scale.set_preferred_scale(scale);
        });
    });
}

impl FractionalScaleHandler for State {
    /// A client created a `wp_fractional_scale_v1` for `surface`. This is the
    /// one moment it needs to hear the scale: it is fixed for the process's
    /// life (see this module's doc), so there is nothing to re-send later.
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        set_preferred_scale(&surface, self.output_scale);
    }
}

/// Creates the `wp_viewporter` global. The caller stores the returned state on
/// [`State`] so the global stays alive for the process's lifetime; the render
/// path reads each surface's `ViewportCachedState` through
/// `on_commit_buffer_handler` (see this module's doc), so nothing else here
/// has to know the global exists.
pub(super) fn viewporter(display_handle: &DisplayHandle) -> ViewporterState {
    ViewporterState::new::<State>(display_handle)
}
