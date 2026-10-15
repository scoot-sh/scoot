//! Rendering, the render cache, the effectful checks, and the swap itself.
//!
//! The swap is the design's section 4.5 under one `flock`: render and
//! validate, write `look.json` then the link (repairing the link from
//! `look.json` whenever they disagree), scoped reloads in order (the
//! compositor, then every bar of the session), skip what is not running,
//! roll back on a refusal, and list shadowed keys. The render cache is
//! pruned under the same lock (the active, the previous, the four most
//! recent; never a directory named `look.json` or `look`). An unwritable
//! state dir refuses loudly, before anything is touched.
//!
//! The reload and settings peers are traits ([`CompositorReload`],
//! [`BarReload`], [`Dconf`], [`Checks`]), so the transaction unit-tests
//! against scripted peers: atomicity, refuse-and-keep, rollback,
//! rollback-with-errors, the lock, link repair, a missing bar, shadowed
//! keys, and a pending user edit never applied by a swap.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::grammar::is_name;
use scoot_look::state::{
    LINK_NAME, Record, RecordRead, acquire_lock, ensure_link, read_capped, read_record,
    state_dir_from_env, write_record,
};
use scoot_look::{
    Origin, RawLook, RenderInputs, check_contrast, render_bar, render_json, render_scoot, resolve,
    source_hash,
};

use super::catalog::{Catalog, Entry, cache_home, config_home};
use super::reload::{BarReload, CompositorReload, ReloadError};
use super::toolkit::{Dconf, apply_toolkit};

/// How long a second writer waits on `<state>/look.lock` before refusing.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);

/// The largest wallpaper image accepted without `scootbg`: the same 32 MiB
/// as scootbg's own download cap (`crates/scootbg/src/fetch.rs`
/// `MAX_BYTES`). With `scootbg` present `check-image` enforces its own
/// byte cap, the pixel cap from the header, and a full decode instead.
pub const MAX_IMAGE_BYTES: u64 = 32 << 20;

/// Options for one `apply`, from flags and the environment.
pub struct ApplyOptions {
    /// How long to wait on the lock.
    pub lock_wait: Duration,
    /// Skip every dconf write (`--no-toolkit`).
    pub no_toolkit: bool,
}

/// What one `apply` did.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// The look now active.
    pub name: String,
    /// The rendered directory the link points at.
    pub rendered: String,
    /// The record generation the swap wrote.
    pub generation: u64,
    /// Apps reloaded, in order (`compositor`, `bar <display>`, ...).
    pub reloaded: Vec<String>,
    /// Apps not running, toolkit notes, anything skipped.
    pub skipped: Vec<String>,
    /// Notes: repaired link, warnings, adopted keys.
    pub notes: Vec<String>,
    /// User-file keys that override a look token.
    pub shadowed: Vec<String>,
    /// User-file keys that failed to parse and fell back to the look.
    pub invalid: Vec<String>,
}

/// Why an `apply` did not swap: the refusal, and what the rollback did.
#[derive(Debug)]
pub struct ApplyFailure {
    /// The refusal itself.
    pub message: String,
    /// The look restored, if the swap had repointed before refusing.
    pub rolled_back_to: Option<String>,
    /// Refusals from the rollback reloads, if those refused too.
    pub rollback_errors: Vec<String>,
}

impl std::fmt::Display for ApplyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(previous) = &self.rolled_back_to {
            write!(f, "; rolled back to {previous}")?;
        }
        if !self.rollback_errors.is_empty() {
            write!(
                f,
                "; ROLLED BACK WITH ERRORS: {}",
                self.rollback_errors.join("; ")
            )?;
        }
        Ok(())
    }
}

/// A font family, the wallpaper, or the bar fragment: resolved, backend
/// missing (a note, never a refusal), or refused.
pub enum Availability<T> {
    Available(T),
    Unavailable(String),
}

/// The effectful checks the pure crate leaves to the tool: fonts through
/// their carrier, the wallpaper through `scootbg check-image`, the bar
/// fragment through `scootbar daemon --check`.
pub trait Checks {
    /// Resolves a font family to a file through fontconfig. A missing
    /// `fc-match` is [`Availability::Unavailable`]; a failing one refuses.
    fn font_file(&self, family: &str) -> Result<Availability<String>, String>;
    /// Checks an image wallpaper: `scootbg check-image` when the binary is
    /// around (byte cap, pixel cap from the header, full decode), else the
    /// byte cap alone with a note that no decode ran.
    fn wallpaper(&self, path: &Path) -> Result<Availability<()>, String>;
    /// Validates a rendered `bar.toml` with the installed binary's own
    /// parser (`scootbar daemon --check --config`). A missing binary is
    /// [`Availability::Unavailable`]; a failing check refuses.
    fn bar_fragment(&self, fragment: &Path) -> Result<Availability<()>, String>;
}

/// What `validate` found.
#[derive(Debug, Default)]
pub struct Validation {
    /// Refusals: the look does not apply.
    pub errors: Vec<String>,
    /// Contrast shortfalls on warn-origins, and other non-fatal reports.
    pub warnings: Vec<String>,
    /// Effectful skips (a missing backend) and repairs.
    pub notes: Vec<String>,
}

/// Validates a catalog look without touching anything: resolve, contrast,
/// and the effectful checks.
pub fn validate_entry(
    catalog: &Catalog,
    name: &str,
    checks: &dyn Checks,
) -> Result<Validation, String> {
    let entry = catalog
        .find(name)
        .ok_or_else(|| unknown_look(name, catalog))?;
    let chain = collect_chain(catalog, name)?;
    let merged = resolve(&catalog.sources(), name)
        .map_err(|error| format!("cannot resolve `{name}`: {error}"))?;
    let mut validation = Validation::default();
    let report = check_contrast(&merged, entry.origin);
    for refusal in report.refusals() {
        validation.errors.push(format!(
            "{} is {} (needs {})",
            refusal.name, refusal.ratio, refusal.threshold
        ));
    }
    for warning in report.warnings() {
        validation.warnings.push(format!(
            "{} is {} (needs {})",
            warning.name, warning.ratio, warning.threshold
        ));
    }
    check_inputs(&chain, &merged, checks, &mut validation)?;
    if !validation.errors.is_empty() {
        return Ok(validation);
    }
    Ok(validation)
}

