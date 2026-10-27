//! The pointer cursor drawn on `--tty` real hardware.
//!
//! `--headless` has no display to draw one on, and `--nested` already shows
//! the host compositor's own cursor on top -- so this backend-neutral module
//! knows nothing about which backend is active; `render/elements.rs`'s
//! `State::frame_draws_cursor` is the one that gates its use in frames to
//! `--tty`. Captures are the exception: one that asks for the pointer gets
//! these same elements drawn into it on every backend
//! (`render/capture_cursor.rs`).
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
//!   compositor's own bitmaps is drawn instead,
//!   chosen by [`shapes::Shape::for_icon`]: an I-beam for `text`, a
//!   double-headed arrow for a resize edge, a crosshair, and so on, with the
//!   arrow below as the answer for every name none of those fits. Their size
//!   and fill color are configurable (`[appearance]`'s
//!   `cursor_size`/`cursor_color`, built at startup and rebuilt by
//!   [`Cursor::rebuild`] on a config reload -- see [`Cursor::new`]).
//! - [`CursorImageStatus::Hidden`] -- nothing is drawn.
//!
//! A client reaches the `Named` path either through `wl_pointer.set_cursor`
//! with no surface, or -- since `wp-cursor-shape-v1` is advertised (see
//! `state.rs`'s `cursor_shape_manager_state`) -- by naming a shape directly
//! and never allocating a cursor buffer at all. Both arrive here as the same
//! [`CursorImageStatus::Named`], which is the point of the protocol: the
//! compositor's own shapes, consistently, across every client that asks.
//!
//! Those bitmaps are Mocu (Severin Meyer's "modest xcursor theme", CC0 1.0
//! -- public domain, so MIT-compatible with no strings; its SVG sources are
//! vendored unchanged with their license under `cursor/shapes/mocu/` and
//! credited in `NOTICE`), rasterised at startup by scoot itself at each
//! output's own pixel size -- not loaded from an image file. Nothing is taken
//! from niri's cursor assets (GPL) or Adwaita's (not MIT-clean) --
//! `CLAUDE.md`'s license note says not to borrow either.
//!
//! What *is* honored, when available, is the machine's own already-installed
//! theme -- reading it carries no licensing obligation, unlike shipping one
//! (see `cursor/theme.rs`, which resolves `[appearance] cursor_theme` /
//! `$XCURSOR_THEME` and loads that theme's real artwork through the
//! MIT-licensed `xcursor` crate). [`Cursor::theme`] holds it; a `Named`
//! status is drawn from it when it has that shape, falling back to the
//! drawn bitmaps above otherwise -- a machine with no theme installed at
//! all, such as a webtop or minimal container, is the everyday case that
//! fallback exists for. See `cursor/shapes.rs` for how each drawn shape is
//! generated.
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
use smithay::reexports::calloop::channel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{SurfaceAttributes, get_parent, with_states};

use super::decorations::{Appearance, Color};
use shapes::Shape;
use theme::{Theme, Themed};

#[cfg(feature = "gpu-scanout")]
pub mod plane;
pub mod raster;
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
    // A drawn shape with a dma-buf twin an overlay plane can scan out, where
    // the CRTC has no cursor plane. Never built here: `render::cursor_plane`
    // swaps it in for `Fallback` on the outputs that can use it.
    #[cfg(feature = "gpu-scanout")]
    Plane = plane::PlaneCursorElement<R>,
}

/// The fill/stroke byte pair one configured cursor color draws as: the fill
/// is the color itself, the stroke its [`contrast_rim`]. One function so
/// `new` and `rebuild` cannot disagree about it.
fn paints(color: Color) -> ([u8; 4], [u8; 4]) {
    (color.to_argb8888(), contrast_rim(color).to_argb8888())
}

