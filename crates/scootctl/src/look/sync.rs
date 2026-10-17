//! `scoot look sync`: the home-manager rule (design section 6.3).
//!
//! Home-manager owns the catalog and the declared default; the live choice
//! is state it does not own. On every activation, under the lock, in
//! order:
//!
//! 1. No state yet → start on the declared default.
//! 2. The declared default changed since the live choice was recorded →
//!    the declaration wins: switch to the new default.
//! 3. The live choice is no longer in the catalog → fall back to the
//!    declared default, and say so.
//! 4. Otherwise → keep the live choice; re-resolve it through the new
//!    catalog, so an updated look's new contents apply.
//!
//! The same name with changed contents applies the new contents (rule 4);
//! `look = null` removes the layer (rule 2); a rollback runs the same
//! rule against the older catalog and default. With no session running,
//! sync moves the link and stops (unanswered peers are skipped, never a
//! refusal).
//!
//! The declared default lives beside the catalog links: home-manager
//! writes `<config>/scoot/look-default` (the name, or an empty file for
//! null), the NixOS module `/etc/scoot/look-default`. A missing file is
//! "not Nix-managed": sync leaves the live choice alone and exits 0, so
//! a non-Nix box that runs sync by hand loses nothing. The module records
//! the `org/gnome/desktop/interface` key names its merged
//! `dconf.settings` still carry (set by the user or another module, since
//! scoot defines none) in `look-nix-claims` beside it, and the toolkit
//! step skips those however the live value reads.
//!
//! A declared look renders at build time into the catalog's
//! `rendered/<name>`; sync points the state link at that entry (never a
//! raw store path, so the next generation re-resolves it), verifying
//! name, source hash and the fragments first and rendering fresh into
//! the cache with a note when verification fails. Anything else (a kept
//! user look, a hand-placed `look-default` on a non-Nix box) renders into
//! the cache exactly like `apply`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::grammar::is_name;
use scoot_look::source_hash;
use scoot_look::state::{RecordRead, acquire_lock, ensure_link, read_capped_follow, read_record};

use super::apply::{
    ApplyFailure, ApplyReport, Availability, Checks, RenderSource, SwapRequest, SwapTarget,
    Validation, absolutize, check_inputs, collect_chain, complete_swap, render_for_swap,
    unknown_look,
};
use super::reload::{BarReload, CompositorReload};
use super::toolkit::Dconf;
use scoot_look::catalog::{Catalog, config_home, data_home};

/// Options for one `sync`, from flags.
pub struct SyncOptions {
    /// How long to wait on the lock.
    pub lock_wait: Duration,
    /// Skip every dconf write (`--no-toolkit`).
    pub no_toolkit: bool,
}

/// The declared default and the Nix-owned dconf keys, as read.
#[derive(Debug)]
pub struct Declared {
    /// The declared default: `None` for null (no look layer).
    pub default: Option<String>,
    /// Full dconf key paths home-manager owns (one per line in the
    /// claims file).
    pub claims: Vec<String>,
}

/// The `look-default` file name, in `<config>/scoot` or `/etc/scoot`.
pub const DEFAULT_FILE: &str = "look-default";
/// The Nix-owned dconf key names beside it, one full path per line.
pub const CLAIMS_FILE: &str = "look-nix-claims";
/// The Nix theme targets opted out beside it, one name per line
/// (`cursor`, `fonts`): written by the home-manager module (or the NixOS
/// module system-wide) from `desktop.theme.targets.*.enable`, read here
/// so the managed sync honors the opt-outs the eval-time theme already
/// honors. A hand-written file works the same: no Nix is needed to
/// leave the cursor or the fonts alone.
pub const TARGETS_FILE: &str = "look-nix-targets";
/// The Stylix targets owning a path scoot also themes, one app per line
/// (`gtk`, `qt`, `foot`, `mako`, `fuzzel`, `swaylock`, `wob`,
/// `fcitx5`, `starship`, `helix`, `btop`): written by the
/// home-manager module from `stylix.targets.*.enable`, read here so
/// the apply reply names those apps as owned by Stylix's target
/// instead of pretending the look reached them.
pub const STYLIX_TARGETS_FILE: &str = "look-stylix-targets";
/// The NixOS-module-only declared default.
pub const SYSTEM_DEFAULT_FILE: &str = "/etc/scoot/look-default";
/// The NixOS-module-only claims file.
pub const SYSTEM_CLAIMS_FILE: &str = "/etc/scoot/look-nix-claims";
/// The NixOS-module-only opted-out targets.
pub const SYSTEM_TARGETS_FILE: &str = "/etc/scoot/look-nix-targets";
/// The NixOS-module-only Stylix-owned apps.
pub const SYSTEM_STYLIX_TARGETS_FILE: &str = "/etc/scoot/look-stylix-targets";

