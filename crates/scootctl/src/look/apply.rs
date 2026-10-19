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
    LINK_NAME, Record, RecordRead, acquire_lock, ensure_link, read_capped, read_capped_follow,
    read_record, state_dir_from_env, write_record,
};
use scoot_look::{
    Origin, RawLook, RenderInputs, check_contrast, parse_kv_config, render_bar, render_foot,
    render_fuzzel, render_greet, render_gtk3_css, render_gtk4_css, render_json, render_launch,
    render_leaf_files, render_notify, render_qt_scheme, render_scoot, render_swaylock, resolve,
    source_hash,
};

use super::cycle::{Preview, PreviewRead, clear_preview, read_preview, write_preview};
use super::foot::foot_notes;
use super::reload::{BarReload, CompositorReload, ReloadError};
use super::sync::TargetSkips;
use super::toolkit::{Dconf, apply_toolkit_files, apply_toolkit_with_claims};
use scoot_look::catalog::{Catalog, Entry, cache_home, config_home};

/// How long a second writer waits on `<state>/look.lock` before refusing.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);

/// The refusal prefix when a guarded revert finds the choice moved: the
/// reverter maps this back to "superseded, do nothing" rather than a
/// failure. Intra-crate, matched with `starts_with`.
pub const PREVIEW_SUPERSEDED: &str = "preview superseded:";

/// The largest wallpaper image accepted without `scootbg`: the same 32 MiB
/// as scootbg's own download cap (`crates/scootbg/src/fetch.rs`
/// `MAX_BYTES`). With `scootbg` present `check-image` enforces its own
/// byte cap, the pixel cap from the header, and a full decode instead.
pub const MAX_IMAGE_BYTES: u64 = 32 << 20;

