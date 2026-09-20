//! Window decorations: a niri-style focus ring plus a solid background.
//!
//! This project draws no titlebars -- see the design note in the roadmap
//! this module implements. Instead, the compositor draws a colored ring
//! *around* the focused window, outside its own rect, in the gap the layout
//! already leaves between windows, plus a solid background behind
//! everything. It also negotiates `zxdg_decoration_manager_v1` (see
//! `handlers.rs`'s `XdgDecorationHandler` impl) so well-behaved clients stop
//! drawing their own titlebar in the first place, rather than leaving a
//! server-drawn ring floating next to a client-drawn one.
//!
//! Explicitly out of scope, matching niri's own take on the same tradeoff:
//! real titlebar text or buttons, per-window/per-app-id overrides, drop
//! shadows, and any animation on focus change. A client told `ServerSide`
//! that expects the compositor to draw a close button or a drag area gets
//! neither -- just the ring. That's an accepted rough edge, not a bug to fix
//! here. (Rounded corners used to be on this list; `[appearance]
//! corner_radius` implements them -- see `rounded.rs`.)
//!
//! # Where the background lives (it isn't a render element)
//!
//! The natural-looking design -- a `SolidColorRenderElement` the size of the
//! output, drawn behind everything -- isn't what this module does. Every
//! `render_output`/`render_output` call already takes a `clear_color` that
//! `OutputDamageTracker` uses to clear whatever's damaged before drawing
//! elements on top; that *is* a background, for free, already guaranteed to
//! be the bottom-most thing on screen by construction (nothing draws before
//! the clear). Passing `Appearance::background_color` as that clear color
//! (see `render.rs::draw_frame_with`) is simpler than a full-output element, needs
//! no persistent buffer, and structurally cannot end up on the wrong side of
//! the window content the way a stray ordering bug in an element list could
//! -- see pitfall #3 in this feature's task notes. Only the ring is built as
//! real render elements here.
//!
//! # Persistent buffers, not fresh ones (the damage-tracking pitfall)
//!
//! [`Decorations`] keeps one [`SolidColorBuffer`] per ring segment (top,
//! bottom, left, right) per window, reused and `.update()`-d in place across
//! frames rather than rebuilt from scratch, so each keeps the same Smithay
//! element `Id` for as long as its window exists. That matters for
//! `OutputDamageTracker` in general: a fresh `Id` every frame reads as "new
//! content" and forces a full redraw of that element's area even when
//! nothing changed.
//!
//! It happens not to matter *today*, in this specific codebase: every call
//! to `render.rs::draw_frame_with` passes `age: 0` to `render_output`, which
//! (see `damage_output_internal` in Smithay's `backend::renderer::damage`)
//! unconditionally damages the *entire* output on every call, regardless of
//! any element's identity or commit counter -- the fine-grained per-element
//! damage this project's `OutputDamageTracker` computes is calculated and
//! then thrown away by that always-full-redraw fallback. This project's
//! actual idle-CPU win is entirely upstream of that: `State::request_render`
//! sets `needs_render`, and the frame timer (`headless::frame_tick`) simply
//! doesn't call `render()` at all -- doesn't bind, doesn't damage, doesn't
//! draw -- when nothing is dirty, rather than polling and finding no damage
//! every tick the way the pre-idle-fix version of this compositor did. So a
//! render call that *does* run was already a full-output redraw before this
//! module existed.
//!
//! Persistent buffers are still the right thing to build: it's the correct
//! architecture regardless of `age`'s current value, it's what the rest of
//! this codebase already does for any other per-window Smithay state, and it
//! means nothing here needs to be revisited if `age` is ever wired up to do
//! real partial redraws as a separate, later optimization.

use std::collections::{HashMap, HashSet};

use scoot_core::{Arrangement, Rect, WindowId};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

use super::rounded::{
    RingPaint, clip_rect, element_canvas, paint_ring, physical_radius, ring_layout,
};

/// A straight (non-premultiplied) RGBA color in `0.0..=1.0`, the form a
/// `"#rrggbb"`/`"#rrggbbaa"` config string parses into. Kept distinct from
/// Smithay's [`Color32F`] -- which wants *premultiplied* alpha -- so the one
/// conversion that matters (see [`From<Color> for Color32F`]) happens at a
/// single, tested boundary instead of wherever a color happens to get used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Parses `"#rrggbb"` (opaque) or `"#rrggbbaa"`, case-insensitive hex.
    /// `None` for anything else -- the caller (`config.rs`) is responsible
    /// for logging and falling back to a default on `None`, the same
    /// "isolate the damage to one field" rule the rest of that module's
    /// parsing already follows.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        let channel = |i: usize| -> Option<f32> {
            let byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
            Some(f32::from(byte) / 255.0)
        };
        match hex.len() {
            6 => Some(Self::new(channel(0)?, channel(1)?, channel(2)?, 1.0)),
            8 => Some(Self::new(
                channel(0)?,
                channel(1)?,
                channel(2)?,
                channel(3)?,
            )),
            _ => None,
        }
    }

    /// This color as one `Fourcc::Argb8888` pixel, in the byte order that
    /// format has in memory on a little-endian machine: `[B, G, R, A]`, with
    /// R/G/B premultiplied by A.
    ///
    /// Both halves of that matter and neither is guesswork:
    ///
    /// - **Byte order.** `Argb8888` names the channels from the *most*
    ///   significant bit of a 32-bit little-endian word down, so the lowest
    ///   address holds B -- the same layout `cursor.rs::generate_bitmap`
    ///   writes, `render/pixman.rs` renders into and `tty/buffers.rs` scans out
    ///   (see their module docs on the same fact), which is why nothing
    ///   downstream converts.
    /// - **Premultiplied.** The pinned Smithay rev hands an `Argb8888` memory
    ///   buffer to pixman as `a8r8g8b8` and composites it with
    ///   `Operation::Over` (`backend/renderer/pixman/mod.rs`, lines 389 and
    ///   605) -- Porter-Duff source-over, which is defined over premultiplied
    ///   components. That is the same assumption Smithay states for the
    ///   solid-color path in `backend::renderer::color`, and the reason
    ///   [`From<Color> for Color32F`] exists right below; this is the
    ///   byte-per-channel equivalent of it for an image buffer.
    ///
    /// For `a == 1.0` -- every color default this project ships, including
    /// the cursor's -- premultiplying changes nothing; it only matters once a
    /// translucent color is configured. See the unit tests below for both.
    ///
    /// Assumes `r`/`g`/`b`/`a` are each in `0.0..=1.0`, which every value
    /// `Color::parse` can produce is -- a `"#rrggbbaa"` string can't encode
    /// anything outside that range. A `Color` built directly (in-process,
    /// bypassing `parse`) with a channel `> 1.0` and `a < 1.0` can produce a
    /// channel byte greater than the alpha byte, which is not a valid
    /// premultiplied pixel and would over-brighten under `Operation::Over`
    /// rather than saturate -- `Color`'s fields are `pub`, so this is
    /// reachable in Rust, just not from any config a user can write.
    pub fn to_argb8888(self) -> [u8; 4] {
        let channel = |v: f32| (v * self.a * 255.0).round().clamp(0.0, 255.0) as u8;
        [
            channel(self.b),
            channel(self.g),
            channel(self.r),
            (self.a * 255.0).round().clamp(0.0, 255.0) as u8,
        ]
    }
}

