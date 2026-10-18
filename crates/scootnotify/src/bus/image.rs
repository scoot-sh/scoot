//! `image-data` validation and downscale: the spec's §5.2 struct to a
//! small RGBA icon.
//!
//! The struct is `(iiibiiay)`: width, height, rowstride, has_alpha,
//! bits_per_sample, channels, pixels. Validated as the design requires —
//! sides 1 to 2048, 8 bits, 3 or 4 channels agreeing with `has_alpha`, a
//! rowstride holding a row, and checked arithmetic proving the pixels
//! cover the rows — anything else means no image. A valid image is
//! downscaled at arrival to the icon size in device pixels (at most
//! [`MAX_ICON_SIDE`]) through an area average, and the source is dropped
//! by the caller keeping only the result.

use scoot_dbus::proto::Reader;

/// A side past this is no image: the design's bound.
pub const MAX_IMAGE_SIDE: i32 = 2048;

/// The icon's longest side, in device pixels: the downscale target.
pub const MAX_ICON_SIDE: u32 = 128;

/// The struct's fields, borrowed: what the hint carried, before the
/// bounds below are checked.
#[derive(Debug)]
pub struct RawImage<'a> {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub pixels: &'a [u8],
}

/// A validated image, ready to downscale.
#[derive(Debug)]
pub struct ValidImage<'a> {
    raw: RawImage<'a>,
}

/// The downscaled icon: RGBA, at most [`MAX_ICON_SIDE`] a side, the only
/// pixels the daemon keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The icon's width, in pixels.
    pub width: u32,
    /// The icon's height, in pixels.
    pub height: u32,
    /// Row-major RGBA, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Reads the struct's fields from a variant value (whose signature the
/// caller already matched): `Err` refuses it, and the hint is ignored.
/// Never panics: every read is bounds-checked by the cursor.
pub fn read<'a>(reader: &mut Reader<'a>) -> Result<RawImage<'a>, ()> {
    reader.enter_struct()?;
    let image = RawImage {
        width: reader.i32()?,
        height: reader.i32()?,
        rowstride: reader.i32()?,
        has_alpha: reader.boolean()?,
        bits_per_sample: reader.i32()?,
        channels: reader.i32()?,
        pixels: reader.array_raw(1)?,
    };
    reader.leave_struct();
    // The struct is the variant's whole value: anything trailing it is a
    // misshapen hint, not an image.
    if !reader.exhausted() {
        return Err(());
    }
    Ok(image)
}

/// Checks the design's bounds with checked arithmetic: sides 1 to 2048,
/// 8 bits, 3 or 4 channels agreeing with `has_alpha`, a rowstride holding
/// a row, and pixels covering every row. `None` means no image.
pub fn validate<'a>(raw: &'a RawImage<'a>) -> Option<ValidImage<'a>> {
    if raw.width < 1 || raw.width > MAX_IMAGE_SIDE {
        return None;
    }
    if raw.height < 1 || raw.height > MAX_IMAGE_SIDE {
        return None;
    }
    if raw.bits_per_sample != 8 {
        return None;
    }
    let channels = match (raw.channels, raw.has_alpha) {
        (3, false) => 3usize,
        (4, true) => 4usize,
        _ => return None,
    };
    let row_len = (raw.width as usize).checked_mul(channels)?;
    let rowstride = usize::try_from(raw.rowstride).ok()?;
    if rowstride < row_len {
        return None;
    }
    // The last row's end: `rowstride * (height - 1) + row_len`, every op
    // checked, against what arrived.
    let need = rowstride
        .checked_mul(raw.height as usize - 1)?
        .checked_add(row_len)?;
    if need > raw.pixels.len() {
        return None;
    }
    Some(ValidImage {
        raw: RawImage {
            width: raw.width,
            height: raw.height,
            rowstride: raw.rowstride,
            has_alpha: raw.has_alpha,
            bits_per_sample: raw.bits_per_sample,
            channels: raw.channels,
            pixels: raw.pixels,
        },
    })
}

/// Downscales a validated image to at most [`MAX_ICON_SIDE`] a side,
/// keeping the aspect: an area average over each destination pixel's
/// source rect (a box filter — the tray's triangle filter lives behind
/// the drawing crate this ticket does not pull in), expanding 3 channels
/// to opaque RGBA. Small images are converted, never enlarged.
pub fn downscale(valid: &ValidImage<'_>) -> Image {
    let raw = &valid.raw;
    let (width, height) = (raw.width as u32, raw.height as u32);
    let longest = width.max(height);
    let (out_w, out_h) = if longest <= MAX_ICON_SIDE {
        (width, height)
    } else {
        // Ceiled division keeps the aspect without ever exceeding the
        // bound; each side at least 1. `longest` is at least 1 (both
        // sides validated so), so the divisor is never zero.
        let scaled = |side: u32| {
            ((u64::from(side) * u64::from(MAX_ICON_SIDE)).div_ceil(u64::from(longest))).max(1)
                as u32
        };
        (scaled(width), scaled(height))
    };
    let channels = raw.channels as usize;
    let rowstride = raw.rowstride as usize;
    let mut pixels = Vec::with_capacity((out_w as usize) * (out_h as usize) * 4);
    for out_y in 0..out_h {
        let src_y0 = ((out_y as u64) * (height as u64) / (out_h as u64)) as usize;
        let src_y1 = (((out_y as u64) + 1) * (height as u64) / (out_h as u64)) as usize;
        for out_x in 0..out_w {
            let src_x0 = ((out_x as u64) * (width as u64) / (out_w as u64)) as usize;
            let src_x1 = (((out_x as u64) + 1) * (width as u64) / (out_w as u64)) as usize;
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for src_y in src_y0..src_y1.max(src_y0 + 1) {
                for src_x in src_x0..src_x1.max(src_x0 + 1) {
                    let at = src_y * rowstride + src_x * channels;
                    let Some(px) = raw.pixels.get(at..at + channels) else {
                        continue;
                    };
                    sum[0] += px[0] as u64;
                    sum[1] += px[1] as u64;
                    sum[2] += px[2] as u64;
                    sum[3] += if channels == 4 { px[3] as u64 } else { 255 };
                    count += 1;
                }
            }
            let count = count.max(1);
            pixels.push((sum[0] / count) as u8);
            pixels.push((sum[1] / count) as u8);
            pixels.push((sum[2] / count) as u8);
            pixels.push((sum[3] / count) as u8);
        }
    }
    Image {
        width: out_w,
        height: out_h,
        pixels,
    }
}

/// The fuzz target's check over arbitrary bytes: a struct's value bytes,
/// refused or validated without a panic, and whatever validates
/// downscales within the icon bound with exactly its pixels.
pub fn fuzz_check(bytes: &[u8]) -> bool {
    let mut reader = Reader::le(bytes);
    let Ok(raw) = read(&mut reader) else {
        return false;
    };
    let Some(valid) = validate(&raw) else {
        return false;
    };
    let image = downscale(&valid);
    assert!(
        image.width >= 1 && image.width <= MAX_ICON_SIDE,
        "{}",
        image.width
    );
    assert!(
        image.height >= 1 && image.height <= MAX_ICON_SIDE,
        "{}",
        image.height
    );
    assert_eq!(
        image.pixels.len(),
        (image.width as usize) * (image.height as usize) * 4,
        "exactly its pixels"
    );
    true
}

#[cfg(test)]
mod tests;
