//! Loading the cursor theme the machine already has installed.
//!
//! # Why this exists, and why it is not a license problem
//!
//! `cursor/shapes.rs` draws ten shapes because scoot may not *ship* a cursor
//! theme: niri's assets are GPL and Adwaita's are not MIT-clean, so there is
//! nothing this repository is allowed to carry. That constraint is about
//! **shipping an asset**, and it was wrongly read as blocking per-shape
//! cursors altogether (see `docs/backlog/resolved/cursor-theme-name-done.md`,
//! which used to say so).
//!
//! **Reading the theme already installed on the user's own machine is a
//! different thing entirely, and carries no license obligation for this
//! project**: the file belongs to whoever installed it, scoot neither
//! redistributes nor derives from it, and this is precisely what sway, niri,
//! Hyprland and Smithay's own `anvil` do (`anvil/src/cursor.rs`). The parsing
//! is the MIT-licensed `xcursor` crate; nothing is vendored.
//!
//! So this module is the primary source of cursor pixels for a named shape,
//! and `shapes.rs` is the fallback when there is no theme to read — which is
//! not a hypothetical: a linuxserver webtop or any minimal container is a
//! first-class scoot target and frequently has no icon theme installed at
//! all.
//!
//! # Why this matters more since `wp-cursor-shape-v1`
//!
//! Before that protocol, a client that wanted a real I-beam loaded its own
//! theme and uploaded a surface, and scoot drew the client's pixels
//! (`CursorImageStatus::Surface`). Advertising cursor-shape makes modern
//! toolkits (GTK4, and `foot`) *stop* doing that and name a shape instead —
//! so without this module, turning the protocol on would have **replaced a
//! correctly themed cursor with line art** for exactly the clients that
//! looked best. With it, the compositor answers with the same theme the
//! client would have loaded, which is what the protocol is for.
//!
//! # When the work happens
//!
//! Never on the render path. A theme image is loaded and decoded the first
//! time a client asks for that shape ([`Theme::image`], called from
//! `Cursor::set_status`, an event path), and cached from then on — including
//! a *negative* cache, so a theme that has no `zoom-in` is not re-searched on
//! every pointer move over a zoomable widget. `Cursor::element` only ever
//! reads an already-built [`MemoryRenderBuffer`].

use std::collections::HashMap;
use std::path::PathBuf;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point, Transform};
use xcursor::parser::{Image, parse_xcursor};

#[cfg(test)]
mod tests;

/// The theme name used when neither the config nor the environment names one.
///
/// `"default"` is the freedesktop convention: an icon theme directory that is
/// usually a symlink to whatever the distribution considers the system
/// cursor. `CursorTheme::load` also falls back to it when following
/// inheritance, so this matches what every other client on the machine
/// resolves to.
const DEFAULT_THEME: &str = "default";

/// One cursor image, ready to draw.
#[derive(Clone, Debug)]
pub struct Themed {
    pub buffer: MemoryRenderBuffer,
    /// The theme's own hotspot for this shape, in the image's pixels. Not
    /// interchangeable with a [`Shape`](super::shapes::Shape)'s: a theme's
    /// I-beam is anchored where *that artwork* puts its centre, which is not
    /// necessarily the middle of the bitmap.
    pub hotspot: Point<i32, Logical>,
}

/// The installed cursor theme, if there is one, plus everything loaded from
/// it so far.
pub struct Theme {
    /// `None` when no theme could be resolved at all, which is the ordinary
    /// state on a minimal container. Every lookup then misses and the caller
    /// falls back to `shapes.rs`.
    theme: Option<xcursor::CursorTheme>,
    /// The name actually resolved, kept so it can be exported to children
    /// (see [`Theme::name`]).
    name: String,
    /// The nominal size to pick images at.
    size: u32,
    /// What has been looked up. `None` memoizes "this theme does not have
    /// that shape", so a missing icon costs one failed lookup per session
    /// rather than one per `set_cursor`.
    cache: HashMap<CursorIcon, Option<Themed>>,
}

