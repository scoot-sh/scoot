//! The pointer cursor drawn on `--tty` real hardware.
//!
//! `--headless` has no display to draw one on, and `--nested` already shows
//! the host compositor's own cursor on top -- so this backend-neutral module
//! knows nothing about which backend is active; `headless.rs::render` is the
//! one that gates its use to `self.tty.is_some()`.
//!
//! # Two sources of cursor pixels
//!
//! A client asks for a cursor image through `wl_pointer.set_cursor`, which
//! Smithay turns into a [`CursorImageStatus`]:
//!
//! - [`CursorImageStatus::Surface`] -- the client handed over a real
//!   `wl_surface` with its own buffer (an I-beam over a text field, a resize
//!   arrow on a window edge, an animated spinner). Those pixels are the
//!   client's, so they're what gets drawn: [`Cursor::element`] renders that
//!   surface's whole subsurface tree, at the hotspot the client set.
//! - [`CursorImageStatus::Named`] -- the client named a shape but supplied no
//!   pixels, so there is nothing of the client's to draw. One of this
//!   compositor's own procedurally-generated bitmaps is drawn instead,
//!   chosen by [`shapes::Shape::for_icon`]: an I-beam for `text`, a
//!   double-headed arrow for a resize edge, a crosshair, and so on, with the
//!   arrow below as the answer for every name none of those fits. Their size
//!   and fill color are configurable (`[appearance]`'s
//!   `cursor_size`/`cursor_color`, resolved once at startup -- see
//!   [`Cursor::new`]).
//! - [`CursorImageStatus::Hidden`] -- nothing is drawn.
//!
//! A client reaches the `Named` path either through `wl_pointer.set_cursor`
//! with no surface, or -- since `wp-cursor-shape-v1` is advertised (see
//! `state.rs`'s `cursor_shape_manager_state`) -- by naming a shape directly
//! and never allocating a cursor buffer at all. Both arrive here as the same
//! [`CursorImageStatus::Named`], which is the point of the protocol: the
//! compositor's own shapes, consistently, across every client that asks.
//!
//! Every one of those bitmaps is procedurally generated, not an embedded
//! image file or a copy of any cursor theme's actual pixel data -- niri's own
//! cursor assets are GPL, Adwaita's aren't MIT-clean, and `CLAUDE.md`'s
//! license note says not to borrow either. They are line art, not
//! pixel-accurate theme shapes, and no config option can make them one: there
//! is no `cursor_theme` field here, because honoring a theme name means
//! loading a real xcursor asset, and this project has no MIT-clean one to
//! load. Nothing here loads or ships a theme asset; a client surface's pixels
//! come from the client, over the wire. See `cursor/shapes.rs` for how each
//! shape is drawn.
//!
//! # Renderer-generic, like `decorations.rs`
//!
//! Everything here is generic over `R` rather than fixed to `PixmanRenderer`,
//! so a future GPU backend needs no change in this module. That's why
//! [`CursorElement`] exists: the two sources above produce two different
//! concrete render-element types, and a `match` can't return one-of-two, so
//! they're wrapped in one enum the caller can treat uniformly.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Kind, render_elements};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Transform};
use smithay::wayland::compositor::with_states;

use super::decorations::{Appearance, Color};
use shapes::Shape;
use theme::{Theme, Themed};

pub mod shapes;
pub mod theme;

#[cfg(test)]
mod tests;

// Either source of cursor pixels, in one type. `Surface` is one node of a
// client cursor surface's subsurface tree -- a tree is spec-legal and rare,
// so `Cursor::element` returns a list of these rather than a single one.
render_elements! {
    pub CursorElement<R> where R: ImportAll + ImportMem;
    Fallback = MemoryRenderBufferRenderElement<R>,
    Surface = WaylandSurfaceRenderElement<R>,
}

