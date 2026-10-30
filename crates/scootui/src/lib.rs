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
//! Features: `widgets` (the popup and the ellipsis cut), `entry` (the
//! text entry with its password mode, the virtualized list, the inline
//! selector and the focus chain), `icons` (path
//! icons and the cache), `png` (PNG image icons), `tray` (tray pixmaps),
//! `surfaces` (the on-demand layer and full-screen toplevel surfaces with
//! their buffer pool), `wayland` (the poll loop's Wayland primitives:
//! flush, the gone check, the read after a poll), `control` (the control
//! socket's server half: line framing, socket paths, the claim and the
//! connection bounds, with the socket name a parameter of the app's),
//! `keyboard` (the seat keyboard: keymap, modifiers, repeat, keysyms and
//! UTF-8 into a caller-supplied buffer) and `test-support` (the
//! headless-compositor harness the apps' integration tests share). The
//! default is `widgets` and `icons`; the smallest build has none of them
//! and draws nothing but plain canvas and text.
//!
//! No `unsafe` here, except in one place: `keyboard::raw`, the ~60 lines
//! that write libxkbcommon's UTF-8 into a caller-supplied buffer, carries
//! the crate's only `#[allow(unsafe_code)]` (see that module for the
//! invariants). The crate is therefore `#![deny(unsafe_code)]` rather than
//! `#![forbid(unsafe_code)]`: deny still forbids `unsafe` everywhere the
//! module-level `allow` does not name. A separate crate for those lines
//! was rejected: they are no API (two `extern "C"` declarations plus two
//! wrappers), and a new crate would need its own CI classify arms, its own
//! scope line and its own deny.toml review for what is one call site.

#![deny(unsafe_code)]

// The allocation pins (`count_allocations`) count nothing without this.
#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

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
pub mod motion;
pub mod output;
pub mod outputs;
pub mod paint;
pub mod region;
pub mod text;
pub mod theme;

#[cfg(feature = "wayland")]
pub mod wayland;

#[cfg(feature = "surfaces")]
pub mod surfaces;

#[cfg(feature = "control")]
pub mod control;

#[cfg(feature = "keyboard")]
pub mod keyboard;

#[cfg(feature = "test-support")]
pub mod test_support;

#[cfg(feature = "widgets")]
pub mod popup;

#[cfg(feature = "entry")]
pub mod entry;

#[cfg(feature = "icons")]
pub mod icon;

/// The seven-segment test font, built in code: test support for scootui's
/// own pixel tests and for the apps' integration tests (which write it to a
/// file and pass it as `--font`).
pub mod testfont;
