//! X windows at a scaled output: the X server draws in its own pixels at
//! an integer scale, and Smithay converts every coordinate that crosses
//! between X and the layout.
//!
//! # The mechanism
//!
//! XWayland's Wayland client carries a *client scale*
//! (`CompositorClientState::set_client_scale`): a factor between scoot's
//! logical pixels and the pixels that client speaks in. With it set to `S`,
//! Smithay (at the pinned fork rev) converts in every place X and the
//! layout meet -- the `wl_output`/`xdg_output` sizes XWayland builds its X
//! screen from (so the X screen is `S` times the logical layout), every
//! `wl_pointer`/`wl_touch` position and relative motion it is sent,
//! every buffer, damage and input region it commits (so its buffers are
//! read as scale `S`), and in the window manager: configure requests and
//! notifies, the geometry an X window starts at, `WM_NORMAL_HINTS` sizes,
//! `_GTK_FRAME_EXTENTS`, `_NET_WM_OPAQUE_REGION`, and the root position in
//! every `XdndPosition`. The XDND proxy covers the X screen in X pixels
//! (`RandrScreenChangeNotify` keeps it covering it). So everything scoot
//! reads off an `X11Surface` (`last_configure`, `bbox`, `geometry`, the
//! hints) is logical already, and everything it hands one is converted on
//! the way out -- nothing in scoot multiplies or divides by `S` except the
//! `INT16`/`CARD16` bounds in `manage.rs`, which are X-side limits and so
//! shrink by `S` in logical pixels.
//!
//! One scale event scoot sends itself is not converted:
//! `wl_surface.preferred_buffer_scale` (`handlers.rs`'s `new_surface` and a
//! reload's re-send) carries the output's integer, not that divided by the
//! client scale as Smithay's `wl_output.scale` is. It never reaches
//! XWayland: Smithay sends it only to a `wl_surface` of version 6 or above,
//! and XWayland 24.1 binds version 4 (measured). An XWayland that bound 6
//! and acted on it would draw at twice the scale; that is the place to
//! divide.
//!
//! The client scale is set on XWayland's client the moment it is spawned,
//! before the first dispatch, so every global it binds -- the outputs above
//! all -- is bound at the right scale from the start. (Setting it at `READY`
//! instead, as Smithay's anvil does, is too late: XWayland has sized its X
//! screen from the outputs by then, and would have to be re-sent them.)
//!
//! # Which scale: `ceil([output] scale)`, the largest over the outputs
//!
//! X toolkits scale only by integers (GTK's window scale, Qt's and Java's
//! device ratio off it), so `S` is an integer, and it is the one scoot
//! already advertises on `wl_output.scale` -- the largest one, when
//! `[[outputs]]` gives outputs different scales. The X server has one
//! scale for every X window on every output, so outputs that disagree
//! cannot all be matched; the largest keeps X apps sharp on the densest
//! screen and has the renderer scale them down on the others (the same
//! down-only direction a fractional scale already takes, below), where the
//! smallest would blur them on the dense one. With every output at one
//! scale -- every session without `[[outputs]]` -- that is simply its
//! integer, exactly what it was. At an
//! integer output scale the X server draws at exactly the output's
//! resolution -- one X pixel per physical pixel, sharp. At a fractional one
//! (1.5) X draws at the integer above (2) and the renderer scales it down
//! to the output, like any Wayland client that renders at `ceil`: sharp
//! toolkits sized for 2 are right-sized at 1.5, where drawing at 1 and
//! scaling up blurred every X app. The alternatives, and what they cost,
//! are in `docs/backlog/resolved/xwayland-scale-aware-done.md`; in short:
//! the exact fractional scale would put X windows on non-integer logical
//! positions (every conversion rounds, and a toolkit would still draw at
//! 1 or 2), and `floor` is the old blur at every scale below 2. Below 1
//! the integer is 1 -- X at scale 1, as before.
//!
//! # The bound: the whole layout in X's coordinates
//!
//! The X screen is the layout times `S`, and X speaks 16-bit coordinates:
//! positions are `INT16`, and XWayland keeps the screen size in the
//! server's signed-`short` width and height. A layout wider than 32767 X
//! pixels leaves every X window past that edge unaddressable -- the wire
//! clamp in `manage.rs` pins it at the edge in X while scoot draws it where
//! it is placed, `QueryPointer` over it answers a wrapped root position and
//! no window, and whatever X does in root coordinates there (a menu or
//! tooltip it places, an XDND position, `USPosition`) lands wrong. Eight
//! 3840-pixel outputs at 1.25 are 24576 logical pixels: 49152 X pixels at
//! 2 (measured, before this bound existed).
//!
//! So `S` is the largest integer from 1 up to `ceil([output] scale)` at
//! which the whole layout fits -- every output's logical right and bottom
//! edge times `S` at most 32767, its left and top (an origin can be
//! negative) times `S` at least -32768: [`fit_x_scale`]. A layout that fits
//! at its integer is unaffected; a huge one draws X at the largest scale
//! that fits (blurrier, the way X at 1 was, but addressable), logged at
//! info once per change. One that does not fit even at 1 keeps 1 -- the
//! limit X always had -- with a warning. The layout moves at runtime, so
//! [`State::refit_xwayland`] re-chooses at every change to it: an output
//! added, removed or resized, and a reload of the scale.
//!
//! # Telling toolkits: XSETTINGS
//!
//! The X server drawing at `S` only gives an X app `S` times the pixels;
//! the app has to draw bigger in them, or it comes out `S` times smaller.
//! Toolkits learn the scale from XSETTINGS -- what a GNOME session's
//! settings daemon publishes, read from the window manager's
//! `_XSETTINGS_S0` (GTK measured to follow them live; Qt 6 and Java
//! document reading them, unmeasured; Qt 5 takes only the font DPI unless
//! the app enables high-DPI scaling) -- through Smithay's
//! `X11Wm::set_xsettings`: [`toolkit_settings`]. An XSETTINGS daemon that
//! takes `_XSETTINGS_S0` over replaces them. Not environment variables:
//! `GDK_SCALE` pins GTK against the live setting (so a reload could never
//! reach it), and `QT_SCALE_FACTOR` would scale a Qt app that runs on
//! Wayland a second time -- `State::spawn` cannot know which a child is.
//! An app that reads none of these (bare Xlib: `xterm`'s bitmap fonts,
//! `xclock`, Wine, Steam's own UI) draws at scale 1 in X pixels and is `S`
//! times smaller -- sharp, but small; the trade-off every compositor that
//! draws X natively makes. The `Xft.dpi` X resource is not written
//! (Smithay's window manager has no way to write the root's
//! `RESOURCE_MANAGER`, and a second X connection from the compositor could
//! deadlock against its own server); an app that reads only that takes
//! `xrdb -merge` from the user.
//!
//! # A scale-1 session is untouched
//!
//! At `S = 1` from startup the client scale is set to the 1.0 it already
//! was and no XSETTINGS entry is written (a change back to 1 does write
//! them, at 1, so the old ones do not linger): nothing an X client or
//! toolkit reads differs from the session before any of this existed
//! (pinned: `tests/scale.rs`'s `a_scale_1_session_is_untouched`). The same
//! holds for a session whose layout bounds `S` to 1 from startup. A change
//! that leaves `S` where it was sends X nothing.
//!
//! # A change that moves `S`
//!
//! [`State::refit_xwayland`] runs once the new layout is in place: it sets
//! the new client scale, re-advertises every output (`change_current_state`
//! with nothing changed, which Smithay turns into a re-send of exactly what
//! the client scale moved -- to XWayland's `xdg_output`s their logical
//! size and position, to its `wl_output`s the scale -- so XWayland resizes
//! its X screen to the new X pixels; every other client gets a bare
//! `wl_output.done`), publishes the new toolkit settings -- GTK rescales
//! live off them -- and reconfigures every managed X window into its new X
//! pixels at its unchanged logical place. The layout change itself was
//! advertised a moment before in the old client scale, so XWayland briefly
//! holds a screen sized for that (both are queued in the same dispatch, and
//! it settles on the second); an X client that reads the screen size in
//! that instant reads the first. An app that read the scale once at
//! startup keeps drawing at its old scale in the new pixels (smaller or
//! bigger by the ratio) until it is restarted; its geometry is right
//! either way. An override-redirect
//! window (a menu, a tooltip) open across the change cannot be configured
//! by a window manager: Smithay recorded its position in the old logical
//! pixels, so it is drawn and hit there, offset, until it next moves
//! itself or closes -- menus close on the next click anyway.