/// Which theme targets a sync leaves alone: the Nix `cursor` and
/// `fonts` opt-outs (`desktop.theme.targets.*.enable = false`), read
/// from [`TARGETS_FILE`]. A skipped cursor renders no cursor keys into
/// the scoot fragment and writes no cursor dconf keys or settings.ini
/// cursor lines; skipped fonts render no bar `font` key, resolve no
/// font, and write no `font-name` key, settings.ini font line or
/// fontconfig file. Everything else themes as usual.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetSkips {
    /// The cursor target is off: leave the cursor alone entirely.
    pub cursor: bool,
    /// The fonts target is off: keep the box's own fonts (the bar
    /// falls back to its well-known font paths).
    pub fonts: bool,
}

impl TargetSkips {
    /// Parses the opted-out target names, one per line. Blank lines
    /// and unknown names are ignored (a newer writer's target never
    /// refuses an older sync).
    pub fn from_lines(lines: &[String]) -> Self {
        let mut skips = TargetSkips::default();
        for line in lines {
            match line.trim() {
                "cursor" => skips.cursor = true,
                "fonts" => skips.fonts = true,
                _ => {}
            }
        }
        skips
    }

    /// Whether any target is skipped.
    pub fn any(self) -> bool {
        self.cursor || self.fonts
    }
}

/// Reads the declared default and the claims: the config-home file when
/// it exists, else the system one, else unmanaged (`None`). A present
/// but unreadable file, or a name outside the look grammar, refuses
/// loudly: sync's whole job is that file.
pub fn read_declared() -> Result<Option<Declared>, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    let user_default = config_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot").join(DEFAULT_FILE));
    let user_claims = config_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot").join(CLAIMS_FILE));
    let user = user_default
        .zip(user_claims)
        .filter(|(default, _)| default.is_file());
    if let Some((default, claims)) = user {
        return parse_declared(&default, &claims);
    }
    let system_default = PathBuf::from(SYSTEM_DEFAULT_FILE);
    if system_default.is_file() {
        return parse_declared(&system_default, Path::new(SYSTEM_CLAIMS_FILE));
    }
    Ok(None)
}

/// Parses one `look-default` + claims pair. Pure over the paths, so tests
/// pin the files without touching the process environment. A missing
/// default is unmanaged (`None`); a present but unreadable one, or a
/// name outside the look grammar, refuses loudly.
pub fn parse_declared(default: &Path, claims: &Path) -> Result<Option<Declared>, String> {
    if !default.is_file() {
        return Ok(None);
    }
    Ok(Some(Declared {
        default: read_default(default)?,
        claims: read_claims(claims),
    }))
}

/// Reads just the Nix-owned key paths (for `apply` and `toolkit`, which
/// own no rule but honor the claims): best-effort, empty on any failure.
pub fn read_nix_claims() -> Vec<String> {
    read_declared()
        .map(|declared| declared.map(|d| d.claims).unwrap_or_default())
        .unwrap_or_default()
}

/// Reads the Stylix-owned app names (for `apply`, which names them in
/// the reply instead of pretending the look reached them): the
/// config-home file when it exists, else the system one, else none.
/// Best-effort, like the claims: a missing or unreadable file owns
/// nothing, never a refusal. Blank lines are kept for the caller to
/// ignore; names are matched verbatim.
pub fn read_stylix_targets() -> Vec<String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    let user = config_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot").join(STYLIX_TARGETS_FILE));
    parse_stylix_targets(user.as_deref(), &PathBuf::from(SYSTEM_STYLIX_TARGETS_FILE))
}

/// Picks the Stylix-owned app names: the config-home file when it
/// exists, else the system one, else none. Pure over the paths, so
/// tests pin the precedence without touching the process
/// environment.
fn parse_stylix_targets(user: Option<&Path>, system: &Path) -> Vec<String> {
    if let Some(path) = user
        && path.is_file()
    {
        return read_target_lines(path);
    }
    if system.is_file() {
        return read_target_lines(system);
    }
    Vec::new()
}

