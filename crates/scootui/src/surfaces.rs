//! On-demand surfaces: one layer surface or full-screen toplevel that
//! exists only while there is content to show.
//!
//! The bar keeps one layer surface per output for its whole life. Popups
//! and panels do the opposite: scootnotify's card stack and scootlaunch's
//! panel appear on an `overlay` layer surface that is created when the
//! first card arrives and destroyed with its buffers unmapped when the
//! last one leaves, so no surface is held at idle; scootgreet runs under
//! cage 0.3.1, which has no layer-shell, so it takes a full-screen
//! `xdg_toplevel` instead. This module is the shared half of both: the
//! two surface specs, the pure lifecycle ([`Demand`]), the Wayland glue
//! that creates and destroys each kind, and the named-memfd buffer pool
//! ([`Buffers`]).
//!
//! The app owns its event loop and its globals; this module never sees
//! them. The app's `Dispatch` impls forward protocol events to the
//! [`Demand`] (which answers what to do as an [`Effect`]), and carry the
//! effects out through the glue here, the way `daemon::surfaces` carries
//! out the per-output model's effects in scootbar. The pattern each event
//! needs is one line:
//!
//! - `configure` → `demand.configure(serial, width, height)` → `Ack` → `live.ack(serial)`.
//! - `closed` → `demand.closed()` → `DestroyAndRetry` → destroy the live
//!   objects, clear the buffers, send `wl_display.sync` and call
//!   `demand.retry()` when its callback lands.
//! - a global or output going away → `demand.output_removed()` → `Destroy`
//!   → destroy the live objects and clear the buffers.
//!
//! Object identity still filters stale events first: a `configure` or
//! `closed` for a proxy that is not the live one is dropped, whatever the
//! tracker says, because the compositor may have events in flight for a
//! surface destroyed after `closed` (scootbar's `daemon::surfaces` does
//! the same). The tracker guards the rest: a `configure` on a surface that
//! is not live answers [`Effect::None`].
//!
//! A `wl_surface.preferred_buffer_scale` event (v6) and a fractional-scale
//! `preferred_scale` go to [`Demand::prefer_buffer_scale`] and
//! [`Demand::prefer_fractional`]; [`Demand::scale`] is the scale to draw
//! at. The surface's own output scale is the integer fallback: the app
//! sets it from the output it shows on ([`Demand::set_output_scale`]), or
//! leaves 1 for a null-output surface, whose own scale events arrive
//! anyway.
//!
//! An app that binds `xdg_wm_base` answers its `ping` itself (with `pong`);
//! missing that kills the client, and the base is the app's, not this
//! module's.
//!
//! No `unsafe` here: the mappings behind the buffers are `scootbg-mem`'s.
//! Nothing here allocates on the content path: [`Demand`]'s methods and
//! [`Buffers::take`] keep no heap.

use std::ffi::CStr;
use std::fmt;

use scootbg_mem::{ShmBuffer, ShmError};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{
    Layer, ZwlrLayerShellV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

use crate::density::{Preferred, Scale};
use crate::output::Size;
use crate::outputs::Frame;

#[cfg(test)]
mod tests;

/// An `overlay` layer surface for popups and panels: created on the
/// output the app passes (or none, the compositor's choice), committed
/// with no buffer, sized to `width` x `height` until the app calls
/// [`LayerLive::set_size`].
///
/// The layer is always `overlay`: popups and panels float above windows
/// and reserve no space (`exclusive_zone` 0). What differs between the
/// consumers is the keyboard: none for scootnotify's stack, exclusive for
/// scootlaunch's panel.
#[derive(Debug, Clone, Copy)]
pub struct LayerSpec<'a> {
    /// What the surface is for (`scootnotify`, `scootlaunch`): the
    /// layer-shell namespace.
    pub namespace: &'a str,
    /// Exclusive keyboard (scootlaunch) or none (scootnotify).
    pub exclusive_keyboard: bool,
    /// Where the surface sits: the anchor corners, e.g. top-right for a
    /// notification stack.
    pub anchor: Anchor,
    /// The initial size in logical pixels; [`LayerLive::set_size`] changes
    /// it as the content does.
    pub width: u32,
    pub height: u32,
    /// Margins in logical pixels, in protocol order: top, right, bottom,
    /// left. Saturated to `i32::MAX` at creation (as the bar's are).
    pub margin: [u32; 4],
    /// 0: the surface reserves no space.
    pub exclusive_zone: i32,
}

