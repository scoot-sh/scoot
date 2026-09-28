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
//! screen from the outputs by then, and nothing re-sends them.)
//!
//! # Which scale: `ceil([output] scale)`
//!
//! X toolkits scale only by integers (GTK's window scale, Qt's and Java's
//! device ratio off it), so `S` is an integer, and it is the one scoot
//! already advertises on `wl_output.scale`: [`State::integer_scale`]. At an
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
//! was and no XSETTINGS entry is written (a reload back to 1 does write
//! them, at 1, so the old ones do not linger): nothing an X client or toolkit reads
//! differs from the session before any of this existed (pinned:
//! `tests/scale.rs`'s `a_scale_1_session_is_untouched`).
//!
//! # A reload that moves `S`
//!
//! [`State::rescale_xwayland`] sets the new client scale before the
//! outputs are re-advertised (so XWayland resizes its X screen to the new
//! X pixels), publishes the new toolkit settings -- GTK rescales live off
//! them -- and reconfigures every managed X window into
//! its new X pixels at its unchanged logical place. An app that read the
//! scale once at startup keeps drawing at its old scale in the new pixels
//! (smaller or bigger by the ratio) until it is restarted; its geometry is
//! right either way. An override-redirect window (a menu, a tooltip) open
//! across the reload cannot be configured by a window manager: Smithay
//! recorded its position in the old logical pixels, so it is drawn and hit
//! there, offset, until it next moves itself or closes -- menus close on
//! the next click anyway.

use smithay::reexports::wayland_server::Client;
use smithay::xwayland::XWaylandClientData;
use smithay::xwayland::xwm::settings::Value;

use super::super::State;
use super::manage::x_rect;

/// The DPI X toolkits assume at scale 1.
const BASE_DPI: i32 = 96;

/// XSETTINGS carries DPI in 1024ths of a dot per inch.
const DPI_UNIT: i32 = 1024;

/// The scale X draws at for an output whose `wl_output.scale` integer is
/// `integer_scale`: that integer, never below 1. `integer_scale` is
/// `ceil([output] scale)` and the configured scale is at least 0.5, so it
/// is already at least 1; the floor is what makes that true by
/// construction rather than by the config clamp's range.
pub(in crate::compositor) fn x_scale(integer_scale: i32) -> i32 {
    integer_scale.max(1)
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
    /// The scale the X server draws at: see the module doc.
    pub(in crate::compositor) fn x11_scale(&self) -> i32 {
        x_scale(self.integer_scale)
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

    /// A reload moved `[output] scale`, and `integer_scale` with it, from
    /// `before`: follow it on the X side, if the X scale moved. Runs before
    /// the outputs are re-advertised (see the module doc), on the cold reload
    /// path: one walk of the windows, one XSETTINGS write, one configure per
    /// managed X window.
    pub(in crate::compositor) fn rescale_xwayland(&mut self, before: i32) {
        let scale = self.x11_scale();
        if x_scale(before) == scale {
            return;
        }
        let Some(client) = self.xwayland_client.as_ref() else {
            return;
        };
        set_client_scale(client, scale);
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
        // not at 2. `apply()` after the reload moves any whose logical place
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
