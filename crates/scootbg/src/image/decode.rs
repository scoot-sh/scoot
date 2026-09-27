//! From a file to packed RGB, 3 bytes a pixel, with its EXIF orientation.
//!
//! The format is sniffed from the first bytes, never the name: PNG
//! (`\x89PNG\r\n\x1a\n`), JPEG (`FF D8 FF`) and WebP (`RIFF....WEBP`),
//! each through its crate directly (dependencies-done.md §2). Anything
//! else is [`DecodeError::NotAnImage`].
//!
//! **Size first.** Every decoder's header is read, and the image refused
//! past [`MAX_PIXELS`], before anything the size of the image is
//! allocated: a few hundred bytes of PNG can claim to be 100000×100000. A
//! JPEG must also hold at least the data its size needs
//! ([`jpeg_min_len`]).
//!
//! **What is fallible, and what is committed.** Only the pixel buffer
//! allocated here (the decoded image, and the decoder's RGBA or grey
//! before it becomes RGB) is fallible: the allocator refusing it is an
//! error reply. It is also committed only as it is written
//! (`scootbg_mem::zeroed`), so a file that claims a large size and holds
//! little costs what it decodes, not what it claims. The decoders' own
//! working memory is plain infallible `Vec`s, so a refusal there aborts
//! the daemon: `zune-jpeg`'s row buffers and its progressive
//! coefficients (the image's size again, as `i16`s), `png`'s row buffers,
//! `image-webp`'s RGBA frame for an opaque lossless image (4 bytes a
//! pixel) and its YUV planes for a lossy one. Their large ones are zeroed
//! allocations, which the allocator commits lazily too: a header-only
//! 16384×16384 file of each kind raises the daemon's peak by under 0.5 MB
//! (measured in `docs/scootbg/backlog/resolved/images-decode-and-fit-done.md`).
//!
//! **Opening cannot hang.** The file is opened `O_NONBLOCK` and refused
//! unless it is a regular file, so a FIFO or a device named by mistake
//! neither blocks the worker nor streams forever.
//!
//! **Transparency** is flattened over the fill color here, once: a
//! wallpaper is opaque. Grey images become RGB, 16-bit channels 8-bit.
//! Animated PNG and WebP show their first frame (animation is milestone 2).

use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

use super::exif;
use super::orientation::Orientation;
use super::scale::rgb_len;
use crate::color::Color;

#[cfg(test)]
mod tests;

/// The largest image decoded, in pixels: 16384 × 16384 (268 megapixels,
/// 805 MB as RGB), "well above 8K × 8K" as the ticket asked. It bounds
/// what a small file can make the daemon allocate.
pub const MAX_PIXELS: u64 = 1 << 28;

/// The EXIF block read from a WebP, at most. The orientation sits in its
/// first entries, and a JPEG's is capped at 64 KiB by its segment length.
const EXIF_LIMIT: usize = 1 << 20;

/// A decoded image, as stored.
#[derive(Debug)]
pub struct Decoded {
    /// `width × height × 3` bytes, row by row.
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
}

/// Why an image could not be decoded.
#[derive(Debug)]
pub enum DecodeError {
    NotFound,
    /// A directory, a FIFO, a device: not something to decode.
    NotAFile,
    Unreadable(io::Error),
    /// Not a PNG, JPEG or WebP.
    NotAnImage,
    /// Past [`MAX_PIXELS`].
    TooLarge {
        width: u64,
        height: u64,
    },
    /// Truncated or corrupt: what the decoder said.
    Corrupt(String),
    /// The pixel buffer could not be allocated.
    OutOfMemory(usize),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "no such file"),
            Self::NotAFile => write!(f, "not a regular file"),
            Self::Unreadable(error) => write!(f, "cannot read it: {error}"),
            Self::NotAnImage => write!(f, "not an image scootbg reads (PNG, JPEG or WebP)"),
            Self::TooLarge { width, height } => write!(
                f,
                "image too large: {width}x{height} is more than the {MAX_PIXELS} pixels \
                 (16384x16384) scootbg decodes"
            ),
            Self::Corrupt(why) => write!(f, "cannot decode it (truncated or corrupt?): {why}"),
            Self::OutOfMemory(bytes) => {
                write!(
                    f,
                    "out of memory: cannot allocate {bytes} bytes for its pixels"
                )
            }
        }
    }
}