impl From<Color> for Color32F {
    /// Smithay's solid-color renderer elements are documented (and, per
    /// `backend::renderer::color`'s doc comment, actually implemented) to
    /// want premultiplied alpha: R/G/B already scaled by A. This project's
    /// config stores what a user types in a `"#rrggbbaa"` string -- straight
    /// alpha -- so this is the one place that conversion has to happen. For
    /// `a == 1.0` (opaque, the overwhelmingly common case for every default
    /// this project ships) the two representations are numerically
    /// identical; it only changes anything once a translucent color is
    /// configured. See the unit tests below for both cases.
    fn from(c: Color) -> Self {
        Color32F::new(c.r * c.a, c.g * c.a, c.b * c.a, c.a)
    }
}

/// The validated, ready-to-render form of `config::AppearanceConfig` --
/// same relationship [`scoot_core::Config`] has to that module's
/// `LayoutConfig`. Colors are already parsed and clamps already applied by
/// the time one of these exists; nothing downstream needs to re-check either.
///
/// It mirrors the `[appearance]` *table*, not this module's own scope, so two
/// of its fields (`cursor_size`/`cursor_color`) are consumed by `cursor.rs`
/// rather than by anything here -- the alternative, a second config struct
/// for one pair of fields, would split one TOML table across two types for no
/// benefit. Everything this module's [`Color`] knows about pixel formats is
/// shared by both consumers anyway (see [`Color::to_argb8888`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub focus_ring_width: i32,
    pub focus_ring_active_color: Color,
    pub focus_ring_inactive_color: Color,
    pub background_color: Color,
    /// Window corner radius in logical pixels (`[appearance] corner_radius`,
    /// default 0 = square). Rounds the window content (see `rounded.rs`) and
    /// the focus ring below together: a square ring around a rounded window
    /// is worse than none, so the ring follows the radius rather than
    /// staying rectangular.
    ///
    /// Always within `0..=` once [`Appearance::clamped`] has run (negatives
    /// become 0); the upper bound lives per window at render time
    /// ([`rounded::effective_radius`]: half the window's smaller dimension),
    /// because windows differ in size and no one config value fits all.
    pub corner_radius: i32,
    /// Both dimensions of the square fallback cursor bitmap, in pixels --
    /// always within [`Appearance::MIN_CURSOR_SIZE`]`..=`[`Appearance::MAX_CURSOR_SIZE`]
    /// once [`Appearance::clamped`] has run. Only the *fallback* shape
    /// `cursor.rs` draws procedurally; a client that supplies its own cursor
    /// surface sizes that itself.
    pub cursor_size: i32,
    /// The fallback cursor shape's fill color. Its 1px outline is always
    /// black (at this color's own alpha) -- see `cursor::generate_bitmap`.
    pub cursor_color: Color,
    /// Which installed xcursor theme to draw named cursor shapes from, or
    /// `None` to take `$XCURSOR_THEME` (and `"default"` if that is unset too)
    /// -- see `cursor/theme.rs` for the resolution order and for why reading
    /// the machine's own theme is not the license problem *shipping* one
    /// would be.
    ///
    /// Only names a theme; it never makes scoot carry one. A name that
    /// matches nothing installed is not an error: named shapes then come from
    /// `cursor/shapes.rs`, exactly as they do on a machine with no themes at
    /// all.
    pub cursor_theme: Option<String>,
    /// Whether to answer a client's `zxdg_toplevel_decoration_v1` request
    /// with `ServerSide` -- see `handlers.rs`'s `XdgDecorationHandler` impl.
    /// niri's own default is `true`; this project uses the same default for
    /// the same reason (this project draws no titlebar, so a client left to
    /// draw its own would double up with the compositor's ring).
    pub prefer_no_csd: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            focus_ring_width: 3,
            // A bright accent blue for the focused window's ring...
            focus_ring_active_color: Color::new(0.42, 0.65, 0.98, 1.0),
            // ...and a muted gray for every other window's, close in spirit
            // to niri's own default palette without copying its exact hex
            // values.
            focus_ring_inactive_color: Color::new(0.35, 0.35, 0.38, 1.0),
            background_color: Color::new(0.08, 0.08, 0.1, 1.0),
            // Square until configured: any non-zero radius opts the session
            // into the rounded window + ring path (see `rounded.rs`), so the
            // default session is byte-identical to before it existed.
            corner_radius: 0,
            // The shape `cursor.rs` has drawn since it existed: a 16x16
            // triangle with a white fill. Unchanged defaults, so an existing
            // config file (or none) looks exactly as it did before this was
            // configurable.
            cursor_size: 16,
            cursor_color: Color::new(1.0, 1.0, 1.0, 1.0),
            // Unset, i.e. follow `$XCURSOR_THEME` like every other client on
            // the machine does, rather than overriding the user's desktop
            // from a compositor default.
            cursor_theme: None,
            prefer_no_csd: true,
        }
    }
}

impl Appearance {
    /// The smallest [`cursor_size`](Self::cursor_size) a config may ask for.
    ///
    /// `cursor.rs`'s shape is a triangle whose 1px outline takes the left
    /// column and the diagonal, so the fill only appears where `0 < x < y`:
    /// at size 3 that is the single pixel `(1, 2)`, at size 2 and 1 there is
    /// no fill pixel at all. 4 is therefore the smallest size that draws the
    /// shape this module actually describes rather than a couple of stray
    /// dark pixels -- and a pointer that small is indistinguishable from a
    /// dead pixel on any real display, which on `--tty` (where scoot *is*
    /// the session) leaves a user with no visible pointer and no other window
    /// manager to fix it from.
    pub const MIN_CURSOR_SIZE: i32 = 4;

    /// The largest [`cursor_size`](Self::cursor_size) a config may ask for.
    ///
    /// Two reasons for this exact number, both arithmetic rather than taste,
    /// in the same spirit as [`scoot_core::Config::MAX_GAP`]:
    ///
    /// - **Nothing real can use more.** 256px is a quarter of a 1080p
    ///   display's height; a pointer that size covers 3.2% of such a screen
    ///   and hides whatever it is pointing at. A config asking past it is a
    ///   typo or a probe, not a preference.
    /// - **It keeps the one allocation this value drives small and far from
    ///   overflow.** The bitmap is `size * size * 4` bytes, built once at
    ///   startup and copied once more by `MemoryRenderBuffer::from_slice`:
    ///   256 KiB at this cap. Without a cap that product is not merely an
    ///   absurd allocation but **overflows `i32` outright** -- it already does
    ///   at `size = 23171` (`23171 * 23171 * 4` is past `i32::MAX`), long
    ///   before anything a config could plausibly spell -- which is a debug
    ///   panic and a wrapped, therefore wrong, length in release, both inside
    ///   `generate_bitmap` and inside Smithay's own `stride * size.h` length
    ///   assertion.
    pub const MAX_CURSOR_SIZE: i32 = 256;