/// The stroke color Mocu pairs with a fill: its white theme's `#222222`
/// around a light fill, its black theme's `#f9f9f9` around a dark one, at
/// the fill's own alpha.
///
/// The split is on the fill's gamma-space luminance against a 0.45
/// threshold: Mocu's white (`#fcfcfc`) gets the dark stroke, Mocu's black
/// (`#111111`) the light one, and a mid-gray fill (`#808080`, luminance
/// ~0.50) still gets the dark stroke -- a light rim around mid-gray glares,
/// while a dark one reads as definition. Matching the fill's alpha is what
/// makes a translucent `cursor_color` actually look translucent instead of
/// an opaque rim around a see-through middle.
pub fn contrast_rim(fill: Color) -> Color {
    let luminance = 0.2126 * fill.r + 0.7152 * fill.g + 0.0722 * fill.b;
    let v: u8 = if luminance < 0.45 { 0xf9 } else { 0x22 };
    let c = f32::from(v) / 255.0;
    Color::new(c, c, c, fill.a)
}

/// The cursor's current image request and the persistent render buffers
/// behind this compositor's own shapes.
///
/// The buffers are scale-aware: one bitmap set per output scale in use (see
/// [`physical_size`]), built on cold paths only -- startup, reload, output
/// add and scale change -- and selected per frame without allocating. Same
/// stable-`Id` reasoning as `decorations.rs`'s persistent per-window buffers,
/// per scale: a static cursor doesn't read as "new content" to the damage
/// tracker on every frame it happens to still be visible, and a client
/// flipping between `default` and `text` as the pointer crosses a text field
/// re-uses two buffers rather than allocating on the way past. A
/// client-supplied cursor surface needs no equivalent here: its texture is
/// owned and cached by Smithay's own per-surface renderer state, keyed off
/// the client's commits.
pub struct Cursor {
    /// One [`ScaleEntry`] per output scale in use, in first-use order.
    ///
    /// Built eagerly for scale 1.0 and for every live output scale (see
    /// [`Cursor::sync_scales`]), never on the frame path, and bounded by
    /// [`MAX_SCALES`]: the whole set is small (`Shape::COUNT` bitmaps of
    /// `physical * physical * 4` bytes per entry -- 34,816 B per scale at
    /// the default 16px cursor, 139,264 B at 2x; 252,416 B for 16+24+32,
    /// 306,816 B for four distinct scales at logical 16 (16/20/24/32) --
    /// per cold-path build rather than per frame (`rebuild` replaces the
    /// whole set at once). The bound is on the count of scales, not on
    /// bytes: at logical 256 four scales (physical 256/320/384/512) hold
    /// 78,544,896 B (~75 MiB), and rebuilding that set costs seconds --
    /// which is why a reload builds it on a worker and swaps it in
    /// (see `reload.rs`), never on the loop thread.
    scales: Vec<ScaleEntry>,
    /// The clamped logical edge length every entry is drawn for, kept so the
    /// element can be sized in logical units. Not a hotspot itself: each
    /// shape has its own (the arrow points at its top-left corner, the
    /// symmetric shapes at their middle), and none of them is the hotspot of
    /// a client-supplied cursor surface -- that one belongs to the client,
    /// lives in the surface's own user data, and is read per frame by
    /// [`surface_hotspot`].
    size: i32,
    /// The paints [`ScaleEntry`] bitmaps are built from, kept so
    /// [`Cursor::rebuild`] and [`Cursor::ensure_scale`] build the same
    /// pixels without re-deriving them.
    fill: [u8; 4],
    rim: [u8; 4],
    status: CursorImageStatus,
    /// The machine's own installed cursor theme, and everything drawn from it
    /// so far. Empty on a machine with no icon theme, which is the ordinary
    /// state in a container -- see `cursor/theme.rs`.
    theme: Theme,
}

/// What a cursor rebuild needs that the loop thread already holds: the new
/// paints plus the bitmap set to build. Cheap to move onto a worker; the
/// bitmaps are built from it, never from compositor state.
pub struct RebuildInput {
    /// The new logical edge length (clamped like [`Cursor::new`]).
    pub size: i32,
    /// The new fill color.
    pub color: Color,
    /// The new theme name.
    pub theme_name: Option<String>,
    /// The named shape showing now, if the cursor shows one: what the
    /// worker warms the fresh theme's cache with.
    pub icon: Option<CursorIcon>,
    /// The output scales to build alongside scale 1.0, in first-use order.
    pub scales: Vec<f64>,
}