/// Reads the opted-out theme targets (for `sync`, which honors them):
/// the config-home file when it exists, else the system one, else no
/// skips. Best-effort, like the claims: a missing or unreadable file
/// themes everything, never a refusal.
pub fn read_nix_targets() -> TargetSkips {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    let user = config_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot").join(TARGETS_FILE));
    if let Some(path) = user
        && path.is_file()
    {
        return TargetSkips::from_lines(&read_target_lines(&path));
    }
    let system = PathBuf::from(SYSTEM_TARGETS_FILE);
    if system.is_file() {
        return TargetSkips::from_lines(&read_target_lines(&system));
    }
    TargetSkips::default()
}

/// Reads one targets file: names, one per line, blanks kept for the
/// parser to ignore. Missing or unreadable is no skips, never a refusal.
fn read_target_lines(path: &Path) -> Vec<String> {
    let Ok(text) = read_capped_follow(path) else {
        return Vec::new();
    };
    text.lines().map(str::to_owned).collect()
}

/// Reads one `look-default` file: the trimmed first line is the name, an
/// empty file is null. Anything else refuses, naming the file.
fn read_default(path: &Path) -> Result<Option<String>, String> {
    let text = read_capped_follow(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let first = text.lines().next().unwrap_or_default().trim();
    if first.is_empty() {
        return Ok(None);
    }
    if !is_name(first) {
        return Err(format!(
            "the declared default in {} is not a look name ([a-z0-9-]{{1,32}}): `{first}`",
            path.display()
        ));
    }
    Ok(Some(first.to_owned()))
}

/// Reads one claims file: full dconf paths, one per line, blanks
/// skipped. Missing or unreadable is no claims, never a refusal.
fn read_claims(path: &Path) -> Vec<String> {
    let Ok(text) = read_capped_follow(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The catalog's rendered roots, in catalog order: the home-manager
/// catalog beside `~/.local/share/scoot/looks`, then the system one.
/// `$XDG_DATA_HOME` does not move the declared catalog (the discovery
/// pins it to `~/.local/share`), so it moves no rendered root either.
pub fn rendered_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        roots.push(home.join(".local/share/scoot/rendered"));
    } else if let Some(data) = data_home(
        std::env::var_os("XDG_DATA_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    ) {
        roots.push(data.join("scoot/rendered"));
    }
    roots.push(PathBuf::from("/run/current-system/sw/share/scoot/rendered"));
    roots
}

/// The catalog's pre-rendered directory for `name`: the first rendered
/// root holding it, verified (name, source hash and both fragments).
/// Returns the directory and, when a candidate failed verification, the
/// note for the fresh-render fallback: a stale catalog render re-renders,
/// never refuses (the sources validate separately).
pub fn prerendered_for(
    name: &str,
    source_hash_hex: &str,
    roots: &[PathBuf],
) -> (Option<PathBuf>, Option<String>) {
    let mut note = None;
    for root in roots {
        let dir = root.join(name);
        if !dir.is_dir() {
            continue;
        }
        match verify_prerendered(name, source_hash_hex, &dir) {
            Ok(()) => return (Some(dir), note),
            Err(fallback) => note = Some(fallback),
        }
    }
    (None, note)
}

/// Verifies one pre-rendered directory: `render.json` names this look
/// with this source hash, and the fragments exist (`scoot.toml`,
/// `bar.toml` and the greeter's `greet.toml`: the state link points at
/// this directory, so an incomplete catalog render re-renders fresh).
/// Returns the note for the fresh-render fallback when it does not.
fn verify_prerendered(name: &str, source_hash_hex: &str, dir: &Path) -> Result<(), String> {
    let refused = |why: String| {
        format!(
            "catalog render at {}: {why}: rendering fresh",
            dir.display()
        )
    };
    let text = std::fs::read_to_string(dir.join("render.json"))
        .map_err(|error| refused(format!("cannot read render.json: {error}")))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| refused(format!("render.json: {error}")))?;
    if json.get("name").and_then(|n| n.as_str()) != Some(name) {
        return Err(refused("render.json names another look".to_owned()));
    }
    if json.get("source_hash").and_then(|h| h.as_str()) != Some(source_hash_hex) {
        return Err(refused("render.json source hash differs".to_owned()));
    }
    for file in ["scoot.toml", "bar.toml", "greet.toml"] {
        if !dir.join(file).is_file() {
            return Err(refused(format!("{file} is missing")));
        }
    }
    Ok(())
}

/// [`Checks`] with every font resolution answered "unchecked": the
/// fonts target is off ("keep your own fonts"), so no face is resolved
/// (a missing face must never refuse a sync that themes no font), no
/// UI font feeds the render (the bar fragment carries no `font` key),
/// and the notes say why. Wallpaper, bar and foot checks run
/// unchanged.
struct SkipFontsChecks<'a> {
    inner: &'a dyn Checks,
}