/// Decodes the image at `path`, transparency flattened over `fill`.
pub fn decode_file(path: &Path, fill: Color) -> Result<Decoded, DecodeError> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY;
    let fd = rustix::fs::open(path, flags, Mode::empty()).map_err(|errno| match errno {
        Errno::NOENT | Errno::NOTDIR => DecodeError::NotFound,
        other => DecodeError::Unreadable(other.into()),
    })?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(DecodeError::Unreadable)?;
    if !metadata.is_file() {
        return Err(DecodeError::NotAFile);
    }
    decode(BufReader::new(file), fill)
}

/// Decodes an image from `reader`, sniffing its format.
pub fn decode<R: BufRead + Seek>(mut reader: R, fill: Color) -> Result<Decoded, DecodeError> {
    let mut magic = [0; 12];
    let mut got = 0;
    while got < magic.len() {
        match reader.read(&mut magic[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(DecodeError::Unreadable(error)),
        }
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(DecodeError::Unreadable)?;
    let magic = &magic[..got];
    if magic.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(reader, fill)
    } else if magic.starts_with(&[0xff, 0xd8, 0xff]) {
        jpeg(reader)
    } else if magic.len() == 12 && magic.starts_with(b"RIFF") && &magic[8..12] == b"WEBP" {
        webp(reader, fill)
    } else {
        Err(DecodeError::NotAnImage)
    }
}

/// Checks a header's size against the budget.
fn budget(width: u64, height: u64) -> Result<(u32, u32), DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::Corrupt(format!(
            "the image is {width}x{height}"
        )));
    }
    let too_large = || DecodeError::TooLarge { width, height };
    if width
        .checked_mul(height)
        .is_none_or(|pixels| pixels > MAX_PIXELS)
    {
        return Err(too_large());
    }
    // Within the budget, each side is below 2^28.
    match (u32::try_from(width), u32::try_from(height)) {
        (Ok(width), Ok(height)) => Ok((width, height)),
        _ => Err(too_large()),
    }
}

/// A zeroed buffer of `len` bytes, or an error if it cannot be had. The
/// pages are committed only as the decoder writes them
/// (`scootbg_mem::zeroed`), so a file that claims a large size and holds
/// little costs what it holds, not what it claims.
fn buffer(len: usize) -> Result<Vec<u8>, DecodeError> {
    scootbg_mem::zeroed_bytes(len).ok_or(DecodeError::OutOfMemory(len))
}

fn jpeg<R: BufRead + Seek>(mut reader: R) -> Result<Decoded, DecodeError> {
    let corrupt = |error: zune_jpeg::errors::DecodeErrors| DecodeError::Corrupt(error.to_string());
    // No size limit of zune's own (the format's is 65535 a side): the
    // budget below is the limit. Strict, so a truncated file is an error
    // rather than a picture with a grey bottom.
    let options = DecoderOptions::default()
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX))
        .set_strict_mode(true)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let file_len = reader
        .seek(SeekFrom::End(0))
        .and_then(|len| reader.seek(SeekFrom::Start(0)).map(|_| len))
        .map_err(DecodeError::Unreadable)?;
    // By reference, so the reader is still there afterwards to check where
    // the decoder stopped.
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(&mut reader, options);
    decoder.decode_headers().map_err(corrupt)?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| DecodeError::Corrupt("no image size".into()))?;
    let (width, height) = budget(width as u64, height as u64)?;
    if file_len < jpeg_min_len(width, height) {
        return Err(DecodeError::Corrupt(format!(
            "it claims {width}x{height} pixels, which takes at least {} bytes of JPEG, \
             but the file is {file_len} bytes",
            jpeg_min_len(width, height)
        )));
    }
    if decoder.output_colorspace() != Some(ColorSpace::RGB) {
        return Err(DecodeError::NotAnImage);
    }
    let orientation = decoder
        .exif()
        .map_or(Orientation::NORMAL, |block| exif::orientation(block));
    let len = rgb_len(width, height).ok_or(DecodeError::TooLarge {
        width: width.into(),
        height: height.into(),
    })?;
    if decoder.output_buffer_size() != Some(len) {
        return Err(DecodeError::NotAnImage);
    }
    let mut rgb = buffer(len)?;
    decoder.decode_into(&mut rgb).map_err(corrupt)?;
    drop(decoder);
    if !jpeg_ended(&mut reader).map_err(DecodeError::Unreadable)? {
        return Err(DecodeError::Corrupt(
            "the file ends inside the image data".into(),
        ));
    }
    Ok(Decoded {
        rgb,
        width,
        height,
        orientation,
    })
}

