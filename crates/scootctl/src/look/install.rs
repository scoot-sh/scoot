//! `scoot look install PATH [--trust]`, `export NAME [--out FILE]` and
//! `save NAME`: looks across the trust boundary (design section 8.3).
//!
//! An installed look is someone else's: it lands in `installed-looks/`
//! (never `user-looks/`, so its origin is on disk), may not take the name
//! of a shipped, declared or user-authored look, and can be applied but
//! never extended. Everything arriving is validated before it lands:
//! regular files only (a symlink, a hard link, a device, a FIFO or a
//! socket refuses the install), only the file kinds a look uses, by name and place
//! (`look.toml`, `preview.png`, image wallpapers, `fonts/*.ttf|otf`, the
//! six named `apps/` leaves, `LICENSE*`, `COPYING*`, `README.md`), every
//! referenced path inside the look directory, file count and total size
//! capped, `license` required, and the `apps/` static leaves dropped
//! unless `--trust` (printed first).
//!
//! A wallpaper given by URL is fetched, verified against its `sha256` and
//! cached at install time (the install refuses if that fails), so a swap
//! to it never makes a network request. The install-time fetch never
//! reuses scootbg's redirect-following fetch: it follows no redirects,
//! resolves the host first, refuses a loopback, link-local, private,
//! unique-local or unspecified address, and pins the connection to the
//! checked address (`curl --resolve host:443:ADDR`), so a second lookup
//! cannot swap it (DNS rebinding).
//!
//! `export` writes a tarball (plain ustar `.tar`) of a source look,
//! regular files only; `save` snapshots the user's own appearance and
//! color values as a source look in `user-looks/` (user-authored, so
//! extendable). Export and save round-trip through install.
//!
//! Works with no Nix: plain files in, plain files out, `curl` beside the
//! binary or on `PATH` for the URL fetch.

use std::collections::BTreeSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use scoot_look::catalog::{Catalog, cache_home, config_home, data_home};
use scoot_look::grammar::{check_relative_path, is_font_file_name, is_image_file_name};
use scoot_look::{Origin, parse_look_toml, validate_apps_allowlist, validate_for_origin};

use super::apply::{self, Checks, Validation};

/// The most files one install or tarball may carry. A look holds about ten;
/// past this is a mistake or hostile.
pub const MAX_INSTALL_FILES: usize = 64;

/// The most bytes one install or tarball may carry in total.
pub const MAX_INSTALL_BYTES: u64 = 96 * 1024 * 1024;

/// The largest bundled font accepted: Nerd Fonts run past the 64 KiB every
/// other look file is capped at, but never to megabytes times ten.
pub const MAX_FONT_BYTES: u64 = 16 * 1024 * 1024;

/// The `apps/` leaves the install gate knows, by name and place (design
/// section 8.3): foot's config and the greeter's beside the three terminal
/// leaves and the Neovim colorscheme [`scoot_look::STATIC_LEAVES`] owns.
/// Static files, copied verbatim, never rendered — which is why an
/// installed look drops them unless `--trust` is given (starship custom
/// commands and Lua both run code: the former a shell, the latter in the
/// editor itself).
pub const APPS_LEAVES: &[&str] = &[
    "apps/foot.ini",
    "apps/helix.toml",
    "apps/btop.theme",
    "apps/starship.toml",
    "apps/regreet.css",
    "apps/nvim.lua",
];

/// Whether `rel` (a `/`-separated path inside a look) is a file kind a look
/// uses, in the place it uses it: `look.toml`, `preview.png`, `README.md`,
/// `LICENSE*`/`COPYING*` and image wallpapers at the top level,
/// `fonts/*.ttf|otf` under `fonts/`, the six named leaves under `apps/`.
/// Anything else refuses the install by name. The caller validates
/// normalization first ([`check_relative_path`]).
pub fn permitted_rel(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        [top] => {
            if *top == "look.toml" || *top == "preview.png" || *top == "README.md" {
                return true;
            }
            if top == &"LICENSE"
                || top.starts_with("LICENSE.")
                || top.starts_with("LICENSE-")
                || top == &"COPYING"
                || top.starts_with("COPYING.")
            {
                return true;
            }
            is_image_file_name(rel)
        }
        ["fonts", file] => is_font_file_name(file),
        ["apps", _] => APPS_LEAVES.contains(&rel),
        _ => false,
    }
}

/// The byte cap for one file, by kind: images take scootbg's own 32 MiB
/// ([`apply::MAX_IMAGE_BYTES`]), fonts take [`MAX_FONT_BYTES`], everything
/// else the 64 KiB every other look file is capped at.
fn kind_cap(rel: &str) -> u64 {
    if rel.starts_with("fonts/") {
        MAX_FONT_BYTES
    } else if is_image_file_name(rel) {
        apply::MAX_IMAGE_BYTES
    } else {
        scoot_look::state::MAX_FILE
    }
}

/// The installed-looks directory: `$XDG_DATA_HOME/scoot/installed-looks`,
/// else `~/.local/share/scoot/installed-looks`. `None` only when no home
/// resolves.
pub fn installed_looks_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    data_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot/installed-looks"))
        .ok_or_else(|| "no home resolves: set XDG_DATA_HOME or HOME".to_owned())
}

/// The config home: `$XDG_CONFIG_HOME`, else `~/.config`. `None` only when
/// no home resolves.
pub fn config_home_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    config_home(xdg.as_deref(), home.as_deref())
        .ok_or_else(|| "no home resolves: set XDG_CONFIG_HOME or HOME".to_owned())
}

/// The download cache the install-time URL fetch fills:
/// `$XDG_CACHE_HOME/scootbg`, else `~/.cache/scootbg` — the same directory
/// scootbg's own fetch reads, so a swap to the installed look finds the
/// bytes without a network request.
pub fn scootbg_cache_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    cache_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scootbg"))
        .ok_or_else(|| "no home resolves: set XDG_CACHE_HOME or HOME".to_owned())
}

/// What an install landed.
#[derive(Debug)]
pub struct InstallReport {
    /// The look's name (its manifest's, which names its directory).
    pub name: String,
    /// The installed directory.
    pub dir: PathBuf,
    /// The `apps/` leaves dropped for want of `--trust`.
    pub dropped: Vec<String>,
    /// The `apps/` leaves kept under `--trust` (printed first).
    pub kept: Vec<String>,
    /// The cached download when the wallpaper came by URL.
    pub fetched: Option<PathBuf>,
    /// Effectful notes (backend skips from validation).
    pub notes: Vec<String>,
}

