//! Animated images: GIF, APNG and animated WebP, decoded to full-canvas
//! RGB frames with their delays.
//!
//! The decoders are the ones already in the tree where possible: `png`
//! for APNG (`Reader::next_frame`, composited here with its dispose and
//! blend ops) and `image-webp` for animated WebP (`read_frame`, already
//! composited by the crate). GIF needs a new decoder: `gif` (MIT/Apache,
//! pure Rust, `default-features = false` so no `color_quant`, which only
//! the encoder needs).
//!
//! **What is stored.** One RGB frame per animation frame (3 bytes a
//! pixel, flattened over the fill, like a static decode), at the image's
//! own size. No diffs, no compressed retention: the cap below keeps this
//! honest, and the worker scales each frame once per output size (the
//! ticket's "decode once, scale once per output").
//!
//! **Caps.** At most [`MAX_ANIMATED_FRAMES`] frames and
//! [`MAX_ANIMATED_BYTES`] bytes of source RGB in all; a file past either
//! is refused with [`DecodeError::TooLarge`]-shaped messaging that names
//! `--no-animate` (show the first frame only). Each frame's size is also
//! checked against [`crate::image::decode::MAX_PIXELS`] from its header,
//! before anything its size is allocated. The scaler's own probe
//! (`crate::image::scale`) still guards each scaled frame: a refusal
//! there is a `draw_error`, never an abort.
//!
//! **Delays.** Each frame's delay is normalized by [`normalize_delay`]:
//! anything below 20 ms becomes 100 ms (what browsers do for tiny GIF
//! delays; a 0 ms animation would otherwise busy-loop the daemon), and
//! anything above an hour is clamped there. Looping is forever: the file's
//! own loop count is ignored (a wallpaper loops; trivially reversible to
//! respecting it later).
//!
//! **Orientation.** EXIF orientation (JPEG/WebP/PNG eXIf) is read once
//! and stored on the animation; each frame is rendered through it like a
//! static image, so an oriented animation shows oriented.
//!
//! A single-frame GIF, a non-animated PNG/WebP, or a JPEG is not an
//! animation: [`decode_animated`] returns `None` for those, and the caller
//! draws them the static way (zero wakeups).

use std::io::{BufRead, Seek};
use std::path::Path;

use crate::color::Color;
use crate::image::decode::{DecodeError, Decoded, budget, to_rgb};
use crate::image::orientation::Orientation;
use crate::image::{exif, scale::rgb_len};

#[cfg(test)]
mod tests;

/// At most this many frames are kept: a 64-frame animation at 20 ms a
/// frame still runs 1.3 s a loop; more is not a wallpaper.
pub const MAX_ANIMATED_FRAMES: usize = 64;

/// At most this many bytes of source RGB in all frames: 64 MiB, about ten
/// 1080p frames or two 4K frames. Past it the animation is refused (show
/// the first frame with `--no-animate`).
pub const MAX_ANIMATED_BYTES: usize = 64 << 20;

/// A delay below this becomes [`FALLBACK_DELAY_MS`]: browsers show tiny
/// GIF delays at 100 ms rather than busy-looping, and so does scootbg.
pub const MIN_DELAY_MS: u64 = 20;
/// What a tiny or zero delay becomes, in milliseconds.
pub const FALLBACK_DELAY_MS: u32 = 100;
/// No frame waits longer than this: an hour, against a stuck timer.
pub const MAX_DELAY_MS: u32 = 3_600_000;

/// One animation frame: full-canvas RGB, 3 bytes a pixel.
#[derive(Debug)]
pub struct AnimFrame {
    pub rgb: Vec<u8>,
    /// How long the frame shows. Unread until frame playback lands (the
    /// follow-up): stills show only the first frame.
    #[allow(dead_code)]
    pub delay_ms: u32,
}

/// An animation: all frames at the image's own size.
#[derive(Debug)]
pub struct Animated {
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
    pub frames: Vec<AnimFrame>,
}

