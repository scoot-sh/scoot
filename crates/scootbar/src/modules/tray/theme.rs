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
//! The fixed size dir closest to the drawn size wins (the freedesktop
//! closest-size rule among the sizes present; ties prefer the larger),
//! then `scalable`, then the base itself: the shared cache scales the
//! stored entry to the output's device pixels like a pixmap.
//!
//! ## Bounds (a hostile item names the file)
//!
//! The name, after one accepted trailing `.png` is stripped, is at most
//! 128 bytes with no `/`, no NUL and no leading dot: no traversal, no
//! absolute path, no hidden file. `.svg`/`.xpm` names stay
//! hidden, as does any other unresolvable name. Only lowercase `.png`
//! is stripped (an `IconName` is a name without extension; a trailing
//! `.PNG` names no file the lookup probes, so it stays hidden like any
//! other unresolvable name, unless a real app trips over it).
//! `IconThemePath` must
//! be an absolute path with no `..` component. Every candidate is
//! resolved with `canonicalize` (a symlink loop is the kernel's `ELOOP`)
//! and must stay under its base's canonical form, else it is skipped.
//! The open itself goes through `openat2` with `RESOLVE_BENEATH` against
//! the base directory fd, so an intermediate component swapped after the
//! `canonicalize` check (a same-user peer controlling its own
//! `IconThemePath` directory) cannot escape the base: the kernel refuses
//! the escape with `EXDEV` and the name stays hidden. `O_NONBLOCK`/
//! `O_NOFOLLOW` and the opened fd's regular-file check still hold, so a
//! path swapped to a FIFO, device or trailing symlink between the check
//! and the open can never block the bus turn. The remaining swap the
//! kernel cannot close is the base directory itself (a peer that owns
//! its `IconThemePath` can always point it at bytes it could already
//! read): same-user threat model, payoff no more than `kill`, impact
//! capped at decoding a PNG the attacker could already read.
//! Relative `$XDG_DATA_DIRS` entries are skipped outright (the variable
//! is the bar's own environment; a relative entry would resolve against
//! the bar's working directory and at best miss). The file is at most 8 MiB, the decoder's budget
//! 16 MiB, the header's size checked before any pixel buffer exists (at
//! most 512 a side, 1 M pixels: a decompression bomb declaring a huge
//! image is refused after a few dozen bytes). The stored entry is at
//! most 64 a side, the pixmap path's bound (larger decodes are
//! downscaled once, on the bus turn): one icon per item and 32 items at
//! most, so themed icons hold at most 512 KiB in all. Only the first
//! frame of an animated PNG is used.
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

/// Stored themed bytes at most, across every item: one icon each
/// (see [`fill`](super::item::fill)), at most
/// [`MAX_ITEMS`](super::MAX_ITEMS) items, each stored at most
/// [`MAX_STORED_SIDE`](super::MAX_STORED_SIDE) a side (larger decodes
/// are downscaled to it on the bus turn, once per icon version).
pub const MAX_THEME_TOTAL_BYTES: usize =
    super::MAX_ITEMS * super::MAX_STORED_SIDE as usize * super::MAX_STORED_SIDE as usize * 4;

/// The size the lookup targets when no output size is known (the bus
/// turn has none): 24 device pixels, the middle of the 14-48 tray range
/// (the default font's 14 at scale 1 to a large font at scale 2), where
/// the 22 and 24 dirs real themes ship bracket it. A compromise at HiDPI
/// by design: the shared cache scales the stored entry smoothly to the
/// output's device pixels like a pixmap, so the lookup needs only the
/// closest kept size, not the exact one.
pub const LOOKUP_SIDE: u32 = 24;

/// Fixed size directories probed under each base's `hicolor`, with the
/// nominal side each holds: probed closest to the drawn size first (see
/// [`probe`]), so the stored entry scales from the closest kept size.
/// `scalable` and the base itself are probed after every fixed dir.
const SIZE_DIRS: &[(&str, u32)] = &[
    ("22x22/apps", 22),
    ("24x24/apps", 24),
    ("16x16/apps", 16),
    ("32x32/apps", 32),
    ("48x48/apps", 48),
    ("64x64/apps", 64),
    ("128x128/apps", 128),
    ("256x256/apps", 256),
];
/// The scalable dir, probed after every fixed size dir.
const SCALABLE_DIR: &str = "scalable/apps";

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
            return data_dirs_from(&dirs);
        }
    }
    ["/usr/local/share", "/usr/share"]
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// Splits an `XDG_DATA_DIRS` value the way [`data_dirs`] does, keeping
/// only absolute entries: the variable is the bar's own environment, and
/// a relative entry would resolve against the bar's working directory
/// (usually `/`), so at best it misses. Skipped explicitly, never fatal.
fn data_dirs_from(dirs: &std::ffi::OsStr) -> Vec<PathBuf> {
    std::env::split_paths(dirs)
        .filter(|dir| dir.is_absolute())
        .collect()
}