/// Options for one `apply`, from flags and the environment.
pub struct ApplyOptions {
    /// How long to wait on the lock.
    pub lock_wait: Duration,
    /// Skip the toolkit step (`--no-toolkit`): no dconf key and no
    /// toolkit file is written for this apply.
    pub no_toolkit: bool,
    /// Keep an installed look's static terminal leaves (`--trust`):
    /// without it they are dropped, with it they render and their
    /// contents print in the reply first.
    pub trust: bool,
    /// Skip the Helix signal (`--no-helix`): the leaf still renders.
    pub no_helix: bool,
    /// Skip the btop signal (`--no-btop`): the leaf still renders.
    pub no_btop: bool,
    /// This apply was spawned by the compositor for an IPC `look_apply`
    /// (`--from-compositor`): recorded as the choice's provenance, so
    /// `current` tells a local swap from a remote one. Changes nothing
    /// else: the child still renders, records, repoints and drives every
    /// scoped reload itself.
    pub from_compositor: bool,
    /// Arm a timed preview (`apply --for`): on success the preview file
    /// records this swap's generation and the previous choice, so the
    /// detached reverter can restore it.
    pub preview: bool,
    /// Revert only while the live generation still equals this (the
    /// detached reverter's guard): a later apply moved the choice, so the
    /// revert refuses as superseded rather than undoing a newer choice.
    /// Checked under the lock before anything is touched.
    pub only_if_generation: Option<u64>,
    /// dconf key paths home-manager owns (the `look-nix-claims` file
    /// beside the declared default): the toolkit step leaves those alone
    /// however the live value reads.
    pub nix_claims: Vec<String>,
    /// Apps whose Stylix target owns its path (the
    /// `look-stylix-targets` file beside the declared default): the
    /// reply names those instead of pretending the look reached them.
    pub stylix_targets: Vec<String>,
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
    /// Whether the swap removed the layer (`sync` to a null default):
    /// no look is recorded and the link is gone.
    pub removed: bool,
    /// User-file keys that override a look token.
    pub shadowed: Vec<String>,
    /// User-file keys that failed to parse and fell back to the look.
    pub invalid: Vec<String>,
    /// The pending preview this swap armed (`apply --for`), if any: what
    /// the detached reverter restores, and the generation it reverts from.
    pub preview: Option<Preview>,
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
    /// Validates a rendered `foot.ini` with the installed binary's own
    /// parser (`foot --check-config --config`). A missing binary is
    /// [`Availability::Unavailable`]; a failing check refuses.
    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String>;
    /// Resolves a URL wallpaper from the download cache: `install` fetched
    /// and verified it at install time, and a swap never fetches. A miss
    /// refuses (never fetches here); `install` owns the fetch.
    fn url_wallpaper(
        &self,
        url: &str,
        sha256: Option<&str>,
    ) -> Result<Availability<String>, String> {
        match super::install::cached_url_wallpaper(url, sha256) {
            Ok(path) => match path_to_string(&path) {
                Ok(text) => Ok(Availability::Available(text)),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        }
    }
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
/// and the effectful checks. `trust` keeps an installed look's static
/// terminal leaves (and prints them); without it they are dropped, as in
/// `apply`.
pub fn validate_entry(
    catalog: &Catalog,
    name: &str,
    checks: &dyn Checks,
    trust: bool,
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
    if let Ok(inputs) = check_inputs(&chain, &merged, checks, &mut validation) {
        // The bar's own parser over the rendered fragment, as `apply`
        // runs it: a look whose fragment its own bar refuses must not
        // validate.
        check_bar_fragment(name, &merged, &inputs.inputs, checks, &mut validation);
        // Foot's own parser over its fragment, as `apply` runs it.
        check_foot_fragment(name, &merged, checks, &mut validation);
        // The launcher and locker leaves, as `apply` writes them.
        check_leaf_fragments(name, &merged, &mut validation);
        // The terminal static leaves, as `apply` renders them (or drops
        // them, for an installed look without `trust`).
        check_static_fragments(&chain, trust, &mut validation);
    }
    Ok(validation)
}

/// Validates a source directory without touching anything. A directory with
/// `extends` resolves its base through the catalog; without one it resolves
/// alone, like `scoot-look build`.
///
/// When `dir` has no file name (`.`, `./`, a trailing `/.`), `$PWD`'s
/// basename wins if it names a catalog entry resolving to this same
/// directory: inside a symlinked look dir (`user-looks/linked ->
/// abc-mine-src`) the entry name (`linked`) is what the manifest must
/// match, not the store target's basename. Pure (no I/O past the catalog
/// it is handed), so tests pin `$PWD` explicitly.
fn pwd_entry_name(canonical: &Path, catalog: &Catalog) -> Option<String> {
    pwd_basename_for(std::env::var_os("PWD").as_deref(), canonical, catalog)
}

fn pwd_basename_for(
    pwd: Option<&std::ffi::OsStr>,
    canonical: &Path,
    catalog: &Catalog,
) -> Option<String> {
    let pwd = pwd?;
    let base = Path::new(pwd).file_name().and_then(|name| name.to_str())?;
    if base.is_empty() {
        return None;
    }
    let entry = catalog.find(base)?;
    if entry.dir == canonical {
        Some(base.to_owned())
    } else {
        None
    }
}

/// Validates a source directory without touching anything. A directory with
/// `extends` resolves its base through the catalog; without one it resolves
/// alone, like `scoot-look build`.
pub fn validate_dir(
    dir: &Path,
    origin: Origin,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
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
    // never the store target's basename. `.`, `./` and paths ending in
    // `/.` have no file name: prefer `$PWD`'s basename when it names a
    // catalog entry resolving to this same directory (a symlinked look
    // dir, e.g. `user-looks/linked -> abc-mine-src`, validated from
    // inside), else fall back to the canonical basename (the cwd's name
    // for `.`).
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| pwd_entry_name(&canonical, catalog))
        .or_else(|| {
            canonical
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if let Some(error) = scoot_look::check_name_matches_dir(&raw, &dir_name) {
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
    if let Ok(inputs) = check_inputs(&chain, &merged, checks, &mut validation) {
        check_bar_fragment(
            &merged.name,
            &merged,
            &inputs.inputs,
            checks,
            &mut validation,
        );
        check_foot_fragment(&merged.name, &merged, checks, &mut validation);
        check_leaf_fragments(&merged.name, &merged, &mut validation);
        check_static_fragments(&chain, trust, &mut validation);
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
            dir: dir.clone(),
            // A directly-named directory is read without following
            // symlinks, so its manifest is the regular file inside it.
            manifest_dir: dir,
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
    read_regular_capped(path, scoot_look::state::MAX_FILE)
}

/// Reads one file like [`read_regular`] but with the caller's byte cap
/// (the install gate's per-kind caps): opens `O_NONBLOCK | O_NOFOLLOW`,
/// re-checks the handle is a regular file with a single link, and reads
/// at most `cap + 1` bytes so a header that lied about a small size
/// refuses instead of landing outside bytes.
pub fn read_regular_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let refusal = |why: &str| format!("refused: {} is {why}", path.display());
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    if meta.is_symlink() {
        return Err(refusal("a symlink"));
    }
    if !meta.is_file() {
        return Err(refusal("not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.nlink() != 1 {
            return Err(refusal(
                "a hard link: installed looks carry regular files only",
            ));
        }
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
    if stat.st_nlink != 1 {
        return Err(refusal(
            "a hard link: installed looks carry regular files only",
        ));
    }
    if stat.st_size as u64 > cap {
        return Err(format!(
            "refused: {} is larger than {cap} bytes",
            path.display(),
        ));
    }
    let mut bytes = Vec::new();
    use std::io::Read as _;
    std::fs::File::from(fd)
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!(
            "refused: {} is larger than {cap} bytes",
            path.display(),
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

/// Caps `text` at `max` bytes for an error line, keeping the head (where
/// the refusal names itself) and marking the cut. Messages here are a few
/// lines; this only bounds a backend gone wrong.
fn truncate_head(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    // Cut at a character boundary, never inside one.
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &text[..end])
}

/// A unique scratch dir under the system temp dir, removed on drop: every
/// exit path (including `?` and a failed check) cleans the stage, and a
/// crash never leaves tool droppings in the user's cwd or `--out`.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(tag: &str) -> Result<Self, String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // A SIGTERM/SIGINT never runs `Drop`, so a killed apply leaves its
        // stage behind: sweep other dead pids' stages at start (cheap: one
        // temp-dir listing per staging, and only `scoot-look-*` names).
        cleanup_stale_stages();
        // pid plus a per-process counter plus nanos: two applies (or two
        // threads) never share a temp name, and a planted symlink at a
        // guessable name is never opened (`create_dir` refuses it instead
        // of writing through one).
        let dir = std::env::temp_dir().join(format!(
            "scoot-look-{tag}.{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        std::fs::create_dir(&dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        // The stage holds rendered fragments: private from the start, never
        // relying on the umask.
        #[cfg(unix)]
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .map_err(|error| format!("cannot secure {}: {error}", dir.display()))?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

/// Removes `scoot-look-<tag>.<pid>-*` stage dirs whose owner pid is dead.
/// A signal-killed run never reaches `Drop`, so without this sweep every
/// SIGTERM/SIGINT leaks one stage dir. The sweep runs at the next staging
/// (the next `apply`, `build` or `validate` that creates a `ScratchDir`),
/// not on its own timer. Only dead pids (never the current one, never a
/// live one: pid reuse errs toward keeping), only directories owned by the
/// same uid, failures ignored. Linux probes `/proc`; elsewhere there is no
/// cheap dead-pid check without a new dependency, so the sweep keeps
/// (rather than deletes) there.
///
/// Caveat: the liveness check keys on `/proc/<pid>`, so another PID
/// namespace sharing `/tmp` could read a live stage's pid as dead and
/// remove a live stage. Same uid only narrows it, it does not close it:
/// keep `/tmp` private to the PID namespace, or accept the small risk.
fn cleanup_stale_stages() {
    let entries = match std::fs::read_dir(std::env::temp_dir()) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mine = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix("scoot-look-") else {
            continue;
        };
        let Some(dot) = rest.find('.') else { continue };
        let pid: u32 = match rest[dot + 1..].split('-').next().unwrap_or("").parse() {
            Ok(pid) => pid,
            Err(_) => continue,
        };
        if pid == mine || !pid_dead(pid) {
            continue;
        }
        // Same uid only: never sweep another user's stage, even when its
        // pid reads as dead here.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let owned = std::fs::metadata(entry.path())
                .map(|meta| meta.uid() == rustix::process::geteuid().as_raw())
                .unwrap_or(false);
            if !owned {
                continue;
            }
        }
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// Whether `pid` no longer exists (a reused pid reads as live: the sweep
/// keeps its dir, which is the safe direction).
fn pid_dead(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        !std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Renders the bar fragment and runs the installed binary's own parser over
/// it — the same check `apply` and `build` run, so `validate` and `apply`
/// agree on the bar. A missing binary is a note. On a missing-feature
/// refusal the fragment re-renders without what the installed bar lacks
/// and checks again, as `apply` does: first the `popup` keys, then whole
/// module sections (a minimal bar answers `unknown field inactive-color,
/// there are no fields` for `[workspaces]`). Refusals land in `errors`.
fn check_bar_fragment(
    name: &str,
    merged: &scoot_look::ResolvedLook,
    inputs: &RenderInputs,
    checks: &dyn Checks,
    validation: &mut Validation,
) {
    let scratch = match ScratchDir::new("validate") {
        Ok(scratch) => scratch,
        Err(error) => {
            validation.errors.push(error);
            return;
        }
    };
    let path = scratch.path().join("bar.toml");
    let mut render_inputs = inputs.clone();
    let mut fragment = match render_bar(merged, &render_inputs) {
        Ok(bar) => bar,
        Err(error) => {
            validation
                .errors
                .push(format!("cannot render {name}: {error}"));
            return;
        }
    };
    let mut drops: Vec<BarDrop> = Vec::new();
    loop {
        if let Err(error) = std::fs::write(&path, &fragment) {
            validation.errors.push(format!(
                "cannot stage the bar fragment {}: {error}",
                path.display()
            ));
            return;
        }
        match checks.bar_fragment(&path) {
            Ok(Availability::Available(())) => {
                for drop_ in &drops {
                    validation.notes.push(drop_.validated_note());
                }
                return;
            }
            Ok(Availability::Unavailable(note)) => {
                validation.notes.push(note);
                return;
            }
            Err(message) => {
                let Some(drop_) = bar_downgrade(&message, &fragment) else {
                    validation.errors.push(message);
                    return;
                };
                if !drop_.apply_to(&mut render_inputs) {
                    validation.errors.push(message);
                    return;
                }
                match render_bar(merged, &render_inputs) {
                    Ok(next) => {
                        drops.push(drop_);
                        fragment = next;
                    }
                    Err(error) => {
                        validation
                            .errors
                            .push(format!("cannot render {name}: {error}"));
                        return;
                    }
                }
            }
        }
    }
}

/// Renders foot's fragment and runs the installed binary's own parser over
/// it — the same check `apply` and `build` run, so `validate` and `apply`
/// agree on foot. A missing binary is a note. Looks without a `[terminal]`
/// table render no fragment and check nothing. Refusals land in `errors`.
fn check_foot_fragment(
    name: &str,
    merged: &scoot_look::ResolvedLook,
    checks: &dyn Checks,
    validation: &mut Validation,
) {
    let foot = match render_foot(merged) {
        Ok(Some(foot)) => foot,
        Ok(None) => return,
        Err(error) => {
            validation
                .errors
                .push(format!("cannot render {name}: {error}"));
            return;
        }
    };
    let scratch = match ScratchDir::new("validate") {
        Ok(scratch) => scratch,
        Err(error) => {
            validation.errors.push(error);
            return;
        }
    };
    let path = scratch.path().join("foot.ini");
    if let Err(error) = std::fs::write(&path, &foot) {
        validation.errors.push(format!(
            "cannot stage the foot fragment {}: {error}",
            path.display()
        ));
        return;
    }
    match checks.foot_fragment(&path) {
        Ok(Availability::Available(())) => {}
        Ok(Availability::Unavailable(note)) => validation.notes.push(note),
        Err(message) => validation.errors.push(message),
    }
}

/// Renders the launcher, locker and greeter leaves and reparses each —
/// the same fragments `apply` and `build` write, so `validate` and
/// `apply` agree on them. The reparse is the check: neither line-based
/// format has a check the tool can always run, and the greeter and
/// launcher fragments have no app parser yet (scootgreet is later;
/// scootlaunch reads its fragment live, so a bad render would break
/// every open), so a fragment that does not read back as the pairs or
/// table it wrote names a renderer bug. Refusals land in `errors`.
fn check_leaf_fragments(
    name: &str,
    merged: &scoot_look::ResolvedLook,
    validation: &mut Validation,
) {
    for (file, fragment) in [
        ("fuzzel.ini", render_fuzzel(merged)),
        ("swaylock", render_swaylock(merged)),
    ] {
        let fragment = match fragment {
            Ok(fragment) => fragment,
            Err(error) => {
                validation
                    .errors
                    .push(format!("cannot render {name}: {error}"));
                return;
            }
        };
        if let Err(error) = parse_kv_config(&fragment) {
            validation
                .errors
                .push(format!("rendered {file} does not parse: {error}"));
            return;
        }
    }
    for (file, fragment) in [
        ("greet.toml", render_greet(merged)),
        ("launch.toml", render_launch(merged)),
        ("notify.toml", render_notify(merged)),
    ] {
        let fragment = match fragment {
            Ok(fragment) => fragment,
            Err(error) => {
                validation
                    .errors
                    .push(format!("cannot render {name}: {error}"));
                return;
            }
        };
        if let Err(error) = fragment.parse::<toml::Table>() {
            validation
                .errors
                .push(format!("rendered {file} does not parse: {error}"));
        }
    }
}

/// What the installed bar lacks, dropped from the re-render the way the
/// `popup` keys are: `popup-radius` for a bar built without `popup`, or a
/// whole module section (the `[workspaces]` table on a minimal bar) for a
/// build without that module's feature.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BarDrop {
    /// A bar without `popup`: re-render with `bar_popup = false`.
    Popup,
    /// A build without the module behind `section`: skip that override
    /// section. `field` is the refused key the re-render drops with it,
    /// for the note.
    Section { section: String, field: String },
}

impl BarDrop {
    /// Drops this from `inputs`: false when already dropped (the refusal
    /// repeats: a genuine error, not a downgrade).
    pub(crate) fn apply_to(&self, inputs: &mut RenderInputs) -> bool {
        match self {
            BarDrop::Popup => {
                if !inputs.bar_popup {
                    return false;
                }
                inputs.bar_popup = false;
                true
            }
            BarDrop::Section { section, .. } => {
                if inputs.bar_drop_sections.contains(section) {
                    return false;
                }
                inputs.bar_drop_sections.push(section.clone());
                true
            }
        }
    }

    fn validated_note(&self) -> String {
        match self {
            BarDrop::Popup => "bar without `popup`: validated without popup-radius".to_owned(),
            BarDrop::Section { section, field } => {
                format!("bar without `{section}`: validated without {section} tokens ({field})")
            }
        }
    }

    pub(crate) fn rendered_note(&self) -> String {
        match self {
            BarDrop::Popup => "bar without `popup`: rendered without popup-radius".to_owned(),
            BarDrop::Section { section, field } => {
                format!("bar without `{section}`: rendered without {section} tokens ({field})")
            }
        }
    }
}

/// Whether a `bar_fragment` refusal is the missing-`popup` shape (a bar
/// built `--no-default-features` refusing `popup-radius`): the caller
/// re-renders with `bar_popup = false` instead of refusing every look.
fn is_popup_refusal(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("popup-radius") || (lower.contains("popup") && lower.contains("unknown field"))
}

/// The `unknown field \`name\`` a deny-unknown-fields parser names, if the
/// refusal carries one (the whole stderr, not just its first line: the key
/// sits past the `TOML parse error at ...` summary).
fn unknown_field(message: &str) -> Option<String> {
    let at = message.find("unknown field")?;
    let rest = &message[at..];
    let open = rest.find('`')?;
    let close = rest[open + 1..].find('`')?;
    let field = &rest[open + 1..open + 1 + close];
    if field.is_empty() {
        return None;
    }
    Some(field.to_owned())
}

/// The `TOML parse error at line N, column M` the refusal carries, if it
/// names one (the whole stderr: the summary line holds the span, the
/// unknown-field detail sits past it).
fn error_line(message: &str) -> Option<usize> {
    let at = message.find("line ")?;
    let rest = &message[at + "line ".len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

/// The table header at or preceding `line` (1-based) in the rendered
/// fragment: the module whose key the parser refused. Headers render as
/// `[section]` (bare module names), one per line.
fn section_at(fragment: &str, line: usize) -> Option<String> {
    let mut section: Option<String> = None;
    for (index, raw) in fragment.lines().enumerate() {
        if index + 1 > line {
            break;
        }
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix('[')
            && let Some(end) = rest.find(']')
        {
            let name = rest[..end].trim().trim_matches('"').to_owned();
            if !name.is_empty() {
                section = Some(name);
            }
        }
    }
    section
}

/// Whether `table` holds `field` at any depth (rendered override keys are
/// flat, but a dotted key nests one level in the parsed fragment).
fn table_holds(table: &toml::Table, field: &str) -> bool {
    for value in table.values() {
        if let toml::Value::Table(inner) = value
            && (inner.contains_key(field) || table_holds(inner, field))
        {
            return true;
        }
    }
    false
}

/// The downgrade a `bar_fragment` refusal asks for, if it names something
/// droppable: the missing-`popup` shape first, else the section the error's
/// line/column points at (the table header at or preceding the span in the
/// rendered text). Allowlisted keys collide across sections (`*.icon` in
/// `clock`, `window-title`, `volume`, ...), so the first BTreeMap holder is
/// the wrong answer: a clock-only bar refusing `window-title`'s `icon` must
/// drop `window-title`, never `clock`. `[colors]` and `[bar]` are never
/// dropped (a refusal naming one of their keys is genuine); anything else
/// the fragment carries is an override section for a module the installed
/// bar may lack, and dropping it is what lets a shipped look apply on the
/// lightest bar. Refusals without a parseable line fall back to the
/// first-holder search.
pub(crate) fn bar_downgrade(message: &str, fragment: &str) -> Option<BarDrop> {
    if is_popup_refusal(message) {
        return Some(BarDrop::Popup);
    }
    let field = unknown_field(message)?;
    let table: toml::Table = fragment.parse().ok()?;
    if field == "colors" || field == "bar" {
        return None;
    }
    if let Some(line) = error_line(message)
        && let Some(section) = section_at(fragment, line)
    {
        if section == "colors" || section == "bar" {
            return None;
        }
        if table.contains_key(&section) {
            let holds = match table.get(&section) {
                Some(toml::Value::Table(inner)) => {
                    inner.contains_key(&field) || table_holds(inner, &field)
                }
                _ => false,
            };
            // The unknown-table shape names the table itself
            // (`unknown field `volume`` at `[volume]`): the section
            // cannot hold itself, so equality accepts it.
            if holds || section == field {
                return Some(BarDrop::Section { section, field });
            }
            return None;
        }
        return None;
    }
    if table.contains_key(&field) {
        return Some(BarDrop::Section {
            section: field.clone(),
            field,
        });
    }
    for (section, value) in &table {
        if section == "colors" || section == "bar" {
            continue;
        }
        let holds = matches!(value, toml::Value::Table(inner) if inner.contains_key(&field) || table_holds(inner, &field));
        if holds {
            return Some(BarDrop::Section {
                section: section.clone(),
                field,
            });
        }
    }
    None
}

/// One link of an `extends` chain, base first.
pub(crate) struct ChainEntry {
    pub(crate) entry: Entry,
    /// The `look.toml` bytes (the winner's, from the catalog).
    pub(crate) bytes: Vec<u8>,
}

/// Collects the `extends` chain for `name`, base first, following the same
/// rules `resolve` enforces (unknown bases, cycles, depth, installed
/// bases): the provenance walk the render inputs need (a wallpaper image
/// resolves against the directory of the look that names it, not the
/// child that inherits it).
pub(crate) fn collect_chain(catalog: &Catalog, name: &str) -> Result<Vec<ChainEntry>, String> {
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
pub(crate) struct Inputs {
    pub(crate) inputs: RenderInputs,
    /// Files consulted, for the notes.
    pub(crate) details: Vec<String>,
}

/// Resolves the render inputs and runs the effectful checks that need only
/// the sources (fonts, wallpaper). The bar-fragment check runs later,
/// against the rendered file.
pub(crate) fn check_inputs(
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
                            Err(error) => validation
                                .errors
                                .push(format!("font `{family}` ({key}): {error}")),
                        }
                    }
                }
                Err(error) => validation
                    .errors
                    .push(format!("font `{family}` ({key}): {error}")),
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
    if let Some((dir, image, sha256)) = wallpaper_image(chain) {
        if image.starts_with("https://") {
            // A URL wallpaper resolves from the download cache only:
            // `install` fetched and verified it at install time, and a
            // swap never makes a network request. Anything uncached
            // refuses here (it never fetches).
            match checks.url_wallpaper(&image, sha256.as_deref()) {
                Ok(Availability::Available(path)) => {
                    details.push(format!("wallpaper: {image} (cached)"));
                    wallpaper_abs = Some(path);
                }
                Ok(Availability::Unavailable(note)) => {
                    validation.notes.push(note);
                }
                Err(refusal) => validation.errors.push(refusal),
            }
        } else {
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
                        Err(error) => validation
                            .errors
                            .push(format!("wallpaper `{image}`: {error}")),
                    }
                }
                Err(error) => validation
                    .errors
                    .push(format!("wallpaper `{image}`: {error}")),
            }
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
            bar_drop_sections: Vec::new(),
            omit_cursor: false,
        },
        details,
    })
}

/// The `[fonts.files]` entry for `key` from the chain's last look that
/// names one, with the manifest directory that names it.
fn carrier_file(chain: &[ChainEntry], key: &str) -> Option<(PathBuf, String)> {
    let mut found = None;
    for link in chain {
        if let Some(rel) = link.entry.raw.fonts.files.get(key) {
            found = Some((link.entry.manifest_dir.clone(), rel.clone()));
        }
    }
    found
}

/// The wallpaper image from the chain's last look that names a
/// `[wallpaper]` table with one: the manifest directory that names it, the
/// image (a relative path or an `https://` URL), and that table's `sha256`
/// pin, if any.
fn wallpaper_image(chain: &[ChainEntry]) -> Option<(PathBuf, String, Option<String>)> {
    let mut found = None;
    for link in chain {
        if let Some(wallpaper) = &link.entry.raw.wallpaper
            && let Some(image) = &wallpaper.image
        {
            found = Some((
                link.entry.manifest_dir.clone(),
                image.clone(),
                wallpaper.sha256.clone(),
            ));
        }
    }
    found
}

/// The static terminal leaves for a chain: each `apps/` file from the
/// chain's last look whose source directory carries it (inherited unless
/// the child names its own, like `[fonts.files]` and the wallpaper
/// image), read capped and shape-checked. Returns the rendered relative
/// paths with their bytes.
///
/// An installed (someone else's) look drops them without `trust`: its
/// formats are never generated, so a hostile file could carry anything
/// (starship custom commands run a shell). With `trust` they render and
/// each prints into `notes` first, so the reply shows what was trusted.
/// A look with no `apps/` file renders none, whatever the origin.
pub(crate) fn static_leaf_files(
    chain: &[ChainEntry],
    trust: bool,
    notes: &mut Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    use scoot_look::{STATIC_LEAVES, check_static_leaf};
    let Some(root) = chain.last() else {
        return Ok(Vec::new());
    };
    let name = root.entry.name.as_str();
    if root.entry.origin == Origin::Installed && !trust {
        let mut dropped = Vec::new();
        for leaf in STATIC_LEAVES {
            if static_leaf_source(chain, leaf.path).is_some() {
                dropped.push(leaf.path.to_owned());
            }
        }
        if !dropped.is_empty() {
            dropped.sort();
            notes.push(format!(
                "dropping {}: `{name}` is an installed look (pass --trust to keep them)",
                dropped.join(", ")
            ));
        }
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for leaf in STATIC_LEAVES {
        let Some(dir) = static_leaf_source(chain, leaf.path) else {
            continue;
        };
        let path = contained_file(&dir, leaf.path)?;
        // Capped at read time and required UTF-8, like every other
        // look file read.
        let text = read_capped_follow(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let bytes = text.into_bytes();
        check_static_leaf(leaf.path, &bytes)
            .map_err(|error| format!("cannot render {name}: {error}"))?;
        if root.entry.origin == Origin::Installed {
            notes.push(format!(
                "trusted static leaf {} ({len} bytes):\n{contents}",
                leaf.path,
                len = bytes.len(),
                contents = String::from_utf8_lossy(&bytes),
            ));
        }
        files.push((leaf.path.to_owned(), bytes));
    }
    Ok(files)
}

/// The manifest directory of the chain's last look carrying the `apps/`
/// file at `rel`, if any. Existence only (no read): the caller resolves
/// and reads through [`contained_file`], so an escape refuses there.
fn static_leaf_source(chain: &[ChainEntry], rel: &str) -> Option<PathBuf> {
    let mut found = None;
    for link in chain {
        if link.entry.manifest_dir.join(rel).is_file() {
            found = Some(link.entry.manifest_dir.clone());
        }
    }
    found
}

/// Validates the static terminal leaves the way `apply` renders them, so
/// `validate` and `apply` agree: readability plus shape, under the same
/// trust rule. Refusals land in `errors`; the trust drop and the trusted
/// contents land in `notes`.
fn check_static_fragments(chain: &[ChainEntry], trust: bool, validation: &mut Validation) {
    let mut notes = Vec::new();
    match static_leaf_files(chain, trust, &mut notes) {
        Ok(_) => validation.notes.extend(notes),
        Err(error) => validation.errors.push(error),
    }
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
pub(crate) fn path_to_string(path: &Path) -> Result<String, String> {
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
/// cached directory. Part of the cache key. 3 adds foot's `foot.ini`; 4
/// selects the look's polarity as foot's initial theme and carries the
/// hues in both `colors-dark` and `colors-light`; 5 adds the toolkit
/// fragments (`gtk/gtk-3.0.css`, `gtk/gtk-4.0.css`,
/// `qt/scoot-look.conf`); 6 adds the daemon
/// leaves (`mako`, `wob.ini`, `fcitx5/theme.conf`, `nightlight`); 7 adds
/// the launcher and locker leaves (`fuzzel.ini`, `swaylock`); 8 honors
/// the cursor/fonts target opt-outs (an opted-out cursor renders no
/// cursor keys, opted-out fonts no bar `font` key — each keys apart
/// through the inputs, so the bump only retires pre-opt-out caches); 9
/// adds the greeter fragment (`greet.toml`, read at system build time
/// only, never live); 10 adds the terminal static leaves
/// (`apps/helix.toml`, `apps/btop.theme`, `apps/starship.toml`, dropped
/// from installed looks without `--trust`); 11 adds the launcher
/// fragment (`launch.toml`, read at every panel open); 12 adds the
/// notification fragment (`notify.toml`, read at daemon start and on
/// re-theme).
pub const RENDER_VERSION: u32 = 12;

/// The cache key for a render: the chain's names and bytes plus the
/// resolved inputs (wallpaper and font paths with mtimes, the bar's popup
/// feature and dropped module sections), hashed like `source_hash`. A font
/// package update or an edited source re-renders instead of reusing a stale
/// directory.
fn cache_key(
    chain: &[ChainEntry],
    inputs: &RenderInputs,
    trust: bool,
    static_files: &[(String, Vec<u8>)],
) -> String {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&RENDER_VERSION.to_le_bytes());
    buffer.push(0);
    for link in chain {
        buffer.extend_from_slice(link.entry.name.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(&link.bytes);
        buffer.push(0);
    }
    // The static leaves ride the key by bytes: an edited `apps/` file
    // re-renders instead of reusing the stale directory (look.toml alone
    // cannot tell them apart).
    for (file, bytes) in static_files {
        buffer.extend_from_slice(file.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(bytes);
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
    // An opted-out cursor renders a different fragment (no cursor keys),
    // so it keys apart from the full render of the same sources.
    buffer.push(if inputs.omit_cursor { 1 } else { 0 });
    // The static terminal leaves ride the key by origin and trust: an
    // installed look renders them only with `--trust`, so the two
    // renders must never share a directory.
    buffer.push(match chain.last().map(|link| link.entry.origin) {
        Some(Origin::Shipped) => 0,
        Some(Origin::Declared) => 1,
        Some(Origin::User) => 2,
        Some(Origin::Installed) => 3,
        None => 4,
    });
    buffer.push(if trust { 1 } else { 0 });
    let mut dropped = inputs.bar_drop_sections.clone();
    dropped.sort();
    for section in &dropped {
        buffer.extend_from_slice(section.as_bytes());
        buffer.push(0);
    }
    buffer.push(0);
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

/// Every fragment one render writes, owned: the per-app configs plus the
/// daemon leaves, the greeter and launcher fragments, the terminal
/// static leaves and the identity document. Both cache slots (the
/// content-keyed one and `watch`'s reused one) render through this, so a
/// watched save renders exactly what an apply would.
struct RenderedFragments {
    scoot: String,
    bar: String,
    gtk3: String,
    gtk4: String,
    qt: String,
    foot: Option<String>,
    fuzzel: String,
    swaylock: String,
    greet: String,
    launch: String,
    notify: String,
    rendered_json: String,
    leaves: Vec<(String, String)>,
    static_files: Vec<(String, Vec<u8>)>,
}

/// Renders every fragment for `name`, and proves the TOML ones parse (the
/// renderers quote, and this is where a quoting failure shows itself, before
/// any app reads them). `gtk.css` and the Qt scheme have no strict parser
/// handy; their renderers only interpolate `#rrggbb` tokens and are pinned
/// by unit tests instead. `static_files` are the already-checked terminal
/// static leaves (verbatim bytes); the caller computed them with
/// [`static_leaf_files`] so their notes ride the caller's notes.
fn render_all_fragments(
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    inputs: &RenderInputs,
    static_files: Vec<(String, Vec<u8>)>,
) -> Result<RenderedFragments, String> {
    let scoot =
        render_scoot(merged, inputs).map_err(|error| format!("cannot render {name}: {error}"))?;
    let bar =
        render_bar(merged, inputs).map_err(|error| format!("cannot render {name}: {error}"))?;
    let gtk3_css = render_gtk3_css(merged);
    let gtk4_css = render_gtk4_css(merged);
    let qt_scheme = render_qt_scheme(merged);
    let foot = render_foot(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let fuzzel = render_fuzzel(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let swaylock =
        render_swaylock(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let greet = render_greet(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let launch = render_launch(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let notify = render_notify(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    let leaves =
        render_leaf_files(merged).map_err(|error| format!("cannot render {name}: {error}"))?;
    for (file, text) in [
        ("scoot.toml", &scoot),
        ("bar.toml", &bar),
        ("greet.toml", &greet),
        ("launch.toml", &launch),
        ("notify.toml", &notify),
    ] {
        text.parse::<toml::Table>()
            .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
    }
    for (file, text) in [("fuzzel.ini", &fuzzel), ("swaylock", &swaylock)] {
        parse_kv_config(text)
            .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
    }
    let target_bytes = chain
        .last()
        .map(|link| link.bytes.as_slice())
        .unwrap_or_default();
    let rendered_json = render_json(name, &source_hash(target_bytes), inputs);
    Ok(RenderedFragments {
        scoot,
        bar,
        gtk3: gtk3_css,
        gtk4: gtk4_css,
        qt: qt_scheme,
        foot,
        fuzzel,
        swaylock,
        greet,
        launch,
        notify,
        rendered_json,
        leaves,
        static_files,
    })
}

/// Writes every fragment into `tmp` (an empty dir the caller owns) and syncs
/// each file, so the renamed directory is complete on disk. Every path here
/// is joined to the tool-owned temp dir, never attacker-controlled.
fn write_render_dir(tmp: &Path, fragments: &RenderedFragments) -> Result<(), std::io::Error> {
    let mut files: Vec<(&str, &str)> = vec![
        ("scoot.toml", fragments.scoot.as_str()),
        ("bar.toml", fragments.bar.as_str()),
        ("render.json", fragments.rendered_json.as_str()),
        ("gtk/gtk-3.0.css", fragments.gtk3.as_str()),
        ("gtk/gtk-4.0.css", fragments.gtk4.as_str()),
        ("qt/scoot-look.conf", fragments.qt.as_str()),
        ("fuzzel.ini", fragments.fuzzel.as_str()),
        ("swaylock", fragments.swaylock.as_str()),
        ("greet.toml", fragments.greet.as_str()),
        ("launch.toml", fragments.launch.as_str()),
        ("notify.toml", fragments.notify.as_str()),
    ];
    if let Some(foot) = &fragments.foot {
        files.push(("foot.ini", foot.as_str()));
    }
    for (file, contents) in &files {
        let path = tmp.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        std::fs::File::open(&path)?.sync_all()?;
    }
    for (file, contents) in &fragments.leaves {
        let path = tmp.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        std::fs::File::open(&path)?.sync_all()?;
    }
    // The terminal static leaves (`apps/*` needs its directory first,
    // like `fcitx5/`): verbatim bytes, already shape-checked.
    for (file, contents) in &fragments.static_files {
        let path = tmp.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        std::fs::File::open(&path)?.sync_all()?;
    }
    Ok(())
}

/// A render temp dir unique across processes and threads of this process:
/// two renders into one cache never share a temp name, and a planted
/// symlink at a guessable name is never opened (`create_dir` refuses it
/// instead of writing through one).
fn render_tmp_dir(cache: &Path, name: &str) -> PathBuf {
    static RENDER_COUNTER: AtomicU64 = AtomicU64::new(0);
    cache.join(format!(
        ".{name}.tmp.{}-{}-{}",
        std::process::id(),
        RENDER_COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ))
}

/// Whether the cached directory holds a complete render of `name`: its
/// `render.json` names this look and every expected file is present,
/// including the greeter, launcher and notification fragments and the expected
/// terminal static leaves (recomputed with `trust`, notes discarded:
/// completeness only).
fn cached_complete(
    target: &Path,
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    trust: bool,
) -> bool {
    let leaves = match render_leaf_files(merged) {
        Ok(leaves) => leaves,
        Err(_) => return false,
    };
    let mut discard = Vec::new();
    let static_files = match static_leaf_files(chain, trust, &mut discard) {
        Ok(files) => files,
        Err(_) => return false,
    };
    let render = target.join("render.json");
    let identity: serde_json::Value = read_capped(&render)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    identity.get("name").and_then(|n| n.as_str()) == Some(name)
        && target.join("scoot.toml").is_file()
        && target.join("bar.toml").is_file()
        && target.join("gtk/gtk-3.0.css").is_file()
        && target.join("gtk/gtk-4.0.css").is_file()
        && target.join("qt/scoot-look.conf").is_file()
        && (merged.terminal.is_none() || target.join("foot.ini").is_file())
        && target.join("fuzzel.ini").is_file()
        && target.join("swaylock").is_file()
        && target.join("greet.toml").is_file()
        && target.join("launch.toml").is_file()
        && target.join("notify.toml").is_file()
        && leaves.iter().all(|(file, _)| target.join(file).is_file())
        && static_files
            .iter()
            .all(|(file, _)| target.join(file).is_file())
}

/// Renders the look into the cache: `<cache>/<name>-<hash>/` with
/// `scoot.toml`, `bar.toml`, the toolkit fragments
/// (`gtk/gtk-3.0.css`, `gtk/gtk-4.0.css`, `qt/scoot-look.conf`),
/// `foot.ini` (only when the look has a `[terminal]` table), the launcher
/// and locker leaves (`fuzzel.ini`, `swaylock`), the greeter fragment
/// (`greet.toml`, read at system build time only, never live), the
/// launcher fragment (`launch.toml`, read at every panel open), the
/// notification fragment (`notify.toml`, read at daemon start and on
/// re-theme),
/// `render.json` and the daemon leaves (`mako`,
/// `wob.ini`, `fcitx5/theme.conf`, `nightlight` when the look names a
/// temperature) and the terminal static leaves (`apps/helix.toml`,
/// `apps/btop.theme`, `apps/starship.toml`, each only when the look
/// carries it — and never from an installed look without `trust`),
/// written into a fresh directory and renamed, so a reader
/// racing the render sees the old or the new directory, never a half-written
/// one. An existing directory with the same key is reused after its
/// `render.json` names this look and every expected file is present.
pub(crate) fn render_cached(
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    inputs: &RenderInputs,
    cache: &Path,
    notes: &mut Vec<String>,
    trust: bool,
) -> Result<PathBuf, String> {
    let mut static_notes = Vec::new();
    let static_files = static_leaf_files(chain, trust, &mut static_notes)
        .map_err(|error| format!("cannot render {name}: {error}"))?;
    let target = cache.join(format!(
        "{name}-{}",
        cache_key(chain, inputs, trust, &static_files)
    ));
    if target.is_dir() {
        if cached_complete(&target, name, chain, merged, trust) {
            notes.extend(static_notes);
            notes.push(format!("reusing cached render {}", target.display()));
            return Ok(target);
        }
        std::fs::remove_dir_all(&target)
            .map_err(|error| format!("cannot clear {}: {error}", target.display()))?;
    }
    notes.extend(static_notes);
    let fragments = render_all_fragments(name, chain, merged, inputs, static_files)?;
    std::fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let tmp = render_tmp_dir(cache, name);
    // `create_dir` (not `create_dir_all`) refuses a planted symlink at the
    // temp path instead of writing through one.
    std::fs::create_dir(&tmp)
        .map_err(|error| format!("cannot create {}: {error}", tmp.display()))?;
    // The write: every file syncs before the rename, so the renamed
    // directory is complete on disk.
    let write = (|| {
        write_render_dir(&tmp, &fragments)?;
        std::fs::rename(&tmp, &target)?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = write {
        std::fs::remove_dir_all(&tmp).ok();
        // A concurrent apply won the rename: reuse its directory when it
        // is complete, else refuse.
        if cached_complete(&target, name, chain, merged, trust) {
            notes.push(format!("reusing cached render {}", target.display()));
            return Ok(target);
        }
        return Err(format!("cannot render into {}: {error}", target.display()));
    }
    Ok(target)
}

/// Renders the look into one reused cache slot (`<cache>/<slot>/`, where
/// `watch` passes `<name>-watch`): every save re-renders the same directory
/// instead of keying a new one per content hash, so the cache never grows
/// per save. The swap runs through a temp dir plus a `<slot>.old` aside
/// dance, so a reader racing the render sees the old or the new directory,
/// never a half-written one; a stale `.old` from a crashed render is cleared
/// first. The slot name never matches [`is_render_name`], so the prune never
/// collects it: the last good render stays until the next watched save.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_fixed_slot(
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    inputs: &RenderInputs,
    cache: &Path,
    slot: &str,
    notes: &mut Vec<String>,
    trust: bool,
) -> Result<PathBuf, String> {
    if slot.is_empty() || slot.contains('/') || slot.starts_with('.') {
        return Err(format!("refusing a cache slot outside the cache: `{slot}`"));
    }
    let target = cache.join(slot);
    let mut static_notes = Vec::new();
    let static_files = static_leaf_files(chain, trust, &mut static_notes)
        .map_err(|error| format!("cannot render {name}: {error}"))?;
    notes.extend(static_notes);
    let fragments = render_all_fragments(name, chain, merged, inputs, static_files)?;
    std::fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let tmp = render_tmp_dir(cache, name);
    // `create_dir` (not `create_dir_all`) refuses a planted symlink at the
    // temp path instead of writing through one.
    std::fs::create_dir(&tmp)
        .map_err(|error| format!("cannot create {}: {error}", tmp.display()))?;
    let write = (|| {
        write_render_dir(&tmp, &fragments)?;
        // Clear a stale aside from a crashed render before swapping.
        let old = cache.join(format!(".{slot}.old"));
        if old.is_dir() {
            std::fs::remove_dir_all(&old)?;
        } else if old.is_symlink() || old.is_file() {
            std::fs::remove_file(&old)?;
        }
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() && !meta.is_symlink() => {
                // `rename` cannot replace a non-empty dir: move it aside,
                // move the render in, then drop the aside.
                std::fs::rename(&target, &old)?;
                let swapped = std::fs::rename(&tmp, &target).and_then(|()| {
                    std::fs::remove_dir_all(&old)?;
                    Ok(())
                });
                if swapped.is_err() {
                    // The new render did not land: put the old one back so
                    // the slot still names the last good look.
                    std::fs::rename(&old, &target).ok();
                    swapped?;
                }
            }
            Ok(_) => {
                std::fs::remove_file(&target)?;
                std::fs::rename(&tmp, &target)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::rename(&tmp, &target)?;
            }
            Err(error) => return Err(error),
        }
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = write {
        std::fs::remove_dir_all(&tmp).ok();
        return Err(format!("cannot render into {}: {error}", target.display()));
    }
    notes.push(format!("rendered watched save into {}", target.display()));
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
pub(crate) fn live_render_targets() -> Vec<PathBuf> {
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
/// a crash between the two renames heals on the next repair. Shared with
/// `preview --capture`, which seeds a scratch state the same way.
pub(crate) fn point_link(dir: &Path, rendered: &str) -> Result<(), String> {
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

/// The config home for this apply: the injected `config` when the caller
/// pins one (every test does), else the live `$XDG_CONFIG_HOME`/`HOME`
/// fallback, exactly as before. The toolkit files and the shadowed-keys
/// read share this one resolution, so a pinned caller never touches the
/// real `~/.config`.
fn resolve_config_home(config: Option<&Path>) -> Option<PathBuf> {
    match config {
        Some(dir) => Some(dir.to_owned()),
        None => {
            let xdg = std::env::var_os("XDG_CONFIG_HOME");
            let home = std::env::var_os("HOME");
            config_home(xdg.as_deref(), home.as_deref())
        }
    }
}

/// One reload round's completed sets: the peers that may hold the new
/// layer, so the rollback re-runs them against the restored choice.
/// `failure` carries the refusal for the caller to roll back; the
/// completed sets ride beside it (a sent-but-unanswered peer may hold
/// the new layer, so the rollback re-runs it like a positive receipt).
pub(crate) struct PeerReload {
    pub completed_compositor: bool,
    pub completed_bars: Vec<usize>,
    pub failure: Option<ApplyFailure>,
}

/// Reloads, in order: the compositor, then every bar. Each refusal stops
/// the swap with the failure (the caller rolls back); a skipped app is a
/// note. A peer joins the completed sets on a positive receipt (it read
/// the new link and may hold the new layer, so the rollback re-runs it
/// against the restored choice), on a partial one (a `Reloaded` reply
/// with a non-empty `refused`: per-field refusals do not hold back the
/// applied fields), and on a sent-but-unanswered one (a 30 s timeout, a
/// connection closed after the write: it may have applied before hanging
/// or dying). Only an explicit error reply, the "predates" wall or a
/// never-sent request means "never accepted": re-running one of those
/// would only refuse twice (an old peer answers the rollback with the
/// same wall), so the rollback skips it. A scoped reload only reads the
/// link, so re-running one converges on the restored choice.
pub(crate) fn reload_peers(
    report: &mut ApplyReport,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
) -> PeerReload {
    let mut completed_compositor = false;
    let mut completed_bars: Vec<usize> = Vec::new();
    let failed = |message: String| ApplyFailure {
        message,
        rolled_back_to: None,
        rollback_errors: Vec::new(),
    };
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
                return PeerReload {
                    completed_compositor,
                    completed_bars,
                    failure: Some(failed(format!(
                        "the compositor refused the look reload: {}",
                        reloaded.refused.join("; ")
                    ))),
                };
            }
        }
        Err(ReloadError::NotRunning(note)) => {
            report.skipped.push(format!("compositor: {note}"));
        }
        Err(ReloadError::Refused(message)) => {
            // Refused (an explicit error reply, an old compositor's
            // unknown-tag wall, or a never-sent request): the peer never
            // accepted the new layer, so the rollback skips re-running it
            // rather than refusing twice.
            return PeerReload {
                completed_compositor,
                completed_bars,
                failure: Some(failed(message)),
            };
        }
        Err(ReloadError::Unanswered(message)) => {
            // Sent but unanswered (a timeout, a connection closed after the
            // write): it may hold the new layer, so the rollback re-runs
            // it against the restored choice, like a positive receipt.
            completed_compositor = true;
            return PeerReload {
                completed_compositor,
                completed_bars,
                failure: Some(failed(message)),
            };
        }
    }
    for (index, bar) in bars.iter().enumerate() {
        // A positive receipt completes the bar; an absent listener never
        // sends, so it is skipped with a note and completes nothing. A
        // sent-but-unanswered bar (a timeout, a connection closed after the
        // write: it may hold the new layer) completes like a positive
        // receipt, so the rollback re-runs it. A refusal (an explicit error
        // reply, an old bar's unknown-verb wall, a never-sent request, an
        // unreadable or unexpected reply) never accepted the new layer, so
        // the rollback skips re-running it rather than refusing twice —
        // and nothing is ever retried as a plain reload.
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
            Err(ReloadError::NotRunning(note)) => {
                report
                    .skipped
                    .push(format!("bar {}: {note}", bar.display()));
            }
            Err(ReloadError::Unanswered(message)) => {
                completed_bars.push(index);
                return PeerReload {
                    completed_compositor,
                    completed_bars,
                    failure: Some(failed(format!("bar {}: {message}", bar.display()))),
                };
            }
            Err(ReloadError::Refused(message)) => {
                return PeerReload {
                    completed_compositor,
                    completed_bars,
                    failure: Some(failed(format!("bar {}: {message}", bar.display()))),
                };
            }
        }
    }
    PeerReload {
        completed_compositor,
        completed_bars,
        failure: None,
    }
}

/// What one swap points the link at: a rendered look, or nothing.
pub(crate) enum SwapTarget {
    /// Point the link at this rendered directory and reload it in.
    Point(PathBuf),
    /// Remove the link and the record: no look layer at all (a null
    /// declared default).
    Remove,
}

/// One record-and-repoint with its reloads, shared by `apply` (a live
/// choice: point at a fresh render, carry the recorded default) and
/// `sync` (the declared default: point at the catalog's render, record
/// the file just read — or remove the layer for a null default).
pub(crate) struct SwapRequest {
    /// The look being pointed at, or the look being removed.
    pub name: String,
    /// Point or remove.
    pub target: SwapTarget,
    /// What recorded the choice (`apply`, `sync`).
    pub chosen_by: String,
    /// The declared default to record.
    pub declared_default: Option<String>,
    /// Skip every toolkit write (`--no-toolkit`).
    pub no_toolkit: bool,
    /// Skip the Helix signal (`--no-helix`).
    pub no_helix: bool,
    /// Skip the btop signal (`--no-btop`).
    pub no_btop: bool,
    /// dconf key paths home-manager owns: left alone however they read.
    pub nix_claims: Vec<String>,
    /// Apps whose Stylix target owns its path: the reply names those
    /// instead of pretending the look reached them.
    pub stylix_targets: Vec<String>,
    /// Theme targets the swap leaves alone (the Nix cursor/fonts
    /// opt-outs, honored by `sync`; `apply` never skips: a manual
    /// choice themes everything until the next managed sync).
    pub target_skips: TargetSkips,
}

/// Records, repoints and reloads one swap: the design's section 4.5 steps
/// 2-6. Rolls back on a refusal: the record and the link go back to
/// `previous` (or vanish, for a first choice), completed reloads re-run.
/// The lock is held and the link repaired by the caller; the report
/// carries the caller's validation notes in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn complete_swap(
    report: &mut ApplyReport,
    state_dir: &Path,
    cache: &Path,
    previous: Option<&Record>,
    swap: &SwapRequest,
    merged: Option<&scoot_look::ResolvedLook>,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    dconf: &dyn Dconf,
    leaves: &dyn super::leaves::Leaves,
    live: &[PathBuf],
) -> Result<(), ApplyFailure> {
    let refuse = |message: String| ApplyFailure {
        message,
        rolled_back_to: None,
        rollback_errors: Vec::new(),
    };
    let generation = previous
        .as_ref()
        .map(|record| record.generation + 1)
        .unwrap_or(1);
    let failed = Record {
        name: swap.name.clone(),
        rendered: match &swap.target {
            SwapTarget::Point(target) => path_to_string(target).map_err(refuse)?,
            SwapTarget::Remove => previous
                .as_ref()
                .map(|record| record.rendered.clone())
                .unwrap_or_default(),
        },
        generation,
        chosen_by: Some(swap.chosen_by.clone()),
        declared_default: swap.declared_default.clone(),
        toolkit: previous.as_ref().and_then(|record| record.toolkit),
        dconf_wrote: previous
            .as_ref()
            .map(|record| record.dconf_wrote.clone())
            .unwrap_or_default(),
        dconf_orig: previous
            .as_ref()
            .map(|record| record.dconf_orig.clone())
            .unwrap_or_default(),
    };
    match &swap.target {
        SwapTarget::Point(_) => {
            write_record(state_dir, &failed)
                .map_err(|error| refuse(format!("cannot record the choice: {error}")))?;
            // `failed.rendered` is absolute by construction; the record
            // carries what the link points at.
            point_link(state_dir, &failed.rendered)
                .map_err(|error| refuse(format!("cannot repoint the look link: {error}")))?;
        }
        SwapTarget::Remove => {
            // The link first, then the record: a crash between them
            // resurrects the old look at the next repair (record present,
            // link missing), which the next sync removes again — never a
            // link no record can repair. A link already gone is fine.
            match std::fs::remove_file(state_dir.join(LINK_NAME)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(refuse(format!(
                        "cannot remove {}: {error}",
                        state_dir.join(LINK_NAME).display()
                    )));
                }
            }
        }
    }
    let target = PathBuf::from(failed.rendered.clone());
    let reload = reload_peers(report, compositor, bars);
    if let Some(failure) = reload.failure {
        return rollback(
            state_dir,
            previous,
            &failed,
            reload.completed_compositor,
            &reload.completed_bars,
            compositor,
            bars,
            failure,
        )
        .map(|_| ());
    }
    match swap.target {
        SwapTarget::Point(_) => {
            // The toolkit step is best-effort: a missing `dconf`, a key
            // someone else owns, or a foreign config file is a note,
            // never a refusal. dconf keys first (recorded in
            // `look.json`), then the owned toolkit files (the `gtk.css`
            // shims, the inis, qt6ct, fontconfig), all through the state
            // link so a swap rewrites no file per look.
            if swap.no_toolkit {
                report
                    .notes
                    .push("toolkit step off for this swap (--no-toolkit)".to_owned());
            } else if let Some(merged) = merged {
                let mut record = failed.clone();
                report.notes.extend(apply_toolkit_with_claims(
                    state_dir,
                    &mut record,
                    merged,
                    dconf,
                    &swap.nix_claims,
                    swap.target_skips,
                ));
                if let Err(error) = write_record(state_dir, &record) {
                    report.notes.push(format!(
                        "cannot record the toolkit state: {error}: the look applied, the record lags"
                    ));
                }
                let link = state_dir.join(LINK_NAME);
                report.notes.extend(apply_toolkit_files(
                    state_dir,
                    &record,
                    merged,
                    resolve_config_home(config).as_deref(),
                    &link,
                    swap.target_skips,
                    &swap.stylix_targets,
                ));
            }
            // The daemon leaves are best-effort too, like the toolkit
            // step: each app's own reload signal for the swapped look.
            // A private state runs none of them (they would reach the
            // user's real session, not the test's).
            if let Some(merged) = merged {
                let leaf_report = super::leaves::apply_leaves(state_dir, merged, leaves);
                report.reloaded.extend(leaf_report.reloaded);
                report.notes.extend(leaf_report.notes);
            }
            // The terminal leaves are best-effort too: `SIGUSR1` to the
            // user's own `hx`, `SIGUSR2` to their `btop` (each opt-out),
            // and the starship next-prompt note. A private state runs no
            // signal (it would reach the user's real editors).
            if merged.is_some() {
                let terminal_report = super::terminals::apply_terminals(
                    state_dir,
                    &target,
                    leaves,
                    swap.no_helix,
                    swap.no_btop,
                    config,
                );
                report.reloaded.extend(terminal_report.reloaded);
                report.notes.extend(terminal_report.notes);
            }
            // Shadowed keys: the compositor's reply when it reloaded; else
            // the files, read directly.
            if let Some(merged) = merged {
                report.notes.extend(foot_notes(merged, state_dir, config));
            }
            if !reload.completed_compositor {
                let (shadowed, mut file_notes) = shadowed_from_files_in(&target, config);
                report.shadowed.extend(shadowed);
                report.notes.append(&mut file_notes);
                report.notes.push(
                    "compositor not running: shadowed keys read from the config files, not the live session".to_owned(),
                );
            }
        }
        SwapTarget::Remove => {
            // The layer is gone: restore the toolkit key the tool may
            // have written (best-effort, noted), then drop the record.
            // A private state never touches the real key at all.
            if swap.no_toolkit {
                report
                    .notes
                    .push("toolkit writes off for this swap (--no-toolkit)".to_owned());
            } else {
                let mut record = failed.clone();
                report.notes.extend(super::toolkit::toolkit_off_with_claims(
                    state_dir,
                    &mut record,
                    dconf,
                    &swap.nix_claims,
                ));
            }
            std::fs::remove_file(state_dir.join("look.json")).ok();
            report.removed = true;
            report
                .notes
                .push("look layer removed: the declared default is null".to_owned());
        }
    }
    report.shadowed.sort();
    report.shadowed.dedup();
    report.invalid.sort();
    report.invalid.dedup();
    // The previous render survives until the new one is confirmed: prune
    // only on success, never before a rollback. The keep-set spans every
    // live session's link (the cache is global, the state dirs are not),
    // or churning one session could prune another's render. A removed
    // layer keeps nothing back: its render rejoins the recency set.
    let mut keep = Vec::new();
    if matches!(swap.target, SwapTarget::Point(_)) {
        keep.push(target.clone());
        if let Some(previous) = &previous {
            keep.push(PathBuf::from(&previous.rendered));
        }
    }
    keep.extend(live.iter().cloned());
    prune_cache(cache, &keep, &mut report.notes);
    report.name = swap.name.clone();
    report.rendered = match &swap.target {
        SwapTarget::Point(_) => failed.rendered.clone(),
        SwapTarget::Remove => String::new(),
    };
    report.generation = generation;
    Ok(())
}

/// Where one swap renders from: a fresh cache render, the Nix
/// catalog's pre-rendered directory (verified by the caller before use),
/// or `watch`'s one reused cache slot (re-rendered every save, so the
/// cache never grows per save).
pub(crate) enum RenderSource {
    /// Render into the cache, reusing a matching render.
    Fresh,
    /// Use this catalog directory as the render.
    PreRendered { path: PathBuf },
    /// Render into `<cache>/<slot>/`, reusing the same directory every
    /// time (never pruned: the name never matches [`is_render_name`]).
    FixedSlot { slot: String },
}

/// Renders `name` for a swap from `initial` inputs: a verified
/// pre-rendered catalog directory when given, `watch`'s reused slot, else
/// the render cache — then the bar's own parser over the fragment, against
/// the installed binary's features, re-rendering without what the
/// installed bar lacks (see the downgrade note in `apply_with`).
/// `trust` keeps an installed look's static terminal leaves; without it
/// they are dropped (a managed sync never trusts).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_for_swap(
    name: &str,
    chain: &[ChainEntry],
    merged: &scoot_look::ResolvedLook,
    initial: RenderInputs,
    source: RenderSource,
    cache: &Path,
    checks: &dyn Checks,
    notes: &mut Vec<String>,
    trust: bool,
) -> Result<(PathBuf, RenderInputs), String> {
    let mut target = match &source {
        RenderSource::PreRendered { path } => path.clone(),
        RenderSource::Fresh => render_cached(name, chain, merged, &initial, cache, notes, trust)?,
        RenderSource::FixedSlot { slot } => {
            render_fixed_slot(name, chain, merged, &initial, cache, slot, notes, trust)?
        }
    };
    let mut render_inputs = initial;
    // The bar's own parser over the rendered fragment, against the
    // installed binary's features. A bar built `--no-default-features`
    // (no `popup`) refuses `popup-radius`, and a minimal bar refuses whole
    // module tables (`unknown field inactive-color, there are no fields`
    // for `[workspaces]`): re-render without what the installed bar lacks
    // rather than refusing every look, and say what was dropped.
    let mut drops: Vec<BarDrop> = Vec::new();
    loop {
        match checks.bar_fragment(&target.join("bar.toml")) {
            Ok(Availability::Available(())) => {
                for drop_ in &drops {
                    notes.push(drop_.rendered_note());
                }
                break;
            }
            Ok(Availability::Unavailable(note)) => {
                notes.push(note);
                break;
            }
            Err(message) => {
                let fragment = match std::fs::read_to_string(target.join("bar.toml")) {
                    Ok(fragment) => fragment,
                    Err(_) => return Err(message),
                };
                let Some(drop_) = bar_downgrade(&message, &fragment) else {
                    return Err(message);
                };
                if !drop_.apply_to(&mut render_inputs) {
                    return Err(message);
                }
                drops.push(drop_);
                target = match &source {
                    RenderSource::PreRendered { path } => path.clone(),
                    RenderSource::Fresh => {
                        render_cached(name, chain, merged, &render_inputs, cache, notes, trust)?
                    }
                    RenderSource::FixedSlot { slot } => render_fixed_slot(
                        name,
                        chain,
                        merged,
                        &render_inputs,
                        cache,
                        slot,
                        notes,
                        trust,
                    )?,
                };
            }
        }
    }
    // Foot's own parser over the rendered fragment, when the look renders
    // one. A missing foot is a note; a refusal refuses the swap before
    // anything is recorded, like the bar's genuine errors.
    if merged.terminal.is_some() {
        match checks.foot_fragment(&target.join("foot.ini")) {
            Ok(Availability::Available(())) => {}
            Ok(Availability::Unavailable(note)) => notes.push(note),
            Err(message) => return Err(message),
        }
    }
    Ok((target, render_inputs))
}
/// Applies `name`: the full transaction. Pure over its `catalog`, `cache`,
/// `state_dir`, `config` home and peers except the lock, the record, the
/// link, the cache and the reloads themselves. `config` pins the config
/// home (the toolkit files land there, the shadowed keys read there);
/// `None` resolves it from the live environment, for production. Other
/// live sessions' renders are kept via [`live_render_targets`] (read from
/// the environment here); tests use [`apply_with_live`] with an explicit
/// keep-set instead, so no test ever reads the real home or runtime dirs.
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
    leaves: &dyn super::leaves::Leaves,
) -> Result<ApplyReport, ApplyFailure> {
    let live = live_render_targets();
    apply_with_live(
        name, options, catalog, cache, state_dir, config, compositor, bars, checks, dconf, leaves,
        &live,
    )
}

/// [`apply_with`] with the other live sessions' renders passed in: the
/// keep-set is `target`, `previous`, plus `live`. Production passes
/// [`live_render_targets`]; tests pass `&[]` (their caches are scratch,
/// so nothing else can name a render in them).
#[allow(clippy::too_many_arguments)]
pub fn apply_with_live(
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
    leaves: &dyn super::leaves::Leaves,
    live: &[PathBuf],
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
    // A guarded revert (the detached `--for` child) lands here holding no
    // lock of its own; the check runs under this swap's lock, atomically
    // with the swap itself, so a concurrent newer apply can never slip
    // between the check and the repoint.
    if let Some(want) = options.only_if_generation {
        let have = previous.as_ref().map(|record| record.generation);
        if have != Some(want) {
            return Err(refuse(format!(
                "{PREVIEW_SUPERSEDED} the choice moved past generation {want} (now {}): not reverting",
                have.map_or("nothing".to_owned(), |generation| generation.to_string()),
            )));
        }
    }
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
        options.trust,
    )
    .map_err(refuse)?;
    // The bar's own parser over the rendered fragment, against the
    // installed binary's features. A bar built `--no-default-features`
    // (no `popup`) refuses `popup-radius`, and a minimal bar refuses whole
    // module sections (`unknown field inactive-color, there are no fields`
    // for `[workspaces]`): re-render without what the installed bar lacks
    // rather than refusing every look, and say what was dropped.
    let mut drops: Vec<BarDrop> = Vec::new();
    loop {
        match checks.bar_fragment(&target.join("bar.toml")) {
            Ok(Availability::Available(())) => {
                for drop_ in &drops {
                    report.notes.push(drop_.rendered_note());
                }
                break;
            }
            Ok(Availability::Unavailable(note)) => {
                report.notes.push(note);
                break;
            }
            Err(message) => {
                let fragment = match std::fs::read_to_string(target.join("bar.toml")) {
                    Ok(fragment) => fragment,
                    Err(_) => return Err(refuse(message)),
                };
                let Some(drop_) = bar_downgrade(&message, &fragment) else {
                    return Err(refuse(message));
                };
                if !drop_.apply_to(&mut render_inputs) {
                    return Err(refuse(message));
                }
                drops.push(drop_);
                target = render_cached(
                    name,
                    &chain,
                    &merged,
                    &render_inputs,
                    cache,
                    &mut report.notes,
                    options.trust,
                )
                .map_err(refuse)?;
            }
        }
    }
    // Foot's own parser over the rendered fragment, when the look renders
    // one. A missing foot is a note; a refusal refuses the swap before
    // anything is recorded, like the bar's genuine errors.
    if merged.terminal.is_some() {
        match checks.foot_fragment(&target.join("foot.ini")) {
            Ok(Availability::Available(())) => {}
            Ok(Availability::Unavailable(note)) => report.notes.push(note),
            Err(message) => return Err(refuse(message)),
        }
    }
    let generation = previous
        .as_ref()
        .map(|record| record.generation + 1)
        .unwrap_or(1);
    let mut record = Record {
        name: name.to_owned(),
        rendered: path_to_string(&target).map_err(refuse)?,
        generation,
        chosen_by: Some(
            if options.from_compositor {
                "compositor"
            } else {
                "apply"
            }
            .to_owned(),
        ),
        declared_default: previous
            .as_ref()
            .and_then(|record| record.declared_default.clone()),
        toolkit: previous.as_ref().and_then(|record| record.toolkit),
        dconf_wrote: previous
            .as_ref()
            .map(|record| record.dconf_wrote.clone())
            .unwrap_or_default(),
        dconf_orig: previous
            .as_ref()
            .map(|record| record.dconf_orig.clone())
            .unwrap_or_default(),
    };
    write_record(state_dir, &record)
        .map_err(|error| refuse(format!("cannot record the choice: {error}")))?;
    // `target` is absolute by construction (the cache dir resolves from
    // an absolute home); the record carries what the link points at.
    target = PathBuf::from(record.rendered.clone());
    point_link(state_dir, &record.rendered)
        .map_err(|error| refuse(format!("cannot repoint the look link: {error}")))?;
    // Reloads, in order: the compositor, then every bar. Each refusal
    // rolls the swap back; a skipped app is a note. A peer joins the
    // completed sets on a positive receipt (it read the new link and may
    // hold the new layer, so the rollback re-runs it against the restored
    // choice), on a partial one (a `Reloaded` reply with a non-empty
    // `refused`: per-field refusals do not hold back the applied fields),
    // and on a sent-but-unanswered one (a 30 s timeout, a connection
    // closed after the write: it may have applied before hanging or
    // dying). Only an explicit error reply, the "predates" wall or a
    // never-sent request means "never accepted": re-running one of those
    // would only refuse twice (an old peer answers the rollback with the
    // same wall), so the rollback skips it. A scoped reload only reads
    // the link, so re-running one converges on the restored choice.
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
            // Refused (an explicit error reply, an old compositor's
            // unknown-tag wall, or a never-sent request): the peer never
            // accepted the new layer, so the rollback skips re-running it
            // rather than refusing twice.
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
        Err(ReloadError::Unanswered(message)) => {
            // Sent but unanswered (a timeout, a connection closed after the
            // write): it may hold the new layer, so the rollback re-runs
            // it against the restored choice, like a positive receipt.
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
        // A positive receipt completes the bar; an absent listener never
        // sends, so it is skipped with a note and completes nothing. A
        // sent-but-unanswered bar (a timeout, a connection closed after the
        // write: it may hold the new layer) completes like a positive
        // receipt, so the rollback re-runs it. A refusal (an explicit error
        // reply, an old bar's unknown-verb wall, a never-sent request, an
        // unreadable or unexpected reply) never accepted the new layer, so
        // the rollback skips re-running it rather than refusing twice —
        // and nothing is ever retried as a plain reload.
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
            Err(ReloadError::NotRunning(note)) => {
                report
                    .skipped
                    .push(format!("bar {}: {note}", bar.display()));
            }
            Err(ReloadError::Unanswered(message)) => {
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
            Err(ReloadError::Refused(message)) => {
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
    // The toolkit step is best-effort: a missing `dconf`, a key someone
    // else owns, or a foreign config file is a note, never a refusal.
    // dconf keys first (recorded in `look.json`), then the owned toolkit
    // files (the `gtk.css` shims, the inis, qt6ct, fontconfig), all
    // through the state link so a swap rewrites no file per look.
    if options.no_toolkit {
        report
            .notes
            .push("toolkit step off for this apply (--no-toolkit)".to_owned());
    } else {
        report.notes.extend(apply_toolkit_with_claims(
            state_dir,
            &mut record,
            &merged,
            dconf,
            &options.nix_claims,
            TargetSkips::default(),
        ));
        if let Err(error) = write_record(state_dir, &record) {
            report.notes.push(format!(
                "cannot record the toolkit state: {error}: the look applied, the record lags"
            ));
        }
        let link = state_dir.join(LINK_NAME);
        report.notes.extend(apply_toolkit_files(
            state_dir,
            &record,
            &merged,
            resolve_config_home(config).as_deref(),
            &link,
            TargetSkips::default(),
            &options.stylix_targets,
        ));
    }
    // The daemon leaves are best-effort too: `makoctl reload`, the fcitx5
    // classic UI reload, and restarts of the OSD and night-light units.
    // A missing binary or a daemon that is not running is a note, and a
    // private state runs none of them (they would reach the user's real
    // session, not the test's).
    let leaf_report = super::leaves::apply_leaves(state_dir, &merged, leaves);
    report.reloaded.extend(leaf_report.reloaded);
    report.notes.extend(leaf_report.notes);
    // The terminal leaves are best-effort too: `SIGUSR1` to the user's
    // own `hx` and `SIGUSR2` to their `btop` (each opt-out, so an editor
    // mid-flow can sit a swap out), and the starship next-prompt note.
    // A private state runs no signal (it would reach the user's real
    // editors, not the test's).
    let terminal_report = super::terminals::apply_terminals(
        state_dir,
        &target,
        leaves,
        options.no_helix,
        options.no_btop,
        config,
    );
    report.reloaded.extend(terminal_report.reloaded);
    report.notes.extend(terminal_report.notes);
    // Shadowed keys: the compositor's reply when it reloaded; else the
    // files, read directly.
    report.notes.extend(foot_notes(&merged, state_dir, config));
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
    keep.extend(live.iter().cloned());
    prune_cache(cache, &keep, &mut report.notes);
    if options.preview {
        let planned = Preview {
            revert_to: previous.as_ref().map(|prior| prior.name.clone()),
            from_generation: generation,
        };
        if let Err(error) = write_preview(state_dir, &planned) {
            // The swap applied but its timer never armed: without the
            // file the look would stick past the delay, so roll back to
            // the previous choice loudly rather than keep a preview that
            // can never revert.
            let failure = ApplyFailure {
                message: format!("cannot arm the preview: {error}"),
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
        report.preview = Some(planned);
    } else if matches!(read_preview(state_dir), PreviewRead::Present(_)) {
        // A plain apply supersedes any pending preview (its generation
        // moved by construction): forget the dead file so `--keep` and a
        // late reverter never trip over it.
        clear_preview(state_dir);
    }
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
pub(crate) fn rollback(
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
                dconf_wrote: previous.dconf_wrote.clone(),
                dconf_orig: previous.dconf_orig.clone(),
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
            // previous layer to reload back to, so the apps that positively
            // reloaded still show the new look while `current` says none:
            // say so plainly instead of re-running reloads that would
            // refuse with "no look layer is configured". Peers that merely
            // refused are not named here: they never left the old layer.
            std::fs::remove_file(state_dir.join("look.json")).ok();
            std::fs::remove_file(state_dir.join(LINK_NAME)).ok();
            let holders = holders_text(completed_compositor, completed_bars, bars);
            if let Some(holders) = holders {
                failure.rollback_errors.push(format!(
                    "no previous look exists to roll back to: {holders} while no look is recorded"
                ));
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
            ReloadError::Refused(message) | ReloadError::Unanswered(message) => message,
        });
    }
    for index in completed_bars {
        // `Ok(false)` (absent at rollback) needs nothing: nothing there
        // holds the new layer.
        if let Some(bar) = bars.get(*index)
            && let Err(error) = bar.reload_look()
        {
            let message = match error {
                ReloadError::NotRunning(note) => {
                    format!("bar {} rollback skipped: {note}", bar.display())
                }
                ReloadError::Refused(message) | ReloadError::Unanswered(message) => {
                    format!("bar {} rollback: {message}", bar.display())
                }
            };
            failure.rollback_errors.push(message);
        }
    }
    Err(failure)
}

/// Names the peers that may hold the new look, for the first-apply
/// rollback note (`None` when nothing may: with no compositor and a bar
/// that merely refused, nothing reloaded and the note says nothing). A
/// positive receipt counts, and so do a partial one (a `Reloaded` reply
/// with a non-empty `refused`: the applied fields landed) and a
/// sent-but-unanswered one (a timeout, a close after the write): each may
/// hold the new layer. A peer that refused never left its old layer. The
/// verb agrees with the holder count.
fn holders_text(
    completed_compositor: bool,
    completed_bars: &[usize],
    bars: &[&dyn BarReload],
) -> Option<String> {
    let mut holders: Vec<String> = Vec::new();
    if completed_compositor {
        holders.push("the compositor".to_owned());
    }
    for index in completed_bars {
        if let Some(bar) = bars.get(*index) {
            holders.push(format!("bar {}", bar.display()));
        }
    }
    match holders.len() {
        0 => None,
        1 => Some(format!("{} still shows the new look,", holders[0])),
        _ => Some(format!(
            "{} still show the new look,",
            holders.join(" and ")
        )),
    }
}

/// The unknown-look refusal, naming what the catalog holds.
pub(crate) fn unknown_look(name: &str, catalog: &Catalog) -> String {
    let mut names: Vec<&str> = catalog
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    names.sort();
    if names.is_empty() {
        return format!("unknown look `{name}`: {}", super::empty_catalog_hint());
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
    let owned = config
        .map(Path::to_path_buf)
        .or_else(|| resolve_config_home(None));
    let config = owned.as_deref();
    let user_scoot = config.map(|base| base.join("scoot/config.toml"));
    let user_bar = config.map(|base| base.join("scoot/bar.toml"));
    let mut read_table = |path: &Path| -> Option<toml::Table> {
        // The user's own files: home-manager/stow link them into the
        // store, so symlinks are followed here. The no-follow policy is
        // for look-dir inputs, not the user's config.
        match read_capped_follow(path) {
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
/// touching state, the cache, or any app: `build` without `apply`. The
/// output file list and directory guard live in [`super::build_dir`].
pub fn build_to_out(
    target: &BuildTarget,
    out: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
) -> Result<BuildReport, String> {
    let (chain, merged, name) = match target {
        BuildTarget::Dir { path, origin } => {
            let validation = validate_dir(path, *origin, catalog, checks, trust)?;
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
    // The terminal static leaves render from the sources, not the
    // inputs: read once, outside the bar-downgrade re-render loop.
    let mut static_notes = Vec::new();
    let static_files = static_leaf_files(&chain, trust, &mut static_notes)
        .map_err(|error| format!("cannot build {name}: {error}"))?;
    validation.notes.extend(static_notes);
    /// One full render: the two first-party fragments, foot's `foot.ini`
    /// (only when the look has a `[terminal]` table), the launcher and
    /// locker leaves (`fuzzel.ini`, `swaylock`), the greeter fragment
    /// (`greet.toml`), the launcher fragment (`launch.toml`), the
    /// notification fragment (`notify.toml`), the
    /// record, and the daemon leaves (`fcitx5/theme.conf` under its
    /// directory).
    type FullRender = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<(String, String)>,
    );
    let render_once = |render_inputs: &RenderInputs| -> Result<FullRender, String> {
        let scoot = render_scoot(&merged, render_inputs)
            .map_err(|error| format!("cannot render {name}: {error}"))?;
        let bar = render_bar(&merged, render_inputs)
            .map_err(|error| format!("cannot render {name}: {error}"))?;
        let foot =
            render_foot(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let fuzzel =
            render_fuzzel(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let swaylock =
            render_swaylock(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let greet =
            render_greet(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let launch =
            render_launch(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let notify =
            render_notify(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        for (file, text) in [
            ("scoot.toml", &scoot),
            ("bar.toml", &bar),
            ("greet.toml", &greet),
            ("launch.toml", &launch),
            ("notify.toml", &notify),
        ] {
            text.parse::<toml::Table>()
                .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
        }
        for (file, text) in [("fuzzel.ini", &fuzzel), ("swaylock", &swaylock)] {
            parse_kv_config(text)
                .map_err(|error| format!("rendered {file} does not parse: {error}"))?;
        }
        let leaves =
            render_leaf_files(&merged).map_err(|error| format!("cannot render {name}: {error}"))?;
        let target_bytes = chain
            .last()
            .map(|link| link.bytes.as_slice())
            .unwrap_or_default();
        let rendered_json = render_json(&name, &source_hash(target_bytes), render_inputs);
        Ok((
            scoot,
            bar,
            foot,
            fuzzel,
            swaylock,
            greet,
            launch,
            notify,
            rendered_json,
            leaves,
        ))
    };
    // One staged list per render, so the bar downgrade loop below rebuilds
    // the same files it checks. The toolkit triple renders from tokens
    // alone (no resolved inputs), so it renders once, outside the loop.
    let gtk3_css = render_gtk3_css(&merged);
    let gtk4_css = render_gtk4_css(&merged);
    let qt_scheme = render_qt_scheme(&merged);
    let stage_of = |scoot: &str,
                    bar: &str,
                    foot: &Option<String>,
                    fuzzel: &str,
                    swaylock: &str,
                    greet: &str,
                    launch: &str,
                    notify: &str,
                    gtk3: &str,
                    gtk4: &str,
                    qt: &str,
                    json: &str,
                    leaves: &[(String, String)],
                    static_files: &[(String, Vec<u8>)]| {
        let mut staged = vec![
            ("scoot.toml".to_owned(), scoot.to_owned()),
            ("bar.toml".to_owned(), bar.to_owned()),
            ("render.json".to_owned(), json.to_owned()),
            ("gtk/gtk-3.0.css".to_owned(), gtk3.to_owned()),
            ("gtk/gtk-4.0.css".to_owned(), gtk4.to_owned()),
            ("qt/scoot-look.conf".to_owned(), qt.to_owned()),
            ("fuzzel.ini".to_owned(), fuzzel.to_owned()),
            ("swaylock".to_owned(), swaylock.to_owned()),
            ("greet.toml".to_owned(), greet.to_owned()),
            ("launch.toml".to_owned(), launch.to_owned()),
            ("notify.toml".to_owned(), notify.to_owned()),
        ];
        if let Some(foot) = foot {
            staged.push(("foot.ini".to_owned(), foot.clone()));
        }
        staged.extend(leaves.iter().cloned());
        // Already shape-checked UTF-8, so this copies verbatim.
        staged.extend(
            static_files
                .iter()
                .map(|(file, bytes)| (file.clone(), String::from_utf8_lossy(bytes).into_owned())),
        );
        staged
    };
    let (
        mut scoot,
        mut bar,
        mut foot,
        mut fuzzel,
        mut swaylock,
        mut greet,
        mut launch,
        mut notify,
        mut rendered_json,
        mut leaf_files,
    ) = render_once(&render_inputs)?;
    // The bar's own parser runs before anything lands in `--out`: a
    // refused fragment leaves no complete-looking output behind. The
    // check needs a file, so the fragments stage in a tool-owned temp dir
    // (gone on every exit path via `Drop`, even on `?` and on a crash,
    // which leaves nothing in the cwd or `--out`), then copy over file by
    // file with symlink-safe atomic writes. Staging anywhere under `out`'s
    // parent is what made `--out .` unusable (the stage file itself read
    // as foreign) and a writable `out` under an unwritable parent fail.
    //
    // `out` itself is never followed through a symlink, and a non-empty
    // foreign dir is refused: only an absent dir, an empty dir, or one
    // holding only our own files is written into. That keeps
    // `--out .` from wiping the cwd.
    let scratch = ScratchDir::new("build")?;
    let stage: &Path = scratch.path();
    // The staged files mirror the cache render exactly (first-party
    // fragments, the toolkit triple, foot's `foot.ini` when the look has a
    // `[terminal]` table, the launcher and locker leaves, the greeter,
    // launcher and notification fragments, plus every leaf
    // file, `fcitx5/theme.conf` under its directory): validate renders
    // what apply renders.
    let mut staged: Vec<(String, String)> = stage_of(
        &scoot,
        &bar,
        &foot,
        &fuzzel,
        &swaylock,
        &greet,
        &launch,
        &notify,
        &gtk3_css,
        &gtk4_css,
        &qt_scheme,
        &rendered_json,
        &leaf_files,
        &static_files,
    );
    for (file, contents) in &staged {
        let path = stage.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
        std::fs::write(&path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    // Every `return Err` below (and every `?` above) drops `scratch`,
    // removing the stage: no exit path leaks it. A bar built
    // `--no-default-features` refuses `popup-radius`, and a minimal bar
    // refuses whole module tables: re-render without what the installed
    // bar lacks rather than refusing every look, and say what was dropped.
    let mut drops: Vec<BarDrop> = Vec::new();
    loop {
        match checks.bar_fragment(&stage.join("bar.toml")) {
            Ok(Availability::Available(())) => {
                for drop_ in &drops {
                    validation.notes.push(drop_.rendered_note());
                }
                break;
            }
            Ok(Availability::Unavailable(note)) => {
                validation.notes.push(note);
                break;
            }
            Err(message) => {
                let fragment = match staged
                    .iter()
                    .find(|(file, _)| file == "bar.toml")
                    .map(|(_, contents)| contents.clone())
                {
                    Some(fragment) => fragment,
                    None => return Err(message),
                };
                let Some(drop_) = bar_downgrade(&message, &fragment) else {
                    return Err(message);
                };
                if !drop_.apply_to(&mut render_inputs) {
                    return Err(message);
                }
                drops.push(drop_);
                (
                    scoot,
                    bar,
                    foot,
                    fuzzel,
                    swaylock,
                    greet,
                    launch,
                    notify,
                    rendered_json,
                    leaf_files,
                ) = render_once(&render_inputs)?;
                staged = stage_of(
                    &scoot,
                    &bar,
                    &foot,
                    &fuzzel,
                    &swaylock,
                    &greet,
                    &launch,
                    &notify,
                    &gtk3_css,
                    &gtk4_css,
                    &qt_scheme,
                    &rendered_json,
                    &leaf_files,
                    &static_files,
                );
                for (file, contents) in &staged {
                    let path = stage.join(file);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                    }
                    std::fs::write(&path, contents)
                        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                }
            }
        }
    }
    // Foot's own parser over the staged fragment, when the look renders
    // one. A missing foot is a note; a refusal refuses the build before
    // anything lands in `--out`.
    if foot.is_some() {
        match checks.foot_fragment(&stage.join("foot.ini")) {
            Ok(Availability::Available(())) => {}
            Ok(Availability::Unavailable(note)) => validation.notes.push(note),
            Err(message) => return Err(message),
        }
    }
    // The output dir guard lives in `build_dir` (refuses symlinks and
    // foreign files, creates an absent dir).
    super::build_dir::check_out_dir(out)?;
    for (file, contents) in &staged {
        let path = out.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
        if let Err(error) = write_nofollow(&path, contents.as_bytes()) {
            return Err(format!("cannot write {}: {error}", path.display()));
        }
    }
    // Conditional outputs leave no stale copies behind: a rebuild of a look
    // without a `[terminal]` table removes a previous `foot.ini`, and a
    // rebuild for a look without `[night]` removes a previous `nightlight`,
    // so `--out` never carries stale files. The same for the static
    // leaves: a file under `apps/` the look no longer carries is removed.
    if foot.is_none()
        && let Err(error) = std::fs::remove_file(out.join("foot.ini"))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!(
            "cannot clear {}: {error}",
            out.join("foot.ini").display()
        ));
    }
    if !staged.iter().any(|(file, _)| file == "nightlight") {
        let stale = out.join("nightlight");
        if stale.is_file() && !stale.is_symlink() {
            std::fs::remove_file(&stale)
                .map_err(|error| format!("cannot clear {}: {error}", stale.display()))?;
        }
    }
    if let Ok(entries) = std::fs::read_dir(out.join("apps")) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("apps/{name}");
            if !staged.iter().any(|(file, _)| file == &rel) {
                let stale = entry.path();
                if stale.is_file() && !stale.is_symlink() {
                    std::fs::remove_file(&stale)
                        .map_err(|error| format!("cannot clear {}: {error}", stale.display()))?;
                }
            }
        }
    }
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
    foot: Option<PathBuf>,
}

impl ProdChecks {
    pub fn new() -> Self {
        Self {
            scootbg: find_binary("scootbg"),
            scootbar: find_binary("scootbar"),
            fc_match: find_binary("fc-match"),
            foot: find_binary("foot"),
        }
    }

    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            scootbg: None,
            scootbar: None,
            fc_match: None,
            foot: None,
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
            foot: None,
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
            // The whole stderr, not just its first line: a bar built
            // `--no-default-features` (no `popup`) names the offending key
            // (`popup-radius`) past the first line's `TOML parse error at …
            // ` summary, and the popup-less re-render keys off that name.
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let detail = truncate_head(&detail, 4096);
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

    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        let Some(foot) = &self.foot else {
            return Ok(Availability::Unavailable(
                "no `foot` found: foot fragment unchecked".to_owned(),
            ));
        };
        // `foot --check-config` verifies the configuration and exits
        // nonzero when it refuses it (its report goes to stderr: `err:
        // config.c:…`). The fragment alone is a valid config: foot fills
        // every missing key with its default.
        let out = run_with_timeout(
            foot,
            &["--check-config", "--config", &fragment.to_string_lossy()],
        )?;
        if out.status.success() {
            Ok(Availability::Available(()))
        } else {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let detail = truncate_head(&detail, 4096);
            Err(format!(
                "foot fragment refused by foot --check-config{}",
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