    /// Brings a configured cursor size into the range the bitmap path is safe
    /// for -- see the two constants above. Pure and separately tested, like
    /// [`scoot_core::Config::clamp_gap`]; [`Self::clamped`] is what applies
    /// it (and warns) for a real config file, and `cursor::Cursor::new`
    /// applies it again at the allocation itself.
    pub fn clamp_cursor_size(size: i32) -> i32 {
        size.clamp(Self::MIN_CURSOR_SIZE, Self::MAX_CURSOR_SIZE)
    }

    /// The load-time clamp every config-derived [`Appearance`] goes through:
    ///
    /// - `focus_ring_width` to at most half the layout's gap. A ring wider
    ///   than half the gap could reach past the midpoint between two adjacent
    ///   windows and visually collide with the neighbor's own ring or window
    ///   content -- a real visual bug, not a preference, so this clamps
    ///   rather than trusting a config value. Takes `gap` rather than reading
    ///   `scoot_core::Config` directly to keep this module independent of
    ///   that crate's config type -- see the module doc's broader point about
    ///   this crate, not `scoot_core`, owning decorations.
    /// - `cursor_size` into [`Self::MIN_CURSOR_SIZE`]`..=`[`Self::MAX_CURSOR_SIZE`],
    ///   which is about the bitmap allocation it drives, not just how it
    ///   looks -- see those constants.
    ///
    /// Each warns if it had to, naming the one field it changed: a clamped
    /// value is a config the user wrote that is not the config they are
    /// getting, and on `--tty` the log is the only place that can be said.
    pub fn clamped(mut self, gap: i32) -> Self {
        let max = (gap.max(0)) / 2;
        if self.focus_ring_width > max {
            tracing::warn!(
                configured = self.focus_ring_width,
                max,
                "focus_ring_width is wider than half the layout gap; clamping"
            );
            self.focus_ring_width = max;
        }
        if self.corner_radius < 0 {
            tracing::warn!(
                configured = self.corner_radius,
                "corner_radius is negative; clamping to zero"
            );
            self.corner_radius = 0;
        }
        let cursor_size = Self::clamp_cursor_size(self.cursor_size);
        if cursor_size != self.cursor_size {
            tracing::warn!(
                configured = self.cursor_size,
                min = Self::MIN_CURSOR_SIZE,
                max = Self::MAX_CURSOR_SIZE,
                "cursor_size is out of range; clamping"
            );
            self.cursor_size = cursor_size;
        }
        self
    }
}

/// The (up to 4) rectangles [`ring_rects`] decomposes a focus ring into,
/// named by which side of the window they're on. `None` means that segment
/// is entirely empty (a zero-width ring) or was clipped away entirely (fully
/// off the bounds passed to `ring_rects`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingRects {
    pub top: Option<Rect>,
    pub bottom: Option<Rect>,
    pub left: Option<Rect>,
    pub right: Option<Rect>,
}

/// Computes the ring drawn *outside* `rect` (a window's placement), `width`
/// pixels wide, clipped to `bounds` (normally the output's own rect).
///
/// The frame is decomposed into 4 rectangles with the top and bottom strips
/// spanning the full outer width (so they cover the corners) and the left
/// and right strips spanning only `rect`'s own height (so the corners aren't
/// drawn twice) -- the standard decomposition for a solid-color "frame"
/// shape when the only drawing primitive available is a filled rectangle.
///
/// Pure and unit-tested (see below) without needing a live renderer or
/// display: everything here is plain `i32` arithmetic on
/// [`scoot_core::Rect`], not a Smithay type, so this module doesn't need
/// Smithay at all to know its geometry is right.
pub fn ring_rects(rect: Rect, width: i32, bounds: Rect) -> RingRects {
    if width <= 0 {
        return RingRects::default();
    }
    let top = Rect::new(rect.x - width, rect.y - width, rect.w + 2 * width, width);
    let bottom = Rect::new(rect.x - width, rect.y + rect.h, rect.w + 2 * width, width);
    let left = Rect::new(rect.x - width, rect.y, width, rect.h);
    let right = Rect::new(rect.x + rect.w, rect.y, width, rect.h);
    RingRects {
        top: clip(top, bounds),
        bottom: clip(bottom, bounds),
        left: clip(left, bounds),
        right: clip(right, bounds),
    }
}

/// The intersection of `rect` and `bounds`, or `None` if they don't overlap
/// at all (or only touch along an edge, leaving zero area).
fn clip(rect: Rect, bounds: Rect) -> Option<Rect> {
    let x0 = rect.x.max(bounds.x);
    let y0 = rect.y.max(bounds.y);
    let x1 = rect.right().min(bounds.right());
    let y1 = rect.bottom().min(bounds.bottom());
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// One window's persistent ring buffers -- see the module doc's section on
/// why these are kept and updated in place instead of rebuilt every frame.
#[derive(Debug, Default)]
struct WindowRing {
    top: SolidColorBuffer,
    bottom: SolidColorBuffer,
    left: SolidColorBuffer,
    right: SolidColorBuffer,
}

/// One window's ring element: either a solid bar (the square path, and the
/// fallback when a painted ring cannot be built) or one painted rounded
/// strip (top or bottom -- the rounded path paints two strips plus the two
/// solid side bars, so a frame holds the same four elements per window
/// either way).
///
/// The caller (`render/elements.rs`) maps each into the frame's element list;
/// nothing about the frame path changes, only which variant each window
/// contributes.
pub enum RingElement<R: Renderer> {
    /// One bar of the square ring (four per window), or a side bar / the
    /// whole fallback on the rounded path.
    Rect(SolidColorRenderElement),
    /// One painted rounded strip (top or bottom). Boxed: the element carries
    /// the imported texture and dwarfs the bar variant, and without the box
    /// every window's slot in the per-frame ring `Vec` pays the big
    /// variant's size.
    Painted(Box<MemoryRenderBufferRenderElement<R>>),
}

/// What decides whether a window's painted ring is still current. Every
/// input to the paint, so a stale buffer is impossible by construction: any
/// change repaints before the element is built.
///
/// Position is deliberately *not* an input: the paint is
/// position-independent (the same window paints the same strips wherever it
/// sits), so a move must not repaint -- only the strips' draw origins go
/// stale, and [`Decorations::push_painted`] refreshes those in place on
/// every cache hit (see `refresh_strip_origins`). Keying on position instead
/// would repaint every window on every scroll frame for identical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintedKey {
    /// Logical window size (the paint's inputs, before scaling).
    w: i32,
    h: i32,
    /// Logical ring thickness.
    thickness: i32,
    /// Configured radius, logical pixels.
    radius: i32,
    /// Ring color bytes (premultiplied `Argb8888`, little-endian).
    color: [u8; 4],
    /// Output scale bits (`f64` has no `Eq`; the bits do).
    scale_bits: u64,
}

/// One window's painted ring: the two strip buffers the elements borrow,
/// where to draw them, the paint scratch reused across repaints, and the key
/// the buffers were painted for -- or `top: None` when the last build failed
/// and this window is on the square fallback until its key changes (a failed
/// build retried every frame would warn every frame).
///
/// Two strips rather than one full-window image: the middle of a full-canvas
/// ring is transparent, and compositing it costs a full-window blend every
/// frame for pixels that change nothing -- most of what an early benchmark
/// measured as the rounding cost. The strips cover only rows that hold ring
/// pixels (top band plus upper arcs, bottom band plus lower arcs); the
/// straight side runs stay solid rects through the existing `rings` buffers.
struct PaintedRing {
    top: Option<MemoryRenderBuffer>,
    bottom: Option<MemoryRenderBuffer>,
    /// Where the strips draw, and at what logical size: computed at build
    /// alongside the buffers (they only change with the key).
    top_at: Option<StripGeometry>,
    bottom_at: Option<StripGeometry>,
    pixels: Vec<u8>,
    key: Option<PaintedKey>,
}

