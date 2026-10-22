//! App icons: the Icon Theme lookup, PNG first, SVG through `resvg`.
//!
//! The desktop entry's `Icon=` is a theme name (`firefox`), an absolute
//! path (`/usr/share/icons/.../firefox.png`), or empty. A name resolves
//! through the configured theme, its `Inherits` chain, `hicolor`, then
//! the base directories; an absolute path is used directly. PNG wins
//! when the theme ships one at a usable size; SVG rasterizes through
//! `resvg` only when no PNG does (a default-on `svg` feature). Without
//! the feature, or when nothing resolves or decodes, the row draws a
//! letter tile in `accent`.
//!
//! Only the visible rows load, once each per open, into a bounded
//! process-lifetime cache (at most [`MAX_ENTRIES`] icons,
//! [`MAX_BYTES`] in all: past either the whole cache is dropped, as the
//! glyph cache does). The keystroke path ([`Ui`](crate::ui::Ui))
//! never touches this module, so the warm-query `count_allocations`
//! pin holds with icons on: decode and paint happen on the draw, not
//! the filter.
//!
//! ## Bounds (a hostile entry, theme or file never panics or stalls)
//!
//! The name is at most 128 bytes with no `/`, no NUL and no leading dot
//! (no traversal, no absolute path smuggled as a name). Absolute paths
//! must be absolute with no `..` component. Every file must be regular,
//! PNG at most 8 MiB with the decoder's 16 MiB budget, the header's size
//! checked before any pixel buffer exists (at most 1024 a side, 1 M
//! pixels: a decompression bomb is refused after a few dozen bytes);
//! SVG at most [`MAX_SVG_FILE`] bytes, at most [`MAX_SVG_NODES`]
//! `<` opens (one pass over the bytes before parsing), rendered at most
//! [`MAX_RENDER_SIDE`] device pixels a side. SVG loads nothing over the
//! network and no system fonts: `usvg` is built with no `resources_dir`,
//! so a file `image href` resolves to nothing drawable unless it is a
//! valid image, and the `resvg` build carries no network and no system-font
//! loading (`default-features = false`). A corrupt PNG, a huge header, a
//! symlink loop (the kernel's `ELOOP`), a missing theme and a missing
//! file are all a letter tile, never an error and never a stall.
//!
//! Rendered at device pixels: the side is the logical icon side times
//! the surface's scale factor, capped to [`MAX_RENDER_SIDE`], so a
//! 1.5 fractional output draws a 1.5x bitmap through the viewport
//! instead of a stretched 1x one.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// The longest icon name taken, in bytes.
pub const MAX_NAME_LEN: usize = 128;
/// PNG files past this are refused (as `scootui::icon::image`'s).
pub const MAX_PNG_FILE: u64 = 8 * 1024 * 1024;
/// SVG files past this are refused before parsing: icons are text-small.
#[cfg(feature = "svg")]
pub const MAX_SVG_FILE: u64 = 512 * 1024;
/// SVG `<` opens past this are refused before parsing: bounds parse
/// time and memory without parsing first.
#[cfg(feature = "svg")]
pub const MAX_SVG_NODES: usize = 20_000;
/// The longest icon side rendered, in device pixels: bounds raster
/// time. The panel asks for at most a row height times a 3x scale.
pub const MAX_RENDER_SIDE: u32 = 256;
/// Cached icons at most: the visible rows (at most 64) with room for a
/// scale change. Past it the whole cache is dropped.
pub const MAX_ENTRIES: usize = 64;
/// Cached icon bytes at most (64 icons at 64x64 premultiplied).
pub const MAX_BYTES: usize = 1024 * 1024;
/// The fallback theme when the chain names nothing usable.
pub const HICOLOR: &str = "hicolor";

/// Whether `name` is a usable theme name: 1 to 128 bytes, no `/`, no
/// NUL, not starting with a dot.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN || name.starts_with('.') {
        return false;
    }
    !name
        .bytes()
        .any(|byte| byte == b'/' || byte == b'\\' || byte == 0)
}