/// Validates a source directory without touching anything. A directory with
/// `extends` resolves its base through the catalog; without one it resolves
/// alone, like `scoot-look build`.
pub fn validate_dir(
    dir: &Path,
    origin: Origin,
    catalog: &Catalog,
    checks: &dyn Checks,
) -> Result<Validation, String> {
    use scoot_look::{parse_look_toml, validate_apps_allowlist, validate_for_origin};
    let bytes = read_regular(&dir.join("look.toml"))?;
    let raw = parse_look_toml(&bytes).map_err(|errors| {
        errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let canonical = std::fs::canonicalize(dir)
        .map_err(|error| format!("cannot resolve {}: {error}", dir.display()))?;
    let dir_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if let Some(error) = scoot_look::check_name_matches_dir(&raw, dir_name) {
        return Err(error.to_string());
    }
    let mut validation = Validation::default();
    for error in validate_for_origin(&raw, origin)
        .into_iter()
        .chain(validate_apps_allowlist(&raw.apps))
    {
        validation.errors.push(error.to_string());
    }
    if !validation.errors.is_empty() {
        return Ok(validation);
    }
    let merged = match &raw.extends {
        Some(_) => {
            let mut sources = catalog.sources();
            sources.insert(raw.name.clone(), (raw.clone(), origin));
            resolve(&sources, &raw.name)
                .map_err(|error| format!("cannot resolve `{}`: {error}", raw.name))?
        }
        None => {
            let sources = BTreeMap::from([(raw.name.clone(), (raw.clone(), origin))]);
            resolve(&sources, &raw.name)
                .map_err(|error| format!("cannot resolve `{}`: {error}", raw.name))?
        }
    };
    let report = check_contrast(&merged, origin);
    for refusal in report.refusals() {
        validation.errors.push(format!(
            "{} is {} (needs {})",
            refusal.name, refusal.ratio, refusal.threshold
        ));
    }
    for warning in report.warnings() {
        validation.warnings.push(format!(
            "{} is {} (needs {})",
            warning.name, warning.ratio, warning.threshold
        ));
    }
    // The effectful checks run against the directory as given (a catalog
    // entry only when the name resolves to one).
    let chain = match catalog.find(&raw.name) {
        Some(entry) => collect_chain(catalog, &raw.name).unwrap_or_else(|_| {
            vec![ChainEntry {
                entry: entry.clone(),
                bytes: bytes.clone(),
            }]
        }),
        None => vec![ChainEntry {
            entry: Entry {
                name: raw.name.clone(),
                dir: canonical,
                origin,
                raw: raw.clone(),
                bytes: bytes.clone(),
            },
            bytes,
        }],
    };
    check_inputs(&chain, &merged, checks, &mut validation)?;
    Ok(validation)
}

/// Reads one file, refusing anything but a regular file: symlinks (which
/// could point anywhere outside the look), directories, devices, FIFOs and
/// sockets. The pre-check is `symlink_metadata`, which never follows the
/// path, and the open itself passes `O_NONBLOCK | O_NOFOLLOW` and re-checks
/// the opened handle, so a FIFO swapped in between the stat and the open
/// refuses instead of hanging, and a symlink raced into the same window
/// fails with `ELOOP` and refuses.
pub fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    let refusal = |why: &str| format!("refused: {} is {why}", path.display());
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    if meta.is_symlink() {
        return Err(refusal("a symlink"));
    }
    if !meta.is_file() {
        return Err(refusal("not a regular file"));
    }
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::LOOP {
            refusal("a symlink")
        } else {
            format!("cannot read {}: {error}", path.display())
        }
    })?;
    let stat = rustix::fs::fstat(&fd)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return Err(refusal("not a regular file"));
    }
    if stat.st_size as u64 > scoot_look::state::MAX_FILE {
        return Err(format!(
            "refused: {} is larger than {} bytes",
            path.display(),
            scoot_look::state::MAX_FILE
        ));
    }
    let mut bytes = Vec::new();
    use std::io::Read as _;
    std::fs::File::from(fd)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(bytes)
}

/// One link of an `extends` chain, base first.
struct ChainEntry {
    entry: Entry,
    /// The `look.toml` bytes (the winner's, from the catalog).
    bytes: Vec<u8>,
}

/// Collects the `extends` chain for `name`, base first, following the same
/// rules `resolve` enforces (unknown bases, cycles, depth, installed
/// bases): the provenance walk the render inputs need (a wallpaper image
/// resolves against the directory of the look that names it, not the
/// child that inherits it).
fn collect_chain(catalog: &Catalog, name: &str) -> Result<Vec<ChainEntry>, String> {
    let mut chain: Vec<String> = Vec::new();
    let mut entries: Vec<ChainEntry> = Vec::new();
    collect_into(catalog, name, true, &mut chain, &mut entries)?;
    Ok(entries)
}

fn collect_into(
    catalog: &Catalog,
    name: &str,
    root: bool,
    chain: &mut Vec<String>,
    entries: &mut Vec<ChainEntry>,
) -> Result<(), String> {
    use scoot_look::schema::MAX_EXTENDS_DEPTH;
    if chain.len() > MAX_EXTENDS_DEPTH as usize {
        return Err(format!(
            "extends chain deeper than {MAX_EXTENDS_DEPTH}: {}",
            chain.join(" -> ")
        ));
    }
    if chain.contains(&name.to_owned()) {
        return Err(format!("extends cycle: {} -> {name}", chain.join(" -> ")));
    }
    let entry = catalog
        .find(name)
        .ok_or_else(|| format!("unknown base look `{name}`"))?;
    if !root && !entry.origin.extendable() {
        return Err(format!(
            "`{name}` is an installed look and can be applied but never extended"
        ));
    }
    chain.push(name.to_owned());
    if let Some(base) = entry.raw.extends.clone() {
        collect_into(catalog, &base, false, chain, entries)?;
    }
    chain.pop();
    entries.push(ChainEntry {
        entry: entry.clone(),
        bytes: entry.bytes.clone(),
    });
    Ok(())
}