/// A worker-built cursor bitmap set: installed by
/// [`Cursor::install_rebuilt`] in one atomic swap. Everything the loop
/// thread's `rebuild` replaced, built off it.
pub struct RebuiltCursor {
    size: i32,
    fill: [u8; 4],
    rim: [u8; 4],
    theme: Theme,
    scales: Vec<ScaleEntry>,
}

/// A finished off-loop cursor rebuild, with the generation it was built
/// for: what a cursor worker sends back, and what the loop installs when it
/// is still the newest (see `reload.rs`).
pub struct CursorBuilt {
    /// The [`State::cursor_generation`] this rebuild was spawned for.
    pub generation: u64,
    /// The bitmap set to swap in.
    pub rebuilt: RebuiltCursor,
}

/// The sending end of the channel cursor rebuild workers answer through.
/// One per `State`, created lazily with its event source and kept across
/// rebuilds -- so a restart reuses the one registered source rather than
/// abandoning one per generation (the screenshot worker's arrangement).
pub struct CursorSink {
    pub(crate) done: channel::Sender<CursorBuilt>,
}

/// One output scale's bitmaps: the fallback shapes rasterised at
/// [`physical_size`] pixels plus the theme image resolved at that size for
/// the current [`Cursor::status`].
///
/// The per-entry theme image keeps [`Cursor::themed`'s old invariant](Cursor)
/// in a new place: **every write to `status` refreshes every entry's image
/// in the same statement.** There are exactly two writes to `status` -- the
/// constructor in [`Cursor::new`] (immediately followed by a refresh) and
/// [`Cursor::set_status`] (which refreshes inline) -- plus
/// [`Cursor::install_rebuilt`], which swaps the whole bitmap set without
/// touching `status` and then refreshes for the live one. Adding another
/// writer of either without refreshing these would leave the previous
/// shape's pixels drawn under the new shape's name -- the single most
/// likely way this module could silently draw the wrong thing.
///
/// `themed: None` means "no theme image for this status at this size": a
/// hidden cursor, a client surface, or a named shape the theme does not
/// carry. All three fall through to the entry's own shapes.
struct ScaleEntry {
    /// [`scale_key`] of [`Self::scale`].
    key: u64,
    /// The quantized output scale this entry serves.
    scale: f64,
    /// The bitmap edge length: `round(logical * scale)`.
    physical: i32,
    /// One bitmap per [`Shape`], indexed by [`Shape::index`].
    shapes: [MemoryRenderBuffer; Shape::COUNT],
    /// Each bitmap's hotspot, indexed the same way: worked out once with the
    /// bitmaps (placing the outline art costs a parse), so the frame path
    /// reads it rather than recomputing it.
    hotspots: [Point<i32, Logical>; Shape::COUNT],
    /// The theme image for the current status at [`Self::physical`].
    themed: Option<Themed>,
}

/// How many output scales keep prebuilt bitmaps before the oldest is
/// evicted: 1.0 plus the fractional and 2x scales a mixed-DPI session uses,
/// with one spare. Bounded so a session cycling scales cannot grow memory
/// without limit; eviction only drops a cache (rebuilt on next
/// [`Cursor::sync_scales`]), never correctness -- the frame path falls back
/// to the nearest kept scale.
const MAX_SCALES: usize = 4;

/// The bitmap edge length for `logical`-px cursor art on an output at
/// `scale`: `round(logical * scale)` physical pixels (16 at 1x, 20 at 1.25,
/// 24 at 1.5, 32 at 2x), clamped to at least 1 and to [`MAX_PHYSICAL`].
/// Non-finite or non-positive scales fall back to the logical size rather
/// than panicking or allocating absurdly: scale math follows config and
/// client input, both of which have produced NaN before.
pub(crate) fn physical_size(logical: i32, scale: f64) -> i32 {
    if !scale.is_finite() || scale <= 0.0 {
        return logical;
    }
    ((f64::from(logical) * scale).round() as i32).clamp(1, MAX_PHYSICAL)
}

/// The largest physical bitmap edge this builds: far above any real cursor
/// (`Appearance::MAX_CURSOR_SIZE` at 3x is 768), it only keeps a corrupt
/// scale from becoming a huge allocation. Matches the overlay-plane twin's
/// own bound (`render::cursor_plane`).
const MAX_PHYSICAL: i32 = 1024;