/// Where one painted strip draws: its origin in exact physical pixels, its
/// size in logical pixels (what the element is built at), and its canvas in
/// physical pixels (what the cached buffer holds -- the refresh path
/// compares this to decide whether the buffers still fit).
#[derive(Debug, Clone, Copy)]
struct StripGeometry {
    loc: Point<f64, Physical>,
    logical: Size<i32, Logical>,
    canvas: Size<i32, Physical>,
}

/// Per-window decoration bookkeeping, owned by [`State`](super::State) for
/// as long as the compositor runs. `render.rs` calls [`Decorations::elements`]
/// once per render to get this frame's ring, and otherwise doesn't know this
/// type exists -- see the module doc's opening paragraph on keeping
/// decoration logic out of the render loop itself.
#[derive(Default)]
pub struct Decorations {
    rings: HashMap<WindowId, WindowRing>,
    painted: HashMap<WindowId, PaintedRing>,
    /// Scratch space for `elements`'s "which windows are still around"
    /// check, cleared and refilled in place every call instead of a fresh
    /// `HashSet` collected from scratch each time -- this runs on every
    /// actual render, so avoiding a per-call allocation here matters the
    /// same way it does everywhere else in this project's render path.
    live: HashSet<WindowId>,
}

impl Decorations {
    /// Builds this frame's focus-ring render elements from the current
    /// arrangement, updating each window's persistent buffers in place.
    /// Invisible windows (scrolled off-screen or on an inactive workspace --
    /// see `scoot_core::Placement::visible`) get no ring. A window whose
    /// buffers shrink to nothing (a zero-width ring, or one fully clipped
    /// away at an output edge) still keeps its buffer entries -- just
    /// resized to empty and producing no element -- so its `Id`s stay
    /// stable in case it reappears next frame.
    ///
    /// Every window still present in `arrangement` keeps its buffers;
    /// windows no longer present (closed) have theirs dropped, so this
    /// map can't grow without bound over a long-running session.
    ///
    /// This is the square ring, unchanged: the rounded session reaches
    /// [`Decorations::elements_rounded`] instead, so this path stays
    /// byte-identical with no branch on the radius. Painted buffers from an
    /// earlier rounded stretch are dropped here, so toggling the radius back
    /// to square frees them instead of leaking them for the session.
    pub fn elements(
        &mut self,
        arrangement: &Arrangement,
        appearance: &Appearance,
        bounds: Rect,
        scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        self.retain(arrangement);
        self.painted.clear();
        let mut elements = Vec::new();
        for placement in &arrangement.placements {
            if !placement.visible {
                continue;
            }
            let color = ring_color(arrangement, placement.id, appearance);
            let rects = ring_rects(placement.rect, appearance.focus_ring_width, bounds);
            let ring = self.rings.entry(placement.id).or_default();
            push(&mut elements, &mut ring.top, rects.top, color, scale);
            push(&mut elements, &mut ring.bottom, rects.bottom, color, scale);
            push(&mut elements, &mut ring.left, rects.left, color, scale);
            push(&mut elements, &mut ring.right, rects.right, color, scale);
        }
        elements
    }

    /// The rounded session's ring: one painted rounded ring per window (see
    /// `rounded.rs`), falling back to the square bars for a window whose
    /// painted buffer cannot be built. `renderer` is only touched to import a
    /// freshly repainted buffer -- steady-state frames reuse the cached one
    /// and never touch it.
    ///
    /// Painted buffers are dropped when the session goes back to square
    /// (`corner_radius == 0` reaches [`Decorations::elements`] instead and
    /// never calls this), so toggling the radius does not leak them.
    pub fn elements_rounded<R>(
        &mut self,
        arrangement: &Arrangement,
        appearance: &Appearance,
        bounds: Rect,
        scale: f64,
        renderer: &mut R,
    ) -> Vec<RingElement<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        self.retain(arrangement);
        let mut elements = Vec::new();
        for placement in &arrangement.placements {
            if !placement.visible {
                continue;
            }
            let color = ring_color(arrangement, placement.id, appearance);
            self.push_painted(
                &mut elements,
                placement.id,
                placement.rect,
                appearance,
                color,
                bounds,
                scale,
                renderer,
            );
        }
        elements
    }

    /// Drops every buffer -- rect and painted -- for windows no longer in the
    /// arrangement. Shared by both ring paths.
    fn retain(&mut self, arrangement: &Arrangement) {
        self.live.clear();
        self.live
            .extend(arrangement.placements.iter().map(|p| p.id));
        let live = &self.live;
        self.rings.retain(|id, _| live.contains(id));
        self.painted.retain(|id, _| live.contains(id));
    }

    /// One window's painted rounded ring -- two strips plus the two solid
    /// side bars -- or the square fallback when the painted buffers cannot
    /// be built. Repaints only when the key changed (resize, radius, width,
    /// color or scale); steady-state frames reuse the cached buffers with no
    /// allocation and no renderer touch. A hit still re-derives the strips'
    /// origins from the live rect -- moves and scrolls change no key input
    /// -- repainting only when fractional-scale rounding changed a strip
    /// canvas out from under the cached buffers.
    #[allow(clippy::too_many_arguments)]
    fn push_painted<R>(
        &mut self,
        elements: &mut Vec<RingElement<R>>,
        id: WindowId,
        rect: Rect,
        appearance: &Appearance,
        color: Color,
        bounds: Rect,
        scale: f64,
        renderer: &mut R,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        let thickness = appearance.focus_ring_width;
        if thickness <= 0 {
            return;
        }
        let key = PaintedKey {
            w: rect.w,
            h: rect.h,
            thickness,
            radius: appearance.corner_radius,
            color: color.to_argb8888(),
            scale_bits: scale.to_bits(),
        };
        let entry = self.painted.entry(id).or_insert_with(|| PaintedRing {
            top: None,
            bottom: None,
            top_at: None,
            bottom_at: None,
            pixels: Vec::new(),
            key: None,
        });
        if entry.key != Some(key) {
            entry.key = Some(key);
            build_strips(rect, appearance, color, scale, entry);
        } else if entry.top_at.is_some() && !refresh_strip_origins(rect, appearance, scale, entry) {
            // A fractional-scale rounding boundary moved under a cached ring
            // and a strip canvas no longer matches its buffer: repaint this
            // frame. At integer scales the canvases are exact, so this never
            // fires and a move costs two origin stores, not a repaint. The
            // `top_at` guard keeps the poison contract below: a window whose
            // last build failed stays on the silent square fallback until
            // its key changes, rather than retrying (and warning) every
            // frame.
            build_strips(rect, appearance, color, scale, entry);
        }
        let (Some(top), Some(bottom), Some(top_at), Some(bottom_at)) =
            (&entry.top, &entry.bottom, &entry.top_at, &entry.bottom_at)
        else {
            // The last build failed (already warned there): square fallback
            // through the persistent rect buffers, silently until the key
            // changes and a rebuild is attempted.
            self.push_fallback(elements, id, rect, thickness, color, bounds, scale);
            return;
        };
        let top = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            top_at.loc,
            top,
            None,
            None,
            Some(top_at.logical),
            Kind::Unspecified,
        );
        let bottom = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            bottom_at.loc,
            bottom,
            None,
            None,
            Some(bottom_at.logical),
            Kind::Unspecified,
        );
        match (top, bottom) {
            (Ok(top), Ok(bottom)) => {
                elements.push(RingElement::Painted(Box::new(top)));
                elements.push(RingElement::Painted(Box::new(bottom)));
                let rects = ring_rects(rect, thickness, bounds);
                let ring = self.rings.entry(id).or_default();
                push_painted_rect(elements, &mut ring.left, rects.left, color, scale);
                push_painted_rect(elements, &mut ring.right, rects.right, color, scale);
            }
            (Err(error), _) | (_, Err(error)) => {
                // Import-time failure (the paint succeeded): poison the entry
                // so this window falls back silently until its key changes --
                // a renderer refusing one import will refuse the retry next
                // frame too, and warning every frame is log spam.
                tracing::warn!(
                    %error,
                    "could not import the painted focus ring; falling back to a square ring"
                );
                entry.top = None;
                entry.bottom = None;
                entry.top_at = None;
                entry.bottom_at = None;
                self.push_fallback(elements, id, rect, thickness, color, bounds, scale);
            }
        }
    }

    /// The square fallback for one window on the rounded path: the same four
    /// bars [`Decorations::elements`] builds, through the same persistent
    /// buffers, so a window that cannot paint still gets a ring.
    #[allow(clippy::too_many_arguments)]
    fn push_fallback<R>(
        &mut self,
        elements: &mut Vec<RingElement<R>>,
        id: WindowId,
        rect: Rect,
        thickness: i32,
        color: Color,
        bounds: Rect,
        scale: f64,
    ) where
        R: Renderer,
    {
        let rects = ring_rects(rect, thickness, bounds);
        let ring = self.rings.entry(id).or_default();
        push_painted_rect(elements, &mut ring.top, rects.top, color, scale);
        push_painted_rect(elements, &mut ring.bottom, rects.bottom, color, scale);
        push_painted_rect(elements, &mut ring.left, rects.left, color, scale);
        push_painted_rect(elements, &mut ring.right, rects.right, color, scale);
    }
}

