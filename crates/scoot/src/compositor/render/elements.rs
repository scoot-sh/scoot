//! What a frame is made of, gathered without naming a renderer.
//!
//! This is the renderer-agnostic half of the seam: everything here is
//! generic over `R`, so the same code assembles a frame whichever
//! implementation [`Backend`](super::Backend) is carrying. The element
//! sources it draws from were already renderer-generic before the seam
//! existed -- `cursor.rs` returns `CursorElement<R>`, `session_lock.rs`'s
//! `lock_elements` returns `WaylandSurfaceRenderElement<R>`, and
//! `decorations.rs` returns plain `SolidColorRenderElement`s with no
//! renderer in the type at all -- so what this module adds is the one enum
//! that holds all three, and the gathering order between them.

use std::collections::HashMap;

use scoot_core::{Arrangement, OutputId, Rect, WindowId};
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{AsRenderElements, Kind, render_elements};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::space::SpaceElement;
use smithay::desktop::{
    LayerMap, PopupManager, Space, Window, WindowSurface, layer_map_for_output,
};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Scale};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::compositor::State;
use crate::compositor::cursor::CursorElement;
use crate::compositor::decorations::{Appearance, Decorations, RingElement};
use crate::compositor::layer_shell;
use crate::compositor::output_clip::to_output_local;
use crate::compositor::rounded::{Rounded, clip_rect, physical_radius};

// What a frame can draw. Which kind covers which is decided by the *order*
// they go into the list (see `gather` below on Smithay's back-to-front
// convention), not by this enum. The background isn't a variant here at all
// -- it's the `render_output` call's `clear_color`, always the bottom-most
// thing on screen by construction; see `decorations.rs`'s module doc for why
// that's simpler and safer than a full-output element.
//
// Windows and layer-shell surfaces (bars, wallpapers) share the one `Surface`
// variant because they produce the same element type: a variant each would
// need two `From<WaylandSurfaceRenderElement<R>>` impls on this enum, which
// cannot coexist. Nothing is lost by that -- what puts a bar in front of a
// window and a wallpaper behind one is where `gather` inserts it, and a
// variant could not have expressed that anyway.
//
// Generic over `R` rather than fixed to one renderer, which is the whole
// point of the seam: `Decoration` names no renderer at all, and the other
// two carry whichever one drew them.
render_elements! {
    pub(super) Elements<R> where R: ImportAll + ImportMem;
    Cursor = CursorElement<R>,
    Surface = WaylandSurfaceRenderElement<R>,
    Decoration = SolidColorRenderElement,
    PaintedRing = MemoryRenderBufferRenderElement<R>,
    RoundedSurface = Rounded<WaylandSurfaceRenderElement<R>>,
}

/// What every element source needs to know about the frame being drawn, read
/// once from the output so no two of them can disagree about it.
pub(super) struct FrameContext {
    /// The *physical* render-target size: what the framebuffer is, in
    /// pixels.
    pub(super) size: (i32, i32),
    /// What every element's own coordinates are built at, read from the
    /// output rather than hardcoded so windows, layer surfaces, the ring and
    /// the cursor can never disagree about it. 1.0 unless `[output] scale`
    /// says otherwise (see `output_scale.rs`).
    pub(super) scale: f64,
    /// The output rectangle in *logical* coordinates -- the space the core
    /// arranges in and the space every element is built in. `None` only for
    /// an output that isn't in the `Space`, which `headless::init_named`
    /// cannot produce.
    pub(super) geometry: Option<Rectangle<i32, Logical>>,
    /// The core's id for this output: which windows and rings this frame
    /// draws. A window is drawn only on the output it is placed on (see
    /// `output_clip.rs`), so everything placed elsewhere is left out of the
    /// frame entirely. `None` only for an output `Outputs` does not know,
    /// which the render loop -- walking that very collection -- cannot
    /// produce; such a frame draws no window rather than every window.
    pub(super) output: Option<OutputId>,
    /// Whether the session is locked. Read once by the caller so elements,
    /// clear colour and frame callbacks are all answering the same question
    /// about the same frame.
    pub(super) locked: bool,
}