/// The cache key for an output scale: the scale in hundredths, so 1.0, 1.25,
/// 1.5, 2.0 and 3.0 are distinct entries while float noise around one
/// configured value is not. Non-finite or non-positive scales key as 1.0
/// (see [`physical_size`]).
pub(crate) fn scale_key(scale: f64) -> u64 {
    let sane = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (sane * 100.0).round().max(1.0) as u64
}

impl Cursor {
    /// Builds the fallback bitmap from the resolved `[appearance]` values and
    /// keeps it for the process's lifetime, or until [`Cursor::rebuild`]
    /// replaces it on a config reload.
    ///
    /// Called once, from `State::new`, with `appearance.cursor_size` and
    /// `appearance.cursor_color`.
    /// [`Cursor::rebuild`] is the only other path that builds these buffers;
    /// nothing else may, which is what keeps the stable-`Id` reasoning on
    /// [`Self::shapes`] honest (see that field).
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
    /// The rim auto-contrasts the body fill (see [`contrast_rim`]), at the
    /// fill's own alpha, rather than being a second config field: its whole
    /// job is to keep the shape's edges legible against content of a similar
    /// color, which a configurable rim could only undo. Matching the fill's
    /// alpha is what makes a translucent `cursor_color` actually look
    /// translucent instead of an opaque rim around a see-through middle.
    pub fn new(size: i32, color: Color, theme_name: Option<&str>) -> Self {
        let size = Appearance::clamp_cursor_size(size);
        let (fill, rim) = paints(color);
        let theme = Theme::load(theme_name, size);
        let mut cursor = Self {
            scales: Vec::new(),
            size,
            fill,
            rim,
            status: CursorImageStatus::default_named(),
            theme,
        };
        // The scale-1 bitmaps, eagerly: every session has a scale to draw at
        // before any output exists to name one, and the per-frame path only
        // ever selects entries (see `entry_for_scale`).
        cursor.ensure_scale(1.0);
        // The default status is a *named* shape, so the themed image for it
        // has to be resolved here too -- the entry invariant is "every
        // write to `status` refreshes every entry", and the construction
        // above is one such write.
        cursor.refresh_themed();
        cursor
    }

    /// Rebuilds the fallback bitmaps and re-resolves the theme from fresh
    /// `[appearance]` values, synchronously.
    ///
    /// The `status` is untouched -- whatever shape the pointer shows keeps
    /// showing, redrawn from the new pixels -- but the theme is reloaded
    /// whole (dropping the old image cache, which belonged to the old theme
    /// at the old size) and the themed image is re-resolved for that same
    /// status. `size` is clamped exactly as in [`Cursor::new`].
    ///
    /// Replacing the buffers (rather than reusing their `Id`s) is what makes
    /// the new pixels actually appear: to the damage tracker these read as
    /// new content for one frame, which is true -- the pixels did change.
    /// `Theme::load` never fails (an unresolvable name is an empty theme,
    /// drawn as the fallback shapes), so this cannot fail either.
    ///
    /// Synchronous: only startup-adjacent and test callers run it directly.
    /// A live reload builds the same set on a worker through
    /// [`Cursor::rebuild_async`] and swaps it in with
    /// [`Cursor::install_rebuilt`] (see `reload.rs`), because the build
    /// exceeds a frame at common cursor sizes.
    pub fn rebuild(&mut self, size: i32, color: Color, theme_name: Option<&str>) {
        let rebuilt = Self::rebuild_async(RebuildInput {
            size,
            color,
            theme_name: theme_name.map(str::to_owned),
            icon: self.status_icon(),
            scales: self.scale_list(),
        });
        self.install_rebuilt(rebuilt);
    }

    /// The named shape showing now, if the cursor shows one: what a worker
    /// warms the fresh theme's cache with, so installing its bitmaps needs
    /// no image load on the loop. A client surface or a hidden cursor has
    /// no theme image either way.
    pub(crate) fn status_icon(&self) -> Option<CursorIcon> {
        match &self.status {
            CursorImageStatus::Named(icon) => Some(*icon),
            CursorImageStatus::Surface(_) | CursorImageStatus::Hidden => None,
        }
    }

