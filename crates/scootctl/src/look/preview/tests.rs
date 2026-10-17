//! `preview` target tests: names, directories and the active look.
//! Hermetic: scratch catalogs and state dirs, never the environment (the
//! capture itself runs against a real session in the integration suite).

use scoot_look::Origin;
use scoot_look::catalog::discover_ordered;
use scoot_look::state::write_record;

use super::resolve_preview_target;

fn look_toml(name: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "dark"
license = "MIT"

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

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("scoot-look-preview-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch dir")
}

#[test]
fn a_catalog_name_resolves_to_its_source_dir() {
    let scratch = scratch("name");
    let shipped = scratch.path().join("shipped");
    let dir = shipped.join("look-a");
    std::fs::create_dir_all(&dir).expect("a look dir");
    std::fs::write(dir.join("look.toml"), look_toml("look-a")).expect("a look.toml");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let state = scratch.path().join("state");
    let (name, resolved, origin) =
        resolve_preview_target(Some("look-a"), Origin::User, &catalog, &state)
            .expect("a catalog name resolves");
    assert_eq!(name, "look-a");
    assert_eq!(resolved, std::fs::canonicalize(&dir).expect("canonical"));
    assert_eq!(origin, Origin::Shipped);
}

#[test]
fn a_directory_resolves_through_its_manifest() {
    let scratch = scratch("dir");
    let dir = scratch.path().join("mine");
    std::fs::create_dir_all(&dir).expect("a look dir");
    std::fs::write(dir.join("look.toml"), look_toml("mine")).expect("a look.toml");
    let catalog = discover_ordered(&[(scratch.path().join("empty"), Origin::Shipped)]);
    let state = scratch.path().join("state");
    let arg = dir.to_string_lossy().into_owned();
    let (name, resolved, _) = resolve_preview_target(Some(&arg), Origin::User, &catalog, &state)
        .expect("a directory resolves");
    assert_eq!(name, "mine");
    assert_eq!(resolved, std::fs::canonicalize(&dir).expect("canonical"));
}

#[test]
fn no_target_means_the_active_look() {
    let scratch = scratch("active");
    let shipped = scratch.path().join("shipped");
    let dir = shipped.join("look-a");
    std::fs::create_dir_all(&dir).expect("a look dir");
    std::fs::write(dir.join("look.toml"), look_toml("look-a")).expect("a look.toml");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let state = scratch.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    write_record(
        &state,
        &scoot_look::state::Record {
            name: "look-a".to_owned(),
            rendered: scratch.path().join("rendered").display().to_string(),
            generation: 1,
            chosen_by: Some("apply".to_owned()),
            declared_default: None,
            toolkit: None,
            dconf_wrote: Default::default(),
            dconf_orig: Default::default(),
        },
    )
    .expect("a record");
    let (name, resolved, _) = resolve_preview_target(None, Origin::User, &catalog, &state)
        .expect("the active look resolves");
    assert_eq!(name, "look-a");
    assert_eq!(resolved, std::fs::canonicalize(&dir).expect("canonical"));
}

#[test]
fn no_target_without_a_record_refuses() {
    let scratch = scratch("empty-state");
    let catalog = discover_ordered(&[(scratch.path().join("empty"), Origin::Shipped)]);
    let state = scratch.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    let error = resolve_preview_target(None, Origin::User, &catalog, &state)
        .expect_err("no record refuses");
    assert!(error.contains("no look applied yet"), "{error}");
}

#[test]
fn an_unknown_name_refuses() {
    let scratch = scratch("unknown");
    let catalog = discover_ordered(&[(scratch.path().join("empty"), Origin::Shipped)]);
    let state = scratch.path().join("state");
    let error = resolve_preview_target(Some("nosuchlook"), Origin::User, &catalog, &state)
        .expect_err("an unknown name refuses");
    assert!(error.contains("unknown look `nosuchlook`"), "{error}");
}

#[test]
fn a_missing_directory_refuses() {
    let scratch = scratch("missing");
    let catalog = discover_ordered(&[(scratch.path().join("empty"), Origin::Shipped)]);
    let state = scratch.path().join("state");
    let missing = scratch.path().join("nosuchdir").display().to_string();
    let error = resolve_preview_target(Some(&missing), Origin::User, &catalog, &state)
        .expect_err("a missing directory refuses");
    assert!(error.contains("not a directory"), "{error}");
}