/// Frame playback (the follow-up) accounts memory and indexes frames
/// with these; stills show only the first frame.
#[allow(dead_code)]
impl Animated {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Source bytes held, in all frames.
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|frame| frame.rgb.len()).sum()
    }
}

/// Normalizes a raw per-frame delay to milliseconds: below
/// [`MIN_DELAY_MS`] becomes [`FALLBACK_DELAY_MS`], above
/// [`MAX_DELAY_MS`] clamps there.
pub fn normalize_delay(raw_ms: u64) -> u32 {
    if raw_ms < MIN_DELAY_MS {
        FALLBACK_DELAY_MS
    } else if raw_ms > u64::from(MAX_DELAY_MS) {
        MAX_DELAY_MS
    } else {
        raw_ms as u32
    }
}

/// Whether `reader` holds an animated image, without decoding: GIF89a
/// with more than one image descriptor is checked by decoding (cheap:
/// headers only); this only sniffs the container.
pub fn sniff_kind(magic: &[u8]) -> Option<AnimKind> {
    if magic.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(AnimKind::Png)
    } else if magic.len() == 12 && magic.starts_with(b"RIFF") && &magic[8..12] == b"WEBP" {
        Some(AnimKind::Webp)
    } else if magic.starts_with(b"GIF87a") || magic.starts_with(b"GIF89a") {
        Some(AnimKind::Gif)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimKind {
    Gif,
    Png,
    Webp,
}

/// Decodes an animated image, or returns `None` when the image is static
/// (a JPEG, a single-frame GIF/PNG/WebP). Animated past the caps is
/// refused with a message naming `--no-animate`. The reader is borrowed
/// and always sniffed from the start, so a caller that checked first
/// hands over the same open and the image is read once.
pub fn decode_animated<R: BufRead + Seek>(
    reader: &mut R,
    fill: Color,
) -> Result<Option<Animated>, DecodeError> {
    use std::io::SeekFrom;
    // From the start, whatever a previous read left the position at.
    reader
        .seek(SeekFrom::Start(0))
        .map_err(DecodeError::Unreadable)?;
    let mut magic = [0; 12];
    let mut got = 0;
    while got < magic.len() {
        match reader.read(&mut magic[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(DecodeError::Unreadable(error)),
        }
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(DecodeError::Unreadable)?;
    let magic = &magic[..got];
    match sniff_kind(magic) {
        Some(AnimKind::Gif) => gif(reader, fill),
        Some(AnimKind::Png) => apng(reader, fill),
        Some(AnimKind::Webp) => webp(reader, fill),
        None => Ok(None),
    }
}

fn too_large_animated(width: u64, height: u64, frames: usize) -> DecodeError {
    DecodeError::Corrupt(format!(
        "animation too large: {frames} frames of {width}x{height} is past the \
         {MAX_ANIMATED_FRAMES} frames / {} MiB scootbg keeps (try --no-animate for the first frame only)",
        MAX_ANIMATED_BYTES >> 20,
    ))
}

/// Checks a candidate animation against the frame-count and byte caps.
fn check_caps(width: u32, height: u32, frames: usize) -> Result<usize, DecodeError> {
    if frames > MAX_ANIMATED_FRAMES {
        return Err(too_large_animated(width.into(), height.into(), frames));
    }
    let per = rgb_len(width, height).ok_or_else(|| DecodeError::TooLarge {
        width: width.into(),
        height: height.into(),
    })?;
    let total = per
        .checked_mul(frames)
        .ok_or_else(|| DecodeError::TooLarge {
            width: width.into(),
            height: height.into(),
        })?;
    if total > MAX_ANIMATED_BYTES {
        return Err(too_large_animated(width.into(), height.into(), frames));
    }
    Ok(per)
}

fn buffer(len: usize) -> Result<Vec<u8>, DecodeError> {
    scootbg_mem::zeroed_bytes(len).ok_or(DecodeError::OutOfMemory(len))
}

/// What an animation check found: the still to draw, or the animation
/// (whose first frame the worker draws until playback lands).
pub enum Checked {
    Static(Decoded),
    Animated(Animated),
}

/// Decodes the image at `path`, animated or static, opening the file
/// once: an animation check that fell through to the static decode must
/// not read (and open) the file twice — the daemon decodes once.
pub fn decode_checked(path: &Path, fill: Color) -> Result<Checked, DecodeError> {
    let mut reader = crate::image::decode::open(path)?;
    match decode_animated(&mut reader, fill)? {
        Some(animated) => Ok(Checked::Animated(animated)),
        None => Ok(Checked::Static(crate::image::decode::decode(
            &mut reader,
            fill,
        )?)),
    }
}

// --- GIF ---

/// Starts a GIF decode: RGBA frames, the byte cap as the decoder's own
/// memory limit, and the header size checked against the pixel budget.
fn gif_decoder<R: BufRead + Seek>(
    reader: &mut R,
) -> Result<(gif::Decoder<&mut R>, u32, u32), DecodeError> {
    use gif::ColorOutput;
    let corrupt = |error: gif::DecodingError| DecodeError::Corrupt(error.to_string());
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    // Bound each frame's allocation inside the decoder too: a header-only
    // bomb is refused before it costs the canvas.
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        std::num::NonZeroU64::new(MAX_ANIMATED_BYTES as u64)
            .unwrap_or(std::num::NonZeroU64::new(1).unwrap()),
    ));
    let decoder = options.read_info(reader).map_err(corrupt)?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let (width, height) = budget(width as u64, height as u64)?;
    Ok((decoder, width, height))
}

/// The first frame of a GIF, decoded the static way: no animation caps
/// (the caller checked them, or was asked for a still with
/// `--no-animate`). Transparent pixels show `fill`.
pub(crate) fn gif_first_frame<R: BufRead + Seek>(
    reader: &mut R,
    fill: Color,
) -> Result<Decoded, DecodeError> {
    let corrupt = |error: gif::DecodingError| DecodeError::Corrupt(error.to_string());
    let (mut decoder, width, height) = gif_decoder(reader)?;
    let Some(frame) = decoder.read_next_frame().map_err(corrupt)? else {
        return Err(DecodeError::Corrupt("the GIF holds no frames".into()));
    };
    if frame.width == 0 || frame.height == 0 {
        return Err(DecodeError::Corrupt("a GIF frame is empty".into()));
    }
    if u32::from(frame.width) > width
        || u32::from(frame.height) > height
        || u32::from(frame.left) + u32::from(frame.width) > width
        || u32::from(frame.top) + u32::from(frame.height) > height
    {
        return Err(DecodeError::Corrupt(
            "a GIF frame is outside the image".into(),
        ));
    }
    // The first frame composites onto transparency, then flattens over
    // the fill: exactly what the full decode keeps for frame zero. The
    // canvas is fallible like every other pixel buffer: past the address
    // space, a clean `OutOfMemory` refusal, never an abort.
    let pixels = width as usize * height as usize;
    let mut canvas = buffer(pixels * 4)?;
    composite_gif_rgba(&mut canvas, frame, width);
    let mut rgb = buffer(pixels * 3)?;
    flatten_rgba_over_fill(&canvas, &mut rgb, fill);
    Ok(Decoded {
        rgb,
        width,
        height,
        orientation: Orientation::NORMAL,
    })
}

fn gif<R: BufRead + Seek>(reader: &mut R, fill: Color) -> Result<Option<Animated>, DecodeError> {
    let corrupt = |error: gif::DecodingError| DecodeError::Corrupt(error.to_string());
    let (mut decoder, width, height) = gif_decoder(reader)?;
    // First pass: collect frames (RGBA at their rects) without compositing,
    // so the count cap is checked before the canvas is allocated.
    let mut raw: Vec<(gif::Frame<'static>, u32)> = Vec::new();
    // `read_next_frame` borrows the decoder; take each frame by value.
    loop {
        let next = decoder.read_next_frame().map_err(corrupt)?;
        let Some(frame) = next else { break };
        // A frame with no pixels is not a frame.
        if frame.width == 0 || frame.height == 0 {
            return Err(DecodeError::Corrupt("a GIF frame is empty".into()));
        }
        if u32::from(frame.width) > width
            || u32::from(frame.height) > height
            || u32::from(frame.left) + u32::from(frame.width) > width
            || u32::from(frame.top) + u32::from(frame.height) > height
        {
            return Err(DecodeError::Corrupt(
                "a GIF frame is outside the image".into(),
            ));
        }
        let delay_ms = normalize_delay(u64::from(frame.delay) * 10);
        // Clone the frame: the decoder reuses its buffer.
        let owned = gif::Frame {
            delay: frame.delay,
            dispose: frame.dispose,
            transparent: frame.transparent,
            needs_user_input: frame.needs_user_input,
            top: frame.top,
            left: frame.left,
            width: frame.width,
            height: frame.height,
            interlaced: frame.interlaced,
            palette: frame.palette.clone(),
            buffer: std::borrow::Cow::Owned(frame.buffer.to_vec()),
        };
        raw.push((owned, delay_ms));
        if raw.len() > MAX_ANIMATED_FRAMES {
            return Err(too_large_animated(width.into(), height.into(), raw.len()));
        }
        // Early byte-cap check: each full frame will cost width*height*3.
        if raw.len() >= 2 {
            check_caps(width, height, raw.len())?;
        }
    }
    if raw.len() < 2 {
        return Ok(None);
    }
    check_caps(width, height, raw.len())?;
    // Composite onto an RGBA canvas, starting transparent (flattened over
    // the fill per frame). Fallible, like every other pixel buffer.
    let pixels = width as usize * height as usize;
    let mut canvas = buffer(pixels * 4)?;
    let mut frames: Vec<AnimFrame> = Vec::with_capacity(raw.len());
    let mut saved: Option<Vec<u8>> = None;
    for (frame, delay_ms) in &raw {
        // `Previous` restores what was there before this frame.
        let previous = frame.dispose == gif::DisposalMethod::Previous;
        if previous {
            saved = Some(canvas.clone());
        }
        composite_gif_rgba(&mut canvas, frame, width);
        // Flatten to RGB over the fill for storage.
        let mut rgb = buffer(pixels * 3)?;
        flatten_rgba_over_fill(&canvas, &mut rgb, fill);
        frames.push(AnimFrame {
            rgb,
            delay_ms: *delay_ms,
        });
        match frame.dispose {
            gif::DisposalMethod::Background => {
                clear_rect_rgba(
                    &mut canvas,
                    frame.left,
                    frame.top,
                    frame.width,
                    frame.height,
                    width,
                );
            }
            gif::DisposalMethod::Previous => {
                if let Some(before) = saved.take() {
                    canvas.copy_from_slice(&before);
                }
            }
            _ => {}
        }
    }
    Ok(Some(Animated {
        width,
        height,
        orientation: Orientation::NORMAL,
        frames,
    }))
}

/// Composites one RGBA GIF frame onto the RGBA canvas: opaque pixels
/// replace, transparent (alpha 0) keep what was there.
fn composite_gif_rgba(canvas: &mut [u8], frame: &gif::Frame<'_>, canvas_width: u32) {
    let (left, top) = (
        u32::from(frame.left) as usize,
        u32::from(frame.top) as usize,
    );
    let (fw, fh) = (usize::from(frame.width), usize::from(frame.height));
    let cw = canvas_width as usize;
    let src = &frame.buffer;
    debug_assert_eq!(src.len(), fw * fh * 4);
    for row in 0..fh {
        let dst_row = (top + row) * cw + left;
        let src_row = row * fw;
        for col in 0..fw {
            let s = (src_row + col) * 4;
            let d = (dst_row + col) * 4;
            if s + 3 >= src.len() || d + 3 >= canvas.len() {
                continue;
            }
            if src[s + 3] == 0 {
                continue;
            }
            canvas[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

fn clear_rect_rgba(
    canvas: &mut [u8],
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    canvas_width: u32,
) {
    let (left, top) = (usize::from(left), usize::from(top));
    let (w, h) = (usize::from(width), usize::from(height));
    let cw = canvas_width as usize;
    for row in 0..h {
        let start = ((top + row) * cw + left) * 4;
        let end = start + w * 4;
        if end <= canvas.len() {
            canvas[start..end].fill(0);
        }
    }
}

fn flatten_rgba_over_fill(canvas: &[u8], rgb: &mut [u8], fill: Color) {
    debug_assert_eq!(canvas.len(), rgb.len() / 3 * 4);
    let over = |c: u8, a: u8, f: u8| -> u8 {
        let (c, a, f) = (u16::from(c), u16::from(a), u16::from(f));
        ((c * a + f * (255 - a) + 127) / 255) as u8
    };
    for (i, px) in canvas.chunks_exact(4).enumerate() {
        rgb[3 * i] = over(px[0], px[3], fill.r);
        rgb[3 * i + 1] = over(px[1], px[3], fill.g);
        rgb[3 * i + 2] = over(px[2], px[3], fill.b);
    }
}

// --- APNG ---

fn apng<R: BufRead + Seek>(reader: &mut R, fill: Color) -> Result<Option<Animated>, DecodeError> {
    let corrupt = |error: png::DecodingError| match error {
        png::DecodingError::IoError(error) if error.kind() != std::io::ErrorKind::UnexpectedEof => {
            DecodeError::Unreadable(error)
        }
        other => DecodeError::Corrupt(other.to_string()),
    };
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let header = decoder.read_header_info().map_err(corrupt)?.clone();
    let (width, height) = budget(header.width.into(), header.height.into())?;
    // The animation control is only parsed once the reader reaches the
    // first image data (`read_info` reads through acTL/fcTL to IDAT;
    // `read_header_info` alone leaves them unread). Check there: a static
    // PNG returns None and the caller draws it the static way.
    let mut reader = decoder.read_info().map_err(corrupt)?;
    let (num_frames, orientation) = match reader.info().animation_control {
        None => return Ok(None),
        Some(ac) => {
            let orientation = reader
                .info()
                .exif_metadata
                .as_deref()
                .map_or(Orientation::NORMAL, exif::orientation);
            (ac.num_frames, orientation)
        }
    };
    if num_frames < 2 {
        return Ok(None);
    }
    if num_frames as usize > MAX_ANIMATED_FRAMES {
        return Err(too_large_animated(
            width.into(),
            height.into(),
            num_frames as usize,
        ));
    }
    check_caps(width, height, num_frames as usize)?;
    let (channels, is_rgba) = match reader.output_color_type() {
        (png::ColorType::Rgb, png::BitDepth::Eight) => (3, false),
        (png::ColorType::Rgba, png::BitDepth::Eight) => (4, true),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => (1, false),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => (2, false),
        _ => return Err(DecodeError::NotAnImage),
    };
    // Canvas: RGBA always, so blend works; grey/RGB frames expand on the
    // way in. Fallible, like every other pixel buffer.
    let pixels = width as usize * height as usize;
    let mut canvas = buffer(pixels * 4)?;
    // Seed the canvas with the fill flattened? No: start transparent and
    // flatten per frame (transparent shows the fill). An opaque first
    // frame covers it anyway.
    let mut frames: Vec<AnimFrame> = Vec::with_capacity(num_frames as usize);
    let mut saved: Option<Vec<u8>> = None;
    // The decoder wants a buffer large enough for a whole frame, and
    // writes the subframe's rows into its start (`next_frame` docs).
    let full_len = pixels
        .checked_mul(channels)
        .ok_or_else(|| DecodeError::Corrupt("APNG frame size overflow".into()))?;
    let mut frame_bytes = buffer(
        reader
            .output_buffer_size()
            .unwrap_or(full_len)
            .max(full_len),
    )?;
    for index in 0..num_frames {
        let fc = reader
            .info()
            .frame_control
            .ok_or_else(|| DecodeError::Corrupt("an APNG frame has no control".into()))?;
        if fc.width == 0 || fc.height == 0 {
            return Err(DecodeError::Corrupt("an APNG frame is empty".into()));
        }
        if fc.x_offset + fc.width > width || fc.y_offset + fc.height > height {
            return Err(DecodeError::Corrupt(
                "an APNG frame is outside the image".into(),
            ));
        }
        let delay_ms = apng_delay_ms(&fc);
        let info = reader
            .next_frame(frame_bytes.as_mut_slice())
            .map_err(corrupt)?;
        if (info.width, info.height) != (fc.width, fc.height) {
            return Err(DecodeError::Corrupt(
                "an APNG frame is not its control's size".into(),
            ));
        }
        let sub = &frame_bytes[..info.buffer_size().min(frame_bytes.len())];
        let previous = fc.dispose_op == png::DisposeOp::Previous;
        if previous {
            saved = Some(canvas.clone());
        }
        composite_apng_subframe(&mut canvas, sub, &fc, width, channels, is_rgba, fill);
        let mut rgb = buffer(pixels * 3)?;
        flatten_rgba_over_fill(&canvas, &mut rgb, fill);
        frames.push(AnimFrame { rgb, delay_ms });
        match fc.dispose_op {
            png::DisposeOp::Background => {
                clear_rect_rgba(
                    &mut canvas,
                    fc.x_offset as u16,
                    fc.y_offset as u16,
                    fc.width as u16,
                    fc.height as u16,
                    width,
                );
            }
            png::DisposeOp::Previous => {
                if let Some(before) = saved.take() {
                    canvas.copy_from_slice(&before);
                }
            }
            png::DisposeOp::None => {}
        }
        if index + 1 < num_frames {
            if let Err(error) = reader.next_frame_info() {
                return Err(corrupt(error));
            }
        }
    }
    if frames.len() < 2 {
        return Ok(None);
    }
    Ok(Some(Animated {
        width,
        height,
        orientation,
        frames,
    }))
}

fn apng_delay_ms(fc: &png::FrameControl) -> u32 {
    let (num, den) = (u64::from(fc.delay_num), u64::from(fc.delay_den));
    // Per the APNG spec a denominator of 0 means 100 (ticks per second).
    let den = if den == 0 { 100 } else { den };
    if den == 0 {
        return FALLBACK_DELAY_MS;
    }
    normalize_delay(num.saturating_mul(1000) / den)
}

/// Composites one APNG subframe (packed `channels`-byte pixels) onto the
/// RGBA canvas at the control's offset, with its blend op.
fn composite_apng_subframe(
    canvas: &mut [u8],
    sub: &[u8],
    fc: &png::FrameControl,
    canvas_width: u32,
    channels: usize,
    is_rgba: bool,
    fill: Color,
) {
    let _ = fill;
    let (left, top) = (fc.x_offset as usize, fc.y_offset as usize);
    let (fw, fh) = (fc.width as usize, fc.height as usize);
    let cw = canvas_width as usize;
    let over = fc.blend_op == png::BlendOp::Over;
    for row in 0..fh {
        for col in 0..fw {
            let s = (row * fw + col) * channels;
            let d = ((top + row) * cw + left + col) * 4;
            if s + channels > sub.len() || d + 4 > canvas.len() {
                continue;
            }
            let (r, g, b, a) = match channels {
                4 => (sub[s], sub[s + 1], sub[s + 2], sub[s + 3]),
                3 => (sub[s], sub[s + 1], sub[s + 2], 255),
                2 => (sub[s], sub[s], sub[s], sub[s + 1]),
                _ => {
                    let v = sub[s];
                    (v, v, v, 255)
                }
            };
            if !over || !is_rgba && a == 255 {
                // Source: replace (opaque or whole-pixel).
                if a == 0 && over {
                    continue;
                }
                canvas[d..d + 4].copy_from_slice(&[r, g, b, a]);
            } else if a == 0 {
                continue;
            } else if a == 255 {
                canvas[d..d + 4].copy_from_slice(&[r, g, b, 255]);
            } else {
                // Porter-Duff over, non-premultiplied, rounded.
                let (dr, dg, db, da) = (canvas[d], canvas[d + 1], canvas[d + 2], canvas[d + 3]);
                let blend = |s: u8, d: u8| -> u8 {
                    let (s, d, a) = (u16::from(s), u16::from(d), u16::from(a));
                    ((s * a + d * (255 - a) + 127) / 255) as u8
                };
                canvas[d] = blend(r, dr);
                canvas[d + 1] = blend(g, dg);
                canvas[d + 2] = blend(b, db);
                // Alpha channel composes too (kept for later flattens).
                let sa = u16::from(a);
                let da = u16::from(da);
                canvas[d + 3] = (sa + da * (255 - sa) / 255).min(255) as u8;
            }
        }
    }
}

// --- Animated WebP ---

fn webp<R: BufRead + Seek>(reader: &mut R, fill: Color) -> Result<Option<Animated>, DecodeError> {
    let corrupt = |error: image_webp::DecodingError| DecodeError::Corrupt(error.to_string());
    let mut decoder = image_webp::WebPDecoder::new(reader).map_err(corrupt)?;
    if !decoder.is_animated() {
        return Ok(None);
    }
    let (width, height) = decoder.dimensions();
    let (width, height) = budget(width.into(), height.into())?;
    let num_frames = decoder.num_frames();
    if num_frames < 2 {
        return Ok(None);
    }
    if num_frames as usize > MAX_ANIMATED_FRAMES {
        return Err(too_large_animated(
            width.into(),
            height.into(),
            num_frames as usize,
        ));
    }
    check_caps(width, height, num_frames as usize)?;
    decoder.set_memory_limit(1 << 20);
    let orientation = decoder
        .exif_metadata()
        .ok()
        .flatten()
        .map_or(Orientation::NORMAL, |block| exif::orientation(&block));
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let frame_len = decoder
        .output_buffer_size()
        .ok_or_else(|| DecodeError::TooLarge {
            width: width.into(),
            height: height.into(),
        })?;
    let pixels = width as usize * height as usize;
    if frame_len != pixels * channels {
        return Err(DecodeError::Corrupt("unexpected WebP pixel layout".into()));
    }
    let mut raw = buffer(frame_len)?;
    let mut frames: Vec<AnimFrame> = Vec::with_capacity(num_frames as usize);
    // The first frame comes from `read_image` semantics? No: for animated
    // images `read_frame` returns each frame in order, already composited
    // onto the canvas, starting with the first.
    for _ in 0..num_frames {
        let duration_ms = match decoder.read_frame(raw.get_mut(..frame_len).unwrap_or_default()) {
            Ok(duration) => normalize_delay(u64::from(duration)),
            Err(image_webp::DecodingError::NoMoreFrames) => break,
            Err(error) => return Err(corrupt(error)),
        };
        let rgb = match channels {
            3 => {
                let mut out = buffer(pixels * 3)?;
                out.copy_from_slice(&raw[..pixels * 3]);
                out
            }
            _ => {
                let mut tmp = raw.clone();
                tmp.resize(frame_len.max(pixels * 3), 0);
                to_rgb(&mut tmp, pixels, 4, fill)?;
                tmp
            }
        };
        frames.push(AnimFrame {
            rgb,
            delay_ms: duration_ms,
        });
        if frames.len() > MAX_ANIMATED_FRAMES {
            return Err(too_large_animated(
                width.into(),
                height.into(),
                frames.len(),
            ));
        }
    }
    if frames.len() < 2 {
        return Ok(None);
    }
    check_caps(width, height, frames.len())?;
    Ok(Some(Animated {
        width,
        height,
        orientation,
        frames,
    }))
}
