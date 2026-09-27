//! Where an image goes on an output, for each [`Mode`]: the part of the
//! image that shows, the size it is scaled to, and where it lands. All in
//! the image's *displayed* orientation, in pixels; the caller maps the
//! crop to the stored image (`super::orientation`).
//!
//! Integer throughout. `fill`'s crop and `fit`'s scaled size are rounded
//! to the nearest pixel, so the aspect ratio is kept to within half a
//! pixel (the scaler takes no fractional crop; see dependencies-done.md
//! §3b), and both are at least one pixel, so an extreme aspect ratio (a
//! 10000×1 panorama on a 1920×1080 output) still has something to scale.

use super::Mode;

#[cfg(test)]
mod tests;

/// A rectangle, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// The whole of a `width` × `height` area.
    pub fn whole(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Whether it lies inside a `width` × `height` area and is not empty.
    pub fn is_inside(self, width: u32, height: u32) -> bool {
        self.width > 0
            && self.height > 0
            && u64::from(self.x) + u64::from(self.width) <= u64::from(width)
            && u64::from(self.y) + u64::from(self.height) <= u64::from(height)
    }
}

/// How the image is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The part of the displayed image that is shown.
    pub crop: Rect,
    /// The size `crop` is scaled to; equal to its size when not scaled.
    pub scaled: (u32, u32),
    /// Where the scaled crop's top-left corner lands on the target.
    pub at: (u32, u32),
    /// The placed image is repeated across the whole target from there.
    pub tile: bool,
}

impl Layout {
    /// Whether the placed image leaves some of the target uncovered, to be
    /// painted in the fill color.
    pub fn letterboxed(&self, target: (u32, u32)) -> bool {
        !self.tile && self.scaled != target
    }

    /// Whether the crop has to be scaled.
    pub fn scales(&self) -> bool {
        self.scaled != (self.crop.width, self.crop.height)
    }
}

/// The layout of an image displayed at `image` on a target of `target`,
/// or `None` if either is empty.
pub fn layout(mode: Mode, image: (u32, u32), target: (u32, u32)) -> Option<Layout> {
    let (iw, ih) = image;
    let (tw, th) = target;
    if iw == 0 || ih == 0 || tw == 0 || th == 0 {
        return None;
    }
    // Whether the image is wider than the target, in aspect: iw/ih > tw/th.
    let wider = u64::from(iw) * u64::from(th) > u64::from(ih) * u64::from(tw);
    let layout = match mode {
        Mode::Fill => {
            // Cover: the crop has the target's aspect, as large as the
            // image allows, centred.
            let crop = if wider {
                let width = scaled(ih, tw, th).clamp(1, iw);
                Rect {
                    x: (iw - width) / 2,
                    y: 0,
                    width,
                    height: ih,
                }
            } else {
                let height = scaled(iw, th, tw).clamp(1, ih);
                Rect {
                    x: 0,
                    y: (ih - height) / 2,
                    width: iw,
                    height,
                }
            };
            Layout {
                crop,
                scaled: target,
                at: (0, 0),
                tile: false,
            }
        }
        Mode::Fit => {
            let size = if wider {
                (tw, scaled(ih, tw, iw).clamp(1, th))
            } else {
                (scaled(iw, th, ih).clamp(1, tw), th)
            };
            Layout {
                crop: Rect::whole(iw, ih),
                scaled: size,
                at: ((tw - size.0) / 2, (th - size.1) / 2),
                tile: false,
            }
        }
        Mode::Stretch => Layout {
            crop: Rect::whole(iw, ih),
            scaled: target,
            at: (0, 0),
            tile: false,
        },
        Mode::Center => {
            let (width, height) = (iw.min(tw), ih.min(th));
            Layout {
                crop: Rect {
                    x: (iw - width) / 2,
                    y: (ih - height) / 2,
                    width,
                    height,
                },
                scaled: (width, height),
                at: ((tw - width) / 2, (th - height) / 2),
                tile: false,
            }
        }
        Mode::Tile => {
            // Only the first repeat is read from the image; the rest are
            // copies of it (`super::pack::repeat`).
            let (width, height) = (iw.min(tw), ih.min(th));
            Layout {
                crop: Rect::whole(width, height),
                scaled: (width, height),
                at: (0, 0),
                tile: true,
            }
        }
    };
    Some(layout)
}

/// `value * numerator / denominator`, rounded to nearest, saturating at
/// `u32::MAX`. `denominator` is not zero (the caller checked).
fn scaled(value: u32, numerator: u32, denominator: u32) -> u32 {
    let denominator = u64::from(denominator.max(1));
    let exact = u64::from(value) * u64::from(numerator) + denominator / 2;
    u32::try_from(exact / denominator).unwrap_or(u32::MAX)
}