/// Finds and decodes the themed icon `name` for `item_id`: `theme_path`
/// (the item's `IconThemePath`) first, then `bases`. `None` when the name
/// is hostile, nothing usable is installed, or the file does not decode:
/// the caller keeps the item tracked-but-hidden, as before.
///
/// The size dir closest to [`LOOKUP_SIDE`] wins (see [`load_for_side`]).
pub fn load(
    item_id: &str,
    name: &str,
    theme_path: Option<&str>,
    bases: &[PathBuf],
) -> Option<TrayIcon> {
    load_for_side(item_id, name, theme_path, bases, LOOKUP_SIDE)
}

/// [`load`], against the size dir closest to `side` device pixels: the
/// tests' fixture sizes, instead of the default lookup size.
pub fn load_for_side(
    item_id: &str,
    name: &str,
    theme_path: Option<&str>,
    bases: &[PathBuf],
    side: u32,
) -> Option<TrayIcon> {
    // An `IconName` is a name without extension, but many apps send one
    // with: one trailing lowercase `.png` names the file it decodes to
    // (uppercase `.PNG` is left alone and stays hidden, like any other
    // unresolvable name), while
    // `.svg`/`.xpm` name formats the bar never decodes, so they stay
    // hidden like any other unresolvable name.
    if name.ends_with(".svg") || name.ends_with(".xpm") {
        return None;
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    if stem.ends_with(".svg") || stem.ends_with(".xpm") || !is_valid_name(stem) {
        return None;
    }
    let file = format!("{stem}.png");
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
        if let Some(icon) = probe(item_id, &canonical, &file, side) {
            return Some(icon);
        }
    }
    None
}

/// Probes one canonical base for `file`: the `hicolor` size dirs
/// closest to `side` first, then `scalable`, then the base itself (an
/// `IconThemePath` root or `pixmaps` holds files directly). The first
/// file that resolves inside the base and decodes wins.
fn probe(item_id: &str, base: &Path, file: &str, side: u32) -> Option<TrayIcon> {
    for dir in closest_order(side) {
        let candidate = base.join("hicolor").join(dir).join(file);
        if let Some(icon) = read_if_inside(item_id, base, &candidate) {
            return Some(icon);
        }
    }
    let candidate = base.join("hicolor").join(SCALABLE_DIR).join(file);
    if let Some(icon) = read_if_inside(item_id, base, &candidate) {
        return Some(icon);
    }
    read_if_inside(item_id, base, &base.join(file))
}