/// A filled right triangle `size` x `size`, point at the top-left corner
/// (which is also the hotspot -- see [`Cursor::new`]): a 1px `outline` on the
/// left edge and the diagonal, `fill` between them, transparent everywhere
/// else.
///
/// Both colors are single `Argb8888` pixels in little-endian memory order
/// (`[B, G, R, A]`, premultiplied) -- the same layout `headless.rs` renders
/// into and `tty/buffers.rs` scans out (see their module docs on the same
/// fact), so this needs no conversion anywhere downstream.
/// [`Color::to_argb8888`] is what produces one from a config color; this
/// function stays pure and takes whatever it is given, so it is testable
/// without a renderer and its tests can assert exact bytes.
///
/// `size` is expected in
/// [`Appearance::MIN_CURSOR_SIZE`]`..=`[`Appearance::MAX_CURSOR_SIZE`] --
/// [`Cursor::new`], the only non-test caller, clamps it there, which is what
/// keeps `size * size * 4` both a small allocation and nowhere near
/// overflowing `i32` (see [`Appearance::MAX_CURSOR_SIZE`]).
fn generate_bitmap(size: i32, fill: [u8; 4], outline: [u8; 4]) -> Vec<u8> {
    let transparent = [0u8; 4];
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let pixel: [u8; 4] = if x == 0 || x == y {
                outline
            } else if x < y {
                fill
            } else {
                transparent
            };
            pixels[idx..idx + 4].copy_from_slice(&pixel);
        }
    }
    pixels
}

/// The cursor's current image request and the persistent render buffers
/// behind this compositor's own shapes. Those are built once (see
/// [`Cursor::new`]) and never rebuilt -- same stable-`Id` reasoning as
/// `decorations.rs`'s persistent per-window buffers, so a static cursor
/// doesn't read as "new content" to the damage tracker on every frame it
/// happens to still be visible, and a client flipping between `default` and
/// `text` as the pointer crosses a text field re-uses two buffers rather than
/// allocating on the way past. A client-supplied cursor surface needs no
/// equivalent here: its texture is owned and cached by Smithay's own
/// per-surface renderer state, keyed off the client's commits.
pub struct Cursor {
    /// One bitmap per [`Shape`], indexed by [`Shape::index`].
    ///
    /// Built eagerly rather than on first use, for the stable-`Id` reason
    /// above -- a buffer created mid-session is new content to the damage
    /// tracker at the moment the pointer is moving fastest -- and because
    /// the whole set is small: `Shape::COUNT` bitmaps of `size * size * 4`
    /// bytes, i.e. ~10 KiB at the default 16px cursor and ~2.6 MiB at the
    /// largest size `Appearance::MAX_CURSOR_SIZE` allows, once, for the
    /// process's lifetime.
    shapes: [MemoryRenderBuffer; Shape::COUNT],
    /// The clamped edge length every bitmap in [`Self::shapes`] was built at,
    /// kept so [`Shape::hotspot`] can be asked about them. Not a hotspot
    /// itself: each shape has its own (the arrow points at its top-left
    /// corner, the symmetric shapes at their middle), and none of them is the
    /// hotspot of a client-supplied cursor surface -- that one belongs to the
    /// client, lives in the surface's own user data, and is read per frame by
    /// [`surface_hotspot`].
    size: i32,
    status: CursorImageStatus,
    /// The machine's own installed cursor theme, and everything drawn from it
    /// so far. Empty on a machine with no icon theme, which is the ordinary
    /// state in a container -- see `cursor/theme.rs`.
    theme: Theme,
    /// The theme image for the *current* [`Self::status`], resolved once when
    /// that status changes rather than per frame.
    ///
    /// The invariant that makes this safe to read in [`Cursor::element`]:
    /// **every write to `status` goes through [`Cursor::set_named`] or
    /// [`Cursor::set_status`], and both rewrite this field in the same
    /// statement.** There is no third write site, and adding one without
    /// refreshing this would leave the previous shape's pixels drawn under
    /// the new shape's name -- the single most likely way this module could
    /// silently draw the wrong thing.
    ///
    /// `None` means "no theme image for this status": a hidden cursor, a
    /// client surface, or a named shape the theme does not carry. All three
    /// fall through to [`Self::shapes`].
    themed: Option<Themed>,
}

