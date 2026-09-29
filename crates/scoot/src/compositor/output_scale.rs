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
//! # Live-reloadable, and one scale per output
//!
//! [`State::default_scale`](super::State) is resolved from `[output] scale`
//! at startup, and `[[outputs]]` entries override it per output name (see
//! `output_config.rs`); [`State::configured_scale`] is the one function
//! that combines the two. An output's scale is decided when it is created
//! (`headless.rs`'s `create_output`) and when a reload re-decides every
//! output's (`rescale_outputs`), and from then on it lives on the `Output`
//! itself -- [`scale_of`] reads it back, and every reader that means "the
//! scale this is on" resolves an output and asks it, never the default.
//!
//! A surface is told the scale of the output it belongs to:
//!
//! - a **window** (and every subsurface and popup below it): the output the
//!   core places it on -- its workspace's output. A floating window
//!   straddling two outputs has one answer, the same one
//!   `wlr-foreign-toplevel-management`'s `output_enter` gives, not whichever
//!   screen holds more of it. Re-told whenever that output changes (a
//!   cross-output move, an unplugged monitor's windows adopted, a replug
//!   restoring them), from `apply()`'s [`State::refresh_window_scales`] --
//!   which runs only while the outputs disagree on a scale
//!   ([`State::mixed_scales`]), so a session without `[[outputs]]` pays one
//!   `bool` read per `apply()` for it;
//! - a **layer surface**: the output its layer map belongs to, told when
//!   the surface is admitted (`layer_shell.rs`);
//! - a **lock surface**: its own output, told when it is admitted
//!   (`session_lock.rs`);
//! - the **cursor surface**: the output under the pointer, told each time a
//!   client sets it (`wl_pointer.set_cursor`, which every pointer `enter`
//!   is followed by). Not re-told on bare motion: a pointer crossing the
//!   seam inside one client surface keeps the old scale on its cursor
//!   surface until the next `set_cursor` -- a softer cursor for a moment,
//!   never a wrong position (the render path places it per output). A
//!   drag icon is in the same class: told the scale of the output under the
//!   pointer at creation, never re-told while dragged across the seam;
//! - a **subsurface or popup**: whatever its parent was told, copied when
//!   the role is assigned (`handlers.rs`'s `new_subsurface`/`new_popup`,
//!   `layer_shell.rs`'s popup adoption, the input-method popup), and
//!   re-told with its root from then on;
//! - a surface with **no role yet** (`wl_compositor.create_surface`): the
//!   pointer's output, where a new window opens (`shell.rs`) -- the best
//!   guess there is, corrected at role time by the rows above.
//!
//! Every tell goes through Smithay's own per-surface caches
//! (`FractionalScaleState::set_preferred_scale` and `send_surface_state`
//! both send only what differs, verified against the pinned rev), so
//! telling a surface what it already knows is silent on the wire. That is
//! what keeps a session whose outputs all share one scale -- every session
//! without `[[outputs]]` -- byte-identical to the single-scale one before.
//!
//! A reload re-decides every output's scale (`rescale_outputs`) and re-tells
//! every live surface its own output's ([`State::resend_output_scale`]).
//!
//! `--nested` is explicitly scale-1-only: the host compositor owns the scale
//! of the window scoot is drawn in, and forward-scaled output would only
//! double-count it. `compositor::run` warns and forces 1.0 there, and a
//! reload refuses a non-1.0 value rather than applying it.

use std::borrow::Cow;
use std::cell::Cell;

use scoot_core::{Arrangement, OutputId};

use smithay::desktop::{PopupManager, layer_map_for_output};
use smithay::output::{Output, Scale};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Size, Transform};
use smithay::wayland::compositor::{get_children, send_surface_state, with_states};
use smithay::wayland::fractional_scale::{FractionalScaleHandler, with_fractional_scale};
use smithay::wayland::seat::WaylandFocus;
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

/// The range half of [`clamp_scale`], on its own so the config layer can
/// tell an out-of-range value (worth a warning) from one that only needs
/// 1/120 resolution (silent: a sub-percent adjustment the user did not
/// misconfigure, documented in `docs/configuration.md`'s `[output]` entry).
/// `scale` is finite wherever this runs -- `clamp_scale` and `into_scale`
/// both reject the non-finite spellings first.
pub(super) fn clamp_scale_range(scale: f64) -> f64 {
    scale.clamp(MIN_SCALE, MAX_SCALE)
}