/// Active color for the focused window, inactive for the rest: the one rule
/// both ring paths share.
fn ring_color(arrangement: &Arrangement, id: WindowId, appearance: &Appearance) -> Color {
    if arrangement.focused == Some(id) {
        appearance.focus_ring_active_color
    } else {
        appearance.focus_ring_inactive_color
    }
}

/// Everything about one window's painted ring that follows from geometry
/// alone (no color): the two strip placements plus the full-canvas paint
/// inputs [`build_strips`] fills them from.
///
/// Pure -- the same inputs give the same plan -- which is what makes the
/// per-frame refresh sound: recomputing the plan for the live rect and
/// finding the same strip canvases means the cached buffers still fit, and
/// only their origins went stale.
struct StripPlan {
    top: StripGeometry,
    bottom: StripGeometry,
    /// The full-canvas size (what the shared paint rects live in).
    canvas: Size<i32, Physical>,
    /// The window's rect and the ring's outer rect in full-canvas
    /// coordinates: both strips paint sub-rects of this one band, so the
    /// strips and the solid side bars agree on every boundary by
    /// construction. At scale 1.0 the inner rect is exactly `(thickness,
    /// thickness, w, h)`; at fractional scales both roundings come from the
    /// same conversions, so the band still hugs the clip.
    inner: Rectangle<i32, Physical>,
    outer: Rectangle<i32, Physical>,
    radius_inner: i32,
    radius_outer: i32,
}

/// Computes the strip plan for `rect`, or `None` when a canvas is degenerate
/// (a zero-area window paints nothing rather than allocating a zero-byte
/// buffer: the ring of an invisible window is invisible either way, and
/// `MemoryRenderBuffer::from_slice` on an empty slice would only assert
/// downstream).
fn plan_strips(rect: Rect, appearance: &Appearance, scale: f64) -> Option<StripPlan> {
    let thickness = appearance.focus_ring_width;
    let (loc, logical, canvas) = ring_layout(rect, thickness, scale);
    if canvas.w <= 0 || canvas.h <= 0 {
        return None;
    }
    let clip = clip_rect(rect, scale);
    let radius_inner = physical_radius(appearance.corner_radius, clip, scale);
    let thickness_phys = (f64::from(thickness) * scale).round() as i32;
    let radius_outer = radius_inner + thickness_phys.max(0);
    let origin: Point<i32, Physical> = loc.to_i32_round();
    let inner = Rectangle::new(
        (clip.loc.x - origin.x, clip.loc.y - origin.y).into(),
        clip.size,
    );
    let outer = Rectangle::new((0, 0).into(), (canvas.w, canvas.h).into());
    // Strip height in physical pixels: the band rows plus the arc rows. The
    // logical height rounds UP, so the element's canvas always covers the
    // target: a strip one row short would leave a 1px gap in the ring at a
    // fractional scale, while a strip one row long only repaints side-band
    // pixels the solid bars already cover (same opaque color -- idempotent).
    let strip_target = thickness_phys + radius_outer;
    let strip_logical_h = ((strip_target as f64 / scale).ceil() as i32).max(1);
    let strip_logical = Size::<i32, Logical>::from((logical.w, strip_logical_h));
    // Top strip: full-canvas rows `0..canvas_h`, painted in place.
    let top_canvas = element_canvas(loc, strip_logical, scale);
    if top_canvas.w <= 0 || top_canvas.h <= 0 {
        return None;
    }
    // Bottom strip: full-canvas rows `canvas.h - bottom_h..canvas.h`. Its
    // canvas height comes from the same logical height at its own origin, so
    // it can differ from the top's by a pixel at fractional scales -- each
    // strip paints exactly its own canvas, so both stay correct. Seeded from
    // the top height (exact at integer scales); a degenerate window whose
    // full height is shorter than two strips just overlaps them, painting
    // the same opaque color twice -- idempotent.
    //
    // Recomputed rather than mirrored: the element rounding is
    // origin-sensitive, and the bottom origin differs.
    let bottom_loc =
        Point::<f64, Physical>::from((loc.x, loc.y + f64::from(canvas.h - top_canvas.h)));
    let bottom_canvas = element_canvas(bottom_loc, strip_logical, scale);
    if bottom_canvas.w <= 0 || bottom_canvas.h <= 0 {
        return None;
    }
    Some(StripPlan {
        top: StripGeometry {
            loc,
            logical: strip_logical,
            canvas: top_canvas,
        },
        bottom: StripGeometry {
            loc: bottom_loc,
            logical: strip_logical,
            canvas: bottom_canvas,
        },
        canvas,
        inner,
        outer,
        radius_inner,
        radius_outer,
    })
}

