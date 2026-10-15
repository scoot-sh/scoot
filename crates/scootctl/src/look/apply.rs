//! Rendering, the render cache, the effectful checks, and the swap itself.
//!
//! The swap is the design's section 4.5 under one `flock`: render and
//! validate, write `look.json` then the link (repairing the link from
//! `look.json` whenever they disagree), scoped reloads in order (the
//! compositor, then every bar of the session), skip what is not running,
//! roll back on a refusal, and list shadowed keys. The render cache is
//! pruned under the same lock (the active, the previous, the four most
//! recent, plus any render another live session's link names; never a
//! directory named `look.json`, `look`, `look.lock`, or a dotfile). An
//! unwritable state dir refuses loudly, before any record, link or reload
//! is touched (the lock file and cache dirs may already exist: creating
//! them is not a swap).
//!
//! A crash between the repoint and the reloads leaves the apps on the old
//! look with the link on the new one; re-running `apply`, or any app's
//! next reload, heals it. Rollback covers refusals, not crashes: a failed
//! render keeps the previous render until the next successful prune, and
//! a failed first swap removes what it wrote.
//!
//! The reload and settings peers are traits ([`CompositorReload`],
//! [`BarReload`], [`Dconf`], [`Checks`]), so the transaction unit-tests
//! against scripted peers: atomicity, refuse-and-keep, rollback,
//! rollback-with-errors, the lock, link repair, a missing bar, shadowed
//! keys, and a pending user edit never applied by a swap.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
#[derive(Debug)]
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
    // Effectful refusals land in `errors`, not in `Err`, so `--json`
    // always prints a document: only catalog and resolution failures
    // (unknown names, unresolvable chains) refuse the command itself.
    let _ = check_inputs(&chain, &merged, checks, &mut validation);
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
    // The name matches the directory as named (a symlink's own name),
    // never the store target's basename.
    let dir_name = dir
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
    let (chain, merged) = resolve_dir(raw, origin, bytes, canonical, catalog)?;
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
    if check_inputs(&chain, &merged, checks, &mut validation).is_err() {
        return Ok(validation);
    }
    Ok(validation)
}

