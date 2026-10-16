//! `new` tests: the manifest written, and every refusal. Hermetic: the
//! parent directory is pinned to a tempdir, so no test touches the real
//! home or data dirs.

use std::path::PathBuf;

use scoot_look::Origin;
use scoot_look::catalog::discover_ordered;

use super::create_new_into;

/// A minimal valid base look.
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
        .prefix(&format!("scoot-look-new-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch dir")
}

fn write_look(dir: &std::path::Path, name: &str) -> PathBuf {
    let look = dir.join(name);
    std::fs::create_dir_all(&look).expect("a look dir");
    std::fs::write(look.join("look.toml"), look_toml(name)).expect("a look.toml");
    look
}

#[test]
fn new_writes_an_extending_manifest() {
    let scratch = scratch("manifest");
    let shipped = scratch.path().join("shipped");
    write_look(&shipped, "look-a");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let parent = scratch.path().join("user-looks");
    let dir = create_new_into("my-look", "look-a", &catalog, &parent).expect("a new look");
    assert_eq!(dir, parent.join("my-look"));
    let text = std::fs::read_to_string(dir.join("look.toml")).expect("a manifest");
    assert!(text.contains("name = \"my-look\""), "{text}");
    assert!(text.contains("extends = \"look-a\""), "{text}");
    // The manifest parses, names its directory, and resolves its base
    // through the catalog.
    let raw = scoot_look::parse_look_toml(text.as_bytes()).expect("it parses");
    assert!(
        scoot_look::check_name_matches_dir(&raw, "my-look").is_none(),
        "the name matches the directory"
    );
    let mut sources = catalog.sources();
    sources.insert(raw.name.clone(), (raw, Origin::User));
    scoot_look::resolve(&sources, "my-look").expect("the base resolves");
}

#[test]
fn new_chains_onto_a_user_look() {
    let scratch = scratch("chain");
    let shipped = scratch.path().join("shipped");
    write_look(&shipped, "look-a");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let parent = scratch.path().join("user-looks");
    create_new_into("my-look", "look-a", &catalog, &parent).expect("a first look");
    // A look made with `new` is user-authored, so it can be the next
    // `new`'s base: rebuild the catalog over the parent dir.
    let catalog = discover_ordered(&[(parent.clone(), Origin::User), (scratch.path().join("shipped"), Origin::Shipped)]);
    let dir = create_new_into("my-look-2", "my-look", &catalog, &parent).expect("a chained look");
    let text = std::fs::read_to_string(dir.join("look.toml")).expect("a manifest");
    assert!(text.contains("extends = \"my-look\""), "{text}");
}

#[test]
fn new_refuses_an_unknown_base() {
    let scratch = scratch("unknown");
    let catalog = discover_ordered(&[(scratch.path().join("empty"), Origin::Shipped)]);
    let error = create_new_into(
        "my-look",
        "nosuchlook",
        &catalog,
        &scratch.path().join("user-looks"),
    )
    .expect_err("an unknown base refuses");
    assert!(error.contains("unknown base look `nosuchlook`"), "{error}");
}

#[test]
fn new_refuses_an_installed_base() {
    let scratch = scratch("installed");
    let installed = scratch.path().join("installed");
    write_look(&installed, "theirs");
    let catalog = discover_ordered(&[(installed, Origin::Installed)]);
    let error = create_new_into(
        "my-look",
        "theirs",
        &catalog,
        &scratch.path().join("user-looks"),
    )
    .expect_err("an installed base refuses");
    assert!(
        error.contains("can be applied but never extended"),
        "{error}"
    );
}

#[test]
fn new_refuses_a_taken_name() {
    let scratch = scratch("taken");
    let shipped = scratch.path().join("shipped");
    write_look(&shipped, "look-a");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let parent = scratch.path().join("user-looks");
    for name in ["look-a", "Look-A", "not a name", ""] {
        let error = create_new_into(name, "look-a", &catalog, &parent)
            .expect_err("a taken or bad name refuses");
        assert!(
            error.contains("already a look") || error.contains("not a look name"),
            "{name}: {error}"
        );
    }
    // Nothing was written for any of them.
    assert!(
        std::fs::read_dir(&parent).is_err()
            || std::fs::read_dir(&parent).expect("readable").next().is_none(),
        "no look dir lingers"
    );
}

#[test]
fn new_refuses_an_existing_directory() {
    let scratch = scratch("exists");
    let shipped = scratch.path().join("shipped");
    write_look(&shipped, "look-a");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    let parent = scratch.path().join("user-looks");
    std::fs::create_dir_all(parent.join("my-look")).expect("a clashing dir");
    let error = create_new_into("my-look", "look-a", &catalog, &parent)
        .expect_err("an existing directory refuses");
    assert!(error.contains("already exists"), "{error}");
}