/// The render inputs for a resolved look: the wallpaper's absolute path
/// and the bar's UI font file, resolved against the chain.
struct Inputs {
    inputs: RenderInputs,
    /// Files consulted, for the notes.
    details: Vec<String>,
}

/// Resolves the render inputs and runs the effectful checks that need only
/// the sources (fonts, wallpaper). The bar-fragment check runs later,
/// against the rendered file.
fn check_inputs(
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    checks: &dyn Checks,
    validation: &mut Validation,
) -> Result<Inputs, String> {
    let mut details = Vec::new();
    // Fonts: each family through its carrier. A `[fonts.files]` entry
    // resolves inside the look directory that names it; otherwise
    // fontconfig resolves the family name.
    let mut ui_font_file: Option<String> = None;
    for (key, family) in [
        ("ui", merged.fonts.ui.as_str()),
        ("sans", merged.fonts.sans.as_str()),
        ("mono", merged.fonts.mono.as_str()),
    ] {
        if let Some((dir, rel)) = carrier_file(chain, key) {
            let path = contained_file(&dir, &rel)
                .map_err(|error| format!("font `{family}` ({key}): {error}"))?;
            details.push(format!("font {key} `{family}`: {}", path.display()));
            if key == "ui" {
                ui_font_file = Some(path_to_string(&path)?);
            }
            continue;
        }
        match checks.font_file(family) {
            Ok(Availability::Available(path)) => {
                details.push(format!("font {key} `{family}`: {path}"));
                if key == "ui" {
                    ui_font_file = Some(path);
                }
            }
            Ok(Availability::Unavailable(note)) => validation.notes.push(note),
            Err(refusal) => validation.errors.push(refusal),
        }
    }
    // Wallpaper: an image resolves inside the look directory that names
    // it (the unit is inherited whole, so that directory is the chain's
    // last entry with a `[wallpaper]` table); a color needs no path.
    let mut wallpaper_abs: Option<String> = None;
    if let Some((dir, image)) = wallpaper_image(chain) {
        let path = contained_file(&dir, &image)
            .map_err(|error| format!("wallpaper `{image}`: {error}"))?;
        match checks.wallpaper(&path) {
            Ok(Availability::Available(())) => {
                details.push(format!("wallpaper: {}", path.display()));
            }
            Ok(Availability::Unavailable(note)) => {
                validation.notes.push(note);
                details.push(format!("wallpaper: {}", path.display()));
            }
            Err(refusal) => validation.errors.push(refusal),
        }
        wallpaper_abs = Some(path_to_string(&path)?);
    }
    if !validation.errors.is_empty() {
        return Err(validation
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "invalid look".to_owned()));
    }
    // The bar's UI font and the wallpaper feed the cache key through the
    // resolved files' paths and mtimes, so a font package update
    // re-renders instead of leaving the bar pointing at a vanished file.
    let mut key_files: Vec<String> = Vec::new();
    if let Some(path) = &ui_font_file {
        key_files.push(format!("{path}:{}", file_mtime(Path::new(path))?));
    }
    if let Some(path) = &wallpaper_abs {
        key_files.push(format!("{path}:{}", file_mtime(Path::new(path))?));
    }
    let _ = key_files;
    Ok(Inputs {
        inputs: RenderInputs {
            wallpaper_abs,
            ui_font_file,
            bar_popup: true,
        },
        details,
    })
}

/// The `[fonts.files]` entry for `key` from the chain's last look that
/// names one, with the directory that names it.
fn carrier_file(chain: &[ChainEntry], key: &str) -> Option<(PathBuf, String)> {
    let mut found = None;
    for link in chain {
        if let Some(rel) = link.entry.raw.fonts.files.get(key) {
            found = Some((link.entry.dir.clone(), rel.clone()));
        }
    }
    found
}

/// The wallpaper image from the chain's last look that names a
/// `[wallpaper]` table with one, with the directory that names it.
fn wallpaper_image(chain: &[ChainEntry]) -> Option<(PathBuf, String)> {
    let mut found = None;
    for link in chain {
        if let Some(wallpaper) = &link.entry.raw.wallpaper
            && let Some(image) = &wallpaper.image
        {
            found = Some((link.entry.dir.clone(), image.clone()));
        }
    }
    found
}

/// Resolves `rel` inside `dir`: the canonical target must stay inside the
/// canonical directory and be a file. A planted symlink pointing outside
/// refuses; a missing file refuses.
fn contained_file(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let canonical_dir = std::fs::canonicalize(dir)
        .map_err(|error| format!("cannot resolve {}: {error}", dir.display()))?;
    let joined = canonical_dir.join(rel);
    let canonical = std::fs::canonicalize(&joined)
        .map_err(|error| format!("cannot resolve {}: {error}", joined.display()))?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(format!(
            "{} escapes the look directory: refusing it",
            joined.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!("{} is not a regular file", canonical.display()));
    }
    Ok(canonical)
}

/// A path as a string: non-UTF-8 paths refuse rather than corrupting a
/// fragment.
fn path_to_string(path: &Path) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        format!(
            "{} is not valid UTF-8: refusing to render it into a config",
            path.display()
        )
    })
}

/// A file's mtime, seconds and nanos, for the cache key.
fn file_mtime(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    let modified = meta
        .modified()
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    let age = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(format!("{}.{}", age.as_secs(), age.subsec_nanos()))
}