use smithay::reexports::wayland_server::Client;
use smithay::utils::{Logical, Rectangle};
use smithay::xwayland::XWaylandClientData;
use smithay::xwayland::xwm::settings::Value;

use super::super::State;
use super::manage::x_rect;

/// The DPI X toolkits assume at scale 1.
const BASE_DPI: i32 = 96;

/// XSETTINGS carries DPI in 1024ths of a dot per inch.
const DPI_UNIT: i32 = 1024;

/// The scale X would draw at for an output whose `wl_output.scale` integer
/// is `integer_scale`, before the layout's bound ([`fit_x_scale`]): that
/// integer, never below 1. `integer_scale` is `ceil([output] scale)` and
/// the configured scale is at least 0.5, so it is already at least 1; the
/// floor is what makes that true by construction rather than by the config
/// clamp's range.
pub(in crate::compositor) fn x_scale(integer_scale: i32) -> i32 {
    integer_scale.max(1)
}

/// The largest right or bottom edge the X screen can have, in X pixels:
/// XWayland keeps the screen's size in the X server's `ScreenRec`, whose
/// width and height are signed 16-bit (`short`), so an edge past this
/// wraps negative there, whatever the `CARD16` the wire carries.
const X_MAX_EDGE: i64 = i16::MAX as i64;

