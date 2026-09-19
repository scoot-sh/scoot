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

use scoot_core::Rect;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{AsRenderElements, render_elements};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::{LayerMap, layer_map_for_output};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle, Scale};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::compositor::State;
use crate::compositor::cursor::CursorElement;
use crate::compositor::layer_shell;

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
    /// Everything this frame draws, front-most first, plus the client cursor
    /// surface it drew from if there was one.
    ///
    /// `ring_elements` is built by the caller rather than here, because it
    /// needs no renderer and is deliberately computed *before* the
    /// framebuffer is bound -- see `super::draw_frame_with`.
    ///
    /// The returned surface is set only on the frames that actually went
    /// looking for one (`--tty` with a pointer); it is what the frame
    /// callback pass at the end of `render()` wakes.
    pub(super) fn gather_elements<R>(
        &mut self,
        renderer: &mut R,
        output: &Output,
        frame: &FrameContext,
        ring_elements: Vec<SolidColorRenderElement>,
    ) -> (Vec<Elements<R>>, Option<WlSurface>)
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let FrameContext {
            size: (width, height),
            scale,
            geometry,
            locked,
        } = *frame;
        // The client-supplied cursor surface this frame drew from, if any.
        let mut cursor_surface: Option<WlSurface> = None;
        // Only `--tty` ever draws a cursor -- see `cursor.rs`'s module doc;
        // headless has no display and `--nested` already shows the host's
        // own. The list is empty when there's nothing to draw (hidden, or a
        // client cursor surface with no content yet) and can hold more than
        // one element when a client's cursor surface has subsurfaces of its
        // own.
        let cursor_elements = if self.tty.is_some() {
            match self.seat.get_pointer() {
                Some(pointer) => {
                    cursor_surface = self.cursor.surface().cloned();
                    self.cursor
                        .element(renderer, pointer.current_location(), scale)
                }
                None => Vec::new(),
            }
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
        //    notification);
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
        // `Space::render_elements_for_region` (windows only, by construction
        // -- see its own doc) and the layers are gathered here, around the
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
        let elements = if locked {
            let origin = geometry
                .map(|geometry| geometry.loc.to_physical_precise_round(scale))
                .unwrap_or_default();
            let (lock_surfaces, backdrop) =
                self.lock_elements(renderer, origin, scale, (width, height));
            let mut elements = Vec::with_capacity(cursor_elements.len() + lock_surfaces.len() + 1);
            elements.extend(cursor_elements.into_iter().map(Elements::Cursor));
            elements.extend(lock_surfaces.into_iter().map(Elements::Surface));
            elements.push(Elements::Decoration(backdrop));
            elements
        } else {
            let window_elements = match geometry {
                Some(region) => self
                    .space
                    .render_elements_for_region(renderer, &region, scale, 1.0),
                // Unreachable while `output` is the primary output
                // `headless::init_named` mapped into the space; an output that
                // isn't in the space has no region to render.
                None => Vec::new(),
            };
            let layers = layer_map_for_output(output);
            let mut elements = Vec::with_capacity(
                cursor_elements.len() + window_elements.len() + ring_elements.len() + layers.len(),
            );
            elements.extend(cursor_elements.into_iter().map(Elements::Cursor));
            layer_elements(
                &layers,
                &layer_shell::ABOVE_WINDOWS,
                renderer,
                scale,
                &mut elements,
            );
            elements.extend(window_elements.into_iter().map(Elements::Surface));
            elements.extend(ring_elements.into_iter().map(Elements::Decoration));
            layer_elements(
                &layers,
                &layer_shell::BELOW_WINDOWS,
                renderer,
                scale,
                &mut elements,
            );
            // Nothing below this point needs the layer map, and the
            // frame-callback pass at the end of `render()` takes the same
            // per-output lock again -- holding this one across the render
            // would deadlock the compositor against itself.
            drop(layers);
            elements
        };
        (elements, cursor_surface)
    }
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
