//! The blurred copy of a wallpaper the lock screen shows.
//!
//! A locker cannot capture scoot's screen (the compositor publishes no
//! `wlr-screencopy`), so scootbg, which owns what each output shows, makes
//! the picture the locker draws: the buffer an output was rendered at,
//! averaged down to a small image and blurred there. The locker scales it
//! back up to fill the output, and a blur survives scaling, so the small
//! image is the whole product: a few tens of kilobytes where a full-size
//! blur would be megabytes and a hundred milliseconds. It is the same
//! trick the build-time lock blur uses (`-resize 10% -blur`).
//!
//! **Where it runs.** On the decode worker thread, once per rendered
//! size, never on the loop and never per frame ([`blurred_png`] is called
//! from `daemon::worker`). A solid color is the same image with nothing to
//! average ([`solid_png`], on the loop, once per color change: a flat
//! picture encodes in about 0.25 ms at any size, `bench::solid`).
//!
//! **Why the rendered buffer, not the decoded image.** The buffer is what
//! the output shows after fit, crop and the wallpaper's look, at the
//! output's aspect, so the locker's `fill` shows the same framing the
//! wallpaper had.
//!
//! **Bounds.** Every dimension is checked before any arithmetic that
//! depends on it ([`MAX_SIDE`]), so the accumulators below cannot
//! overflow: the largest box averaged into one small pixel is under
//! 140 columns by [`SAMPLE_ROWS`] rows, 255 each, far inside a `u32`. Nothing here
//! indexes without a check: a malformed buffer is an error, not a panic
//! (release builds abort on one).

use std::fmt;

use crate::color::Color;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

/// The longer side of the small image, at most. The locker upscales it by
/// the ratio to the output (about 6 at 1080p), and the blur radius scales
/// with it so the look is the same at any output size.
pub const LONG_SIDE: u32 = 320;

/// The largest buffer side blurred. Past it (no real output is anywhere
/// near) the box sizes below could overflow, so there is no lock image.
pub const MAX_SIDE: u32 = 65_536;

/// Blur passes: three box passes approximate a Gaussian.
const PASSES: usize = 3;

/// The blur's standard deviation as thousandths of the small image's
/// width (about the build-time blur's 3.1 percent of the picture), which
/// `PASSES` box passes of radius `r` give as `r + 0.5`.
const SIGMA_PERMILLE: u32 = 31;

/// Why there is no blurred image for a buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum BlurError {
    /// A side is zero or past [`MAX_SIDE`].
    Size(u32, u32),
    /// The pixels are shorter than the dimensions say.
    Short,
    /// The PNG encoder failed (out of memory is the only way).
    Encode(String),
}

impl fmt::Display for BlurError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size(w, h) => write!(f, "a {w}x{h} buffer cannot be blurred"),
            Self::Short => write!(f, "the buffer is shorter than its size says"),
            Self::Encode(why) => write!(f, "cannot encode the lock image: {why}"),
        }
    }
}

/// The small image's size for a `width` by `height` buffer: the same
/// aspect (so `fill` crops nothing), the longer side at most
/// [`LONG_SIDE`], neither side under 1.
pub fn small_dims(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let long = width.max(height);
    if long <= LONG_SIDE {
        return Some((width, height));
    }
    let scale = |side: u32| {
        let scaled =
            (u64::from(side) * u64::from(LONG_SIDE) + u64::from(long) / 2) / u64::from(long);
        u32::try_from(scaled)
            .unwrap_or(LONG_SIDE)
            .clamp(1, LONG_SIDE)
    };
    Some((scale(width), scale(height)))
}