/// The fewest bytes a (Huffman-coded) JPEG of `width` × `height` can be.
///
/// Every 8×8 block of the full-resolution component codes its DC
/// coefficient with a Huffman code, and a Huffman code is at least one
/// bit, so the file holds at least ⌈w/8⌉·⌈h/8⌉ bits. Arithmetic-coded
/// JPEGs (which could go below this) are not read by `zune-jpeg` at all.
///
/// It matters because `zune-jpeg` feeds zeros once the entropy-coded
/// data reaches a marker: a few hundred bytes with a frame header claiming
/// 16384×16384 and an end-of-image marker would otherwise "decode" into
/// 805 MB of flat grey, every byte written, and be shown. Real files are
/// far above the bound (a 6000×4000 photo: 7.9 MB against 47 KB).
pub fn jpeg_min_len(width: u32, height: u32) -> u64 {
    let blocks = u64::from(width.div_ceil(8)) * u64::from(height.div_ceil(8));
    blocks.div_ceil(8)
}

/// Whether a JPEG decode stopped at its end-of-image marker (`FF D9`).
///
/// Even in strict mode `zune-jpeg` checks for running out of data only at
/// the start of each row of MCUs, so a file cut inside its last row decodes
/// "successfully", padded with zeros. A complete file has its EOI right
/// where the entropy-coded data ends, which is where the decoder stopped
/// (it reads a few bytes ahead at most, or consumes the marker itself);
/// entropy-coded data never holds `FF D9` (a data `FF` is stuffed as
/// `FF 00`). So a window of 32 bytes each side of the decoder's position
/// holds the marker exactly when the scan was complete. Data *after* the
/// EOI (a phone's appended motion-photo video, a trailer) does not matter.
fn jpeg_ended<R: Read + Seek>(reader: &mut R) -> io::Result<bool> {
    let at = reader.stream_position()?;
    reader.seek(SeekFrom::Start(at.saturating_sub(32)))?;
    let mut window = [0; 64];
    let mut got = 0;
    while got < window.len() {
        match reader.read(&mut window[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(window[..got].windows(2).any(|pair| pair == [0xff, 0xd9]))
}

fn png<R: BufRead + Seek>(reader: R, fill: Color) -> Result<Decoded, DecodeError> {
    let corrupt = |error: png::DecodingError| match error {
        png::DecodingError::IoError(error) if error.kind() != io::ErrorKind::UnexpectedEof => {
            DecodeError::Unreadable(error)
        }
        other => DecodeError::Corrupt(other.to_string()),
    };
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let header = decoder.read_header_info().map_err(corrupt)?;
    let (width, height) = budget(header.width.into(), header.height.into())?;
    let mut reader = decoder.read_info().map_err(corrupt)?;
    let channels = match reader.output_color_type() {
        (png::ColorType::Grayscale, png::BitDepth::Eight) => 1,
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => 2,
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        _ => return Err(DecodeError::NotAnImage),
    };
    let orientation = reader
        .info()
        .exif_metadata
        .as_deref()
        .map_or(Orientation::NORMAL, exif::orientation);
    let frame = reader.output_buffer_size().ok_or(DecodeError::TooLarge {
        width: width.into(),
        height: height.into(),
    })?;
    let rgb_bytes = rgb_len(width, height).ok_or(DecodeError::TooLarge {
        width: width.into(),
        height: height.into(),
    })?;
    let mut rgb = buffer(frame.max(rgb_bytes))?;
    let info = reader
        .next_frame(rgb.get_mut(..frame).unwrap_or_default())
        .map_err(corrupt)?;
    if (info.width, info.height) != (width, height) || info.buffer_size() != frame {
        return Err(DecodeError::Corrupt(
            "the frame is not the image's size".into(),
        ));
    }
    to_rgb(&mut rgb, width as usize * height as usize, channels, fill)?;
    Ok(Decoded {
        rgb,
        width,
        height,
        orientation,
    })
}

fn webp<R: BufRead + Seek>(reader: R, fill: Color) -> Result<Decoded, DecodeError> {
    let corrupt = |error: image_webp::DecodingError| DecodeError::Corrupt(error.to_string());
    let mut decoder = image_webp::WebPDecoder::new(reader).map_err(corrupt)?;
    let (width, height) = decoder.dimensions();
    let (width, height) = budget(width.into(), height.into())?;
    decoder.set_memory_limit(EXIF_LIMIT);
    // A broken or oversized EXIF block is not a broken picture.
    let orientation = decoder
        .exif_metadata()
        .ok()
        .flatten()
        .map_or(Orientation::NORMAL, |block| exif::orientation(&block));
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let too_large = || DecodeError::TooLarge {
        width: width.into(),
        height: height.into(),
    };
    let frame = decoder.output_buffer_size().ok_or_else(too_large)?;
    let rgb_bytes = rgb_len(width, height).ok_or_else(too_large)?;
    if frame != rgb_bytes / 3 * channels {
        return Err(DecodeError::Corrupt("unexpected pixel layout".into()));
    }
    let mut rgb = buffer(frame.max(rgb_bytes))?;
    decoder
        .read_image(rgb.get_mut(..frame).unwrap_or_default())
        .map_err(corrupt)?;
    to_rgb(&mut rgb, width as usize * height as usize, channels, fill)?;
    Ok(Decoded {
        rgb,
        width,
        height,
        orientation,
    })
}

/// Turns `pixels` pixels of `channels` bytes each (grey, grey + alpha,
/// RGB, RGBA), at the start of `buffer`, into RGB in place, alpha
/// flattened over `fill`; `buffer` ends up exactly `pixels × 3` bytes.
///
/// Narrowing (RGBA) runs forwards, since each write lands at or before
/// what it reads; widening (grey) runs backwards for the same reason. So
/// no second buffer is needed. `buffer` must hold `pixels × max(3,
/// channels)` bytes.
pub fn to_rgb(
    buffer: &mut Vec<u8>,
    pixels: usize,
    channels: usize,
    fill: Color,
) -> Result<(), DecodeError> {
    let wrong = || DecodeError::Corrupt("pixel buffer size mismatch".into());
    let need = pixels.checked_mul(channels.max(3)).ok_or_else(wrong)?;
    if buffer.len() < need || !(1..=4).contains(&channels) {
        return Err(wrong());
    }
    let over = |c: u8, a: u8, f: u8| -> u8 {
        let (c, a, f) = (u16::from(c), u16::from(a), u16::from(f));
        // (c a + f (255 - a)) / 255, rounded: at most 255 * 255 + 127.
        ((c * a + f * (255 - a) + 127) / 255) as u8
    };
    match channels {
        3 => {}
        4 => {
            for i in 0..pixels {
                let (r, g, b, a) = (
                    buffer[4 * i],
                    buffer[4 * i + 1],
                    buffer[4 * i + 2],
                    buffer[4 * i + 3],
                );
                buffer[3 * i] = over(r, a, fill.r);
                buffer[3 * i + 1] = over(g, a, fill.g);
                buffer[3 * i + 2] = over(b, a, fill.b);
            }
        }
        1 => {
            for i in (0..pixels).rev() {
                let v = buffer[i];
                buffer[3 * i..3 * i + 3].copy_from_slice(&[v, v, v]);
            }
        }
        _ => {
            for i in (0..pixels).rev() {
                let (v, a) = (buffer[2 * i], buffer[2 * i + 1]);
                buffer[3 * i] = over(v, a, fill.r);
                buffer[3 * i + 1] = over(v, a, fill.g);
                buffer[3 * i + 2] = over(v, a, fill.b);
            }
        }
    }
    buffer.truncate(pixels * 3);
    Ok(())
}
