//! Override-redirect X windows: menus, tooltips, drop-downs, drag icons.
//!
//! An override-redirect window places, sizes and maps itself, and the X
//! protocol lets no window manager refuse or move it. So it never enters
//! the core -- it is not a column, not in any window list, never focused --
//! and is drawn where it put itself, above every window and below the
//! layer shell's top and overlay layers: where an xdg popup, drawn with its
//! window, is too. It takes the pointer at the same depth, so a menu is
//! clickable over the window it opened from. Newest on top.
//!
//! The session lock replaces all of it exactly as it replaces every
//! window: a locked frame gathers the lock screen and nothing else, the
//! pointer finds only lock surfaces, and none of these gets a frame
//! callback. It does not close them -- nothing here can unmap an
//! override-redirect window, and toolkits keep their menus open through the
//! lock's focus release (see `mod.rs`) -- so a menu open at lock is hidden
//! and inert until unlock, then shown again. Nothing here is lock-aware on its own; each caller -- the
//! hit test, the frame gathering in `render/elements.rs`, the frame
//! callback and presentation passes -- sits behind the lock branch of its
//! path.

use smithay::desktop::WindowSurfaceType;
use smithay::desktop::utils::{
    OutputPresentationFeedback, send_frames_surface_tree, take_presentation_feedback_surface_tree,
};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::SurfaceData;
use smithay::xwayland::X11Surface;

use super::super::State;
use super::super::pointer_focus::PointerFocus;
use super::super::toplevel_cap::{
    MAX_X11_UNMANAGED_PER_CLIENT, MAX_X11_UNMANAGED_REFUSED_TOLERANCE,
};
use super::focus::x_client_key;

/// Where an override-redirect window is on the screen: the position it
/// last configured itself to (Smithay records every `ConfigureNotify`),
/// and the size of what it drew.
pub(in crate::compositor) fn rect_of(window: &X11Surface) -> Rectangle<i32, Logical> {
    Rectangle::new(window.last_configure().loc, window.bbox().size)
}

