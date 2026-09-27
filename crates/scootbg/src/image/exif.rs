//! The one thing scootbg reads from EXIF: the orientation tag (0x0112) in
//! the first IFD. `zune-jpeg`, `png` and `image-webp` hand over the raw
//! EXIF block without interpreting it (decided in
//! `docs/scootbg/backlog/resolved/dependencies-done.md` §2).
//!
//! The block is a TIFF structure: a byte-order mark (`II` little-endian or
//! `MM` big-endian), the magic 42, and the offset of IFD0; IFD0 is a count
//! and that many 12-byte entries (tag, type, count, value). The
//! orientation is a SHORT with a count of 1, its value in the entry's first
//! two value bytes. Some writers put the JPEG APP1 marker's `Exif\0\0`
//! before it (WebP's EXIF chunk from some tools), which is skipped.
//!
//! The input is untrusted: every read is bounds-checked, offsets are added
//! with overflow checks, and anything malformed means "no orientation"
//! ([`Orientation::NORMAL`]), never a panic or an error: a picture with a
//! broken EXIF block is still a picture.

use super::orientation::Orientation;

#[cfg(test)]
pub(crate) mod tests;

const ORIENTATION_TAG: u16 = 0x0112;
/// TIFF field type SHORT (16-bit unsigned).
const SHORT: u16 = 3;

/// The orientation `exif` records, or [`Orientation::NORMAL`] when it
/// records none or cannot be read.
pub fn orientation(exif: &[u8]) -> Orientation {
    read(exif).map_or(Orientation::NORMAL, Orientation::from_exif)
}

fn read(exif: &[u8]) -> Option<u16> {
    let tiff = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let big_endian = match tiff.get(..2)? {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    let entries = ifd.checked_add(2)?;
    // At most 65,535 entries of 12 bytes, each read bounds-checked, and the
    // loop stops at the first entry past the end: bounded by the input.
    for index in 0..count {
        let entry = entries.checked_add(index.checked_mul(12)?)?;
        let tag = u16_at(entry)?;
        if tag != ORIENTATION_TAG {
            continue;
        }
        if u16_at(entry.checked_add(2)?)? != SHORT || u32_at(entry.checked_add(4)?)? != 1 {
            return None;
        }
        return u16_at(entry.checked_add(8)?);
    }
    None
}
