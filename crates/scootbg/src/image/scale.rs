//! The one call into the scaler, `pic-scale-safe` (`forbid(unsafe_code)`,
//! chosen by measurement in dependencies-done.md §3b), so a swap stays
//! contained here.
//!
//! RGB to RGB, 3 bytes a pixel. Every size is validated before the call:
//! the scaler is safe code, so a size it cannot handle is a panic rather
//! than memory corruption, but a panic still aborts the daemon (the release
//! profile is `panic = "abort"`), so none may reach it.

use std::fmt;

use pic_scale_safe::{ImageSize, ResamplingFunction};

use super::Filter;

#[cfg(test)]
mod tests;

/// Why an image could not be scaled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleError {
    /// A width or height of zero, on either side.
    Empty,
    /// A size whose byte count overflows, or a source slice that is not
    /// exactly `width × height × 3` bytes.
    Size,
    /// The scaler refused (its own checks, which ours should pre-empt).
    Scaler(String),
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "cannot scale to or from an empty image"),
            Self::Size => write!(f, "image size out of range for scaling"),
            Self::Scaler(why) => write!(f, "scaling failed: {why}"),
        }
    }
}

/// The byte length of a `width` × `height` RGB image, if it fits both
/// `usize` and `isize` (the most any allocation can hold).
pub fn rgb_len(width: u32, height: u32) -> Option<usize> {
    let len = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(3)?;
    isize::try_from(len).ok()?;
    Some(len)
}

/// Scales `source`, a `from` = (width, height) RGB image, to `to`, with
/// `filter`. Returns a new `to.0 × to.1 × 3`-byte image. A same-size call
/// is refused as [`ScaleError::Size`]: the caller packs the source as it
/// is instead of paying for a copy.
pub fn scale(
    source: &[u8],
    from: (u32, u32),
    to: (u32, u32),
    filter: Filter,
) -> Result<Vec<u8>, ScaleError> {
    if from.0 == 0 || from.1 == 0 || to.0 == 0 || to.1 == 0 {
        return Err(ScaleError::Empty);
    }
    if from == to {
        return Err(ScaleError::Size);
    }
    let source_len = rgb_len(from.0, from.1).ok_or(ScaleError::Size)?;
    let target_len = rgb_len(to.0, to.1).ok_or(ScaleError::Size)?;
    if source.len() != source_len {
        return Err(ScaleError::Size);
    }
    // Both lengths fit `usize`, so each dimension does too.
    let size = |(width, height): (u32, u32)| ImageSize::new(width as usize, height as usize);
    let scaled = pic_scale_safe::resize_rgb8(source, size(from), size(to), function(filter))
        .map_err(ScaleError::Scaler)?;
    if scaled.len() != target_len {
        return Err(ScaleError::Size);
    }
    Ok(scaled)
}

fn function(filter: Filter) -> ResamplingFunction {
    match filter {
        Filter::Lanczos3 => ResamplingFunction::Lanczos3,
        Filter::CatmullRom => ResamplingFunction::CatmullRom,
        Filter::Bilinear => ResamplingFunction::Bilinear,
        Filter::Nearest => ResamplingFunction::Nearest,
    }
}