impl FrameContext {
    /// The output rectangle in logical coordinates, falling back to the
    /// framebuffer's own size for an output with no geometry.
    ///
    /// This is the whole coordinate-space split in one place: what this
    /// returns is logical, while [`FrameContext::size`] is the physical
    /// render target.
    pub(super) fn bounds(&self) -> Rect {
        let (width, height) = self.size;
        self.geometry
            .map(|geometry| {
                Rect::new(
                    geometry.loc.x,
                    geometry.loc.y,
                    geometry.size.w,
                    geometry.size.h,
                )
            })
            .unwrap_or_else(|| Rect::new(0, 0, width, height))
    }
}

impl State {
    /// Whether the frames this session draws carry the cursor.
    ///
    /// Only `--tty` puts one on screen (see `cursor.rs`'s module doc):
    /// headless has no display and `--nested` shows the host's own. The
    /// frame paths ask this whether to gather the cursor, and every cursor
    /// change reaches one decision about redrawing it --
    /// `State::cursor_changed`, which asks this too -- so the frame and its
    /// redraw triggers cannot disagree. (`grep` for `tty.is_some()` beside
    /// a cursor change should find nothing.)
    ///
    /// The test seam: a harness has no `Tty`, so the suites that need a
    /// frame with the cursor composited into it (the shape every `--tty`
    /// frame on the dumb tier has) set `frame_cursor_for_test` instead.
    pub(crate) fn frame_draws_cursor(&self) -> bool {
        #[cfg(test)]
        if let Some(forced) = self.frame_cursor_for_test {
            return forced;
        }
        self.tty.is_some()
    }