    /// The output scales this cursor serves, in first-use order: what a
    /// worker rebuilds alongside scale 1.0, so installing its set drops no
    /// live scale.
    pub(crate) fn scale_list(&self) -> Vec<f64> {
        self.scales.iter().map(|entry| entry.scale).collect()
    }

    /// Builds the bitmap set a reload installs, off the loop thread (see
    /// `reload.rs`'s cursor worker): the fallback shapes rasterised for
    /// scale 1.0 plus every scale in `input.scales`, and the fresh theme
    /// with `input.icon` resolved at each size, so installing needs no file
    /// or image load on the loop. Pure over its input -- no compositor
    /// state, no globals -- and infallible like [`Cursor::rebuild`].
    pub fn rebuild_async(input: RebuildInput) -> RebuiltCursor {
        let size = Appearance::clamp_cursor_size(input.size);
        let (fill, rim) = paints(input.color);
        let mut theme = Theme::load(input.theme_name.as_deref(), size);
        // Scale 1.0 first, then the live scales in first-use order -- the
        // same sequence [`Cursor::rebuild`] feeds `ensure_scale`, including
        // its oldest-first eviction past [`MAX_SCALES`]: keeping the last
        // four is exactly what pushing each and dropping index 0 at four
        // leaves behind.
        let mut keys: Vec<(u64, f64)> = Vec::with_capacity(input.scales.len() + 1);
        for scale in std::iter::once(1.0).chain(input.scales) {
            let key = scale_key(scale);
            if !keys.iter().any(|(known, _)| *known == key) {
                keys.push((key, scale));
            }
        }
        while keys.len() > MAX_SCALES {
            keys.remove(0);
        }
        let mut scales = Vec::with_capacity(keys.len());
        for (_, scale) in keys {
            scales.push(Self::build_entry_for(
                size, fill, rim, &mut theme, input.icon, scale,
            ));
        }
        RebuiltCursor {
            size,
            fill,
            rim,
            theme,
            scales,
        }
    }

    /// Installs a worker-built bitmap set in one atomic swap: the size, the
    /// paints, the theme and every scale's buffers together, never a
    /// half-rebuilt cursor. Re-resolves the theme image for the *live*
    /// status afterwards -- a cache hit when the pointer still shows the
    /// shape the worker warmed, a load when it moved on meanwhile -- which
    /// keeps [`ScaleEntry`]'s invariant across this fourth write site.
    pub fn install_rebuilt(&mut self, rebuilt: RebuiltCursor) {
        self.size = rebuilt.size;
        self.fill = rebuilt.fill;
        self.rim = rebuilt.rim;
        self.theme = rebuilt.theme;
        self.scales = rebuilt.scales;
        self.refresh_themed();
    }

    /// Builds the bitmaps for one output scale: the `Shape::COUNT` fallback
    /// bitmaps rasterised natively at [`physical_size`] pixels, in
    /// [`Shape::index`] order, plus the current status's theme image resolved
    /// at that size. Every shape is `shapes::generate` -- Mocu's art at
    /// physical pixels, so a 2x output draws the theme's 32px art, not its
    /// 16px art stretched.
    ///
    /// Cold paths only ([`Cursor::new`], [`Cursor::rebuild`],
    /// [`Cursor::ensure_scale`]); the frame path only reads entries. Shared
    /// by all three so they cannot disagree about what a size/scale pair
    /// draws.
    fn build_entry(&mut self, scale: f64) -> ScaleEntry {
        let icon = self.status_icon();
        Self::build_entry_for(self.size, self.fill, self.rim, &mut self.theme, icon, scale)
    }

    /// The worker half of [`build_entry`](Self::build_entry): the same
    /// bitmaps from explicit paints and theme instead of `&mut self`, so a
    /// worker thread builds without touching the live cursor. The loop-side
    /// entry point stays the one cold paths call; the worker calls this one.
    fn build_entry_for(
        size: i32,
        fill: [u8; 4],
        rim: [u8; 4],
        theme: &mut Theme,
        icon: Option<CursorIcon>,
        scale: f64,
    ) -> ScaleEntry {
        let key = scale_key(scale);
        let sane = key as f64 / 100.0;
        let physical = physical_size(size, sane);
        let shapes = Self::build_shapes(physical, fill, rim);
        let mut entry = ScaleEntry {
            key,
            scale: sane,
            physical,
            shapes,
            hotspots: Shape::ALL.map(|shape| shape.hotspot(physical)),
            themed: None,
        };
        entry.themed = match icon {
            Some(icon) => theme.image_at(icon, entry.physical.max(1) as u32).cloned(),
            None => None,
        };
        entry
    }

