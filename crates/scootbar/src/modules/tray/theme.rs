//! Themed icon names: the freedesktop icon-theme lookup for tray items.
//!
//! An item that sends only `IconName` (no `IconPixmap`) names a file in
//! an icon theme instead of sending pixels. This module finds that file
//! and decodes it into a [`TrayIcon`], through the same shared icon cache
//! as pixmaps: the lookup and decode run once per `GetAll` answer (per
//! icon *version*, on the bus turn in [`fill`](super::item::fill)), never
//! per frame on the render path, and a steady item hits the cache every
//! frame like a pixmap one.
//!
//! ## What is searched
//!
//! `hicolor` only, across `$XDG_DATA_DIRS` (plus `$XDG_DATA_HOME`, and
//! `/usr/share/pixmaps` as the legacy last base), with the item's
//! `IconThemePath` first when it is one. `hicolor` is the theme every
//! theme inherits, and the one apps install their own icons into, so it
//! covers the real items (pasystray, CopyQ, KeePassXC) without the bar
//! knowing the user's theme. Full theme inheritance (reading the user's
//! theme and `index.theme` chains) is future work, said so in the ticket.
//!
//! PNG only, through the `png` decoder the `icon-image` feature already
//! carries (same version, same lock entry, no new package in the tree).
//! SVG would pull a renderer nothing else links; XPM is legacy sizes
//! real items no longer need first. A name with only an SVG or XPM beside
//! it stays tracked-but-hidden, as before.
//!
//! ## Bounds (a hostile item names the file)
//!
//! The name is at most 128 bytes with no `/`, no NUL and no leading dot:
//! no traversal, no absolute path, no hidden file. `IconThemePath` must
//! be an absolute path with no `..` component. Every candidate is
//! resolved with `canonicalize` (a symlink loop is the kernel's `ELOOP`)
//! and must stay under its base's canonical form, else it is skipped.
//! The file is at most 8 MiB, the decoder's budget 16 MiB, the header's
//! size checked before any pixel buffer exists (at most 512 a side, 1 M
//! pixels: a decompression bomb declaring a huge image is refused after
//! a few dozen bytes). Only the first frame of an animated PNG is used.
//!
//! ## Laziness (the bar's no-polling rule)
//!
//! Theme changes are picked up lazily: the next `GetAll` answer
//! re-resolves the name (`NewIcon`/`NewStatus` signals re-read the item
//! behind the 50 ms floor), or the bar's restart. Nothing watches the
//! theme directories.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use super::item::fnv;
use crate::icon::tray::TrayIcon;

/// The longest icon name taken, in bytes: item strings are cut here.
pub const MAX_NAME_LEN: usize = 128;
/// The longest theme path taken, in bytes: object paths cap here.
pub const MAX_THEME_PATH_LEN: usize = 1024;
/// The largest theme file taken, in bytes (as `image`'s).
pub const MAX_FILE: u64 = 8 * 1024 * 1024;
/// The decoder's allocation budget, in bytes (as `image`'s).
pub const MAX_DECODE: usize = 16 * 1024 * 1024;
/// The longest side of a decoded theme icon, in pixels (the tray's own
/// [`MAX_SIDE`](crate::icon::MAX_SIDE)): larger headers are refused
/// before any pixel buffer exists.
pub const MAX_THEME_SIDE: u32 = 512;
/// The most pixels a decoded theme icon has (as `image`'s).
pub const MAX_THEME_PIXELS: u64 = 1024 * 1024;

/// Size directories probed under each base's `hicolor`, in order: the
/// bar's slots are tens of device pixels, so small sizes come first and
/// the stored entry scales from the nearest kept one like a pixmap.
const SIZE_DIRS: &[&str] = &[
    "22x22/apps",
    "24x24/apps",
    "16x16/apps",
    "32x32/apps",
    "48x48/apps",
    "64x64/apps",
    "128x128/apps",
    "256x256/apps",
    "scalable/apps",
];

/// Whether `name` is a usable icon name: 1 to [`MAX_NAME_LEN`] bytes, no
/// `/`, no backslash, no NUL, not starting with a dot (no traversal, no
/// absolute path, no hidden file, no `..`).
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if name.starts_with('.') {
        return false;
    }
    !name
        .bytes()
        .any(|byte| byte == b'/' || byte == b'\\' || byte == 0)
}

/// Whether `path` is a usable `IconThemePath`: an absolute path, at most
/// [`MAX_THEME_PATH_LEN`] bytes, no NUL and no `..` component. Relative
/// paths and traversals are refused outright.
pub fn is_valid_theme_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_THEME_PATH_LEN {
        return false;
    }
    if !path.starts_with('/') {
        return false;
    }
    if path.bytes().any(|byte| byte == 0) {
        return false;
    }
    Path::new(path)
        .components()
        .all(|component| !matches!(component, Component::ParentDir))
}