impl State {
    /// An override-redirect window mapped itself: draw it from now on.
    ///
    /// Refused, past its client's live-window cap (see `toplevel_cap.rs`):
    /// never drawn, never hit-tested, never sent frame callbacks -- the
    /// refuse-the-map form, the way `map_x11_window` refuses a managed
    /// window past its own cap. X has no client object to post an error
    /// to, so there is no kill to send. Up to the kill tolerance past
    /// the cap the window simply stays invisible, and its client keeps
    /// everything it had; past the tolerance its X connection is killed,
    /// so one client cannot spend the server's budget on refused menus
    /// alone (see `kill_runaway_x_client`). Other X clients (other
    /// client bits) are unaffected: the count is per X client.
    pub(super) fn map_x11_unmanaged(&mut self, window: X11Surface) {
        let (xwm, xid) = (window.xwm_id(), window.window_id());
        if self
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == xid && known.xwm_id() == xwm)
        {
            // A repeated report for a window already drawn: leave its place
            // alone, and claim nothing twice.
            return;
        }
        // The per-X-client cap, read before anything is granted: past it
        // the map is refused outright.
        let x_client = x_client_key(xid);
        if !self.x11_unmanaged_cap.admits(&x_client) {
            tracing::warn!(
                xid,
                live = self.x11_unmanaged_cap.live_for(&x_client),
                cap = MAX_X11_UNMANAGED_PER_CLIENT,
                "refusing to map an X11 override-redirect window: its client is past the per-client cap"
            );
            // Tolerated refused windows stay mapped X-side (never drawn)
            // up to the reclaim tolerance; past it the runaway client is
            // killed, so one client cannot spend the server's budget on
            // its own (see `kill_runaway_x_client`).
            if self.x11_unmanaged_cap.track_refused(x_client, xid) {
                return;
            }
            self.kill_runaway_x_client(xid);
            return;
        }
        self.x11_unmanaged.push(std::sync::Arc::new(window));
        // Claimed now that the window is in: `admits` said yes above, on
        // this same dispatch, so this cannot overflow the bound (see
        // `X11UnmanagedCap::admits` for why the split is sound). Released
        // in `unmap_x11_unmanaged` on every path the window can leave by
        // short of the server's death, which drains the whole count.
        self.x11_unmanaged_cap.claim(x_client, xid);
        // Deliberately no cursor-hide update here: the armed deadline
        // harmlessly survives the open -- no hide can fire while the menu
        // covers the pointer, and a firing then only disarms, which the
        // unmap path below re-arms. (Re-arming fresh at open instead would
        // wrongly extend the wait past a quickly-dismissed menu.)
        self.request_render();
    }

    /// A runaway X client past the reclaim tolerance: kill its X
    /// connection, so its windows' server buffers go and it maps no more.
    ///
    /// A refused menu costs the XWayland server what a drawn one does,
    /// spent before the window manager hears of the map (see
    /// `xwayland/tests/refused_cost.rs` and the ticket): untracked, one X
    /// client mapping menus past its cap spends the server's budget
    /// (`xwayland_budget.rs`) on its own, and at the budget scoot
    /// disconnects the server and every X client's windows go with it.
    /// Withholding `_XWAYLAND_ALLOW_COMMITS` was measured and changes
    /// nothing (the buffers are spent before the refusal, and a refused
    /// window never commits again for lack of frame callbacks). Closing
    /// refused windows one by one was measured too and does not keep up:
    /// a paced 5000-menu storm still disconnects with thousands reclaimed
    /// (the frees lag the maps even settling every map), so the source
    /// has to stop, not be chased.
    ///
    /// Up to [`MAX_X11_UNMANAGED_REFUSED_TOLERANCE`] refused windows past
    /// the cap are merely refused (tracked, never drawn) -- killing a
    /// client for its 129th menu would be hostile to a legitimate burst,
    /// and a toolkit whose menu is destroyed under it can fail on its
    /// next request to it (`BadWindow`, which kills a plain Xlib client).
    /// Past the tolerance (192 mapped menus total) the client is
    /// unmistakably runaway, and it is killed whole: every one of its
    /// windows -- drawn, tolerated and in-flight refused alike -- is
    /// destroyed by the X server at once, their server buffers freed on
    /// unrealize, and its connection broken so it maps no more. Other X
    /// clients (other client bits) and the server itself stay served --
    /// the same last-resort shape as disconnecting a Wayland client past
    /// its own bound, except X has no client object to post an error to,
    /// so the kill is the disconnect. A later request by the dead client
    /// fails on its broken connection, never as a dangling `BadWindow` in
    /// a live client: cleaner than destroying its windows one by one and
    /// leaving it alive to trip over their ids.
    ///
    /// The kill runs over scoot's own X connection to its own server
    /// (`State::xkill`, connected lazily here, dropped with the server),
    /// with `XKillClient` on any one of the runaway's window ids (the X
    /// server kills the connection that owns the resource). One X request
    /// plus a flush, on the window-open abuse path only -- nothing for a
    /// client under its tolerance, and no per-frame cost. A kill that
    /// fails (no display, no connection, the window or the server already
    /// gone) is a debug line, never a panic: the refusal above already
    /// holds, and the server's death drains everything anyway.
    fn kill_runaway_x_client(&mut self, xid: u32) {
        use smithay::reexports::x11rb::connection::Connection as _;
        use smithay::reexports::x11rb::protocol::xproto::ConnectionExt as _;
        if self.xkill.is_none() {
            let Some(display) = self.xdisplay else {
                tracing::debug!(xid, "no X display to kill a runaway X client on");
                return;
            };
            match smithay::reexports::x11rb::connect(Some(&super::display_value(display))) {
                Ok((conn, _)) => self.xkill = Some(conn),
                Err(error) => {
                    tracing::debug!(xid, %error, "could not connect to kill a runaway X client");
                    return;
                }
            }
        }
        // `kill_client` sends; `check` reports an X11 error for a bad
        // resource (the window already gone); `flush` (or the send
        // itself) reports a transport error when the server is gone. Any
        // failure drops the connection (reconnected lazily on the next
        // kill; a bad resource is rare enough that the reconnect costs
        // nothing measurable) and is a debug line, never a panic. The
        // borrow ends before any assignment below.
        let result: Result<(), String> = {
            let Some(conn) = self.xkill.as_ref() else {
                return;
            };
            (|| {
                let cookie = conn
                    .kill_client(xid)
                    .map_err(|error| format!("send: {error}"))?;
                cookie
                    .check()
                    .map_err(|error| format!("refused: {error:?}"))?;
                conn.flush().map_err(|error| format!("flush: {error}"))?;
                Ok(())
            })()
        };
        match result {
            Ok(()) => tracing::warn!(
                xid,
                cap = MAX_X11_UNMANAGED_PER_CLIENT,
                tolerance = MAX_X11_UNMANAGED_REFUSED_TOLERANCE,
                "killed a runaway X11 client past the override-redirect reclaim tolerance"
            ),
            Err(where_) => {
                tracing::debug!(xid, %where_, "could not kill a runaway X11 client");
                self.xkill = None;
            }
        }
    }

    /// An override-redirect window unmapped or died. The pointer is
    /// re-derived because it may have been over it: `wl_pointer.button`
    /// goes to whatever the pointer last entered, and a menu that closed
    /// under a resting pointer would otherwise take the next click with it.
    pub(super) fn unmap_x11_unmanaged(&mut self, window: &X11Surface) {
        let (xwm, xid) = (window.xwm_id(), window.window_id());
        // A tolerated refused window was never drawn, so its unmap or
        // destroy frees only its refused unit -- idempotent (a reclaimed
        // window was never tracked, a drawn window holds the drawn unit
        // instead). A drawn window's removal below is unchanged.
        self.x11_unmanaged_cap.release_refused(&xid);
        let before = self.x11_unmanaged.len();
        self.x11_unmanaged
            .retain(|known| !(known.window_id() == xid && known.xwm_id() == xwm));
        if self.x11_unmanaged.len() != before {
            // Both removal paths reach here: an unmap, and the destroy that
            // follows it (`forget_x11_window` funnels destroy here too, and
            // a client that destroys a mapped window outright lands here
            // directly). The release is idempotent by the owner map, so the
            // destroy after an unmap frees nothing twice.
            self.x11_unmanaged_cap.release(&xid);
            self.refresh_pointer_focus();
            // The mirror of the map path above: the cover may be eligible
            // for the cursor hide again.
            self.update_cursor_hide(std::time::Instant::now());
            self.request_render();
        }
    }

    /// An override-redirect window moved or resized itself.
    pub(super) fn x11_unmanaged_moved(&mut self) {
        self.request_render();
    }

    /// Every override-redirect window gone at once: the server died.
    /// A dead server sends no unmap or destroy for the windows it had, so
    /// without the drain their units would stay claimed against window ids
    /// a restarted server reuses. The cap's clear drains the drawn units
    /// and the tolerated-refused ones together (a refused window's X id is
    /// as reusable as a drawn one's); the draw-list refresh below runs
    /// only when drawn windows existed, since a refused window was never
    /// drawn.
    pub(super) fn clear_x11_unmanaged(&mut self) {
        self.x11_unmanaged_cap.clear();
        if !self.x11_unmanaged.is_empty() {
            self.x11_unmanaged.clear();
            self.refresh_pointer_focus();
            self.request_render();
        }
    }

    /// The override-redirect window at `pos`, top-most first, the way
    /// `window_under` answers for managed windows -- as the X pointer focus,
    /// so a drag over a menu or tooltip reaches the window manager's XDND
    /// side like one over any X window (see `pointer_focus.rs`). Asks each
    /// window's surface tree, so its input region is honoured.
    /// Allocation-free: this is on the per-motion hit test, and with no
    /// override-redirect window mapped -- nearly always -- it is an
    /// empty-`Vec` test; a hit costs one reference count (the windows are
    /// kept shared for this -- see `State::x11_unmanaged`).
    pub(in crate::compositor) fn x11_unmanaged_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(PointerFocus, Point<f64, Logical>)> {
        self.x11_unmanaged.iter().rev().find_map(|window| {
            let location = window.last_configure().loc;
            window
                .surface_under(pos - location.to_f64(), (0, 0), WindowSurfaceType::ALL)
                .map(|(surface, point)| {
                    (
                        PointerFocus::X11 {
                            window: std::sync::Arc::clone(window),
                            surface,
                        },
                        (point + location).to_f64(),
                    )
                })
        })
    }

    /// Frame callbacks for the override-redirect windows on `output`: none
    /// of them is in the `Space`, so the window loop never reaches them, and
    /// XWayland paces each window's updates on its callbacks -- a menu would
    /// freeze on its first frame without this. Every one overlapping the
    /// output, told with the output as its token, as the window loop does.
    pub(in crate::compositor) fn x11_unmanaged_frames(
        &self,
        output: &Output,
        region: Option<Rectangle<i32, Logical>>,
        time: std::time::Duration,
    ) {
        for window in &self.x11_unmanaged {
            if region.is_some_and(|region| !region.overlaps(rect_of(window))) {
                continue;
            }
            if let Some(surface) = window.wl_surface() {
                send_frames_surface_tree(
                    &surface,
                    output,
                    time,
                    Some(std::time::Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
        }
    }

    /// Presentation feedback for the override-redirect windows, alongside
    /// the windows' own (see `presentation_time.rs`).
    pub(in crate::compositor) fn x11_unmanaged_feedback(
        &self,
        output: &Output,
        feedback: &mut OutputPresentationFeedback,
        flags: impl FnMut(&WlSurface, &SurfaceData) -> wp_presentation_feedback::Kind + Copy,
    ) {
        for window in &self.x11_unmanaged {
            if let Some(surface) = window.wl_surface() {
                take_presentation_feedback_surface_tree(
                    &surface,
                    feedback,
                    |_, _| Some(output.clone()),
                    flags,
                );
            }
        }
    }
}
