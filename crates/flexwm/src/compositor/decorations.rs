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
//! real titlebar text or buttons, per-window/per-app-id overrides, rounded
//! corners, drop shadows, and any animation on focus change. A client told
//! `ServerSide` that expects the compositor to draw a close button or a drag
//! area gets neither -- just the ring. That's an accepted rough edge, not a
//! bug to fix here.
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
//! (see `headless.rs::render`) is simpler than a full-output element, needs
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
//! to `headless.rs::render()` passes `age: 0` to `render_output`, which
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

use flexwm_core::{Arrangement, Rect, WindowId};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::utils::{Physical, Point};

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
    ///   writes, `headless.rs` renders into and `tty/buffers.rs` scans out
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
/// same relationship [`flexwm_core::Config`] has to that module's
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
    /// Both dimensions of the square fallback cursor bitmap, in pixels --
    /// always within [`Appearance::MIN_CURSOR_SIZE`]`..=`[`Appearance::MAX_CURSOR_SIZE`]
    /// once [`Appearance::clamped`] has run. Only the *fallback* shape
    /// `cursor.rs` draws procedurally; a client that supplies its own cursor
    /// surface sizes that itself.
    pub cursor_size: i32,
    /// The fallback cursor shape's fill color. Its 1px outline is always
    /// black (at this color's own alpha) -- see `cursor::generate_bitmap`.
    pub cursor_color: Color,
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
            // The shape `cursor.rs` has drawn since it existed: a 16x16
            // triangle with a white fill. Unchanged defaults, so an existing
            // config file (or none) looks exactly as it did before this was
            // configurable.
            cursor_size: 16,
            cursor_color: Color::new(1.0, 1.0, 1.0, 1.0),
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
    /// dead pixel on any real display, which on `--tty` (where flexwm *is*
    /// the session) leaves a user with no visible pointer and no other window
    /// manager to fix it from.
    pub const MIN_CURSOR_SIZE: i32 = 4;

    /// The largest [`cursor_size`](Self::cursor_size) a config may ask for.
    ///
    /// Two reasons for this exact number, both arithmetic rather than taste,
    /// in the same spirit as [`flexwm_core::Config::MAX_GAP`]:
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
    /// [`flexwm_core::Config::clamp_gap`]; [`Self::clamped`] is what applies
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
    ///   `flexwm_core::Config` directly to keep this module independent of
    ///   that crate's config type -- see the module doc's broader point about
    ///   this crate, not `flexwm_core`, owning decorations.
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
/// [`flexwm_core::Rect`], not a Smithay type, so this module doesn't need
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

/// Per-window decoration bookkeeping, owned by [`State`](super::State) for
/// as long as the compositor runs. `headless.rs` calls [`Decorations::elements`]
/// once per render to get this frame's ring, and otherwise doesn't know this
/// type exists -- see the module doc's opening paragraph on keeping
/// decoration logic out of the render loop itself.
#[derive(Debug, Default)]
pub struct Decorations {
    rings: HashMap<WindowId, WindowRing>,
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
    /// see `flexwm_core::Placement::visible`) get no ring. A window whose
    /// buffers shrink to nothing (a zero-width ring, or one fully clipped
    /// away at an output edge) still keeps its buffer entries -- just
    /// resized to empty and producing no element -- so its `Id`s stay
    /// stable in case it reappears next frame.
    ///
    /// Every window still present in `arrangement` keeps its buffers;
    /// windows no longer present (closed) have theirs dropped, so this
    /// map can't grow without bound over a long-running session.
    pub fn elements(
        &mut self,
        arrangement: &Arrangement,
        appearance: &Appearance,
        bounds: Rect,
    ) -> Vec<SolidColorRenderElement> {
        self.live.clear();
        self.live
            .extend(arrangement.placements.iter().map(|p| p.id));
        let live = &self.live;
        self.rings.retain(|id, _| live.contains(id));

        let mut elements = Vec::new();
        for placement in &arrangement.placements {
            if !placement.visible {
                continue;
            }
            let color = if arrangement.focused == Some(placement.id) {
                appearance.focus_ring_active_color
            } else {
                appearance.focus_ring_inactive_color
            };
            let rects = ring_rects(placement.rect, appearance.focus_ring_width, bounds);
            let ring = self.rings.entry(placement.id).or_default();
            push(&mut elements, &mut ring.top, rects.top, color);
            push(&mut elements, &mut ring.bottom, rects.bottom, color);
            push(&mut elements, &mut ring.left, rects.left, color);
            push(&mut elements, &mut ring.right, rects.right, color);
        }
        elements
    }
}

/// Updates one persistent segment buffer to `rect` (or to empty, if this
/// segment isn't present this frame) and, only if it has real area, pushes a
/// render element for it built from that same buffer -- so the element's
/// `Id` is the buffer's stable one, not a fresh one each call.
fn push(
    elements: &mut Vec<SolidColorRenderElement>,
    buffer: &mut SolidColorBuffer,
    rect: Option<Rect>,
    color: Color,
) {
    let Some(rect) = rect else {
        buffer.resize((0, 0));
        return;
    };
    buffer.update((rect.w, rect.h), color);
    let location = Point::<i32, Physical>::from((rect.x, rect.y));
    elements.push(SolidColorRenderElement::from_buffer(
        buffer,
        location,
        1.0,
        1.0,
        Kind::Unspecified,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use flexwm_core::{OutputId, Placement};
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

        let elements = decorations.elements(&arrangement, &appearance, SCREEN);

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
                .elements(&arrangement, &appearance, SCREEN)
                .is_empty()
        );
    }

    #[test]
    fn closing_a_window_drops_its_persistent_buffers() {
        let appearance = Appearance::default();
        let mut decorations = Decorations::default();
        let with_window = arrangement(vec![placement(1, Rect::new(100, 100, 200, 150))], 1);
        decorations.elements(&with_window, &appearance, SCREEN);
        assert_eq!(decorations.rings.len(), 1);

        let closed = Arrangement::default();
        decorations.elements(&closed, &appearance, SCREEN);
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
        let elements = decorations.elements(&a_focused, &appearance, SCREEN);
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
        let elements = decorations.elements(&b_focused, &appearance, SCREEN);
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