/// The render cache: `$XDG_CACHE_HOME/scoot/rendered`.
pub fn render_cache_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    cache_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot/rendered"))
        .ok_or_else(|| "no home resolves: set XDG_CACHE_HOME or HOME".to_owned())
}

/// The cache key for a render: the chain's names and bytes plus the
/// resolved inputs (wallpaper and font paths with mtimes, the bar's popup
/// feature), hashed like `source_hash`. A font package update or an edited
/// source re-renders instead of reusing a stale directory.
fn cache_key(chain: &[ChainEntry], inputs: &RenderInputs) -> String {
    let mut buffer = Vec::new();
    for link in chain {
        buffer.extend_from_slice(link.entry.name.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(&link.bytes);
        buffer.push(0);
    }
    for value in [
        inputs.wallpaper_abs.as_deref().unwrap_or(""),
        inputs.ui_font_file.as_deref().unwrap_or(""),
    ] {
        buffer.extend_from_slice(value.as_bytes());
        buffer.push(0);
    }
    buffer.push(if inputs.bar_popup { 1 } else { 0 });
    // Mtimes ride beside the paths (see `check_inputs`): the key below
    // carries them by re-reading the resolved files' mtimes.
    for path in [
        inputs.wallpaper_abs.as_deref(),
        inputs.ui_font_file.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        buffer.extend_from_slice(file_mtime(Path::new(path)).unwrap_or_default().as_bytes());
        buffer.push(0);
    }
    source_hash(&buffer)
}

/// Renders the look into the cache: `<cache>/<name>-<hash>/` with
/// `scoot.toml`, `bar.toml` and `render.json`, written into a fresh
/// directory and renamed, so a reader racing the render sees the old or
/// the new directory, never a half-written one. An existing directory with
/// the same key is reused after its `render.json` names this look.
fn render_cached(
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    inputs: &RenderInputs,
    cache: &Path,
    notes: &mut Vec<String>,
) -> Result<PathBuf, String> {
    let target = cache.join(format!("{name}-{}", cache_key(chain, inputs)));
    if target.is_dir() {
        let render = target.join("render.json");
        let identity: serde_json::Value = read_capped(&render)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::Value::Null);
        if identity.get("name").and_then(|n| n.as_str()) == Some(name)
            && target.join("scoot.toml").is_file()
            && target.join("bar.toml").is_file()
        {
            notes.push(format!("reusing cached render {}", target.display()));
            return Ok(target);
        }
        std::fs::remove_dir_all(&target)
            .map_err(|error| format!("cannot clear {}: {error}", target.display()))?;
    }
    let scoot =
        render_scoot(merged, inputs).map_err(|error| format!("cannot render {name}: {error}"))?;
    let bar =
        render_bar(merged, inputs).map_err(|error| format!("cannot render {name}: {error}"))?;
    // The fragments must parse as TOML: the renderers quote, and this is
    // where a quoting failure shows itself, before any app reads them.
    for (file, text) in [("scoot.toml", &scoot), ("bar.toml", &bar)] {
        text.parse::<toml::Table>()
            .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
    }
    let target_bytes = chain
        .last()
        .map(|link| link.bytes.as_slice())
        .unwrap_or_default();
    let rendered_json = render_json(name, &source_hash(target_bytes), inputs);
    std::fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let tmp = cache.join(format!(
        ".{name}.tmp.{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ));
    // `create_dir` (not `create_dir_all`) refuses a planted symlink at the
    // temp path instead of writing through one.
    std::fs::create_dir(&tmp)
        .map_err(|error| format!("cannot create {}: {error}", tmp.display()))?;
    let write = (|| {
        for (file, contents) in [
            ("scoot.toml", scoot.as_str()),
            ("bar.toml", bar.as_str()),
            ("render.json", rendered_json.as_str()),
        ] {
            let path = tmp.join(file);
            std::fs::write(&path, contents)?;
            std::fs::File::open(&path)?.sync_all()?;
        }
        std::fs::rename(&tmp, &target)?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = write {
        std::fs::remove_dir_all(&tmp).ok();
        // A concurrent apply won the rename: reuse its directory when it
        // is complete, else refuse.
        if target.is_dir()
            && target.join("scoot.toml").is_file()
            && target.join("bar.toml").is_file()
        {
            notes.push(format!("reusing cached render {}", target.display()));
            return Ok(target);
        }
        return Err(format!("cannot render into {}: {error}", target.display()));
    }
    Ok(target)
}

/// Prunes the render cache under the lock: keeps the active directory,
/// the previous one, and the four most recent, and deletes the rest.
/// Never a directory named `look.json`, `look` or `look.lock`, whatever
/// pattern it matches.
fn prune_cache(cache: &Path, keep: &[PathBuf], notes: &mut Vec<String>) {
    let entries = match std::fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut rendered: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "look.json" || name == LINK_NAME || name == "look.lock" || name.starts_with('.')
        {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() || !is_render_name(&name) {
            continue;
        }
        if keep.iter().any(|kept| kept == &path) {
            continue;
        }
        let mtime = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        rendered.push((path, mtime));
    }
    rendered.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (path, _) in rendered.into_iter().skip(4) {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => notes.push(format!("pruned cached render {}", path.display())),
            Err(error) => notes.push(format!(
                "cannot prune {}: {error}: left alone",
                path.display()
            )),
        }
    }
}

/// Whether a cache entry name is a render (`<name>-<16 hex>`).
fn is_render_name(name: &str) -> bool {
    let Some((head, hash)) = name.rsplit_once('-') else {
        return false;
    };
    !head.is_empty()
        && hash.len() == 16
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && is_name(head)
}

/// Repoints the `look` link at the recorded directory: temp symlink plus
/// `rename`, after `write_record`. The link is derived from the record, so
/// a crash between the two renames heals on the next repair.
fn point_link(dir: &Path, rendered: &str) -> Result<(), String> {
    let link = dir.join(LINK_NAME);
    // Unique temp names, so a planted symlink at a guessable name is never
    // opened: the counter lives in this process, the pid across them.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let tmp = dir.join(format!("{LINK_NAME}.tmp.{}-{id}", std::process::id()));
    let pointed =
        std::os::unix::fs::symlink(rendered, &tmp).and_then(|()| std::fs::rename(&tmp, &link));
    if pointed.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    pointed.map_err(|error| format!("cannot point {} at {rendered}: {error}", link.display()))
}

/// Applies `name`: the full transaction. Pure over its `catalog`, `cache`,
/// `state_dir` and peers except the lock, the record, the link, the cache
/// and the reloads themselves.
#[allow(clippy::too_many_arguments)]
pub fn apply_with(
    name: &str,
    options: &ApplyOptions,
    catalog: &Catalog,
    cache: &Path,
    state_dir: &Path,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn Checks,
    dconf: &dyn Dconf,
) -> Result<ApplyReport, ApplyFailure> {
    if !is_name(name) {
        return Err(ApplyFailure {
            message: format!("`{name}` is not a look name ([a-z0-9-]{{1,32}})"),
            rolled_back_to: None,
            rollback_errors: Vec::new(),
        });
    }
    let refuse = |message: String| ApplyFailure {
        message,
        rolled_back_to: None,
        rollback_errors: Vec::new(),
    };
    // An unwritable state dir refuses loudly, before anything is touched.
    std::fs::create_dir_all(state_dir).map_err(|error| {
        refuse(format!(
            "cannot use state dir {}: {error}",
            state_dir.display()
        ))
    })?;
    let _lock = acquire_lock(state_dir, options.lock_wait).map_err(refuse)?;
    let mut report = ApplyReport::default();
    // The link is derived from the record: repair it first, so a crash
    // between the two renames heals here.
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
                "cannot apply: the recorded choice is corrupt ({reason}): fix or remove {}",
                state_dir.join("look.json").display()
            )));
        }
    };
    let entry = catalog
        .find(name)
        .ok_or_else(|| refuse(unknown_look(name, catalog)))?;
    let chain = collect_chain(catalog, name).map_err(refuse)?;
    let merged = resolve(&catalog.sources(), name)
        .map_err(|error| refuse(format!("cannot resolve `{name}`: {error}")))?;
    let contrast = check_contrast(&merged, entry.origin);
    let mut contrast_errors = Vec::new();
    for refusal in contrast.refusals() {
        contrast_errors.push(format!(
            "{} is {} (needs {})",
            refusal.name, refusal.ratio, refusal.threshold
        ));
    }
    if !contrast_errors.is_empty() {
        return Err(refuse(format!(
            "cannot apply `{name}`: {}",
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
    let inputs = check_inputs(&chain, &merged, checks, &mut validation).map_err(refuse)?;
    report.notes.extend(validation.notes);
    report.notes.extend(inputs.details);
    let mut target = render_cached(
        name,
        &chain,
        &merged,
        &inputs.inputs,
        cache,
        &mut report.notes,
    )
    .map_err(refuse)?;
    // The bar's own parser over the rendered fragment, against the
    // installed binary's features.
    match checks.bar_fragment(&target.join("bar.toml")) {
        Ok(Availability::Available(())) => {}
        Ok(Availability::Unavailable(note)) => report.notes.push(note),
        Err(message) => return Err(refuse(message)),
    }
    let generation = previous
        .as_ref()
        .map(|record| record.generation + 1)
        .unwrap_or(1);
    let mut record = Record {
        name: name.to_owned(),
        rendered: path_to_string(&target).map_err(refuse)?,
        generation,
        chosen_by: Some("apply".to_owned()),
        declared_default: previous
            .as_ref()
            .and_then(|record| record.declared_default.clone()),
        toolkit: previous.as_ref().and_then(|record| record.toolkit),
        color_scheme: previous
            .as_ref()
            .and_then(|record| record.color_scheme.clone()),
    };
    write_record(state_dir, &record)
        .map_err(|error| refuse(format!("cannot record the choice: {error}")))?;
    // `target` is absolute by construction (the cache dir resolves from
    // an absolute home); the record carries what the link points at.
    target = PathBuf::from(record.rendered.clone());
    point_link(state_dir, &record.rendered)
        .map_err(|error| refuse(format!("cannot repoint the look link: {error}")))?;
    // Reloads, in order: the compositor, then every bar. Each refusal
    // rolls the swap back; a skipped app is a note. Every sent reload is
    // recorded before its reply is read, so the rollback re-runs whatever
    // may hold the new layer: a scoped reload only reads the link, so
    // re-running one converges on the restored choice.
    let mut completed_compositor = false;
    let mut completed_bars: Vec<usize> = Vec::new();
    match compositor.reload_look() {
        Ok(reloaded) => {
            completed_compositor = true;
            report.reloaded.push("compositor".to_owned());
            if !reloaded.applied.is_empty() {
                report.notes.push(format!(
                    "compositor applied {}",
                    reloaded.applied.join(", ")
                ));
            }
            report.shadowed.extend(reloaded.shadowed);
            report.invalid.extend(reloaded.invalid);
            if !reloaded.refused.is_empty() {
                let failure = ApplyFailure {
                    message: format!(
                        "the compositor refused the look reload: {}",
                        reloaded.refused.join("; ")
                    ),
                    rolled_back_to: None,
                    rollback_errors: Vec::new(),
                };
                return rollback(
                    state_dir,
                    previous.as_ref(),
                    &record,
                    completed_compositor,
                    &completed_bars,
                    compositor,
                    bars,
                    failure,
                );
            }
        }
        Err(ReloadError::NotRunning(note)) => {
            report.skipped.push(format!("compositor: {note}"));
        }
        Err(ReloadError::Refused(message)) => {
            // Sent but refused: the app may hold the new layer (or nothing
            // at all, when the peer is older than the tag), so the
            // rollback re-runs this reload against the restored choice.
            completed_compositor = true;
            let failure = ApplyFailure {
                message,
                rolled_back_to: None,
                rollback_errors: Vec::new(),
            };
            return rollback(
                state_dir,
                previous.as_ref(),
                &record,
                completed_compositor,
                &completed_bars,
                compositor,
                bars,
                failure,
            );
        }
    }
    for (index, bar) in bars.iter().enumerate() {
        // Sent is completed, answered or not: the rollback re-runs every
        // reload that may hold the new layer.
        completed_bars.push(index);
        match bar.reload_look() {
            Ok(()) => {
                report.reloaded.push(format!("bar {}", bar.display()));
            }
            Err(message) => {
                let failure = ApplyFailure {
                    message: format!("bar {}: {message}", bar.display()),
                    rolled_back_to: None,
                    rollback_errors: Vec::new(),
                };
                return rollback(
                    state_dir,
                    previous.as_ref(),
                    &record,
                    completed_compositor,
                    &completed_bars,
                    compositor,
                    bars,
                    failure,
                );
            }
        }
    }
    // The toolkit step is best-effort: a missing `dconf` or a key someone
    // else owns is a note, never a refusal.
    let dark = merged.polarity == scoot_look::schema::Polarity::Dark;
    if options.no_toolkit {
        report
            .notes
            .push("toolkit writes off for this apply (--no-toolkit)".to_owned());
    } else {
        report.notes.extend(apply_toolkit(&mut record, dark, dconf));
        if let Err(error) = write_record(state_dir, &record) {
            report.notes.push(format!(
                "cannot record the toolkit state: {error}: the look applied, the record lags"
            ));
        }
    }
    // Shadowed keys: the compositor's reply when it reloaded; else the
    // files, read directly.
    if !completed_compositor {
        let (shadowed, mut file_notes) = shadowed_from_files_in(&target, config);
        report.shadowed.extend(shadowed);
        report.notes.append(&mut file_notes);
        report.notes.push(
            "compositor not running: shadowed keys read from the config files, not the live session".to_owned(),
        );
    }
    report.shadowed.sort();
    report.shadowed.dedup();
    report.invalid.sort();
    report.invalid.dedup();
    // The previous render survives until the new one is confirmed: prune
    // only on success, never before a rollback.
    let mut keep = vec![target.clone()];
    if let Some(previous) = &previous {
        keep.push(PathBuf::from(&previous.rendered));
    }
    prune_cache(cache, &keep, &mut report.notes);
    report.name = name.to_owned();
    report.rendered = record.rendered.clone();
    report.generation = generation;
    Ok(report)
}

/// Rolls the swap back: the record and the link go back to `previous`
/// (or vanish, when this was the first choice), and the reloads that
/// already ran run again, scoped to the look. A rollback reload that also
/// refuses leaves the link at the previous look and reports both errors:
// "rolled back with errors".
#[allow(clippy::too_many_arguments)]
fn rollback(
    state_dir: &Path,
    previous: Option<&Record>,
    failed: &Record,
    completed_compositor: bool,
    completed_bars: &[usize],
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    mut failure: ApplyFailure,
) -> Result<ApplyReport, ApplyFailure> {
    let restored = match previous {
        Some(previous) => {
            let restored = Record {
                name: previous.name.clone(),
                rendered: previous.rendered.clone(),
                generation: failed.generation + 1,
                chosen_by: Some("rollback".to_owned()),
                declared_default: previous.declared_default.clone(),
                toolkit: previous.toolkit,
                color_scheme: previous.color_scheme.clone(),
            };
            if let Err(error) = write_record(state_dir, &restored) {
                failure
                    .rollback_errors
                    .push(format!("cannot restore the record: {error}"));
            }
            if let Err(error) = point_link(state_dir, &restored.rendered) {
                failure
                    .rollback_errors
                    .push(format!("cannot restore the link: {error}"));
            }
            Some(restored.name.clone())
        }
        None => {
            // The first choice never recorded one: remove what the failed
            // swap wrote, so no dangling choice survives it.
            std::fs::remove_file(state_dir.join("look.json")).ok();
            std::fs::remove_file(state_dir.join(LINK_NAME)).ok();
            None
        }
    };
    failure.rolled_back_to = restored;
    if completed_compositor
        && let Err(error) = compositor.reload_look().and_then(|reloaded| {
            if reloaded.refused.is_empty() {
                Ok(reloaded)
            } else {
                Err(ReloadError::Refused(format!(
                    "the compositor refused the rollback reload: {}",
                    reloaded.refused.join("; ")
                )))
            }
        })
    {
        failure.rollback_errors.push(match error {
            ReloadError::NotRunning(note) => format!("compositor rollback skipped: {note}"),
            ReloadError::Refused(message) => message,
        });
    }
    for index in completed_bars {
        if let Some(bar) = bars.get(*index)
            && let Err(message) = bar.reload_look()
        {
            failure
                .rollback_errors
                .push(format!("bar {} rollback: {message}", bar.display()));
        }
    }
    Err(failure)
}

/// The unknown-look refusal, naming what the catalog holds.
fn unknown_look(name: &str, catalog: &Catalog) -> String {
    let mut names: Vec<&str> = catalog
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    names.sort();
    if names.is_empty() {
        return format!("unknown look `{name}`: the catalog is empty");
    }
    format!("unknown look `{name}` (available: {})", names.join(", "))
}

/// Reads the user's own config files and reports the keys that override
/// the rendered look: the offline shadowed-keys approximation for when the
/// compositor is not running to answer. Missing or unparsable files are
/// notes, never failures.
pub(crate) fn shadowed_from_files(target: &Path) -> (Vec<String>, Vec<String>) {
    shadowed_from_files_in(target, None)
}

/// [`shadowed_from_files`] over an explicit config home (`None` resolves
/// the live environment), so tests pin the files without touching the
/// process environment.
pub(crate) fn shadowed_from_files_in(
    target: &Path,
    config: Option<&Path>,
) -> (Vec<String>, Vec<String>) {
    let mut shadowed = Vec::new();
    let mut notes = Vec::new();
    let owned;
    let config = match config {
        Some(dir) => Some(dir),
        None => {
            let xdg = std::env::var_os("XDG_CONFIG_HOME");
            let home = std::env::var_os("HOME");
            owned = config_home(xdg.as_deref(), home.as_deref());
            owned.as_deref()
        }
    };
    let user_scoot = config.map(|base| base.join("scoot/config.toml"));
    let user_bar = config.map(|base| base.join("scoot/bar.toml"));
    let mut read_table = |path: &Path| -> Option<toml::Table> {
        match read_capped(path) {
            Ok(text) => match text.parse::<toml::Table>() {
                Ok(table) => Some(table),
                Err(error) => {
                    notes.push(format!(
                        "cannot parse {}: {error}: shadowed keys unchecked there",
                        path.display()
                    ));
                    None
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                notes.push(format!(
                    "cannot read {}: {error}: shadowed keys unchecked there",
                    path.display()
                ));
                None
            }
        }
    };
    let read_fragment = |path: &Path| -> toml::Table {
        read_capped(path)
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .unwrap_or_default()
    };
    if let Some(path) = user_scoot.as_deref()
        && let Some(user) = read_table(path)
    {
        let fragment = read_fragment(&target.join("scoot.toml"));
        shadowed.extend(intersect_keys(&user, &fragment, true));
    }
    if let Some(path) = user_bar.as_deref()
        && let Some(user) = read_table(path)
    {
        let fragment = read_fragment(&target.join("bar.toml"));
        shadowed.extend(intersect_keys(&user, &fragment, false));
    }
    shadowed.sort();
    shadowed.dedup();
    (shadowed, notes)
}

/// The dotted keys present in both tables. For the compositor the
/// `[wallpaper]` unit collapses to one `wallpaper` key (a user `color`
/// merged per key with a look `image` is both, which scootbg refuses);
/// the bar has no such unit.
fn intersect_keys(user: &toml::Table, fragment: &toml::Table, wallpaper_unit: bool) -> Vec<String> {
    let user_keys = flatten_keys(user, String::new());
    let fragment_keys: std::collections::BTreeSet<String> =
        flatten_keys(fragment, String::new()).into_iter().collect();
    let mut shadowed: Vec<String> = user_keys
        .into_iter()
        .filter(|key| fragment_keys.contains(key))
        .collect();
    if wallpaper_unit
        && shadowed
            .iter()
            .any(|key| key == "wallpaper" || key.starts_with("wallpaper."))
    {
        shadowed.retain(|key| key != "wallpaper" && !key.starts_with("wallpaper."));
        shadowed.push("wallpaper".to_owned());
    }
    shadowed
}

/// Every leaf key of a TOML table, dotted (`appearance.background_color`).
fn flatten_keys(table: &toml::Table, prefix: String) -> Vec<String> {
    let mut keys = Vec::new();
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => keys.extend(flatten_keys(inner, dotted)),
            toml::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if let toml::Value::Table(inner) = item {
                        keys.extend(flatten_keys(inner, format!("{dotted}.{index}")));
                    } else {
                        keys.push(dotted.clone());
                    }
                }
            }
            _ => keys.push(dotted),
        }
    }
    keys
}