/// Installs the directory `src` into `dest_parent` (the installed-looks
/// directory): the trust-boundary gate, the URL wallpaper fetch, then the
/// same validation an apply runs. Nothing lands on failure.
pub fn install_dir_into(
    src: &Path,
    dest_parent: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
    fetch: &dyn UrlFetch,
) -> Result<InstallReport, String> {
    let meta = std::fs::symlink_metadata(src)
        .map_err(|error| format!("cannot stat {}: {error}", src.display()))?;
    if meta.is_symlink() {
        return Err(format!(
            "refused: {} is a symlink: install takes a directory or a tarball, never a link",
            src.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!(
            "refused: {} is not a directory (a file installs only as a tarball)",
            src.display()
        ));
    }
    let files = collect_dir(src)?;
    finish_install(files, dest_parent, catalog, checks, trust, fetch)
}

/// Installs the tarball `file` into `dest_parent`: the same gate as
/// [`install_dir_into`], reading the archive entry by entry (never
/// unpacking it: every path is re-validated, only regular files land, and
/// sizes are capped before they are read).
pub fn install_tar_into(
    file: &Path,
    dest_parent: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
    fetch: &dyn UrlFetch,
) -> Result<InstallReport, String> {
    let files = collect_tar(file)?;
    finish_install(files, dest_parent, catalog, checks, trust, fetch)
}

/// One validated file inside a look, headed for the installed copy.
struct LookFile {
    /// `/`-separated, normalized, permitted.
    rel: String,
    bytes: Vec<u8>,
}

/// Reads `src` through the trust gate: regular files only, permitted names
/// and places only, count/size/kind caps. Refuses loudly on the first
/// violation, naming it.
fn collect_dir(src: &Path) -> Result<Vec<LookFile>, String> {
    let mut files = Vec::new();
    let mut total: u64 = 0;
    collect_dir_into(src, src, &mut files, &mut total)?;
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

fn collect_dir_into(
    root: &Path,
    dir: &Path,
    files: &mut Vec<LookFile>,
    total: &mut u64,
) -> Result<(), String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot list {}: {error}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|_| format!("{} escapes the look directory", path.display()))?
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
            .replace('\\', "/");
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        if meta.is_symlink() {
            return Err(format!(
                "refused: {rel} is a symlink: installed looks carry regular files only"
            ));
        }
        if meta.is_dir() {
            // Only `fonts/` and `apps/` hold files; anything deeper, or any
            // other directory, refuses by name.
            if dir != root || (rel != "fonts" && rel != "apps") {
                return Err(format!(
                    "refused: {rel}/ is not a file a look uses (only look.toml, preview.png, image wallpapers, fonts/*.ttf|otf, the six apps/ leaves, LICENSE*, COPYING*, README.md install)"
                ));
            }
            collect_dir_into(root, &path, files, total)?;
            continue;
        }
        if !meta.is_file() {
            return Err(format!(
                "refused: {rel} is not a regular file: installed looks carry regular files only"
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if meta.nlink() != 1 {
                return Err(format!(
                    "refused: {rel} is a hard link: installed looks carry regular files only"
                ));
            }
        }
        if let Some(error) = check_relative_path("file", &rel) {
            return Err(format!("refused: {rel}: {error}"));
        }
        if !permitted_rel(&rel) {
            return Err(format!(
                "refused: {rel} is not a file a look uses (only look.toml, preview.png, image wallpapers, fonts/*.ttf|otf, the six apps/ leaves, LICENSE*, COPYING*, README.md install)"
            ));
        }
        gate_file(&rel, meta.len(), files.len(), *total)?;
        // The pre-check above is `symlink_metadata` (never follows) and
        // this read opens `O_NOFOLLOW` and re-checks the handle (regular
        // file, single link), reading at most the kind cap, so a link or
        // a FIFO swapped in between refuses instead of landing outside
        // bytes.
        let bytes = apply::read_regular_capped(&path, kind_cap(&rel))?;
        *total += bytes.len() as u64;
        if *total > MAX_INSTALL_BYTES {
            return Err(format!(
                "refused: the look is larger than {MAX_INSTALL_BYTES} bytes in total"
            ));
        }
        files.push(LookFile { rel, bytes });
    }
    Ok(())
}

/// The count, declared-size and total caps, shared by the directory and
/// tarball gates.
fn gate_file(rel: &str, size: u64, have: usize, total: u64) -> Result<(), String> {
    if have >= MAX_INSTALL_FILES {
        return Err(format!(
            "refused: more than {MAX_INSTALL_FILES} files: {rel} does not fit"
        ));
    }
    let cap = kind_cap(rel);
    if size > cap {
        return Err(format!(
            "refused: {rel} is {size} bytes (over the {cap} byte cap)"
        ));
    }
    if total > MAX_INSTALL_BYTES {
        return Err(format!(
            "refused: the look is larger than {MAX_INSTALL_BYTES} bytes in total"
        ));
    }
    Ok(())
}

/// Whether `file` starts as a plain (uncompressed) tar archive: the ustar
/// magic at offset 257. Anything else — including gzip — refuses with its
/// own message at the call site.
fn is_tarball(head: &[u8]) -> bool {
    head.len() >= 262 && head[257..262] == *b"ustar"
}

/// Reads the tarball `file` through the trust gate: plain ustar only,
/// regular files only (directory entries for `fonts/` and `apps/` are
/// created, never followed), every path re-validated, sizes capped before
/// they are read, duplicates refused.
fn collect_tar(path: &Path) -> Result<Vec<LookFile>, String> {
    use std::io::Read as _;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .read(&mut head)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if n >= 2 && head[0] == 0x1f && head[1] == 0x8b {
        return Err(format!(
            "refused: {} is gzip-compressed: `export` writes plain .tar, install takes plain .tar",
            path.display()
        ));
    }
    if !is_tarball(&head[..n]) {
        return Err(format!(
            "refused: {} is not a tarball: install takes a look directory or a plain .tar (`export` writes one)",
            path.display()
        ));
    }
    let fh = std::fs::File::open(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut archive = tar::Archive::new(fh);
    // The entries' paths first: when every entry shares one directory
    // prefix (`export` writes `<name>/...`), it strips. Collected up front
    // so the strip decision sees the whole archive before anything
    // validates (unread entries seek past their data on drop).
    let mut paths: Vec<String> = Vec::new();
    for entry in archive
        .entries()
        .map_err(|error| format!("cannot read the tarball: {error}"))?
    {
        let entry = entry.map_err(|error| format!("cannot read the tarball: {error}"))?;
        let path = entry
            .header()
            .path()
            .map_err(|_| "refused: a tarball entry is not valid UTF-8".to_owned())?
            .to_string_lossy()
            .replace('\\', "/");
        paths.push(path);
    }
    let strip = shared_prefix(&paths);
    // The second pass reads the bytes: the archive is small enough to
    // install that reading it twice is bounded by the same caps.
    let fh = std::fs::File::open(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut archive = tar::Archive::new(fh);
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total: u64 = 0;
    for entry in archive
        .entries()
        .map_err(|error| format!("cannot read the tarball: {error}"))?
    {
        let entry = entry.map_err(|error| format!("cannot read the tarball: {error}"))?;
        let kind = entry.header().entry_type();
        let mut rel = entry
            .header()
            .path()
            .map_err(|_| "refused: a tarball entry is not valid UTF-8".to_owned())?
            .to_string_lossy()
            .replace('\\', "/");
        // Tar writers prefix entries with `./`: strip one harmless leading
        // marker before the strict checks (stripping a prefix can never
        // smuggle in `..`).
        while rel.starts_with("./") {
            rel = rel["./".len()..].to_owned();
        }
        if let Some(prefix) = &strip {
            rel = rel
                .strip_prefix(prefix)
                .unwrap_or(&rel)
                .trim_start_matches('/')
                .to_owned();
            if rel.is_empty() {
                continue;
            }
        }
        if kind.is_dir() {
            let dir = rel.trim_end_matches('/');
            if dir.is_empty() || dir == "." || dir == "fonts" || dir == "apps" {
                continue;
            }
            return Err(format!(
                "refused: {dir}/ is not a directory a look uses (only fonts/ and apps/ install)"
            ));
        }
        if !kind.is_file() {
            let what = match kind {
                k if k.is_symlink() => "a symlink",
                k if k.is_hard_link() => "a hard link",
                _ => "not a regular file",
            };
            return Err(format!(
                "refused: {rel} is {what}: installed looks carry regular files only"
            ));
        }
        if let Some(error) = check_relative_path("file", &rel) {
            return Err(format!("refused: {rel}: {error}"));
        }
        if !permitted_rel(&rel) {
            return Err(format!(
                "refused: {rel} is not a file a look uses (only look.toml, preview.png, image wallpapers, fonts/*.ttf|otf, the six apps/ leaves, LICENSE*, COPYING*, README.md install)"
            ));
        }
        if !seen.insert(rel.clone()) {
            return Err(format!("refused: {rel} appears twice in the tarball"));
        }
        let size = entry
            .header()
            .size()
            .map_err(|error| format!("cannot read the tarball: {error}"))?;
        gate_file(&rel, size, files.len(), total)?;
        let cap = kind_cap(&rel);
        let mut bytes = Vec::new();
        entry
            .take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {rel} from the tarball: {error}"))?;
        if bytes.len() as u64 > cap {
            return Err(format!(
                "refused: {rel} is over the {cap} byte cap (its header lied about {size} bytes)"
            ));
        }
        total += bytes.len() as u64;
        if total > MAX_INSTALL_BYTES {
            return Err(format!(
                "refused: the look is larger than {MAX_INSTALL_BYTES} bytes in total"
            ));
        }
        files.push(LookFile { rel, bytes });
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

/// The single top-level directory prefix shared by every tarball entry, if
/// any (`export` writes `<name>/...`): stripped on install, so extracting
/// never scatters files and never needs the prefix to match the manifest.
fn shared_prefix(paths: &[String]) -> Option<String> {
    let mut firsts: BTreeSet<&str> = BTreeSet::new();
    for path in paths {
        let mut path = path.as_str();
        while path.starts_with("./") {
            path = &path["./".len()..];
        }
        let (first, _) = path.split_once('/')?;
        if first.is_empty() {
            return None;
        }
        firsts.insert(first);
    }
    let mut only = firsts.into_iter();
    match (only.next(), only.next()) {
        (Some(prefix), None) => Some(format!("{prefix}/")),
        _ => None,
    }
}

/// Stages `files` under `dest_parent`, fetches a URL wallpaper into the
/// download cache, and validates the staged look exactly as an apply would.
/// The staged directory is named for the manifest and renamed into place
/// only once everything passes; any failure removes it, so a refused
/// install leaves nothing behind.
fn finish_install(
    mut files: Vec<LookFile>,
    dest_parent: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
    fetch: &dyn UrlFetch,
) -> Result<InstallReport, String> {
    let manifest = files
        .iter()
        .find(|file| file.rel == "look.toml")
        .ok_or_else(|| "refused: no look.toml: not a look".to_owned())?;
    let raw = parse_look_toml(&manifest.bytes).map_err(|errors| {
        format!(
            "refused: look.toml: {}",
            errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let name = raw.name.clone();
    let dest = dest_parent.join(&name);
    if let Some(entry) = catalog.find(&name) {
        if entry.origin == Origin::Installed {
            return Err(format!(
                "refused: `{name}` is already installed: remove {} first",
                dest.display()
            ));
        }
        return Err(format!(
            "refused: `{name}` is already a {} look: an installed look may not take the name of a shipped, declared or user-authored look",
            scoot_look::catalog::origin_name(entry.origin)
        ));
    }
    if dest.exists() {
        return Err(format!(
            "refused: {} already exists: remove it first",
            dest.display()
        ));
    }
    let mut validation = Validation::default();
    for error in validate_for_origin(&raw, Origin::Installed)
        .into_iter()
        .chain(validate_apps_allowlist(&raw.apps))
    {
        validation.errors.push(error.to_string());
    }
    if !validation.errors.is_empty() {
        return Err(format!("refused: {}", validation.errors.join("; ")));
    }
    // The `apps/` leaves: dropped without `--trust`, kept (and printed
    // first by the caller) with it.
    let mut dropped = Vec::new();
    let mut kept = Vec::new();
    if !trust {
        files.retain(|file| {
            if APPS_LEAVES.contains(&file.rel.as_str()) {
                dropped.push(file.rel.clone());
                false
            } else {
                true
            }
        });
        dropped.sort();
    } else {
        kept = files
            .iter()
            .filter(|file| APPS_LEAVES.contains(&file.rel.as_str()))
            .map(|file| file.rel.clone())
            .collect();
        kept.sort();
    }
    // Every referenced file must have ridden along: a manifest naming a
    // file the gate refused (or the archive omitted) refuses here, naming
    // it, rather than failing later inside validation.
    let mut referenced = Vec::new();
    if let Some(wallpaper) = &raw.wallpaper
        && let Some(image) = &wallpaper.image
        && !image.starts_with("https://")
    {
        referenced.push(image.clone());
    }
    if let Some(preview) = &raw.preview {
        referenced.push(preview.clone());
    }
    referenced.extend(raw.fonts.files.values().cloned());
    for need in &referenced {
        if !files.iter().any(|file| &file.rel == need) {
            return Err(format!(
                "refused: look.toml references `{need}`, which is not among the installed files"
            ));
        }
    }
    // Stage under a temporary parent (same filesystem): the inner directory
    // already carries the manifest's name, so the name-matches-directory
    // validation runs on the staged copy as it would on the final one.
    let staging_parent = staged_parent(dest_parent, &name)?;
    let staging = staging_parent.join(&name);
    let cleanup = Cleanup(&staging_parent);
    std::fs::create_dir_all(staging.join("fonts"))
        .map_err(|error| format!("cannot stage the install: {error}"))?;
    std::fs::create_dir_all(staging.join("apps"))
        .map_err(|error| format!("cannot stage the install: {error}"))?;
    for file in &files {
        let path = staging.join(&file.rel);
        std::fs::write(&path, &file.bytes)
            .map_err(|error| format!("cannot stage {}: {error}", file.rel))?;
    }
    // A URL wallpaper is fetched, verified and cached now: a swap to the
    // installed look must never make a network request.
    let mut fetched = None;
    if let Some(wallpaper) = &raw.wallpaper
        && let Some(image) = &wallpaper.image
        && image.starts_with("https://")
    {
        let pin = wallpaper.sha256.clone().unwrap_or_default();
        let cache = scootbg_cache_dir()?;
        match fetch.fetch_url(image, &pin, &cache) {
            Ok(path) => fetched = Some(path),
            Err(error) => {
                return Err(format!("refused: wallpaper {image}: {error}"));
            }
        }
    }
    let staged = apply::validate_dir(&staging, Origin::Installed, catalog, checks, trust)?;
    if !staged.errors.is_empty() {
        return Err(format!("refused: {}", staged.errors.join("; ")));
    }
    std::fs::create_dir_all(dest_parent)
        .map_err(|error| format!("cannot create {}: {error}", dest_parent.display()))?;
    std::fs::rename(&staging, &dest)
        .map_err(|error| format!("cannot install {}: {error}", dest.display()))?;
    std::mem::forget(cleanup);
    let _ = std::fs::remove_dir(&staging_parent);
    let mut notes = staged.notes;
    notes.extend(staged.warnings.into_iter().map(|w| format!("warning: {w}")));
    Ok(InstallReport {
        name,
        dir: dest,
        dropped,
        kept,
        fetched,
        notes,
    })
}

/// Removes the staging parent unless the install succeeded (see
/// [`finish_install`]).
struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0);
    }
}

/// A fresh staging parent beside the destination: `.install-<name>-<pid>`,
/// so a planted entry at a guessable name is never written through.
fn staged_parent(dest_parent: &Path, name: &str) -> Result<PathBuf, String> {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let parent = dest_parent.join(format!(".install-{safe}-{}", std::process::id()));
    if parent.exists() {
        return Err(format!(
            "cannot stage the install: {} already exists",
            parent.display()
        ));
    }
    std::fs::create_dir_all(&parent)
        .map_err(|error| format!("cannot stage the install: {error}"))?;
    Ok(parent)
}

/// What an export wrote.
#[derive(Debug)]
pub struct ExportReport {
    /// The look's catalog name.
    pub name: String,
    /// The tarball written.
    pub tarball: PathBuf,
    /// The archive members, `<name>/`-prefixed, in order.
    pub files: Vec<String>,
    /// Files skipped on the way (unreadable, or not a kind a look uses).
    pub notes: Vec<String>,
}

/// Exports the catalog look `name` as a tarball (plain ustar `.tar`) at
/// `out`: every regular file the source holds that a look uses, under one
/// `<name>/` prefix, deterministic (sorted, zeroed mtime/ownership). The
/// tarball installs back through [`install_tar_into`], unchanged.
pub fn export_into(name: &str, catalog: &Catalog, out: &Path) -> Result<ExportReport, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static EXPORT_COUNTER: AtomicU64 = AtomicU64::new(0);
    let entry = catalog
        .find(name)
        .ok_or_else(|| apply::unknown_look(name, catalog))?;
    // Look-relative files resolve against the manifest's own directory: a
    // home-manager per-file farm links each file into the store, so the
    // bytes live beside the manifest target, not the entry.
    let root = entry.manifest_dir.clone();
    let mut files: Vec<LookFile> = Vec::new();
    let mut notes = Vec::new();
    let mut total: u64 = 0;
    export_dir_into(&root, &root, &mut files, &mut total, &mut notes)?;
    if !files.iter().any(|file| file.rel == "look.toml") {
        return Err(format!(
            "cannot export `{name}`: no look.toml beside its manifest"
        ));
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    let tmp = out.with_extension(format!(
        "part-{}-{}",
        std::process::id(),
        EXPORT_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let result: Result<(), String> = (|| {
        let file = std::fs::File::create(&tmp)
            .map_err(|error| format!("cannot write {}: {error}", tmp.display()))?;
        let mut builder = tar::Builder::new(file);
        for file in &files {
            let member = format!("{name}/{}", file.rel);
            let mut header = tar::Header::new_ustar();
            header.set_size(file.bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            builder
                .append_data(&mut header, member.as_str(), file.bytes.as_slice())
                .map_err(|error| format!("cannot write the tarball: {error}"))?;
        }
        builder
            .finish()
            .map_err(|error| format!("cannot write the tarball: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        std::fs::remove_file(&tmp).ok();
        return Err(error);
    }
    std::fs::rename(&tmp, out)
        .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    Ok(ExportReport {
        name: name.to_owned(),
        tarball: out.to_owned(),
        files: files
            .iter()
            .map(|file| format!("{name}/{}", file.rel))
            .collect(),
        notes,
    })
}

/// Collects one source directory for export: regular files with permitted
/// names and places land; anything else is a note, never a refusal — the
/// source is already the user's or the catalog's, and install re-gates
/// whatever the tarball carries. Symlinks (even to regular files) skip
/// with a note like every other skip, so a stray link never bakes outside
/// bytes into the handed-over tarball.
fn export_dir_into(
    root: &Path,
    dir: &Path,
    files: &mut Vec<LookFile>,
    total: &mut u64,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot list {}: {error}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|_| format!("{} escapes the look directory", path.display()))?
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
            .replace('\\', "/");
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        if meta.is_symlink() {
            notes.push(format!("skipping {rel}: is a symlink"));
            continue;
        }
        if meta.is_dir() {
            if dir != root || (rel != "fonts" && rel != "apps") {
                notes.push(format!("skipping {rel}/: not a directory a look uses"));
                continue;
            }
            export_dir_into(root, &path, files, total, notes)?;
            continue;
        }
        if check_relative_path("file", &rel).is_some() || !permitted_rel(&rel) {
            notes.push(format!("skipping {rel}: not a file a look uses"));
            continue;
        }
        if !meta.is_file() {
            notes.push(format!("skipping {rel}: not a regular file"));
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if meta.nlink() != 1 {
                notes.push(format!("skipping {rel}: is a hard link"));
                continue;
            }
        }
        // Never follows: symlinks already skipped above, and the capped
        // read re-checks the handle, so a link raced in refuses as a
        // skip instead of landing outside bytes.
        let cap = kind_cap(&rel);
        let bytes = match apply::read_regular_capped(&path, cap) {
            Ok(bytes) => bytes,
            Err(error) => {
                notes.push(format!("skipping {rel}: {error}"));
                continue;
            }
        };
        if bytes.len() as u64 > cap {
            notes.push(format!(
                "skipping {rel}: {} bytes (over the {cap} byte cap)",
                bytes.len()
            ));
            continue;
        }
        if files.len() >= MAX_INSTALL_FILES {
            notes.push(format!(
                "skipping {rel}: more than {MAX_INSTALL_FILES} files"
            ));
            continue;
        }
        *total += bytes.len() as u64;
        if *total > MAX_INSTALL_BYTES {
            notes.push(format!(
                "skipping {rel}: the export is larger than {MAX_INSTALL_BYTES} bytes in total"
            ));
            *total -= bytes.len() as u64;
            continue;
        }
        files.push(LookFile { rel, bytes });
    }
    Ok(())
}

/// What a save wrote.
#[derive(Debug)]
pub struct SaveReport {
    /// The look's name (its new directory's).
    pub name: String,
    /// The saved directory.
    pub dir: PathBuf,
    /// The active look the saved one extends, if any.
    pub extends: Option<String>,
    /// The token groups captured, for the reply.
    pub tokens: Vec<String>,
    /// Notes (values both files set, files carried over).
    pub notes: Vec<String>,
}

/// Saves the user's own appearance and color values as a source look in
/// `parent` (the user-looks directory): the compositor's `[appearance]`
/// and `[wallpaper]` from `config/scoot/config.toml` plus the bar's
/// `[colors]` from `config/scoot/bar.toml`, written as tokens extending
/// the live look (`state_dir`'s record, which must exist and still be in
/// the catalog: the base supplies the fonts, opacity and every other token
/// the user's files cannot give, so applying the save reproduces the
/// desktop exactly). Fonts, opacity, the terminal hues, night, icons and
/// polarity are not the user's files' to give in v1. Non-token
/// customizations (per-app overrides, binds, module lists) stay in the
/// user's files, where they keep shadowing — the save snapshots
/// presentation only, never behavior.
#[allow(clippy::too_many_arguments)]
pub fn save_into(
    name: &str,
    config: &Path,
    parent: &Path,
    state_dir: Option<&Path>,
    catalog: &Catalog,
    checks: &dyn Checks,
) -> Result<SaveReport, String> {
    if !scoot_look::grammar::is_name(name) {
        return Err(format!("`{name}` is not a look name ([a-z0-9-]{{1,32}})"));
    }
    if catalog.find(name).is_some() {
        return Err(format!(
            "`{name}` is already a look: pick another name (nothing was written)"
        ));
    }
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!(
            "`{}` already exists: pick another name (nothing was written)",
            dir.display()
        ));
    }
    let scoot_path = config.join("scoot/config.toml");
    let bar_path = config.join("scoot/bar.toml");
    // The user's own files: links (home-manager, stow) are followed, as in
    // the shadowed-keys read.
    let scoot = read_user_table(&scoot_path)?;
    let bar = read_user_table(&bar_path)?;
    if scoot.is_none() && bar.is_none() {
        return Err(format!(
            "nothing to save: neither {} nor {} exists",
            scoot_path.display(),
            bar_path.display()
        ));
    }
    let mut palette: Vec<(String, String)> = Vec::new();
    let mut shape: Vec<(String, String)> = Vec::new();
    let mut cursor: Vec<(String, String)> = Vec::new();
    let mut wallpaper: Vec<(String, String)> = Vec::new();
    let mut tokens = Vec::new();
    let mut notes = Vec::new();
    // The bar's colors first; the compositor's appearance wins on conflict
    // (said aloud: both files set the same desktop color).
    if let Some(bar) = &bar
        && let Some(colors) = bar.get("colors").and_then(|v| v.as_table())
    {
        for role in [
            "background",
            "foreground",
            "dim",
            "accent",
            "urgent",
            "hover",
            "focus",
            "border",
            "accent-fg",
            "selection",
            "selection-text",
            "match",
        ] {
            if let Some(color) = colors.get(role).and_then(|v| v.as_str())
                && is_hex_color(color)
            {
                palette.push((role.to_owned(), color.to_owned()));
            }
        }
    }
    if let Some(scoot) = &scoot
        && let Some(appearance) = scoot.get("appearance").and_then(|v| v.as_table())
    {
        // (user key, token role): the render mapping, inverted.
        for (key, role) in [
            ("focus_ring_active_color", "focus"),
            ("focus_ring_inactive_color", "border"),
            ("background_color", "background"),
        ] {
            if let Some(color) = appearance.get(key).and_then(|v| v.as_str()) {
                if !is_hex_color(color) {
                    notes.push(format!(
                        "skipping appearance.{key} `{color}`: not a #rrggbb color"
                    ));
                    continue;
                }
                if let Some(same) = palette.iter_mut().find(|(r, _)| r == role) {
                    if same.1 != color {
                        notes.push(format!(
                            "appearance.{key} `{color}` wins over the bar's {role} `{}`",
                            same.1
                        ));
                    }
                    same.1 = color.to_owned();
                } else {
                    palette.push((role.to_owned(), color.to_owned()));
                }
            }
        }
        for (key, token) in [
            ("corner_radius", "radius"),
            ("focus_ring_width", "ring-width"),
            ("focus_ring_inactive_width", "ring-inactive-width"),
        ] {
            if let Some(value) = appearance.get(key).and_then(|v| v.as_integer()) {
                shape.push((token.to_owned(), value.to_string()));
            }
        }
        if let Some(theme) = appearance.get("cursor_theme").and_then(|v| v.as_str()) {
            cursor.push(("theme".to_owned(), theme.to_owned()));
        }
        if let Some(size) = appearance.get("cursor_size").and_then(|v| v.as_integer()) {
            cursor.push(("size".to_owned(), size.to_string()));
        }
        if let Some(accent) = appearance.get("cursor_color").and_then(|v| v.as_str()) {
            if accent == "none" || is_hex_color(accent) {
                cursor.push(("accent".to_owned(), accent.to_owned()));
            } else {
                notes.push(format!(
                    "skipping appearance.cursor_color `{accent}`: not a #rrggbb color or `none`"
                ));
            }
        }
        if let Some(table) = scoot.get("wallpaper").and_then(|v| v.as_table()) {
            save_wallpaper(table, config, &dir, &mut wallpaper, &mut notes)?;
        }
    }
    if !palette.is_empty() {
        tokens.push("palette".to_owned());
    }
    if !shape.is_empty() {
        tokens.push("shape".to_owned());
    }
    if !cursor.is_empty() {
        tokens.push("cursor".to_owned());
    }
    if !wallpaper.is_empty() {
        tokens.push("wallpaper".to_owned());
    }
    if tokens.is_empty() {
        return Err(format!(
            "nothing to save: no appearance or color values in {} or {}",
            scoot_path.display(),
            bar_path.display()
        ));
    }
    // The save extends the live look: the base supplies the fonts,
    // opacity and every other token the user's files cannot give, so the
    // save reproduces the desktop. With no live choice there is no base
    // to complete the look, and the save refuses rather than writing a
    // look that cannot validate.
    let extends = match state_dir {
        Some(state) => match scoot_look::state::read_record(state) {
            scoot_look::state::RecordRead::Present(record) => {
                if catalog.find(&record.name).is_some() {
                    record.name
                } else {
                    return Err(format!(
                        "cannot save `{name}`: the live look is gone from the catalog"
                    ));
                }
            }
            scoot_look::state::RecordRead::Missing => {
                return Err(format!(
                    "cannot save `{name}` with no live look: `scoot look apply` one first, then the save extends it"
                ));
            }
            scoot_look::state::RecordRead::Corrupt(reason) => {
                return Err(format!("the recorded choice is corrupt: {reason}"));
            }
        },
        None => {
            return Err(format!(
                "cannot save `{name}` with no live look: `scoot look apply` one first, then the save extends it"
            ));
        }
    };
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let manifest = save_manifest(name, Some(&extends), &palette, &shape, &cursor, &wallpaper)?;
    // The dir is ours alone (just created): a failure removes it, like `new`.
    if let Err(error) = std::fs::write(dir.join("look.toml"), manifest) {
        std::fs::remove_dir_all(&dir).ok();
        return Err(format!(
            "cannot write {}: {error}",
            dir.join("look.toml").display()
        ));
    }
    let validation = apply::validate_dir(&dir, Origin::User, catalog, checks, false)?;
    if !validation.errors.is_empty() {
        return Err(format!(
            "invalid: {name} (the directory stays: fix it and re-run): {}",
            validation.errors.join("; ")
        ));
    }
    Ok(SaveReport {
        name: name.to_owned(),
        dir,
        extends: Some(extends),
        tokens,
        notes,
    })
}

/// Reads one of the user's own config files: missing is absent (the other
/// file may still save), unparsable refuses loudly.
fn read_user_table(path: &Path) -> Result<Option<toml::Table>, String> {
    match scoot_look::state::read_capped_follow(path) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map(Some)
            .map_err(|error| format!("cannot parse {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Whether `value` is a `#rrggbb` color, the only color shape a look takes.
fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Captures the user's `[wallpaper]` unit: a color carries over; an image
/// file is copied into the saved look under its own name; a URL carries
/// with its pin (a pinless URL refuses: a look URL without `sha256` never
/// validates). `command` and friends are not presentation and never save.
fn save_wallpaper(
    table: &toml::Table,
    config: &Path,
    dir: &Path,
    wallpaper: &mut Vec<(String, String)>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(color) = table.get("color").and_then(|v| v.as_str()) {
        if !is_hex_color(color) {
            notes.push(format!(
                "skipping wallpaper.color `{color}`: not a #rrggbb color"
            ));
        } else {
            wallpaper.push(("color".to_owned(), color.to_owned()));
        }
    }
    if let Some(image) = table.get("image").and_then(|v| v.as_str()) {
        if image.starts_with("https://") {
            match table.get("sha256").and_then(|v| v.as_str()) {
                Some(pin) if pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    wallpaper.push(("image".to_owned(), image.to_owned()));
                    wallpaper.push(("sha256".to_owned(), pin.to_owned()));
                    notes.push(format!(
                        "wallpaper `{image}` is a URL: the saved look's first swap needs its download cached, so reinstall the look if that swap refuses a cache miss"
                    ));
                }
                _ => {
                    return Err("cannot save a URL wallpaper without its sha256 pin".to_owned());
                }
            }
        } else {
            // The user's own path: followed (their file, their links),
            // then gated like any wallpaper — an image kind, within the
            // byte cap — and copied in under its own name. Capped before
            // reading (never `std::fs::read`), so a huge file or FIFO
            // refuses instead of wedging the save.
            let source = if Path::new(image).is_absolute() {
                PathBuf::from(image)
            } else {
                config.join("scoot").join(image)
            };
            let bytes = read_user_bytes_capped(&source, apply::MAX_IMAGE_BYTES)
                .map_err(|error| format!("cannot save wallpaper `{image}`: {error}"))?;
            if bytes.len() as u64 > apply::MAX_IMAGE_BYTES {
                return Err(format!(
                    "cannot save wallpaper `{image}`: {} bytes (over the {} byte cap)",
                    bytes.len(),
                    apply::MAX_IMAGE_BYTES
                ));
            }
            let base = Path::new(image)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("cannot save wallpaper `{image}`: bad file name"))?;
            if !is_image_file_name(base) {
                return Err(format!(
                    "cannot save wallpaper `{image}`: not a .png, .jpg, .jpeg, .webp or .gif file"
                ));
            }
            if !image_sniff(&bytes) {
                return Err(format!(
                    "cannot save wallpaper `{image}`: not a PNG, JPEG, GIF or WebP image"
                ));
            }
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
            std::fs::write(dir.join(base), &bytes)
                .map_err(|error| format!("cannot save wallpaper `{image}`: {error}"))?;
            wallpaper.push(("image".to_owned(), base.to_owned()));
            notes.push(format!("wallpaper `{image}` copied in as `{base}`"));
        }
    }
    for key in ["mode", "fill", "transition"] {
        if let Some(value) = table.get(key).and_then(|v| v.as_str()) {
            wallpaper.push((key.to_owned(), value.to_owned()));
        }
    }
    if table.get("command").is_some() {
        notes.push("wallpaper.command is not a look key and was not saved".to_owned());
    }
    Ok(())
}

/// Reads the user's own wallpaper file with a byte cap: follows their
/// links (like the shadowed-keys read), but opens `O_NONBLOCK`, requires
/// a regular file, and reads at most `cap + 1` bytes, so a FIFO never
/// hangs the save and a huge file refuses instead of landing.
fn read_user_bytes_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let stat = rustix::fs::fstat(&fd)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return Err(format!("refused: {} is not a regular file", path.display()));
    }
    let mut bytes = Vec::new();
    use std::io::Read as _;
    std::fs::File::from(fd)
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!(
            "{} is {} bytes (over the {cap} byte cap)",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// Whether `bytes` start as a PNG, JPEG, GIF or WebP: the signatures
/// scootbg decodes, before it decodes them.
fn image_sniff(bytes: &[u8]) -> bool {
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'] {
        return true;
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return true;
    }
    if bytes.len() >= 6 && (bytes[..6] == *b"GIF87a" || bytes[..6] == *b"GIF89a") {
        return true;
    }
    if bytes.len() >= 12 && bytes[..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return true;
    }
    false
}

/// Renders the saved `look.toml`: the name, a title to rename, the base
/// when one is live, and only the captured groups. Every string is quoted
/// ([`toml_string`]); the saved directory validates right after, so a value
/// the user's file smuggled in refuses loudly instead of landing.
fn save_manifest(
    name: &str,
    extends: Option<&str>,
    palette: &[(String, String)],
    shape: &[(String, String)],
    cursor: &[(String, String)],
    wallpaper: &[(String, String)],
) -> Result<String, String> {
    let mut out = format!(
        "schema = 1\nname = {}\ntitle = {}\n",
        toml_string(name)?,
        toml_string(name)?
    );
    if let Some(base) = extends {
        out.push_str(&format!("extends = {}\n", toml_string(base)?));
    }
    out.push_str("\n[palette]\n");
    for (role, color) in palette {
        out.push_str(&format!("{role} = {}\n", toml_string(color)?));
    }
    if !shape.is_empty() {
        out.push_str("\n[shape]\n");
        for (token, value) in shape {
            out.push_str(&format!("{token} = {value}\n"));
        }
    }
    if !cursor.is_empty() {
        out.push_str("\n[cursor]\n");
        for (token, value) in cursor {
            if token == "size" {
                out.push_str(&format!("{token} = {value}\n"));
            } else {
                out.push_str(&format!("{token} = {}\n", toml_string(value)?));
            }
        }
    }
    if !wallpaper.is_empty() {
        out.push_str("\n[wallpaper]\n");
        for (token, value) in wallpaper {
            out.push_str(&format!("{token} = {}\n", toml_string(value)?));
        }
    }
    Ok(out)
}

/// One TOML basic string: quoted, with `\` and `"` escaped. Control
/// characters refuse (display text and token values never hold them); the
/// saved directory's validation re-checks every value after.
fn toml_string(value: &str) -> Result<String, String> {
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("cannot save `{value}`: control characters refuse"));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

// ---------------------------------------------------------------------------
// The install-time URL fetch: resolve, classify, pin, download, verify.
// ---------------------------------------------------------------------------

/// How long the install-time fetch may take, all in (past it the install
/// refuses: a swap must never wait on the network, and neither must an
/// install hang on it).
pub const FETCH_TIME_SECS: u64 = 25;

/// How long the fetch's connection may take to establish.
pub const FETCH_CONNECT_SECS: u64 = 10;

/// Resolves a hostname to addresses, injectable so tests pin a hostile
/// resolver (RFC 1918 answers, rebinding answers) without touching DNS.
pub trait Dns {
    fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>>;
}

/// The system resolver.
pub struct SystemDns;

impl Dns for SystemDns {
    fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        use std::net::ToSocketAddrs as _;
        let mut addrs: Vec<IpAddr> = (host, 443).to_socket_addrs()?.map(|a| a.ip()).collect();
        addrs.sort();
        addrs.dedup();
        Ok(addrs)
    }
}

/// Runs the pinned download, injectable so tests record the exact argv a
/// real `curl` would run (no `--location`, `--resolve` pinning the checked
/// address) without touching the network.
pub trait Fetcher {
    fn fetch(&self, url: &str, host: &str, addr: IpAddr, out: &Path) -> Result<(), String>;
}

/// `curl` beside the binary or on `PATH`. `None` when neither resolves.
pub struct CurlFetcher {
    bin: PathBuf,
}

impl CurlFetcher {
    pub fn from_path() -> Option<Self> {
        super::toolkit::find_on_path("curl").map(|bin| Self { bin })
    }

    #[cfg(test)]
    pub fn explicit(bin: PathBuf) -> Self {
        Self { bin }
    }
}

impl Fetcher for CurlFetcher {
    /// The pinned download: no redirects (`--max-redirs 0` and no
    /// `--location`, so a public URL cannot bounce to a private one),
    /// `https` only, and `--resolve` pinning the connection to the address
    /// the classify step checked, so a second lookup cannot rebind it.
    fn fetch(&self, url: &str, host: &str, addr: IpAddr, out: &Path) -> Result<(), String> {
        let argv = curl_argv(url, host, addr, out);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let out = super::apply::run_with_timeout(&self.bin, &args)?;
        if out.status.success() {
            return Ok(());
        }
        // Exit 63 is the cap (`--max-filesize`): name the limit, not the
        // number. `--fail` writes nothing on an HTTP error (notably the
        // 3xx a redirect would answer, since none is followed).
        if out.status.code() == Some(63) {
            return Err(format!(
                "the download is over the {} byte cap",
                apply::MAX_IMAGE_BYTES
            ));
        }
        let detail = String::from_utf8_lossy(&out.stderr);
        let detail = detail.lines().next().unwrap_or_default().trim();
        Err(format!(
            "cannot download {url}: curl exited {}{}",
            out.status
                .code()
                .map(|c| c.to_string())
                .as_deref()
                .unwrap_or("?"),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ))
    }
}

/// The exact `curl` argv a URL fetch runs, as data: unit-tested below
/// ([`tests`]), run by [`CurlFetcher::fetch`].
fn curl_argv(url: &str, host: &str, addr: IpAddr, out: &Path) -> Vec<String> {
    vec![
        "--fail".to_owned(),
        "--silent".to_owned(),
        "--max-redirs".to_owned(),
        "0".to_owned(),
        "--proto".to_owned(),
        "https".to_owned(),
        "--connect-timeout".to_owned(),
        FETCH_CONNECT_SECS.to_string(),
        "--max-time".to_owned(),
        FETCH_TIME_SECS.to_string(),
        "--max-filesize".to_owned(),
        apply::MAX_IMAGE_BYTES.to_string(),
        "--resolve".to_owned(),
        format!("{host}:443:{addr}"),
        "--output".to_owned(),
        out.to_string_lossy().into_owned(),
        "--".to_owned(),
        url.to_owned(),
    ]
}
/// The two injectables behind one fetch: [`Dns`] and [`Fetcher`].
pub trait UrlFetch {
    fn fetch_url(&self, url: &str, sha256: &str, cache_dir: &Path) -> Result<PathBuf, String>;
}

/// The production fetch: [`SystemDns`] plus `curl`.
pub struct ProdFetch;

impl UrlFetch for ProdFetch {
    fn fetch_url(&self, url: &str, sha256: &str, cache_dir: &Path) -> Result<PathBuf, String> {
        let curl = CurlFetcher::from_path()
            .ok_or_else(|| "cannot download: no `curl` beside the binary or on PATH".to_owned())?;
        fetch_url_wallpaper(url, sha256, cache_dir, &SystemDns, &curl)
    }
}

/// Fetches a URL wallpaper into the download cache: resolves the host,
/// refuses any non-public address, pins the checked one, downloads without
/// following redirects, verifies the `sha256` pin and the image magic, and
/// renames into place. A cache hit (present, pin still matching) skips the
/// network entirely.
pub fn fetch_url_wallpaper(
    url: &str,
    sha256: &str,
    cache_dir: &Path,
    dns: &dyn Dns,
    fetch: &dyn Fetcher,
) -> Result<PathBuf, String> {
    let expected = parse_sha256_pin(sha256)?;
    let host = url_host(url)?;
    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("cannot create {}: {error}", cache_dir.display()))?;
    let cached = cache_dir.join(url_cache_name(url));
    if let Some(hit) = cache_hit(&cached, &expected)? {
        return Ok(hit);
    }
    let addrs = dns
        .resolve(&host)
        .map_err(|error| format!("nothing resolves {host}: {error}"))?;
    if addrs.is_empty() {
        return Err(format!("nothing resolves {host}"));
    }
    // Any non-public answer refuses the whole fetch: a hostile resolver
    // mixing a public address with a private one must not get the
    // connection rolled onto the private one.
    for addr in &addrs {
        if !is_public_addr(*addr) {
            return Err(format!(
                "{host} resolves to {addr}, which is not a public address: refusing it"
            ));
        }
    }
    // Pin the first checked address: one lookup, one connection, no second
    // resolution to rebind between the check and the bytes.
    let addr = addrs[0];
    use std::sync::atomic::{AtomicU64, Ordering};
    static FETCH_COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp = cache_dir.join(format!(
        "{}.part-{}-{}",
        url_cache_name(url),
        std::process::id(),
        FETCH_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    if let Err(error) = fetch.fetch(url, &host, addr, &tmp) {
        std::fs::remove_file(&tmp).ok();
        return Err(error);
    }
    let bytes =
        std::fs::read(&tmp).map_err(|error| format!("cannot read the download: {error}"))?;
    if bytes.len() as u64 > apply::MAX_IMAGE_BYTES {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "the download is {} bytes (over the {} byte cap)",
            bytes.len(),
            apply::MAX_IMAGE_BYTES
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != expected {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "the download's sha256 is {actual}, not the pinned {expected}"
        ));
    }
    if !image_sniff(&bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err("the download is not a PNG, JPEG, GIF or WebP image".to_owned());
    }
    std::fs::rename(&tmp, &cached)
        .map_err(|error| format!("cannot cache the download: {error}"))?;
    Ok(cached)
}

/// The cache file for `url`: the hex SHA-256 of the URL, extensionless —
/// the same name scootbg's own fetch computes, so the swap finds install's
/// download without fetching.
pub(crate) fn url_cache_file(cache_dir: &Path, url: &str) -> PathBuf {
    cache_dir.join(url_cache_name(url))
}

/// Resolves a URL wallpaper from the download cache: present, a regular
/// file, and matching the pin when one is pinned. Anything else refuses —
/// this path never fetches (see [`fetch_url_wallpaper`]).
pub(crate) fn cached_url_wallpaper(url: &str, sha256: Option<&str>) -> Result<PathBuf, String> {
    let cache = scootbg_cache_dir()?;
    cached_url_wallpaper_in(url, sha256, &cache)
}

/// [`cached_url_wallpaper`] with the cache directory explicit, so tests
/// pin it without touching the process environment.
pub(crate) fn cached_url_wallpaper_in(
    url: &str,
    sha256: Option<&str>,
    cache_dir: &Path,
) -> Result<PathBuf, String> {
    let cached = url_cache_file(cache_dir, url);
    let meta = match std::fs::metadata(&cached) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "wallpaper `{url}` is not in the download cache: reinstall the look so its URL wallpaper is fetched and verified"
            ));
        }
        Err(error) => {
            return Err(format!("cannot stat {}: {error}", cached.display()));
        }
    };
    if !meta.is_file() {
        return Err(format!(
            "wallpaper `{url}` is not in the download cache: reinstall the look so its URL wallpaper is fetched and verified"
        ));
    }
    if let Some(pin) = sha256 {
        let expected = parse_sha256_pin(pin)?;
        let bytes = std::fs::read(&cached)
            .map_err(|error| format!("cannot read {}: {error}", cached.display()))?;
        if sha256_hex(&bytes) != expected {
            return Err(format!(
                "wallpaper `{url}`: the cached download no longer matches its sha256 pin: reinstall the look"
            ));
        }
    }
    Ok(cached)
}

/// The hex SHA-256 of a URL: the download cache's file name.
fn url_cache_name(url: &str) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(url.as_bytes());
    hex_bytes(&hasher.finalize())
}

/// The cache file's name for tests (the hex SHA-256 of the URL): the swap
/// path and the fetch path must agree on it, byte for byte.
#[cfg(test)]
pub(crate) fn url_cache_name_for_test(url: &str) -> String {
    url_cache_name(url)
}

/// The cached file, when it can be shown as is: present, and matching the
/// pin. A present file failing the pin is removed (stale or corrupt), so
/// the fetch below replaces it.
fn cache_hit(cached: &Path, expected: &str) -> Result<Option<PathBuf>, String> {
    let meta = match std::fs::metadata(cached) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("cannot stat {}: {error}", cached.display()));
        }
    };
    if !meta.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(cached)
        .map_err(|error| format!("cannot read {}: {error}", cached.display()))?;
    if sha256_hex(&bytes) == expected {
        Ok(Some(cached.to_owned()))
    } else {
        let _ = std::fs::remove_file(cached);
        Ok(None)
    }
}

