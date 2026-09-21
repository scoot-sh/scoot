//! Loading `[layout]`/`[binds]`/`[autostart]` from a TOML config file into the types the
//! rest of the compositor already uses: [`scoot_core::Config`] and
//! [`Keybindings`].
//!
//! # Failure semantics (read this before changing any of them)
//!
//! - An explicit `--config PATH` that doesn't exist or can't be read is a
//!   hard startup error (see [`load`]) -- the caller pointed at this file on
//!   purpose, so silently ignoring it would be worse than failing loud.
//! - Every other failure -- no file at the default path, malformed TOML, an
//!   unknown field (`deny_unknown_fields`), a bad individual bind, input
//!   nested past the `toml` crate's own depth limits -- logs a
//!   `tracing::error!`/`tracing::warn!` and falls back to defaults. It never
//!   fails startup.
//!
//! That second rule is deliberate and easy to "fix" into a hard failure
//! without understanding the cost of doing so: on `--tty`, the real
//! deployment target, scoot *is* the session -- there is no other window
//! manager to fall back to and no easy remote access the way this project's
//! dev VM has over SSH. A compositor that refuses to start over a config
//! typo is a hard lockout on real hardware. Starting with defaults and
//! saying what's wrong in the log is strictly better than that, even though
//! it means a typo can go unnoticed until someone reads the log.
//!
//! One deliberate exception: `[tty] gpu`, when the file names a device,
//! behaves like `--gpu PATH` -- exactly that device, no fallback, and a
//! startup error when it cannot be driven. Falling back to the automatic
//! pick there would be fail-open (silently driving a device the user
//! explicitly ruled out), so the fail-closed refusal wins over the
//! never-block-startup rule for this one key. See [`LoadedConfig::gpu`]
//! and `tty::gpu::resolve`.
//!
//! `[renderer] backend` is a near-miss worth spelling out, because it splits
//! the two halves across the rule. An *unknown* name (`backend = "vulkan"`)
//! is an ordinary malformed value: warn, use the default, start. But a name
//! this build knows and then cannot build -- `"gles"` on a box with no
//! working EGL -- fails startup, from the config file exactly as from
//! `--renderer gles`, because silently compositing with the other renderer
//! would make every "verified under GLES" claim false while looking fine.
//! That is not the lockout the rule above exists to prevent: `--tty` never
//! reaches it (without the `gpu-scanout` feature it warns and keeps pixman
//! in `render::resolve`; with it the fallback happens in `tty::init`
//! instead), so the only
//! sessions that can fail this way are `--headless` and `--nested`, both of
//! which are started from a shell that is still there to read the error.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use scoot_core::{Action, Config};
use serde::Deserialize;
use smithay::input::keyboard::{Keysym, xkb};

use crate::cli::RendererKind;

use super::decorations::{Appearance, Color};
use super::input::keysym_named;
use super::keybindings::{Bound, Keybindings, Modifiers};
use super::output_scale::{MAX_SCALE, MIN_SCALE, clamp_scale};

/// `[layout]`. Mirrors `scoot_core::Config` field-for-field, each optional
/// so a partial table (e.g. just `gap`) leaves the rest at their defaults
/// rather than requiring every field to be repeated.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct LayoutConfig {
    gap: Option<i32>,
    column_widths: Option<Vec<f64>>,
    default_column_width: Option<usize>,
}

impl LayoutConfig {
    fn into_config(self) -> Config {
        let defaults = Config::default();
        Config {
            gap: self.gap.unwrap_or(defaults.gap),
            column_widths: self.column_widths.unwrap_or(defaults.column_widths),
            default_column_width: self
                .default_column_width
                .unwrap_or(defaults.default_column_width),
        }
        // Range/sanity clamping (negative gap, an out-of-range default
        // index, ...) is `World::new`'s job via `Config::validated`, not
        // this module's -- no need to duplicate it here.
    }
}

/// `[output]`. One field today: `scale`, the output scale scoot advertises
/// to clients and renders at (see `output_scale.rs`). `Option`-everything for
/// the same reason [`LayoutConfig`] is: a partial table leaves the rest at
/// their defaults.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    scale: Option<f64>,
}

impl OutputConfig {
    /// Resolves `scale` to a usable value, warning (never failing) when it
    /// had to change the configured one -- the same graceful-degradation rule
    /// every other config field follows (see this module's doc).
    ///
    /// TOML can spell `nan` and `inf`, and `NaN.clamp(..)` propagates NaN
    /// rather than clamping, so non-finite values are rejected explicitly
    /// before the range clamp. `Scale::fractional_scale()` feeding
    /// `physical / scale` (and `wl_output.scale`'s `ceil`) with NaN or
    /// infinity is a compositor that lays nothing out, so this is a
    /// correctness bound, not taste.
    fn into_scale(self) -> f64 {
        let Some(scale) = self.scale else {
            return 1.0;
        };
        if !scale.is_finite() {
            tracing::warn!(
                configured = scale,
                "output scale is not a finite number; using 1.0"
            );
            return 1.0;
        }
        let clamped = clamp_scale(scale);
        if clamped != scale {
            tracing::warn!(
                configured = scale,
                min = MIN_SCALE,
                max = MAX_SCALE,
                "output scale is out of range; clamping"
            );
        }
        clamped
    }
}

/// `[appearance]`. Mirrors [`Appearance`] field for field, the same
/// `Option`-everything pattern [`LayoutConfig`] uses -- except each color
/// field is a raw `"#rrggbb"`/`"#rrggbbaa"` string here, parsed one at a
/// time in [`AppearanceConfig::into_appearance`] rather than by `serde`
/// directly, so one malformed color string degrades gracefully instead of
/// invalidating the whole `[appearance]` table -- see that function's doc
/// and `apply_binds`'s identical philosophy for `[binds]`.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct AppearanceConfig {
    focus_ring_width: Option<i32>,
    focus_ring_active_color: Option<String>,
    focus_ring_inactive_color: Option<String>,
    background_color: Option<String>,
    corner_radius: Option<i32>,
    cursor_size: Option<i32>,
    cursor_color: Option<String>,
    cursor_theme: Option<String>,
    prefer_no_csd: Option<bool>,
}

impl AppearanceConfig {
    /// `gap` is the `[layout]` gap this same file resolved to, already put
    /// through [`Config::clamp_gap`] by the caller -- used to clamp
    /// `focus_ring_width` at load time (see [`Appearance::clamped`]). It has
    /// to be the clamped value, not the raw one: the ring is sized against
    /// half the gap, and `World::new` clamps the gap it actually lays out
    /// with, so measuring the ring against a larger raw number would let a
    /// ring through that is wider than half the gap the layout really uses.
    /// `Appearance::clamped` treats a negative gap as zero anyway, so the
    /// lower half of that clamp changes nothing here. That same call also
    /// bounds `cursor_size`, which has nothing to do with `gap` -- see
    /// [`Appearance::clamped`].
    fn into_appearance(self, gap: i32) -> Appearance {
        let defaults = Appearance::default();
        let color = |field: Option<String>, name: &'static str, default: Color| match field {
            None => default,
            Some(raw) => Color::parse(&raw).unwrap_or_else(|| {
                tracing::warn!(
                    field = name,
                    value = %raw,
                    "invalid color in config file; using the default for this field only"
                );
                default
            }),
        };
        Appearance {
            focus_ring_width: self.focus_ring_width.unwrap_or(defaults.focus_ring_width),
            focus_ring_active_color: color(
                self.focus_ring_active_color,
                "focus_ring_active_color",
                defaults.focus_ring_active_color,
            ),
            focus_ring_inactive_color: color(
                self.focus_ring_inactive_color,
                "focus_ring_inactive_color",
                defaults.focus_ring_inactive_color,
            ),
            background_color: color(
                self.background_color,
                "background_color",
                defaults.background_color,
            ),
            corner_radius: self.corner_radius.unwrap_or(defaults.corner_radius),
            cursor_size: self.cursor_size.unwrap_or(defaults.cursor_size),
            cursor_color: color(self.cursor_color, "cursor_color", defaults.cursor_color),
            // An empty string is treated as unset rather than as a theme
            // named "": it is what a user writing `cursor_theme = ""` to mean
            // "no override" would expect, and `Theme::load` would otherwise
            // search for a theme that cannot exist.
            cursor_theme: self.cursor_theme.filter(|name| !name.is_empty()),
            prefer_no_csd: self.prefer_no_csd.unwrap_or(defaults.prefer_no_csd),
        }
        .clamped(gap)
    }
}

/// `[tty]`. One field today: `gpu`, the DRM device `--tty` drives when
/// the automatic choice is wrong (see `tty::gpu`). `Option`-everything for
/// the same reason [`LayoutConfig`] is: a partial table leaves the rest at
/// their defaults.
///
/// An empty `gpu = ""` is *not* normalized to `None` here: silently
/// dropping what the user wrote would be fail-open (driving the automatic
/// pick while the config names a device). It parses to `Some("")` and
/// `tty::gpu::resolve` refuses it with a hard startup error naming the key.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct TtyConfig {
    gpu: Option<PathBuf>,
}

/// `[renderer]`. One field today: `backend`, which renderer composites each
/// frame -- `"pixman"` (the default, and the only one that needs no graphics
/// device) or `"gles"`. Named `backend` for the renderer *behind* the
/// compositor, which is a different axis from `--headless`/`--nested`/`--tty`
/// (how the compositor presents what it drew); `docs/tty.md` says so in as
/// many words.
///
/// A `String` rather than a `RendererKind` so that an unrecognised name
/// degrades the way every other malformed value in this file does -- a
/// warning naming the key, and the default for that field only (see
/// [`RendererConfig::into_kind`]) -- instead of `serde` rejecting the whole
/// file over one typo. `--renderer` refuses the same typo outright, because
/// a flag is a thing the user just typed and can retype.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RendererConfig {
    backend: Option<String>,
}

impl RendererConfig {
    /// `None` when the file says nothing (or says something unusable), which
    /// is what lets `--renderer` and the built-in default share one
    /// resolution rule -- see `render::resolve`.
    fn into_kind(self) -> Option<RendererKind> {
        let name = self.backend?;
        let kind = RendererKind::parse(&name);
        if kind.is_none() {
            tracing::warn!(
                value = %name,
                "unknown [renderer] backend in config file; expected `pixman` or `gles`, \
                 using the default"
            );
        }
        kind
    }
}

/// `[autostart]`. One field: `commands`, a flat list of action strings in
/// exactly the grammar `scootctl action ...` (and a config file's `[binds]`
/// values) use -- see `scootctl::action`, reused here rather than duplicated.
/// No ordering, no conditionals, no supervision: entries run once each, in
/// file order, before the `--` command (see `compositor::run`), and anything
/// fancier belongs in the session script. Parsed lazily one at a time (see
/// [`AutostartConfig::into_actions`]), so one bad entry can't take the rest
/// down with it -- the same isolation philosophy [`apply_binds`] has for
/// `[binds]`.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct AutostartConfig {
    #[serde(default)]
    commands: Vec<String>,
}

impl AutostartConfig {
    /// Parses every entry through the shared action grammar, keeping the ones
    /// that check out, in file order.
    ///
    /// A malformed entry (an unknown action, a missing argument, trailing
    /// text after the action) is skipped with a `tracing::warn!` naming just
    /// that entry; every other entry still runs. Fail-open, the same rule
    /// `[binds]` follows: on `--tty` scoot *is* the session, so a typo must
    /// never cost the session -- that refusal-to-start shape is the
    /// user-facing harm this degrades away from, and it is pinned by test
    /// (see `an_invalid_autostart_entry_is_skipped_and_the_session_still_starts`).
    fn into_actions(self) -> Vec<Action> {
        let mut actions = Vec::new();
        for command in &self.commands {
            match parse_autostart(command) {
                Ok(action) => actions.push(action),
                Err(reason) => {
                    tracing::warn!(
                        command = %command, %reason,
                        "skipping an invalid [autostart] entry"
                    );
                }
            }
        }
        actions
    }
}

/// The whole file. `binds`' values and `autostart`'s entries are parsed
/// lazily, one at a time (see [`apply_binds`] and
/// [`AutostartConfig::into_actions`]), so one bad bind or entry can't take
/// the rest down with it.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    layout: Option<LayoutConfig>,
    #[serde(default)]
    appearance: Option<AppearanceConfig>,
    #[serde(default)]
    output: Option<OutputConfig>,
    #[serde(default)]
    renderer: Option<RendererConfig>,
    #[serde(default)]
    tty: Option<TtyConfig>,
    #[serde(default)]
    autostart: Option<AutostartConfig>,
    #[serde(default)]
    binds: HashMap<String, String>,
}

