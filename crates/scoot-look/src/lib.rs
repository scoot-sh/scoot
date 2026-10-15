//! `scoot-look`: the look format as a pure crate.
//!
//! A look is a directory with a `look.toml` of tokens; this crate parses,
//! validates (keys *and* values) and renders per-app fragments from it.
//! Pure by contract (design section 3.3): no Wayland, no fontconfig, no
//! image decoder, no process spawning. It builds on macOS beside
//! `scoot-core`, and nothing in it needs Nix.
//!
//! # Nix and non-Nix paths
//!
//! The same source tree serves both. A plain binary install (tagged-release
//! tarball, `install.sh`, a distro package) carries its catalog as
//! directories of plain files and renders them with the `scoot-look`
//! binary in this crate; a Nix install renders the same sources at build
//! time with the same binary. Listing, previewing, applying, switching and
//! authoring looks never require Nix: Nix is one way to render and install
//! the catalog, never the only way.
//!
//! Catalog order (both paths): user-looks, installed-looks, home-manager,
//! system, shipped. Origin is by directory ([`Origin`]); only the lookup of
//! those directories differs between a Nix and a non-Nix box, and that
//! lookup lives in the caller, not here.
//!
//! # Pipeline
//!
//! 1. [`parse_look_toml`](schema::parse_look_toml) parses one file.
//! 2. [`validate_for_origin`](schema::validate_for_origin) adds the
//!    origin-dependent rules; [`validate_apps_allowlist`](allowlist::validate_apps_allowlist)
//!    refuses non-allowlisted `[apps.<app>]` keys.
//! 3. [`resolve`] follows `extends` over source looks only.
//! 4. [`check`](contrast::check) measures contrast, refusing or warning by
//!    origin.
//! 5. [`render_scoot`](render::render_scoot) and
//!    [`render_bar`](render::render_bar) emit the first-party fragments from
//!    the resolved look plus [`RenderInputs`](render::RenderInputs).
//!
//! The effectful checks live in the caller: fonts through their carrier,
//! the wallpaper through `scootbg check-image`, the fragments through the
//! apps' own strict parsers.

pub mod allowlist;
pub mod contrast;
pub mod grammar;
pub mod render;
pub mod resolve;
pub mod resolved;
pub mod schema;

pub use allowlist::{allows, validate_apps_allowlist};
pub use contrast::{ContrastPair, ContrastReport, check as check_contrast};
pub use grammar::GrammarError;
pub use render::{RenderInputs, render_bar, render_json, render_scoot, source_hash};
pub use resolve::resolve;
pub use resolved::ResolvedLook;
pub use schema::{Origin, RawLook, parse_look_toml, validate_for_origin};
