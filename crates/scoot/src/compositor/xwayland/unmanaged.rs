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

/// Where an override-redirect window is on the screen: the position it
/// last configured itself to (Smithay records every `ConfigureNotify`),
/// and the size of what it drew.
pub(in crate::compositor) fn rect_of(window: &X11Surface) -> Rectangle<i32, Logical> {
    Rectangle::new(window.last_configure().loc, window.bbox().size)
}

impl State {
    /// An override-redirect window mapped itself: draw it from now on.
    pub(super) fn map_x11_unmanaged(&mut self, window: X11Surface) {
        let (xwm, xid) = (window.xwm_id(), window.window_id());
        if !self
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == xid && known.xwm_id() == xwm)
        {
            self.x11_unmanaged.push(window);
        }
        self.request_render();
    }

    /// An override-redirect window unmapped or died. The pointer is
    /// re-derived because it may have been over it: `wl_pointer.button`
    /// goes to whatever the pointer last entered, and a menu that closed
    /// under a resting pointer would otherwise take the next click with it.
    pub(super) fn unmap_x11_unmanaged(&mut self, window: &X11Surface) {
        let (xwm, xid) = (window.xwm_id(), window.window_id());
        let before = self.x11_unmanaged.len();
        self.x11_unmanaged
            .retain(|known| !(known.window_id() == xid && known.xwm_id() == xwm));
        if self.x11_unmanaged.len() != before {
            self.refresh_pointer_focus();
            self.request_render();
        }
    }

    /// An override-redirect window moved or resized itself.
    pub(super) fn x11_unmanaged_moved(&mut self) {
        self.request_render();
    }

    /// Every override-redirect window gone at once: the server died.
    pub(super) fn clear_x11_unmanaged(&mut self) {
        if !self.x11_unmanaged.is_empty() {
            self.x11_unmanaged.clear();
            self.refresh_pointer_focus();
            self.request_render();
        }
    }

    /// The override-redirect surface at `pos`, top-most first, the way
    /// `window_under` answers for managed windows. Asks each window's
    /// surface tree, so its input region is honoured. Allocation-free: this
    /// is on the per-motion hit test, and with no override-redirect window
    /// mapped -- nearly always -- it is an empty-`Vec` test.
    pub(in crate::compositor) fn x11_unmanaged_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.x11_unmanaged.iter().rev().find_map(|window| {
            let location = window.last_configure().loc;
            window
                .surface_under(pos - location.to_f64(), (0, 0), WindowSurfaceType::ALL)
                .map(|(surface, point)| (surface, (point + location).to_f64()))
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