/// The scale a config value actually gets. Pure and separately tested, the
/// same shape as [`Appearance::clamped`](super::decorations::Appearance::clamped)
/// and [`Config::clamp_gap`](scoot_core::Config::clamp_gap): out-of-range
/// values are brought into range rather than failing startup (on `--tty` this
/// compositor *is* the session -- see `config.rs`'s module doc).
///
/// A non-finite value (TOML can spell `nan` and `inf`) has no meaningful
/// clamp -- `NaN.clamp(..)` is `NaN` -- so it resolves to 1.0, the same value
/// its absence would.
///
/// The clamped value is then resolved to the nearest multiple of 1/120, so
/// 1.33 becomes 160/120. `wp_fractional_scale_v1.preferred_scale` is a count
/// of 120ths -- the only fractional scale the protocol can say -- while the
/// render path would otherwise draw at the configured value as given, and a
/// client buffer sized `round(logical * 160 / 120)` would be resampled into
/// `logical * 1.33` device pixels. Resolving here makes the one stored value
/// what rendering, `wl_output.scale`'s `ceil`, `preferred_scale` and the
/// `wlr-output-management` report all agree on (Smithay's own
/// `set_preferred_scale` sends `round(scale * 120)`, verified against the
/// pinned rev's `fractional_scale/mod.rs`, so the round trip is exact).
/// Rounding after the clamp cannot leave the range: both ends (0.5 = 60/120,
/// 4.0 = 480/120) are exact multiples already, and the nearest multiple of
/// 1/120 to a value inside the range stays inside it.
pub(super) fn clamp_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        let clamped = clamp_scale_range(scale);
        (clamped * 120.0).round() / 120.0
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

/// The integer scale scoot advertises where only an integer fits:
/// `ceil(scale)`, matching [`Scale::integer_scale`] -- which is what Smithay
/// puts on `wl_output.scale` for the same configured value (verified against
/// the pinned rev's `output.rs`). `1.5` and `2.0` therefore both resolve to
/// `2`, and `1.0` to `1`.
///
/// `scale` is already clamped and finite by the time it reaches here (see
/// [`clamp_scale`]), so the cast cannot see `NaN`/`inf`. Pure and tested so the
/// value sent on `wl_surface.preferred_buffer_scale` and the one Smithay sends
/// on `wl_output.scale` can be proved to agree.
pub(super) fn integer_scale(scale: f64) -> i32 {
    scale.ceil() as i32
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
/// only emits when an object exists. Called at the two bind-time moments
/// below and, on a config reload, for every live surface at once (see
/// [`State::resend_output_scale`]).
pub(super) fn set_preferred_scale(surface: &WlSurface, scale: f64) {
    with_states(surface, |states| {
        with_fractional_scale(states, |fractional_scale| {
            fractional_scale.set_preferred_scale(scale);
        });
    });
}

/// Sends `surface` the integer `preferred_buffer_scale` (and the default
/// `preferred_buffer_transform`) that accompanies [`set_preferred_scale`],
/// through Smithay's own `send_surface_state`.
///
/// This is for protocol completeness, matching wlroots: a v6 client that binds
/// `wp_fractional_scale_v1` is entitled to the integer companion too, and
/// sending it is what a client is otherwise left to infer. It is **not**
/// established that this fixes the reported Ghostty-at-`1.5` symptom — see
/// `docs/backlog/resolved/ghostty-fails-at-1-5-done.md`, which keeps that
/// confirmation open, and the finding that GTK4 does not act on this event
/// while a fractional object exists.
///
/// `send_surface_state` caches the last `(scale, transform)` per surface in
/// that surface's own state and only emits when either differs, so a surface
/// at the fixed scale emits nothing after its first call. It also
/// early-returns for a surface below `wl_compositor` v6, so a client that never
/// opted into the event pays only a version check. The first call on a v6
/// surface allocates the cache entry; every later call does not.
pub(super) fn send_preferred_buffer_scale(surface: &WlSurface, integer_scale: i32) {
    with_states(surface, |states| {
        send_surface_state(surface, states, integer_scale, Transform::Normal);
    });
}

/// The scale `output` runs at: the one place an output's scale lives (see
/// the module doc). `Scale::Integer(1)` -- how exactly 1.0 is spelled, see
/// [`smithay_scale`] -- reads back as `1.0`.
pub(super) fn scale_of(output: &Output) -> f64 {
    output.current_scale().fractional_scale()
}

/// Tells `surface` to render at `scale`: the fractional `preferred_scale`
/// and its integer companion, together, so the two can never disagree for
/// one surface.
///
/// The fractional half is recorded even while the surface has no
/// `wp_fractional_scale_v1` object: Smithay sends the recorded value the
/// moment one is created (verified against the pinned rev's
/// `fractional_scale/mod.rs`), which is what lets a tell made at role time
/// reach an object the client creates after it. Both halves are cached per
/// surface by Smithay and sent only when they change.
pub(super) fn tell_scale(surface: &WlSurface, scale: f64) {
    set_preferred_scale(surface, scale);
    send_preferred_buffer_scale(surface, integer_scale(scale));
}

/// The fractional scale `surface` was last told, if it was told one: what a
/// subsurface or popup inherits from its parent when its role is assigned.
pub(super) fn told_scale(surface: &WlSurface) -> Option<f64> {
    with_states(surface, |states| {
        with_fractional_scale(states, |fractional_scale| {
            fractional_scale.preferred_scale()
        })
    })
}

/// Tells `child` what `parent` was told, when `parent` was told anything --
/// a subsurface or popup takes its parent's scale at role time (see the
/// module doc). A parent never told anything leaves the child with its
/// bind-time guess.
pub(super) fn inherit_scale(child: &WlSurface, parent: &WlSurface) {
    if let Some(scale) = told_scale(parent) {
        tell_scale(child, scale);
    }
}

impl FractionalScaleHandler for State {
    /// A client created a `wp_fractional_scale_v1` for `surface`. Smithay
    /// has already sent it whatever the surface was told before (every
    /// surface is told at `new_surface`, and again when its role picks its
    /// output -- see the module doc), so this only covers a surface that was
    /// somehow never told: the pointer's output, as at `new_surface`.
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        if told_scale(&surface).is_none() {
            let scale = self.pointer_scale();
            set_preferred_scale(&surface, scale);
        }
    }
}

