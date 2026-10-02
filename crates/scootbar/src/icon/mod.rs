//! Icons that are not a font's glyph: a **path icon** (SVG path data,
//! [`path`], filled by the bar's own anti-aliased rasterizer, [`raster`])
//! and, with the `icon-image` feature, an **image icon** (a PNG, decoded
//! once, [`image`]). Both are drawn square, `em` device pixels on a side,
//! before the module's text, and scaled to the output's real scale: the
//! bitmap is made at the device size (the scale is in the size), never a
//! smaller one stretched.
//!
//! A module holds an [`Icon`] from its settings and shows it through
//! [`crate::modules::View::show_icon`]; the bar measures, lays out and
//! draws it (`crate::render`). This is the mechanism the button, volume,
//! network and battery modules reuse: a module names an icon, never pixels,
//! and a path icon is tinted from the theme's token for the view's class,
//! so it follows Stylix like the text beside it.
//!
//! ## The cache
//!
//! A bitmap is made the first time an (icon, size) is drawn, and kept in
//! one arena with one small list of entries ([`Cache`]). Both are bounded
//! ([`MAX_ENTRIES`], [`MAX_ARENA`]): past either the whole cache is dropped
//! and refilled from what is drawn next, as the glyph cache does. Steady
//! state (every frame after the first at a size) allocates nothing: a
//! lookup walks at most [`MAX_ENTRIES`] entries and a draw reads the arena.
//! A new output scale is a new size, so it is a miss, once, and the old
//! size's bitmap ages out with the bound. An icon drawn at a size past
//! [`MAX_SIDE`] draws nothing (a 512-pixel icon in a bar is already
//! larger than any bar's text).

pub mod cache;
#[cfg(feature = "icon-image")]
pub mod image;
pub mod path;
pub mod raster;
pub mod sample;
pub mod tray;

use std::sync::Arc;

pub use cache::{Bitmap, Cache};
use path::Vector;

/// The largest icon drawn, in device pixels on a side.
pub const MAX_SIDE: u32 = 512;
/// Cached bitmaps at most.
pub const MAX_ENTRIES: usize = 16;
/// Bytes of cached bitmaps at most (a full-size image icon is 1 MiB).
pub const MAX_ARENA: usize = 4 * 1024 * 1024;

/// A fresh id for a parsed icon: the cache's key, which no two icons share
/// (an address could be reused once a reload drops an icon).
fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What a module shows before its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    /// One character of the font chain (a symbol font's glyph).
    Glyph(char),
    /// A path or image, drawn by the bar.
    Art(Art),
}

/// An icon the bar draws itself. Cloning shares it (a reference count, no
/// copy), so a view can hold one without allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Art {
    Vector(Arc<Vector>),
    #[cfg(feature = "icon-image")]
    Image(Arc<image::Image>),
    /// A tray item's pixmap: premultiplied pixels from the bus, drawn
    /// as they are (never tinted).
    Tray(Arc<tray::TrayIcon>),
}

impl Art {
    /// A key no other icon has, for the cache.
    pub fn id(&self) -> u64 {
        match self {
            Self::Vector(vector) => vector.id(),
            #[cfg(feature = "icon-image")]
            Self::Image(image) => image.id(),
            Self::Tray(icon) => icon.id(),
        }
    }
}
