//! How a color or an image gets onto an output: the pure decisions, with
//! no Wayland objects, so each is a unit test. `daemon::canvas` carries
//! them out.
//!
//! An image is always a full-size `wl_shm` buffer at the surface's real
//! device pixels (`crate::density`: the fractional scale under a viewport
//! where the compositor has both, else an integer buffer scale), as path 3
//! draws a color, whatever the path: the paths below are how *colors* are
//! drawn.
//!
//! ## Three paths, best first
//!
//! 1. **[`Path::SinglePixel`]**: `wp_single_pixel_buffer_manager_v1` makes
//!    a 1×1 buffer from four `u32` channels, and `wp_viewporter` scales it
//!    to the surface. No shared memory at all, and the compositor knows it
//!    is one color (scoot can then scan a fullscreen window out over a
//!    black one, `docs/tty.md`).
//! 2. **[`Path::ViewportShm`]**: no single-pixel buffers, but a viewporter:
//!    a 1×1 `wl_shm` buffer, scaled the same way. 4 bytes of pixels.
//! 3. **[`Path::FullShm`]**: neither: a full-size `wl_shm` buffer, filled
//!    once. With no viewporter that is the surface's size times an integer
//!    scale; a fractional one needs a viewport to size the buffer.
//!
//! The path is chosen once, from the globals bound at start-up.
//!
//! ## Buffers on the shm paths
//!
//! A buffer the compositor may be reading (attached, not yet released) is
//! never written, and neither are pixels another output's buffer shares
//! (`crate::share`). A change reuses a released, unshared buffer of the
//! right size in place, else takes a second one, at most [`SLOTS`] in all;
//! with both still held by the compositor the draw waits for a
//! `wl_buffer.release` ([`pick`]).

use crate::density::{Buffer, Scale};
use crate::outputs::Size;
use crate::wallpaper::Wallpaper;

#[cfg(test)]
mod tests;

/// How colors are drawn; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Path {
    SinglePixel,
    ViewportShm,
    FullShm,
}

impl Path {
    /// The best path the compositor's globals allow. `forced` (a debug
    /// build's test knob, see `daemon::wayland`) can only pick a *worse*
    /// path that is still possible: a single-pixel buffer needs both
    /// globals, a 1×1 shm buffer the viewporter.
    pub fn choose(viewporter: bool, single_pixel: bool, forced: Option<Path>) -> Self {
        let best = match (viewporter, single_pixel) {
            (true, true) => Self::SinglePixel,
            (true, false) => Self::ViewportShm,
            (false, _) => Self::FullShm,
        };
        match forced {
            Some(forced) if forced > best => forced,
            _ => best,
        }
    }

    /// The name the debug knob uses (`daemon::wayland`). Debug builds
    /// only, like the knob: a release build has no use for it.
    #[cfg(debug_assertions)]
    pub fn name(self) -> &'static str {
        match self {
            Self::SinglePixel => "single-pixel",
            Self::ViewportShm => "viewport-shm",
            Self::FullShm => "full-shm",
        }
    }

    #[cfg(debug_assertions)]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::SinglePixel, Self::ViewportShm, Self::FullShm]
            .into_iter()
            .find(|path| path.name() == name)
    }

    /// Whether the surface is sized by a `wp_viewport`.
    pub fn uses_viewport(self) -> bool {
        self != Self::FullShm
    }
}

/// The scale to draw `wanted` at on `path`, given the surface's
/// (`Output::scale`): an image always has pixels to make sharp, a color
/// only on the full-size path. A 1×1 color buffer stays at scale 1, which
/// a larger buffer scale would make a protocol error (a buffer's size must
/// be a multiple of it), and its viewport sets the size anyway; so a new
/// scale does not redraw it.
pub fn scale_for(path: Path, wanted: Option<&Wallpaper>, surface: Scale) -> Scale {
    match wanted {
        Some(Wallpaper::Image(_)) => surface,
        Some(Wallpaper::Color(_)) | None if path == Path::FullShm => surface,
        Some(Wallpaper::Color(_)) | None => Scale::Integer(1),
    }
}

/// What a surface was last committed with: a color or an image at a size
/// (logical pixels) and scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    pub content: Wallpaper,
    pub size: Size,
    pub scale: Scale,
}

impl Drawn {
    /// The buffer on `path`: 1×1 at scale 1 for a color with a viewport,
    /// else the surface size at its scale ([`Scale::buffer`], the one
    /// place a full-size buffer's size is worked out). `None` if that
    /// overflows `u32`; the buffer type then refuses anything past
    /// `i32::MAX` bytes.
    pub fn buffer(&self, path: Path) -> Option<Buffer> {
        if path.uses_viewport() && matches!(self.content, Wallpaper::Color(_)) {
            return Some(Buffer {
                dims: (1, 1),
                scale: 1,
            });
        }
        self.scale.buffer(self.size)
    }
}

/// What to do to make a surface show what it should.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// It already does, or cannot yet (not configured, no size known).
    Nothing,
    /// Attach (or keep) a buffer with this content at this size, and
    /// commit.
    Show(Drawn),
    /// It shows something and should show nothing: replace the surface
    /// with a fresh one (see `daemon::canvas` for why not a null attach).
    Clear,
}

/// The shm buffers one output may have at once: the one on screen and one
/// to draw the next color into while the compositor still holds it.
pub const SLOTS: usize = 2;

/// What the decision needs to know about a buffer in a slot.
pub trait Slot {
    /// Released (or never attached): the compositor is done with this
    /// buffer, so it may be dropped.
    fn is_free(&self) -> bool;
    /// Free, and no other buffer shares its pixels: they may be written in
    /// place.
    fn is_writable(&self) -> bool;
    /// Its size in pixels.
    fn dims(&self) -> (u32, u32);
}

/// Where to draw a buffer of `dims`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// A writable buffer of that size: write it in place, no allocation.
    Reuse(usize),
    /// A free buffer that cannot be reused (another size, or pixels another
    /// output shares): drop it and allocate in its place.
    Replace(usize),
    /// An empty slot: allocate.
    Fill(usize),
    /// Every buffer is still held by the compositor: wait for a release.
    Stall,
}

/// Chooses a slot for a buffer of `dims`, cheapest first: reuse, then
/// replace a useless free one (so it does not linger), then a new one.
pub fn pick<S: Slot>(slots: &[Option<S>; SLOTS], dims: (u32, u32)) -> Pick {
    let reusable = |i: &usize| {
        slots[*i]
            .as_ref()
            .is_some_and(|slot| slot.is_writable() && slot.dims() == dims)
    };
    if let Some(i) = (0..SLOTS).find(reusable) {
        return Pick::Reuse(i);
    }
    if let Some(i) = (0..SLOTS).find(|&i| slots[i].as_ref().is_some_and(Slot::is_free)) {
        return Pick::Replace(i);
    }
    match (0..SLOTS).find(|&i| slots[i].is_none()) {
        Some(i) => Pick::Fill(i),
        None => Pick::Stall,
    }
}