/// Refreshes one window's cached strip origins for its current `rect`
/// without repainting: the paint depends only on the key (size, shape,
/// color, scale), but the placement moves with the layout -- scrolling,
/// `fix_view`, `move-column` -- so a cache hit must still re-derive where
/// the strips draw.
///
/// Returns `false` when the cached buffers no longer fit -- a degenerate
/// plan, a first build that never happened, or a fractional-scale rounding
/// boundary that moved a strip canvas across a pixel boundary -- so the
/// caller repaints instead. No allocation and no renderer touch on `true`:
/// two origin stores.
fn refresh_strip_origins(
    rect: Rect,
    appearance: &Appearance,
    scale: f64,
    entry: &mut PaintedRing,
) -> bool {
    let Some(plan) = plan_strips(rect, appearance, scale) else {
        return false;
    };
    let (Some(top_at), Some(bottom_at)) = (&mut entry.top_at, &mut entry.bottom_at) else {
        return false;
    };
    if top_at.canvas != plan.top.canvas || bottom_at.canvas != plan.bottom.canvas {
        return false;
    }
    // The key is unchanged, so size, thickness and scale are too -- the
    // logical sizes cannot have moved, only the origins.
    debug_assert_eq!(top_at.logical, plan.top.logical);
    debug_assert_eq!(bottom_at.logical, plan.bottom.logical);
    top_at.loc = plan.top.loc;
    bottom_at.loc = plan.bottom.loc;
    true
}

/// Repaints `entry`'s two strips for `rect`: computes the full-canvas ring
/// geometry once (the band both strips share -- see [`plan_strips`]), then
/// paints the top rows and the bottom rows into the shared scratch,
/// importing each into its buffer. On any failure warns once and leaves the
/// entry empty (the caller falls back to the square ring until the key
/// changes).
fn build_strips(
    rect: Rect,
    appearance: &Appearance,
    color: Color,
    scale: f64,
    entry: &mut PaintedRing,
) {
    entry.top = None;
    entry.bottom = None;
    entry.top_at = None;
    entry.bottom_at = None;
    let Some(plan) = plan_strips(rect, appearance, scale) else {
        tracing::warn!("cannot paint a focus ring with no pixels; falling back to a square ring");
        return;
    };
    let argb = color.to_argb8888();
    paint_strip(
        &mut entry.pixels,
        plan.top.canvas,
        RingPaint {
            canvas: plan.top.canvas,
            outer: plan.outer,
            radius_outer: plan.radius_outer,
            inner: plan.inner,
            radius_inner: plan.radius_inner,
        },
        argb,
    );
    entry.top = Some(MemoryRenderBuffer::from_slice(
        &entry.pixels,
        Fourcc::Argb8888,
        (plan.top.canvas.w, plan.top.canvas.h),
        1,
        Transform::Normal,
        None,
    ));
    // The bottom strip's origin in full-canvas rows, so the shared `outer` /
    // `inner` rects land on the right rows when painted into the small
    // canvas: shift both rects up by the strip's first full-canvas row.
    let first_row = plan.canvas.h - plan.bottom.canvas.h;
    let shift = |rect: Rectangle<i32, Physical>| {
        Rectangle::new((rect.loc.x, rect.loc.y - first_row).into(), rect.size)
    };
    paint_strip(
        &mut entry.pixels,
        plan.bottom.canvas,
        RingPaint {
            canvas: plan.bottom.canvas,
            outer: shift(plan.outer),
            radius_outer: plan.radius_outer,
            inner: shift(plan.inner),
            radius_inner: plan.radius_inner,
        },
        argb,
    );
    entry.bottom = Some(MemoryRenderBuffer::from_slice(
        &entry.pixels,
        Fourcc::Argb8888,
        (plan.bottom.canvas.w, plan.bottom.canvas.h),
        1,
        Transform::Normal,
        None,
    ));
    entry.top_at = Some(plan.top);
    entry.bottom_at = Some(plan.bottom);
}

/// Sizes `pixels` for a strip canvas, zeroes it, and paints the shared ring
/// band into it: the same `outer`/`inner` rects (in full-canvas coordinates)
/// either strip passes, since `paint_ring` only fills rows its rects reach.
fn paint_strip(
    pixels: &mut Vec<u8>,
    canvas: Size<i32, Physical>,
    paint: RingPaint,
    color: [u8; 4],
) {
    let len = canvas.w as usize * canvas.h as usize * 4;
    if pixels.len() != len {
        pixels.resize(len, 0);
    }
    pixels.fill(0);
    paint_ring(pixels, &paint, color);
}

/// Updates one persistent segment buffer to `rect` (or to empty, if this
/// segment isn't present this frame) and, only if it has real area, pushes a
/// render element for it built from that same buffer -- so the element's
/// `Id` is the buffer's stable one, not a fresh one each call.
///
/// `rect` is logical (`ring_rects` works in the core's coordinates, and the
/// configured ring width is a logical width); `scale` is the output scale the
/// element is ultimately drawn at. The persistent buffer therefore stays
/// sized in logical pixels and `SolidColorRenderElement::from_buffer` does the
/// logical→physical conversion, while the location is converted here by the
/// same `to_physical_precise_round` the rest of the render path uses, so the
/// size and the origin agree. At scale 1.0 both conversions are the identity
/// (this is the code path that shipped before fractional scaling), which is
/// what keeps a scale-1 session pixel-identical.
fn push(
    elements: &mut Vec<SolidColorRenderElement>,
    buffer: &mut SolidColorBuffer,
    rect: Option<Rect>,
    color: Color,
    scale: f64,
) {
    let Some(rect) = rect else {
        buffer.resize((0, 0));
        return;
    };
    buffer.update((rect.w, rect.h), color);
    let location = Point::<i32, Logical>::from((rect.x, rect.y)).to_physical_precise_round(scale);
    elements.push(SolidColorRenderElement::from_buffer(
        buffer,
        location,
        scale,
        1.0,
        Kind::Unspecified,
    ));
}