/// Renders a source (a directory, or a catalog name) into `out` without
/// touching state, the cache, or any app: `build` without `apply`.
pub fn build_to_out(
    target: &BuildTarget,
    out: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
) -> Result<BuildReport, String> {
    let (chain, merged, name) = match target {
        BuildTarget::Dir { path, origin } => {
            let validation = validate_dir(path, *origin, catalog, checks)?;
            if !validation.errors.is_empty() {
                return Err(validation.errors.join("; "));
            }
            let raw_name = look_name_in(path)?;
            let chain = match catalog.find(&raw_name) {
                Some(_) => collect_chain(catalog, &raw_name)?,
                None => {
                    let bytes = read_regular(&path.join("look.toml"))?;
                    let canonical = std::fs::canonicalize(path)
                        .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
                    let raw = scoot_look::parse_look_toml(&bytes).map_err(|errors| {
                        errors
                            .iter()
                            .map(|error| error.to_string())
                            .collect::<Vec<_>>()
                            .join("; ")
                    })?;
                    vec![ChainEntry {
                        entry: Entry {
                            name: raw_name.clone(),
                            dir: canonical,
                            origin: *origin,
                            raw,
                            bytes: bytes.clone(),
                        },
                        bytes,
                    }]
                }
            };
            let merged = resolve(&build_sources(catalog, &chain), &raw_name)
                .map_err(|error| format!("cannot resolve `{raw_name}`: {error}"))?;
            (chain, merged, raw_name)
        }
        BuildTarget::Name(name) => {
            let entry = catalog
                .find(name)
                .ok_or_else(|| unknown_look(name, catalog))?;
            let _ = entry;
            let chain = collect_chain(catalog, name)?;
            let merged = resolve(&catalog.sources(), name)
                .map_err(|error| format!("cannot resolve `{name}`: {error}"))?;
            (chain, merged, name.clone())
        }
    };
    let mut validation = Validation::default();
    let inputs = check_inputs(&chain, &merged, checks, &mut validation)
        .map_err(|error| validation.errors.first().cloned().unwrap_or(error))?;
    let scoot = render_scoot(&merged, &inputs.inputs)
        .map_err(|error| format!("cannot render {name}: {error}"))?;
    let bar = render_bar(&merged, &inputs.inputs)
        .map_err(|error| format!("cannot render {name}: {error}"))?;
    for (file, text) in [("scoot.toml", &scoot), ("bar.toml", &bar)] {
        text.parse::<toml::Table>()
            .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
    }
    let target_bytes = chain
        .last()
        .map(|link| link.bytes.as_slice())
        .unwrap_or_default();
    let rendered_json = render_json(&name, &source_hash(target_bytes), &inputs.inputs);
    std::fs::create_dir_all(out)
        .map_err(|error| format!("cannot create {}: {error}", out.display()))?;
    for (file, contents) in [
        ("scoot.toml", scoot.as_str()),
        ("bar.toml", bar.as_str()),
        ("render.json", rendered_json.as_str()),
    ] {
        let path = out.join(file);
        std::fs::write(&path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        std::fs::File::open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    match checks.bar_fragment(&out.join("bar.toml")) {
        Ok(Availability::Available(())) => {}
        Ok(Availability::Unavailable(note)) => validation.notes.push(note),
        Err(message) => return Err(message),
    }
    Ok(BuildReport {
        name,
        out: path_to_string(out)?,
        warnings: validation.warnings,
        notes: validation.notes.into_iter().chain(inputs.details).collect(),
    })
}

/// What `build` renders.
pub enum BuildTarget {
    /// A source directory, with the origin to validate it under.
    Dir { path: PathBuf, origin: Origin },
    /// A catalog name.
    Name(String),
}

/// What one `build` wrote.
pub struct BuildReport {
    pub name: String,
    pub out: String,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

/// The `sources` map for a directory build: the catalog's winners plus the
/// chain's own entries (a directory under another name still resolves).
fn build_sources(catalog: &Catalog, chain: &[ChainEntry]) -> BTreeMap<String, (RawLook, Origin)> {
    let mut sources = catalog.sources();
    for link in chain {
        sources.insert(
            link.entry.name.clone(),
            (link.entry.raw.clone(), link.entry.origin),
        );
    }
    sources
}

/// The look name a directory's `look.toml` carries, parsed only.
fn look_name_in(dir: &Path) -> Result<String, String> {
    let bytes = read_regular(&dir.join("look.toml"))?;
    let raw = scoot_look::parse_look_toml(&bytes).map_err(|errors| {
        errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(raw.name)
}

/// Resolves the state dir for the tool: `--state`, then the shared rule.
/// Refuses when no home resolves at all.
pub fn resolve_state_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    state_dir_from_env(explicit)
        .ok_or_else(|| "no state dir resolves: set --state, SCOOT_LOOK_STATE or HOME".to_owned())
}

/// The production effectful checks: the real binaries beside this one and
/// on `PATH`.
pub struct ProdChecks {
    scootbg: Option<PathBuf>,
    scootbar: Option<PathBuf>,
    fc_match: Option<PathBuf>,
}

impl ProdChecks {
    pub fn new() -> Self {
        Self {
            scootbg: find_binary("scootbg"),
            scootbar: find_binary("scootbar"),
            fc_match: find_binary("fc-match"),
        }
    }

    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            scootbg: None,
            scootbar: None,
            fc_match: None,
        }
    }
}

impl Default for ProdChecks {
    fn default() -> Self {
        Self::new()
    }
}

/// Finds a helper binary: beside this binary first (a release tarball lays
/// `scoot`, `scootbg` and `scootbar` in one `bin/` dir), then on `PATH`.
fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        let sibling = bin.join(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    crate::look::toolkit::find_on_path(name)
}

impl Checks for ProdChecks {
    fn font_file(&self, family: &str) -> Result<Availability<String>, String> {
        let Some(fc_match) = &self.fc_match else {
            return Ok(Availability::Unavailable(
                "no `fc-match` on PATH: font files unchecked".to_owned(),
            ));
        };
        let out = std::process::Command::new(fc_match)
            .arg("--format=%{file}\n")
            .arg(family)
            .output()
            .map_err(|error| format!("cannot run `fc-match`: {error}"))?;
        if !out.status.success() {
            return Err(format!(
                "font `{family}` resolves to nothing (fc-match failed)"
            ));
        }
        let path = String::from_utf8(out.stdout)
            .map_err(|_| format!("font `{family}`: fc-match answered non-UTF-8"))?;
        let path = path.trim().to_owned();
        if path.is_empty() {
            return Err(format!(
                "font `{family}` resolves to nothing (fc-match answered empty)"
            ));
        }
        Ok(Availability::Available(path))
    }

    fn wallpaper(&self, path: &Path) -> Result<Availability<()>, String> {
        let Some(scootbg) = &self.scootbg else {
            let size = std::fs::metadata(path)
                .map_err(|error| format!("cannot stat {}: {error}", path.display()))?
                .len();
            if size > MAX_IMAGE_BYTES {
                return Err(format!(
                    "{} is {} bytes (over the {} byte cap): refusing it",
                    path.display(),
                    size,
                    MAX_IMAGE_BYTES
                ));
            }
            return Ok(Availability::Unavailable(
                "no `scootbg` found: wallpaper bytes capped, decode unchecked".to_owned(),
            ));
        };
        let out = std::process::Command::new(scootbg)
            .arg("check-image")
            .arg(path)
            .output()
            .map_err(|error| format!("cannot run `scootbg check-image`: {error}"))?;
        if out.status.success() {
            Ok(Availability::Available(()))
        } else {
            let detail = String::from_utf8(out.stderr)
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            Err(format!(
                "wallpaper {} refused by scootbg check-image{}",
                path.display(),
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ))
        }
    }

    fn bar_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        let Some(scootbar) = &self.scootbar else {
            return Ok(Availability::Unavailable(
                "no `scootbar` found: bar fragment unchecked".to_owned(),
            ));
        };
        let out = std::process::Command::new(scootbar)
            .arg("daemon")
            .arg("--check")
            .arg("--config")
            .arg(fragment)
            .output()
            .map_err(|error| format!("cannot run `scootbar daemon --check`: {error}"))?;
        if out.status.success() {
            Ok(Availability::Available(()))
        } else {
            let detail = String::from_utf8(out.stderr)
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            Err(format!(
                "bar fragment refused by scootbar daemon --check{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ))
        }
    }
}

#[cfg(test)]
mod tests;