    /// The `Shape::COUNT` fallback bitmaps for one pixel size, in
    /// [`Shape::index`] order.
    fn build_shapes(size: i32, fill: [u8; 4], rim: [u8; 4]) -> [MemoryRenderBuffer; Shape::COUNT] {
        // `Shape::ALL` in its own order, indexed back by `Shape::index`
        // -- see that constant's doc for why the order lives there and
        // not here.
        Shape::ALL.map(|shape| {
            let pixels = shapes::generate(shape, size, fill, rim);
            MemoryRenderBuffer::from_slice(
                &pixels,
                Fourcc::Argb8888,
                (size, size),
                1,
                Transform::Normal,
                None,
            )
        })
    }

    /// Builds the bitmaps for `scale` unless they already exist, evicting
    /// the oldest entry past [`MAX_SCALES`]. Called from cold paths that
    /// know the output set changed -- output add, scale change
    /// (`sync_scales`), [`Cursor::new`], [`Cursor::rebuild`] -- never from
    /// the frame path, which only selects entries.
    pub(crate) fn ensure_scale(&mut self, scale: f64) {
        let key = scale_key(scale);
        if self.scales.iter().any(|entry| entry.key == key) {
            return;
        }
        if self.scales.len() >= MAX_SCALES {
            self.scales.remove(0);
        }
        let entry = self.build_entry(scale);
        self.scales.push(entry);
    }

    /// Builds bitmaps for every scale in `scales` and drops entries for
    /// scales no longer in it: what output add, scale change, removal and
    /// reload call once the live output set is known. Idempotent -- entries
    /// for scales already served are kept with their stable `Id`s, so
    /// calling it when nothing changed allocates nothing.
    pub(crate) fn sync_scales(&mut self, scales: impl IntoIterator<Item = f64>) {
        let mut keys: Vec<u64> = Vec::new();
        for scale in scales {
            let key = scale_key(scale);
            if !keys.contains(&key) {
                keys.push(key);
            }
            self.ensure_scale(scale);
        }
        if keys.is_empty() {
            return;
        }
        self.scales.retain(|entry| keys.contains(&entry.key));
    }