/// [`push`] for the rounded path's square fallback: same segment update,
/// wrapped as [`RingElement::Rect`] instead of a bare solid element.
fn push_painted_rect<R: Renderer>(
    elements: &mut Vec<RingElement<R>>,
    buffer: &mut SolidColorBuffer,
    rect: Option<Rect>,
    color: Color,
    scale: f64,
) {
    let Some(rect) = rect else {
        buffer.resize((0, 0));
        return;
    };
    buffer.update((rect.w, rect.h), color);
    let location = Point::<i32, Logical>::from((rect.x, rect.y)).to_physical_precise_round(scale);
    elements.push(RingElement::Rect(SolidColorRenderElement::from_buffer(
        buffer,
        location,
        scale,
        1.0,
        Kind::Unspecified,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use scoot_core::{OutputId, Placement};
    use smithay::backend::renderer::element::Element;

    // -- Color::parse -----------------------------------------------------

    #[test]
    fn parses_a_six_digit_hex_color_as_opaque() {
        assert_eq!(
            Color::parse("#ff8000"),
            Some(Color::new(1.0, 128.0 / 255.0, 0.0, 1.0))
        );
    }

    #[test]
    fn parses_an_eight_digit_hex_color_with_alpha() {
        assert_eq!(
            Color::parse("#ff800080"),
            Some(Color::new(1.0, 128.0 / 255.0, 0.0, 128.0 / 255.0))
        );
    }

    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(Color::parse("#FF8000"), Color::parse("#ff8000"));
    }

    #[test]
    fn rejects_a_missing_hash() {
        assert_eq!(Color::parse("ff8000"), None);
    }

    #[test]
    fn rejects_the_wrong_length() {
        assert_eq!(Color::parse("#fff"), None);
        assert_eq!(Color::parse("#ff80000"), None);
    }

    #[test]
    fn rejects_non_hex_digits() {
        assert_eq!(Color::parse("#gggggg"), None);
    }

    // -- premultiplication --------------------------------------------------

    #[test]
    fn opaque_color_is_unchanged_by_premultiplication() {
        let opaque = Color::new(0.2, 0.4, 0.6, 1.0);
        let premultiplied: Color32F = opaque.into();
        assert_eq!(premultiplied.components(), [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn translucent_color_has_rgb_scaled_by_alpha() {
        // Chosen as dyadic fractions (0.5, 0.25, 0.125) specifically so the
        // f32 multiplication is exact and `assert_eq!` can compare it
        // directly -- a color parsed from an arbitrary "#rrggbbaa" string
        // (e.g. dividing by 255) will not generally multiply this cleanly,
        // and a future test built that way should compare with a tolerance
        // instead of `assert_eq!`, not treat a failure here as a sign this
        // conversion itself is wrong.
        let translucent = Color::new(1.0, 0.5, 0.25, 0.5);
        let premultiplied: Color32F = translucent.into();
        assert_eq!(premultiplied.components(), [0.5, 0.25, 0.125, 0.5]);
    }

    #[test]
    fn fully_transparent_color_premultiplies_to_all_zero_rgb() {
        let invisible = Color::new(1.0, 1.0, 1.0, 0.0);
        let premultiplied: Color32F = invisible.into();
        assert_eq!(premultiplied.components(), [0.0, 0.0, 0.0, 0.0]);
    }

    // -- Color::to_argb8888 --------------------------------------------------

    /// The byte order, with a color whose channels are all different -- the
    /// only kind that can tell `[B, G, R, A]` from `[R, G, B, A]`. (White,
    /// black and any other gray are symmetric under that swap, which is why
    /// no existing test of the cursor bitmap could have caught a swap.)
    #[test]
    fn an_opaque_color_becomes_bgra_bytes_unscaled() {
        let orange = Color::parse("#ff8000").expect("a valid color");
        assert_eq!(orange.to_argb8888(), [0x00, 0x80, 0xff, 0xff]);
    }

    /// Premultiplication, and the round trip through the same `/255` scaling
    /// `Color::parse` applies: `#ff800080` is R=255, G=128, B=0 at A=128, so
    /// R premultiplies to 255 * (128/255) = 128 and G to 128 * (128/255) =
    /// 64.25, which rounds to 64.
    #[test]
    fn a_translucent_color_has_its_channels_premultiplied() {
        let orange = Color::parse("#ff800080").expect("a valid color");
        assert_eq!(orange.to_argb8888(), [0x00, 64, 0x80, 0x80]);
    }

    #[test]
    fn a_fully_transparent_color_becomes_four_zero_bytes() {
        assert_eq!(Color::new(1.0, 1.0, 1.0, 0.0).to_argb8888(), [0, 0, 0, 0]);
    }

    #[test]
    fn white_and_black_are_exactly_the_bytes_the_cursor_has_always_used() {
        assert_eq!(
            Color::new(1.0, 1.0, 1.0, 1.0).to_argb8888(),
            [255, 255, 255, 255]
        );
        assert_eq!(Color::new(0.0, 0.0, 0.0, 1.0).to_argb8888(), [0, 0, 0, 255]);
    }

    /// `Color`'s fields are plain public floats, so a value outside `0.0..=1.0`
    /// is constructible even though `Color::parse` cannot produce one. The
    /// cast must saturate rather than wrap or panic.
    #[test]
    fn out_of_range_components_saturate_instead_of_wrapping() {
        assert_eq!(
            Color::new(2.0, -1.0, f32::NAN, 1.0).to_argb8888(),
            [0, 0, 255, 255]
        );
    }

    // -- Appearance::clamp_cursor_size ---------------------------------------

    #[test]
    fn cursor_size_clamps_at_both_bounds() {
        // One under the minimum, exactly at it, the default, exactly at the
        // maximum, one over it, and the two extremes a config can spell.
        for (configured, expected) in [
            (Appearance::MIN_CURSOR_SIZE - 1, Appearance::MIN_CURSOR_SIZE),
            (Appearance::MIN_CURSOR_SIZE, Appearance::MIN_CURSOR_SIZE),
            (16, 16),
            (Appearance::MAX_CURSOR_SIZE, Appearance::MAX_CURSOR_SIZE),
            (Appearance::MAX_CURSOR_SIZE + 1, Appearance::MAX_CURSOR_SIZE),
            (i32::MAX, Appearance::MAX_CURSOR_SIZE),
            (i32::MIN, Appearance::MIN_CURSOR_SIZE),
            (0, Appearance::MIN_CURSOR_SIZE),
            (-1, Appearance::MIN_CURSOR_SIZE),
        ] {
            assert_eq!(
                Appearance::clamp_cursor_size(configured),
                expected,
                "cursor_size {configured} clamped wrongly"
            );
        }
    }

    /// The reason the bound exists at all: `size * size * 4` is the bitmap's
    /// length in bytes, and at `i32::MAX` that product overflows `i32` (a
    /// debug panic inside `cursor::generate_bitmap`, a wrapped length in
    /// release). Clamping first keeps it at 256 KiB.
    #[test]
    fn the_clamped_cursor_size_cannot_overflow_the_bitmap_length() {
        let size = Appearance::clamp_cursor_size(i32::MAX);
        assert_eq!(size * size * 4, 262_144);
    }

    // -- Appearance::clamped -------------------------------------------------

    #[test]
    fn an_out_of_range_cursor_size_is_clamped_by_clamped() {
        let appearance = Appearance {
            cursor_size: i32::MAX,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.cursor_size, Appearance::MAX_CURSOR_SIZE);

        let appearance = Appearance {
            cursor_size: 0,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.cursor_size, Appearance::MIN_CURSOR_SIZE);
    }

    #[test]
    fn an_in_range_cursor_size_survives_clamped_untouched() {
        let appearance = Appearance {
            cursor_size: 48,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.cursor_size, 48);
    }

    #[test]
    fn a_ring_width_within_half_the_gap_is_left_alone() {
        let appearance = Appearance {
            focus_ring_width: 3,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.focus_ring_width, 3);
    }

    #[test]
    fn a_ring_wider_than_half_the_gap_is_clamped() {
        let appearance = Appearance {
            focus_ring_width: 8,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.focus_ring_width, 5);
    }

    #[test]
    fn a_zero_gap_clamps_the_ring_to_zero() {
        let appearance = Appearance {
            focus_ring_width: 4,
            ..Appearance::default()
        }
        .clamped(0);
        assert_eq!(appearance.focus_ring_width, 0);
    }

    #[test]
    fn a_negative_gap_is_treated_as_zero() {
        let appearance = Appearance {
            focus_ring_width: 4,
            ..Appearance::default()
        }
        .clamped(-6);
        assert_eq!(appearance.focus_ring_width, 0);
    }

    #[test]
    fn a_negative_corner_radius_is_clamped_to_zero() {
        let appearance = Appearance {
            corner_radius: -8,
            ..Appearance::default()
        }
        .clamped(10);
        assert_eq!(appearance.corner_radius, 0);
    }

    #[test]
    fn a_zero_or_positive_corner_radius_survives_clamped() {
        for configured in [0, 1, 12, i32::MAX] {
            let appearance = Appearance {
                corner_radius: configured,
                ..Appearance::default()
            }
            .clamped(10);
            assert_eq!(
                appearance.corner_radius, configured,
                "corner_radius {configured} must not be clamped: the upper bound \
                 is per window at render time, not per config at load time"
            );
        }
    }

    #[test]
    fn the_default_corner_radius_is_square() {
        assert_eq!(Appearance::default().corner_radius, 0);
    }

    // -- ring_rects -----------------------------------------------------------

    const SCREEN: Rect = Rect::new(0, 0, 1200, 800);

    #[test]
    fn a_zero_width_ring_produces_no_rects() {
        let rects = ring_rects(Rect::new(100, 100, 200, 150), 0, SCREEN);
        assert_eq!(rects, RingRects::default());
    }

    #[test]
    fn a_negative_width_also_produces_no_rects() {
        let rects = ring_rects(Rect::new(100, 100, 200, 150), -1, SCREEN);
        assert_eq!(rects, RingRects::default());
    }

    #[test]
    fn a_window_with_plenty_of_gap_gets_a_full_frame() {
        let rects = ring_rects(Rect::new(100, 100, 200, 150), 4, SCREEN);
        assert_eq!(rects.top, Some(Rect::new(96, 96, 208, 4)));
        assert_eq!(rects.bottom, Some(Rect::new(96, 250, 208, 4)));
        assert_eq!(rects.left, Some(Rect::new(96, 100, 4, 150)));
        assert_eq!(rects.right, Some(Rect::new(300, 100, 4, 150)));
    }

    #[test]
    fn a_window_at_the_top_left_corner_clips_the_off_screen_sides() {
        // The top and left strips would extend to negative coordinates --
        // entirely off `SCREEN` -- so they disappear; bottom and right are
        // still real, with top/bottom clipped to start at x=0.
        let rects = ring_rects(Rect::new(0, 0, 100, 100), 4, SCREEN);
        assert_eq!(rects.top, None);
        assert_eq!(rects.left, None);
        assert_eq!(rects.bottom, Some(Rect::new(0, 100, 104, 4)));
        assert_eq!(rects.right, Some(Rect::new(100, 0, 4, 100)));
    }

    #[test]
    fn a_window_scrolled_partly_off_the_left_edge_clips_its_left_ring() {
        // A column mid-scroll can have a negative x -- the layout still
        // hands this rect to decorations as-is (see the module doc on where
        // the "given rect" comes from).
        let rects = ring_rects(Rect::new(-50, 100, 200, 150), 4, SCREEN);
        assert_eq!(
            rects.left, None,
            "the left strip is entirely at x < -50, off screen"
        );
        assert_eq!(rects.top, Some(Rect::new(0, 96, 154, 4)));
        assert_eq!(rects.right, Some(Rect::new(150, 100, 4, 150)));
    }

    // -- Decorations::elements -----------------------------------------------

    fn placement(id: u64, rect: Rect) -> Placement {
        Placement {
            id: WindowId(id),
            output: OutputId(1),
            rect,
            visible: true,
        }
    }

    fn arrangement(placements: Vec<Placement>, focused: u64) -> Arrangement {
        Arrangement {
            placements,
            focused: Some(WindowId(focused)),
            focused_output: Some(OutputId(1)),
        }
    }

    #[test]
    fn a_visible_focused_window_produces_four_active_colored_segments() {
        let appearance = Appearance::default();
        let arrangement = arrangement(vec![placement(1, Rect::new(100, 100, 200, 150))], 1);
        let mut decorations = Decorations::default();

        let elements = decorations.elements(&arrangement, &appearance, SCREEN, 1.0);

        assert_eq!(elements.len(), 4);
        let expected: Color32F = appearance.focus_ring_active_color.into();
        for element in &elements {
            assert_eq!(element.color(), expected);
        }
    }

    #[test]
    fn an_invisible_window_produces_no_segments() {
        let appearance = Appearance::default();
        let mut invisible = placement(1, Rect::new(100, 100, 200, 150));
        invisible.visible = false;
        let arrangement = arrangement(vec![invisible], 1);
        let mut decorations = Decorations::default();

        assert!(
            decorations
                .elements(&arrangement, &appearance, SCREEN, 1.0)
                .is_empty()
        );
    }

    #[test]
    fn closing_a_window_drops_its_persistent_buffers() {
        let appearance = Appearance::default();
        let mut decorations = Decorations::default();
        let with_window = arrangement(vec![placement(1, Rect::new(100, 100, 200, 150))], 1);
        decorations.elements(&with_window, &appearance, SCREEN, 1.0);
        assert_eq!(decorations.rings.len(), 1);

        let closed = Arrangement::default();
        decorations.elements(&closed, &appearance, SCREEN, 1.0);
        assert!(decorations.rings.is_empty());
    }

    /// Pitfall #2 from this feature's task notes: when focus moves from one
    /// window to another, *both* windows' rings must change color -- not
    /// just the newly-focused one's. This exercises the exact bug shape a
    /// half-finished persistent-buffer update would have: only touching the
    /// buffers for whichever window's ring changed to *active*, and
    /// forgetting the one that changed to *inactive*.
    #[test]
    fn moving_focus_recolors_both_the_old_and_new_focused_windows_ring() {
        let appearance = Appearance::default();
        let active: Color32F = appearance.focus_ring_active_color.into();
        let inactive: Color32F = appearance.focus_ring_inactive_color.into();
        let mut decorations = Decorations::default();

        let a = placement(1, Rect::new(100, 100, 200, 150));
        let b = placement(2, Rect::new(400, 100, 200, 150));

        let a_focused = arrangement(vec![a, b], 1);
        let elements = decorations.elements(&a_focused, &appearance, SCREEN, 1.0);
        assert_eq!(elements.len(), 8);
        for element in &elements {
            let expected = if element.geometry(1.0.into()).loc.x < 350 {
                active
            } else {
                inactive
            };
            assert_eq!(element.color(), expected);
        }

        let b_focused = arrangement(vec![a, b], 2);
        let elements = decorations.elements(&b_focused, &appearance, SCREEN, 1.0);
        assert_eq!(elements.len(), 8);
        for element in &elements {
            let expected = if element.geometry(1.0.into()).loc.x < 350 {
                inactive
            } else {
                active
            };
            assert_eq!(
                element.color(),
                expected,
                "window {}'s ring segment at {:?} did not update after focus moved away from it",
                if element.geometry(1.0.into()).loc.x < 350 {
                    "a"
                } else {
                    "b"
                },
                element.geometry(1.0.into())
            );
        }
    }
}