/// Whether `path` is a usable absolute icon path: absolute, at most
/// 4096 bytes, no NUL, no `..`.
fn is_valid_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 4096 || !path.starts_with('/') {
        return false;
    }
    if path.bytes().any(|byte| byte == 0) {
        return false;
    }
    Path::new(path)
        .components()
        .all(|c| !matches!(c, Component::ParentDir))
}

/// The search bases for theme directories: `$HOME/.icons` (legacy,
/// themes directly under it), `$XDG_DATA_HOME/icons`, each of
/// `$XDG_DATA_DIRS/icons`, in order.
pub fn theme_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && home.is_absolute()
    {
        bases.push(home.join(".icons"));
    }
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    if let Some(home) = data_home {
        bases.push(home.join("icons"));
    }
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|raw| {
            std::env::split_paths(&raw)
                .filter(|dir| dir.is_absolute())
                .collect::<Vec<_>>()
        })
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    for dir in dirs {
        bases.push(dir.join("icons"));
    }
    bases
}

/// One theme directory's size rule (Icon Theme §4).
#[derive(Debug, Clone, Copy)]
struct DirRule {
    kind: DirKind,
    size: u32,
    min: u32,
    max: u32,
    threshold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

/// One parsed theme: its directories in file order with their rules,
/// and its `Inherits` chain in order.
#[derive(Debug, Clone, Default)]
struct Theme {
    dirs: Vec<(String, DirRule)>,
    inherits: Vec<String>,
}

/// Reads `<base>/<theme>/index.theme` if present: the `Directories`
/// list and each directory's type/size bounds, plus `Inherits`. Any
/// parse failure is an empty theme (no dirs, no parents): the lookup
/// falls through to the next theme, never an error.
fn read_theme(base: &Path, theme: &str) -> Theme {
    let path = base.join(theme).join("index.theme");
    let bytes = read_capped(&path, 64 * 1024);
    let Some(bytes) = bytes else {
        return Theme::default();
    };
    let text = String::from_utf8_lossy(&bytes);
    parse_index_theme(&text)
}

/// Reads at most `cap` bytes of a regular file, following symlinks
/// (a loop is the kernel's `ELOOP`): `None` on any refusal.
fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file = std::fs::File::from(fd);
    if !file.metadata().map(|meta| meta.is_file()).unwrap_or(false) {
        return None;
    }
    use std::io::Read as _;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

/// Parses an `index.theme` body: the `[Icon Theme]` section's
/// `Directories` and `Inherits`, and each directory's `Type`, `Size`,
/// `MinSize`, `MaxSize`, `Threshold` and `Scale`. Unknown keys are
/// ignored; malformed numbers fall back to the spec's defaults.
fn parse_index_theme(text: &str) -> Theme {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        sections
            .entry(current.clone())
            .or_default()
            .insert(key.trim().to_string(), value.trim().to_string());
    }
    let icon_theme = sections.get("Icon Theme");
    let dirs: Vec<String> = icon_theme
        .and_then(|table| table.get("Directories"))
        .map(|list| {
            list.split(',')
                .map(|dir| dir.trim().to_string())
                .filter(|dir| !dir.is_empty() && dir.len() <= 256 && !dir.contains(".."))
                .collect()
        })
        .unwrap_or_default();
    let inherits: Vec<String> = icon_theme
        .and_then(|table| table.get("Inherits"))
        .map(|list| {
            list.split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| is_valid_name(name))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Theme {
        dirs: Vec::new(),
        inherits,
    };
    for dir in dirs.into_iter().take(256) {
        let table = sections.get(&dir);
        let kind = match table
            .and_then(|table| table.get("Type"))
            .map(String::as_str)
        {
            Some("Scalable") => DirKind::Scalable,
            Some("Threshold") => DirKind::Threshold,
            _ => DirKind::Fixed,
        };
        let number = |key: &str, default: u32| {
            table
                .and_then(|table| table.get(key))
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|value| (1..=1024).contains(value))
                .map(|value| value as u32)
                .unwrap_or(default)
        };
        let size = number("Size", 48);
        let rule = match kind {
            DirKind::Fixed => DirRule {
                kind,
                size,
                min: size,
                max: size,
                threshold: 2,
            },
            DirKind::Scalable => DirRule {
                kind,
                size,
                min: number("MinSize", 1),
                max: number("MaxSize", 512),
                threshold: 2,
            },
            DirKind::Threshold => DirRule {
                kind,
                size,
                min: size.saturating_sub(number("Threshold", 2)),
                max: size.saturating_add(number("Threshold", 2)),
                threshold: number("Threshold", 2),
            },
        };
        out.dirs.push((dir, rule));
    }
    out
}