    /// The cursor's render elements for one frame of one output, at the
    /// pointer's position *on that output*: the pointer's global logical
    /// location minus the output's logical origin, which is what every
    /// other element in the frame is placed against. With one output at the
    /// origin (every `--tty` session today) that is the identity; with more
    /// than one it is what keeps a pointer on one output off every other
    /// output's frame (its elements land outside that framebuffer).
    ///
    /// Empty without a pointer, for a hidden cursor, and for a client
    /// cursor surface with nothing committed yet.
    pub(super) fn cursor_elements<R>(
        &self,
        renderer: &mut R,
        frame: &FrameContext,
    ) -> Vec<CursorElement<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        match self.cursor_location(frame) {
            Some(location) => self.cursor.element(renderer, location, frame.scale),
            None => Vec::new(),
        }
    }

    /// [`State::cursor_elements`], appended to `out` -- the capture path's
    /// pooled list.
    pub(super) fn cursor_elements_into<R>(
        &self,
        renderer: &mut R,
        frame: &FrameContext,
        out: &mut Vec<CursorElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        if let Some(location) = self.cursor_location(frame) {
            self.cursor
                .element_into(renderer, location, frame.scale, out);
        }
    }

    /// The pointer's position on `frame`'s output, in that output's logical
    /// coordinates, or `None` without a pointer.
    fn cursor_location(&self, frame: &FrameContext) -> Option<Point<f64, Logical>> {
        let mut location = self.seat.get_pointer()?.current_location();
        if let Some(geometry) = frame.geometry {
            location -= geometry.loc.to_f64();
        }
        Some(location)
    }

    /// Everything this frame draws, front-most first, plus the client cursor
    /// surface it drew from if there was one.
    ///
    /// `ring_elements` is built by the caller rather than here, because the
    /// painted ring needs the renderer and is deliberately computed *before*
    /// the framebuffer is bound -- see `super::draw_frame_with`. It arrives
    /// already mapped into frame elements for the same reason the window
    /// split below maps its own: one `Vec<Elements<R>>` in, extended, done.
    ///
    /// The returned surface is set only on the frames that actually went
    /// looking for one (`cursor`, with a pointer); it is what the frame
    /// callback pass at the end of `render()` wakes.
    ///
    /// `cursor` is whether the list carries the cursor at all: the frame
    /// paths pass [`State::frame_draws_cursor`] (only `--tty` draws one on
    /// screen), and the capture path (`render::capture_cursor`) passes
    /// `true`, because a capture may ask for the pointer on any backend.
    pub(super) fn gather_elements<R>(
        &mut self,
        renderer: &mut R,
        output: &Output,
        frame: &FrameContext,
        ring_elements: Vec<Elements<R>>,
        arrangement: Option<&Arrangement>,
        cursor: bool,
    ) -> (Vec<Elements<R>>, Option<WlSurface>)
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let mut elements = Vec::new();
        let cursor_surface = self.gather_elements_into(
            renderer,
            output,
            frame,
            ring_elements,
            arrangement,
            cursor,
            &mut elements,
        );
        (elements, cursor_surface)
    }

    /// [`State::gather_elements`], appended to `out` -- the capture path's
    /// pooled list (`render/capture_cursor.rs`). `out` is expected empty.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gather_elements_into<R>(
        &mut self,
        renderer: &mut R,
        output: &Output,
        frame: &FrameContext,
        ring_elements: Vec<Elements<R>>,
        arrangement: Option<&Arrangement>,
        cursor: bool,
        out: &mut Vec<Elements<R>>,
    ) -> Option<WlSurface>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let FrameContext {
            size: (width, height),
            scale,
            geometry,
            output: output_id,
            locked,
        } = *frame;
        // The client-supplied cursor surface this frame drew from, if any.
        let mut cursor_surface: Option<WlSurface> = None;
        // The list is empty when there's nothing to draw (no pointer,
        // hidden, or a client cursor surface with no content yet) and can
        // hold more than one element when a client's cursor surface has
        // subsurfaces of its own.
        let cursor_elements = if cursor && self.seat.get_pointer().is_some() {
            cursor_surface = self.cursor.surface().cloned();
            self.cursor_elements(renderer, frame)
        } else {
            Vec::new()
        };
        // Smithay's damage tracker draws a `&[E]` back-to-front by walking it
        // in reverse (confirmed in
        // `OutputDamageTracker::render_output_internal`, which iterates
        // `render_elements.iter().rev()`), so the *first* entry here ends up
        // drawn *last*, i.e. on top. Read this list as "front to back":
        //
        // 1. the cursor -- meaningless hidden behind anything;
        // 2. the overlay and top layer-shell layers, which the protocol
        //    defines as being above ordinary windows (a bar, a launcher, a
        //    notification) -- just the overlay while a fullscreen window
        //    covers the output;
        // 3. windows, which must still win over the ring: `shell.rs::apply()`
        //    positions a window from the layout's rect but sizes it from
        //    whatever the client actually committed, and a client that's slow
        //    to shrink (or a `--nested` resize still in flight) can briefly
        //    have a surface larger than its placement rect, reaching into the
        //    gap the ring is drawn in. Ring-on-top would paint over that live
        //    content every such frame; windows-on-top instead means the
        //    stale/oversized content can only ever cover the ring, never the
        //    reverse -- the same direction niri itself picks, and the only one
        //    of the two that can't corrupt what a client is showing;
        // 4. the focus ring, drawn in the layout's own gap;
        // 5. the bottom and background layers (a wallpaper), which the
        //    protocol defines as being below windows.
        //
        // The ring sits *between* windows and the background layer rather
        // than below both, which is the one thing
        // `space::space_render_elements` -- which gathers layer surfaces
        // itself, in one fixed order -- cannot express: it would put a
        // full-screen wallpaper on top of the ring, i.e. hide the ring
        // completely for anyone running `swaybg`. So windows come from
        // `window_elements` below (this output's windows only -- see
        // `output_clip.rs`) and the layers are gathered here, around the
        // ring.
        //
        // ...unless the session is locked, in which case this whole list is
        // replaced -- not reordered -- by the lock screen's own (see
        // `session_lock.rs`). Everything above is skipped outright rather
        // than pushed behind an opaque backdrop, because "drawn behind
        // something opaque" is a weaker guarantee than "never gathered": it
        // would rest on element ordering, on no client surface ever being
        // larger than the rect it was placed at, and on the damage tracker
        // never surprising us. The cursor is the one thing still drawn in
        // front, and it is this compositor's own shape
        // (`SessionLockHandler::lock` resets it at lock time) -- a lock screen
        // with a password field needs a pointer.
        if locked {
            let (lock_surfaces, backdrop) =
                self.lock_elements(renderer, output, scale, (width, height));
            out.reserve(cursor_elements.len() + lock_surfaces.len() + 1);
            out.extend(cursor_elements.into_iter().map(Elements::Cursor));
            out.extend(lock_surfaces.into_iter().map(Elements::Surface));
            out.push(Elements::Decoration(backdrop));
        } else {
            let window_elements: Vec<Elements<R>> = match (geometry, arrangement, output_id) {
                (Some(region), Some(arranged), Some(output_id)) => window_elements(
                    &self.space,
                    &self.windows,
                    arranged,
                    output_id,
                    renderer,
                    region,
                    scale,
                    self.appearance.corner_radius,
                ),
                // Unreachable: every output the render loop walks is in the
                // space and in `Outputs`, and the arrangement is only `None`
                // while locked, which is the other branch. An output that
                // isn't in the space has no region to render.
                _ => Vec::new(),
            };
            // While a fullscreen window covers this output, only the
            // overlay layer (notifications, OSDs) stays above it: the top
            // layer (bars) is not drawn at all, rather than drawn behind an
            // opaque window. See `layer_shell::above_windows`, which the
            // pointer and keyboard paths read too, so a hidden bar is never
            // clicked or typed into either.
            let above = layer_shell::above_windows(self.covered_by_fullscreen(output));
            let layers = layer_map_for_output(output);
            out.reserve(
                cursor_elements.len() + window_elements.len() + ring_elements.len() + layers.len(),
            );
            out.extend(cursor_elements.into_iter().map(Elements::Cursor));
            layer_elements(&layers, above, renderer, scale, out);
            out.extend(window_elements);
            out.extend(ring_elements);
            layer_elements(&layers, &layer_shell::BELOW_WINDOWS, renderer, scale, out);
            // Nothing below this point needs the layer map, and the
            // frame-callback pass at the end of `render()` takes the same
            // per-output lock again -- holding this one across the render
            // would deadlock the compositor against itself.
            drop(layers);
        }
        cursor_surface
    }
}