/// The blurred small image of `xrgb`, an `XRGB8888` buffer (little
/// endian: B, G, R, unused) of `width` by `height` pixels with `stride`
/// bytes per row, as a PNG.
pub fn blurred_png(
    xrgb: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<Vec<u8>, BlurError> {
    let (small_w, small_h) = small_dims(width, height).ok_or(BlurError::Size(width, height))?;
    let mut rgb = average(xrgb, (width, height), stride, (small_w, small_h))?;
    let radius = usize::try_from(small_w * SIGMA_PERMILLE / 1000).map_or(1, |r| r.max(1));
    blur(&mut rgb, small_w as usize, small_h as usize, radius);
    encode(&rgb, small_w, small_h)
}

/// The same-sized image of one color, for a solid wallpaper.
pub fn solid_png(color: Color, width: u32, height: u32) -> Result<Vec<u8>, BlurError> {
    let (small_w, small_h) = small_dims(width, height).ok_or(BlurError::Size(width, height))?;
    let pixels = (small_w as usize)
        .checked_mul(small_h as usize)
        .ok_or(BlurError::Size(width, height))?;
    let mut rgb = Vec::with_capacity(pixels * 3);
    for _ in 0..pixels {
        rgb.extend_from_slice(&[color.r, color.g, color.b]);
    }
    encode(&rgb, small_w, small_h)
}

/// Where box `index` of `boxes` starts when `total` units are split
/// among them: every box is at least one unit when `boxes <= total`, and
/// the sizes differ by at most one.
fn edge(index: usize, boxes: usize, total: usize) -> usize {
    // In u64: `index * total` is under 2^32 by `MAX_SIDE`, but 32-bit usize
    // is not assumed. The result is at most `total`.
    ((index as u64 * total as u64) / boxes as u64) as usize
}

/// Rows of a box actually read, at most: past this the box is sampled on
/// every few rows (the blur that follows erases what they would add), which
/// cuts the memory traffic of the one pass that reads the whole buffer.
const SAMPLE_ROWS: usize = 4;

/// Area-averages `xrgb` down to `small`, as tightly packed RGB: every
/// pixel of a box across, and a spread of at most [`SAMPLE_ROWS`] rows down
/// it (all of them while the box is that short).
fn average(
    xrgb: &[u8],
    (width, height): (u32, u32),
    stride: usize,
    (small_w, small_h): (u32, u32),
) -> Result<Vec<u8>, BlurError> {
    let (w, h) = (width as usize, height as usize);
    let (sw, sh) = (small_w as usize, small_h as usize);
    let row_bytes = w.checked_mul(4).ok_or(BlurError::Short)?;
    if stride < row_bytes {
        return Err(BlurError::Short);
    }
    // The last row needs only its pixels, not the stride's padding.
    let needed = stride
        .checked_mul(h - 1)
        .and_then(|rows| rows.checked_add(row_bytes))
        .ok_or(BlurError::Short)?;
    if xrgb.len() < needed {
        return Err(BlurError::Short);
    }
    // Box edges across, once: not one division per box per row.
    let columns: Vec<usize> = (0..=sw).map(|i| edge(i, sw, w)).collect();
    let mut out = vec![0_u8; sw * sh * 3];
    // One small row's sums at a time: B, G, R per column.
    let mut sums = vec![[0_u32; 3]; sw];
    for sy in 0..sh {
        sums.fill([0; 3]);
        let (y0, y1) = (edge(sy, sh, h), edge(sy + 1, sh, h));
        let step = ((y1 - y0) / SAMPLE_ROWS).max(1);
        let mut rows = 0_u32;
        for y in (y0..y1).step_by(step) {
            rows += 1;
            let row = xrgb
                .get(y * stride..y * stride + row_bytes)
                .ok_or(BlurError::Short)?;
            for (cell, span) in sums.iter_mut().zip(columns.windows(2)) {
                let pixels = row.get(span[0] * 4..span[1] * 4).ok_or(BlurError::Short)?;
                let (mut b, mut g, mut r) = (0_u32, 0_u32, 0_u32);
                for &[pb, pg, pr, _] in pixels.as_chunks::<4>().0 {
                    b += u32::from(pb);
                    g += u32::from(pg);
                    r += u32::from(pr);
                }
                cell[0] += b;
                cell[1] += g;
                cell[2] += r;
            }
        }
        for (sx, (cell, span)) in sums.iter().zip(columns.windows(2)).enumerate() {
            // At least one row and one column: `small` is at most the size.
            let count = rows * (span[1] - span[0]) as u32;
            let at = (sy * sw + sx) * 3;
            // Sums are B, G, R; the picture is R, G, B. Rounded to nearest.
            let mean = |sum: u32| ((sum + count / 2) / count) as u8;
            out[at] = mean(cell[2]);
            out[at + 1] = mean(cell[1]);
            out[at + 2] = mean(cell[0]);
        }
    }
    Ok(out)
}

/// Blurs the packed RGB image in place: `PASSES` box blurs of `radius`
/// along each axis, edges clamped (the picture's border pixel repeats, so
/// the edges do not darken or fade).
fn blur(rgb: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut line = vec![0_u8; width.max(height)];
    for _ in 0..PASSES {
        for channel in 0..3 {
            for y in 0..height {
                box_line(rgb, y * width * 3 + channel, 3, width, radius, &mut line);
            }
            for x in 0..width {
                box_line(rgb, x * 3 + channel, width * 3, height, radius, &mut line);
            }
        }
    }
}

/// One box blur of the `len` samples at `start`, `step` apart. Sums slide
/// by one sample per output; rounded to nearest, so a flat line stays
/// exactly as it was.
fn box_line(
    data: &mut [u8],
    start: usize,
    step: usize,
    len: usize,
    radius: usize,
    line: &mut [u8],
) {
    let Some(line) = line.get_mut(..len) else {
        return;
    };
    for (i, slot) in line.iter_mut().enumerate() {
        *slot = data.get(start + i * step).copied().unwrap_or(0);
    }
    let last = len - 1;
    let window = (2 * radius + 1) as u32;
    // Window for sample 0 covers -radius..=radius, clamped to the line.
    let mut sum: u32 = (0..=2 * radius)
        .map(|k| u32::from(line[k.saturating_sub(radius).min(last)]))
        .sum();
    for i in 0..len {
        if let Some(out) = data.get_mut(start + i * step) {
            *out = ((sum + window / 2) / window) as u8;
        }
        let leaving = i.saturating_sub(radius);
        let entering = (i + radius + 1).min(last);
        sum = sum + u32::from(line[entering]) - u32::from(line[leaving]);
    }
}

/// `rgb` as an 8-bit RGB PNG.
fn encode(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, BlurError> {
    let mut out = Vec::with_capacity(rgb.len() / 4 + 1024);
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    // Smooth gradients: the filter that predicts from the neighbors
    // shrinks them several times over. The fast deflate level keeps a
    // photograph's picture to about 1 ms and 50 KB where the default took
    // 20 to 30 ms for 36 KB: the picture is small either way, and the
    // milliseconds are on every `set`.
    encoder.set_filter(png::Filter::Paeth);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|e| BlurError::Encode(e.to_string()))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| BlurError::Encode(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| BlurError::Encode(e.to_string()))?;
    Ok(out)
}