/// The lowest X coordinate: `INT16`'s floor, what an output at a negative
/// logical origin must stay above once scaled.
const X_MIN_COORDINATE: i64 = i16::MIN as i64;

/// The union of every output's logical rectangle: `left`/`top` inclusive,
/// `right`/`bottom` exclusive (the edge, as a size is).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) struct Bounds {
    pub(in crate::compositor) left: i32,
    pub(in crate::compositor) top: i32,
    pub(in crate::compositor) right: i32,
    pub(in crate::compositor) bottom: i32,
}

impl Bounds {
    /// The union of `rects`, or `None` when there are none. Saturating: a
    /// rectangle reaching past `i32` bounds at `i32::MAX`, which fits no X
    /// scale, rather than wrapping into one that seems to.
    pub(in crate::compositor) fn of(
        rects: impl IntoIterator<Item = Rectangle<i32, Logical>>,
    ) -> Option<Self> {
        rects.into_iter().fold(None, |union: Option<Self>, rect| {
            let next = Self {
                left: rect.loc.x,
                top: rect.loc.y,
                right: rect.loc.x.saturating_add(rect.size.w),
                bottom: rect.loc.y.saturating_add(rect.size.h),
            };
            Some(union.map_or(next, |union| Self {
                left: union.left.min(next.left),
                top: union.top.min(next.top),
                right: union.right.max(next.right),
                bottom: union.bottom.max(next.bottom),
            }))
        })
    }

    /// Whether every coordinate inside, times `scale`, is one X can
    /// address. In `i64`, where any `i32` times any `i32` fits.
    fn fit(self, scale: i32) -> bool {
        let scale = i64::from(scale);
        i64::from(self.left) * scale >= X_MIN_COORDINATE
            && i64::from(self.top) * scale >= X_MIN_COORDINATE
            && i64::from(self.right) * scale <= X_MAX_EDGE
            && i64::from(self.bottom) * scale <= X_MAX_EDGE
    }
}

/// The scale X draws at, and whether the layout fits X's coordinates at
/// it: see the module doc's "The bound".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) struct XScale {
    pub(in crate::compositor) scale: i32,
    pub(in crate::compositor) fits: bool,
}

impl XScale {
    /// Before XWayland is spawned, and what a session without it keeps.
    pub(in crate::compositor) const UNSET: Self = Self {
        scale: 1,
        fits: true,
    };
}