/// One ring element into the frame list. The square path's bars keep their
/// `Decoration` variant; the painted ring has its own.
pub(super) fn map_ring<R: Renderer>(element: RingElement<R>) -> Elements<R> {
    match element {
        RingElement::Rect(bar) => Elements::Decoration(bar),
        RingElement::Painted(ring) => Elements::PaintedRing(*ring),
    }
}

/// This frame's ring, already mapped into frame elements: the painted
/// rounded ring when the session rounds, the four solid bars otherwise,
/// nothing while locked (`arrangement` is `None` then). Only the rings of
/// windows placed on this frame's output, in that output's coordinates --
/// see `output_clip.rs`.
///
/// Shared by both frame bodies (`draw_frame_with` and the scanout tier), so
/// the radius branch cannot drift between them. Built before the framebuffer
/// is bound, like the arrangement it comes from.
pub(super) fn ring_elements<R>(
    decorations: &mut Decorations,
    appearance: &Appearance,
    arrangement: Option<&Arrangement>,
    frame: &FrameContext,
    renderer: &mut R,
) -> Vec<Elements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    match (arrangement, frame.output) {
        (Some(arranged), Some(output)) if appearance.corner_radius > 0 => decorations
            .elements_rounded(
                arranged,
                appearance,
                output,
                frame.bounds(),
                frame.scale,
                renderer,
            )
            .into_iter()
            .map(map_ring)
            .collect(),
        (Some(arranged), Some(output)) => decorations
            .elements(arranged, appearance, output, frame.bounds(), frame.scale)
            .into_iter()
            .map(Elements::Decoration)
            .collect(),
        _ => Vec::new(),
    }
}