impl Checks for SkipFontsChecks<'_> {
    fn font_file(&self, family: &str) -> Result<Availability<String>, String> {
        Ok(Availability::Unavailable(format!(
            "font `{family}` unchecked: the fonts target is off"
        )))
    }

    fn wallpaper(&self, path: &Path) -> Result<Availability<()>, String> {
        self.inner.wallpaper(path)
    }

    fn bar_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        self.inner.bar_fragment(fragment)
    }

    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        self.inner.foot_fragment(fragment)
    }
}

/// Runs the rule: re-resolve the live choice or switch to the declared
/// default, then swap exactly like `apply` (record, repoint, scoped
/// reloads, rollback, shadowed keys). Pure over its `catalog`, `cache`,
/// `rendered_roots`, `state_dir` and peers except the lock, the record,
/// the link, the cache and the reloads themselves.
///
/// `declared` is the file just read: `None` unmanaged (leave the choice
/// alone), `Some(None)` a null default (remove the layer),
/// `Some(Some(name))` the declared look. `target_skips` carries the
/// opted-out theme targets from [`TARGETS_FILE`]: whatever sync points
/// at renders without the skipped pieces, so an explicit Nix opt-out
/// holds through the managed sync (a manual `apply` still themes
/// everything: it is the live choice, not the declaration).
#[allow(clippy::too_many_arguments)]
pub fn sync_with(
    declared: Option<Option<String>>,
    nix_claims: Vec<String>,
    target_skips: TargetSkips,
    stylix_targets: Vec<String>,
    options: &SyncOptions,
    catalog: &Catalog,
    cache: &Path,
    roots: &[PathBuf],
    state_dir: &Path,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn Checks,
    dconf: &dyn Dconf,
    leaves: &dyn super::leaves::Leaves,
    live: &[PathBuf],
) -> Result<ApplyReport, ApplyFailure> {
    let refuse = |message: String| ApplyFailure {
        message,
        rolled_back_to: None,
        rollback_errors: Vec::new(),
    };
    if let Some(Some(name)) = &declared
        && !is_name(name)
    {
        return Err(refuse(format!(
            "`{name}` is not a look name ([a-z0-9-]{{1,32}})"
        )));
    }
    // Work on absolute dirs throughout, like `apply`: a relative render
    // path fails the next record read as corrupt.
    let state_dir = absolutize(state_dir).map_err(refuse)?;
    let state_dir = state_dir.as_path();
    let cache = absolutize(cache).map_err(refuse)?;
    let cache = cache.as_path();
    std::fs::create_dir_all(state_dir).map_err(|error| {
        refuse(format!(
            "cannot use state dir {}: {error}",
            state_dir.display()
        ))
    })?;
    let _lock = acquire_lock(state_dir, options.lock_wait).map_err(refuse)?;
    let mut report = ApplyReport::default();
    match ensure_link(state_dir).map_err(refuse)? {
        scoot_look::state::LinkRepair::Healed(message) => report.notes.push(message),
        scoot_look::state::LinkRepair::Skipped(message) => {
            if !matches!(read_record(state_dir), RecordRead::Missing) {
                report.notes.push(message);
            }
        }
        scoot_look::state::LinkRepair::Consistent => {}
    }
    let previous = match read_record(state_dir) {
        RecordRead::Present(record) => Some(record),
        RecordRead::Missing => None,
        RecordRead::Corrupt(reason) => {
            return Err(refuse(format!(
                "cannot sync: the recorded choice is corrupt ({reason}): fix or remove {}",
                state_dir.join("look.json").display()
            )));
        }
    };
    let declared_default: Option<String> = declared.clone().flatten();
    // Unmanaged (no `look-default` file anywhere): this box is not
    // Nix-managed for looks, so the live choice is left alone — a
    // non-Nix box that runs sync by hand loses nothing, whatever the
    // catalog holds.
    if declared.is_none() {
        match &previous {
            Some(record) => {
                report.notes.push(format!(
                    "no declared default: keeping the live choice `{}`",
                    record.name
                ));
                report.name = record.name.clone();
                report.rendered = record.rendered.clone();
                report.generation = record.generation;
                return Ok(report);
            }
            None => {
                report
                    .notes
                    .push("no declared default and no recorded choice: nothing to do".to_owned());
                return Ok(report);
            }
        }
    }
    // The rule, in order. A choice recorded before any sync
    // (`declared_default` none) meets a managed declaration — even a
    // null one — as a change (rule 2): flattening would collapse "never
    // recorded" and "recorded null" and keep a stale layer. Rule 3's
    // fallback says so; every branch names the reason it picked this
    // target.
    let declared_changed = match (&previous, &declared) {
        (Some(record), Some(_)) => match &record.declared_default {
            None => true,
            Some(prev) => Some(prev.clone()) != declared.clone().flatten(),
        },
        _ => false,
    };
    let desired: Option<String> = match &previous {
        None => match &declared_default {
            Some(name) => {
                report.notes.push(format!(
                    "no recorded choice: starting on the declared default `{name}`"
                ));
                declared_default.clone()
            }
            None => {
                report.notes.push(
                    "the declared default is null and no choice is recorded: nothing to do"
                        .to_owned(),
                );
                return Ok(report);
            }
        },
        Some(_) if declared_changed => match &declared_default {
            Some(name) => {
                report.notes.push(format!(
                    "the declared default changed to `{name}`: the declaration wins"
                ));
                declared_default.clone()
            }
            None => {
                report
                    .notes
                    .push("the declared default is null: removing the layer".to_owned());
                None
            }
        },
        Some(record) if catalog.find(&record.name).is_none() => match &declared_default {
            Some(name) => {
                report.notes.push(format!(
                    "the live choice `{}` is no longer in the catalog: falling back to `{name}`",
                    record.name
                ));
                declared_default.clone()
            }
            None => {
                report.notes.push(format!(
                    "the live choice `{}` is no longer in the catalog: removing the layer",
                    record.name
                ));
                None
            }
        },
        Some(record) => {
            report.notes.push(format!(
                "keeping the live choice `{}`: re-resolving it through the new catalog",
                record.name
            ));
            Some(record.name.clone())
        }
    };
    let Some(name) = desired else {
        // A null default with a live choice: remove the layer, rolling
        // back to the choice on a refusal like any other swap. With no
        // previous choice every null branch returns above; the guard
        // below is dead-code insurance, not a path.
        match previous {
            Some(previous) => {
                let swap = SwapRequest {
                    name: previous.name.clone(),
                    target: SwapTarget::Remove,
                    chosen_by: "sync".to_owned(),
                    declared_default: declared_default.clone(),
                    no_toolkit: options.no_toolkit,
                    no_helix: false,
                    no_btop: false,
                    nix_claims,
                    stylix_targets: stylix_targets.clone(),
                    target_skips,
                };
                complete_swap(
                    &mut report,
                    state_dir,
                    cache,
                    Some(&previous),
                    &swap,
                    None,
                    config,
                    compositor,
                    bars,
                    dconf,
                    leaves,
                    live,
                )?;
                return Ok(report);
            }
            None => {
                report
                    .notes
                    .push("the declared default is null: nothing to do".to_owned());
                return Ok(report);
            }
        }
    };
    let entry = catalog.find(&name).ok_or_else(|| {
        if Some(name.as_str()) == declared_default.as_deref() {
            refuse(format!(
                "the declared default `{name}` is not in the catalog ({})",
                catalog_names(catalog)
            ))
        } else {
            refuse(unknown_look(&name, catalog))
        }
    })?;
    let chain = collect_chain(catalog, &name).map_err(refuse)?;
    let merged = scoot_look::resolve(&catalog.sources(), &name)
        .map_err(|error| refuse(format!("cannot resolve `{name}`: {error}")))?;
    let contrast = scoot_look::check_contrast(&merged, entry.origin);
    let mut contrast_errors = Vec::new();
    for refusal in contrast.refusals() {
        contrast_errors.push(format!(
            "{} is {} (needs {})",
            refusal.name, refusal.ratio, refusal.threshold
        ));
    }
    if !contrast_errors.is_empty() {
        return Err(refuse(format!(
            "cannot sync `{name}`: {}",
            contrast_errors.join("; ")
        )));
    }
    for warning in contrast.warnings() {
        report.notes.push(format!(
            "contrast: {} is {} (needs {})",
            warning.name, warning.ratio, warning.threshold
        ));
    }
    let mut validation = Validation::default();
    // The fonts target is off: resolve no face at all (a missing face
    // must never refuse a sync that themes no font). The wrapper only
    // shapes this call: the catalog, the record and the peers see the
    // same values either way.
    let skip_fonts = SkipFontsChecks { inner: checks };
    let checks: &dyn Checks = if target_skips.fonts {
        &skip_fonts
    } else {
        checks
    };
    let inputs = match check_inputs(&chain, &merged, checks, &mut validation) {
        Ok(inputs) => inputs,
        Err(_) => {
            let detail = validation.errors.join("; ");
            let detail = if detail.trim().is_empty() {
                "invalid look (no detail)".to_owned()
            } else {
                detail
            };
            return Err(refuse(format!("cannot sync `{name}`: {detail}")));
        }
    };
    let mut inputs = inputs;
    // The cursor target is off: the fragment carries no cursor keys, so
    // the look layer leaves the compositor's own cursor alone. The flag
    // rides the cache key, so the skipped render never collides with
    // the full one.
    if target_skips.cursor {
        inputs.inputs.omit_cursor = true;
        report
            .notes
            .push("the cursor target is off: rendering without the cursor keys".to_owned());
    }
    if target_skips.fonts {
        report.notes.push(
            "the fonts target is off: resolving no font, rendering without the bar font".to_owned(),
        );
    }
    report.notes.extend(validation.notes);
    let target_bytes = chain
        .last()
        .map(|link| link.bytes.as_slice())
        .unwrap_or_default();
    let hash = source_hash(target_bytes);
    // A catalog render the derivation built, verified before use;
    // anything else (a kept user look, no Nix catalog) renders fresh.
    // An opted-out target always renders fresh: the catalog's
    // pre-rendered directory carries the cursor keys and the bar font
    // unconditionally, so pointing the link at it would theme exactly
    // what the target leaves alone.
    let (prerendered, fallback_note) = if target_skips.any() {
        (None, None)
    } else {
        prerendered_for(&name, &hash, roots)
    };
    if let Some(note) = fallback_note {
        report.notes.push(note);
    }
    let source = match prerendered {
        Some(path) => {
            report
                .notes
                .push(format!("using the catalog render at {}", path.display()));
            RenderSource::PreRendered { path }
        }
        None => RenderSource::Fresh,
    };
    report.notes.extend(inputs.details);
    let (target, _) = render_for_swap(
        &name,
        &chain,
        &merged,
        inputs.inputs,
        source,
        cache,
        checks,
        &mut report.notes,
        false,
    )
    .map_err(refuse)?;
    // Already there: the same name, the same declared default, and the
    // resolved render already under the link — update nothing (in
    // particular, run no reload: an unchanged generation re-resolves to
    // the same store paths, so path equality here means byte-identical).
    if let Some(previous) = &previous
        && previous.name == name
        && previous.declared_default == declared_default
        && resolved_same(&previous.rendered, &target)
    {
        report
            .notes
            .push(format!("`{name}` is already active: no reload"));
        report.name = name.clone();
        report.rendered = previous.rendered.clone();
        report.generation = previous.generation;
        return Ok(report);
    }
    let swap = SwapRequest {
        name: name.clone(),
        target: SwapTarget::Point(target),
        chosen_by: "sync".to_owned(),
        declared_default: declared_default.clone(),
        no_toolkit: options.no_toolkit,
        no_helix: false,
        no_btop: false,
        nix_claims,
        stylix_targets,
        target_skips,
    };
    complete_swap(
        &mut report,
        state_dir,
        cache,
        previous.as_ref(),
        &swap,
        Some(&merged),
        config,
        compositor,
        bars,
        dconf,
        leaves,
        live,
    )?;
    Ok(report)
}

/// Whether the previous render already resolves to this swap's render.
/// Canonicalized on both sides, so a catalog path and its store target
/// compare equal; an unresolvable side compares unequal (a garbage-
/// collected generation re-renders instead of matching a ghost).
fn resolved_same(previous: &str, target: &Path) -> bool {
    std::fs::canonicalize(previous)
        .and_then(|a| std::fs::canonicalize(target).map(|b| a == b))
        .unwrap_or(false)
}

/// The catalog's names, for the unknown-declared refusal.
fn catalog_names(catalog: &Catalog) -> String {
    let mut names: Vec<&str> = catalog
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    names.sort();
    if names.is_empty() {
        return "the catalog is empty".to_owned();
    }
    format!("available: {}", names.join(", "))
}

#[cfg(test)]
mod tests;
