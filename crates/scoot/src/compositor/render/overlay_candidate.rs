//! Which tiled window may ride an overlay plane: the scanout-candidate pick.
//!
//! Smithay's overlay assignment only considers elements of kind
//! [`Kind::ScanoutCandidate`](smithay::backend::renderer::element::Kind) or
//! `Cursor`, and scoot builds every window surface `Kind::Unspecified` -- so
//! no window rides an overlay until one is marked. This module picks at most
//! one window per output per frame to mark, within the narrow bounds the
//! hardware allows (currently Apple Silicon's `apple,dcp`: overlays take
//! `LINEAR` alpha formats only, and there is no cursor plane, so a composited
//! cursor overlapping the candidate refuses the ride -- see
//! `tty/scanout.rs`'s overlay section for the priority this implies).
//!
//! # Mark now, judge last frame
//!
//! The mark has to be in the elements before Smithay sees them, but whether
//! the frame is clean enough to ride on -- none translucent or rounded,
//! [`judge_elements`](super::primary_direct::judge_elements), shared with
//! `render::primary_direct` rather than copied -- is known only once the list
//! is gathered. So the pick is made post-gather from the exact list and
//! applied pre-gather on the *next* frame ([`OverlayPick::marking`], from
//! [`OverlayPick::refresh`]). The lag is one frame, and only on the
//! conservative refusals: everything capture-correctness depends on --
//! locked, a capture stream, a forced composite frame, a covering fullscreen
//! window -- gates the mark on the *current* frame, so a frame a capture is
//! about to read, or a streamed one, never marks anything.
//!
//! What the lag can do at most is ride one transitional frame: a window that
//! just turned translucent rides once before the refusal clears it, and a
//! newly suitable window waits one frame. Smithay's own guards (the overlap
//! with anything composited above, the plane's format list, the atomic
//! `TEST_ONLY` commit) still apply to that frame, and the capture contract
//! below does not care what was marked -- only what rode.
//!
//! # The capture contract
//!
//! A window on an overlay is absent from the swapchain slot exactly like a
//! primary-direct frame, so a capture served off the slot would silently miss
//! it. The outcome -- what Smithay actually assigned, read off
//! `DrmCompositor`'s `overlay_elements` in `render_and_queue`, filtered to
//! non-cursor elements -- fires the same [`Captures::note_direct`] mark a
//! primary-direct frame does (`render::scanout`), and the same force clears
//! it. The forced frame unmarks by construction: the mark is suppressed while
//! a force is armed (see [`OverlayPick::marking`]), so the frame a capture
//! reads always composites whole.
//!
//! # Cost
//!
//! [`OverlayPick::marking`] is a placement scan, no allocation.
//! [`OverlayPick::refresh`] is the shared element scan (the same
//! handful of elements `render::primary_direct` already walks on covered
//! outputs) plus a placement scan. Neither runs where there is no overlay
//! plane to ride: the gather call sites pass no candidate then, and the
//! frame path skips the refresh -- the one comparison that gates it, never
//! the scans.

use std::collections::HashMap;

use scoot_core::{Arrangement, OutputId, WindowId};
use smithay::backend::renderer::element::Element;
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;

use super::elements::Elements;
use super::primary_direct::{PrimaryDirect, in_tree, judge_elements};

#[cfg(test)]
mod tests;

/// The overlay-candidate pick for one output: the window the next frame's
/// gather marks `Kind::ScanoutCandidate`, chosen by the last frame's exact
/// element list. Lives on the scanout backend beside `judge_scratch`, per
/// output across frames.
#[derive(Default)]
pub(super) struct OverlayPick {
    /// The picked window, if last frame was clean and had a suitable one.
    window: Option<WindowId>,
}