/// The system search bases after `IconThemePath`: `$XDG_DATA_HOME/icons`
/// (or `~/.local/share/icons`), each of `$XDG_DATA_DIRS/icons` (or the
/// spec's default), then the legacy `/usr/share/pixmaps`.
pub fn default_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = data_home() {
        bases.push(home.join("icons"));
    }
    for dir in data_dirs() {
        bases.push(dir.join("icons"));
    }
    bases.push(PathBuf::from("/usr/share/pixmaps"));
    bases
}

fn data_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share"))
}

fn data_dirs() -> Vec<PathBuf> {
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        if !dirs.is_empty() {
            return std::env::split_paths(&dirs).collect();
        }
    }
    ["/usr/local/share", "/usr/share"]
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// Finds and decodes the themed icon `name` for `item_id`: `theme_path`
/// (the item's `IconThemePath`) first, then `bases`. `None` when the name
/// is hostile, nothing usable is installed, or the file does not decode:
/// the caller keeps the item tracked-but-hidden, as before.
pub fn load(
    item_id: &str,
    name: &str,
    theme_path: Option<&str>,
    bases: &[PathBuf],
) -> Option<TrayIcon> {
    if !is_valid_name(name) {
        return None;
    }
    let file = format!("{name}.png");
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(path) = theme_path {
        if is_valid_theme_path(path) {
            roots.push(PathBuf::from(path));
        }
    }
    roots.extend(bases.iter().cloned());
    for root in &roots {
        let Ok(canonical) = std::fs::canonicalize(root) else {
            continue;
        };
        if let Some(icon) = probe(item_id, &canonical, &file) {
            return Some(icon);
        }
    }
    None
}

/// Probes one canonical base for `file`: the `hicolor` size dirs, then
/// the base itself (an `IconThemePath` root or `pixmaps` holds files
/// directly). The first file that resolves inside the base and decodes
/// wins.
fn probe(item_id: &str, base: &Path, file: &str) -> Option<TrayIcon> {
    for size in SIZE_DIRS {
        let candidate = base.join("hicolor").join(size).join(file);
        if let Some(icon) = read_if_inside(item_id, base, &candidate) {
            return Some(icon);
        }
    }
    read_if_inside(item_id, base, &base.join(file))
}

/// Reads `candidate` when it resolves to a file inside `base`, and
/// decodes it: symlink escapes, loops and oversized files are skipped,
/// never fatal.
fn read_if_inside(item_id: &str, base: &Path, candidate: &PathBuf) -> Option<TrayIcon> {
    let canonical = std::fs::canonicalize(candidate).ok()?;
    if !canonical.starts_with(base) {
        return None;
    }
    let size = canonical.metadata().ok()?.len();
    if size == 0 || size > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::with_capacity(size.min(MAX_FILE) as usize);
    std::fs::File::open(&canonical)
        .ok()?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FILE {
        return None;
    }
    decode(item_id, &bytes)
}

/// Decodes PNG `bytes` to premultiplied pixels under an id stable for the
/// same item and pixels (so a steady icon hits the shared cache, and a
/// changed theme misses once). `None` for anything the bar does not
/// take: not a PNG, truncated, a bad checksum, a zero or past-bound
/// size, an unexpected pixel format.
fn decode(item_id: &str, bytes: &[u8]) -> Option<TrayIcon> {
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: MAX_DECODE });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if width == 0
        || height == 0
        || width > MAX_THEME_SIDE
        || height > MAX_THEME_SIDE
        || u64::from(width) * u64::from(height) > MAX_THEME_PIXELS
    {
        return None;
    }
    let size = reader
        .output_buffer_size()
        .filter(|&size| size <= MAX_DECODE)?;
    let mut buffer = vec![0u8; size];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let data = buffer.get(..frame.buffer_size())?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    if frame.bit_depth != png::BitDepth::Eight || frame.width != width || frame.height != height {
        return None;
    }
    let count = width as usize * height as usize;
    if data.len() < count * channels {
        return None;
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
        let premultiply = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
        out.copy_from_slice(&[premultiply(b), premultiply(g), premultiply(r), a]);
    }
    let id = fnv_payload(item_id, width, height, &pixels);
    TrayIcon::from_premultiplied(id, width, height, pixels)
}

/// The cache key over the item and the decoded pixels (as the pixmap
/// path's [`fnv`](super::item::fnv), but over decoded bytes).
fn fnv_payload(item_id: &str, width: u32, height: u32, pixels: &[u8]) -> u64 {
    fnv(item_id, width, height, pixels)
}

#[cfg(test)]
mod tests;
