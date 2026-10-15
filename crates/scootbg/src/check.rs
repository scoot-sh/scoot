//! `scootbg check-image`: validate one wallpaper file without showing it.
//!
//! What `scoot look apply` runs over a look's wallpaper before a swap: the
//! byte cap (32 MiB) from the file's size, then the pixel cap from the
//! header, then
//! a full decode (the byte cap is a `check-image` gate; a swap-time apply
//! enforces the pixel cap and the decode),
//! answered before anything is repointed. Prints `ok: <format> <WxH>,
//! <bytes> bytes` and exits 0; anything else exits 1 naming the reason.
//! Opening cannot hang (the file is opened `O_NONBLOCK` and refused unless
//! it is a regular file, so a FIFO or a device neither blocks nor streams
//! forever), and the caps run before anything the size of the image is
//! allocated (a few hundred bytes claiming to be 100000×100000 refuses
//! from the header).

use std::path::Path;

use crate::color::Color;
use crate::image::decode::{MAX_FILE_BYTES, decode_file};

/// What `check-image` validated.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    /// `PNG`, `JPEG`, `GIF` or `WebP`, sniffed from the first bytes.
    pub format: &'static str,
    /// The decoded size.
    pub width: u32,
    pub height: u32,
    /// The file's size in bytes.
    pub bytes: u64,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ok: {} {}x{}, {} bytes",
            self.format, self.width, self.height, self.bytes
        )
    }
}

/// Validates the image at `path`: a regular file, within the byte cap, a
/// format scootbg reads, within the pixel cap, and fully decodable.
pub fn check(path: &Path) -> Result<Report, String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    if meta.is_symlink() {
        return Err(format!(
            "refused: {} is a symlink: point at the file itself",
            path.display()
        ));
    }
    if !meta.is_file() {
        return Err(format!("refused: {} is not a regular file", path.display()));
    }
    let bytes = meta.len();
    if bytes > MAX_FILE_BYTES {
        return Err(format!(
            "{} is {bytes} bytes (over the {MAX_FILE_BYTES} byte cap): refusing it",
            path.display()
        ));
    }
    let format = sniff(path).ok_or_else(|| {
        format!(
            "not an image scootbg reads (PNG, JPEG, GIF or WebP): {}",
            path.display()
        )
    })?;
    // The fill is irrelevant here (nothing is shown); black flattens
    // transparency deterministically for the decode below.
    let decoded = decode_file(path, Color { r: 0, g: 0, b: 0 })
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Report {
        format,
        width: decoded.width,
        height: decoded.height,
        bytes,
    })
}

/// Sniffs the format from the first bytes, never the name. Opens
/// `O_NONBLOCK | O_NOFOLLOW` and refuses unless the opened handle is a
/// regular file, so a FIFO swapped in after `check`'s own stat refuses
/// instead of hanging, and a planted symlink fails with `ELOOP`.
fn sniff(path: &Path) -> Option<&'static str> {
    use std::io::Read as _;
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let stat = rustix::fs::fstat(&fd).ok()?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return None;
    }
    let mut head = [0u8; 12];
    let mut file = std::fs::File::from(fd);
    let n = file.read(&mut head).ok()?;
    if n >= 8 && head[..8] == [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'] {
        Some("PNG")
    } else if n >= 3 && head[..3] == [0xFF, 0xD8, 0xFF] {
        Some("JPEG")
    } else if n >= 6 && (head[..6] == *b"GIF87a" || head[..6] == *b"GIF89a") {
        Some("GIF")
    } else if n >= 12 && head[..4] == *b"RIFF" && head[8..12] == *b"WEBP" {
        Some("WebP")
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