impl Cursor {
    /// Builds the fallback bitmap from the resolved `[appearance]` values and
    /// keeps it for the process's lifetime.
    ///
    /// Called once, from `State::new`, with `appearance.cursor_size` and
    /// `appearance.cursor_color`. There is deliberately no way to rebuild it
    /// afterwards: nothing in this project reloads config after startup, and
    /// inventing a path for it here would be an abstraction with no caller.
    ///
    /// `size` is put through [`Appearance::clamp_cursor_size`] again rather
    /// than trusted. `Appearance::clamped` is the load-time gate (and the
    /// place a config value out of range gets its warning), but
    /// [`Appearance`]'s fields are public and *this* is where `size * size *
    /// 4` bytes are actually allocated and where Smithay asserts the slice is
    /// long enough for the size it was told -- so the bound is re-applied at
    /// the allocation, the same way `decorations::ring_rects` re-checks
    /// `width <= 0` at its own arithmetic rather than trusting its caller.
    ///
    /// The outline is always black, at the fill's own alpha, rather than a
    /// second config field: its whole job is to keep the shape's edges legible
    /// against content of a similar color, which a configurable outline could
    /// only undo, and at the default opaque fill this is byte-identical to the
    /// fixed black outline this shape has always had. Matching the fill's
    /// alpha is what makes a translucent `cursor_color` actually look
    /// translucent instead of an opaque black triangle outline around a
    /// see-through middle.
    pub fn new(size: i32, color: Color, theme_name: Option<&str>) -> Self {
        let size = Appearance::clamp_cursor_size(size);
        let outline = Color::new(0.0, 0.0, 0.0, color.a);
        let (fill, outline) = (color.to_argb8888(), outline.to_argb8888());
        let theme = Theme::load(theme_name, size);
        let mut cursor = Self {
            // `Shape::ALL` in its own order, indexed back by `Shape::index`
            // -- see that constant's doc for why the order lives there and
            // not here. `Shape::Arrow` is the one shape `shapes::generate`
            // does not draw: it is `generate_bitmap` above, kept exactly as
            // it has always been (see `shapes`'s module doc on the two
            // outline styles), so it is routed here rather than there.
            shapes: Shape::ALL.map(|shape| {
                let pixels = match shape {
                    Shape::Arrow => generate_bitmap(size, fill, outline),
                    other => shapes::generate(other, size, fill, outline),
                };
                MemoryRenderBuffer::from_slice(
                    &pixels,
                    Fourcc::Argb8888,
                    (size, size),
                    1,
                    Transform::Normal,
                    None,
                )
            }),
            size,
            status: CursorImageStatus::default_named(),
            theme,
            themed: None,
        };
        // The default status is a *named* shape, so the themed image for it
        // has to be resolved here too -- the field's invariant is "every
        // write to `status` refreshes this", and the struct literal above is
        // one such write.
        cursor.refresh_themed();
        cursor
    }

