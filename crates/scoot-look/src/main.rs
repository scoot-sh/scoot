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
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use scoot_look::{
    Origin, RenderInputs, check_contrast, render_bar, render_json, render_scoot, resolve,
    source_hash, validate_apps_allowlist, validate_for_origin,
};

fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  scoot-look validate <dir> [--origin shipped|declared|user|installed]");
    eprintln!(
        "  scoot-look build <dir> --out <dir> [--wallpaper-abs PATH] [--ui-font-file PATH] [--no-popup]"
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

fn read_look_toml(dir: &Path) -> Vec<u8> {
    let path = dir.join("look.toml");
    std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", path.display());
        std::process::exit(1);
    })
}

/// Refuse anything but a regular file inside `dir`: a symlink, device, FIFO
/// or socket refuses the read (design section 8.3).
fn check_regular_inside(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path).unwrap_or_else(|e| {
        eprintln!("cannot stat {}: {e}", path.display());
        std::process::exit(1);
    });
    if !meta.is_file() {
        eprintln!("refused: {} is not a regular file", path.display());
        std::process::exit(1);
    }
    path
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| usage());
    match cmd.as_str() {
        "validate" => {
            let dir = args.next().unwrap_or_else(|| usage());
            let mut origin = Origin::User;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--origin" => origin = parse_origin(&args.next().unwrap_or_else(|| usage())),
                    _ => usage(),
                }
            }
            validate_dir(Path::new(&dir), origin);
        }
        "build" => {
            let dir = args.next().unwrap_or_else(|| usage());
            let mut out: Option<String> = None;
            let mut inputs = RenderInputs {
                bar_popup: true,
                ..RenderInputs::default()
            };
            let mut origin = Origin::User;
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
                    _ => usage(),
                }
            }
            let out = out.unwrap_or_else(|| usage());
            build_dir(Path::new(&dir), Path::new(&out), &inputs, origin);
        }
        _ => usage(),
    }
}

fn load_raw(dir: &Path) -> (scoot_look::RawLook, Vec<u8>) {
    let path = check_regular_inside(dir, "look.toml");
    let _ = path;
    let bytes = read_look_toml(dir);
    match scoot_look::parse_look_toml(&bytes) {
        Ok(raw) => (raw, bytes),
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            std::process::exit(1);
        }
    }
}

fn validate_dir(dir: &Path, origin: Origin) {
    let (raw, _) = load_raw(dir);
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
    // (extends-free) looks resolve here.
    if raw.extends.is_none() {
        let map = BTreeMap::from([(raw.name.clone(), (raw.clone(), origin))]);
        match resolve(&map, &raw.name) {
            Ok(look) => {
                let report = check_contrast(&look, origin);
                for w in report.warnings() {
                    println!("warning: {} is {} (bar {})", w.name, w.ratio, w.threshold);
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

fn build_dir(dir: &Path, out: &Path, inputs: &RenderInputs, origin: Origin) {
    let (raw, bytes) = load_raw(dir);
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
    std::fs::create_dir_all(out).unwrap_or_else(|e| {
        eprintln!("cannot create {}: {e}", out.display());
        std::process::exit(1);
    });
    for (file, contents) in [
        ("scoot.toml", scoot),
        ("bar.toml", bar),
        (
            "render.json",
            render_json(&look.name, &source_hash(&bytes), inputs),
        ),
    ] {
        std::fs::write(out.join(file), contents).unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", out.join(file).display());
            std::process::exit(1);
        });
    }
    println!("built {} -> {}", dir.display(), out.display());
}