/// Which output a window's scale follows, remembered on the window itself
/// so `apply()` can tell a window that did not change outputs from one that
/// did with one `Cell` read (see [`State::refresh_window_scales`]).
struct ScaleOutput(Cell<Option<OutputId>>);

impl State {
    /// The scale output `name` is configured to run at: its `[[outputs]]`
    /// entry's, else the session default (see `output_config.rs`).
    pub(super) fn configured_scale(&self, name: &str) -> f64 {
        self.output_entries.scale_for(name, self.default_scale)
    }

    /// The scale of the output under the pointer, else of the primary, else
    /// the session default (no output at all -- only a bare test harness):
    /// what a surface with no role yet is told, since a new window opens on
    /// the pointer's output (`shell.rs`).
    pub(super) fn pointer_scale(&self) -> f64 {
        self.pointer_output()
            .or_else(|| self.outputs.primary().cloned())
            .map_or(self.default_scale, |output| scale_of(&output))
    }

    /// Recomputes [`State::mixed_scales`] from the outputs as they are now,
    /// and answers what it was before -- so a caller that just took an
    /// output away can tell a session that stopped mixing scales (whose
    /// windows `apply()` will no longer re-tell) from one that did not.
    /// One pass over the outputs, no allocation; called only where outputs
    /// are added, removed or rescaled.
    pub(super) fn note_output_scales(&mut self) -> bool {
        let mut scales = self.outputs.iter().map(scale_of);
        let first = scales.next();
        let mixed = first.is_some_and(|first| scales.any(|scale| scale != first));
        std::mem::replace(&mut self.mixed_scales, mixed)
    }

    /// Re-tells every window whose output changed since it was last told --
    /// the window's whole tree, popups included -- its new output's scale.
    /// Called by `apply()` with the arrangement it just published, which is
    /// where every cross-output move, adoption and restore ends.
    ///
    /// Only called while the outputs run at more than one scale
    /// ([`State::mixed_scales`]): with one scale everywhere, a window that
    /// changes outputs has nothing new to hear. Its marks may go stale
    /// meanwhile, which only ever costs a silent re-tell later: a mark is
    /// set only when the window is told that output's scale, and an
    /// output's scale moves only through a reload's rescale, which re-tells
    /// and re-marks every window ([`State::resend_output_scale`]). A session
    /// that stops mixing scales (an output removed) is re-told whole by
    /// `remove_output` for the same reason.
    ///
    /// While mixed, a window already told for its output costs one
    /// user-data lookup and a `Cell` read. The first time a window is seen
    /// allocates its user-data slot once. A window with no `wl_surface` yet
    /// (an X window before XWayland associates one) is left unmarked and
    /// picked up by a later `apply()`.
    pub(super) fn refresh_window_scales(&self, arrangement: &Arrangement) {
        for placement in &arrangement.placements {
            let Some(window) = self.windows.get(&placement.id) else {
                continue;
            };
            let told = window
                .user_data()
                .get_or_insert(|| ScaleOutput(Cell::new(None)));
            if told.0.get() == Some(placement.output) {
                continue;
            }
            let Some(output) = self.outputs.get(placement.output) else {
                continue;
            };
            let Some(root) = window.wl_surface() else {
                continue;
            };
            tell_tree_and_popups(&root, scale_of(output));
            told.0.set(Some(placement.output));
        }
    }