/// The fixed size dirs, closest to `side` first: the freedesktop
/// closest-size rule among the sizes present (minimal distance to the
/// drawn size; ties prefer the larger, which downscales sharper than
/// the smaller stretches). Stack-sorted: this runs once per lookup on
/// the bus turn, never per frame, and allocates nothing.
fn closest_order(side: u32) -> [&'static str; 8] {
    let mut order: [(u32, u32, &'static str); 8] = [(u32::MAX, u32::MAX, ""); 8];
    for (i, &(dir, size)) in SIZE_DIRS.iter().enumerate() {
        order[i] = (size.abs_diff(side), u32::MAX - size, dir);
    }
    // Insertion sort: eight entries, no allocation.
    for i in 1..order.len() {
        let mut j = i;
        while j > 0 && order[j] < order[j - 1] {
            order.swap(j, j - 1);
            j -= 1;
        }
    }
    let mut dirs = [""; 8];
    for (i, &(_, _, dir)) in order.iter().enumerate() {
        dirs[i] = dir;
    }
    dirs
}

/// Reads `candidate` when it resolves to a file inside `base`, and
/// decodes it: symlink escapes, loops and oversized files are skipped,
/// never fatal.
fn read_if_inside(item_id: &str, base: &Path, candidate: &PathBuf) -> Option<TrayIcon> {
    read_if_inside_impl(item_id, base, candidate, None)
}

/// [`read_if_inside`], with a seam for the race test: `between`, when
/// set, runs after the `canonicalize` containment check and before the
/// contained open, so a test can swap an intermediate directory component
/// the way a same-user peer owning its `IconThemePath` could. Production
/// passes `None` (one extra branch, no allocation).
fn read_if_inside_impl(
    item_id: &str,
    base: &Path,
    candidate: &PathBuf,
    between: Option<&dyn Fn()>,
) -> Option<TrayIcon> {
    let canonical = std::fs::canonicalize(candidate).ok()?;
    if !canonical.starts_with(base) {
        return None;
    }
    if let Some(swap) = between {
        swap();
    }
    // The contained open closes the intermediate-component swap the
    // check above cannot: `RESOLVE_BENEATH` refuses any escape past the
    // base with `EXDEV`, atomically in the kernel.
    let file = open_contained(base, candidate).or_else(|| read_file_fallback(&canonical))?;
    read_from_file(item_id, file)
}

/// Opens `candidate` (built as `base` plus a relative suffix) contained
/// under `base` with `openat2` `RESOLVE_BENEATH`: an intermediate symlink
/// swapped in after the `canonicalize` check cannot escape. `None` on any
/// refusal, including an escape (`EXDEV`), a trailing symlink
/// (`O_NOFOLLOW`), or a kernel without `openat2` (`ENOSYS`, left for the
/// fallback below).
fn open_contained(base: &Path, candidate: &Path) -> Option<std::fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags};
    let relative = candidate.strip_prefix(base).ok()?;
    let base_fd = rustix::fs::open(
        base,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let fd = rustix::fs::openat2(
        &base_fd,
        relative,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH,
    )
    .ok()?;
    Some(std::fs::File::from(fd))
}

/// The pre-`openat2` open, kept as the `ENOSYS` fallback only: kernels
/// before 5.6 have no `openat2`, so a contained open is impossible there
/// and the `canonicalize` check plus `O_NOFOLLOW`/`O_NONBLOCK` is still
/// better than hiding every themed icon. It never runs on a kernel that
/// refused an escape (`EXDEV` stays refused): [`open_contained`] maps
/// every `openat2` error to `None`, and only this fallback's own `None`
/// hides the icon.
fn read_file_fallback(canonical: &Path) -> Option<std::fs::File> {
    use rustix::fs::{Mode, OFlags};
    // Probe once whether `openat2` exists at all: kernels before 5.6
    // return `ENOSYS`. Newer kernels that refused this very path (an
    // escape, a trailing symlink) must not fall through to an
    // unconstrained open, so the fallback opens nothing unless the
    // syscall itself is missing. The probe costs one failed `openat2`
    // per fallback call, which runs only when the contained open already
    // failed (a miss, never per frame).
    if !openat2_missing() {
        return None;
    }
    let fd = rustix::fs::open(
        canonical,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    Some(std::fs::File::from(fd))
}

/// Whether this kernel has no `openat2` syscall (before 5.6): the one
/// case the contained open cannot cover. Probed with `CWD` and an empty
/// path so no filesystem path is touched; re-probed per fallback call (a
/// miss path only, never per frame).
fn openat2_missing() -> bool {
    use rustix::fs::{Mode, OFlags, ResolveFlags};
    matches!(
        rustix::fs::openat2(
            rustix::fs::CWD,
            std::path::Path::new(""),
            OFlags::RDONLY,
            Mode::empty(),
            ResolveFlags::empty(),
        ),
        Err(rustix::io::Errno::NOSYS)
    )
}

/// Reads the resolved `path` and decodes it: symlink escapes, loops
/// and oversized files are skipped, never fatal.
///
/// Test-only: the FIFO watchdog test's direct call (it opens without a
/// base, exactly the old semantics; the lookup itself goes through
/// [`read_if_inside`]'s contained open above).
///
/// The open never blocks the bus turn: `O_NONBLOCK` means a FIFO or
/// socket opened here (a path swapped between the `canonicalize` above
/// and this open) returns at once instead of waiting for a writer, and
/// the `fstat` below refuses anything but a regular file before a byte
/// is read. `O_NOFOLLOW` refuses a trailing symlink the same way (the
/// canonical path has none: only a racy replacement trips it, and a
/// miss stays tracked-but-hidden). The size cap is re-checked on the
/// opened fd's own metadata, and the read is bounded by the cap, so a
/// file growing during the read is refused rather than over-read.
#[cfg(test)]
fn read_file(item_id: &str, path: &Path) -> Option<TrayIcon> {
    use rustix::fs::{Mode, OFlags};
    use std::os::fd::OwnedFd;
    let fd: OwnedFd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    read_from_file(item_id, std::fs::File::from(fd))
}

/// Reads an opened icon file and decodes it: the shared tail of the
/// contained open and the fallback (and the direct `read_file` above).
/// The `fstat` refuses anything but a regular file under the size cap
/// before a byte is read, and the read stays `take`-bounded.
fn read_from_file(item_id: &str, file: std::fs::File) -> Option<TrayIcon> {
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    if size == 0 || size > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::with_capacity(size.min(MAX_FILE) as usize);
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
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
    // `Indexed` never arrives here: with `EXPAND` the png 0.18 decoder
    // expands palette images to `Rgb`/`Rgba` before the frame is returned
    // (see `output_color_type`: `Indexed` maps to `Rgb`, or `Rgba` with
    // transparency), so palette PNGs decode through the arms above. The
    // arm below pins that: keep it, so a decoder upgrade that stops
    // expanding surfaces here as a refused icon rather than a misread one.
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
    // The stored entry is the pixmap path's bound (real theme icons are
    // 16-48 px; the cache scales it to the output's device pixels like a
    // pixmap): a larger decode is downscaled once, here on the bus turn,
    // through the same filter the cache draws with.
    let bound = super::MAX_STORED_SIDE;
    let (width, height, pixels) = if width.max(height) > bound {
        let mut out = vec![0u8; bound as usize * bound as usize * 4];
        crate::icon::sample::scale_into(&pixels, width as usize, height as usize, bound, &mut out);
        (bound, bound, out.into_boxed_slice())
    } else {
        (width, height, pixels)
    };
    // One icon's share of the total themed budget: the downscale above
    // holds it, and this pins the two together if either changes.
    debug_assert!(
        width as usize * height as usize * 4 <= MAX_THEME_TOTAL_BYTES / super::MAX_ITEMS,
        "a stored themed icon exceeds its share of the total budget"
    );
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
