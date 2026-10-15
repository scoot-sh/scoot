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
//! scoot-look import-base16 scheme.yaml [--name NAME] [--extends LOOK] [--out user-looks/NAME]
//! ```
//!
//! `validate` and `build` take `--name NAME` to check the look's `name`
//! against that instead of the directory (a Nix store path carries a
//! hash prefix the check strips; anything else unusual goes here).
//! `import-base16` takes `--name` to name the look instead of the
//! scheme's slug, `--extends` to emit a child of that look, and `--out`
//! to write `<dir>/look.toml` (without it the `look.toml` goes to
//! stdout, for the caller to place under `user-looks/`).

use std::collections::BTreeMap;
use std::path::Path;

use scoot_look::{
    ImportOptions, Origin, RenderInputs, check_contrast, import_base16, render_bar,
    render_gtk3_css, render_gtk4_css, render_json, render_qt_scheme, render_scoot, resolve,
    source_hash, validate_apps_allowlist, validate_for_origin,
};

fn usage() -> ! {
    eprintln!("usage:");
    eprintln!(
        "  scoot-look validate <dir> [--origin shipped|declared|user|installed] [--name NAME]"
    );
    eprintln!(
        "  scoot-look build <dir> --out <dir> [--wallpaper-abs PATH] [--ui-font-file PATH] [--no-popup] [--origin shipped|declared|user|installed] [--name NAME]"
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
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--origin" => origin = parse_origin(&args.next().unwrap_or_else(|| usage())),
                    "--name" => name = Some(args.next().unwrap_or_else(|| usage())),
                    _ => usage(),
                }
            }
            validate_dir(Path::new(&dir), origin, name.as_deref());
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

fn validate_dir(dir: &Path, origin: Origin, explicit_name: Option<&str>) {
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
    if failed {
        std::process::exit(1);
    }
    println!("valid: {}", dir.display());
}

fn build_dir(
    dir: &Path,
    out: &Path,
    inputs: &RenderInputs,
    origin: Origin,
    explicit_name: Option<&str>,
) {
    let (raw, bytes) = load_raw(dir, explicit_name);
    if raw.extends.is_some() {
        eprintln!("error: build takes a single source without extends; resolve the catalog first");
        std::process::exit(1);
    }
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
    let map = BTreeMap::from([(raw.name.clone(), (raw, origin))]);
    let name = map.keys().next().unwrap().clone();
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
    std::fs::create_dir_all(out).unwrap_or_else(|e| {
        eprintln!("cannot create {}: {e}", out.display());
        std::process::exit(1);
    });
    for dir in ["gtk", "qt"] {
        std::fs::create_dir_all(out.join(dir)).unwrap_or_else(|e| {
            eprintln!("cannot create {}: {e}", out.join(dir).display());
            std::process::exit(1);
        });
    }
    for (file, contents) in [
        ("scoot.toml", scoot),
        ("bar.toml", bar),
        (
            "render.json",
            render_json(&look.name, &source_hash(&bytes), inputs),
        ),
        ("gtk/gtk-3.0.css", gtk3_css),
        ("gtk/gtk-4.0.css", gtk4_css),
        ("qt/scoot-look.conf", qt_scheme),
    ] {
        std::fs::write(out.join(file), contents).unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", out.join(file).display());
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
}