/// Parses a `sha256` pin: 64 hex digits, either case (as `sha256sum`
/// prints). The manifest grammar already enforces this; the check here
/// keeps the fetch total without trusting its caller.
fn parse_sha256_pin(pin: &str) -> Result<String, String> {
    if pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(pin.to_lowercase())
    } else {
        Err("a URL wallpaper requires a sha256 pin of exactly 64 hex digits".to_owned())
    }
}

/// The host of an `https://` wallpaper URL: the authority up to the first
/// `/`, with no userinfo, port or IP literal — the grammar refused those
/// at parse, and the fetch refuses them again before resolving.
fn url_host(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "wallpaper URL must start with `https://`".to_owned())?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err("wallpaper URL has no host".to_owned());
    }
    if authority.contains('@') {
        return Err("wallpaper URL must not contain userinfo".to_owned());
    }
    if authority.contains(':') {
        return Err("wallpaper URL must not name a port".to_owned());
    }
    if authority.parse::<IpAddr>().is_ok() {
        return Err("wallpaper URL must name a DNS host, not an IP literal".to_owned());
    }
    Ok(authority.to_lowercase())
}

/// Whether `ip` is a public unicast address: anything else (loopback,
/// private, link-local, unique-local, unspecified, multicast, broadcast,
/// documentation, benchmarking, transitional relays) refuses the fetch. An
/// IPv4-mapped IPv6 address is judged by the IPv4 inside it.
pub fn is_public_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    // Loopback 127/8, unspecified 0/8, broadcast, multicast 224/4.
    if ip.is_loopback() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return false;
    }
    // Private 10/8, 172.16/12, 192.168/16; link-local 169.254/16.
    if ip.is_private() || ip.is_link_local() {
        return false;
    }
    // Documentation TEST-NET-1/2/3, benchmarking 198.18/15, reserved 240/4.
    if matches!(
        (o[0], o[1]),
        (192, 0) | (198, 51) | (203, 0) | (198, 18) | (198, 19)
    ) {
        return false;
    }
    if o[0] == 100 && (o[1] & 0xc0) == 0x40 {
        return false; // Shared CGNAT 100.64/10.
    }
    if o[0] & 0xf0 == 0xf0 {
        return false; // Reserved 240/4.
    }
    if o[0] == 0 {
        return false; // 0/8 ("this network").
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // IPv4-mapped ::ffff:0:0/96: judge the IPv4 inside.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let seg = ip.segments();
    // Unique-local fc00::/7, link-local fe80::/10.
    if (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // Documentation 2001:db8::/32, discard 100::/64, NAT64 64:ff9b::/96,
    // Teredo 2001::/32, 6to4 2002::/16 (each embeds or hides the real
    // destination, so none is routable as named).
    if (seg[0] == 0x2001 && seg[1] == 0x0db8)
        || (seg[0] == 0x0064 && seg[1] == 0xff9b)
        || (seg[0] == 0x2001 && seg[1] == 0x0000)
        || seg[0] == 0x2002
    {
        return false;
    }
    if seg[0] == 0x0100 {
        return false;
    }
    true
}

/// Hex of a SHA-256 digest (the cache name and the pin comparison).
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex_bytes(&hasher.finalize())
}

fn hex_bytes(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests;