    /// Whether a real cursor theme was found, i.e. whether a named shape is
    /// drawn from the machine's own theme rather than from `shapes.rs`.
    /// Read only by tests and by `compositor::run`'s environment export.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;
        self.refresh_themed();
    }

    /// Re-resolves [`Self::themed`] for whatever [`Self::status`] now is.
    ///
    /// The one place that field is written. Called from every site that
    /// writes `status`, which is what keeps the two from disagreeing -- see
    /// the field's doc. Loading happens here, on the event that changed the
    /// shape, and never on the render path; a shape already looked up (or
    /// already known missing) costs one hash lookup.
    fn refresh_themed(&mut self) {
        self.themed = match &self.status {
            CursorImageStatus::Named(icon) => self.theme.image(*icon).cloned(),
            // A client surface draws the client's own pixels, and a hidden
            // cursor draws nothing; neither has a theme image, and leaving a
            // stale one here is exactly the bug the invariant exists to stop.
            CursorImageStatus::Surface(_) | CursorImageStatus::Hidden => None,
        };
    }

    /// Drops `surface` as the active cursor image if that's what it was,
    /// answering whether it had to -- i.e. whether the cursor now looks
    /// different and the screen needs redrawing.
    ///
    /// Called from `CompositorHandler::destroyed`: a client may destroy the
    /// `wl_surface` it gave as its cursor without ever setting a
    /// replacement, and nothing in the pinned Smithay rev resets
    /// [`CursorImageStatus`] when that happens (its `wl_pointer` handler
    /// tracks the `WlPointer` object's own destruction, not the cursor
    /// surface's), so this compositor would otherwise hold a dead surface as
    /// the active cursor indefinitely. Falling back to the default named
    /// shape -- rather than to `Hidden` -- keeps a pointer on screen: the
    /// pointer still exists and is still being moved around, and a cursor
    /// that vanishes because a client tore down a surface is a worse outcome
    /// than the wrong shape.
    pub fn forget_surface(&mut self, surface: &WlSurface) -> bool {
        if !matches!(&self.status, CursorImageStatus::Surface(active) if active == surface) {
            return false;
        }
        // Through the one setter, not a bare field write: this is the second
        // site that changes `status`, and `themed` has to follow it (see that
        // field's invariant).
        self.set_status(CursorImageStatus::default_named());
        true
    }

    /// The client surface this frame's cursor is drawn from, if the cursor
    /// is a live client surface at all.
    ///
    /// [`Cursor::element`] renders exactly this surface's tree whenever this
    /// returns `Some`, and never touches a client surface when it returns
    /// `None` -- both go through [`Cursor::live_surface`], so the two can't
    /// disagree. `headless.rs::render` relies on that: it sends frame
    /// callbacks to this surface after presenting a frame, and a callback
    /// for a surface that wasn't drawn (or a missing one for a surface that
    /// was) is what stalls an animated client cursor forever.
    pub fn surface(&self) -> Option<&WlSurface> {
        self.live_surface()
    }

    /// `Some` only when the active status is a client surface that is still
    /// alive. A destroyed surface deliberately reads as `None` here --
    /// `forget_surface` above normally clears it first, but that depends on
    /// `dispatch.rs` forwarding `Dispatch::destroyed`, so this second check
    /// keeps a dead surface out of the render path even if that link ever
    /// breaks.
    fn live_surface(&self) -> Option<&WlSurface> {
        match &self.status {
            CursorImageStatus::Surface(surface) if surface.alive() => Some(surface),
            _ => None,
        }
    }

    /// This frame's cursor render elements at `pointer_location`, front-most
    /// first: empty when the cursor is hidden (or when a client cursor
    /// surface has no content to show yet), one element for whichever of
    /// this compositor's own shapes the client named, and one per mapped
    /// node of a client cursor surface's tree.
    ///
    /// Failing to build the shape element (one of the fixed bitmaps failing
    /// to import) is logged and dropped here rather than returned: the client
    /// surface path can't report failures the same way -- Smithay's
    /// `render_elements_from_surface_tree` logs a failed import itself and
    /// simply omits that node -- so a `Result` out of this function would
    /// only ever describe one of the two paths. Either way the outcome is
    /// the same and is never fatal: no cursor this frame, not a skipped
    /// frame.
    pub fn element<R>(
        &self,
        renderer: &mut R,
        pointer_location: Point<f64, Logical>,
        scale: f64,
    ) -> Vec<CursorElement<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        if let Some(surface) = self.live_surface() {
            // Re-read per frame, never cached: a client may call
            // `set_cursor` again with the *same* surface and a different
            // hotspot, which re-enters `SeatHandler::cursor_image` with a
            // `CursorImageStatus::Surface` that compares equal to the
            // previous one (the enum carries the surface, not the hotspot),
            // so "the status didn't change" does not mean "the hotspot
            // didn't change".
            let hotspot = surface_hotspot(surface);
            let location = element_location(pointer_location, hotspot, scale).to_i32_round();
            // `scale` (the output scale), not 1.0: a client cursor surface is
            // drawn at the same scale as everything else, so its own
            // fractional-scale/viewport state lands it at the right physical
            // size and `Kind::Cursor` still lets a damage tracker treat it as
            // cursor content. `element_location` puts the origin in the same
            // physical space.
            return render_elements_from_surface_tree(
                renderer,
                surface,
                location,
                scale,
                1.0,
                Kind::Cursor,
            );
        }
        let icon = match &self.status {
            CursorImageStatus::Hidden => return Vec::new(),
            CursorImageStatus::Named(icon) => *icon,
            // A client cursor surface that is no longer alive -- the live
            // one returned above. Drawn as the default shape rather than as
            // nothing, for the same reason `forget_surface` falls back to it
            // rather than to `Hidden`: the pointer still exists and is still
            // being moved, so the wrong shape beats no cursor at all.
            CursorImageStatus::Surface(_) => CursorIcon::Default,
        };
        // The machine's own theme first, this module's drawn shapes second.
        // `themed` was resolved when the status changed, so this is a field
        // read, not a lookup -- see `refresh_themed`. It is deliberately not
        // consulted on the dead-surface path above: `themed` is `None` for a
        // `Surface` status by construction, so that path always draws the
        // arrow, which is the defensive fallback `forget_surface` normally
        // replaces within the same dispatch.
        let (buffer, hotspot) = match &self.themed {
            Some(themed) => (&themed.buffer, themed.hotspot),
            None => {
                let shape = Shape::for_icon(icon);
                (&self.shapes[shape.index()], shape.hotspot(self.size))
            }
        };
        let location = element_location(pointer_location, hotspot, scale);
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            buffer,
            None,
            None,
            None,
            Kind::Cursor,
        ) {
            Ok(element) => vec![CursorElement::Fallback(element)],
            Err(error) => {
                tracing::warn!(%error, ?icon, "could not build the cursor element");
                Vec::new()
            }
        }
    }
}

