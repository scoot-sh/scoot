//! The non-Nix path: listing, validating and rendering a look uses plain
//! files and this crate only. The full pipeline must succeed with no `nix`
//! helper involved, so this test runs it with `PATH` pointed at an empty
//! directory: anything the pipeline shells out to (or looks up on `PATH`)
//! would fail, and success proves the render is plain files plus this crate.
//! Purity is also structural: the library links no process, environment or
//! network API (no `std::process`, no `std::env`, no sockets in `src/`,
//! outside this test).

use std::collections::BTreeMap;

use scoot_look::{Origin, RenderInputs, check_contrast, render_bar, render_scoot, resolve};

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
