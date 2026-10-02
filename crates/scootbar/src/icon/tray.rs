//! A tray icon: one item's pixmap, decoded from the bus into
//! premultiplied pixels, scaled to the size drawn.
//!
//! The item sends `ARGB32` in network order (the spec's icon-pixmap
//! page); what is held is premultiplied `b, g, r, a`, rows packed, like
//! an image icon. The icon cache ([`super::Cache`]) scales and keeps it
//! at the sizes drawn, keyed by [`TrayIcon::id`]: the id is stable while
//! the pixels are the same generation, so a steady tray hits the cache
//! every frame and a `NewIcon` misses once.

use super::MAX_SIDE;

/// One tray icon's pixels: premultiplied `b, g, r, a`, rows packed.
#[derive(Debug)]
pub struct TrayIcon {
    id: u64,
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

/// Equal when the same picture: the cache id is not part of it.
impl PartialEq for TrayIcon {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels == other.pixels
    }
}

impl Eq for TrayIcon {}

impl TrayIcon {
    /// Takes `argb` (`width × height` `ARGB32` in network order) as
    /// premultiplied pixels under `id`. `None` for a zero or past-bound
    /// size, or short bytes: a hostile pixmap never becomes an icon.
    pub fn take(id: u64, width: u32, height: u32, argb: &[u8]) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let count = width as usize * height as usize;
        let pixels = argb.get(..count * 4)?;
        let mut held = Vec::with_capacity(count * 4);
        for quad in pixels.chunks_exact(4) {
            let (a, r, g, b) = (quad[0], quad[1], quad[2], quad[3]);
            // Rounded `c × a ÷ 255`, at most `a`.
            let premultiply = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
            held.extend_from_slice(&[premultiply(b), premultiply(g), premultiply(r), a]);
        }
        Some(Self {
            id,
            width,
            height,
            pixels: held.into_boxed_slice(),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// The longer side, in pixels: what the tray picks entries by.
    pub fn side(&self) -> u32 {
        self.width.max(self.height)
    }

    /// The icon fitted into `out`, `side × side` premultiplied pixels
    /// (see [`super::sample`]). `out` shorter than that is left
    /// untouched.
    pub fn scale_into(&self, side: u32, out: &mut [u8]) {
        super::sample::scale_into(&self.pixels, self.width as usize, self.height as usize, side, out);
    }
}
