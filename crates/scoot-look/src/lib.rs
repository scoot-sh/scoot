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
//! 1. [`parse_look_toml`](schema::parse_look_toml) parses one file, refusing
//!    unknown keys, grammar violations, non-allowlisted `[apps.<app>]` keys
//!    and per-key value mismatches: there is no unvalidated path.
//! 2. [`validate_for_origin`](schema::validate_for_origin) adds the
//!    origin-dependent rules; [`validate_apps_allowlist`](allowlist::validate_apps_allowlist)
//!    re-checks the allowlist by name over already-parsed tables.
//! 3. [`resolve`] follows `extends` over source looks only.
//! 4. [`check`](contrast::check) measures contrast, refusing or warning by
//!    origin.
//! 5. [`render_scoot`](render::render_scoot) and
//!    [`render_bar`](render::render_bar) emit the first-party fragments from
//!    the resolved look plus [`RenderInputs`](render::RenderInputs).
//!
//! [`import_base16`](import::import_base16) maps a base16 or base24 scheme
//! to a source look's `look.toml` (design section 5.3): the plain-install
//! source of palettes, pure like the rest here.
//!
//! [`state`] is the per-session state (`<state>/look.json` plus the `look`
//! link): the state-dir rule scoot, scootbar and scootbg share, the record,
//! the link repair, the seed a private session copies at startup, and the
//! lock every writer takes.
//!
//! The effectful checks live in the caller: fonts through their carrier,
//! the wallpaper through `scootbg check-image`, the fragments through the
//! apps' own strict parsers.

pub mod allowlist;
pub mod contrast;
pub mod grammar;
pub mod import;
pub mod render;
pub mod resolve;
pub mod resolved;
pub mod schema;
pub mod state;

pub use allowlist::{
    ValueKind, allows, check_app_value, is_wallpaper_override_key, validate_apps_allowlist,
    value_kind,
};
pub use contrast::{ContrastPair, ContrastReport, check as check_contrast};
pub use grammar::GrammarError;
pub use import::{ImportOptions, import_base16};
pub use render::{
    BAR_RADIUS_LIMIT, RenderInputs, render_bar, render_json, render_scoot, source_hash,
};
pub use resolve::resolve;
pub use resolved::ResolvedLook;
pub use schema::{Origin, RawLook, check_name_matches_dir, parse_look_toml, validate_for_origin};
