//! The bitmaps of icons at the sizes drawn (see the module docs of
//! [`super`]): one arena, one bounded list.

use super::raster::Rasterizer;
use super::{Art, MAX_ARENA, MAX_ENTRIES, MAX_SIDE};

#[cfg(test)]
mod tests;

/// An icon at one size, `side × side` pixels, rows packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bitmap<'a> {
    /// One coverage byte a pixel (a path icon): tinted when drawn.
    Mask(&'a [u8]),
    /// Four bytes a pixel, `b, g, r, a` with the color premultiplied by
    /// the alpha (an image icon): drawn as it is.
    Premultiplied(&'a [u8]),
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u64,
    side: u32,
    offset: usize,
    len: usize,
    mask: bool,
}

/// The cache, and the rasterizer's scratch it fills path icons with.
#[derive(Debug, Default)]
pub struct Cache {
    entries: Vec<Entry>,
    arena: Vec<u8>,
    raster: Rasterizer,
}

impl Cache {
    /// `art` at `side` pixels, made if it is not cached. `None` for a size
    /// of 0 or past [`MAX_SIDE`], which draws nothing.
    pub fn get(&mut self, art: &Art, side: u32) -> Option<Bitmap<'_>> {
        if side == 0 || side > MAX_SIDE {
            return None;
        }
        let id = art.id();
        let index = match self
            .entries
            .iter()
            .position(|e| e.id == id && e.side == side)
        {
            Some(index) => index,
            None => self.fill(art, id, side)?,
        };
        let entry = self.entries.get(index)?;
        let bytes = self.arena.get(entry.offset..entry.offset + entry.len)?;
        Some(if entry.mask {
            Bitmap::Mask(bytes)
        } else {
            Bitmap::Premultiplied(bytes)
        })
    }

    /// Makes `art` at `side`, dropping the whole cache first if it would
    /// pass a bound. Returns the new entry's index.
    fn fill(&mut self, art: &Art, id: u64, side: u32) -> Option<usize> {
        let pixels = side as usize * side as usize;
        let mask = matches!(art, Art::Vector(_));
        let len = if mask { pixels } else { pixels * 4 };
        if self.entries.len() >= MAX_ENTRIES || self.arena.len() + len > MAX_ARENA {
            self.entries.clear();
            self.arena.clear();
        }
        let offset = self.arena.len();
        self.arena.resize(offset + len, 0);
        let out = self.arena.get_mut(offset..offset + len)?;
        match art {
            Art::Vector(vector) => self.raster.fill(vector, side, out),
            #[cfg(feature = "icon-image")]
            Art::Image(image) => image.scale_into(side, out),
            Art::Tray(icon) => icon.scale_into(side, out),
        }
        self.entries.push(Entry {
            id,
            side,
            offset,
            len,
            mask,
        });
        Some(self.entries.len() - 1)
    }

    /// Cached bitmaps and their bytes, for tests.
    #[cfg(test)]
    pub fn cached(&self) -> (usize, usize) {
        (self.entries.len(), self.arena.len())
    }
}