/// The largest integer in `1..=x_scale(integer_scale)` at which the layout
/// `bounds` fits X's coordinates (see [`Bounds::fit`]), or 1, marked as
/// not fitting, where not even 1 does -- the limit X had before any of
/// this. No outputs (`None`) bound nothing. At most four checks: the
/// ceiling is at most `ceil(MAX_SCALE)`.
pub(in crate::compositor) fn fit_x_scale(integer_scale: i32, bounds: Option<Bounds>) -> XScale {
    let ceiling = x_scale(integer_scale);
    let Some(bounds) = bounds else {
        return XScale {
            scale: ceiling,
            fits: true,
        };
    };
    (1..=ceiling).rev().find(|&scale| bounds.fit(scale)).map_or(
        XScale {
            scale: 1,
            fits: false,
        },
        |scale| XScale { scale, fits: true },
    )
}

/// The XSETTINGS that tell toolkits to draw at `scale`, as GNOME's settings
/// daemon names them: GTK's integer window scale; the DPI fonts render at,
/// scaled (what Qt and non-GTK Xft users read); and the same DPI unscaled,
/// which GTK uses instead of `Xft/DPI` when it scales windows itself -- so
/// a GTK font is not scaled twice.
pub(in crate::compositor) fn toolkit_settings(scale: i32) -> [(String, Value); 3] {
    // `scale` is at most `ceil(MAX_SCALE)` = 4, so the largest product,
    // 4 * 96 * 1024, is far inside `i32`.
    [
        ("Gdk/WindowScalingFactor".to_owned(), Value::Integer(scale)),
        (
            "Xft/DPI".to_owned(),
            Value::Integer(scale * BASE_DPI * DPI_UNIT),
        ),
        (
            "Gdk/UnscaledDPI".to_owned(),
            Value::Integer(BASE_DPI * DPI_UNIT),
        ),
    ]
}

/// Sets XWayland's client scale: see the module doc. A client that is not
/// XWayland's has no such data and is left alone (unreachable: the only
/// caller passes the client `XWayland::spawn` made).
pub(in crate::compositor) fn set_client_scale(client: &Client, scale: i32) {
    if let Some(data) = client.get_data::<XWaylandClientData>() {
        data.compositor_state.set_client_scale(f64::from(scale));
    }
}

impl State {
    /// The scale the X server draws at: see the module doc. A field read,
    /// not a computation: `apply()` reads it once per X window it
    /// configures.
    pub(in crate::compositor) fn x11_scale(&self) -> i32 {
        self.x11_fit.scale
    }

    /// The X scale the current layout and the outputs' scales call for:
    /// see the module doc. Two passes over the outputs, no allocation. With
    /// no output at all (a bare harness) the session default's integer
    /// stands in, as the one scale an output would be created at.
    fn chosen_x11_scale(&self) -> XScale {
        fit_x_scale(self.outputs_integer_scale(), self.layout_bounds())
    }

    /// The largest `wl_output.scale` integer over the outputs -- what X
    /// draws at before the layout's bound (see the module doc). With no
    /// output at all (a bare harness) the session default's integer stands
    /// in, as the one scale an output would be created at. One pass, no
    /// allocation.
    fn outputs_integer_scale(&self) -> i32 {
        self.outputs
            .iter()
            .map(|output| output.current_scale().integer_scale())
            .max()
            .unwrap_or_else(|| super::super::output_scale::integer_scale(self.default_scale))
    }

    /// The union of every output's logical rectangle, as the `Space` lays
    /// them out -- the rectangles `xdg_output` advertises to XWayland.
    fn layout_bounds(&self) -> Option<Bounds> {
        Bounds::of(
            self.outputs
                .iter()
                .filter_map(|output| self.space.output_geometry(output)),
        )
    }

    /// Chooses the X scale for a server about to be spawned, records it and
    /// logs a bound: what `xwayland::start` sets the client scale to.
    pub(super) fn adopt_x11_scale(&mut self) -> i32 {
        self.x11_fit = self.chosen_x11_scale();
        self.log_x11_bound();
        self.x11_fit.scale
    }

    /// Tells the X server's toolkits the scale, once its window manager is
    /// up (`READY`, before any X client is accepted). Nothing at scale 1:
    /// see the module doc.
    pub(super) fn publish_x11_scale(&mut self) {
        let scale = self.x11_scale();
        if scale != 1 {
            self.write_toolkit_settings(scale);
        }
    }