/// The windows this frame draws, front-most first: every window placed on
/// `output`, and no other (see `output_clip.rs`).
///
/// Walks the arrangement rather than asking `Space::render_elements_for_region`
/// for everything overlapping the region, because the region test is the
/// bleed: a column scrolled half off the neighbouring output, or a
/// fullscreen window focused away from, overlaps this output without being
/// on it. The output filter is the only difference in *which* windows are
/// drawn; order, the bbox-overlap filter, positioning and alpha replicate
/// `render_elements_for_region` exactly (storage order reversed -- `apply()`
/// maps the visible placements in arrangement order, re-inserting each at
/// the top, so the two orders agree; render location minus the region;
/// `1.0` alpha). The `element_location` lookup is also what skips a
/// placement `apply()` has not mapped (an invisible one).
///
/// Square (`configured_radius == 0`, the default), each window's elements
/// are its own `AsRenderElements` output -- popups first, then its surface
/// tree -- so a single-output session draws exactly what it drew through
/// `render_elements_for_region`.
///
/// Rounded, each window's toplevel tree is clipped to its own rounded rect
/// while its popups stay square: the same split `Window`'s own impl makes at
/// the pinned rev (`desktop/space/wayland/window.rs`), popups first, with a
/// [`Rounded`] wrap on the toplevel half. A window whose effective radius is
/// zero pushes its elements plain, so tiny windows cost nothing. The clip
/// comes from the arrangement placement (the layout rect the ring is painted
/// from too), not from the drawn surface: both edges then coincide by
/// construction. A surface temporarily larger than its placement (a shrink
/// still in flight) is cut to the placement rather than bleeding into the
/// gap -- a behavior change, but only with rounding opted in. A fullscreen
/// window is pushed plain, never wrapped: its corners are the output's
/// corners.
///
/// Both clip and location are in *this output's* coordinates (the placement
/// minus the region's origin), which is what the framebuffer and the damage
/// tracker are in. The first output sits at the origin, so for it this is
/// the identity.
#[allow(clippy::too_many_arguments)]
fn window_elements<R>(
    space: &Space<Window>,
    windows: &HashMap<WindowId, Window>,
    arrangement: &Arrangement,
    output: OutputId,
    renderer: &mut R,
    region: Rectangle<i32, Logical>,
    scale: f64,
    configured_radius: i32,
) -> Vec<Elements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    let mut out = Vec::new();
    let origin = Rect::new(region.loc.x, region.loc.y, region.size.w, region.size.h);
    // Back to front in storage, so reversed here: the same order
    // `render_elements_for_region` gathers in.
    for placement in arrangement.placements.iter().rev() {
        if placement.output != output {
            continue;
        }
        let Some(window) = windows.get(&placement.id) else {
            continue;
        };
        // The mapped location, and from it the render location and the bbox
        // (popups included) exactly as `InnerElement::render_location` and
        // `InnerElement::bbox` compute them -- one space lookup rather than
        // the two `element_location` + `element_bbox` would cost.
        let Some(mapped) = space.element_location(window) else {
            continue;
        };
        let geometry = window.geometry();
        let render_location = mapped - geometry.loc;
        // `SpaceElement::bbox` (popups included), not the inherent
        // `Window::bbox` (the toplevel tree only), which is the one
        // `InnerElement::bbox` reads.
        let mut bbox = SpaceElement::bbox(window);
        bbox.loc += render_location;
        if !region.overlaps(bbox) {
            continue;
        }
        let location = (render_location - region.loc).to_physical_precise_round(scale);
        if configured_radius <= 0 {
            out.extend(
                AsRenderElements::<R>::render_elements::<WaylandSurfaceRenderElement<R>>(
                    window,
                    renderer,
                    location,
                    Scale::from(scale),
                    1.0,
                )
                .into_iter()
                .map(Elements::Surface),
            );
            continue;
        }
        // No X11 arm: without the `xwayland` feature `Wayland` is the only
        // variant -- and if that ever changes this match fails to compile
        // rather than silently dropping windows. With the feature the `X11`
        // variant exists, and the Phase-1 skeleton answers it loudly: no
        // X11 window can exist yet (nothing constructs
        // `Window::new_x11_window` until Phase 2 maps one), so reaching
        // here is a bug, and a bug that logs per frame beats one that
        // silently drops the window -- or one that panics the session.
        #[cfg(not(feature = "xwayland"))]
        let WindowSurface::Wayland(toplevel) = window.underlying_surface();
        #[cfg(feature = "xwayland")]
        let WindowSurface::Wayland(toplevel) = window.underlying_surface() else {
            tracing::error!(
                "an X11 window reached the render path before Phase 2 maps one; skipping it"
            );
            continue;
        };
        let surface = toplevel.wl_surface();
        for (popup, popup_offset) in PopupManager::popups_for_surface(surface) {
            let offset = (geometry.loc + popup_offset - popup.geometry().loc)
                .to_physical_precise_round(scale);
            out.extend(
                render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    location + offset,
                    scale,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(Elements::Surface),
            );
        }
        let main: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
            renderer,
            surface,
            location,
            scale,
            1.0,
            Kind::Unspecified,
        );
        let clip = clip_rect(to_output_local(placement.rect, origin), scale);
        // Never rounded while fullscreen: it covers the output edge to edge,
        // and a rounded clip would cut its corners back to the background.
        let radius = if placement.fullscreen {
            0
        } else {
            physical_radius(configured_radius, clip, scale)
        };
        if radius > 0 {
            out.extend(
                main.into_iter()
                    .map(|element| Elements::RoundedSurface(Rounded::new(element, clip, radius))),
            );
        } else {
            out.extend(main.into_iter().map(Elements::Surface));
        }
    }
    out
}