/// What loading a config file (or using defaults) produces: always a
/// complete, usable triple, never a partial state -- see the module doc.
#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub keybindings: Keybindings,
    pub appearance: Appearance,
    /// The resolved `[output] scale`, already clamped (or the default 1.0).
    pub scale: f64,
    /// The `[tty] gpu` device path, when the file names one. `None` (the
    /// normal case) means the automatic search picks; `Some` means drive
    /// exactly that device, the way `--gpu PATH` does -- including its
    /// fail-closed refusal when the device cannot be driven. `--gpu` wins
    /// over this when both name one (see `tty::gpu::resolve`), and an
    /// explicitly-set-but-empty path in either is a hard startup error,
    /// not a silent fallback. Meaningless outside `--tty`, where
    /// `compositor::run` ignores it with a warning, for the same reason as
    /// `--gpu` itself.
    pub gpu: Option<PathBuf>,
    /// The `[renderer] backend` the file asks for, when it names a renderer
    /// this build knows. `None` -- no `[renderer]` table, no `backend` key,
    /// or a name that is neither `pixman` nor `gles` (warned about at load) --
    /// leaves the choice to `--renderer`, and to the pixman default when that
    /// is unset too. `--renderer` wins over this when both name one; see
    /// `render::resolve`, which is also where `--tty` overrides both.
    pub renderer: Option<RendererKind>,
    /// The `[autostart] commands` that parsed, in file order. Entries that
    /// did not parse were already warned about and dropped at load (see
    /// [`AutostartConfig::into_actions`]); an empty `Vec` -- no table, no
    /// `commands` key, or nothing usable in it -- means nothing runs before
    /// the `--` command. `compositor::run` drains these through `State::act`
    /// before spawning `--`, so the full action grammar applies (a non-`spawn`
    /// action at startup is the user's choice, documented as such).
    pub autostart: Vec<Action>,
}

impl LoadedConfig {
    fn defaults() -> Self {
        Self {
            config: Config::default(),
            keybindings: Keybindings::default(),
            appearance: Appearance::default(),
            scale: 1.0,
            gpu: None,
            renderer: None,
            autostart: Vec::new(),
        }
    }

    fn from_file(file: FileConfig) -> Self {
        Self::from_file_with_vt(file, false)
    }

    /// Like [`from_file`](Self::from_file), but for a live reload rather
    /// than startup: `vt` says whether this session drives `--tty`, in
    /// which case the replacement table gets the `Ctrl+Alt+F1..F12`
    /// recovery bindings layered on last -- exactly what `tty::init` does
    /// to the startup table (see [`enforce_vt_binds`]). Startup itself
    /// always passes `false` here: the VT bindings are added later, once
    /// `--tty` is known to be the backend.
    fn from_file_with_vt(file: FileConfig, vt: bool) -> Self {
        let config = file.layout.unwrap_or_default().into_config();
        let appearance = file
            .appearance
            .unwrap_or_default()
            .into_appearance(Config::clamp_gap(config.gap));
        let scale = file.output.unwrap_or_default().into_scale();
        let gpu = file.tty.and_then(|tty| tty.gpu);
        let renderer = file.renderer.unwrap_or_default().into_kind();
        let autostart = file.autostart.unwrap_or_default().into_actions();
        let keybindings = keybindings_for(&file.binds, vt);
        Self {
            config,
            keybindings,
            appearance,
            scale,
            gpu,
            renderer,
            autostart,
        }
    }
}

/// The config file a session started from: the explicit `--config PATH`
/// when one was given, else the resolved XDG default path -- whether or
/// not a file existed there at startup. What `Request::Reload` re-reads
/// (see `reload.rs`); `None` only when no path resolves at all (neither
/// `XDG_CONFIG_HOME` nor `HOME` set), in which case a reload answers an
/// error rather than guessing.
///
/// Pure over its two env vars like [`default_path`], for the same testability
/// reason -- `run` passes the live ones.
pub fn startup_path(
    explicit: Option<&Path>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    explicit
        .map(Path::to_owned)
        .or_else(|| default_path(xdg_config_home, home))
}

/// Re-reads `path` the way startup would, but without any of startup's
/// graceful degradation: this runs against a live session, where falling
/// back to defaults over a typo would silently revert what the user has.
///
/// - The file must read and the whole TOML must validate (`deny_unknown_fields`
///   included): anything less keeps the running config untouched and comes
///   back as an `Err` naming the path and the reason -- never defaults,
///   never a partial load, never an exit. `LoadedConfig::from_file`'s own
///   per-field fallbacks (one bad color, one bad bind) still apply *within*
///   a file that validates, the same isolation startup has.
/// - `vt` is whether this session drives `--tty` (see
///   [`LoadedConfig::from_file_with_vt`](LoadedConfig::from_file_with_vt)).
pub fn reload_from(path: &Path, vt: bool) -> Result<LoadedConfig, ReloadError> {
    let text = fs::read_to_string(path).map_err(|source| ReloadError {
        path: path.to_owned(),
        reason: source.to_string(),
    })?;
    toml::from_str::<FileConfig>(&text)
        .map(|file| LoadedConfig::from_file_with_vt(file, vt))
        .map_err(|source| ReloadError {
            path: path.to_owned(),
            reason: source.to_string(),
        })
}

/// A reload that could not load or validate the file. The running config is
/// untouched; the message is what the `reload` reply (and the log) carries.
#[derive(Debug)]
pub struct ReloadError {
    path: PathBuf,
    reason: String,
}

impl fmt::Display for ReloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "could not reload config file `{}`: {}; keeping the running config",
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for ReloadError {}

/// Builds the keybinding table a config file's `[binds]` describes: the
/// defaults, the file's binds layered on via [`apply_binds`], and -- when
/// `vt` -- the `--tty` `Ctrl+Alt+F1..F12` recovery bindings on top of those
/// (see [`enforce_vt_binds`]). Startup builds with `false` (the VT bindings
/// land later, in `tty::init`); a reload builds with whether this session
/// drives `--tty`, so a reload can neither strip the recovery path nor gain
/// it on a backend that never had it.
pub fn keybindings_for(binds: &HashMap<String, String>, vt: bool) -> Keybindings {
    let mut table = Keybindings::default();
    apply_binds(&mut table, binds);
    if vt {
        enforce_vt_binds(&mut table);
    }
    table
}

/// Layers the `--tty`-only `Ctrl+Alt+F1..F12` VT-switch bindings over
/// `table`, overriding any colliding bind with a warning.
///
/// One function rather than two call sites (`tty::init` at startup,
/// [`keybindings_for`] on reload) so the two cannot disagree about what
/// "the recovery path always wins" means: a config-file bind landing on the
/// same combo as a VT switch -- a typo or a well-meaning-but-dangerous
/// rebind -- would otherwise silently shadow the one recovery path this
/// project has on real hardware (see this module's doc).
pub fn enforce_vt_binds(table: &mut Keybindings) {
    for (mods, keysym, replaced) in table.extend(Keybindings::vt_switch_bindings()) {
        tracing::warn!(
            ?mods,
            keysym = keysym.raw(),
            ?replaced,
            "a config-file keybinding on this combo was overridden by --tty's \
             VT-switch binding, which must always work as the recovery path"
        );
    }
}

// -- `--print-default-config` -----------------------------------------------

/// Emits a starting config file generated from the compositor's own live
/// defaults: [`Config::default`], [`Appearance::default`],
/// [`Keybindings::default`], scale 1.0, no `[tty] gpu`, no `[renderer]`
/// backend, and no `[autostart]` entries.
///
/// Every key is present and commented out with its default as the value, so
/// the emitted file as-is *is* the defaults -- uncomment a line to set it
/// explicitly. That shape is also what makes the anti-drift test possible:
/// the output parses back (through [`FileConfig`], the same type startup
/// reads) to the defaults it was generated from, so a changed default
/// changes the emitted file automatically instead of going stale like a
/// hand-maintained string would.
///
/// Three appearance colors are the nearest `"#rrggbb"` to built-ins none of
/// whose floats is exactly representable in 8 bits (see
/// `docs/configuration.md`) -- and "nearest" is renderer-dependent by 1 LSB
/// (pixman truncates where the emitter rounds; GLES agrees with the
/// emitter), so no emitted hex is pixel-exact everywhere. Leave one
/// commented for the real default. The conversion is [`hex`]'s, and the
/// test below pins the fixed point: parsing the emitted hex and re-emitting
/// it is byte-stable.
///
/// Determinism is structural, not sorted-at-the-end: `[binds]` iterates the
/// default table in its hardcoded order (see [`Keybindings::iter`]), and
/// every other section is a scalar in a fixed position -- so two emissions
/// are byte-identical, with no timestamp and no `HashMap` order in play.
/// (The file *loader* reads `[binds]` into a `HashMap`, which is why it
/// refuses to arbitrate collisions; the emitter never touches that map.)
pub fn default_config_toml() -> String {
    let config = Config::default();
    let appearance = Appearance::default();
    let keybindings = Keybindings::default();

    let mut out = String::new();
    out.push_str(
        "# A starting scoot config, generated from the compositor's own built-in\n\
         # defaults (`scoot --print-default-config`). Every key is present and\n\
         # commented out with its default as the value, so this file as-is is\n\
         # exactly the defaults: uncomment a line to set it explicitly, and save\n\
         # it as ~/.config/scoot/config.toml (or pass it with --config PATH).\n\
         #\n\
         # The ring/background colors are the nearest \"#rrggbb\" to built-ins\n\
         # none of whose floats is exactly representable in 8 bits -- leave one\n\
         # commented for the real default (see docs/configuration.md, which these\n\
         # comments summarize, not replace).\n\
         #\n\
         # Gap, the ring/background appearance fields, and binds re-apply live\n\
         # with `scootctl reload`; cursor settings, column widths, and everything\n\
         # else are startup-only and a reload refuses them with a message.\n",
    );

    out.push_str("\n[layout]\n");
    out.push_str(
        "# Gap between columns, between windows stacked in a column, and at output edges.\n",
    );
    out.push_str(&format!("# gap = {}\n", config.gap));
    out.push_str("# Column widths as fractions of the output width, in cycle order.\n");
    let widths: Vec<String> = config
        .column_widths
        .iter()
        .map(|w| format!("{w:?}"))
        .collect();
    out.push_str(&format!("# column_widths = [{}]\n", widths.join(", ")));
    out.push_str("# Index into column_widths for newly created columns.\n");
    out.push_str(&format!(
        "# default_column_width = {}\n",
        config.default_column_width
    ));

    out.push_str("\n[appearance]\n");
    out.push_str("# Ring thickness; clamped to at most half of gap.\n");
    out.push_str(&format!(
        "# focus_ring_width = {}\n",
        appearance.focus_ring_width
    ));
    out.push_str(&format!(
        "# focus_ring_active_color = \"{}\"\n",
        hex(appearance.focus_ring_active_color)
    ));
    out.push_str(&format!(
        "# focus_ring_inactive_color = \"{}\"\n",
        hex(appearance.focus_ring_inactive_color)
    ));
    out.push_str(&format!(
        "# background_color = \"{}\"\n",
        hex(appearance.background_color)
    ));
    out.push_str("# Window corner radius in logical pixels; 0 is square. The effective\n");
    out.push_str("# radius is clamped per window to half its smaller dimension.\n");
    out.push_str(&format!("# corner_radius = {}\n", appearance.corner_radius));
    out.push_str(&format!("# cursor_size = {}\n", appearance.cursor_size));
    out.push_str(&format!(
        "# cursor_color = \"{}\"\n",
        hex(appearance.cursor_color)
    ));
    out.push_str(
        "# Unset follows $XCURSOR_THEME, then \"default\"; name one here only to override that.\n",
    );
    out.push_str("# cursor_theme = \"Adwaita\"\n");
    out.push_str(&format!("# prefer_no_csd = {}\n", appearance.prefer_no_csd));

    out.push_str("\n[output]\n");
    out.push_str("# Output scale advertised to clients and rendered at.\n");
    out.push_str("# scale = 1.0\n");

    out.push_str("\n[renderer]\n");
    out.push_str(
        "# Which renderer composites each frame. --renderer wins over this when both name one.\n",
    );
    out.push_str(&format!(
        "# backend = \"{}\"\n",
        RendererKind::default().as_str()
    ));

    out.push_str("\n[tty]\n");
    out.push_str(
        "# Unset means the automatic search picks; --gpu PATH wins over this when both name one.\n\
         # Name the display controller (prefer a stable /dev/dri/by-path/... alias):\n",
    );
    out.push_str("# gpu = \"/dev/dri/card0\"\n");

    out.push_str("\n[autostart]\n");
    out.push_str("# Action strings to run once each, in file order, at session startup.\n");
    out.push_str("# commands = []\n");

    out.push_str("\n[binds]\n");
    for (mods, keysym, bound) in keybindings.iter() {
        match bound {
            Bound::Action(action) => {
                out.push_str(&format!(
                    "# \"{}\" = \"{}\"\n",
                    combo_string(mods, keysym),
                    action_string(action)
                ));
            }
            // No config spelling exists for these: they are layered onto the
            // table by the session itself (`enforce_vt_binds`), not read out
            // of it. The defaults hold none today, so this arm is future
            // proofing, not dead code -- and a comment keeps the emission
            // total rather than silently dropping a binding.
            Bound::ChangeVt(vt) => {
                out.push_str(&format!(
                    "# (plus a session-managed VT-switch binding for VT {vt}, \
                     which has no config spelling)\n"
                ));
            }
        }
    }
    out.push_str(
        "# Under --tty, Ctrl+Alt+F1..F12 VT-switch bindings are layered on last and\n\
         # always win over a colliding bind here; they have no config spelling.\n",
    );
    out
}

