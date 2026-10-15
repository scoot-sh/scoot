//! The non-Nix path: listing, validating and rendering a look uses plain
//! files and this crate only. The full pipeline must succeed with no `nix`
//! helper involved, so this test runs it with `PATH` pointed at an empty
//! directory: anything the pipeline shells out to (or looks up on `PATH`)
//! would fail, and success proves the render is plain files plus this crate.
//! Purity is also structural: the library links no process, environment or
//! network API (no `std::process`, no `std::env`, no sockets in `src/`,
//! outside this test).

use std::collections::BTreeMap;

use scoot_look::{
    ImportOptions, Origin, RenderInputs, check_contrast, import_base16, render_bar, render_scoot,
    resolve,
};

const MINIMAL: &str = r##"
schema = 1
name = "moonrise"
title = "Moonrise"
description = "Slate navy."
polarity = "dark"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DroidSansM Nerd Font Propo"
sans = "DejaVu Sans"
mono = "FiraCode Nerd Font"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24
accent = "#FF9A49"

[wallpaper]
color = "#2B3648"
"##;

/// Run `f` with a `PATH` that resolves nothing: an empty directory. A test
/// that passes because a helper binary happened to be installed is a test
/// of the wrong path; a pipeline that needs no helper passes either way.
fn without_helpers_on_path(f: impl FnOnce()) {
    let old = std::env::var_os("PATH");
    let empty = std::env::temp_dir().join("scoot-look-empty-path");
    std::fs::create_dir_all(&empty).expect("empty PATH dir");
    // `set_var` is `unsafe` (it can race another thread's `getenv`); the
    // body runs alone here and restores the value before returning.
    unsafe { std::env::set_var("PATH", &empty) };
    f();
    if let Some(path) = old {
        unsafe { std::env::set_var("PATH", path) };
    }
}

#[test]
fn validate_render_contrast_without_nix() {
    without_helpers_on_path(|| {
        let raw = scoot_look::parse_look_toml(MINIMAL.as_bytes()).expect("parses");
        assert!(scoot_look::validate_for_origin(&raw, Origin::User).is_empty());
        assert!(scoot_look::validate_apps_allowlist(&raw.apps).is_empty());
        let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::User))]);
        let look = resolve(&map, "moonrise").expect("resolves");
        assert!(check_contrast(&look, Origin::User).refusals().is_empty());
        let inputs = RenderInputs::default();
        let scoot = render_scoot(&look, &inputs).expect("renders scoot.toml");
        let bar = render_bar(&look, &inputs).expect("renders bar.toml");
        let _: toml::Value = toml::from_str(&scoot).expect("scoot.toml parses");
        let _: toml::Value = toml::from_str(&bar).expect("bar.toml parses");
        // Round 2 finding 3: without `--ui-font-file` the bar fragment
        // carries no `font` key (scootbar's is a file path, not the
        // family name), so the bar falls back to its well-known paths.
        for line in bar.lines() {
            let key = line.split('=').next().map(str::trim).unwrap_or("");
            assert!(
                key != "font" && key != "fallback-fonts",
                "non-Nix default emits {key}"
            );
        }
    });
}

/// A base16 scheme becomes a source look with plain files only: import,
/// parse, resolve, contrast and render under an empty `PATH`, the way a
/// plain binary install runs it (no Nix anywhere in the path).
#[test]
fn import_base16_without_nix() {
    without_helpers_on_path(|| {
        let scheme = "name: Plain\nbase00: 090300\nbase01: 3a3432\nbase02: 4a4543\nbase03: 5c5855\nbase04: 807d7c\nbase05: a5a2a2\nbase06: d6d5d4\nbase07: f7f7f7\nbase08: db2d20\nbase09: e8bbd0\nbase0A: fded02\nbase0B: 01a252\nbase0C: b5e4f4\nbase0D: 01a0e4\nbase0E: a16a94\nbase0F: cdab53\n";
        let look = import_base16(scheme.as_bytes(), &ImportOptions::default()).expect("imports");
        let raw = scoot_look::parse_look_toml(look.as_bytes()).expect("parses");
        assert_eq!(raw.name, "plain");
        assert!(scoot_look::validate_for_origin(&raw, Origin::User).is_empty());
        let map = BTreeMap::from([("plain".to_string(), (raw, Origin::User))]);
        let resolved = resolve(&map, "plain").expect("resolves");
        assert_eq!(resolved.palette.accent, "#01a0e4");
        assert!(
            check_contrast(&resolved, Origin::User)
                .refusals()
                .is_empty()
        );
        let inputs = RenderInputs::default();
        let _: toml::Value =
            toml::from_str(&render_scoot(&resolved, &inputs).expect("renders")).expect("parses");
        let _: toml::Value =
            toml::from_str(&render_bar(&resolved, &inputs).expect("renders")).expect("parses");
    });
}

/// Finding 2: without `--out`, stdout is pure `look.toml` even when the
/// import warns (low contrast): warnings go to stderr.
#[test]
fn import_stdout_parses_despite_warnings() {
    let dir = std::env::temp_dir().join(format!("scoot-look-stdout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let scheme = dir.join("low.yaml");
    std::fs::write(
        &scheme,
        "name: low\nauthor: t\nbase00: \"#808080\"\nbase01: \"#808080\"\nbase02: \"#808080\"\nbase03: \"#808080\"\nbase04: \"#808080\"\nbase05: \"#808080\"\nbase06: \"#808080\"\nbase07: \"#808080\"\nbase08: \"#808080\"\nbase09: \"#808080\"\nbase0A: \"#808080\"\nbase0B: \"#808080\"\nbase0C: \"#808080\"\nbase0D: \"#808080\"\nbase0E: \"#808080\"\nbase0F: \"#808080\"\n",
    )
    .expect("write scheme");
    let bin = env!("CARGO_BIN_EXE_scoot-look");
    let out = std::process::Command::new(bin)
        .arg("import-base16")
        .arg(&scheme)
        .output()
        .expect("runs");
    assert!(out.status.success(), "exits 0: {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("stdout UTF-8");
    assert!(
        !stdout.contains("warning:"),
        "no warnings on stdout:\n{stdout}"
    );
    let value: toml::Value = toml::from_str(&stdout).expect("stdout parses as TOML");
    assert_eq!(value["name"].as_str(), Some("low"));
}

/// Finding 8: `--out` into a directory whose name differs from the look
/// name errors with a nonzero exit instead of writing an unvalidatable
/// look.
#[test]
fn import_out_mismatch_errors() {
    let dir = std::env::temp_dir().join(format!("scoot-look-mismatch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let scheme = dir.join("scheme.yaml");
    std::fs::write(
        &scheme,
        "name: Plain\nbase00: 090300\nbase01: 3a3432\nbase02: 4a4543\nbase03: 5c5855\nbase04: 807d7c\nbase05: a5a2a2\nbase06: d6d5d4\nbase07: f7f7f7\nbase08: db2d20\nbase09: e8bbd0\nbase0A: fded02\nbase0B: 01a252\nbase0C: b5e4f4\nbase0D: 01a0e4\nbase0E: a16a94\nbase0F: cdab53\n",
    )
    .expect("write scheme");
    let bin = env!("CARGO_BIN_EXE_scoot-look");
    let out_dir = dir.join("some-other-name");
    let _ = std::fs::remove_dir_all(&out_dir);
    let out = std::process::Command::new(bin)
        .arg("import-base16")
        .arg(&scheme)
        .arg("--out")
        .arg(&out_dir)
        .output()
        .expect("runs");
    assert!(!out.status.success(), "mismatch exits nonzero: {out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not match"),
        "names the mismatch: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