/// The hotspot a client last set for its cursor `surface`, or the surface's
/// origin if it has none recorded yet.
///
/// Smithay's `wl_pointer.set_cursor` handler stores this in the surface's own
/// user data as a [`CursorImageSurfaceData`]; it is absent only if that
/// handler never ran for this surface, which `Cursor`'s status can't
/// currently reflect (the status only ever becomes `Surface` from inside that
/// handler) -- `(0, 0)` is the same value the handler itself seeds.
///
/// Deliberately does no rendering inside the `with_states` closure:
/// `with_states` holds a plain, non-reentrant `Mutex` on this surface's data,
/// and walking the surface tree to render it locks the same one.
fn surface_hotspot(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .and_then(|attributes| attributes.lock().ok())
            .map(|attributes| attributes.hotspot)
    })
    .unwrap_or_default()
}

/// Where the render element's origin belongs given the pointer's own
/// location, the image's hotspot and the output scale -- pulled out of
/// `element` so this arithmetic is testable without a live renderer, same
/// rationale as `input.rs`'s `clamp_to_extent`.
///
/// Physical, not Logical, matches what the render-element constructors want
/// for their location (renderer output space). The pointer and the hotspot
/// are both logical, so both are scaled: subtracting first and scaling the
/// difference keeps the hotspot offset exact at a fractional scale, and at
/// 1.0 the multiplication is the identity, which is what keeps a scale-1
/// session byte-identical to before output scaling existed.
fn element_location(
    pointer: Point<f64, Logical>,
    hotspot: Point<i32, Logical>,
    scale: f64,
) -> Point<f64, Physical> {
    Point::from((
        (pointer.x - f64::from(hotspot.x)) * scale,
        (pointer.y - f64::from(hotspot.y)) * scale,
    ))
}