impl OverlayPick {
    /// The window this frame's gather may mark a scanout candidate, or
    /// `None`.
    ///
    /// The stored pick, revalidated against the *current* arrangement: the
    /// window must still be placed on this output, visible, and not
    /// fullscreen (a fullscreen window is the primary plane's domain, and a
    /// covering one occludes everything tiled behind it). Everything else is
    /// a current-frame gate, so no frame a capture depends on marks:
    /// `locked` (no windows gathered anyway), `streaming` (a capture stream
    /// reads the slot every frame and never forces), `force_armed` (this very
    /// frame is the composite one a capture is about to read),
    /// `covered_by_fullscreen`, and whether the CRTC has any overlay plane
    /// at all. Allocation-free: a placement scan.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn marking(
        &self,
        locked: bool,
        streaming: bool,
        force_armed: bool,
        covered_by_fullscreen: bool,
        overlay_planes: usize,
        arrangement: Option<&Arrangement>,
        output: OutputId,
    ) -> Option<WindowId> {
        if locked || streaming || force_armed || covered_by_fullscreen || overlay_planes == 0 {
            return None;
        }
        let window = self.window?;
        let arrangement = arrangement?;
        arrangement
            .placements
            .iter()
            .any(|placement| {
                placement.id == window
                    && placement.output == output
                    && placement.visible
                    && !placement.fullscreen
            })
            .then_some(window)
    }

    /// Re-picks from the frame just gathered: `None` while locked or covered
    /// (nothing tiled can ride there), `None` when the shared
    /// rounded/translucent refusals hit, else the first suitable window
    /// front-to-back that actually drew. A suitable window is placed on this
    /// output, visible, not fullscreen, and paired with a surface (an X
    /// window XWayland has not paired yet draws nothing and would ride
    /// nothing).
    ///
    /// No separate element-to-window pass: the refusal already proved the
    /// frame clean, so the first suitable placement is the topmost rideable
    /// window -- confirmed by finding one of its surfaces in the gathered
    /// list ([`in_tree`]). The confirmation is what keeps an unmapped window
    /// from stranding the pick: scoot lays a window out from creation to
    /// destruction, so an unmapped one is still placed (and still owns its
    /// surface) while gathering nothing, and the pick must move past it to
    /// the live window below. Front to back is the arrangement's reverse
    /// (the order `window_elements` gathers in). Allocation-free: scans,
    /// lookups and bounded tree walks.
    pub(super) fn refresh<R>(
        &mut self,
        locked: bool,
        covered_by_fullscreen: bool,
        elements: &[Elements<R>],
        arrangement: Option<&Arrangement>,
        windows: &HashMap<WindowId, Window>,
        output: OutputId,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + 'static,
    {
        self.window = pick(
            locked,
            covered_by_fullscreen,
            elements,
            arrangement,
            windows,
            output,
        );
    }
}

/// [`OverlayPick::refresh`]'s body, over the inputs rather than the pick, so
/// every combination is pinnable without building a backend.
fn pick<R>(
    locked: bool,
    covered_by_fullscreen: bool,
    elements: &[Elements<R>],
    arrangement: Option<&Arrangement>,
    windows: &HashMap<WindowId, Window>,
    output: OutputId,
) -> Option<WindowId>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + 'static,
{
    if locked || covered_by_fullscreen {
        return None;
    }
    // The shared refusals: a translucent element the plane would have to
    // blend, a rounded one whose clipped corners the plane would lose (a
    // `Rounded` forwards its inner buffer unscissored). Whole-frame, like
    // the primary judge -- almost never refuses a frame that could have
    // ridden, and never a wrong pixel.
    let refusal = judge_elements(elements.iter().map(|element| {
        (
            matches!(element, Elements::RoundedSurface(_)),
            element.alpha(),
        )
    }));
    if refusal != PrimaryDirect::Eligible {
        return None;
    }
    let arrangement = arrangement?;
    arrangement
        .placements
        .iter()
        .rev()
        .filter(|placement| {
            placement.output == output && placement.visible && !placement.fullscreen
        })
        .filter_map(|placement| {
            let window = windows.get(&placement.id)?;
            let root = window.wl_surface()?;
            drew(&root, elements).then_some(placement.id)
        })
        .next()
}

/// Whether any gathered element is a surface in `root`'s tree: the pick drew
/// this frame. An unmapped window still owns its surface, so the surface
/// existing is not enough -- only a gathered element proves it.
fn drew(root: &WlSurface, elements: &[impl Element]) -> bool {
    elements.iter().any(|element| in_tree(root, element.id()))
}