/// Resolves a directory's own manifest: the directory's entry wins over
/// any same-named catalog entry (a tweaked copy never silently builds the
/// catalog's look), and its `extends` base resolves through the catalog.
/// Returns the provenance chain (base first) with the merge.
fn resolve_dir(
    raw: RawLook,
    origin: Origin,
    bytes: Vec<u8>,
    dir: PathBuf,
    catalog: &Catalog,
) -> Result<(Vec<ChainEntry>, scoot_look::ResolvedLook), String> {
    let mut sources = catalog.sources();
    sources.insert(raw.name.clone(), (raw.clone(), origin));
    let merged = resolve(&sources, &raw.name)
        .map_err(|error| format!("cannot resolve `{}`: {error}", raw.name))?;
    // The resolution above already enforced the chain rules (unknown
    // bases, cycles, depth, installed bases); the walk below only gathers
    // the provenance it validated.
    let mut chain = match &raw.extends {
        Some(base) => collect_chain_from(catalog, base, false)?,
        None => Vec::new(),
    };
    chain.push(ChainEntry {
        entry: Entry {
            name: raw.name.clone(),
            dir,
            origin,
            raw,
            bytes: bytes.clone(),
        },
        bytes,
    });
    Ok((chain, merged))
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
        .take(scoot_look::state::MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > scoot_look::state::MAX_FILE {
        return Err(format!(
            "refused: {} is larger than {} bytes",
            path.display(),
            scoot_look::state::MAX_FILE
        ));
    }
    Ok(bytes)
}

/// Writes `bytes` to `path` without following symlinks: a symlinked
/// destination refuses instead of writing through it. A new file is
/// created `O_EXCL|O_NOFOLLOW`; an existing regular file is replaced
/// atomically (temp plus `rename`, which replaces a symlink rather than
/// following it — but a symlink is still refused above, so a raced-in
/// link can never be overwritten silently either).
fn write_nofollow(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err("it is a symlink: refusing to write through it".to_owned());
    }
    if !path.exists() {
        let fd = rustix::fs::open(
            path,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_bits_truncate(0o644),
        )
        .map_err(|errno| format!("{errno}"))?;
        use std::io::Write as _;
        let mut handle = std::fs::File::from(fd);
        handle
            .write_all(bytes)
            .map_err(|error| format!("{error}"))?;
        handle.sync_all().map_err(|error| format!("{error}"))?;
        return Ok(());
    }
    // Existing regular file: stage beside it and rename over it. The
    // rename replaces whatever is there atomically; the symlink check
    // above (plus `O_NOFOLLOW|O_EXCL` on the temp) means a raced-in link
    // at the temp path refuses instead of redirecting the write.
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let tmp = match parent {
        Some(dir) => dir.join(format!(
            ".{}.tmp.{}-{}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("out"),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        )),
        None => PathBuf::from(format!(
            ".{}.tmp.{}-{}",
            path.display(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        )),
    };
    let fd = rustix::fs::open(
        &tmp,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_bits_truncate(0o644),
    )
    .map_err(|errno| format!("{errno}"))?;
    use std::io::Write as _;
    let mut handle = std::fs::File::from(fd);
    let result: Result<(), String> = (|| {
        handle
            .write_all(bytes)
            .map_err(|error| format!("{error}"))?;
        handle.sync_all().map_err(|error| format!("{error}"))?;
        drop(handle);
        // Re-check the destination is still not a symlink before the
        // rename: a link raced in between the first check and now refuses.
        if let Ok(meta) = std::fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            std::fs::remove_file(&tmp).ok();
            return Err("it became a symlink: refusing to write through it".to_owned());
        }
        std::fs::rename(&tmp, path).map_err(|error| format!("{error}"))?;
        Ok(())
    })();
    if result.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    result
}

/// Whether a `bar_fragment` refusal is the missing-`popup` shape (a bar
/// built `--no-default-features` refusing `popup-radius`): the caller
/// re-renders with `bar_popup = false` instead of refusing every look.
fn is_popup_refusal(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("popup-radius") || (lower.contains("popup") && lower.contains("unknown field"))
}

/// Whether a refusal is the older-peer shape (a compositor or bar that
/// predates the look-scoped request): the peer never accepted the request,
/// so the rollback skips re-running it rather than refusing twice.
pub(crate) fn is_predates_error(message: &str) -> bool {
    message.contains("predates scoot look")
        || (message.contains("unknown variant") && message.contains("reload"))
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
    collect_chain_from(catalog, name, true)
}

/// [`collect_chain`], with the root flag explicit: a directory resolving
/// its own `extends` base walks the base as a non-root, so an installed
/// base is refused the same way `resolve` refuses it.
fn collect_chain_from(
    catalog: &Catalog,
    name: &str,
    root: bool,
) -> Result<Vec<ChainEntry>, String> {
    let mut chain: Vec<String> = Vec::new();
    let mut entries: Vec<ChainEntry> = Vec::new();
    collect_into(catalog, name, root, &mut chain, &mut entries)?;
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
            match contained_file(&dir, &rel) {
                Ok(path) => {
                    details.push(format!("font {key} `{family}`: {}", path.display()));
                    if key == "ui" {
                        match path_to_string(&path) {
                            Ok(text) => ui_font_file = Some(text),
                            Err(error) => validation.errors.push(format!(
                                "font `{family}` ({key}): {error}"
                            )),
                        }
                    }
                }
                Err(error) => validation.errors.push(format!("font `{family}` ({key}): {error}")),
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
        match contained_file(&dir, &image) {
            Ok(path) => {
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
                match path_to_string(&path) {
                    Ok(text) => wallpaper_abs = Some(text),
                    Err(error) => {
                        validation.errors.push(format!("wallpaper `{image}`: {error}"))
                    }
                }
            }
            Err(error) => validation.errors.push(format!("wallpaper `{image}`: {error}")),
        }
    }
    if !validation.errors.is_empty() {
        return Err(validation
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "invalid look".to_owned()));
    }
    // The bar's UI font and the wallpaper feed the cache key through
    // their resolved paths and mtimes (see `cache_key`), so a font
    // package update re-renders instead of leaving the bar pointing at a
    // vanished file.
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

/// Makes `path` absolute against the current directory, lexically (no
/// symlink resolution, so it works for paths that do not exist yet).
/// Records and links must never carry relative paths: a relative render
/// path fails the next record read as corrupt, and a relative link target
/// resolves against the state dir instead of the writer's directory.
pub fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(normalize(path));
    }
    let current = std::env::current_dir()
        .map_err(|error| format!("cannot resolve the working directory: {error}"))?;
    Ok(normalize(&current.join(path)))
}