impl Theme {
    /// Resolves the theme name and opens it, loading nothing yet.
    ///
    /// Resolution order, most specific first: the `[appearance]
    /// cursor_theme` config value, then `$XCURSOR_THEME` (what the rest of
    /// the desktop already honours), then [`DEFAULT_THEME`]. Failing to find
    /// a theme is not an error and not a warning at startup: it is the normal
    /// case on a machine with no icon themes installed, and the procedural
    /// shapes cover it. Failing to find an *individual* shape in a theme that
    /// does exist is logged once, by [`Theme::image`].
    pub fn load(configured: Option<&str>, size: i32) -> Self {
        let name = configured
            .map(str::to_owned)
            .or_else(|| std::env::var("XCURSOR_THEME").ok())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| DEFAULT_THEME.to_owned());
        // `CursorTheme::load` walks the theme search path and never fails; a
        // name that matches nothing simply resolves to a theme with no icons
        // in it, which every lookup then misses. That is why this is not a
        // `Result` -- there is no error to report at load time, only an empty
        // theme to discover one lookup at a time.
        let theme = xcursor::CursorTheme::load(&name);
        // A theme is only worth keeping if it can actually produce the one
        // shape every session needs. This is also the honest place to say
        // "there is no theme here" once, rather than at every miss.
        let usable = lookup(&theme, CursorIcon::Default).is_some();
        let loaded = Self {
            theme: usable.then_some(theme),
            name,
            // Never zero: a zero nominal size would make `nearest` prefer the
            // smallest image in every file rather than the closest one.
            size: size.max(1) as u32,
            cache: HashMap::new(),
        };
        if loaded.is_loaded() {
            tracing::info!(theme = %loaded.name, size = loaded.size, "loaded a cursor theme");
        } else {
            tracing::info!(
                theme = %loaded.name,
                "no cursor theme found; using the compositor's own drawn shapes"
            );
        }
        loaded
    }

    /// The resolved theme name, whether or not it turned out to contain
    /// anything.
    ///
    /// Read by `compositor::run` to put `XCURSOR_THEME`/`XCURSOR_SIZE` in the
    /// environment every child inherits, so a client that loads a theme
    /// *itself* (GTK3, and anything predating `wp-cursor-shape-v1`) picks the
    /// same one this compositor draws — which is the whole "consistent cursor
    /// across clients" point, for the clients the protocol cannot reach.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The nominal size images are picked at.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Whether a theme was found at all.
    pub fn is_loaded(&self) -> bool {
        self.theme.is_some()
    }

    /// The theme's image for `icon`, loading and caching it on first ask.
    ///
    /// `None` means "draw the procedural shape instead": either no theme at
    /// all, or a theme that does not carry this shape under any of its
    /// names.
    pub fn image(&mut self, icon: CursorIcon) -> Option<&Themed> {
        // Not `entry().or_insert_with()`: the closure would need `&self.theme`
        // while `entry` holds `&mut self.cache`. Two statements instead, which
        // also keeps the load out of the common (cached) path entirely.
        if !self.cache.contains_key(&icon) {
            let loaded = self.load_image(icon);
            if loaded.is_none() {
                tracing::debug!(
                    ?icon,
                    theme = %self.name,
                    "the cursor theme has no image for this shape; drawing it instead"
                );
            }
            self.cache.insert(icon, loaded);
        }
        self.cache.get(&icon).and_then(Option::as_ref)
    }

    fn load_image(&self, icon: CursorIcon) -> Option<Themed> {
        let theme = self.theme.as_ref()?;
        let path = lookup(theme, icon)?;
        let bytes = std::fs::read(&path)
            .inspect_err(|error| {
                // The theme named a file and it could not be read: a broken
                // symlink, a permissions problem, a theme uninstalled while
                // the session runs. Worth a warning (unlike a shape the theme
                // simply does not have), and still not fatal.
                tracing::warn!(?path, %error, "could not read a cursor theme file");
            })
            .ok()?;
        decode(&bytes, self.size)
    }
}

/// Turns the bytes of one xcursor file into the image to draw.
///
/// Split out from [`Theme::load_image`] so the part that can be wrong in a
/// way nobody notices -- which of several sizes is picked, where the hotspot
/// lands, and whether the pixels survive with their bytes in the right order
/// -- is testable without a theme installed on the machine running the
/// suite. The file system half above has nothing left to get wrong.
fn decode(bytes: &[u8], size: u32) -> Option<Themed> {
    let images = parse_xcursor(bytes)?;
    let image = nearest(size, &images)?;
    // `pixels_rgba` despite the name: the crate's own doc says it is "in the
    // order of the file", and an xcursor file stores ARGB32 in native byte
    // order -- i.e. `[B, G, R, A]` in memory on a little-endian machine,
    // which is exactly what `Fourcc::Argb8888` means here and what
    // `shapes.rs` produces. (`pixels_argb` is the byte-swapped one, and
    // using it would tint every themed cursor.) anvil pairs the same field
    // with the same Fourcc.
    Some(Themed {
        buffer: MemoryRenderBuffer::from_slice(
            &image.pixels_rgba,
            Fourcc::Argb8888,
            (image.width as i32, image.height as i32),
            1,
            Transform::Normal,
            None,
        ),
        hotspot: (image.xhot as i32, image.yhot as i32).into(),
    })
}

/// Where the theme keeps `icon`, trying the freedesktop name first and then
/// this icon's historical aliases.
///
/// The aliases matter in practice rather than in theory: plenty of installed
/// themes carry only the legacy X11 names (`xterm` for `text`, `sb_h_double_arrow`
/// for `ew-resize`, `fleur` for `move`), and `cursor-icon`'s `alt_names`
/// is exactly that list. Without this pass a perfectly good theme would miss
/// most shapes and fall back to drawn ones.
fn lookup(theme: &xcursor::CursorTheme, icon: CursorIcon) -> Option<PathBuf> {
    std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .find_map(|name| theme.load_icon(name))
}

/// The image to draw out of everything an xcursor file holds.
///
/// A file carries several nominal sizes, and each size may carry several
/// *frames* of an animation. This picks the size closest to the one asked for
/// and then that size's first frame: scoot does not drive cursor animation
/// (there is no per-frame timer feeding the cursor, and adding one to spin a
/// busy pointer is not worth a wakeup on an otherwise idle screen), so an
/// animated cursor shows its first frame rather than nothing.
///
/// `min_by_key` on the absolute difference, like anvil's own `nearest_images`
/// -- using `size` (the nominal size the theme declares) rather than `width`,
/// because those differ for a cursor whose artwork is padded.
fn nearest(size: u32, images: &[Image]) -> Option<&Image> {
    let nearest = images
        .iter()
        .min_by_key(|image| size.abs_diff(image.size))?;
    images
        .iter()
        .find(|image| image.size == nearest.size && image.width > 0 && image.height > 0)
}