/// A full-screen `xdg_toplevel`: scootgreet's login screen, where the host
/// (cage 0.3.1) has no layer-shell. Sized by the compositor, which
/// configures it with the output's size.
#[derive(Debug, Clone, Copy)]
pub struct FullscreenSpec<'a> {
    /// The app id (`scootgreet`).
    pub app_id: &'a str,
    /// The title shown while it maps.
    pub title: &'a str,
}

/// Where one on-demand surface stands. The bar's per-output model
/// (`crate::outputs`) retried here in miniature: one surface, the same
/// shape. `Hidden` is not a state: hiding is `hide()`, which is `Absent`
/// with the slate wiped (a shown-again surface is a fresh start with one
/// chance, as the bar's is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    /// No surface: nothing is shown, or nothing has been created yet.
    Absent,
    /// Created and committed with no buffer; no `configure` yet.
    Pending,
    /// The compositor sent a size.
    Configured {
        /// The size as the compositor sent it; 0 on an axis means "yours
        /// to choose" (see [`Demand::configured_size`]).
        requested: Size,
    },
    /// The compositor closed it and it is destroyed; re-created after one
    /// round trip, once.
    Closed,
    /// Closed a second time: given up until the content clears (`hide`),
    /// so a compositor that configures and then closes every surface is
    /// not answered with a new one forever.
    GaveUp,
}

impl Surface {
    /// Whether a live surface object exists for it.
    fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::Configured { .. })
    }
}

/// What carrying out an event does, in the app's glue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Effect {
    None,
    /// Create the surface and commit it with no buffer.
    Create,
    /// Ack this `configure`.
    Ack(u32),
    /// Destroy the surface and its buffers, then send a `wl_display.sync`
    /// whose callback calls [`Demand::retry`].
    DestroyAndRetry,
    /// Destroy the surface and its buffers and stop trying until the
    /// content clears.
    DestroyAndGiveUp,
    /// Destroy the surface and its buffers: the content is gone.
    Destroy,
}

/// Consecutive failed draws at one frame that each earn another try before
/// the frame goes quiet until it changes: the bar's own present-skip retry
/// (`crate::outputs`), whose transient case is the same (a refused buffer
/// under momentary pressure). Three retries cover a couple of
/// still-pressured turns with one to spare; anything drawn again resets
/// the streak, and a new size is a new chance.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// A frame a draw failed at, and how many consecutive draws failed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failed {
    frame: Frame,
    consecutive: u32,
}

/// One on-demand surface as its app sees it: whether content wants one,
/// and what the live one stands at. Pure: no Wayland types, no I/O, so
/// every ordering of events is testable without a compositor.
#[derive(Debug)]
pub struct Demand {
    surface: Surface,
    /// The output's integer scale, the fallback under the surface's own
    /// scale wishes ([`Demand::set_output_scale`]).
    output_scale: u32,
    /// The scales the compositor asked the live surface to be drawn at.
    /// Kept when the surface is re-created: they describe the output, and
    /// the new surface's own events replace them.
    preferred: Preferred,
    /// A `configure` was acked and the surface not committed since.
    ack_uncommitted: bool,
    /// What the live surface was last committed with; `None` while nothing
    /// is attached. Gone with the surface.
    shown: Option<Frame>,
    /// Draws at this frame that failed, and how many in a row.
    failed: Option<Failed>,
    /// The compositor has closed a surface before.
    closed_once: bool,
}

impl Demand {
    /// Empty, no surface: the idle state, holding nothing.
    pub fn new() -> Self {
        Self {
            surface: Surface::Absent,
            output_scale: 1,
            preferred: Preferred::default(),
            ack_uncommitted: false,
            shown: None,
            failed: None,
            closed_once: false,
        }
    }

    /// Whether a live surface object exists.
    pub fn is_live(&self) -> bool {
        self.surface.is_live()
    }