    /// Re-tells every surface in the session the scale of its own output:
    /// the fractional `preferred_scale` plus its integer companion, down each
    /// root's whole subsurface tree and across every popup parented to it.
    ///
    /// The roots are every surface a scale object can hang off: each known
    /// window's toplevel (mapped or not -- an unmapped window's client still
    /// holds its objects), each output's layer surfaces, each live lock
    /// surface, and the client cursor surface -- each told its own output's
    /// scale by the rules in the module doc. The cached sends mean a surface
    /// whose output's scale did not move sees nothing on the wire.
    ///
    /// Role-less surfaces are not visited: a surface created but not yet
    /// assigned a toplevel, layer, or other role belongs to none of the
    /// roots above, and Smithay keeps no public global surface list to close
    /// that gap. A reload landing between such a surface's creation and its
    /// role assignment leaves its scale events one reload stale until its
    /// role is assigned (which re-tells it) or the next reload. Real clients
    /// create and role their surfaces in one commit burst, so the gap is a
    /// microsecond misfire, not a steady state.
    ///
    /// Unmapped popups share the gap: a popup tracked before it has a parent
    /// (or before its first commit) waits in `PopupManager::unmapped_popups`,
    /// which `popups_for_surface` never reads, so a reload in that window
    /// misses it and the next reload heals it. Either miss is invisible
    /// while stale: the render path gathers popups through the identical
    /// per-root call -- subsurface-parented ones included -- so whatever the
    /// re-send cannot see is also never drawn.
    ///
    /// Cold reload path only -- nothing here runs per frame or per commit,
    /// so the per-surface walk and the small root `Vec`s are acceptable
    /// where they would not be on the commit path.
    ///
    /// Known churn, documented not fixed: a client that cached the scale at
    /// bind time and never re-reads the events may lag one step behind until
    /// it does (see `docs/backlog/resolved/ghostty-fails-at-1-5-done.md`). The
    /// events are all re-sent; acting on them is the client's half.
    pub(super) fn resend_output_scale(&self) {
        // Owned first, sent after: collecting ends each borrow of `self`
        // before any protocol send, so no borrow of the state outlives into
        // the walk. Small `Vec`s, on the cold reload path only.
        let mut roots: Vec<(WlSurface, f64)> = Vec::new();
        // Every window's root surface -- an xdg toplevel's, or the one
        // XWayland associated with an X window -- at the output the core
        // places it on, marked as told for it so the next `apply()` does not
        // tell it again. A window the core has not placed (none reachable:
        // every window is in the arrangement from `add_window` on) takes the
        // pointer's output, like a surface with no role.
        let arrangement = self.world.arrange();
        let fallback = self.pointer_scale();
        for (id, window) in &self.windows {
            let Some(root) = window.wl_surface().map(Cow::into_owned) else {
                continue;
            };
            let placed = arrangement.get(*id).map(|placement| placement.output);
            let output = placed.and_then(|output| self.outputs.get(output));
            roots.push((root, output.map_or(fallback, scale_of)));
            if let (Some(placed), Some(_)) = (placed, output) {
                window
                    .user_data()
                    .get_or_insert(|| ScaleOutput(Cell::new(None)))
                    .0
                    .set(Some(placed));
            }
        }
        for output in self.outputs.iter() {
            let scale = scale_of(output);
            roots.extend(
                layer_map_for_output(output)
                    .layers()
                    .map(|layer| (layer.wl_surface().clone(), scale)),
            );
        }
        roots.extend(
            self.session_lock
                .live_surfaces()
                .into_iter()
                .map(|(surface, output)| (surface, scale_of(&output))),
        );
        if let Some(cursor) = self.cursor.surface() {
            roots.push((cursor.clone(), fallback));
        }
        for (root, scale) in &roots {
            tell_tree_and_popups(root, *scale);
        }
    }
}

/// [`resend_scale_tree`] for `root` and for every popup parented to it.
fn tell_tree_and_popups(root: &WlSurface, scale: f64) {
    resend_scale_tree(root, scale);
    for (popup, _) in PopupManager::popups_for_surface(root) {
        resend_scale_tree(popup.wl_surface(), scale);
    }
}

/// Re-sends `scale` (and its integer companion) to `surface` and every
/// subsurface below it. Dead subtrees are skipped at their root: a surface
/// whose client is gone keeps its handle until the destroy hook runs, and
/// queueing protocol events at it is harmless but pointless.
fn resend_scale_tree(surface: &WlSurface, scale: f64) {
    if !surface.alive() {
        return;
    }
    tell_scale(surface, scale);
    for child in get_children(surface) {
        resend_scale_tree(&child, scale);
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