/// Appends the render elements of every mapped layer surface on `layers`,
/// front-most first, to `elements`.
///
/// Within one layer the most recently mapped surface wins, which is what
/// `layers_on(..).rev()` gives (the map keeps insertion order) and what
/// Smithay's own `space_render_elements` does with the same list. The
/// protocol itself leaves ordering *within* a layer undefined, so this is a
/// choice, not a rule -- but it is the same choice every wlroots-derived
/// compositor makes, and the one a client expects when it maps a second
/// surface on the same layer.
///
/// Allocates only when there is something to draw: `render_elements` returns
/// a `Vec` per surface (Smithay's own signature), so a session with no bars
/// or wallpaper -- the default -- adds no allocation to the frame at all.
fn layer_elements<R>(
    layers: &LayerMap,
    which: &[Layer],
    renderer: &mut R,
    scale: f64,
    elements: &mut Vec<Elements<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    for &layer in which {
        for surface in layers.layers_on(layer).rev() {
            // `layer_geometry` is `None` only for a surface this map never
            // mapped, which `layers_on` cannot produce.
            let Some(geometry) = layers.layer_geometry(surface) else {
                continue;
            };
            elements.extend(
                AsRenderElements::<R>::render_elements::<WaylandSurfaceRenderElement<R>>(
                    surface,
                    renderer,
                    geometry.loc.to_physical_precise_round(scale),
                    Scale::from(scale),
                    1.0,
                )
                .into_iter()
                .map(Elements::Surface),
            );
        }
    }
}