    /// The scale to draw at: the best the compositor said
    /// ([`Preferred::scale`]).
    pub fn scale(&self) -> Scale {
        self.preferred.scale(self.output_scale)
    }

    /// The output's integer scale (`wl_output.scale` of the output shown
    /// on), the fallback under the surface's own scale events. A
    /// null-output surface leaves 1 until its own events arrive. A factor
    /// below 1 is a broken compositor and ignored: the scale divides.
    pub fn set_output_scale(&mut self, factor: i32) {
        if let Ok(scale @ 1..) = u32::try_from(factor) {
            self.output_scale = scale;
        }
    }

    /// The live surface's `wp_fractional_scale_v1.preferred_scale`.
    pub fn prefer_fractional(&mut self, v120: u32) {
        self.preferred.set_fractional(v120);
    }

    /// The live surface's `wl_surface.preferred_buffer_scale`.
    pub fn prefer_buffer_scale(&mut self, factor: i32) {
        self.preferred.set_buffer_scale(factor);
    }

    /// Content arrived: create the surface, unless one is already up or a
    /// retry is already scheduled (a second `show` within one dispatch is
    /// a no-op, as is showing while given up: the content clearing resets
    /// that).
    pub fn show(&mut self) -> Effect {
        if self.surface == Surface::Absent {
            self.surface = Surface::Pending;
            self.ack_uncommitted = false;
            self.shown = None;
            self.failed = None;
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// The content is gone: destroy the surface and its buffers, and
    /// forget whether a surface was ever closed or given up on (a
    /// shown-again surface is a fresh start). Idempotent: hiding what is
    /// already gone is [`Effect::None`].
    pub fn hide(&mut self) -> Effect {
        let live = self.surface.is_live();
        self.surface = Surface::Absent;
        self.ack_uncommitted = false;
        self.shown = None;
        self.failed = None;
        self.closed_once = false;
        if live { Effect::Destroy } else { Effect::None }
    }

    /// A `configure` on the live surface: remember its size, and ack it. A
    /// new size is also a new chance for a draw that failed.
    ///
    /// A side past `i32::MAX` is a compositor's negative size sent as the
    /// protocol's `uint` (the bar met sway 1.12 configuring 4294966528
    /// wide): no surface can be that size, so it is taken as 0, "yours to
    /// choose", and resolved against the content. A `configure` on a
    /// surface that is not live is stale (the glue drops those by object
    /// identity first) and answers [`Effect::None`]: there is nothing to
    /// ack.
    pub fn configure(&mut self, serial: u32, width: u32, height: u32) -> Effect {
        if !self.surface.is_live() {
            return Effect::None;
        }
        let sane = |side: u32| {
            if i32::try_from(side).is_ok() { side } else { 0 }
        };
        self.surface = Surface::Configured {
            requested: Size {
                width: sane(width),
                height: sane(height),
            },
        };
        self.ack_uncommitted = true;
        self.failed = None;
        Effect::Ack(serial)
    }

    /// `closed` on the live surface: retry once, then give up until the
    /// content clears. A `closed` on a surface that is not live is stale.
    pub fn closed(&mut self) -> Effect {
        if !self.surface.is_live() {
            return Effect::None;
        }
        self.shown = None;
        self.ack_uncommitted = false;
        if self.closed_once {
            self.surface = Surface::GaveUp;
            Effect::DestroyAndGiveUp
        } else {
            self.closed_once = true;
            self.surface = Surface::Closed;
            Effect::DestroyAndRetry
        }
    }

    /// The output shown on is gone: destroy the surface and its buffers
    /// with no retry (a retry is for a compositor that dropped the
    /// surface; an output that is gone has nothing to re-create on). A
    /// new output is a fresh start.
    pub fn output_removed(&mut self) -> Effect {
        let live = self.surface.is_live();
        self.surface = Surface::Absent;
        self.ack_uncommitted = false;
        self.shown = None;
        self.failed = None;
        self.closed_once = false;
        if live { Effect::Destroy } else { Effect::None }
    }

    /// The retry callback after a `closed`: the output survived the round
    /// trip, so the surface is created again. Anything else (the content
    /// cleared or the output went meanwhile) is [`Effect::None`].
    pub fn retry(&mut self) -> Effect {
        if self.surface == Surface::Closed {
            self.surface = Surface::Pending;
            self.ack_uncommitted = false;
            self.shown = None;
            self.failed = None;
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// The size the compositor configured, `None` while not configured. A
    /// 0 axis is the compositor not caring: the app sizes that axis to
    /// the content (and, for a layer surface, calls
    /// [`LayerLive::set_size`] so the next `configure` carries it).
    pub fn configured_size(&self) -> Option<Size> {
        match self.surface {
            Surface::Configured { requested } => Some(requested),
            _ => None,
        }
    }

    /// The buffer dimensions for `logical` pixels at the scale to draw at:
    /// `None` when the scaled size overflows (a hostile `configure`), in
    /// which case there is nothing to draw until the size changes. A size
    /// that fits here but not `wl_shm`'s `int32` is refused one layer
    /// down instead, by [`Buffers::take`].
    pub fn buffer_dims(&self, logical: Size) -> Option<(u32, u32)> {
        self.scale().buffer(logical)
    }

    /// What to do now. `size`: the size to draw at (the configured size
    /// with 0 axes resolved against the content, `None` while not
    /// configured); `changed`: the content changed since the surface was
    /// last drawn. A draw that failed past [`MAX_CONSECUTIVE_FAILURES`]
    /// is not tried again until the frame changes, so a persistent
    /// failure (a buffer too large for `wl_shm`, no memory) cannot loop;
    /// a `configure` that changes nothing still needs a commit, so the
    /// ack takes effect.
    pub fn plan(&self, size: Option<Size>, changed: bool) -> Plan {
        let Some(size) = size else {
            return Plan::Nothing;
        };
        let want = Frame {
            size,
            scale: self.scale(),
        };
        if (self.shown != Some(want) || changed) && !self.given_up(want) {
            return Plan::Draw(want);
        }
        if self.ack_uncommitted && self.shown.is_some() {
            Plan::Commit
        } else {
            Plan::Nothing
        }
    }

    /// The surface was committed with `frame` attached. A shown draw ends
    /// any failure streak.
    pub fn drew(&mut self, frame: Frame) {
        if self.surface.is_live() {
            self.shown = Some(frame);
            self.ack_uncommitted = false;
            self.failed = None;
        }
    }

    /// Whether draws at `want` gave up: failed there past
    /// [`MAX_CONSECUTIVE_FAILURES`], so this frame is not tried again
    /// until it changes.
    fn given_up(&self, want: Frame) -> bool {
        matches!(self.failed, Some(failed)
            if failed.frame == want && failed.consecutive > MAX_CONSECUTIVE_FAILURES)
    }

    /// The surface was committed as it was ([`Plan::Commit`]).
    pub fn committed(&mut self) {
        self.ack_uncommitted = false;
    }

    /// Drawing `frame` failed; see the field. Returns whether this starts
    /// a new streak: only then is it said anywhere, so retries stay quiet.
    /// A failure at another frame starts its own streak.
    pub fn draw_failed(&mut self, frame: Frame) -> bool {
        let consecutive = match self.failed {
            Some(failed) if failed.frame == frame => failed.consecutive.saturating_add(1),
            _ => 1,
        };
        self.failed = Some(Failed { frame, consecutive });
        consecutive == 1
    }
}

impl Default for Demand {
    fn default() -> Self {
        Self::new()
    }
}

/// What the app must do for its surface now (see [`Demand::plan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Nothing,
    /// Commit the surface as it is, so an acked `configure` takes effect.
    Commit,
    /// Draw this frame and commit it.
    Draw(Frame),
}

/// Why a surface could not be created: a required global the compositor
/// does not offer. Said loudly by the app; never a silent fallback (a
/// popup that is not a popup is a lost notification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// The compositor has no `zwlr_layer_shell_v1`: not a layer-shell
    /// host (cage), so only the full-screen toplevel works there.
    NoLayerShell,
    /// The compositor has no `xdg_wm_base`: nothing toplevel can be said
    /// to.
    NoXdgWmBase,
}

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLayerShell => {
                f.write_str("the compositor has no zwlr_layer_shell_v1, so no layer surface")
            }
            Self::NoXdgWmBase => {
                f.write_str("the compositor has no xdg_wm_base, so no toplevel surface")
            }
        }
    }
}

