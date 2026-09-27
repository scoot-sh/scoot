//! EXIF orientation: how an image's stored pixels map to what should be
//! displayed.
//!
//! The eight values, as the TIFF/EXIF specification defines them (what to
//! do to the stored image to display it):
//!
//! | Value | Transform | Displayed size |
//! |---|---|---|
//! | 1 | none | w × h |
//! | 2 | mirror left-right | w × h |
//! | 3 | rotate 180° | w × h |
//! | 4 | mirror top-bottom | w × h |
//! | 5 | transpose (mirror across the top-left diagonal) | h × w |
//! | 6 | rotate 90° clockwise | h × w |
//! | 7 | transverse (mirror across the top-right diagonal) | h × w |
//! | 8 | rotate 90° counter-clockwise | h × w |
//!
//! scootbg never rotates a buffer: it works out what to show in displayed
//! coordinates and reads the stored pixels through [`Orientation::walk`]
//! while packing (`super::pack`).

use super::fit::Rect;

#[cfg(test)]
mod tests;

/// An EXIF orientation, always 1 to 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orientation(u8);

impl Default for Orientation {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// How to step through the stored pixels in displayed order: the stored
/// pixel index (`y * width + x`) of displayed pixel (0, 0), and what one
/// displayed step right (`dx`) and down (`dy`) adds to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub start: usize,
    pub dx: isize,
    pub dy: isize,
}

impl Orientation {
    pub const NORMAL: Self = Self(1);

    /// The orientation tag's value; anything outside 1 to 8 is a broken
    /// file, shown as stored.
    pub fn from_exif(value: u16) -> Self {
        match u8::try_from(value) {
            Ok(value @ 1..=8) => Self(value),
            _ => Self::NORMAL,
        }
    }

    #[cfg(test)]
    pub fn value(self) -> u8 {
        self.0
    }

    /// Whether the displayed width is the stored height (5 to 8).
    pub fn swaps_axes(self) -> bool {
        self.0 >= 5
    }

    /// The displayed size of a stored `width` × `height` image.
    pub fn displayed(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The stored size of an image displayed at `width` × `height`.
    pub fn stored(self, width: u32, height: u32) -> (u32, u32) {
        // Swapping is its own inverse.
        self.displayed(width, height)
    }

    /// The stored coordinates of displayed pixel (`x`, `y`) of an image
    /// stored at `width` × `height`. For coordinates inside the displayed
    /// image; saturating, so anything else cannot panic (it maps to an
    /// edge instead).
    pub fn to_stored(self, x: u32, y: u32, width: u32, height: u32) -> (u32, u32) {
        let right = |v: u32| width.saturating_sub(1).saturating_sub(v);
        let bottom = |v: u32| height.saturating_sub(1).saturating_sub(v);
        match self.0 {
            2 => (right(x), y),
            3 => (right(x), bottom(y)),
            4 => (x, bottom(y)),
            5 => (y, x),
            6 => (y, bottom(x)),
            7 => (right(y), bottom(x)),
            8 => (right(y), x),
            _ => (x, y),
        }
    }

    /// The stored rectangle holding exactly the pixels of displayed
    /// rectangle `rect`, for an image stored at `width` × `height`. `rect`
    /// must lie inside the displayed image and not be empty.
    pub fn rect_to_stored(self, rect: Rect, width: u32, height: u32) -> Rect {
        let last_x = rect.x.saturating_add(rect.width.saturating_sub(1));
        let last_y = rect.y.saturating_add(rect.height.saturating_sub(1));
        let (ax, ay) = self.to_stored(rect.x, rect.y, width, height);
        let (bx, by) = self.to_stored(last_x, last_y, width, height);
        let (x, y) = (ax.min(bx), ay.min(by));
        Rect {
            x,
            y,
            width: (ax.max(bx) - x).saturating_add(1),
            height: (ay.max(by) - y).saturating_add(1),
        }
    }

    /// How to walk an image stored at `width` × `height` in displayed
    /// order (see [`Walk`]).
    pub fn walk(self, width: u32, height: u32) -> Walk {
        let (x, y) = self.to_stored(0, 0, width, height);
        // Widths are far below `isize::MAX` (the pixel budget); `max(1)`
        // only keeps a zero from making the steps meaningless.
        let row = isize::try_from(width.max(1)).unwrap_or(isize::MAX);
        let (dx, dy) = match self.0 {
            2 => (-1, row),
            3 => (-1, -row),
            4 => (1, -row),
            5 => (row, 1),
            6 => (-row, 1),
            7 => (-row, -1),
            8 => (row, -1),
            _ => (1, row),
        };
        let start = (y as usize)
            .saturating_mul(width as usize)
            .saturating_add(x as usize);
        Walk { start, dx, dy }
    }
}