    /// The entry the frame path draws at `scale`: the exact prebuilt scale,
    /// else the nearest kept one. Read-only -- the defensive backstop for a
    /// scale no cold path ensured, never the plan: every output add, scale
    /// change, removal and reload syncs its scales right after the live set
    /// changes (see `headless::sync_cursor_scales`), so a miss means a caller
    /// forgot its sync and draws soft until it is fixed. Never an allocation
    /// and never a missing cursor.
    fn entry_for_scale(&self, scale: f64) -> &ScaleEntry {
        let key = scale_key(scale);
        if let Some(entry) = self.scales.iter().find(|entry| entry.key == key) {
            return entry;
        }
        let sane = key as f64 / 100.0;
        self.scales
            .iter()
            .min_by(|a, b| {
                (a.scale - sane)
                    .abs()
                    .partial_cmp(&(b.scale - sane).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("`Cursor::new` always builds scale 1.0")
    }

    /// The machine's own cursor theme, as resolved at startup -- empty
    /// (`Theme::is_loaded() == false`) when none was found, in which case
    /// every named shape draws from `shapes.rs` instead. Read only by tests
    /// and by `compositor::run`'s environment export.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Which shape shows now. Tests only: the render path reads the field.
    #[cfg(test)]
    pub(crate) fn status(&self) -> &CursorImageStatus {
        &self.status
    }

    /// The clamped logical edge length the drawn shapes are built for: what a
    /// cursor session is first told to allocate before any image exists.
    pub(crate) fn logical_size(&self) -> i32 {
        self.size
    }

    /// Whether the client asked for no cursor at all (`set_cursor(NULL)`).
    pub(crate) fn is_hidden(&self) -> bool {
        matches!(self.status, CursorImageStatus::Hidden)
    }

    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;
        self.refresh_themed();
    }

    /// Re-resolves every entry's theme image for whatever [`Self::status`]
    /// now is.
    ///
    /// The one place entries' images are written. Called from every site
    /// that writes `status`, and from [`Self::install_rebuilt`] (which
    /// swaps the bitmaps without touching `status`), which is what keeps
    /// the two from disagreeing -- see [`ScaleEntry`]'s doc. Loading
    /// happens here, on the event that changed the shape, and never on the
    /// render path; a shape already looked up (or already known missing)
    /// costs one hash lookup.
    fn refresh_themed(&mut self) {
        let status = &self.status;
        let theme = &mut self.theme;
        for entry in &mut self.scales {
            entry.themed = match status {
                CursorImageStatus::Named(icon) => {
                    theme.image_at(*icon, entry.physical.max(1) as u32).cloned()
                }
                // A client surface draws the client's own pixels, and a hidden
                // cursor draws nothing; neither has a theme image, and leaving
                // a stale one here is exactly the bug the invariant exists to
                // stop.
                CursorImageStatus::Surface(_) | CursorImageStatus::Hidden => None,
            };
        }
    }

    /// Applies this commit's `wl_surface.offset` to the cursor hotspot, if
    /// the committed surface is the active cursor image.
    ///
    /// `wayland.xml` (`wl_pointer.set_cursor`): "On wl_surface.offset
    /// requests to the pointer surface, hotspot_x and hotspot_y are
    /// decremented by the x and y parameters passed to the request. The
    /// offset must be applied by wl_surface.commit as usual."
    ///
    /// Smithay core stores the offset as `SurfaceAttributes::buffer_delta`
    /// (double-buffered, so it sits in `current` exactly when this commit
    /// applies) and writes the hotspot only from `set_cursor` -- nothing in
    /// core adjusts it. Smithay's own `anvil` does this same decrement in
    /// its shell commit hook (`anvil/src/shell/mod.rs`, "decrementing
    /// cursor hotspot"); this is that hook for scoot, called from
    /// `CompositorHandler::commit`, which also already requests the redraw
    /// the moved hotspot needs.
    ///
    /// Saturating, not wrapping or panicking: both operands are
    /// client-controlled `i32` (`set_cursor` takes the hotspot raw,
    /// `wl_surface.offset` takes the delta raw), so `i32::MIN - 1` is one
    /// malicious commit away -- and a plain `-=` panics a debug build,
    /// taking every client's unsaved state down with the compositor.
    pub fn note_surface_commit(&self, surface: &WlSurface) {
        let CursorImageStatus::Surface(active) = &self.status else {
            return;
        };
        if active != surface {
            return;
        }
        with_states(surface, |states| {
            let delta = states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_delta
                .take();
            let Some(delta) = delta else {
                return;
            };
            // `set_cursor` always inserts this before the status can become
            // `Surface`, so its absence is impossible in practice -- but a
            // missing hotspot reads as "no adjustment", never as a panic,
            // the same stance as `surface_hotspot` below.
            let Some(attributes) = states.data_map.get::<CursorImageSurfaceData>() else {
                return;
            };
            let Ok(mut attributes) = attributes.lock() else {
                return;
            };
            attributes.hotspot.x = attributes.hotspot.x.saturating_sub(delta.x);
            attributes.hotspot.y = attributes.hotspot.y.saturating_sub(delta.y);
        });
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

    /// Whether `surface` belongs to the active cursor image: it is the
    /// client's cursor surface, or a subsurface in its tree. What routes a
    /// commit to the cursor through `State::cursor_changed` rather than a
    /// scene render. A walk up the subsurface parents -- as deep as the
    /// tree, which `subsurface_depth.rs` bounds -- only while a client
    /// cursor surface is active; a lookup on the status otherwise.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        let Some(active) = self.live_surface() else {
            return false;
        };
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        &root == active
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
        // Smithay's own list for a client surface's tree, returned as is;
        // exactly one slot for a drawn shape, none for a hidden cursor --
        // one allocation per frame at most, exactly sized, as before
        // `element_into` existed.
        if let Some(surface) = self.live_surface() {
            return surface_elements(renderer, surface, pointer_location, scale);
        }
        let mut elements = match &self.status {
            CursorImageStatus::Hidden => Vec::new(),
            _ => Vec::with_capacity(1),
        };
        self.element_into(renderer, pointer_location, scale, &mut elements);
        elements
    }

    /// [`Cursor::element`], appended to `out` instead of returned: the
    /// capture path's pooled list (`render/capture_cursor.rs`). A drawn
    /// shape adds its one element without allocating once `out` has
    /// capacity; a client cursor surface's tree still comes back from
    /// Smithay's `render_elements_from_surface_tree` as a list of its own.
    pub fn element_into<R>(
        &self,
        renderer: &mut R,
        pointer_location: Point<f64, Logical>,
        scale: f64,
        out: &mut Vec<CursorElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        if let Some(surface) = self.live_surface() {
            out.extend(surface_elements(renderer, surface, pointer_location, scale));
            return;
        }
        let icon = match &self.status {
            CursorImageStatus::Hidden => return,
            CursorImageStatus::Named(icon) => *icon,
            // A client cursor surface that is no longer alive -- the live
            // one returned above. Drawn as the default shape rather than as
            // nothing, for the same reason `forget_surface` falls back to it
            // rather than to `Hidden`: the pointer still exists and is still
            // being moved, so the wrong shape beats no cursor at all.
            CursorImageStatus::Surface(_) => CursorIcon::Default,
        };
        // The machine's own theme first, this module's drawn shapes second.
        // Both were resolved for this output scale when the status last
        // changed (or when the scale was ensured), so this is a field read,
        // not a lookup -- see `refresh_themed`. It is deliberately not
        // consulted on the dead-surface path above: every entry's image is
        // `None` for a `Surface` status by construction, so that path always
        // draws the arrow, which is the defensive fallback `forget_surface`
        // normally replaces within the same dispatch.
        //
        // The entry serves this output's scale: its bitmaps are already at
        // `round(logical * scale)` physical pixels. The element is sized in
        // logical units, so the on-screen size is unchanged at every scale
        // and the renderer samples the art 1:1 instead of stretching a
        // logical bitmap.
        let entry = self.entry_for_scale(scale);
        let logical = Size::<i32, Logical>::from((self.size, self.size));
        let (buffer, hotspot, src) = match &entry.themed {
            Some(themed) => (
                &themed.buffer,
                themed.hotspot,
                Rectangle::<f64, Logical>::from_size(Size::from((
                    f64::from(themed.size.0),
                    f64::from(themed.size.1),
                ))),
            ),
            None => {
                let shape = Shape::for_icon(icon);
                let physical = entry.physical;
                (
                    &entry.shapes[shape.index()],
                    entry.hotspots[shape.index()],
                    Rectangle::<f64, Logical>::from_size(Size::from((
                        f64::from(physical),
                        f64::from(physical),
                    ))),
                )
            }
        };
        // The pointer in physical pixels minus the hotspot in the bitmap's
        // own (physical) pixels. At scale 1 the hotspot is the logical one,
        // so this is exactly `element_location`'s arithmetic there (pinned
        // by its tests); at other scales the hotspot is the physical bitmap's
        // own tip, which is what keeps the tip under the pointer when insets
        // scale sublinearly.
        let location = Point::<f64, Physical>::from((
            pointer_location.x * scale - f64::from(hotspot.x),
            pointer_location.y * scale - f64::from(hotspot.y),
        ));
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            buffer,
            None,
            Some(src),
            Some(logical),
            Kind::Cursor,
        ) {
            Ok(element) => out.push(CursorElement::Fallback(element)),
            Err(error) => {
                tracing::warn!(%error, ?icon, "could not build the cursor element");
            }
        }
    }
}

/// A client cursor surface's whole tree as render elements, at the hotspot
/// the client set: [`Cursor::element`]'s first case, shared with
/// [`Cursor::element_into`].
fn surface_elements<R>(
    renderer: &mut R,
    surface: &WlSurface,
    pointer_location: Point<f64, Logical>,
    scale: f64,
) -> Vec<CursorElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
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
    render_elements_from_surface_tree(renderer, surface, location, scale, 1.0, Kind::Cursor)
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