    /// The outputs or `[output] scale` changed -- an output added, removed
    /// or resized, a reload of the scale: re-choose the X scale for the
    /// layout as it now is (see the module doc), and follow it on the X side
    /// if it moved. Every site that changes the output layout calls this
    /// once the layout is final and before its `apply()`. Cold: when the X
    /// scale holds, one pass over the outputs and a compare; when it moves,
    /// one output re-send each, one XSETTINGS write and one configure per
    /// managed X window.
    pub(in crate::compositor) fn refit_xwayland(&mut self) {
        if self.xwayland_client.is_none() {
            // Never spawned: `start` chooses from the layout then.
            return;
        }
        let chosen = self.chosen_x11_scale();
        let before = self.x11_fit;
        if chosen == before {
            return;
        }
        self.x11_fit = chosen;
        self.log_x11_bound();
        let scale = chosen.scale;
        if scale == before.scale {
            // Only whether the layout fits at 1 moved: logged, nothing to
            // send.
            return;
        }
        let Some(client) = self.xwayland_client.as_ref() else {
            return;
        };
        set_client_scale(client, scale);
        // Every output re-advertised to XWayland in the new client scale,
        // so it resizes its X screen: with nothing changed, Smithay sends
        // an instance only what its client scale moved -- `xdg_output`'s
        // logical size and position and `wl_output.scale` to XWayland's,
        // a bare `wl_output.done` to everyone else's.
        for output in self.outputs.iter() {
            output.change_current_state(None, None, None, None);
        }
        if self.xwm.is_none() {
            // Not `READY` yet (or the window manager never attached): the
            // client scale is all there is to follow, and `READY` publishes
            // the settings from the scale then.
            return;
        }
        // Written at 1 too: the settings the previous scale published would
        // otherwise stay.
        self.write_toolkit_settings(scale);
        // Each managed X window, re-sent the configure it last had: the same
        // logical rectangle, now in the new X pixels. Through the wire clamp
        // again, at the new scale: the rectangle was clamped for the old one,
        // and a column scrolled far off screen that fit `INT16` at 1 would
        // not at 2. The caller's `apply()` moves any whose logical place
        // changed as well.
        for window in self.windows.values() {
            let Some(x11) = window.x11_surface() else {
                continue;
            };
            let last = x11.last_configure();
            let rect = x_rect(last.loc.x, last.loc.y, last.size.w, last.size.h, scale);
            if let Err(error) = x11.configure(rect) {
                tracing::debug!(id = x11.window_id(), %error, "could not rescale an X11 window");
            }
        }
    }

    /// Says why X draws below the output scale's integer, when it does: at
    /// info when the layout fits at a lower one, a warning when it does not
    /// fit even at 1. Called only when the choice changes, so once per
    /// change.
    fn log_x11_bound(&self) {
        let XScale { scale, fits } = self.x11_fit;
        let ceiling = x_scale(self.outputs_integer_scale());
        let (width, height) = self.layout_bounds().map_or((0, 0), |bounds| {
            (
                bounds.right.saturating_sub(bounds.left),
                bounds.bottom.saturating_sub(bounds.top),
            )
        });
        if !fits {
            tracing::warn!(
                width,
                height,
                "the output layout is wider or taller than X can address \
                 (32767 pixels) even at X scale 1: X windows beyond it are \
                 placed at the edge, and X menus and drags there misplace"
            );
        } else if scale < ceiling {
            tracing::info!(
                scale,
                ceiling,
                width,
                height,
                "X draws below the output scale's integer: the layout at that \
                 scale would be wider or taller than X can address (32767 pixels)"
            );
        }
    }

    fn write_toolkit_settings(&mut self, scale: i32) {
        let Some(wm) = self.xwm.as_mut() else {
            return;
        };
        if let Err(error) = wm.set_xsettings(toolkit_settings(scale).into_iter()) {
            // The window manager's connection failing: the death paths
            // handle the server; X apps just draw at their own default.
            tracing::warn!(%error, scale, "could not tell X toolkits the scale");
        }
    }
}
