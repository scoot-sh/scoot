//! The `scoot-look` binary: validate and build source looks with no
//! Smithay in its closure, for Nix builds and the bar-only path.
//!
//! Nothing here needs Nix: it reads plain files out of a look directory and
//! writes plain files into an output directory, on any box with the binary.
//! Usage:
//!
//! ```sh
//! scoot-look validate looks/moonrise [--origin shipped]
//! scoot-look build looks/moonrise --out /tmp/moonrise --wallpaper-abs /tmp/moonrise/wallpaper.png
//! scoot-look build my-green --out /tmp/my-green --catalog ./catalog --name my-green
//! scoot-look import-base16 scheme.yaml [--name NAME] [--extends LOOK] [--out user-looks/NAME]
//! ```
//!
//! `validate` and `build` take `--name NAME` to check the look's `name`
//! against that instead of the directory (a Nix store path carries a
//! hash prefix the check strips; anything else unusual goes here).
//! `build` takes `--catalog DIR` to resolve `extends` through every
//! source look in that directory (one child directory per look): without
//! it only an extends-free look builds. The Nix catalog derivation lays
//! every declared source down in one directory and builds each look
//! against it, so a declared look with `extends` renders at build time
//! like any other.
//! Both take `--trust` to keep an installed look's static terminal
//! leaves (`apps/helix.toml`, `apps/btop.theme`, `apps/starship.toml`):
//! without it they are dropped, with it they render and print first (to
//! stderr, so stdout stays machine-readable).
//! `import-base16` takes `--name` to name the look instead of the
//! scheme's slug, `--extends` to emit a child of that look, and `--out`
//! to write `<dir>/look.toml` (without it the `look.toml` goes to
//! stdout, for the caller to place under `user-looks/`).

use std::collections::BTreeMap;
use std::path::Path;

use scoot_look::{
    ImportOptions, Origin, RenderInputs, STATIC_LEAVES, check_contrast, check_static_leaf,
    import_base16, render_bar, render_foot, render_fuzzel, render_greet, render_gtk3_css,
    render_gtk4_css, render_json, render_leaf_files, render_qt_scheme, render_scoot,
    render_swaylock, resolve, source_hash, validate_apps_allowlist, validate_for_origin,
};

fn usage() -> ! {
    eprintln!("usage:");
    eprintln!(
        "  scoot-look validate <dir> [--origin shipped|declared|user|installed] [--name NAME] [--trust]"
    );
    eprintln!(
        "  scoot-look build <dir> --out <dir> [--wallpaper-abs PATH] [--ui-font-file PATH] [--no-popup] [--origin shipped|declared|user|installed] [--name NAME] [--catalog DIR] [--trust]"
    );
    eprintln!(
        "  scoot-look import-base16 <file> [--name NAME] [--extends LOOK] [--out <dir>] [--origin shipped|declared|user|installed]"
    );
    std::process::exit(2);
}

fn parse_origin(s: &str) -> Origin {
    match s {
        "shipped" => Origin::Shipped,
        "declared" => Origin::Declared,
        "user" => Origin::User,
        "installed" => Origin::Installed,
        _ => {
            eprintln!("unknown origin `{s}`: shipped|declared|user|installed");
            std::process::exit(2);
        }
    }
}

/// Read one file, refusing anything but a regular file (design section
/// 8.3): symlinks (which could point anywhere outside the look),
/// directories, devices, FIFOs and sockets. The pre-check is
/// `symlink_metadata`, which never follows the path, and refuses
/// non-files before any open, so `mkfifo t/look.toml` refuses instead
/// of hanging the tool in `File::open` (round 2 finding 1). The open
/// itself passes `O_NONBLOCK | O_NOFOLLOW` and re-checks the opened
/// handle — the way scootbar reads its config
/// (`crates/scootbar/src/config.rs:288-311`) — so a FIFO swapped in
/// between the stat and the open refuses instead of hanging, and a
/// symlink raced into the same window fails with `ELOOP` and refuses.
/// What this still does not close is a regular file swapped for another
/// regular file mid-read; with plain file contents that race only
/// changes which bytes validate, never the file type read.
fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let refusal = |why: &str| format!("refused: {} is {why}", path.display());
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    if meta.is_symlink() {
        return Err(refusal("a symlink"));
    }
    if !meta.is_file() {
        return Err(refusal("not a regular file"));
    }
    use rustix::fs::{FileType, Mode, OFlags, fstat, open};
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        if e == rustix::io::Errno::LOOP {
            refusal("a symlink")
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let stat = fstat(&fd).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(refusal("not a regular file"));
    }
    let mut bytes = Vec::new();
    use std::io::Read as _;
    std::fs::File::from(fd)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(bytes)
}