impl std::error::Error for CreateError {}

/// The globals an on-demand surface is created from: the app passes its
/// own bound singletons, `None` where it did not bind one (or the
/// compositor lacks it). Only what the created kind needs must be `Some`.
#[derive(Debug)]
pub struct Globals<'a> {
    pub compositor: &'a WlCompositor,
    pub layer_shell: Option<&'a ZwlrLayerShellV1>,
    pub xdg_wm_base: Option<&'a XdgWmBase>,
    pub viewporter: Option<&'a WpViewporter>,
    /// Bound only with a viewporter, the one way to act on a fraction.
    pub fractional_scale: Option<&'a WpFractionalScaleManagerV1>,
}

/// A size for a protocol `int`. Every size here is bounded far below
/// `i32::MAX` (a buffer's sides by `ShmBuffer`'s own `i32` check, the rest
/// by the compositor that sent them as sizes); saturate rather than wrap
/// all the same.
fn logical(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// A layer surface and what drawing on it needs: the role object, the
/// surface it is on, and its fractional-scale object and viewport where
/// the compositor has them (bound before the first commit, so the scale
/// arrives before the first `configure`). The app attaches its buffers
/// and commits.
#[derive(Debug)]
pub struct LayerLive {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    /// What the surface's persistent state was last set to, so a draw
    /// sends only what changes (as the bar's does).
    buffer_scale: u32,
    destination: Option<Size>,
}

impl LayerLive {
    /// Creates the layer surface for `spec` on `output` (or none, the
    /// compositor's choice) and commits it with no buffer, so the
    /// compositor reserves its place and answers with a `configure`.
    /// `data` is the user data on the surface and role objects, for the
    /// app's `Dispatch` impls.
    pub fn create<S, U>(
        globals: &Globals<'_>,
        output: Option<&WlOutput>,
        spec: &LayerSpec<'_>,
        qh: &QueueHandle<S>,
        data: U,
    ) -> Result<Self, CreateError>
    where
        S: Dispatch<WlSurface, U>
            + Dispatch<ZwlrLayerSurfaceV1, U>
            + Dispatch<WpViewport, U>
            + Dispatch<WpFractionalScaleV1, U>
            + 'static,
        U: Clone + Send + Sync + 'static,
    {
        let Some(layer_shell) = globals.layer_shell else {
            return Err(CreateError::NoLayerShell);
        };
        let surface = globals.compositor.create_surface(qh, data.clone());
        let (viewport, fractional) = {
            let viewport = globals
                .viewporter
                .as_ref()
                .map(|viewporter| viewporter.get_viewport(&surface, qh, data.clone()));
            let fractional = globals
                .fractional_scale
                .as_ref()
                .map(|manager| manager.get_fractional_scale(&surface, qh, data.clone()));
            (viewport, fractional)
        };
        let layer = layer_shell.get_layer_surface(
            &surface,
            output,
            Layer::Overlay,
            spec.namespace.to_owned(),
            qh,
            data,
        );
        layer.set_anchor(spec.anchor);
        layer.set_size(spec.width, spec.height);
        let [top, right, bottom, left] = spec.margin;
        let side = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        layer.set_margin(side(top), side(right), side(bottom), side(left));
        layer.set_exclusive_zone(spec.exclusive_zone);
        layer.set_keyboard_interactivity(if spec.exclusive_keyboard {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        surface.commit();
        Ok(Self {
            surface,
            layer,
            fractional,
            viewport,
            buffer_scale: 1,
            destination: None,
        })
    }

    /// The surface: what the app attaches to and commits.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// The role object: what the app's `Dispatch` matches a `configure`
    /// or `closed` against (stale events for a destroyed surface are
    /// dropped by that comparison).
    pub fn role(&self) -> &ZwlrLayerSurfaceV1 {
        &self.layer
    }

    /// Resizes the surface: the compositor answers with a new `configure`.
    pub fn set_size(&self, width: u32, height: u32) {
        self.layer.set_size(width, height);
    }

    /// Acks a `configure`.
    pub fn ack(&self, serial: u32) {
        self.layer.ack_configure(serial);
    }

    /// Sets the viewport's destination or the buffer scale for a draw at
    /// `scale` of a surface of `size` (logical): with a viewporter every
    /// buffer attaches at buffer scale 1 and the viewport sizes it (the
    /// surface's logical size); without one the scale is an integer and
    /// goes to `set_buffer_scale`. What the bar's own draw does.
    pub fn prepare(&mut self, size: Size, scale: Scale) {
        let scale = match self.viewport {
            Some(_) => scale,
            None => Scale::Integer(scale.integer()),
        };
        match &self.viewport {
            Some(viewport) => {
                if self.destination != Some(size) {
                    viewport.set_destination(logical(size.width), logical(size.height));
                    self.destination = Some(size);
                }
                set_buffer_scale(&self.surface, &mut self.buffer_scale, 1);
            }
            None => set_buffer_scale(&self.surface, &mut self.buffer_scale, scale.integer()),
        }
    }

    /// The fractional-scale object, the viewport and the role first, then
    /// the surface, as the protocols ask. Safe while the compositor holds
    /// a buffer: the buffers die with [`Buffers::clear`], and the
    /// compositor's mapping of the memfd keeps whole under the seals.
    pub fn destroy(self) {
        if let Some(fractional) = self.fractional {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport {
            viewport.destroy();
        }
        self.layer.destroy();
        self.surface.destroy();
    }
}

/// A full-screen `xdg_toplevel` and what drawing on it needs. Sized by
/// the compositor: the app never sets a size.
#[derive(Debug)]
pub struct ToplevelLive {
    surface: WlSurface,
    xdg_surface: XdgSurface,
    toplevel: XdgToplevel,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    buffer_scale: u32,
    destination: Option<Size>,
}

impl ToplevelLive {
    /// Creates the toplevel for `spec`, full-screen on `output` (or the
    /// compositor's choice with none), and commits it with no buffer, so
    /// the compositor answers with a `configure` carrying the output's
    /// size. `data` is the user data on the surface, the `xdg_surface`
    /// and the toplevel, for the app's `Dispatch` impls.
    pub fn create<S, U>(
        globals: &Globals<'_>,
        output: Option<&WlOutput>,
        spec: &FullscreenSpec<'_>,
        qh: &QueueHandle<S>,
        data: U,
    ) -> Result<Self, CreateError>
    where
        S: Dispatch<WlSurface, U>
            + Dispatch<XdgSurface, U>
            + Dispatch<XdgToplevel, U>
            + Dispatch<WpViewport, U>
            + Dispatch<WpFractionalScaleV1, U>
            + 'static,
        U: Clone + Send + Sync + 'static,
    {
        let Some(wm_base) = globals.xdg_wm_base else {
            return Err(CreateError::NoXdgWmBase);
        };
        let surface = globals.compositor.create_surface(qh, data.clone());
        let (viewport, fractional) = {
            let viewport = globals
                .viewporter
                .as_ref()
                .map(|viewporter| viewporter.get_viewport(&surface, qh, data.clone()));
            let fractional = globals
                .fractional_scale
                .as_ref()
                .map(|manager| manager.get_fractional_scale(&surface, qh, data.clone()));
            (viewport, fractional)
        };
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, data.clone());
        let toplevel = xdg_surface.get_toplevel(qh, data);
        toplevel.set_app_id(spec.app_id.to_owned());
        if !spec.title.is_empty() {
            toplevel.set_title(spec.title.to_owned());
        }
        toplevel.set_fullscreen(output);
        surface.commit();
        Ok(Self {
            surface,
            xdg_surface,
            toplevel,
            fractional,
            viewport,
            buffer_scale: 1,
            destination: None,
        })
    }

    /// The surface: what the app attaches to and commits.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// The `xdg_surface`: what the app's `Dispatch` acks a `configure` on
    /// and matches toplevel events against (stale events for a destroyed
    /// surface are dropped by that comparison).
    pub fn xdg_surface(&self) -> &XdgSurface {
        &self.xdg_surface
    }

    /// The toplevel: what the app's `Dispatch` reads the fullscreen
    /// `configure` size from.
    pub fn toplevel(&self) -> &XdgToplevel {
        &self.toplevel
    }

    /// Acks a `configure` (the `xdg_surface`'s serial).
    pub fn ack(&self, serial: u32) {
        self.xdg_surface.ack_configure(serial);
    }

    /// Sets the viewport's destination or the buffer scale for a draw, as
    /// [`LayerLive::prepare`] does.
    pub fn prepare(&mut self, size: Size, scale: Scale) {
        let scale = match self.viewport {
            Some(_) => scale,
            None => Scale::Integer(scale.integer()),
        };
        match &self.viewport {
            Some(viewport) => {
                if self.destination != Some(size) {
                    viewport.set_destination(logical(size.width), logical(size.height));
                    self.destination = Some(size);
                }
                set_buffer_scale(&self.surface, &mut self.buffer_scale, 1);
            }
            None => set_buffer_scale(&self.surface, &mut self.buffer_scale, scale.integer()),
        }
    }

    /// The toplevel, the `xdg_surface`, the fractional-scale object and
    /// the viewport first, then the surface. Safe while the compositor
    /// holds a buffer, as [`LayerLive::destroy`] is.
    pub fn destroy(self) {
        self.toplevel.destroy();
        self.xdg_surface.destroy();
        if let Some(fractional) = self.fractional {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport {
            viewport.destroy();
        }
        self.surface.destroy();
    }
}

/// `wl_surface.set_buffer_scale`, sent only when it changes (`current` is
/// what the surface has).
fn set_buffer_scale(surface: &WlSurface, current: &mut u32, scale: u32) {
    if *current != scale {
        surface.set_buffer_scale(logical(scale));
        *current = scale;
    }
}

/// What [`Buffers::take`] needs to know about a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotState {
    pub dims: (u32, u32),
    /// Attached and not released since: the compositor may be reading.
    pub held: bool,
    /// The `wl_shm` format the slot's pixels are in.
    pub format: wl_shm::Format,
}

/// The slot to draw `dims` in (a buffer of `dims` in `format`), given
/// each slot's state (`None`: empty): a free one already of that size and
/// format, else an empty one, else any free one (replaced at the new size
/// and format); `None` when the compositor holds them all. The bar's own
/// pooled double buffer (`daemon::canvas`) in miniature, keyed on the
/// format as the popup's pool is on its rounding.
pub fn pick(
    slots: impl Iterator<Item = Option<SlotState>> + Clone,
    dims: (u32, u32),
    format: wl_shm::Format,
) -> Option<usize> {
    let free = |slot: &Option<SlotState>| slot.is_some_and(|s| !s.held);
    let find = |wanted: &dyn Fn(&Option<SlotState>) -> bool| slots.clone().position(|s| wanted(&s));
    find(&|s| free(s) && s.is_some_and(|s| s.dims == dims && s.format == format))
        .or_else(|| find(&|s| s.is_none()))
        .or_else(|| find(&free))
}

/// One buffer: its memory, the pool over it, the `wl_buffer`, and
/// whether the compositor may be reading it. Handed out by
/// [`Buffers::take`]; the app draws its pixels, attaches the buffer,
/// commits the surface, and marks it committed.
#[derive(Debug)]
pub struct Slot {
    shm: ShmBuffer,
    pool: WlShmPool,
    buffer: WlBuffer,
    state: SlotState,
}

impl Slot {
    /// The pixels, `stride * height` bytes, row by row. Written only
    /// while the compositor does not hold the buffer (fresh from
    /// [`Buffers::take`], or released since): writing while it is held
    /// tears what is on screen.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.shm.pixels_mut()
    }

    /// The `wl_buffer`: what the app attaches to its surface.
    pub fn buffer(&self) -> &WlBuffer {
        &self.buffer
    }

    /// The buffer was attached and committed: the compositor may be
    /// reading, so the slot is not handed out again until its release.
    pub fn mark_committed(&mut self) {
        self.state.held = true;
    }

    /// Destroys the buffer and the pool; the mapping goes with `shm`.
    /// Safe while the compositor holds the buffer, as
    /// [`Buffers::clear`] is.
    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// One output's buffers, in miniature: up to two sealed-memfd `wl_shm`
/// buffers, each with its own pool, made on first need and reused in
/// place. A buffer is written only while the compositor does not hold it
/// (never attached, or released since), so a redraw reuses a released
/// buffer, else takes the other slot, else waits for a release (the
/// release wakes the loop, which draws then). The memfd is closed as soon
/// as its pool exists and is named for the app, so `/proc/PID/maps`
/// shows whose a leaked buffer is. Destroyed with the surface
/// ([`Buffers::clear`]), unmapping every page: at idle the app holds no
/// surface and no buffer.
///
/// The policy is [`pick`]'s; the shape is the popup's pool
/// (`scootbar/src/daemon/popup/buffers.rs`), which stays the bar's own.
#[derive(Debug)]
pub struct Buffers {
    slots: [Option<Slot>; 2],
    /// The memfd name every buffer is created with (`scootnotify`,
    /// `scootlaunch`, `scootgreet`): a `c"..."` literal, costing nothing.
    name: &'static CStr,
}

impl Buffers {
    /// Empty, holding nothing: no surface, no buffer.
    pub fn new(name: &'static CStr) -> Self {
        Self {
            slots: [None, None],
            name,
        }
    }

    /// A slot to draw `dims` in (a buffer of `dims` in `format`): a free
    /// one already of that size and format, else a new one in an empty
    /// place, else a free one of another size or format replaced;
    /// `None` while the compositor holds every one (the release wakes the
    /// loop, which draws then). `data` is the user data on the created
    /// `wl_buffer`, for the app's `Dispatch`.
    pub fn take<S, U>(
        &mut self,
        shm: &WlShm,
        qh: &QueueHandle<S>,
        dims: (u32, u32),
        format: wl_shm::Format,
        data: U,
    ) -> Result<Option<&mut Slot>, ShmError>
    where
        S: Dispatch<WlBuffer, U> + Dispatch<WlShmPool, ()> + 'static,
        U: Clone + Send + Sync + 'static,
    {
        let Some(index) = pick(
            self.slots.iter().map(|s| s.as_ref().map(|s| s.state)),
            dims,
            format,
        ) else {
            return Ok(None);
        };
        let Some(entry) = self.slots.get_mut(index) else {
            return Ok(None);
        };
        // A free slot of the wrong size or format is replaced.
        if entry
            .as_ref()
            .is_some_and(|slot| slot.state.dims != dims || slot.state.format != format)
            && let Some(stale) = entry.take()
        {
            stale.destroy();
        }
        if entry.is_none() {
            let mut mem = ShmBuffer::with_name(dims.0, dims.1, self.name)?;
            let geometry = mem.geometry();
            let Some(fd) = mem.fd() else {
                // A fresh buffer has its fd; this cannot happen.
                return Err(ShmError::Io(std::io::Error::other(
                    "the buffer's memfd is already closed",
                )));
            };
            let pool = shm.create_pool(fd, geometry.len, qh, ());
            // The queued request holds its own copy of the fd until it is
            // sent, and the compositor its own after that.
            mem.close_fd();
            let buffer = pool.create_buffer(
                0,
                geometry.width,
                geometry.height,
                geometry.stride,
                format,
                qh,
                data,
            );
            *entry = Some(Slot {
                shm: mem,
                pool,
                buffer,
                state: SlotState {
                    dims,
                    held: false,
                    format,
                },
            });
        }
        Ok(entry.as_mut())
    }

    /// `wl_buffer.release` for `buffer`: its slot may be written again. A
    /// release for a buffer destroyed with the surface (or never handed
    /// out) is not one of the slots and is ignored: the compositor may
    /// release a buffer after the surface it was attached to is gone.
    pub fn released(&mut self, buffer: &WlBuffer) {
        for slot in self.slots.iter_mut().flatten() {
            if &slot.buffer == buffer {
                slot.state.held = false;
            }
        }
    }

    /// Destroys every buffer and unmaps every page. Safe while the
    /// compositor holds one (a surface destroyed under it): it has its
    /// own mapping of the memfd, which the seals keep whole, and this
    /// side never writes the pages again.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            if let Some(slot) = slot.take() {
                slot.destroy();
            }
        }
    }
}
