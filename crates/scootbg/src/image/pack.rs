//! Writing RGB pixels into an `XRGB8888` buffer: in memory blue, green,
//! red, then an unused byte (set to 0xff, as `Color::xrgb8888` does).
//!
//! [`Target::draw`] reads the source through its EXIF orientation's
//! [`Walk`](super::orientation::Walk), so a rotated or mirrored image is
//! put right in the same pass that converts its pixels, with no buffer of
//! its own. Rows that stay rows in the stored image (orientations 1 to 4)
//! are read as contiguous runs; the others walk the stored image down its
//! columns, in 32×32 blocks so each block's reads stay in cache.
//!
//! Every size and offset is validated before a pixel is touched, and the
//! loops read and write through `get`, so a mistake here is an error
//! return, never a panic.

use std::fmt;

use super::fit::Rect;
use super::orientation::Orientation;
use super::scale::rgb_len;
use crate::color::Color;

#[cfg(test)]
mod tests;

const BLOCK: u32 = 32;

/// A size or position out of range for the image or the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackError;

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "image placement out of range for the buffer")
    }
}

/// An RGB image as stored (before its orientation is applied).
#[derive(Debug, Clone, Copy)]
pub struct Stored<'a> {
    pub rgb: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
}

/// An `XRGB8888` buffer, `width * 4` bytes a row.
pub struct Target<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
}

fn xrgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 0xff]
}

impl<'a> Target<'a> {
    /// `None` unless `pixels` holds at least `width × height` pixels.
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32) -> Option<Self> {
        let len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (pixels.len() >= len).then_some(Self {
            pixels,
            width,
            height,
        })
    }

    fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// Paints `rect` (clipped to the buffer) in `color`.
    pub fn fill(&mut self, rect: Rect, color: Color) {
        let pixel = color.xrgb8888();
        let x_end = rect.x.saturating_add(rect.width).min(self.width);
        let y_end = rect.y.saturating_add(rect.height).min(self.height);
        if rect.x >= x_end || rect.y >= y_end {
            return;
        }
        let stride = self.stride();
        for y in rect.y..y_end {
            let row = y as usize * stride;
            let span = row + rect.x as usize * 4..row + x_end as usize * 4;
            if let Some(span) = self.pixels.get_mut(span) {
                for chunk in span.chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
        }
    }

    /// Paints everything outside `placed` in `color`: the bars around a
    /// letterboxed image.
    pub fn fill_around(&mut self, placed: Rect, color: Color) {
        let bottom = placed.y.saturating_add(placed.height);
        let right = placed.x.saturating_add(placed.width);
        self.fill(Rect::whole(self.width, placed.y), color);
        self.fill(
            Rect {
                x: 0,
                y: bottom,
                width: self.width,
                height: self.height.saturating_sub(bottom),
            },
            color,
        );
        self.fill(
            Rect {
                x: 0,
                y: placed.y,
                width: placed.x,
                height: placed.height,
            },
            color,
        );
        self.fill(
            Rect {
                x: right,
                y: placed.y,
                width: self.width.saturating_sub(right),
                height: placed.height,
            },
            color,
        );
    }

    /// Draws `region` of `image` (in its *displayed* orientation) with its
    /// top-left corner at `at`.
    pub fn draw(
        &mut self,
        image: Stored<'_>,
        region: Rect,
        at: (u32, u32),
    ) -> Result<(), PackError> {
        let len = rgb_len(image.width, image.height).ok_or(PackError)?;
        if image.rgb.len() < len {
            return Err(PackError);
        }
        let (dw, dh) = image.orientation.displayed(image.width, image.height);
        let placed = Rect {
            x: at.0,
            y: at.1,
            width: region.width,
            height: region.height,
        };
        if !region.is_inside(dw, dh) || !placed.is_inside(self.width, self.height) {
            return Err(PackError);
        }
        let walk = image.orientation.walk(image.width, image.height);
        let (start, dx, dy) = (walk.start as i64, walk.dx as i64, walk.dy as i64);
        // The stored index of displayed pixel (x, y); every one inside the
        // displayed image is inside the stored one.
        let index = |x: u32, y: u32| -> Result<usize, PackError> {
            let at = start + i64::from(x) * dx + i64::from(y) * dy;
            usize::try_from(at).map_err(|_| PackError)
        };
        let stride = self.stride();
        let run = region.width as usize;
        match dx {
            1 | -1 => {
                for row in 0..region.height {
                    let y = region.y + row;
                    // The run's lowest stored index: its first pixel going
                    // right, its last going left.
                    let first = if dx == 1 {
                        index(region.x, y)?
                    } else {
                        index(region.x + region.width - 1, y)?
                    };
                    let source = image
                        .rgb
                        .get(first * 3..(first + run) * 3)
                        .ok_or(PackError)?;
                    let offset = (at.1 + row) as usize * stride + at.0 as usize * 4;
                    let dest = self
                        .pixels
                        .get_mut(offset..offset + run * 4)
                        .ok_or(PackError)?;
                    let pixels = dest.chunks_exact_mut(4);
                    if dx == 1 {
                        for (to, from) in pixels.zip(source.chunks_exact(3)) {
                            to.copy_from_slice(&xrgb(from[0], from[1], from[2]));
                        }
                    } else {
                        for (to, from) in pixels.zip(source.chunks_exact(3).rev()) {
                            to.copy_from_slice(&xrgb(from[0], from[1], from[2]));
                        }
                    }
                }
            }
            _ => {
                for block_y in (0..region.height).step_by(BLOCK as usize) {
                    let rows = block_y..(block_y + BLOCK).min(region.height);
                    for block_x in (0..region.width).step_by(BLOCK as usize) {
                        let columns = block_x..(block_x + BLOCK).min(region.width);
                        for row in rows.clone() {
                            let mut from = index(region.x + block_x, region.y + row)? as i64;
                            let offset =
                                (at.1 + row) as usize * stride + (at.0 + block_x) as usize * 4;
                            let count = columns.len();
                            let dest = self
                                .pixels
                                .get_mut(offset..offset + count * 4)
                                .ok_or(PackError)?;
                            for to in dest.chunks_exact_mut(4) {
                                let at = usize::try_from(from).map_err(|_| PackError)? * 3;
                                let p = image.rgb.get(at..at + 3).ok_or(PackError)?;
                                to.copy_from_slice(&xrgb(p[0], p[1], p[2]));
                                from += dx;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Repeats the `tile` = (width, height) block at the top-left corner
    /// across the whole buffer, by doubling copies (each copy is of what
    /// is already done, so it is one `memmove` per doubling, not one per
    /// repeat).
    pub fn repeat(&mut self, tile: (u32, u32)) {
        let (tw, th) = (tile.0.min(self.width), tile.1.min(self.height));
        if tw == 0 || th == 0 {
            return;
        }
        let stride = self.stride();
        let width = self.width as usize * 4;
        for y in 0..th as usize {
            let row = y * stride;
            let mut done = tw as usize * 4;
            while done < width {
                let n = done.min(width - done);
                self.pixels.copy_within(row..row + n, row + done);
                done += n;
            }
        }
        let total = self.height as usize * stride;
        let mut done = th as usize * stride;
        while done < total {
            let n = done.min(total - done);
            self.pixels.copy_within(0..n, done);
            done += n;
        }
    }
}