fn read_look_toml(dir: &Path) -> Vec<u8> {
    let path = dir.join("look.toml");
    match read_regular_file(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// The name the `name == directory` check runs against (round 2
/// finding 8): an explicit `--name` wins; otherwise the canonicalized
/// directory's file name, so `validate .` checks the real directory
/// name instead of an empty string. A Nix store path
/// (`/nix/store/<32-char-hash>-<name>`) strips the hash prefix, so a
/// look built into the store keeps validating under its own name.
/// Pure over the already-resolved file name, so it stays testable.
fn look_dir_name(canonical_file_name: &str, explicit: Option<&str>) -> Result<String, String> {
    if let Some(name) = explicit {
        return Ok(name.to_string());
    }
    if canonical_file_name.is_empty() {
        return Err("cannot determine the look directory name".to_string());
    }
    if let Some((hash, name)) = canonical_file_name.split_once('-')
        && hash.len() == 32
        && hash
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() && !c.is_ascii_uppercase())
        && !name.is_empty()
    {
        return Ok(name.to_string());
    }
    Ok(canonical_file_name.to_string())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| usage());
    match cmd.as_str() {
        "validate" => {
            let dir = args.next().unwrap_or_else(|| usage());
            let mut origin = Origin::User;
            let mut name: Option<String> = None;
            let mut trust = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--origin" => origin = parse_origin(&args.next().unwrap_or_else(|| usage())),
                    "--name" => name = Some(args.next().unwrap_or_else(|| usage())),
                    "--trust" => trust = true,
                    _ => usage(),
                }
            }
            validate_dir(Path::new(&dir), origin, name.as_deref(), trust);
        }
        "build" => {
            let dir = args.next().unwrap_or_else(|| usage());
            let mut out: Option<String> = None;
            let mut inputs = RenderInputs {
                bar_popup: true,
                ..RenderInputs::default()
            };
            let mut origin = Origin::User;
            let mut name: Option<String> = None;
            let mut catalog: Option<String> = None;
            let mut trust = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--out" => out = Some(args.next().unwrap_or_else(|| usage())),
                    "--wallpaper-abs" => {
                        inputs.wallpaper_abs = Some(args.next().unwrap_or_else(|| usage()));
                    }
                    "--ui-font-file" => {
                        inputs.ui_font_file = Some(args.next().unwrap_or_else(|| usage()));
                    }
                    "--no-popup" => inputs.bar_popup = false,
                    "--origin" => origin = parse_origin(&args.next().unwrap_or_else(|| usage())),
                    "--name" => name = Some(args.next().unwrap_or_else(|| usage())),
                    "--catalog" => catalog = Some(args.next().unwrap_or_else(|| usage())),
                    "--trust" => trust = true,
                    _ => usage(),
                }
            }
            let out = out.unwrap_or_else(|| usage());
            build_dir(
                Path::new(&dir),
                Path::new(&out),
                &inputs,
                origin,
                name.as_deref(),
                catalog.as_deref().map(Path::new),
                trust,
            );
        }
        "import-base16" => {
            let file = args.next().unwrap_or_else(|| usage());
            let mut name: Option<String> = None;
            let mut extends: Option<String> = None;
            let mut out: Option<String> = None;
            let mut origin = Origin::User;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--name" => name = Some(args.next().unwrap_or_else(|| usage())),
                    "--extends" => extends = Some(args.next().unwrap_or_else(|| usage())),
                    "--out" => out = Some(args.next().unwrap_or_else(|| usage())),
                    "--origin" => origin = parse_origin(&args.next().unwrap_or_else(|| usage())),
                    _ => usage(),
                }
            }
            import_scheme(
                Path::new(&file),
                name.as_deref(),
                extends.as_deref(),
                out.as_deref().map(Path::new),
                origin,
            );
        }
        _ => usage(),
    }
}