/// Whether a directory of `rule` holds the requested device side:
/// Fixed matches within threshold 2 (the spec's desktop practice),
/// Scalable within its min/max, Threshold within its threshold.
fn dir_matches(rule: &DirRule, side: u32) -> bool {
    match rule.kind {
        DirKind::Fixed => side.abs_diff(rule.size) <= 2,
        DirKind::Scalable => (rule.min..=rule.max).contains(&side),
        DirKind::Threshold => side.abs_diff(rule.size) <= rule.threshold,
    }
}

/// The theme chain: the configured theme, its `Inherits` closure in
/// order, then `hicolor`. Loops cut by the visited set; at most 16
/// themes deep.
fn chain(configured: &str, bases: &[PathBuf]) -> Vec<String> {
    let mut order = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![configured.to_string()];
    while let Some(name) = stack.pop() {
        if order.len() >= 16 || !seen.insert(name.clone()) {
            continue;
        }
        // Parents resolve after the child: push inherits reversed so
        // the first parent pops first.
        let mut parents = Vec::new();
        for base in bases {
            let theme = read_theme(base, &name);
            if !theme.inherits.is_empty() {
                parents = theme.inherits.clone();
                break;
            }
        }
        order.push(name);
        for parent in parents.into_iter().rev() {
            stack.push(parent);
        }
    }
    if !order.iter().any(|name| name == HICOLOR) {
        order.push(HICOLOR.to_string());
    }
    order
}