/// This color as `"#rrggbb"` (opaque) or `"#rrggbbaa"`, the nearest 8-bit
/// value per channel.
///
/// Nearest, not exact: none of the built-in floats is exactly representable
/// in 8 bits, and the two renderers disagree by 1 LSB on backgrounds anyway
/// (pixman's float-to-16-bit conversion truncates where `.round()` rounds,
/// so a pixman session renders the running default 1 LSB below this hex;
/// GLES agrees with it). No hex string is pixel-exact on both renderers --
/// leave the line commented for the real default. What `.round()` does buy
/// is the fixed point: parsing the emitted hex and re-emitting it is
/// byte-stable (see the round-trip test), so an uncommented line never
/// drifts a second time.
fn hex(color: Color) -> String {
    let channel = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let (r, g, b, a) = (
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    );
    if a == u8::MAX {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// One `[binds]` combo back in the string form the file loader parses
/// (`parse_combo`): modifiers in a fixed order, then the keysym's own name.
///
/// The name comes from xkb itself (`keysym_get_name`), not a table kept
/// beside the defaults -- the same lookup `keysym_named` resolves through,
/// so whatever it spells is spellable back. The round-trip test pins that
/// for every default bind.
fn combo_string(mods: Modifiers, keysym: Keysym) -> String {
    let mut combo = String::new();
    if mods.super_ {
        combo.push_str("super+");
    }
    if mods.shift {
        combo.push_str("shift+");
    }
    if mods.ctrl {
        combo.push_str("ctrl+");
    }
    if mods.alt {
        combo.push_str("alt+");
    }
    combo.push_str(&xkb::keysym_get_name(keysym));
    combo
}

/// One action back in the string form the file loader parses
/// (`scootctl::action`): the same grammar `scootctl action ...` and a
/// `[binds]` value use, so whatever this spells loads back to the same
/// [`Action`]. Total over every variant -- including ones the defaults hold
/// none of -- so a future default needs no second change here.
fn action_string(action: &Action) -> String {
    use scoot_core::{Horizontal, Vertical};
    let direction = |h: &Horizontal| match h {
        Horizontal::Left => "left",
        Horizontal::Right => "right",
    };
    let vertical = |v: &Vertical| match v {
        Vertical::Up => "up",
        Vertical::Down => "down",
    };
    match action {
        Action::FocusColumn(d) => format!("focus-column {}", direction(d)),
        Action::MoveColumn(d) => format!("move-column {}", direction(d)),
        Action::ConsumeOrExpel(d) => format!("consume-or-expel {}", direction(d)),
        Action::FocusWindow(d) => format!("focus-window {}", vertical(d)),
        Action::MoveWindow(d) => format!("move-window {}", vertical(d)),
        Action::FocusWindowId(id) => format!("focus-window-id {}", id.0),
        Action::FocusWorkspace(d) => format!("focus-workspace {}", vertical(d)),
        Action::FocusWorkspaceIndex(index) => format!("focus-workspace-index {index}"),
        Action::MoveWindowToWorkspace(d) => {
            format!("move-window-to-workspace {}", vertical(d))
        }
        Action::MoveWindowToWorkspaceIndex(index) => {
            format!("move-window-to-workspace-index {index}")
        }
        Action::CycleColumnWidth => "cycle-column-width".to_owned(),
        Action::CloseFocused => "close".to_owned(),
        Action::Spawn(command) => format!("spawn {}", command.join(" ")),
        Action::Quit => "quit".to_owned(),
    }
}

/// An explicit `--config PATH` that doesn't exist or can't be read. The one
/// config-related failure that stops startup -- see the module doc.
#[derive(Debug)]
pub struct ConfigFileError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "could not read config file `{}`: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ConfigFileError {}

/// Resolves the config path and loads it. `explicit` is `--config PATH`,
/// when given. See the module doc for the three-way fallback behavior this
/// implements.
pub fn load(explicit: Option<&Path>) -> Result<LoadedConfig, ConfigFileError> {
    match explicit {
        Some(path) => load_from(path, true),
        None => {
            match default_path(
                std::env::var_os("XDG_CONFIG_HOME"),
                std::env::var_os("HOME"),
            ) {
                Some(path) => load_from(&path, false),
                // Neither XDG_CONFIG_HOME nor HOME is set: nowhere sensible
                // to look, same as "the default file doesn't exist".
                None => Ok(LoadedConfig::defaults()),
            }
        }
    }
}

/// `$XDG_CONFIG_HOME/scoot/config.toml`, else `~/.config/scoot/config.toml`.
/// A pure function of the two env vars it needs, like
/// `scoot_ipc::socket::resolve`, so this is testable without touching real
/// environment state.
///
/// Shared by both readers and the one writer: [`load`] (and
/// [`startup_path`]) resolve the file to read through this, and
/// [`write_default_config`] resolves the file to create through this -- so
/// `--write` can never disagree with startup about where the default config
/// lives.
pub fn default_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |v: &OsString| !v.is_empty();
    xdg_config_home
        .filter(non_empty)
        .map(|dir| PathBuf::from(dir).join("scoot/config.toml"))
        .or_else(|| {
            home.filter(non_empty)
                .map(|dir| PathBuf::from(dir).join(".config/scoot/config.toml"))
        })
}

// -- `--print-default-config --write` ----------------------------------------

/// What `scoot --print-default-config --write` can fail with. Every variant
/// is a loud refusal -- a non-zero exit naming the path -- never a silent
/// fallback and never a clobber.
#[derive(Debug)]
pub enum WriteDefaultConfigError {
    /// No default location resolves at all (neither `XDG_CONFIG_HOME` nor
    /// `HOME` is set and non-empty): nowhere sensible to write, and writing
    /// to the current directory instead would strand a config where neither
    /// the user nor the loader looks for it.
    NoBaseDir,
    /// The parent directory is missing and could not be created.
    Mkdir { path: PathBuf, source: io::Error },
    /// Something already exists at the default location, so there is nothing
    /// to do. This includes a symlink: `create_new` (`O_CREAT|O_EXCL`)
    /// refuses the link itself without following it, so neither a live
    /// target nor a dangling link is ever clobbered through this path.
    Exists { path: PathBuf },
    /// The file was created but its bytes could not be written (a
    /// permissions failure past create, a full disk, ...). A mid-write
    /// failure removes the partial file best-effort (see
    /// [`finish_new_file_write`]), so this never leaves a torn config
    /// behind to confuse the next startup.
    Write { path: PathBuf, source: io::Error },
}

impl fmt::Display for WriteDefaultConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseDir => write!(
                f,
                "could not resolve the default config location: \
                 neither XDG_CONFIG_HOME nor HOME is set"
            ),
            Self::Mkdir { path, source } => write!(
                f,
                "could not create the config directory `{}`: {}",
                path.display(),
                source
            ),
            Self::Exists { path } => write!(
                f,
                "refusing to overwrite the existing config file `{}`",
                path.display()
            ),
            Self::Write { path, source } => write!(
                f,
                "could not write the default config to `{}`: {}",
                path.display(),
                source
            ),
        }
    }
}

impl std::error::Error for WriteDefaultConfigError {}

/// Writes the [`default_config_toml`] emission to the default config
/// location and returns that path, creating the parent directory when it is
/// missing. The content is the same `String` stdout would have carried --
/// one function, not a second formatter -- so the anti-drift pins cover
/// both.
///
/// The create is `O_CREAT|O_EXCL` (`create_new`), not check-then-truncate:
/// two concurrent invocations leave exactly one winner and one [`Exists`]
/// refusal, never a torn file. The new file is mode `0o600` at creation --
/// the config may one day hold sensitive values, matching the socket's
/// posture -- and `0o600` carries no group/other bits for any umask to
/// strip, so it stays private under every umask.
///
/// Resolution is [`default_path`] with the same two live env vars [`load`]
/// reads: `--write` and startup cannot disagree about where the default
/// config lives.
pub fn write_default_config() -> Result<PathBuf, WriteDefaultConfigError> {
    let Some(path) = default_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    ) else {
        return Err(WriteDefaultConfigError::NoBaseDir);
    };
    write_default_config_to(&path)?;
    Ok(path)
}

/// The env-free core of [`write_default_config`], over an explicit path so
/// tests can exercise it without touching process environment state.
fn write_default_config_to(path: &Path) -> Result<(), WriteDefaultConfigError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| WriteDefaultConfigError::Mkdir {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = default_config_toml();
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                WriteDefaultConfigError::Exists {
                    path: path.to_owned(),
                }
            } else {
                WriteDefaultConfigError::Write {
                    path: path.to_owned(),
                    source,
                }
            }
        })?;
    finish_new_file_write(file, path, text.as_bytes())
}