fn load_raw(dir: &Path, explicit_name: Option<&str>) -> (scoot_look::RawLook, Vec<u8>) {
    let bytes = read_look_toml(dir);
    match scoot_look::parse_look_toml(&bytes) {
        Ok(raw) => {
            // The look's `name` must match its directory name (site
            // reference): a copy under another name does not validate.
            // The directory resolves through `canonicalize` first, so
            // `validate .` checks the real directory name, and a Nix
            // store path (`<hash>-<name>`) strips its hash prefix;
            // `--name` overrides both for callers that lay the source
            // down under another name.
            let canonical = std::fs::canonicalize(dir).unwrap_or_else(|e| {
                eprintln!("cannot resolve {}: {e}", dir.display());
                std::process::exit(1);
            });
            let file_name = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let dir_name = look_dir_name(file_name, explicit_name).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            });
            if let Some(e) = scoot_look::check_name_matches_dir(&raw, &dir_name) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            (raw, bytes)
        }
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            std::process::exit(1);
        }
    }
}

fn validate_dir(dir: &Path, origin: Origin, explicit_name: Option<&str>, trust: bool) {
    let (raw, _) = load_raw(dir, explicit_name);
    let mut failed = false;
    for e in validate_for_origin(&raw, origin) {
        eprintln!("error: {e}");
        failed = true;
    }
    for e in validate_apps_allowlist(&raw.apps) {
        eprintln!("error: {e}");
        failed = true;
    }
    // Contrast needs a resolved look; without a catalog only single-file
    // (extends-free) looks resolve here. Warnings go to stderr: stdout
    // stays machine-readable (import without `--out` prints `look.toml`
    // there).
    if raw.extends.is_none() {
        let map = BTreeMap::from([(raw.name.clone(), (raw.clone(), origin))]);
        match resolve(&map, &raw.name) {
            Ok(look) => {
                let report = check_contrast(&look, origin);
                for w in report.warnings() {
                    eprintln!("warning: {} is {} (bar {})", w.name, w.ratio, w.threshold);
                }
                for r in report.refusals() {
                    eprintln!("error: {} is {} (bar {})", r.name, r.ratio, r.threshold);
                    failed = true;
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
    }
    // The terminal static leaves, as `build` renders them (or drops
    // them, for an installed look without `trust`). Without a catalog
    // only this directory's own files are checked: an `extends` base is
    // unknown here.
    let _ = static_leaf_files(dir, None, &raw, origin, trust);
    if failed {
        std::process::exit(1);
    }
    println!("valid: {}", dir.display());
}

/// Reads every source look in a catalog directory: one child directory
/// per look, each with a `look.toml` whose `name` matches the directory.
/// A child that is not a directory, or a directory without a readable
/// manifest, is skipped (the derivation lays down only look dirs, and a
/// stray file must not fail every look's build); anything the manifest
/// says wrong — unparsable, misnamed, forbidden for the origin — fails
/// with the look named. Sorted, so the map is deterministic whatever the
/// filesystem returns. Pure over the directory (I/O, no exit), so the
/// refusal paths stay testable; the caller exits on `Err`.
fn read_catalog_sources(
    dir: &Path,
    origin: Origin,
) -> Result<BTreeMap<String, (scoot_look::RawLook, Origin)>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    let mut names: Vec<_> = entries.collect();
    names.sort_by_key(|entry| entry.as_ref().map(|e| e.file_name()).unwrap_or_default());
    let mut sources = BTreeMap::new();
    for entry in names {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
        let meta = std::fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot stat {}: {e}", entry.path().display()))?;
        if !(meta.is_dir() || meta.is_symlink()) {
            continue;
        }
        let manifest = entry.path().join("look.toml");
        let bytes = match read_regular_file(&manifest) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let raw = match scoot_look::parse_look_toml(&bytes) {
            Ok(raw) => raw,
            Err(errors) => {
                let detail = errors
                    .iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!(
                    "catalog {}: invalid look.toml: {detail}",
                    entry.path().display()
                ));
            }
        };
        let dir_name = entry.file_name().to_str().unwrap_or_default().to_owned();
        let dir_name = look_dir_name(&dir_name, None)
            .map_err(|e| format!("catalog {}: {e}", entry.path().display()))?;
        if let Some(e) = scoot_look::check_name_matches_dir(&raw, &dir_name) {
            return Err(format!("catalog {}: {e}", entry.path().display()));
        }
        let mut invalid = Vec::new();
        for e in scoot_look::validate_for_origin(&raw, origin)
            .into_iter()
            .chain(scoot_look::validate_apps_allowlist(&raw.apps))
        {
            invalid.push(e.to_string());
        }
        if let Some(first) = invalid.into_iter().next() {
            return Err(format!(
                "catalog {}: invalid for origin {}: {first}",
                entry.path().display(),
                origin_name(origin)
            ));
        }
        sources.insert(raw.name.clone(), (raw, origin));
    }
    Ok(sources)
}

/// The origin's name, as errors show it.
fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Shipped => "shipped",
        Origin::Declared => "declared",
        Origin::User => "user",
        Origin::Installed => "installed",
    }
}

