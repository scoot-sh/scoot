//! One output's buffer from a decoded image: fit, crop, scale, pack.
//!
//! Peak memory is what the ticket measured the design for
//! (dependencies-done.md §6b): the source is cropped in place (rows are a
//! sub-slice; columns are compacted row by row with `copy_within`, the
//! write never passing the read) and shrunk, scaled to a new buffer the
//! size of the output, and, when this is its last use, **dropped before
//! the shm buffer is allocated**. Rotation costs nothing extra: the scaler
//! works in the stored orientation, and packing reads through the rotated
//! index.
//!
//! When nothing is scaled (`center`, `tile`, or an image exactly the
//! output's size) the source is packed straight into the buffer, with no
//! crop and no copy.

use std::fmt;

use scootbg_mem::shm::Geometry;
use scootbg_mem::{ShmBuffer, ShmError};

use super::decode::Decoded;
use super::fit::{self, Rect};
use super::pack::{PackError, Stored, Target};
use super::scale::{self, ScaleError, rgb_len};
use super::{Filter, Mode};
use crate::color::Color;

#[cfg(test)]
mod tests;

/// How an image is to look on an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub mode: Mode,
    /// Behind a letterboxed or centred image, and under transparency.
    pub fill: Color,
    pub filter: Filter,
}

/// A decoded image, for its last use (`Owned`: cropped in place and
/// dropped as early as possible) or for one of several (`Borrowed`: a
/// crop is a copy).
#[derive(Debug)]
pub enum Source<'a> {
    Owned(Decoded),
    Borrowed(&'a Decoded),
}

impl Source<'_> {
    fn image(&self) -> &Decoded {
        match self {
            Self::Owned(image) => image,
            Self::Borrowed(image) => image,
        }
    }
}

/// Why a buffer could not be rendered.
#[derive(Debug)]
pub enum RenderError {
    /// A zero-sized buffer, or an image of zero size.
    Empty,
    Scale(ScaleError),
    Pack(PackError),
    Shm(ShmError),
    OutOfMemory(usize),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "nothing to draw: an empty image or output"),
            Self::Scale(error) => write!(f, "{error}"),
            Self::Pack(error) => write!(f, "{error}"),
            Self::Shm(error) => write!(f, "{error}"),
            Self::OutOfMemory(bytes) => write!(f, "out of memory: cannot allocate {bytes} bytes"),
        }
    }
}

/// Renders `source` as `look` says into a new `XRGB8888` buffer of `dims`
/// pixels.
pub fn render(source: Source<'_>, look: Look, dims: (u32, u32)) -> Result<ShmBuffer, RenderError> {
    // A size `wl_shm` cannot take is refused before anything that size is
    // scaled: the scaler would allocate it first (a compositor asking for
    // an absurd surface must not cost gigabytes to say no to).
    Geometry::xrgb8888(dims.0, dims.1).map_err(RenderError::Shm)?;
    let image = source.image();
    let (width, height, orientation) = (image.width, image.height, image.orientation);
    if rgb_len(width, height) != Some(image.rgb.len()) {
        return Err(RenderError::Empty);
    }
    let displayed = orientation.displayed(width, height);
    let layout = fit::layout(look.mode, displayed, dims).ok_or(RenderError::Empty)?;
    let placed = Rect {
        x: layout.at.0,
        y: layout.at.1,
        width: layout.scaled.0,
        height: layout.scaled.1,
    };

    if !layout.scales() {
        let mut buffer = ShmBuffer::new(dims.0, dims.1).map_err(RenderError::Shm)?;
        let mut target =
            Target::new(buffer.pixels_mut(), dims.0, dims.1).ok_or(RenderError::Empty)?;
        if layout.letterboxed(dims) {
            target.fill_around(placed, look.fill);
        }
        let stored = Stored {
            rgb: &image.rgb,
            width,
            height,
            orientation,
        };
        target
            .draw(stored, layout.crop, layout.at)
            .map_err(RenderError::Pack)?;
        if layout.tile {
            target.repeat(layout.scaled);
        }
        return Ok(buffer);
    }

    // Scaled in the stored orientation: the crop mapped back to the stored
    // image, the scaled size swapped for the 90° cases.
    let crop = orientation.rect_to_stored(layout.crop, width, height);
    let to = orientation.stored(layout.scaled.0, layout.scaled.1);
    let from = (crop.width, crop.height);
    let whole = crop == Rect::whole(width, height);
    let scaled = match source {
        Source::Owned(mut image) => {
            if !whole {
                crop_in_place(&mut image.rgb, width, height, crop)?;
            }
            scale::scale(&image.rgb, from, to, look.filter).map_err(RenderError::Scale)?
            // `image`, the source, is dropped here: before the buffer.
        }
        Source::Borrowed(image) if whole => {
            scale::scale(&image.rgb, from, to, look.filter).map_err(RenderError::Scale)?
        }
        Source::Borrowed(image) => {
            let cropped = crop_copy(&image.rgb, width, crop)?;
            scale::scale(&cropped, from, to, look.filter).map_err(RenderError::Scale)?
        }
    };
    let mut buffer = ShmBuffer::new(dims.0, dims.1).map_err(RenderError::Shm)?;
    let mut target = Target::new(buffer.pixels_mut(), dims.0, dims.1).ok_or(RenderError::Empty)?;
    if layout.letterboxed(dims) {
        target.fill_around(placed, look.fill);
    }
    let stored = Stored {
        rgb: &scaled,
        width: to.0,
        height: to.1,
        orientation,
    };
    target
        .draw(
            stored,
            Rect::whole(layout.scaled.0, layout.scaled.1),
            layout.at,
        )
        .map_err(RenderError::Pack)?;
    Ok(buffer)
}