/// Completes a create-new write. A mid-write I/O error (disk full, quota,
/// ...) removes the partial file best-effort first: a torn config at the
/// default location would read back as malformed on the next startup, which
/// is worse than no file at all. The unlink is best-effort deliberately --
/// on a full disk there may be nothing left to unlink with -- and its
/// outcome never shadows the write error, which is what the refusal
/// reports.
fn finish_new_file_write(
    mut file: fs::File,
    path: &Path,
    bytes: &[u8],
) -> Result<(), WriteDefaultConfigError> {
    use std::io::Write as _;
    if let Err(source) = file.write_all(bytes) {
        let _ = fs::remove_file(path);
        return Err(WriteDefaultConfigError::Write {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

/// Loads `path`. `explicit` says whether this came from `--config`, which is
/// what decides whether a missing/unreadable file is a hard error or silent
/// defaults -- see the module doc.
fn load_from(path: &Path, explicit: bool) -> Result<LoadedConfig, ConfigFileError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if explicit => {
            return Err(ConfigFileError {
                path: path.to_owned(),
                source,
            });
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            // No config yet, or a fresh install: not a mistake, nothing to
            // warn about.
            return Ok(LoadedConfig::defaults());
        }
        Err(source) => {
            // Not `NotFound` (that's the arm above, including a broken
            // symlink -- it resolves to ENOENT too) -- something like a
            // permissions error, where the path exists but couldn't be
            // read. Same "never block startup" rule as a parse failure
            // below.
            tracing::error!(
                path = %path.display(), %source,
                "could not read the default config file; using defaults"
            );
            return Ok(LoadedConfig::defaults());
        }
    };
    Ok(parse_or_defaults(&text, path))
}

/// Parses `text`; a parse failure (malformed TOML, or a `deny_unknown_fields`
/// rejection) logs why and falls back to full defaults instead of failing
/// startup -- see the module doc.
///
/// # Why deeply nested input can't abort the process here
///
/// `toml` 1.1.6 bounds nesting itself, in two places, both hard-coded at 80
/// and both active unless the crate's `unbounded` feature is on (it is not;
/// `unbounded` is a zero-dependency flag so `-e features` can't see it either
/// way -- `cargo tree -p scoot --target all -f "{p} | {f}"` is the command
/// that actually shows it, and prints `toml v1.1.6+spec-1.1.0 | default,
/// display,parse,serde,std` with no `unbounded` suffix): a `RecursionGuard`
/// over combined
/// inline-table/array nesting, and a separate cap on dotted-key and table-header
/// path segments. Past either, parsing stops and reports an error, which lands
/// in the arm below like any other malformed config -- so at the stack budget
/// this actually runs with, no config file reaches the "recurse until the stack
/// is gone" failure this module's no-hard-failure promise could not otherwise
/// survive.
///
/// That budget is an assumption, not a guarantee, and so are two other things
/// about *how* this is called; all three are invisible at the site that would
/// break them (measured on aarch64 Linux + macOS, `toml` 1.1.6, 2026-09-13 --
/// see `docs/backlog/resolved/config-recursion-depth-resolved.md` for the raw
/// numbers and the commands that produced them):
///
/// - **The 8 MiB main-thread stack.** [`load`] runs on the process's main thread
///   via `compositor::run`, so it gets `RLIMIT_STACK`, which is 8 MiB by default
///   on Linux and macOS. Two ways to lose that: moving config parsing to a
///   spawned thread, which gets Rust's 2 MiB default instead, or launching
///   scoot under a reduced `ulimit -s`. A *debug* build has no margin for
///   either -- under `ulimit -s 2048` it aborts with `fatal runtime error: stack
///   overflow` on a 13 KB file a user could paste by accident. A release build
///   needs 932 KiB and survives both.
/// - **How deep a file can get.** The two limits multiply rather than add: a
///   dotted key can sit at every level of nesting, so the deepest tree they
///   allow is 6,561 nested tables/arrays (counting the document root) out of a
///   13,448-byte file. *Parsing* it is cheap -- 476 KiB of stack in a debug
///   build, and in a release build under the ~134 KiB floor glibc puts beneath
///   any thread stack here, which is as precise as that one gets. The recursive
///   *drop* of the parsed tree is the real cost: 932 KiB release (scoot's
///   `panic = "abort"` profile; ~1,140 KiB if built to unwind, which is what
///   `cargo test --release` produces) and 6,680 KiB debug.
/// - **The deserialization target.** The cost is a function of the target, not
///   just the input: `FileConfig` is shallow and `deny_unknown_fields` stops
///   serde at the first key, but deserializing the same bytes into a
///   `toml::Table` (a passthrough section, say) descends the whole tree --
///   7,288 KiB release, which still fits in 8 MiB but with under 1 MiB to spare
///   rather than over 7, and 32,100 KiB debug, which does not fit at all.
fn parse_or_defaults(text: &str, path: &Path) -> LoadedConfig {
    match toml::from_str::<FileConfig>(text) {
        Ok(file) => LoadedConfig::from_file(file),
        Err(error) => {
            tracing::error!(
                path = %path.display(), %error,
                "could not parse config file; using defaults"
            );
            LoadedConfig::defaults()
        }
    }
}

/// Parses every `[binds]` entry and applies the ones that check out, on top
/// of whatever `keybindings` already holds (the defaults).
///
/// Two isolation rules, both in service of the module doc's "a config
/// mistake degrades gracefully" principle:
///
/// - A bind that fails to parse (bad combo syntax, an unknown key name, a
///   bad action string, trailing text after the action) is skipped with a
///   `tracing::warn!` naming just that bind; every other bind in the file
///   still loads.
/// - Two *different* combo strings that resolve to the same (modifiers,
///   key) once parsed -- aliases (`super`/`logo`/`meta`/`cmd`), modifier
///   order (`super+shift+h` vs `shift+super+h`), or case (`Super+H` vs
///   `super+h`, see `parse_combo`) -- can't be given a well-defined winner:
///   `binds` is a `HashMap`, and its iteration order has no relationship to
///   the order the keys were written in the TOML source, so "the last one
///   in the file wins" isn't something this code can honor truthfully.
///   Rather than pick a winner that would silently vary from run to run,
///   every bind in the colliding group is skipped, with one warning naming
///   all of them; whatever was bound to that combo before this file was
///   loaded (a default, or nothing) is left alone.
fn apply_binds(keybindings: &mut Keybindings, binds: &HashMap<String, String>) {
    let mut parsed: Vec<(String, Modifiers, Keysym, Bound)> = Vec::new();
    for (raw, value) in binds {
        match parse_bind(raw, value) {
            Ok((mods, keysym, bound)) => parsed.push((raw.clone(), mods, keysym, bound)),
            Err(reason) => {
                tracing::warn!(
                    bind = %raw, value = %value, %reason,
                    "skipping an invalid config-file bind"
                );
            }
        }
    }

    // Group by the resolved combo (keyed on the keysym's raw code rather
    // than the `Keysym` itself, which doesn't implement `Hash`) so
    // collisions are found regardless of `HashMap`'s iteration order.
    let mut groups: HashMap<(Modifiers, u32), Vec<(String, Bound)>> = HashMap::new();
    for (raw, mods, keysym, bound) in parsed {
        groups
            .entry((mods, keysym.raw()))
            .or_default()
            .push((raw, bound));
    }

    for ((mods, keysym_raw), mut group) in groups {
        if group.len() > 1 {
            let mut names: Vec<&str> = group.iter().map(|(raw, _)| raw.as_str()).collect();
            names.sort_unstable();
            tracing::warn!(
                binds = ?names,
                "these config-file binds all resolve to the same key combination; \
                 none of them will be applied -- remove all but one"
            );
            continue;
        }
        let (_, bound) = group.pop().expect("group.len() == 1");
        let keysym = Keysym::from(keysym_raw);
        keybindings.insert(mods, keysym, bound);
    }
}

/// Parses one `[binds]` entry: `key` is the TOML key (`"super+h"`), `value`
/// is an action string in exactly the grammar `scootctl action ...` (and its
/// `scoot msg action ...` alias) uses (`"focus-column left"`, `"close"`,
/// `"spawn" "foot"`, ...) -- see `scootctl::action`, reused here rather than
/// duplicated.
fn parse_bind(key: &str, value: &str) -> Result<(Modifiers, Keysym, Bound), String> {
    let (mods, keysym) = parse_combo(key)?;
    let mut tokens = value.split_whitespace().map(str::to_owned);
    let action = scootctl::action(&mut tokens).map_err(|error| error.to_string())?;
    if tokens.next().is_some() {
        return Err(format!("trailing text after the action in `{value}`"));
    }
    Ok((mods, keysym, Bound::Action(action.into())))
}

/// Parses one `[autostart]` entry: `value` is an action string in exactly the
/// grammar `scootctl action ...` (and a `[binds]` value) uses -- see
/// `scootctl::action`, reused here rather than duplicated. No spawn-only
/// restriction: a non-`spawn` action at startup (say, `focus-workspace-index
/// 2`) is the user's choice, documented as such where `[autostart]` is
/// documented.
fn parse_autostart(value: &str) -> Result<Action, String> {
    let mut tokens = value.split_whitespace().map(str::to_owned);
    let action = scootctl::action(&mut tokens).map_err(|error| error.to_string())?;
    if tokens.next().is_some() {
        return Err(format!("trailing text after the action in `{value}`"));
    }
    Ok(action.into())
}

/// Parses a combo string (`"super+shift+t"`) into this table's `Modifiers`
/// plus a `Keysym`. Reuses `scoot_ipc::KeyCombo`'s modifier/key-name
/// splitting -- already parsed the same way for `scoot msg key ...` and
/// well-tested there -- rather than writing a second parser for the same
/// `mod+mod+key` syntax.
///
/// A single ASCII letter is folded to lowercase before resolving it. xkb
/// gives single Latin letters two *different* valid keysyms depending on
/// case (`h` and `H` are both real, distinct keysyms), but
/// `keybindings.rs`'s whole table -- like niri and sway -- binds by the
/// unshifted (level-0) keysym only, tracking Shift as an ordinary modifier
/// (see that module's doc). Without folding first, `"Super+H"` would
/// resolve to `Keysym::H`: a syntactically valid bind that can never fire,
/// since no real keypress ever reports that keysym under this project's
/// matching scheme -- exactly the silently-wrong-config class this loader
/// exists to avoid.
///
/// The fold is deliberately scoped to single ASCII letters, not the whole
/// name the way this function used to do it: some multi-character names
/// are cased pairs of *distinct* keysyms (`OE`/`oe` are Œ/œ), and folding
/// those would silently redirect the bind to another key. Multi-character
/// names resolve exactly as written, through `keysym_named`'s existing
/// case-insensitive fallback -- the same one already used for e.g.
/// `"return"` -- and non-ASCII passes through unfolded, since keysym case
/// semantics outside ASCII are murky (e.g. Turkish dotted/dotless I): a
/// name the lookup doesn't know stays an unknown key, skipped with a
/// warning by `apply_binds`, rather than a folded guess at another keysym.
/// `keysym_named` itself is untouched, so `scoot msg key A` keeps refusing
/// rather than silently becoming `a`.
///
/// Folding changes what the bind means -- `"A"` is the unshifted `a` key,
/// *not* `shift+a` -- so it warns, naming the bind and the spelling for
/// Shift, rather than applying silently. Only an actually ambiguous fold
/// warns: a letter that changed case on a combo that does *not* name Shift.
/// With Shift named (`"shift+A"`) there is no question what was meant --
/// the chord is shift+a either way -- so folding warns about nothing. A
/// lone lowercase letter needs no warning either: nothing was changed.
fn parse_combo(s: &str) -> Result<(Modifiers, Keysym), String> {
    let combo: scoot_ipc::KeyCombo =
        s.parse().map_err(|error: scoot_ipc::ParseKeyComboError| {
            format!("invalid key combination `{s}`: {error}")
        })?;
    let mut mods = Modifiers::default();
    for modifier in &combo.modifiers {
        match modifier {
            scoot_ipc::Modifier::Super => mods.super_ = true,
            scoot_ipc::Modifier::Shift => mods.shift = true,
            scoot_ipc::Modifier::Ctrl => mods.ctrl = true,
            scoot_ipc::Modifier::Alt => mods.alt = true,
        }
    }
    let (name, warn) = fold_letter(&combo.key, mods.shift);
    if warn {
        tracing::warn!(
            bind = s,
            "a single capital letter in [binds] names the unshifted key -- \
             this bind means plain `{}`, not `shift+{}`; write `shift+{}` \
             if Shift was meant",
            name,
            name,
            name,
        );
    }
    let keysym = keysym_named(&name).ok_or_else(|| format!("unknown key `{}`", combo.key))?;
    Ok((mods, keysym))
}

/// Folds a single ASCII letter to lowercase for `[binds]` matching (see
/// `parse_combo`), reporting alongside whether the fold is worth a warning.
///
/// The `bool` is the warn decision, factored out so tests can pin it
/// without a tracing subscriber: only a letter that actually changed case,
/// on a combo that does *not* hold Shift, is ambiguous enough to warn
/// about. Everything else -- an already-lowercase letter, a digit, a
/// multi-character name, a non-ASCII name, or any letter with Shift named
/// -- resolves quietly.
fn fold_letter(key: &str, shift_held: bool) -> (Cow<'_, str>, bool) {
    // One byte: a single ASCII letter is exactly one byte, so this also
    // excludes every non-ASCII letter without naming an encoding.
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        let folded = key.to_ascii_lowercase();
        if folded == key {
            (Cow::Borrowed(key), false)
        } else {
            (Cow::Owned(folded), !shift_held)
        }
    } else {
        (Cow::Borrowed(key), false)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use scoot_core::{Action, Horizontal};

    use super::*;

    fn write_temp(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        let mut file = fs::File::create(&path).expect("create temp config file");
        file.write_all(contents.as_bytes())
            .expect("write temp config");
        (dir, path)
    }

    #[test]
    fn startup_path_is_the_explicit_path_or_the_xdg_default() {
        let explicit = PathBuf::from("/etc/scoot/config.toml");
        assert_eq!(
            startup_path(Some(&explicit), None, None),
            Some(explicit.clone()),
            "an explicit --config wins over every default"
        );
        assert_eq!(
            startup_path(
                None,
                Some(OsString::from("/xdg")),
                Some(OsString::from("/home/dev")),
            ),
            Some(PathBuf::from("/xdg/scoot/config.toml")),
            "XDG_CONFIG_HOME wins over HOME"
        );
        assert_eq!(
            startup_path(
                None,
                Some(OsString::from("")),
                Some(OsString::from("/home/dev"))
            ),
            Some(PathBuf::from("/home/dev/.config/scoot/config.toml")),
            "an empty XDG_CONFIG_HOME falls back to HOME"
        );
        assert_eq!(
            startup_path(None, None, None),
            None,
            "nowhere to look means no path, not a guessed one"
        );
    }

    #[test]
    fn reload_from_loads_a_valid_file_and_refuses_a_bad_one() {
        let (_dir, path) = write_temp("[layout]\ngap = 20\n");
        let loaded = reload_from(&path, false).expect("a valid file reloads");
        assert_eq!(loaded.config.gap, 20);

        let (_dir, bad) = write_temp("this is not valid toml [[[");
        let error = reload_from(&bad, false).expect_err("a malformed file must not reload");
        assert!(
            error.to_string().contains("keeping the running config"),
            "the refusal must say the session is untouched: {error}"
        );

        let missing = bad.parent().unwrap().join("does-not-exist.toml");
        reload_from(&missing, false).expect_err("a vanished path must not reload");
    }

    #[test]
    fn reload_from_rejects_an_unknown_field_like_startup_parses_it() {
        // Whole-file validation before anything applies: an unknown field
        // fails the reload even though every other table is valid.
        let (_dir, path) = write_temp("[layout]\ngaps = 5\n");
        reload_from(&path, false).expect_err("an unknown field must not reload");
    }

    #[test]
    fn a_full_valid_file_round_trips() {
        let toml = r#"
            [layout]
            gap = 20
            column_widths = [0.25, 0.5, 0.75]
            default_column_width = 2

            [binds]
            "super+n" = "focus-column right"
            "super+shift+q" = "spawn foot -e htop"
        "#;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        assert_eq!(
            file.layout,
            Some(LayoutConfig {
                gap: Some(20),
                column_widths: Some(vec![0.25, 0.5, 0.75]),
                default_column_width: Some(2),
            })
        );
        assert_eq!(file.binds.len(), 2);

        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.config.gap, 20);
        assert_eq!(loaded.config.column_widths, vec![0.25, 0.5, 0.75]);
        assert_eq!(loaded.config.default_column_width, 2);
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Right)))
        );
    }

    #[test]
    fn a_partial_layout_table_only_overrides_what_it_names() {
        let toml = "[layout]\ngap = 3\n";
        let file: FileConfig = toml::from_str(toml).unwrap();
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.config.gap, 3);
        assert_eq!(loaded.config.column_widths, Config::default().column_widths);
        assert_eq!(
            loaded.config.default_column_width,
            Config::default().default_column_width
        );
    }

    #[test]
    fn deny_unknown_fields_rejects_a_typo() {
        let toml = "[layout]\ngaps = 5\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn deny_unknown_fields_rejects_a_typo_at_the_top_level() {
        let toml = "[layuot]\ngap = 5\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn a_missing_file_at_the_default_path_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let loaded = load_from(&missing, false).expect("a missing default path is not an error");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn an_explicit_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let error = load_from(&missing, true).expect_err("an explicit --config must be readable");
        assert!(error.to_string().contains("does-not-exist.toml"));
    }

    #[test]
    fn a_user_bind_overrides_the_matching_default() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+h" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid config");
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::h,
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused))
        );
    }

    #[test]
    fn an_index_action_parses_through_a_digit_combo_bind() {
        // The config-grammar half of the workspace-index item: a digit combo
        // plus the new action string, through the shared `scootctl::action`
        // parser rather than a parallel one. Rebinding `super+3` (a combo
        // that now has a default) replaces that default.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+3" = "move-window-to-workspace-index 2"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid config");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("3").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::MoveWindowToWorkspaceIndex(2)))
        );
    }

    #[test]
    fn a_malformed_bind_is_skipped_and_the_rest_of_the_file_still_loads() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+notakey" = "close"
            "super+n" = "focus-column right"
        "#,
        );
        let loaded = load_from(&path, true).expect("the file still parses as toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Right)))
        );
    }

    #[test]
    fn a_bad_action_string_is_skipped_without_touching_other_binds() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+n" = "not-a-real-action"
            "super+m" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("the file still parses as toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("m").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused))
        );
    }

    #[test]
    fn trailing_text_after_the_action_is_rejected() {
        assert_eq!(
            parse_bind("super+n", "focus-column left extra-garbage"),
            Err("trailing text after the action in `focus-column left extra-garbage`".into())
        );
    }

    #[test]
    fn malformed_toml_falls_back_to_full_defaults_not_an_error() {
        let (_dir, path) = write_temp("this is not valid toml [[[");
        let loaded = load_from(&path, true).expect("a parse failure must never fail startup");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn an_unknown_field_falls_back_to_full_defaults_not_an_error() {
        let (_dir, path) = write_temp("[layout]\ngaps = 5\n");
        let loaded = load_from(&path, true).expect("a bad field must never fail startup");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn toml_itself_rejects_a_literal_duplicate_key() {
        // TOML forbids defining the same key twice in a table -- this is a
        // syntax error the `toml` crate's parser raises, before `FileConfig`
        // (or its `HashMap`) ever sees it. There is no "duplicate key" case
        // for this module's code to handle at the Rust level.
        let toml = "[binds]\n\"super+h\" = \"close\"\n\"super+h\" = \"quit\"\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn two_combo_strings_that_normalize_the_same_are_both_skipped() {
        // "Super+H" and "super+h" are two *different* TOML keys (so this
        // parses fine as TOML), but both resolve to the same (mods, keysym)
        // once parsed -- exactly the collision `apply_binds` refuses to
        // arbitrate. Neither is applied; the default for Super+h survives.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "Super+H" = "close"
            "super+h" = "quit"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::h,
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Left))),
            "the collision must be skipped entirely, leaving the default in place"
        );
    }

    #[test]
    fn a_lone_capital_letter_bind_fires_on_the_unshifted_key() {
        // The binds-capital-letter ticket: `"A" = "close"` used to parse
        // and load but never fire, because the exact lookup yields the
        // distinct `A` keysym while matching only ever sees the unshifted
        // `a`. The parse-time fold below must make this bind the `a` key.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "A" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded
                .keybindings
                .match_key(Keysym::a, Modifiers::default()),
            Some(Bound::Action(Action::CloseFocused)),
            "`\"A\"` must bind the unshifted `a` key"
        );
    }

    #[test]
    fn only_a_single_ascii_letter_is_case_folded() {
        // The fold is scoped to single ASCII letters: `"A"` folds to `a`,
        // and an already-lowercase `"a"` resolves identically.
        assert_eq!(parse_combo("A").unwrap().1, Keysym::a);
        assert_eq!(parse_combo("a").unwrap().1, Keysym::a);
        assert_eq!(parse_combo("Z").unwrap().1, Keysym::z);
        // Digits and symbols pass through untouched.
        assert_eq!(parse_combo("1").unwrap().1, keysym_named("1").unwrap());
        // Multi-character names resolve exactly as written -- including via
        // the case-insensitive fallback -- never folded first. That matters
        // because some are cased pairs of distinct keysyms: folding the
        // whole name would silently redirect "OE" (Œ) to "oe" (œ).
        assert_ne!(
            keysym_named("OE"),
            keysym_named("oe"),
            "premise: OE/oe must really be distinct keysyms for this test to mean anything"
        );
        assert_eq!(parse_combo("OE").unwrap().1, keysym_named("OE").unwrap());
        assert_eq!(parse_combo("F1").unwrap().1, Keysym::F1);
        assert_eq!(parse_combo("Return").unwrap().1, Keysym::Return);
        assert_eq!(parse_combo("RETURN").unwrap().1, Keysym::Return);
        // Non-ASCII passes through unfolded: keysym case semantics outside
        // ASCII are murky (e.g. Turkish dotted/dotless I), so a name the
        // lookup doesn't know stays an unknown key -- skipped with a warning
        // by `apply_binds` -- rather than a folded guess at another keysym.
        assert_eq!(
            parse_combo("É"),
            Err("unknown key `É`".into()),
            "`É` is not an xkb keysym name and must not fold into one"
        );
    }

    #[test]
    fn shift_plus_lowercase_still_requires_shift() {
        // The other direction: naming Shift explicitly still means Shift.
        // A lone `"A"` (previous test) must not grow a Shift requirement,
        // and `"shift+a"` must not lose its one.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "shift+a" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded
                .keybindings
                .match_key(Keysym::a, Modifiers::default()),
            None,
            "an unshifted `a` must not fire a `shift+a` bind"
        );
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::a,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused)),
            "`shift+a` must fire with Shift held"
        );
    }

    #[test]
    fn a_capital_with_shift_named_binds_shift_quietly() {
        // The review catch on the warn: `"shift+A"` folds the key but means
        // shift+a either way, so warning "this bind means plain `a`, not
        // `shift+a`" would be factually wrong -- a fully-correct config
        // scolded at every startup. Only an ambiguous fold warns.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "shift+A" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded
                .keybindings
                .match_key(Keysym::a, Modifiers::default()),
            None,
            "an unshifted `a` must not fire a `shift+A` bind"
        );
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::a,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused)),
            "`shift+A` must fire with Shift held"
        );
        // The warn decision itself, pinned without a tracing subscriber:
        // `parse_combo` warns exactly when `fold_letter` says so.
        let (name, warn) = fold_letter("A", false);
        assert_eq!(name.as_ref(), "a");
        assert!(warn, "a bare capital is ambiguous: warn");
        let (name, warn) = fold_letter("A", true);
        assert_eq!(name.as_ref(), "a");
        assert!(!warn, "Shift named means shift+a either way: stay quiet");
        let (name, warn) = fold_letter("a", false);
        assert_eq!(name.as_ref(), "a");
        assert!(!warn, "nothing changed: stay quiet");
        let (name, warn) = fold_letter("OE", false);
        assert_eq!(name.as_ref(), "OE");
        assert!(!warn, "never folded: stay quiet");
    }

    #[test]
    fn a_multi_character_key_name_is_also_case_insensitive() {
        // Unlike a single letter, "return" isn't itself a distinct valid
        // keysym -- exact-case lookup fails and falls back to
        // case-insensitive, landing on the same `Keysym::Return` either way
        // -- and multi-character names are never folded (see `parse_combo`'s
        // doc), so this exercises the fallback path as-is, as opposed to
        // the single-letter test above exercising the fold.
        assert_eq!(
            parse_combo("super+Return").unwrap(),
            parse_combo("super+return").unwrap()
        );
    }

    #[test]
    fn modifier_aliases_collide_like_case_does() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+n" = "close"
            "logo+n" = "quit"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            None,
            "no default exists for Super+n, and the collision must not apply either alias"
        );
    }

    // -- [autostart] --------------------------------------------------------

    #[test]
    fn a_full_autostart_table_round_trips_in_file_order() {
        let toml = r#"
            [autostart]
            commands = [
                "spawn waybar",
                "spawn foot -e htop",
                "focus-workspace-index 2",
            ]
        "#;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.autostart,
            vec![
                Action::Spawn(vec!["waybar".into()]),
                Action::Spawn(vec!["foot".into(), "-e".into(), "htop".into()]),
                Action::FocusWorkspaceIndex(2),
            ]
        );
    }

    #[test]
    fn a_missing_autostart_table_means_nothing_runs() {
        let file: FileConfig = toml::from_str("").unwrap();
        let loaded = LoadedConfig::from_file(file);
        assert!(loaded.autostart.is_empty());
    }

    #[test]
    fn an_empty_autostart_list_means_nothing_runs() {
        let (_dir, path) = write_temp("[autostart]\ncommands = []\n");
        let loaded = load_from(&path, true).expect("valid config");
        assert!(loaded.autostart.is_empty());
    }

    #[test]
    fn an_invalid_autostart_entry_is_skipped_and_the_session_still_starts() {
        // The load-bearing fail-open pin: a typo in one entry must cost that
        // entry, never the session. On `--tty` scoot *is* the session, so a
        // refusal here would be a hard lockout over a typo -- `load_from`
        // still returns `Ok`, the valid entries still parse, and the rest of
        // the file (binds included) still applies.
        let (_dir, path) = write_temp(
            r#"
            [autostart]
            commands = [
                "not-a-real-action",
                "spawn waybar",
                "focus-column sideways",
                "",
                "close extra-garbage",
            ]

            [binds]
            "super+n" = "focus-column right"
        "#,
        );
        let loaded = load_from(&path, true).expect("a bad autostart entry must never fail startup");
        assert_eq!(
            loaded.autostart,
            vec![Action::Spawn(vec!["waybar".into()])],
            "only the one valid entry survives"
        );
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Right))),
            "a bad autostart entry must not cost the rest of the file"
        );
    }

    #[test]
    fn autostart_trailing_text_after_the_action_is_rejected() {
        assert_eq!(
            parse_autostart("focus-column left extra-garbage"),
            Err("trailing text after the action in `focus-column left extra-garbage`".into())
        );
        // `spawn` consumes the rest of the line as its command, so this is
        // one spawn of three words, not an action plus trailing text.
        assert_eq!(
            parse_autostart("spawn foot -e htop"),
            Ok(Action::Spawn(vec![
                "foot".into(),
                "-e".into(),
                "htop".into()
            ]))
        );
    }

    #[test]
    fn autostart_accepts_a_non_spawn_action() {
        // No spawn-only restriction: the full action grammar applies, and a
        // non-spawn action at startup is the user's choice.
        assert_eq!(
            parse_autostart("focus-workspace-index 2"),
            Ok(Action::FocusWorkspaceIndex(2))
        );
        assert_eq!(parse_autostart("quit"), Ok(Action::Quit));
    }

    #[test]
    fn an_autostart_wrong_type_falls_back_to_full_defaults() {
        // `commands` given the wrong type is a whole-file parse error, the
        // same as any other mistyped field -- fail-open at the file level
        // (defaults, never a refusal), while a bad *entry* only costs that
        // entry (see the fail-open pin above).
        let (_dir, path) = write_temp("[autostart]\ncommands = \"spawn waybar\"\n");
        let loaded = load_from(&path, true).expect("a mistyped field must never fail startup");
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.autostart.is_empty());
    }

    #[test]
    fn deny_unknown_fields_rejects_an_autostart_typo() {
        let toml = "[autostart]\ncommand = [\"spawn waybar\"]\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn a_hundred_entry_autostart_list_parses() {
        // The 100-entry edge: a cold-path parse cost only, paid once at
        // startup -- no bound needed, but the shape must hold together.
        let mut toml = String::from("[autostart]\ncommands = [\n");
        for i in 0..100 {
            toml.push_str(&format!("    \"spawn program-{i}\",\n"));
        }
        toml.push_str("]\n");
        let file: FileConfig = toml::from_str(&toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.autostart.len(), 100);
        assert_eq!(
            loaded.autostart[99],
            Action::Spawn(vec!["program-99".into()])
        );
    }

    #[test]
    fn default_path_prefers_xdg_config_home_over_home() {
        assert_eq!(
            default_path(Some("/xdg".into()), Some("/home/u".into())),
            Some(PathBuf::from("/xdg/scoot/config.toml"))
        );
    }

    #[test]
    fn default_path_falls_back_to_home() {
        assert_eq!(
            default_path(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/scoot/config.toml"))
        );
    }

    #[test]
    fn default_path_is_none_with_neither_var_set() {
        assert_eq!(default_path(None, None), None);
    }

    // -- [appearance] -----------------------------------------------------

    #[test]
    fn a_full_appearance_table_round_trips() {
        let toml = r##"
            [appearance]
            focus_ring_width = 5
            focus_ring_active_color = "#ff0000"
            focus_ring_inactive_color = "#00ff0080"
            background_color = "#101010"
            corner_radius = 12
            cursor_size = 32
            cursor_color = "#ff8000"
            cursor_theme = "Adwaita"
            prefer_no_csd = false
        "##;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.appearance.focus_ring_width, 5);
        assert_eq!(loaded.appearance.corner_radius, 12);
        assert_eq!(loaded.appearance.cursor_size, 32);
        assert_eq!(
            loaded.appearance.cursor_theme.as_deref(),
            Some("Adwaita"),
            "cursor_theme did not round-trip"
        );
        assert_eq!(
            loaded.appearance.cursor_color,
            Color::new(1.0, 128.0 / 255.0, 0.0, 1.0)
        );
        assert_eq!(
            loaded.appearance.focus_ring_active_color,
            Color::new(1.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            loaded.appearance.focus_ring_inactive_color,
            Color::new(0.0, 1.0, 0.0, 128.0 / 255.0)
        );
        assert_eq!(
            loaded.appearance.background_color,
            Color::new(16.0 / 255.0, 16.0 / 255.0, 16.0 / 255.0, 1.0)
        );
        assert!(!loaded.appearance.prefer_no_csd);
    }

    #[test]
    fn a_missing_appearance_table_uses_defaults() {
        let file: FileConfig = toml::from_str("").unwrap();
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.appearance, Appearance::default());
    }

    #[test]
    fn a_negative_corner_radius_is_clamped_to_zero_at_load() {
        let toml = "[appearance]\ncorner_radius = -12\n";
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.appearance.corner_radius, 0);
    }

    #[test]
    fn deny_unknown_fields_rejects_an_appearance_typo() {
        let toml = "[appearance]\nfocus_ring_wdith = 5\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn a_malformed_color_falls_back_to_just_that_fields_default() {
        let toml = r##"
            [appearance]
            focus_ring_active_color = "not-a-color"
            focus_ring_inactive_color = "#00ff00"
        "##;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.appearance.focus_ring_active_color,
            Appearance::default().focus_ring_active_color,
            "the bad field falls back to its own default"
        );
        assert_eq!(
            loaded.appearance.focus_ring_inactive_color,
            Color::new(0.0, 1.0, 0.0, 1.0),
            "a bad neighboring field must not affect a valid one"
        );
    }

    #[test]
    fn a_ring_width_wider_than_half_the_configured_gap_is_clamped() {
        let toml = r#"
            [layout]
            gap = 6

            [appearance]
            focus_ring_width = 20
        "#;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.appearance.focus_ring_width, 3,
            "clamped to half of gap=6"
        );
    }

    /// The ring is measured against the gap the layout will actually use, not
    /// the raw number in the file: `scoot_core::Config::validated` caps the
    /// gap at `MAX_GAP`, so a ring sized against an out-of-range gap would
    /// otherwise end up wider than half of the real one.
    #[test]
    fn a_ring_is_clamped_against_the_capped_gap_not_the_configured_one() {
        let toml = format!(
            "[layout]\ngap = {}\n\n[appearance]\nfocus_ring_width = {}\n",
            i32::MAX,
            Config::MAX_GAP
        );
        let file: FileConfig = toml::from_str(&toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.appearance.focus_ring_width,
            Config::MAX_GAP / 2,
            "clamped to half of the capped gap"
        );
    }

    // -- [appearance] cursor fields ---------------------------------------

    /// A config file sets just the cursor fields: they apply, and nothing
    /// else in `[appearance]` moves off its default.
    #[test]
    fn a_partial_appearance_table_can_set_only_the_cursor_fields() {
        let (_dir, path) = write_temp(
            r##"
            [appearance]
            cursor_size = 48
            cursor_color = "#0080ff"
        "##,
        );
        let loaded = load_from(&path, true).expect("valid config");
        assert_eq!(loaded.appearance.cursor_size, 48);
        assert_eq!(
            loaded.appearance.cursor_color,
            Color::new(0.0, 128.0 / 255.0, 1.0, 1.0)
        );
        assert_eq!(
            loaded.appearance.focus_ring_active_color,
            Appearance::default().focus_ring_active_color
        );
        assert_eq!(
            loaded.appearance.background_color,
            Appearance::default().background_color
        );
    }

    /// The same graceful-degradation rule every other `[appearance]` color
    /// follows: one unparseable string costs that field its value, not the
    /// file.
    #[test]
    fn a_malformed_cursor_color_falls_back_to_just_that_field() {
        let (_dir, path) = write_temp(
            r##"
            [appearance]
            cursor_size = 24
            cursor_color = "ff8000"
        "##,
        );
        let loaded = load_from(&path, true).expect("a bad color must not fail startup");
        assert_eq!(
            loaded.appearance.cursor_color,
            Appearance::default().cursor_color,
            "the bad color falls back to its own default"
        );
        assert_eq!(
            loaded.appearance.cursor_size, 24,
            "a bad color must not cost a valid neighboring field"
        );
    }

    /// Both ends of the clamp, from a real file: a size a config file can
    /// spell but the bitmap path must never see (see
    /// `Appearance::MAX_CURSOR_SIZE` for why the upper bound is about an
    /// allocation, not taste).
    #[test]
    fn an_out_of_range_cursor_size_is_clamped_at_load_time() {
        for (configured, expected) in [
            (i32::MAX, Appearance::MAX_CURSOR_SIZE),
            (Appearance::MAX_CURSOR_SIZE + 1, Appearance::MAX_CURSOR_SIZE),
            (0, Appearance::MIN_CURSOR_SIZE),
            (-16, Appearance::MIN_CURSOR_SIZE),
            (i32::MIN, Appearance::MIN_CURSOR_SIZE),
        ] {
            let (_dir, path) = write_temp(&format!("[appearance]\ncursor_size = {configured}\n"));
            let loaded = load_from(&path, true).expect("a bad size must not fail startup");
            assert_eq!(
                loaded.appearance.cursor_size, expected,
                "cursor_size {configured} was not clamped"
            );
        }
    }

    /// A size outside `i32` entirely is a *parse* failure, not a clamp -- the
    /// field's type decides that, and the whole file falls back to defaults
    /// (same rule as any other type mismatch; see the module doc). 3e9 is a
    /// legal TOML integer (they are 64-bit) but not a legal `i32`, so this is
    /// serde's range check, not the TOML parser's.
    #[test]
    fn a_cursor_size_too_large_for_i32_is_a_whole_file_fallback() {
        let (_dir, path) =
            write_temp("[layout]\ngap = 20\n\n[appearance]\ncursor_size = 3000000000\n");
        let loaded = load_from(&path, true).expect("a parse failure must never fail startup");
        assert_eq!(loaded.appearance, Appearance::default());
        assert_eq!(
            loaded.config.gap,
            Config::default().gap,
            "the whole file is discarded, including a valid [layout]"
        );
    }

    /// The cursor clamp and the ring clamp are independent: `cursor_size` has
    /// nothing to do with the gap, and must survive a gap small enough to
    /// clamp the ring to nothing.
    #[test]
    fn a_tiny_gap_clamps_the_ring_but_not_the_cursor() {
        let toml = r#"
            [layout]
            gap = 0

            [appearance]
            focus_ring_width = 9
            cursor_size = 64
        "#;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.appearance.focus_ring_width, 0);
        assert_eq!(loaded.appearance.cursor_size, 64);
    }

    /// The actual overflow-risking input, not just the visual-consistency
    /// case above: with an unclamped gap, `focus_ring_width = i32::MAX` at
    /// `gap = i32::MAX` would clamp to `gap.max(0) / 2 = 1073741823`, and
    /// `decorations::ring_rects`'s `rect.w + 2 * width` overflows for any
    /// `rect.w >= 2` -- a debug panic or a release wraparound on every
    /// render, from a config file alone. Clamping the gap first closes this,
    /// not just the proportion between the ring and the gap.
    #[test]
    fn an_out_of_range_ring_width_is_also_clamped_against_the_capped_gap() {
        let toml = format!(
            "[layout]\ngap = {}\n\n[appearance]\nfocus_ring_width = {}\n",
            i32::MAX,
            i32::MAX
        );
        let file: FileConfig = toml::from_str(&toml).expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.appearance.focus_ring_width,
            Config::MAX_GAP / 2,
            "clamped to half of the capped gap, not half of i32::MAX"
        );
    }

    // -- [output] ---------------------------------------------------------

    #[test]
    fn a_fractional_output_scale_round_trips() {
        let file: FileConfig = toml::from_str("[output]\nscale = 1.5\n").expect("valid toml");
        assert_eq!(file.output, Some(OutputConfig { scale: Some(1.5) }));
        assert_eq!(LoadedConfig::from_file(file).scale, 1.5);
    }

    /// TOML integers and floats are distinct types; `scale = 2` is the way
    /// most people will write a whole-number scale, and serde's `f64` visitor
    /// accepts an integer for exactly this reason. Pinned here so a future
    /// field type change can't silently make `scale = 2` a parse failure.
    #[test]
    fn an_integer_output_scale_is_accepted_as_a_float() {
        let file: FileConfig = toml::from_str("[output]\nscale = 2\n").expect("valid toml");
        assert_eq!(LoadedConfig::from_file(file).scale, 2.0);
    }

    #[test]
    fn a_missing_output_table_means_scale_one() {
        let file: FileConfig = toml::from_str("").unwrap();
        assert_eq!(LoadedConfig::from_file(file).scale, 1.0);
    }

    #[test]
    fn deny_unknown_fields_rejects_an_output_typo() {
        let toml = "[output]\nscales = 2\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    /// The same graceful-degradation rule every other config field follows:
    /// an out-of-range scale is clamped with a warning, never a startup
    /// failure -- on `--tty` a refused config would be a hard lockout.
    #[test]
    fn an_out_of_range_output_scale_is_clamped() {
        // Spelled as TOML *float* literals: `1e300` rather than an
        // integer-looking string, which TOML would reject as integer overflow
        // before the field type ever saw it.
        for (configured, expected) in [
            ("0.1", MIN_SCALE),
            ("-1.0", MIN_SCALE),
            ("9.0", MAX_SCALE),
            ("1e300", MAX_SCALE),
        ] {
            let text = format!("[output]\nscale = {configured}\n");
            let file: FileConfig = toml::from_str(&text).expect("valid toml");
            assert_eq!(
                LoadedConfig::from_file(file).scale,
                expected,
                "scale {configured} was not clamped"
            );
        }
    }

    /// TOML can spell `nan` and `inf`; neither is a usable output scale, so
    /// both resolve to 1.0 rather than poisoning `physical / scale`.
    #[test]
    fn a_non_finite_output_scale_falls_back_to_one() {
        for spelling in ["nan", "inf", "-inf"] {
            let text = format!("[output]\nscale = {spelling}\n");
            let file: FileConfig = toml::from_str(&text).expect("valid toml");
            assert_eq!(
                LoadedConfig::from_file(file).scale,
                1.0,
                "scale = {spelling} did not fall back to 1.0"
            );
        }
    }

    /// Every way TOML can nest, each built to exactly `depth` levels, always
    /// as (or under) one top-level key `a` so `deny_unknown_fields` gives the
    /// same "unknown field" rejection for all five whenever the *parse* got
    /// that far. The first two are bounded by `toml`'s `RecursionGuard`, the
    /// last three by its separate key-path limit -- see `parse_or_defaults`'
    /// doc.
    const NESTING_FORMS: [&str; 5] = [
        "inline table",
        "array",
        "dotted key",
        "table header",
        "array-of-tables header",
    ];

    fn nested_toml(form: &str, depth: usize) -> String {
        let segments = "a.".repeat(depth.saturating_sub(1));
        match form {
            "inline table" => format!("a = {}1{}", "{a=".repeat(depth), "}".repeat(depth)),
            "array" => format!("a = {}{}", "[".repeat(depth), "]".repeat(depth)),
            "dotted key" => format!("{segments}a = 1"),
            "table header" => format!("[{segments}a]\n"),
            "array-of-tables header" => format!("[[{segments}a]]\n"),
            other => unreachable!("unknown nesting form `{other}`"),
        }
    }

    /// Pins where `toml` 1.1.6 stops recursing, through this module's real
    /// parse path. Depth 80 parses (and is then rejected by
    /// `deny_unknown_fields`, which is how we know the parser got all the way
    /// through); 81 is refused by the crate itself.
    ///
    /// This is the regression test that matters if the `toml` dependency
    /// moves: `toml = "1"` accepts any 1.x, and the limits are only active
    /// while the crate's `unbounded` feature is off, which feature unification
    /// from some future dependency could silently flip. Either change fails
    /// this test by assertion -- long before it can fail as an unbounded
    /// recursion on a user's config file.
    #[test]
    fn tomls_own_depth_limits_stop_at_80_levels_of_every_nesting_form() {
        for form in NESTING_FORMS {
            let at_limit = toml::from_str::<FileConfig>(&nested_toml(form, 80))
                .expect_err("the top-level key is not a known field");
            assert!(
                at_limit.message().contains("unknown field"),
                "80 levels of {form} must still parse, leaving only the \
                 unknown-field rejection; got: {}",
                at_limit.message()
            );

            let past_limit = toml::from_str::<FileConfig>(&nested_toml(form, 81))
                .expect_err("81 levels is past toml's limit");
            assert!(
                past_limit.message().contains("recurs"),
                "81 levels of {form} must be refused by toml's own depth \
                 limit; got: {}",
                past_limit.message()
            );
        }
    }

    /// The module's promise under the input the backlog entry worried about:
    /// 100,000 levels of nesting is a logged parse error and full defaults,
    /// not a stack overflow. `toml` stops at level 81, so the remaining
    /// ~400 KB of the file is skipped iteratively and never becomes stack
    /// frames.
    #[test]
    fn an_absurdly_deep_config_file_falls_back_to_defaults_not_a_crash() {
        for form in NESTING_FORMS {
            let (_dir, path) = write_temp(&nested_toml(form, 100_000));
            let loaded =
                load_from(&path, true).expect("deep nesting must never fail startup either");
            assert_eq!(loaded.config, Config::default(), "{form}");
        }
    }

    /// The deepest *tree* `toml`'s two limits allow between them: its 80-level
    /// nesting guard and its 80-segment key-path limit multiply rather than
    /// add, because a dotted key can sit at every level of nesting. Each of the
    /// three parts below is at its own cap, so nothing a config file can
    /// express goes deeper:
    ///
    /// - an 80-segment *array-of-tables* header, worth 81 levels -- its last
    ///   segment is an array holding a table, one level more than the plain
    ///   `[a.a...]` header spends there;
    /// - 80 nested inline tables (all the `RecursionGuard` allows), each keyed
    ///   by a fresh 80-segment dotted key, worth 80 levels apiece;
    /// - an 80-segment dotted key for the leaf, worth 79 more.
    ///
    /// 13,448 bytes, 6,561 nested tables/arrays counting the document root,
    /// with the scalar at the bottom as the 6,562nd node on that path.
    ///
    /// Dropping that tree is recursive, which is what actually costs stack
    /// here -- parsing it needs 476 KiB in a debug build and less than a thread
    /// stack's ~134 KiB floor in a release one. Measured 2026-09-13 on aarch64
    /// (macOS and the Linux dev VM), `toml` 1.1.6: this test's body completes
    /// on a 6,684 KiB stack in a debug build, 1,140 KiB in a release one. So it
    /// runs on an 8 MiB thread, matching the main thread `compositor::run` --
    /// and therefore `load` -- actually gets, rather than the 2 MiB `cargo
    /// test` would otherwise hand it. If this ever overflows, that is a real
    /// finding about production, not test flakiness: the margin in a debug
    /// build is only ~1.5 MiB. Note the symptom to expect, since the overflow
    /// would happen on the spawned thread: it aborts the whole test binary, so
    /// every test reports nothing rather than this one failing an assertion.
    #[test]
    fn the_deepest_tree_tomls_limits_allow_falls_back_to_defaults_too() {
        let segments = "a.".repeat(79);
        let text = format!(
            "[[{segments}a]]\n{}{segments}a = 1{}\n",
            format!("{segments}a = {{").repeat(80),
            "}".repeat(80)
        );
        assert_eq!(
            text.len(),
            13_448,
            "the worst case this test means to build"
        );

        let config = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let (_dir, path) = write_temp(&text);
                load_from(&path, true)
                    .expect("the deepest possible file must never fail startup")
                    .config
            })
            .expect("spawn the 8 MiB parse thread")
            .join()
            .expect("parsing the deepest possible config must not panic or abort");
        assert_eq!(config, Config::default());
    }

    /// The other half of that: a real config is nowhere near any of it. The
    /// deepest shape scoot's own schema can produce is a table holding an
    /// array (3 levels), so the 80-level limits cost a legitimate config
    /// nothing, whichever TOML spelling it uses.
    #[test]
    fn a_realistic_config_nests_far_below_tomls_limits() {
        let inline = "layout = { gap = 7, column_widths = [0.3, 0.7] }\n";
        let dotted = "layout.gap = 7\nlayout.column_widths = [0.3, 0.7]\n";
        for text in [inline, dotted] {
            let (_dir, path) = write_temp(text);
            let loaded = load_from(&path, true).expect("valid toml");
            assert_eq!(loaded.config.gap, 7, "{text}");
            assert_eq!(loaded.config.column_widths, vec![0.3, 0.7], "{text}");
        }
    }

    /// An empty `cursor_theme` means "no override", not "a theme called
    /// nothing" -- see `into_appearance`. Without this, `Theme::load` would
    /// search for a theme that cannot exist instead of falling through to
    /// `$XCURSOR_THEME`.
    #[test]
    fn an_empty_cursor_theme_is_treated_as_unset() {
        let file: FileConfig = toml::from_str(
            r#"
            [appearance]
            cursor_theme = ""
        "#,
        )
        .expect("valid toml");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.appearance.cursor_theme, None);
    }

    // -- [renderer] -------------------------------------------------------

    #[test]
    fn a_renderer_backend_key_round_trips_for_both_names() {
        for (name, expected) in [
            ("pixman", RendererKind::Pixman),
            ("gles", RendererKind::Gles),
        ] {
            let toml = format!("[renderer]\nbackend = \"{name}\"\n");
            let file: FileConfig = toml::from_str(&toml).expect("valid toml");
            assert_eq!(
                file.renderer,
                Some(RendererConfig {
                    backend: Some(name.to_owned()),
                })
            );
            assert_eq!(LoadedConfig::from_file(file).renderer, Some(expected));
        }
    }

    #[test]
    fn a_missing_renderer_table_or_backend_key_leaves_the_choice_open() {
        // `None`, not `Some(Pixman)`: the file saying nothing is what lets
        // `--renderer` decide, and the default only applies when neither
        // does (see `render::resolve`).
        let file: FileConfig = toml::from_str("").unwrap();
        assert_eq!(file.renderer, None);
        assert_eq!(LoadedConfig::from_file(file).renderer, None);

        let file: FileConfig = toml::from_str("[renderer]\n").expect("valid toml");
        assert_eq!(file.renderer, Some(RendererConfig { backend: None }));
        assert_eq!(LoadedConfig::from_file(file).renderer, None);
    }

    /// An unknown renderer name is an ordinary malformed value: warn, use
    /// the default, start. Unlike `--renderer`, which refuses it -- see this
    /// module's doc for why the two halves of the same key differ, and note
    /// that the whole *file* survives, unlike a type mismatch.
    #[test]
    fn an_unknown_renderer_backend_falls_back_without_taking_the_file_down() {
        for bad in ["vulkan", "GLES", "gles2", "", "opengl"] {
            let toml = format!("[layout]\ngap = 20\n\n[renderer]\nbackend = \"{bad}\"\n");
            let file: FileConfig = toml::from_str(&toml).expect("valid toml");
            let loaded = LoadedConfig::from_file(file);
            assert_eq!(loaded.renderer, None, "{bad}");
            assert_eq!(
                loaded.config.gap, 20,
                "{bad}: the rest of the file survives"
            );
        }
    }

    #[test]
    fn deny_unknown_fields_rejects_a_renderer_typo() {
        assert!(toml::from_str::<FileConfig>("[renderer]\nbackends = \"gles\"\n").is_err());
        assert!(toml::from_str::<FileConfig>("[render]\nbackend = \"gles\"\n").is_err());
    }

    #[test]
    fn a_renderer_backend_of_the_wrong_type_is_a_whole_file_fallback() {
        // Same rule as any other type mismatch (see the module doc): the
        // file is discarded, not the key. A *name* this build doesn't know
        // is the graceful case above; a value that isn't even a string is
        // not.
        let (_dir, path) = write_temp("[layout]\ngap = 20\n\n[renderer]\nbackend = true\n");
        let loaded = load_from(&path, true).expect("a parse failure must never fail startup");
        assert_eq!(loaded.renderer, None);
        assert_eq!(
            loaded.config.gap,
            Config::default().gap,
            "the whole file is discarded, including a valid [layout]"
        );
    }

    // -- [tty] ------------------------------------------------------------

    #[test]
    fn a_tty_gpu_key_round_trips() {
        let file: FileConfig =
            toml::from_str("[tty]\ngpu = \"/dev/dri/card1\"\n").expect("valid toml");
        assert_eq!(
            file.tty,
            Some(TtyConfig {
                gpu: Some(PathBuf::from("/dev/dri/card1")),
            })
        );
        assert_eq!(
            LoadedConfig::from_file(file).gpu,
            Some(PathBuf::from("/dev/dri/card1"))
        );
    }

    #[test]
    fn a_missing_tty_table_or_gpu_key_means_no_explicit_device() {
        let file: FileConfig = toml::from_str("").unwrap();
        assert_eq!(file.tty, None);
        assert_eq!(LoadedConfig::from_file(file).gpu, None);

        let file: FileConfig = toml::from_str("[tty]\n").expect("valid toml");
        assert_eq!(file.tty, Some(TtyConfig { gpu: None }));
        assert_eq!(LoadedConfig::from_file(file).gpu, None);
    }

    #[test]
    fn deny_unknown_fields_rejects_a_tty_typo() {
        let toml = "[tty]\ngpus = \"/dev/dri/card1\"\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn a_tty_gpu_of_the_wrong_type_is_a_whole_file_fallback() {
        // Same rule as any other type mismatch (see the module doc): the
        // file is discarded, not the key.
        let (_dir, path) = write_temp("[layout]\ngap = 20\n\n[tty]\ngpu = 5\n");
        let loaded = load_from(&path, true).expect("a parse failure must never fail startup");
        assert_eq!(loaded.gpu, None);
        assert_eq!(
            loaded.config.gap,
            Config::default().gap,
            "the whole file is discarded, including a valid [layout]"
        );
    }

    /// An empty `gpu = ""` parses to `Some("")`, not `None`: the loader
    /// must not silently swallow what the user wrote (that would be
    /// fail-open -- driving the automatic pick while the config names a
    /// device). Refusing it is `gpu::resolve`'s job at startup, where a
    /// hard error can name the key; see that function's doc.
    #[test]
    fn an_empty_tty_gpu_is_preserved_for_resolve_to_refuse() {
        let file: FileConfig = toml::from_str("[tty]\ngpu = \"\"\n").expect("valid toml");
        assert_eq!(
            LoadedConfig::from_file(file).gpu,
            Some(PathBuf::from("")),
            "an empty gpu must survive loading so resolve can refuse it loudly"
        );
    }

    // -- `--print-default-config` -----------------------------------------

    /// The loop-closing pin from the ticket: the emitted file is generated
    /// from the live defaults, so parsing it back through the same
    /// [`FileConfig`] startup reads must yield those same defaults. A
    /// changed default changes the emission automatically; if the two ever
    /// disagree -- a hand-edited emission, a new field someone forgot to
    /// emit -- this fails.
    ///
    /// The user-facing harm this guards is an emitted file that does not
    /// parse back, or parses to different behavior: a starting config that
    /// silently is not the defaults. Load-bearing, and proven so by
    /// mutation (a wrong `gap` in the emission fails the first assertion).
    #[test]
    fn the_emitted_default_config_parses_back_to_the_live_defaults() {
        let emitted = default_config_toml();
        let file: FileConfig =
            toml::from_str(&emitted).expect("the emitted file must parse as a config");
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(
            loaded.config,
            Config::default(),
            "the [layout] emission drifted from Config::default()"
        );
        assert!(
            loaded.keybindings.same_bindings_as(&Keybindings::default()),
            "the [binds] emission drifted from Keybindings::default()"
        );
        assert_eq!(loaded.scale, 1.0, "the [output] emission drifted");
        assert_eq!(loaded.gpu, None, "the [tty] emission drifted");
        assert_eq!(loaded.renderer, None, "the [renderer] emission drifted");
        assert!(
            loaded.autostart.is_empty(),
            "the [autostart] emission drifted"
        );
        let defaults = Appearance::default();
        // Exact, not approximate: every appearance key is commented out, so
        // the file carries no color value at all and each field falls back
        // to its built-in -- including the three colors none of whose floats
        // is exactly representable in 8 bits. The nearest-hex approximation
        // only enters when a user uncomments a color line, which is disclosed
        // in the emission's own header comment.
        assert_eq!(
            loaded.appearance, defaults,
            "the [appearance] emission drifted from Appearance::default()"
        );
        // What must hold for those uncommented colors is the fixed point:
        // parsing an emitted hex and re-emitting it is byte-stable, so an
        // uncommented line never drifts a second time.
        for (name, built_in) in [
            ("focus_ring_active_color", defaults.focus_ring_active_color),
            (
                "focus_ring_inactive_color",
                defaults.focus_ring_inactive_color,
            ),
            ("background_color", defaults.background_color),
            ("cursor_color", defaults.cursor_color),
        ] {
            let parsed = Color::parse(&hex(built_in))
                .unwrap_or_else(|| panic!("the emission of {name} is not a parseable color"));
            assert_eq!(
                hex(parsed),
                hex(built_in),
                "{name} is not a fixed point: re-emitting the parsed color moves it"
            );
        }
    }

    /// The second cheap pin: every `[section]` the loader knows must appear
    /// in the output, so a newly added table cannot silently go un-emitted
    /// while the round-trip test above still passes on everything else.
    #[test]
    fn the_emitted_default_config_names_every_section_the_loader_knows() {
        let emitted = default_config_toml();
        for section in [
            "[layout]",
            "[appearance]",
            "[output]",
            "[renderer]",
            "[tty]",
            "[autostart]",
            "[binds]",
        ] {
            assert!(
                emitted.lines().any(|line| line.trim() == section),
                "the emission never names {section}"
            );
        }
    }

    /// The third pin, at key granularity for `[appearance]`: the round-trip
    /// test above compares the parsed emission against the live defaults,
    /// which a *missing* key also satisfies (a missing key falls back to its
    /// default) -- so an omitted key would pass it silently. Every appearance
    /// key must be named in the emission, commented out with its default.
    #[test]
    fn the_emitted_default_config_names_every_appearance_key() {
        let emitted = default_config_toml();
        for key in [
            "focus_ring_width",
            "focus_ring_active_color",
            "focus_ring_inactive_color",
            "background_color",
            "corner_radius",
            "cursor_size",
            "cursor_color",
            "cursor_theme",
            "prefer_no_csd",
        ] {
            assert!(
                emitted.lines().any(|line| {
                    let trimmed = line.trim();
                    trimmed.starts_with('#') && trimmed[1..].trim_start().starts_with(key)
                }),
                "the emission never names [appearance] {key}"
            );
        }
    }

    /// Byte-identical across runs: no timestamps, and no `HashMap` iteration
    /// order anywhere in the emission path (`[binds]` iterates the default
    /// table's hardcoded `Vec` order -- see `Keybindings::iter`). Humans
    /// diff this output; run-to-run noise would be `apply_binds`' collision
    /// rule applied to the project's own file.
    #[test]
    fn the_emitted_default_config_is_byte_identical_across_runs() {
        assert_eq!(default_config_toml(), default_config_toml());
    }

    /// The ticket's format model: every key present *and* commented, with
    /// its default as the value -- so the file as-is is exactly the
    /// defaults -- and every default bind spelled back in a form the loader
    /// accepts, resolving to the same combo and the same action.
    #[test]
    fn every_emitted_key_is_commented_and_every_default_bind_loads_back() {
        let emitted = default_config_toml();
        for line in emitted.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('[') {
                continue;
            }
            assert!(
                trimmed.starts_with('#'),
                "a live (uncommented) key line, which would stop being the defaults: {line}"
            );
        }

        let mut binds = 0;
        for (mods, keysym, bound) in Keybindings::default().iter() {
            let Bound::Action(action) = bound else {
                panic!("the defaults hold no session-managed binds to emit as comments");
            };
            let combo = combo_string(mods, keysym);
            let spelling = action_string(action);
            assert!(
                emitted
                    .lines()
                    .any(|line| line.trim() == format!("# \"{combo}\" = \"{spelling}\"")),
                "default bind {combo} = {spelling} is missing from the emission"
            );
            let (parsed_mods, parsed_keysym) =
                parse_combo(&combo).expect("an emitted combo must parse");
            assert_eq!(
                (parsed_mods, parsed_keysym),
                (mods, keysym),
                "emitted combo {combo} does not resolve back"
            );
            let mut tokens = spelling.split_whitespace().map(str::to_owned);
            let parsed = scootctl::action(&mut tokens).expect("an emitted action must parse");
            assert!(
                tokens.next().is_none(),
                "emitted action `{spelling}` leaves trailing text"
            );
            assert_eq!(
                Action::from(parsed),
                *action,
                "emitted action `{spelling}` does not resolve back"
            );
            binds += 1;
        }
        // "All 36 of them" (see docs/configuration.md): a dropped default
        // bind must fail loudly here, not just shrink the file.
        assert_eq!(binds, 36, "a default bind was added or lost");
    }

    /// Commented scalar values are pinned to their live defaults, not just
    /// to being commented: the round-trip test above passes on an
    /// all-commented file regardless of comment text (every `Option` is
    /// `None`), so without this a future edit could hardcode a stale
    /// `# gap = 8` while `Config::default().gap` moves and stay green.
    /// Every expectation below is derived from the live defaults -- never
    /// from the emitter -- so a hardcoded emission fails here.
    #[test]
    fn every_emitted_commented_scalar_names_its_live_default() {
        let emitted = default_config_toml();
        let config = Config::default();
        let appearance = Appearance::default();
        let widths: Vec<String> = config
            .column_widths
            .iter()
            .map(|w| format!("{w:?}"))
            .collect();
        for expected in [
            format!("# gap = {}", config.gap),
            format!("# column_widths = [{}]", widths.join(", ")),
            format!("# default_column_width = {}", config.default_column_width),
            format!("# focus_ring_width = {}", appearance.focus_ring_width),
            format!(
                "# focus_ring_active_color = \"{}\"",
                hex(appearance.focus_ring_active_color)
            ),
            format!(
                "# focus_ring_inactive_color = \"{}\"",
                hex(appearance.focus_ring_inactive_color)
            ),
            format!(
                "# background_color = \"{}\"",
                hex(appearance.background_color)
            ),
            format!("# cursor_size = {}", appearance.cursor_size),
            format!("# cursor_color = \"{}\"", hex(appearance.cursor_color)),
            format!("# prefer_no_csd = {}", appearance.prefer_no_csd),
            format!(
                "# backend = \"{}\"",
                crate::cli::RendererKind::default().as_str()
            ),
        ] {
            assert!(
                emitted.lines().any(|line| line.trim() == expected),
                "the emission no longer carries its live default: {expected}"
            );
        }
    }

    /// The 8-bit spelling's other half: a translucent color emits
    /// `"#rrggbbaa"` and parses back to itself, so the alpha arm of `hex`
    /// is pinned even though no default exercises it.
    #[test]
    fn a_translucent_color_emits_eight_digits_and_parses_back() {
        let color = Color::new(1.0, 0.5, 0.0, 0.5);
        let spelled = hex(color);
        assert_eq!(
            spelled.len(),
            9,
            "expected `#` plus eight digits: {spelled}"
        );
        let parsed = Color::parse(&spelled).expect("the emitted alpha form must parse");
        assert_eq!(hex(parsed), spelled, "the alpha form is not a fixed point");
    }

    // -- `--print-default-config --write` ------------------------------------

    /// The default write location is the same resolution the loader reads:
    /// with neither env var set there is no path at all, which is what
    /// [`write_default_config`] refuses as `NoBaseDir` rather than writing
    /// to the current directory.
    #[test]
    fn no_base_dir_resolves_to_no_path() {
        assert_eq!(default_path(None, None), None);
    }

    /// The ticket's first acceptance: an existing file is refused loudly --
    /// a `NoBaseDir`-shaped refusal naming the path, never a truncate --
    /// and its bytes are untouched.
    #[test]
    fn write_refuses_an_existing_file_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scoot/config.toml");
        fs::create_dir_all(path.parent().expect("a parent")).expect("parents");
        fs::write(&path, b"sentinel").expect("sentinel");
        let error = write_default_config_to(&path).expect_err("an existing file must be refused");
        assert!(
            matches!(error, WriteDefaultConfigError::Exists { .. }),
            "wrong refusal for an existing file: {error:?}"
        );
        assert!(
            error.to_string().contains(path.to_str().expect("utf-8")),
            "the refusal must name the path: {error}"
        );
        assert_eq!(
            fs::read(&path).expect("re-read"),
            b"sentinel",
            "a refused write touched the existing file"
        );
    }

    /// File-vs-stdout identity: the write path emits through the same
    /// [`default_config_toml`], so the anti-drift pins cover both by
    /// construction -- and this pins the construction.
    #[test]
    fn write_is_byte_identical_to_the_stdout_emission() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scoot/config.toml");
        write_default_config_to(&path).expect("a fresh path writes");
        assert_eq!(
            fs::read_to_string(&path).expect("re-read"),
            default_config_toml()
        );
    }

    /// Missing parents are created, like any tool writing its own config.
    #[test]
    fn write_creates_missing_parents() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("a/b/c/config.toml");
        assert!(!path.parent().expect("a parent").exists());
        write_default_config_to(&path).expect("missing parents are created");
        assert_eq!(
            fs::read_to_string(&path).expect("re-read"),
            default_config_toml()
        );
    }

    /// The created file is private to the user (`0o600`, matching the
    /// socket's posture -- the config may one day hold sensitive values).
    /// Set at creation, where no umask can add bits back.
    #[test]
    fn write_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scoot/config.toml");
        write_default_config_to(&path).expect("a fresh path writes");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the written config is not private: {mode:o}");
    }

    /// A symlink at the default location is refused *as the link* --
    /// `O_EXCL` applies before any traversal -- so neither a live target
    /// nor a dangling link is clobbered through this path.
    #[test]
    fn write_refuses_a_symlink_without_touching_its_target() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let target = dir.path().join("real.toml");
        fs::write(&target, b"sentinel").expect("target");
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = write_default_config_to(&link).expect_err("a symlink must be refused");
        assert!(
            matches!(error, WriteDefaultConfigError::Exists { .. }),
            "wrong refusal for a symlink: {error:?}"
        );
        assert_eq!(
            fs::read(&target).expect("re-read"),
            b"sentinel",
            "a refused write followed the symlink onto its target"
        );
        assert!(
            fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink(),
            "a refused write replaced the symlink"
        );

        // Dangling links refuse the same way: the refusal is about the link
        // itself existing, not about what it points at.
        let dangling = dir.path().join("dangling.toml");
        std::os::unix::fs::symlink(dir.path().join("nope.toml"), &dangling)
            .expect("dangling symlink");
        assert!(
            matches!(
                write_default_config_to(&dangling).expect_err("dangling refused"),
                WriteDefaultConfigError::Exists { .. }
            ),
            "a dangling symlink took a different refusal path"
        );
    }

    /// Two concurrent invocations leave exactly one winner and one refusal,
    /// never a torn file: the race is on the create-new itself (parents
    /// pre-created so `mkdir` is not what serializes them).
    #[test]
    fn concurrent_double_invoke_leaves_one_winner_and_an_intact_file() {
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scoot/config.toml");
        fs::create_dir_all(path.parent().expect("a parent")).expect("parents");
        let racers = 8;
        let barrier = Arc::new(Barrier::new(racers));
        let handles: Vec<_> = (0..racers)
            .map(|_| {
                let (barrier, path) = (Arc::clone(&barrier), path.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    write_default_config_to(&path).is_ok()
                })
            })
            .collect();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().expect("racer"))
            .filter(|won| *won)
            .count();
        assert_eq!(
            winners, 1,
            "concurrent invocations must leave exactly one winner, not {winners}"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("re-read"),
            default_config_toml(),
            "the winner's file is torn"
        );
    }

    /// An unwritable parent is a loud refusal naming the path, not a panic
    /// and not a file elsewhere.
    #[test]
    fn an_unwritable_parent_is_a_loud_refusal() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("a temp dir");
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).expect("locked dir");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock");
        let path = locked.join("scoot/config.toml");
        let error = write_default_config_to(&path).expect_err("an unwritable dir must be refused");
        assert!(
            error.to_string().contains(locked.to_str().expect("utf-8")),
            "the refusal must name the path: {error}"
        );
        assert!(!path.exists(), "a refused write left a file behind");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
    }

    /// A mid-write I/O failure (disk full, quota, ...) removes the partial
    /// file: a torn config at the default location would read back as
    /// malformed on the next startup, which is worse than no file at all.
    /// Proven through [`finish_new_file_write`] over a read-only handle,
    /// where every write deterministically fails the way ENOSPC would.
    #[test]
    fn a_mid_write_failure_removes_the_partial_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, b"partial").expect("partial");
        let read_only = fs::File::open(&path).expect("a read-only handle");
        let error = finish_new_file_write(read_only, &path, b"more bytes")
            .expect_err("a failed write must be an error");
        assert!(
            matches!(error, WriteDefaultConfigError::Write { .. }),
            "wrong error for a failed write: {error:?}"
        );
        assert!(!path.exists(), "a failed write left a partial file behind");
    }

    /// Emits-then-loads: the written file parses back through the loader to
    /// the live defaults, closing the ticket's loop end to end.
    #[test]
    fn a_written_file_loads_back_to_the_live_defaults() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scoot/config.toml");
        write_default_config_to(&path).expect("a fresh path writes");
        let loaded = load(Some(&path)).expect("the written file must load");
        assert_eq!(
            loaded.config,
            Config::default(),
            "the written [layout] drifted from Config::default()"
        );
        assert!(
            loaded.keybindings.same_bindings_as(&Keybindings::default()),
            "the written [binds] drifted from Keybindings::default()"
        );
        assert_eq!(loaded.scale, 1.0, "the written [output] drifted");
        assert_eq!(loaded.gpu, None, "the written [tty] drifted");
        assert_eq!(loaded.renderer, None, "the written [renderer] drifted");
        assert!(
            loaded.autostart.is_empty(),
            "the written [autostart] drifted"
        );
        assert_eq!(
            loaded.appearance,
            Appearance::default(),
            "the written [appearance] drifted from Appearance::default()"
        );
    }
}