/// One static terminal leaf read from a source tree: the rendered
/// relative path with its bytes. The walk starts at `dir` and follows
/// `extends` through `catalog` (one child directory per look, the Nix
/// derivation's shape): the nearest look carrying the file wins, so a
/// child inherits its base's leaves unless it names its own. Without a
/// catalog only `dir`'s own files are read (`validate` takes none; an
/// `extends` base is unknown there).
///
/// An installed look drops them without `trust` (its formats are never
/// generated, so a hostile file could carry anything); with `trust` each
/// prints to stderr first, so the reply shows what was trusted before
/// anything renders. Reads refuse anything but a regular file, the way
/// `look.toml` does; a missing file at every level simply renders no
/// leaf for it.
fn static_leaf_files(
    dir: &Path,
    catalog: Option<&Path>,
    raw: &scoot_look::RawLook,
    origin: Origin,
    trust: bool,
) -> Vec<(String, Vec<u8>)> {
    // The chain of source dirs, nearest first, cycle-guarded (resolve
    // already refused cycles through the manifest map; the dirs follow
    // the same names, so this only bounds a divergent filesystem).
    let mut chain: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    let mut extends = raw.extends.clone();
    while let Some(base) = extends {
        if chain.len() > scoot_look::schema::MAX_EXTENDS_DEPTH as usize {
            eprintln!("error: extends chain deeper than expected");
            std::process::exit(1);
        }
        let Some(catalog) = catalog else {
            break;
        };
        let base_dir = catalog.join(&base);
        let manifest = base_dir.join("look.toml");
        let bytes = match read_regular_file(&manifest) {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        let base_raw = match scoot_look::parse_look_toml(&bytes) {
            Ok(base_raw) => base_raw,
            Err(errors) => {
                for e in errors {
                    eprintln!("error: catalog {base}: {e}");
                }
                std::process::exit(1);
            }
        };
        chain.push(base_dir);
        extends = base_raw.extends.clone();
    }
    if origin == Origin::Installed && !trust {
        let mut dropped = Vec::new();
        for dir in &chain {
            for leaf in STATIC_LEAVES {
                if dir.join(leaf.path).is_file() && !dropped.contains(&leaf.path) {
                    dropped.push(leaf.path);
                }
            }
        }
        if !dropped.is_empty() {
            dropped.sort();
            eprintln!(
                "note: dropping {}: `{}` is an installed look (pass --trust to keep them)",
                dropped.join(", "),
                raw.name,
            );
        }
        return Vec::new();
    }
    let mut found: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    for dir in &chain {
        for leaf in STATIC_LEAVES {
            if found.contains_key(leaf.path) {
                continue;
            }
            let candidate = dir.join(leaf.path);
            if std::fs::symlink_metadata(&candidate).is_err() {
                continue;
            }
            let bytes = match read_regular_file(&candidate) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = check_static_leaf(leaf.path, &bytes) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            found.insert(leaf.path, bytes);
        }
    }
    let mut files = Vec::new();
    for leaf in STATIC_LEAVES {
        if let Some(bytes) = found.remove(leaf.path) {
            if origin == Origin::Installed {
                eprintln!(
                    "trusted static leaf {} ({} bytes):\n{}",
                    leaf.path,
                    bytes.len(),
                    String::from_utf8_lossy(&bytes),
                );
            }
            files.push((leaf.path.to_owned(), bytes));
        }
    }
    files
}

fn build_dir(
    dir: &Path,
    out: &Path,
    inputs: &RenderInputs,
    origin: Origin,
    explicit_name: Option<&str>,
    catalog: Option<&Path>,
    trust: bool,
) {
    let (raw, bytes) = load_raw(dir, explicit_name);
    let map = match catalog {
        Some(catalog) => match read_catalog_sources(catalog, origin) {
            Ok(mut sources) => {
                // The directory's own entry wins over a same-named
                // catalog entry (a tweaked copy never silently builds
                // the catalog's look).
                sources.insert(raw.name.clone(), (raw.clone(), origin));
                sources
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        None => {
            if raw.extends.is_some() {
                eprintln!(
                    "error: build takes a single source without extends; pass --catalog DIR to resolve it"
                );
                std::process::exit(1);
            }
            BTreeMap::from([(raw.name.clone(), (raw.clone(), origin))])
        }
    };
    let mut failed = false;
    for e in validate_for_origin(&raw, origin)
        .into_iter()
        .chain(validate_apps_allowlist(&raw.apps))
    {
        eprintln!("error: {e}");
        failed = true;
    }
    if failed {
        std::process::exit(1);
    }
    let name = raw.name.clone();
    let look = match resolve(&map, &name) {
        Ok(look) => look,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let report = check_contrast(&look, origin);
    if !report.refusals().is_empty() {
        for r in report.refusals() {
            eprintln!("error: {} is {} (bar {})", r.name, r.ratio, r.threshold);
        }
        std::process::exit(1);
    }
    let scoot = render_scoot(&look, inputs).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let bar = render_bar(&look, inputs).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    // The toolkit fragments beside the first-party ones: per-version
    // `gtk.css` files and the Qt scheme render from tokens alone.
    // `qt6ct.conf`, `settings.ini` and the fontconfig file are
    // apply-rendered (the conf points through the state link, which
    // `build` does not know).
    let gtk3_css = render_gtk3_css(&look);
    let gtk4_css = render_gtk4_css(&look);
    let qt_scheme = render_qt_scheme(&look);
    let foot = render_foot(&look).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let fuzzel = render_fuzzel(&look).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let swaylock = render_swaylock(&look).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    // The login-screen fragment scootgreet reads at system build time
    // (never live): beside the first-party ones, so every catalog render
    // carries the greeter's look.
    let greet = render_greet(&look).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let leaves = render_leaf_files(&look).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    // The terminal static leaves, inherited through the catalog when the
    // look extends a base (dropped from installed looks without `trust`).
    let static_files = static_leaf_files(dir, catalog, &raw, origin, trust);
    std::fs::create_dir_all(out).unwrap_or_else(|e| {
        eprintln!("cannot create {}: {e}", out.display());
        std::process::exit(1);
    });
    for dir in ["gtk", "qt", "apps"] {
        std::fs::create_dir_all(out.join(dir)).unwrap_or_else(|e| {
            eprintln!("cannot create {}: {e}", out.join(dir).display());
            std::process::exit(1);
        });
    }
    let mut files: Vec<(&str, String)> = vec![
        ("scoot.toml", scoot),
        ("bar.toml", bar),
        ("fuzzel.ini", fuzzel),
        ("swaylock", swaylock),
        ("greet.toml", greet),
        (
            "render.json",
            render_json(&look.name, &source_hash(&bytes), inputs),
        ),
        ("gtk/gtk-3.0.css", gtk3_css),
        ("gtk/gtk-4.0.css", gtk4_css),
        ("qt/scoot-look.conf", qt_scheme),
    ];
    // No `[terminal]` table means no foot fragment, never a synthesized
    // palette: the file is simply absent.
    if let Some(foot) = foot {
        files.push(("foot.ini", foot));
    }
    for (file, contents) in &files {
        std::fs::write(out.join(file), contents).unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", out.join(file).display());
            std::process::exit(1);
        });
    }
    for (file, contents) in &leaves {
        let path = out.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("cannot create {}: {e}", parent.display());
                std::process::exit(1);
            });
        }
        std::fs::write(&path, contents).unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", path.display());
            std::process::exit(1);
        });
    }
    for (file, contents) in &static_files {
        let path = out.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("cannot create {}: {e}", parent.display());
                std::process::exit(1);
            });
        }
        std::fs::write(&path, contents).unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", path.display());
            std::process::exit(1);
        });
    }
    println!("built {} -> {}", dir.display(), out.display());
}