/// Renders `image` once for each of `sizes` (distinct, in order), handing
/// each result to `each`. Every size but the last borrows the image (a
/// crop is a copy); the last takes it, so it is cropped in place and
/// dropped before that buffer is allocated. The daemon's worker draws a
/// job through this, and so does the fuzz target (`super::fuzz`), so what
/// is fuzzed is what the daemon runs.
pub fn render_each(
    image: Decoded,
    look: Look,
    sizes: &[(u32, u32)],
    mut each: impl FnMut((u32, u32), Result<ShmBuffer, RenderError>),
) {
    let Some((&last, rest)) = sizes.split_last() else {
        return;
    };
    for &dims in rest {
        each(dims, render(Source::Borrowed(&image), look, dims));
    }
    each(last, render(Source::Owned(image), look, last));
}

/// Renders every frame of `animated` at `dims`, in order: one `XRGB8888`
/// buffer per frame. Each frame goes through the same fit/crop/scale/pack
/// as a static image, so an animation shows exactly what its first frame
/// would as a still. A frame that cannot be scaled (the scaler's probe,
/// a side past its bound) fails the whole animation: a partial animation
/// that stops mid-loop is worse than a clear `draw_error` naming
/// `--no-animate`.
///
/// Unused until frame playback lands (the follow-up), which drives it:
/// stills render only the first frame through [`render`].
#[allow(dead_code)]
pub fn render_animated(
    animated: &super::animated::Animated,
    look: Look,
    dims: (u32, u32),
) -> Result<Vec<ShmBuffer>, RenderError> {
    let mut out = Vec::with_capacity(animated.frames.len());
    for frame in &animated.frames {
        // One clone per frame: the frame is borrowed by outputs that share
        // this size, so it cannot be moved through the in-place crop. The
        // clone is dropped before the next frame's, so the peak is one
        // frame, not the animation.
        let decoded = Decoded {
            rgb: frame.rgb.clone(),
            width: animated.width,
            height: animated.height,
            orientation: animated.orientation,
        };
        out.push(render(Source::Borrowed(&decoded), look, dims)?);
    }
    Ok(out)
}

/// Cuts `rgb`, a `width` × `height` image, down to `crop` in place, and
/// gives the rest back to the allocator.
pub fn crop_in_place(
    rgb: &mut Vec<u8>,
    width: u32,
    height: u32,
    crop: Rect,
) -> Result<(), RenderError> {
    let bad = || RenderError::Pack(PackError);
    if !crop.is_inside(width, height) || rgb_len(width, height) != Some(rgb.len()) {
        return Err(bad());
    }
    let (row, run) = (width as usize * 3, crop.width as usize * 3);
    for line in 0..crop.height as usize {
        let from = (crop.y as usize + line) * row + crop.x as usize * 3;
        let to = line * run;
        // Inside the image (checked above), and `to <= from`.
        if from + run > rgb.len() {
            return Err(bad());
        }
        rgb.copy_within(from..from + run, to);
    }
    rgb.truncate(run * crop.height as usize);
    // A large block: the allocator shrinks its mapping in place.
    rgb.shrink_to_fit();
    Ok(())
}

/// `crop` of `rgb` (rows `width` pixels long) as a new image, for a source
/// other outputs still need.
fn crop_copy(rgb: &[u8], width: u32, crop: Rect) -> Result<Vec<u8>, RenderError> {
    let len = rgb_len(crop.width, crop.height).ok_or(RenderError::Pack(PackError))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| RenderError::OutOfMemory(len))?;
    let (row, run) = (width as usize * 3, crop.width as usize * 3);
    for line in 0..crop.height as usize {
        let from = (crop.y as usize + line) * row + crop.x as usize * 3;
        let part = rgb
            .get(from..from + run)
            .ok_or(RenderError::Pack(PackError))?;
        out.extend_from_slice(part);
    }
    Ok(out)
}
