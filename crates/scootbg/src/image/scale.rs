//! The one call into the scaler, `pic-scale-safe` (`forbid(unsafe_code)`,
//! chosen by measurement in dependencies-done.md §3b), so a swap stays
//! contained here.
//!
//! RGB to RGB, 3 bytes a pixel. Every size is validated before the call,
//! each side against [`MAX_SCALED_SIDE`] as well as for overflow:
//! the scaler is safe code, so a size it cannot handle is a panic rather
//! than memory corruption, but a panic still aborts the daemon (the release
//! profile is `panic = "abort"`), so none may reach it.

use std::fmt;

use pic_scale_safe::{ImageSize, ResamplingFunction};

use super::Filter;

#[cfg(test)]
mod tests;

/// The longest side the scaler is given, on the source or the target:
/// 65536 pixels, eight 8K screens side by side, which no wallpaper reaches
/// (a JPEG cannot: its sides stop at 65535).
///
/// Two limits of `pic-scale-safe` 0.1.12 set it, both per scaled axis:
///
/// - **Precision.** Its filter weights place each tap with `f32`
///   arithmetic (`compute_weights.rs`, `generate_weights`), which stops
///   holding every integer past 2^24: an axis longer than that rounds a
///   tap's end one past the kernel and panics indexing its table (a
///   20,000,000×1 grey PNG of 20 KB, `--mode fit` onto 1920×1080, aborted
///   the daemon; found in review of PR #317). The bound sits 256 times
///   below that.
/// - **Weight memory.** The weights are `kernel × out` `f32`s plus an `i16`
///   copy, where the kernel spans `2 × support × in / out` source pixels
///   when shrinking (`support` 3 for Lanczos3) and `2 × support` when
///   growing: about 36 bytes per pixel of the axis's longer side, committed
///   and infallible. At this bound that is 2.4 MB an axis, against 600 MB
///   for a 16.7-million-pixel row.
///
/// A longer side is refused ([`ScaleError::TooLong`]) rather than shrunk
/// first with `Nearest`: such an image is not a wallpaper anyone has, the
/// refusal is a clear `draw_error` naming the modes that do show it
/// (`fill`, which crops the long side away first, `center` and `tile`,
/// which scale nothing), and a pre-shrink would be a second full-size copy
/// and a second path through the scaler to keep correct for a case no one
/// meets. The pixel budget (`decode::MAX_PIXELS`) bounds the image; this
/// bounds each side the scaler sees.
pub const MAX_SCALED_SIDE: u32 = 1 << 16;

/// Why an image could not be scaled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleError {
    /// A width or height of zero, on either side.
    Empty,
    /// A side, of the part of the image to scale or of the size to scale
    /// it to, is longer than [`MAX_SCALED_SIDE`].
    TooLong { from: (u32, u32), to: (u32, u32) },
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
            Self::TooLong { from, to } => {
                // The hint holds only for an image too long to scale to a
                // target that is not: `fill` crops the long side first
                // (unless the output's aspect keeps the crop too long).
                // For a target that is too long, only modes that scale
                // nothing show it.
                let target_too_long = to.0 > MAX_SCALED_SIDE || to.1 > MAX_SCALED_SIDE;
                let hint = if target_too_long {
                    "--mode center or tile shows it"
                } else {
                    "--mode center or tile shows it, and fill usually does"
                };
                write!(
                    f,
                    "cannot scale {}x{} pixels to {}x{}: scootbg scales no side longer than \
                     {MAX_SCALED_SIDE} pixels ({hint})",
                    from.0, from.1, to.0, to.1
                )
            }
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
    if [from.0, from.1, to.0, to.1]
        .into_iter()
        .any(|side| side > MAX_SCALED_SIDE)
    {
        return Err(ScaleError::TooLong { from, to });
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