/// `scoot-look import-base16 FILE`: map one scheme to a source look.
/// The emitted `look.toml` is verified through the same pipeline as
/// `validate` before anything is written; a contrast shortfall in a look
/// the user's own configuration produced warns (to stderr) and never
/// refuses, except a child (`--extends`) skips the contrast check here:
/// the base is unknown without the catalog. `--extends` is never checked
/// against a catalog here. With `--out DIR` the text lands in
/// `DIR/look.toml` (the caller places that under `user-looks/`); the
/// directory must match the look name or the command errors. Without
/// `--out` the text goes to stdout (warnings still go to stderr, so the
/// stdout parses as TOML). `--origin` defaults to `user`.
fn import_scheme(
    file: &Path,
    name: Option<&str>,
    extends: Option<&str>,
    out: Option<&Path>,
    origin: Origin,
) {
    let bytes = match read_regular_file(file) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    let text = match import_base16(
        &bytes,
        &ImportOptions {
            name: name.map(str::to_string),
            extends: extends.map(str::to_string),
            file_stem: stem,
        },
    ) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let raw = match scoot_look::parse_look_toml(text.as_bytes()) {
        Ok(raw) => raw,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            std::process::exit(1);
        }
    };
    let mut failed = false;
    for e in validate_for_origin(&raw, origin)
        .into_iter()
        .chain(validate_apps_allowlist(&raw.apps))
    {
        eprintln!("error: {e}");
        failed = true;
    }
    // Contrast needs a resolved look; without a catalog only single-file
    // (extends-free) imports resolve here, as in `validate`. A child
    // (`--extends`) skips the check here: the base is unknown without the
    // catalog, so the catalog owner reports the resolved contrast.
    // Warnings go to stderr so stdout without `--out` stays pure TOML.
    if raw.extends.is_none() {
        let map = BTreeMap::from([(raw.name.clone(), (raw.clone(), origin))]);
        match resolve(&map, &raw.name) {
            Ok(look) => {
                let report = check_contrast(&look, origin);
                for w in report.warnings() {
                    eprintln!("warning: {} is {} (bar {})", w.name, w.ratio, w.threshold);
                }
                for r in report.refusals() {
                    eprintln!("error: {} is {} (bar {})", r.name, r.ratio, r.threshold);
                    failed = true;
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    match out {
        Some(dir) => {
            // A mismatched directory fails `validate` (name must match
            // the directory), so refuse here with a nonzero exit instead
            // of writing a look that cannot validate.
            if let Some(base) = dir.file_name().and_then(|n| n.to_str())
                && base != raw.name
            {
                eprintln!(
                    "error: look `{}` does not match directory `{base}`: the name check expects them to match",
                    raw.name
                );
                std::process::exit(1);
            }
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("cannot create {}: {e}", dir.display());
                std::process::exit(1);
            }
            let path = dir.join("look.toml");
            if let Err(e) = std::fs::write(&path, &text) {
                eprintln!("cannot write {}: {e}", path.display());
                std::process::exit(1);
            }
            println!("imported {} -> {}", file.display(), path.display());
        }
        None => print!("{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp_case(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scoot-look-main-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp case dir");
        dir
    }

    /// Run `f` on a path, giving up after 10 seconds: a regression to
    /// blocking `File::open` on a FIFO fails this test instead of
    /// hanging the suite.
    fn without_hanging<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ()> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(10)).map_err(|_| ())
    }

    #[test]
    fn fifo_refuses_without_hanging() {
        // Round 2 finding 1: `mkfifo t/look.toml; scoot-look validate t`
        // hung in `File::open`, past the symlink-only pre-check.
        let dir = tmp_case("fifo");
        let fifo = dir.join("look.toml");
        let _ = std::fs::remove_file(&fifo);
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs");
        assert!(status.success(), "mkfifo failed");
        let out =
            without_hanging(move || read_regular_file(&fifo)).expect("hang: FIFO open blocked");
        assert!(out.is_err(), "FIFO reads");
    }

    #[test]
    fn symlink_refuses() {
        let dir = tmp_case("symlink");
        let real = dir.join("real.toml");
        std::fs::write(&real, b"schema = 1\n").expect("write");
        let link = dir.join("look.toml");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        assert!(read_regular_file(&link).is_err());
    }

    #[test]
    fn device_refuses_without_hanging() {
        // A character device is never a look: `/dev/null` must refuse,
        // and a blocking device must never hang the read.
        let out = without_hanging(|| read_regular_file(Path::new("/dev/null"))).expect("hang");
        assert!(out.is_err(), "/dev/null reads");
    }

    #[test]
    fn regular_file_reads() {
        let dir = tmp_case("regular");
        let file = dir.join("look.toml");
        std::fs::write(&file, b"schema = 1\n").expect("write");
        assert_eq!(
            read_regular_file(&file).expect("reads").as_slice(),
            b"schema = 1\n"
        );
    }

    #[test]
    fn dir_name_shapes() {
        // Round 2 finding 8: `validate .` yielded an empty directory
        // name, and a Nix store path yields `<hash>-<name>`.
        assert_eq!(look_dir_name("moonrise", None).expect("plain"), "moonrise");
        assert_eq!(
            look_dir_name("abc123abc123abc123abc123abc12345-moonrise", None).expect("store path"),
            "moonrise"
        );
        // An explicit `--name` wins over any directory shape.
        assert_eq!(
            look_dir_name("other", Some("moonrise")).expect("explicit"),
            "moonrise"
        );
        // A short prefix is a real name, not a store hash.
        assert_eq!(
            look_dir_name("abc-moonrise", None).expect("short prefix"),
            "abc-moonrise"
        );
    }

    fn catalog_toml(name: &str, extra: &str) -> String {
        format!(
            r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "dark"
license = "MIT"
{extra}
[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DejaVu Sans"
sans = "DejaVu Sans"
mono = "DejaVu Sans Mono"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "#2B3648"
mode = "fill"
"##
        )
    }

    fn write_catalog_look(catalog: &std::path::Path, name: &str, extra: &str) {
        let dir = catalog.join(name);
        std::fs::create_dir_all(&dir).expect("look dir");
        std::fs::write(dir.join("look.toml"), catalog_toml(name, extra)).expect("look.toml");
    }

    #[test]
    fn catalog_sources_resolve_extends() {
        // The Nix derivation's shape: every declared source in one
        // directory, a child resolving its base through it.
        let root = tmp_case("catalog-ok");
        write_catalog_look(&root, "base-a", "");
        write_catalog_look(&root, "child-b", "extends = \"base-a\"\n");
        let sources = read_catalog_sources(&root, Origin::Declared).expect("sources");
        assert_eq!(sources.len(), 2);
        let resolved = resolve(&sources, "child-b").expect("child resolves");
        assert_eq!(resolved.name, "child-b");
        assert_eq!(resolved.palette.accent, "#FFA45C");
    }

    #[test]
    fn catalog_sources_skip_strays() {
        // Stray files and dirs without a manifest are skipped: a README
        // beside the looks must not fail every look's build.
        let root = tmp_case("catalog-strays");
        std::fs::write(root.join("README.md"), b"notes").expect("readme");
        std::fs::create_dir_all(root.join("empty")).expect("empty dir");
        write_catalog_look(&root, "base-a", "");
        let sources = read_catalog_sources(&root, Origin::Declared).expect("sources");
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn catalog_sources_refuse_invalid_manifest() {
        // A broken manifest fails the build with the look named, before
        // anything renders.
        let root = tmp_case("catalog-bad");
        write_catalog_look(&root, "base-a", "");
        let dir = root.join("broken-b");
        std::fs::create_dir_all(&dir).expect("look dir");
        std::fs::write(dir.join("look.toml"), b"schema = 1\nname = 42\n").expect("bad toml");
        let error = read_catalog_sources(&root, Origin::Declared).expect_err("refuses");
        assert!(error.contains("broken-b"), "names the look: {error}");
    }

    #[test]
    fn catalog_sources_refuse_misnamed_look() {
        // The manifest name must match its directory, as in the live
        // catalog scan.
        let root = tmp_case("catalog-misnamed");
        let dir = root.join("dir-name");
        std::fs::create_dir_all(&dir).expect("look dir");
        std::fs::write(dir.join("look.toml"), catalog_toml("other-name", "")).expect("look.toml");
        let error = read_catalog_sources(&root, Origin::Declared).expect_err("refuses");
        assert!(error.contains("dir-name"), "names the directory: {error}");
    }
}