/// Lexically normalizes an absolute path (`.` and `..` components only):
/// the record's `rendered` must not contain `..`, and this keeps that
/// promise for joined paths.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// The render cache: `$XDG_CACHE_HOME/scoot/rendered`.
pub fn render_cache_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    cache_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot/rendered"))
        .ok_or_else(|| "no home resolves: set XDG_CACHE_HOME or HOME".to_owned())
}

/// The render schema: bumped whenever the renderers' output changes shape,
/// so an upgrade re-renders an unchanged look instead of reusing a stale
/// cached directory. Part of the cache key.
pub const RENDER_VERSION: u32 = 1;

/// The cache key for a render: the chain's names and bytes plus the
/// resolved inputs (wallpaper and font paths with mtimes, the bar's popup
/// feature), hashed like `source_hash`. A font package update or an edited
/// source re-renders instead of reusing a stale directory.
fn cache_key(chain: &[ChainEntry], inputs: &RenderInputs) -> String {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&RENDER_VERSION.to_le_bytes());
    buffer.push(0);
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
    // Unique across processes and threads of this process: two applies of
    // different looks into one cache never share a temp name, and a
    // planted symlink at a guessable name is never opened (`create_dir`
    // refuses it instead of writing through one).
    static RENDER_COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp = cache.join(format!(
        ".{name}.tmp.{}-{}-{}",
        std::process::id(),
        RENDER_COUNTER.fetch_add(1, Ordering::SeqCst),
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
        let path = entry.path();
        if name.starts_with('.') {
            // Our own crashed temp dirs (`.<name>.tmp.<pid>-<ctr>-<nanos>`)
            // are reaped a day later; anything else dot-named is left
            // alone.
            if path.is_dir() && is_render_tmp(&name) && dir_older_than(&path, 24 * 3600) {
                std::fs::remove_dir_all(&path).ok();
            }
            continue;
        }
        if name == "look.json" || name == LINK_NAME || name == "look.lock" {
            continue;
        }
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

/// Whether a cache entry name is one of our render temp dirs
/// (`.<name>.tmp.<pid>-<ctr>-<nanos>`).
fn is_render_tmp(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((head, _)) = rest.rsplit_once(".tmp.") else {
        return false;
    };
    is_name(head)
}

/// Whether a directory's mtime is older than `seconds`.
fn dir_older_than(path: &Path, seconds: u64) -> bool {
    let mtime = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(std::time::UNIX_EPOCH);
    mtime
        .elapsed()
        .map(|age| age.as_secs() > seconds)
        .unwrap_or(false)
}

/// Renders other sessions may point at: the user state's link target and
/// every private session's (`$XDG_RUNTIME_DIR/scoot-look-*/look`), read
/// best-effort. The cache is global across state dirs, but the keep-set of
/// one apply is not: without these, churning one session could prune the
/// render another live session's link names.
fn live_render_targets() -> Vec<PathBuf> {
    let mut targets = Vec::new();
    let mut state_dirs = Vec::new();
    if let Some(user) = state_dir_from_env(None) {
        state_dirs.push(user);
    }
    let xdg = std::env::var_os("XDG_RUNTIME_DIR");
    if let Some(runtime) = xdg.filter(|v| !v.is_empty()).map(PathBuf::from)
        && let Ok(entries) = std::fs::read_dir(&runtime)
    {
        for entry in entries.filter_map(|entry| entry.ok()) {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(scoot_look::state::PRIVATE_PREFIX)
            {
                state_dirs.push(entry.path());
            }
        }
    }
    for dir in state_dirs {
        if let Ok(target) = std::fs::read_link(dir.join(LINK_NAME))
            && target.is_absolute()
        {
            targets.push(target);
        }
    }
    targets
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
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let tmp = dir.join(format!("{LINK_NAME}.tmp.{}-{id}", std::process::id()));
    let pointed =
        std::os::unix::fs::symlink(rendered, &tmp).and_then(|()| std::fs::rename(&tmp, &link));
    if pointed.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    pointed.map_err(|error| format!("cannot point {} at {rendered}: {error}", link.display()))?;
    sync_dir(dir).map_err(|error| format!("cannot sync {}: {error}", dir.display()))?;
    Ok(())
}

/// Syncs a directory after a rename, so the rename itself is durable.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    let fd = rustix::fs::open(
        dir,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    rustix::fs::fsync(fd).map_err(std::io::Error::from)?;
    Ok(())
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
    // Work on absolute dirs throughout: a relative cache would record a
    // relative render path (which the next read refuses as corrupt) and
    // point the link at a target relative to the wrong directory.
    let state_dir = absolutize(state_dir).map_err(refuse)?;
    let state_dir = state_dir.as_path();
    let cache = absolutize(cache).map_err(refuse)?;
    let cache = cache.as_path();
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
    let inputs = match check_inputs(&chain, &merged, checks, &mut validation) {
        Ok(inputs) => inputs,
        Err(_) => {
            let detail = validation.errors.join("; ");
            // `check_inputs` pushes every refusal into `errors`, so this
            // is never blank; the fallback is a backstop, not a message.
            let detail = if detail.trim().is_empty() {
                "invalid look (no detail)".to_owned()
            } else {
                detail
            };
            return Err(refuse(format!("cannot apply `{name}`: {detail}")));
        }
    };
    report.notes.extend(validation.notes);
    let mut render_inputs = inputs.inputs;
    let details = inputs.details;
    report.notes.extend(details);
    let mut target = render_cached(
        name,
        &chain,
        &merged,
        &render_inputs,
        cache,
        &mut report.notes,
    )
    .map_err(refuse)?;
    // The bar's own parser over the rendered fragment, against the
    // installed binary's features. A bar built `--no-default-features`
    // (no `popup`) refuses `popup-radius`: re-render without it rather
    // than refusing every look.
    match checks.bar_fragment(&target.join("bar.toml")) {
        Ok(Availability::Available(())) => {}
        Ok(Availability::Unavailable(note)) => report.notes.push(note),
        Err(message) if render_inputs.bar_popup && is_popup_refusal(&message) => {
            render_inputs.bar_popup = false;
            target = render_cached(name, &chain, &merged, &render_inputs, cache, &mut report.notes)
                .map_err(refuse)?;
            match checks.bar_fragment(&target.join("bar.toml")) {
                Ok(Availability::Available(())) => {
                    report.notes.push(
                        "bar without `popup`: rendered without popup-radius".to_owned(),
                    );
                }
                Ok(Availability::Unavailable(note)) => report.notes.push(note),
                Err(retry) => return Err(refuse(retry)),
            }
        }
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
        color_scheme_orig: previous
            .as_ref()
            .and_then(|record| record.color_scheme_orig.clone()),
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
            // Sent but refused: the app may hold the new layer, so the
            // rollback re-runs this reload against the restored choice.
            // The one exception is a peer older than the request tag,
            // which never accepted it: re-running would refuse again with
            // the same wall, so the rollback skips it.
            if !is_predates_error(&message) {
                completed_compositor = true;
            }
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
        // reload that may hold the new layer. An absent listener never
        // sends, so it is skipped with a note and completes nothing.
        match bar.reload_look() {
            Ok(true) => {
                completed_bars.push(index);
                report.reloaded.push(format!("bar {}", bar.display()));
            }
            Ok(false) => {
                report
                    .skipped
                    .push(format!("bar {}: nothing listens: skipped", bar.display()));
            }
            Err(message) => {
                // Sent but refused: the bar may hold the new layer, so the
                // rollback re-runs this reload against the restored choice.
                completed_bars.push(index);
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
        report.notes.extend(apply_toolkit(state_dir, &mut record, dark, dconf));
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
    // only on success, never before a rollback. The keep-set spans every
    // live session's link (the cache is global, the state dirs are not),
    // or churning one session could prune another's render.
    let mut keep = vec![target.clone()];
    if let Some(previous) = &previous {
        keep.push(PathBuf::from(&previous.rendered));
    }
    keep.extend(live_render_targets());
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
                color_scheme_orig: previous.color_scheme_orig.clone(),
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
            // swap wrote, so no dangling choice survives it. There is no
            // previous layer to reload back to, so the apps that already
            // reloaded still show the new look while `current` says none:
            // say so plainly instead of re-running reloads that would
            // refuse with "no look layer is configured".
            std::fs::remove_file(state_dir.join("look.json")).ok();
            std::fs::remove_file(state_dir.join(LINK_NAME)).ok();
            if completed_compositor || !completed_bars.is_empty() {
                failure.rollback_errors.push(
                    "no previous look exists to roll back to: the compositor and any reloaded bars still show the new look, while no look is recorded"
                        .to_owned(),
                );
            }
            failure.rolled_back_to = None;
            return Err(failure);
        }
    };
    if !failure.rollback_errors.is_empty() {
        // The choice itself did not restore: re-running reloads against an
        // undefined link would converge apps on the wrong look, so the
        // restore failure is the report, not a prelude to one.
        failure.rolled_back_to = None;
        return Err(failure);
    }
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
        // `Ok(false)` (absent at rollback) needs nothing: nothing there
        // holds the new layer.
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

/// Our three output files, the only ones `build --out` may write.
const BUILD_FILES: [&str; 3] = ["scoot.toml", "bar.toml", "render.json"];

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
            // Re-parse for the render (validate passed, so this reads the
            // same manifest it checked): the directory's own entry wins
            // over any same-named catalog entry.
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
            let name = raw.name.clone();
            let (chain, merged) = resolve_dir(raw, *origin, bytes, canonical, catalog)?;
            (chain, merged, name)
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
    let inputs = match check_inputs(&chain, &merged, checks, &mut validation) {
        Ok(inputs) => inputs,
        Err(_) => {
            let detail = validation.errors.join("; ");
            if detail.trim().is_empty() {
                return Err(format!("cannot build {name}: invalid look (no detail)"));
            }
            return Err(detail);
        }
    };
    let mut render_inputs = inputs.inputs;
    let render_details = inputs.details;
    let render_once = |render_inputs: &RenderInputs| -> Result<(String, String, String), String> {
        let scoot = render_scoot(&merged, render_inputs)
            .map_err(|error| format!("cannot render {name}: {error}"))?;
        let bar = render_bar(&merged, render_inputs)
            .map_err(|error| format!("cannot render {name}: {error}"))?;
        for (file, text) in [("scoot.toml", &scoot), ("bar.toml", &bar)] {
            text.parse::<toml::Table>()
                .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
        }
        let target_bytes = chain
            .last()
            .map(|link| link.bytes.as_slice())
            .unwrap_or_default();
        let rendered_json = render_json(&name, &source_hash(target_bytes), render_inputs);
        Ok((scoot, bar, rendered_json))
    };
    let (mut scoot, mut bar, mut rendered_json) = render_once(&render_inputs)?;
    // The bar's own parser runs before anything lands in `--out`: a
    // refused fragment leaves no complete-looking output behind. The
    // check needs a file, so the fragments stage in a unique temp dir
    // beside the destination (never a guessable `out.build-tmp` that an
    // existing dir could collide with, and never `remove_dir_all` on a
    // caller-derived path), then copy over file by file with
    // symlink-safe atomic writes.
    //
    // `out` itself is never followed through a symlink, and a non-empty
    // foreign dir is refused: only an absent dir, an empty dir, or one
    // holding only our own three files is written into. That keeps
    // `--out .` from wiping the cwd and `--out dist` from deleting a
    // pre-existing `dist.build-tmp`.
    static BUILD_COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty());
    let stage_parent: PathBuf = match parent {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|error| format!("cannot resolve the working directory: {error}"))?,
    };
    // Ensure the stage parent exists (for `--out newdir/sub`, the parent
    // `newdir` may not exist yet — but never create `out` itself here).
    if !stage_parent.exists() {
        std::fs::create_dir_all(&stage_parent)
            .map_err(|error| format!("cannot create {}: {error}", stage_parent.display()))?;
    }
    let stage_name = format!(
        ".{}.build-tmp.{}-{}-{}",
        out.file_name().and_then(|n| n.to_str()).unwrap_or("out"),
        std::process::id(),
        BUILD_COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    );
    let stage = stage_parent.join(stage_name);
    // `create_dir` (not `create_dir_all`) refuses a planted symlink at the
    // unique temp path instead of writing through one.
    std::fs::create_dir(&stage)
        .map_err(|error| format!("cannot create {}: {error}", stage.display()))?;
    let mut staged: Vec<(String, String)> = [
        ("scoot.toml".to_owned(), scoot),
        ("bar.toml".to_owned(), bar),
        ("render.json".to_owned(), rendered_json),
    ]
    .into_iter()
    .collect();
    for (file, contents) in &staged {
        std::fs::write(stage.join(file), contents)
            .map_err(|error| format!("cannot write {}: {error}", stage.join(file).display()))?;
    }
    let stage_cleanup = |stage: &Path| {
        std::fs::remove_dir_all(stage).ok();
    };
    match checks.bar_fragment(&stage.join("bar.toml")) {
        Ok(Availability::Available(())) => {}
        Ok(Availability::Unavailable(note)) => validation.notes.push(note),
        Err(message) if render_inputs.bar_popup && is_popup_refusal(&message) => {
            // A bar built `--no-default-features` refuses `popup-radius`:
            // re-render without it rather than refusing every look.
            render_inputs.bar_popup = false;
            (scoot, bar, rendered_json) = render_once(&render_inputs)?;
            std::fs::write(stage.join("scoot.toml"), &scoot)
                .map_err(|error| format!("cannot write {}: {error}", stage.join("scoot.toml").display()))?;
            std::fs::write(stage.join("bar.toml"), &bar)
                .map_err(|error| format!("cannot write {}: {error}", stage.join("bar.toml").display()))?;
            std::fs::write(stage.join("render.json"), &rendered_json)
                .map_err(|error| format!("cannot write {}: {error}", stage.join("render.json").display()))?;
            // Rebuild the staged list to match the re-render.
            staged = vec![
                ("scoot.toml".to_owned(), scoot.clone()),
                ("bar.toml".to_owned(), bar.clone()),
                ("render.json".to_owned(), rendered_json.clone()),
            ];
            match checks.bar_fragment(&stage.join("bar.toml")) {
                Ok(Availability::Available(())) => {
                    validation.notes.push(
                        "bar without `popup`: rendered without popup-radius".to_owned(),
                    );
                }
                Ok(Availability::Unavailable(note)) => validation.notes.push(note),
                Err(retry) => {
                    stage_cleanup(&stage);
                    return Err(retry);
                }
            }
        }
        Err(message) => {
            stage_cleanup(&stage);
            return Err(message);
        }
    }
    // Refuse a symlinked `out`: writing through it would land outside the
    // named directory.
    if let Ok(meta) = std::fs::symlink_metadata(out)
        && meta.file_type().is_symlink()
    {
        stage_cleanup(&stage);
        return Err(format!(
            "refusing to build into {}: it is a symlink",
            out.display()
        ));
    }
    if out.exists() && !out.is_dir() {
        stage_cleanup(&stage);
        return Err(format!(
            "refusing to build into {}: not a directory",
            out.display()
        ));
    }
    if out.is_dir() {
        let entries = std::fs::read_dir(out)
            .map_err(|error| format!("cannot list {}: {error}", out.display()))?;
        let mut foreign: Vec<String> = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            if !BUILD_FILES.contains(&entry_name.as_str()) {
                foreign.push(entry_name.clone());
            }
            // A symlinked output file would be written through: refuse it.
            let fp = out.join(&entry_name);
            if BUILD_FILES.contains(&entry_name.as_str())
                && let Ok(meta) = std::fs::symlink_metadata(&fp)
                && meta.file_type().is_symlink()
            {
                stage_cleanup(&stage);
                return Err(format!(
                    "refusing to build into {}: {} is a symlink",
                    out.display(),
                    fp.display()
                ));
            }
        }
        if !foreign.is_empty() {
            foreign.sort();
            stage_cleanup(&stage);
            return Err(format!(
                "refusing to build into {}: it holds files this build did not write ({})",
                out.display(),
                foreign.join(", ")
            ));
        }
    } else {
        std::fs::create_dir_all(out)
            .map_err(|error| format!("cannot create {}: {error}", out.display()))?;
    }
    for (file, contents) in &staged {
        let path = out.join(file);
        if let Err(error) = write_nofollow(&path, contents.as_bytes()) {
            stage_cleanup(&stage);
            return Err(format!("cannot write {}: {error}", path.display()));
        }
    }
    stage_cleanup(&stage);
    Ok(BuildReport {
        name,
        out: path_to_string(out)?,
        warnings: validation.warnings,
        notes: validation.notes.into_iter().chain(render_details).collect(),
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
#[derive(Debug)]
pub struct BuildReport {
    pub name: String,
    pub out: String,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

/// Resolves the state dir for the tool: `--state`, then the shared rule.
/// Refuses when no home resolves at all.
pub fn resolve_state_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    state_dir_from_env(explicit)
        .ok_or_else(|| "no state dir resolves: set --state, SCOOT_LOOK_STATE or HOME".to_owned())
}

/// How long an effectful helper (`fc-match`, `scootbg`, `scootbar`,
/// `dconf`) may run before the swap stops waiting: a hung child must never
/// wedge an apply with the lock held.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a helper with a timeout, collecting its output. A child that
/// outlives the timeout is killed and reported, never waited on forever.
/// The helpers here answer in lines (`fc-match` one path, `check-image`
/// one verdict, `--check` one word), so collecting after exit cannot
/// deadlock on a full pipe.
pub(crate) fn run_with_timeout(
    program: &Path,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run `{}`: {error}", program.display()))?;
    let deadline = std::time::Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("cannot run `{}`: {error}", program.display()));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    child.kill().ok();
                    child.wait().ok();
                    return Err(format!(
                        "`{}` timed out after {}s",
                        program.display(),
                        HELPER_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(format!("cannot run `{}`: {error}", program.display()));
            }
        }
    }
}

