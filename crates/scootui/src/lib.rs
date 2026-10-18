//! `scootui`: pure drawing, widgets and icons shared by the shell apps,
//! plus the Wayland scaffolding they all stand on.
//!
//! A canvas, color tokens, text with a bounded glyph cache, the popup's
//! pure half and icon rasterization, with no Wayland and no I/O. Extracted
//! from `scootbar` so `scootnotify`, `scootlaunch` and `scootgreet` draw the
//! same way without duplicating it. The per-output model (`outputs`), the
//! scale arithmetic (`density`), the input regions (`region`) and the bar
//! geometry (`bar`) moved with `scootui-extract-wayland`, still with no
//! Wayland types in them; what stays in `scootbar` is bar-shaped: modules,
//! layout, render and the layer-surface glue (which moves when
//! `scootui-surfaces-on-demand` generalizes it).
//!
//! Features: `widgets` (the popup and the ellipsis cut), `icons` (path
//! icons and the cache), `png` (PNG image icons), `tray` (tray pixmaps),
//! `wayland` (the poll loop's Wayland primitives: flush, the gone check,
//! the read after a poll), `control` (the control socket's server half:
//! line framing, socket paths, the claim and the connection bounds, with
//! the socket name a parameter of the app's) and `test-support` (the
//! headless-compositor harness the apps' integration tests share). The
//! default is `widgets` and `icons`; the smallest build has none of them
//! and draws nothing but plain canvas and text.
//!
//! No `unsafe` here: the two mappings it needs, the font and the shm
//! buffer, are `scootbg-mem`'s.

#![forbid(unsafe_code)]

// Linux only, like scootbar: the font mapping is `scootbg-mem`'s, and the
// shell apps never run on a Mac.
#[cfg(not(target_os = "linux"))]
compile_error!(
    "scootui runs on Linux only, like scootbar; on a Mac, \
     cargo check --workspace --exclude scootbar --exclude scootbg --exclude scootbg-mem --exclude scootui \
     (dev/README.md)"
);

pub mod bar;
pub mod class;
pub mod color;
pub mod density;
pub mod ellipsis;
pub mod font;
pub mod output;
pub mod outputs;
pub mod paint;
pub mod region;
pub mod text;
pub mod theme;

#[cfg(feature = "wayland")]
pub mod wayland;

#[cfg(feature = "control")]
pub mod control;

#[cfg(feature = "test-support")]
pub mod test_support;

#[cfg(feature = "widgets")]
pub mod popup;

#[cfg(feature = "icons")]
pub mod icon;

/// The seven-segment test font, built in code: test support for scootui's
/// own pixel tests and for the apps' integration tests (which write it to a
/// file and pass it as `--font`).
pub mod testfont;
