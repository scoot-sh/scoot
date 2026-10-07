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
    /// The pre-scale probe could not reserve the scaler's budget: refusing
    /// the draw rather than letting the scaler's infallible allocation
    /// abort the daemon. See [`probe_budget`].
    NoMemory { bytes: usize },
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
            Self::NoMemory { bytes } => write!(
                f,
                "out of memory: cannot allocate {bytes} bytes for scaling"
            ),
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

/// Cover for page rounding and allocator headers between the probe's one
/// block and the scaler's several: each large mapping rounds up to a page
/// (4096), and there are under ten of them (the output, two axes' `f32`
/// and `i16` tables and bounds, the row scratch). 64 KiB is several times
/// that, and noise beside a 4K draw's ~25 MB budget.
pub const PROBE_SLOP: usize = 64 << 10;

/// The scaler's address-space budget for scaling `from` to `to` with
/// `filter`, in bytes, if it fits `usize`: the output plus the transient
/// scratch plus both axes' weight tables.
///
/// Re-derived from the pinned `pic-scale-safe` 0.1.12 source (do not trust
/// the ticket's "about 36 bytes" summary):
///
/// - `resize_rgb8` is `resize_fixed_point::<u8, i32, 3>` (`resizer.rs`);
///   `Nearest` returns early with only its `store` (`resize_fixed_point.rs`:
///   the `Nearest` branch allocates `dst w × h × 3` and nothing else, no
///   weights, no scratch), so its budget is the output alone.
/// - Every other filter builds one `generate_weights::<f32>` table per
///   scaled axis (`compute_weights.rs`): `kernel × out` `f32`s (4 bytes)
///   plus one `FilterBounds` (`filter_weights.rs`: two `usize`s, 16 bytes
///   on 64-bit) per output pixel, where `kernel` is
///   `round(base × max(in / out, 1))` with `base = min_kernel_size × 2`
///   (`sampler.rs`: Lanczos3's `min_kernel_size` 3, Catmull-Rom's 2,
///   Bilinear's 1, so bases 6, 4, 2). Only axes whose size changes get a
///   table: the scaler checks each axis separately.
/// - Each table is then copied by `numerical_approximation_i16::<PRECISION>`
///   (`filter_weights.rs`, called with alignment 0, so `align == kernel`):
///   `out × kernel` `i16`s (2 bytes) plus a second bounds copy (16 bytes
///   per output pixel), with transient `scratch` (`kernel` `f64`s, 8 bytes)
///   and `order` (`kernel` `usize`s, 8 bytes) per conversion, and
///   `local_filters` (`kernel` `f32`s, 4 bytes) while generating.
/// - The two-axis path (`convolve_trampoline_fixed_point`,
///   `fixed_point_dispatch.rs`) holds both axes' `f32` tables and both
///   `i16` tables at once, plus the output and a row scratch of
///   `src_width × 3 × min(4, dst_height)` bytes (the non-rayon path, which
///   is what scootbg builds: `pic-scale-safe` without its `rayon` feature).
///   The single-axis paths (`convolve_row/column_fixed_point`) allocate no
///   scratch, and their intermediate *is* the output, so there is nothing
///   beyond it to budget.
///
/// Per scaled axis that is `kernel × out × (4 + 2)` for the two tables,
/// `out × (16 + 16)` for the two bounds copies, and `kernel × (8 + 8 + 4)`
/// for the transients: `kernel × out × 6 + out × 32 + kernel × 20`. The
/// kernel gets one extra tap past the source's `round(...)` as a guard for
/// `f32` edge rounding, so the probe only ever over-budgets.
///
/// What the probe is, honestly: a heuristic, not a guarantee. It is nearly
/// sound under `RLIMIT_AS` (address-space limits: `ulimit -v`,
/// systemd's `LimitAS=`): the daemon draws one job at a time on its worker
/// thread, and nothing else in it allocates much meanwhile, so address
/// space the probe reserves and frees is still there when the scaler
/// allocates it. It is racy under strict overcommit
/// (`vm.overcommit_memory=2`), where another process can take the commit
/// charge between the probe and the scaler's allocation. It is also
/// conservative about fragmentation: one contiguous probe where the scaler
/// wants several smaller blocks, so it may refuse a draw that would have
/// fit, which is a graceful `draw_error`, never an abort.
pub fn probe_budget(filter: Filter, from: (u32, u32), to: (u32, u32)) -> Option<usize> {
    let target = rgb_len(to.0, to.1)?;
    if filter == Filter::Nearest {
        return target.checked_add(PROBE_SLOP);
    }
    let mut total = target;
    if from.0 != to.0 {
        total = total.checked_add(axis_budget(filter, from.0, to.0)?)?;
    }
    if from.1 != to.1 {
        total = total.checked_add(axis_budget(filter, from.1, to.1)?)?;
    }
    if from.0 != to.0 && from.1 != to.1 {
        // The trampoline's row scratch: `src_stride × min(4, dst_height)`.
        let rows = usize::try_from(to.1.min(4)).ok()?;
        let scratch = usize::try_from(from.0)
            .ok()?
            .checked_mul(3)?
            .checked_mul(rows)?;
        total = total.checked_add(scratch)?;
    }
    total.checked_add(PROBE_SLOP)
}

/// One scaled axis's weight tables plus their transients, in bytes:
/// `kernel × out × 6 + out × 32 + kernel × 20` (see [`probe_budget`]).
/// `None` on overflow. `Nearest`, or an unscaled axis, never calls this.
fn axis_budget(filter: Filter, input: u32, output: u32) -> Option<usize> {
    let base = match filter {
        Filter::Lanczos3 => 6.0f32,
        Filter::CatmullRom => 4.0f32,
        Filter::Bilinear => 2.0f32,
        Filter::Nearest => return Some(0),
    };
    // Same arithmetic as `generate_weights`: `scale = in / out`,
    // `cutoff = max(scale, 1)`, `kernel = round(base × cutoff)`.
    let scale = input as f32 / output as f32;
    let kernel = (base * scale.max(1.0)).round().max(1.0) as usize;
    // One guard tap: the probe over-budgets rather than under-counts.
    let kernel = kernel.checked_add(1)?;
    let out = usize::try_from(output).ok()?;
    kernel
        .checked_mul(out)?
        .checked_mul(6)?
        .checked_add(out.checked_mul(32)?)?
        .checked_add(kernel.checked_mul(20)?)
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
    // The probe: reserve the scaler's whole budget (output, scratch, both
    // axes' weight tables) and free it again, before the scaler's own
    // infallible `Vec`s can abort the daemon. `try_reserve_exact` only:
    // no zeroing, no page touching, just address (and commit) space. A
    // refusal is a `draw_error`, never an abort; see `probe_budget` for
    // what the number covers and where it stays heuristic.
    let budget = probe_budget(filter, from, to).ok_or(ScaleError::Size)?;
    // `try_reserve_exact` on an empty `Vec` is exactly one allocation of
    // `budget` bytes, freed on drop.
    let mut probe = Vec::<u8>::new();
    if probe.try_reserve_exact(budget).is_err() {
        return Err(ScaleError::NoMemory { bytes: budget });
    }
    drop(probe);
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