/// Sniffs an image format from the first bytes, never the name: PNG, JPEG,
/// GIF or WebP, the formats scootbg reads. The fallback wallpaper check
/// (no `scootbg`) refuses anything else without decoding.
fn sniff_image(path: &Path) -> Option<&'static str> {
    use std::io::Read as _;
    let mut head = [0u8; 12];
    let mut file = std::fs::File::open(path).ok()?;
    let n = file.read(&mut head).ok()?;
    if n >= 8 && head[..8] == [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'] {
        Some("PNG")
    } else if n >= 3 && head[..3] == [0xFF, 0xD8, 0xFF] {
        Some("JPEG")
    } else if n >= 6 && (head[..6] == *b"GIF87a" || head[..6] == *b"GIF89a") {
        Some("GIF")
    } else if n >= 12 && head[..4] == *b"RIFF" && head[8..12] == *b"WEBP" {
        Some("WebP")
    } else {
        None
    }
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

    /// Test-only: explicit helper paths, so tests pin a stub `fc-match`
    /// without touching the process environment.
    #[cfg(test)]
    pub fn with_fc_match(path: PathBuf) -> Self {
        Self {
            scootbg: None,
            scootbar: None,
            fc_match: Some(path),
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
        let out = run_with_timeout(fc_match, &["--format=%{file}\n", family])?;
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
        // `fc-match` always answers with a fallback file, so a missing
        // family would otherwise apply with exit 0. Compare the matched
        // family name: only an installed family passes.
        let out = run_with_timeout(fc_match, &["--format=%{family}\n", family])?;
        if !out.status.success() {
            return Err(format!(
                "font `{family}` resolves to nothing (fc-match failed)"
            ));
        }
        let matched = String::from_utf8(out.stdout)
            .map_err(|_| format!("font `{family}`: fc-match answered non-UTF-8"))?;
        let want = family.trim().to_lowercase();
        let installed = matched
            .split(',')
            .map(|part| part.trim().to_lowercase())
            .any(|part| part == want);
        if !installed {
            return Err(format!(
                "font `{family}` is not installed (fc-match resolves it to `{}`)",
                matched.trim()
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
            return match sniff_image(path) {
                Some(format) => Ok(Availability::Unavailable(format!(
                    "no `scootbg` found: {format} wallpaper bytes capped, decode unchecked"
                ))),
                None => Err(format!(
                    "{} is not an image scootbg reads (PNG, JPEG, GIF or WebP)",
                    path.display()
                )),
            };
        };
        let out = run_with_timeout(scootbg, &["check-image", &path.to_string_lossy()])?;
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
        let out = run_with_timeout(
            scootbar,
            &["daemon", "--check", "--config", &fragment.to_string_lossy()],
        )?;
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