/// Finds the icon file for `name` at `side` device pixels: the theme
/// chain's size-matching directories first (PNG before SVG), then any
/// size in the chain (closest wins), then the base directories and
/// `/usr/share/pixmaps`. An absolute path is used directly.
/// `None` when nothing usable is installed.
pub fn find_file(configured: &str, name: &str, side: u32, bases: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        if !is_valid_path(name) {
            return None;
        }
        let path = PathBuf::from(name);
        return is_usable_file(&path).then_some(path);
    }
    if !is_valid_name(name) {
        return None;
    }
    let side = side.clamp(1, MAX_RENDER_SIDE);
    let themes = chain(configured, bases);
    // Pass one: a size-matching file, PNG before SVG.
    for extension in [
        "png",
        #[cfg(feature = "svg")]
        "svg",
    ] {
        let _ = extension;
        #[cfg(not(feature = "svg"))]
        if extension == "svg" {
            continue;
        }
        for theme in &themes {
            for base in bases {
                let parsed = read_theme(base, theme);
                for (dir, rule) in &parsed.dirs {
                    if !dir_matches(rule, side) {
                        continue;
                    }
                    let candidate = base
                        .join(theme)
                        .join(dir)
                        .join(format!("{name}.{extension}"));
                    if is_usable_file(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    // Pass two: any size in the chain, closest fixed wins (ties prefer
    // the larger, which downscales sharper than the smaller stretches).
    let mut best: Option<(u32, u32, PathBuf)> = None;
    for extension in [
        "png",
        #[cfg(feature = "svg")]
        "svg",
    ] {
        let _ = extension;
        #[cfg(not(feature = "svg"))]
        if extension == "svg" {
            continue;
        }
        for theme in &themes {
            for base in bases {
                let parsed = read_theme(base, theme);
                for (dir, rule) in &parsed.dirs {
                    let candidate = base
                        .join(theme)
                        .join(dir)
                        .join(format!("{name}.{extension}"));
                    if !is_usable_file(&candidate) {
                        continue;
                    }
                    let nominal = match rule.kind {
                        DirKind::Fixed | DirKind::Threshold => rule.size,
                        DirKind::Scalable => side.clamp(rule.min, rule.max),
                    };
                    let distance = nominal.abs_diff(side);
                    let larger = u32::MAX - nominal;
                    let key = (distance, larger);
                    if best
                        .as_ref()
                        .map(|(dist, larg, _)| (*dist, *larg))
                        .is_none_or(|(dist, larg)| key < (dist, larg))
                    {
                        best = Some((distance, larger, candidate));
                    }
                }
            }
        }
        if let Some((_, _, path)) = best.take() {
            return Some(path);
        }
    }
    // Pass three: the base directories themselves and pixmaps.
    for extension in [
        "png",
        #[cfg(feature = "svg")]
        "svg",
    ] {
        let _ = extension;
        #[cfg(not(feature = "svg"))]
        if extension == "svg" {
            continue;
        }
        for base in bases {
            let candidate = base.join(format!("{name}.{extension}"));
            if is_usable_file(&candidate) {
                return Some(candidate);
            }
        }
        // `/usr/share/pixmaps` holds loose files with no theme: PNG
        // only, like the rest of the lookup (there is no XPM decoder,
        // so no `.xpm` candidate is ever searched).
        let candidate = PathBuf::from(format!("/usr/share/pixmaps/{name}.{extension}"));
        if is_usable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Whether `path` names a readable regular PNG or SVG file within the
/// size caps (extension check only; decode validates the bytes).
fn is_usable_file(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if extension != "png" && extension != "svg" {
        return false;
    }
    #[cfg(not(feature = "svg"))]
    if extension == "svg" {
        return false;
    }
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    );
    let Ok(fd) = fd else { return false };
    let file = std::fs::File::from(fd);
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    let size = meta.len();
    if size == 0 {
        return false;
    }
    if extension == "png" {
        size <= MAX_PNG_FILE
    } else {
        #[cfg(feature = "svg")]
        {
            size <= MAX_SVG_FILE
        }
        #[cfg(not(feature = "svg"))]
        {
            false
        }
    }
}

/// A decoded icon at its raster size: premultiplied `b, g, r, a`,
/// rows packed.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// What a row draws: a decoded image, or a letter tile when nothing
/// resolved or decoded (or SVG without the feature).
#[derive(Debug, Clone)]
pub enum Icon {
    Image(Decoded),
    Tile(char),
}

/// The process-lifetime icon cache: at most [`MAX_ENTRIES`] icons,
/// [`MAX_BYTES`] in all. Past either the whole cache is dropped and
/// refilled from what is drawn next. Keyed by the icon key (name or
/// path) and the device side. Only decoded images are cached: a letter
/// tile is the row's own label, cheap to redraw, so tiles never enter
/// the map (two icon-less rows must never share one entry).
#[derive(Debug, Default)]
pub struct Cache {
    entries: HashMap<(String, u32), Icon>,
    bytes: usize,
}

impl Cache {
    /// The icon for `icon` (the entry's `Icon=` value, or `None`) at
    /// `side` device pixels, loading and decoding on a miss. `label`
    /// supplies the tile letter when nothing loads. Only call for
    /// visible rows: every call may read and decode one file.
    ///
    /// Letter tiles are never cached and never allocated for: an
    /// icon-less row returns its own label's tile straight from this
    /// call, so the draw path allocates nothing and one row's letter
    /// can never leak into another row.
    pub fn get(
        &mut self,
        configured: &str,
        bases: &[PathBuf],
        icon: Option<&str>,
        label: &str,
        side: u32,
    ) -> Icon {
        let side = side.clamp(1, MAX_RENDER_SIDE);
        let Some(name) = icon.filter(|name| !name.is_empty()) else {
            return Icon::Tile(tile_letter(label));
        };
        let key = (name.to_string(), side);
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        let Some(decoded) =
            find_file(configured, name, side, bases).and_then(|path| decode_file(&path, side))
        else {
            // A name that resolves or decodes to nothing draws the
            // row's own tile, uncached like the nameless case: caching
            // it under the name would hand one row's letter to every
            // other row sharing that name, and tiles cost nothing to
            // redraw.
            return Icon::Tile(tile_letter(label));
        };
        let bytes = decoded.pixels.len();
        if self.entries.len() + 1 > MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes += bytes;
        let icon = Icon::Image(decoded);
        self.entries.insert(key, icon.clone());
        icon
    }

    /// Cached icons and their bytes, for tests.
    #[cfg(test)]
    pub fn cached(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
}

/// The tile letter: the first alphanumeric of `label`, uppercased, or
/// `?` when there is none.
pub fn tile_letter(label: &str) -> char {
    label
        .chars()
        .find(|char| char.is_alphanumeric())
        .map(|char| char.to_ascii_uppercase())
        .unwrap_or('?')
}

/// Decodes `path` (a PNG or SVG found by [`find_file`]) at `side`
/// device pixels. `None` for anything unusable: the caller draws a
/// tile instead.
fn decode_file(path: &Path, side: u32) -> Option<Decoded> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match extension {
        "png" => decode_png(path, side),
        #[cfg(feature = "svg")]
        "svg" => decode_svg(path, side),
        _ => None,
    }
}

/// Decodes a PNG and fits it into a `side x side` premultiplied
/// square through the shared sampler: the same bounded decode the
/// tray uses, then centered with transparent margins.
fn decode_png(path: &Path, side: u32) -> Option<Decoded> {
    let side = side.clamp(1, MAX_RENDER_SIDE);
    let image = scootui::icon::image::load(path).ok()?;
    let mut out = vec![0u8; side as usize * side as usize * 4];
    image.scale_into(side, &mut out);
    Some(Decoded {
        width: side,
        height: side,
        pixels: Arc::from(out.into_boxed_slice()),
    })
}

/// Decodes and rasterizes an SVG at `side` device pixels: the file
/// capped, the node count capped before parsing, no external
/// references (no `resources_dir`), the render capped.
#[cfg(feature = "svg")]
fn decode_svg(path: &Path, side: u32) -> Option<Decoded> {
    let bytes = read_capped(path, MAX_SVG_FILE)?;
    if bytes.len() as u64 > MAX_SVG_FILE {
        return None;
    }
    // The node-count cap: one pass over the bytes, no allocation.
    let mut nodes = 0usize;
    for window in bytes.windows(1) {
        if window[0] == b'<' {
            nodes += 1;
            if nodes > MAX_SVG_NODES {
                return None;
            }
        }
    }
    if nodes == 0 {
        return None;
    }
    let options = usvg_options();
    let tree = resvg::usvg::Tree::from_data(&bytes, &options).ok()?;
    let size = tree.size();
    if size.width() <= 0.0
        || size.height() <= 0.0
        || !size.width().is_finite()
        || !size.height().is_finite()
    {
        return None;
    }
    let side = side.clamp(1, MAX_RENDER_SIDE);
    let scale = side as f32 / size.width().max(size.height());
    if !scale.is_finite() || scale <= 0.0 || scale > 16.0 {
        return None;
    }
    let width = (size.width() * scale)
        .round()
        .clamp(1.0, MAX_RENDER_SIDE as f32) as u32;
    let height = (size.height() * scale)
        .round()
        .clamp(1.0, MAX_RENDER_SIDE as f32) as u32;
    if width == 0 || height == 0 || width > MAX_RENDER_SIDE || height > MAX_RENDER_SIDE {
        return None;
    }
    // Render centered into the square, like the image path's fit.
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    let tx = (side as f32 - width as f32) / 2.0;
    let ty = (side as f32 - height as f32) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_translate(tx, ty).post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let data = pixmap.data();
    let mut out = vec![0u8; side as usize * side as usize * 4];
    for (dest, src) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(data.as_chunks::<4>().0.iter())
    {
        // tiny-skia is R,G,B,A premultiplied; the canvas takes B,G,R,A.
        dest.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
    }
    Some(Decoded {
        width: side,
        height: side,
        pixels: Arc::from(out.into_boxed_slice()),
    })
}

#[cfg(feature = "svg")]
fn usvg_options() -> resvg::usvg::Options<'static> {
    // No `resources_dir`: external images and fonts resolve to
    // nothing, by construction. No system fonts (the `resvg`
    // dependency carries `default-features = false`): text in an icon
    // draws as nothing rather than pulling a font stack.
    resvg::usvg::Options::default()
}
