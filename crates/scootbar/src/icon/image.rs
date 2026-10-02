//! An image icon: a PNG, decoded once when the config is read, held as a
//! premultiplied bitmap, scaled to the size drawn, and dropped when the
//! config is reloaded (the module holding it goes with the reload). Only
//! in a build with the `icon-image` feature, so the smallest build has no
//! decoder.
//!
//! ## Bounded
//!
//! The file is opened `O_NONBLOCK` and must be a regular file (a FIFO or a
//! device cannot hang start-up; a symlink loop is the kernel's `ELOOP`),
//! at most [`MAX_FILE`] bytes, read into memory whole. The decoder gets
//! a byte budget ([`MAX_DECODE`]), the header's size is checked before any
//! pixel buffer exists ([`MAX_SIDE`] a side, [`MAX_PIXELS`] in all, so a
//! decompression bomb declaring a huge image is refused after a few dozen
//! bytes), and an interlaced, paletted, low-bit-depth or 16-bit image is
//! expanded to 8-bit by the decoder. Only the first frame of an animated
//! PNG is used. A truncated file, a bad checksum and a zero-sized image
//! are errors, not partial pictures.
//!
//! ## Scaling
//!
//! To fit a square of `side` pixels, keeping the aspect ratio and
//! centered, with transparent margins. The filter is a separable
//! **triangle** (bilinear) filter over *premultiplied* color, its support
//! widened to the scale ratio when shrinking so every source pixel counts
//! (an area average, not point sampling, so a 256-pixel icon at 20 pixels
//! is smooth and not aliased), and plain bilinear when enlarging.
//! Premultiplied, so a transparent pixel's color never bleeds into a
//! neighbor as a dark or light fringe.

use std::fmt;
use std::io::{self, Cursor, Read};
use std::path::Path;

use rustix::fs::{FileType, Mode, OFlags, fstat, open};

#[cfg(test)]
mod tests;

/// The largest file taken, in bytes.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;
/// The decoder's allocation budget, in bytes.
pub const MAX_DECODE: usize = 16 * 1024 * 1024;
/// The longest side of an image taken, in pixels.
pub const MAX_SIDE: u32 = 1024;
/// The most pixels an image taken has (4 MiB held).
pub const MAX_PIXELS: u64 = 1024 * 1024;

/// Why a file is not usable as an icon.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    NotRegular,
    Empty,
    TooLarge(u64),
    /// The decoder refused it: not a PNG, truncated, a bad checksum.
    Png(String),
    /// A width or height of 0, or past the bounds.
    Dimensions(u32, u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::NotRegular => write!(f, "not a regular file"),
            Self::Empty => write!(f, "empty"),
            Self::TooLarge(size) => write!(f, "{size} bytes, larger than the {MAX_FILE} allowed"),
            Self::Png(error) => write!(f, "not a usable PNG: {error}"),
            Self::Dimensions(w, h) => write!(
                f,
                "{w} x {h} pixels: at least 1 and at most {MAX_SIDE} on a side, \
                 {MAX_PIXELS} pixels in all"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A decoded image: premultiplied `b, g, r, a`, rows packed.
#[derive(Debug)]
pub struct Image {
    id: u64,
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

/// Equal when the same picture: the cache id is not part of it.
impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels == other.pixels
    }
}

impl Eq for Image {}

/// Reads and decodes the PNG at `path`.
pub fn load(path: &Path) -> Result<Image, Error> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(|errno| Error::Io(errno.into()))?;
    let stat = fstat(&fd).map_err(|errno| Error::Io(errno.into()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Error::NotRegular);
    }
    let size = u64::try_from(stat.st_size).unwrap_or(0);
    if size == 0 {
        return Err(Error::Empty);
    }
    if size > MAX_FILE {
        return Err(Error::TooLarge(size));
    }
    // A file that grew past its size since `fstat` is cut at the bound
    // and refused, not read without end.
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    std::fs::File::from(fd)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::Io)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::TooLarge(bytes.len() as u64));
    }
    decode(&bytes)
}

/// Decodes PNG `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Image, Error> {
    let png = |error: png::DecodingError| Error::Png(error.to_string());
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: MAX_DECODE });
    // Ancillary chunks the bar has no use for are not kept: the decoder's
    // byte budget does not cover text and profile chunks, so a file of
    // hundreds of thousands of `tEXt` chunks would otherwise cost memory
    // in proportion. Gamma, sRGB and ICC profiles are ignored (the pixels
    // are taken as they are stored).
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    // 8 bits a channel, palettes and low depths expanded, tRNS applied.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(png)?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(Error::Dimensions(width, height));
    }
    let size = reader
        .output_buffer_size()
        .filter(|&size| size <= MAX_DECODE)
        .ok_or(Error::Dimensions(width, height))?;
    let mut buffer = vec![0u8; size];
    let frame = reader.next_frame(&mut buffer).map_err(png)?;
    let data = buffer.get(..frame.buffer_size()).unwrap_or(&[]);
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        // `EXPAND` removes it.
        png::ColorType::Indexed => return Err(Error::Png("an unexpanded palette".into())),
    };
    if frame.bit_depth != png::BitDepth::Eight || frame.width != width || frame.height != height {
        return Err(Error::Png("an unexpected pixel format".into()));
    }
    let count = width as usize * height as usize;
    if data.len() < count * channels {
        return Err(Error::Png("too little image data".into()));
    }
    let mut pixels = vec![0u8; count * 4].into_boxed_slice();
    for (out, src) in pixels.chunks_exact_mut(4).zip(data.chunks_exact(channels)) {
        let (r, g, b, a) = match *src {
            [v] => (v, v, v, 255),
            [v, a] => (v, v, v, a),
            [r, g, b] => (r, g, b, 255),
            [r, g, b, a] => (r, g, b, a),
            _ => continue,
        };
        // Rounded `c × a ÷ 255`, at most `a`.
        let premultiply = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
        out.copy_from_slice(&[premultiply(b), premultiply(g), premultiply(r), a]);
    }
    Ok(Image {
        id: super::next_id(),
        width,
        height,
        pixels,
    })
}

impl Image {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The image fitted into `out`, `side × side` premultiplied pixels
    /// (see the module docs). `out` shorter than that is left untouched.
    pub fn scale_into(&self, side: u32, out: &mut [u8]) {
        super::sample::scale_into(&self.pixels, self.width as usize, self.height as usize, side, out);
    }
}
