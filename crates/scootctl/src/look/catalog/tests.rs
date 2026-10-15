//! Catalog tests: search order, shadowing, origins, and refused entries.
//! Hermetic: `discover_ordered` over scratch dirs, never the environment.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use scoot_look::Origin;

use super::{Catalog, cache_home, config_home, data_home, discover_ordered, origin_name};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch dir unique to this test process and call.
pub(crate) fn scratch(tag: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "scoot-look-catalog-{tag}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// A minimal valid look: the moonrise palette (which clears the contrast
/// bars), a color wallpaper (no image binary needed), and a license (so it
/// validates under every origin).
pub(crate) fn look_toml(name: &str) -> String {
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

/// Writes a source look `dir/<name>/look.toml`, with `extra` appended.
pub(crate) fn write_look(dir: &Path, name: &str, extra: &str) -> PathBuf {
    let look = dir.join(name);
    std::fs::create_dir_all(&look).expect("a look dir");
    let mut text = look_toml(name);
    text.push_str(extra);
    std::fs::write(look.join("look.toml"), text).expect("a look.toml");
    look
}

#[test]
fn home_resolution_prefers_xdg_and_falls_back() {
    use std::ffi::{OsStr, OsString};
    let xdg = |v: &str| Some(OsString::from(v));
    assert_eq!(
        data_home(xdg("/xdg").as_deref(), xdg("/home").as_deref()),
        Some(PathBuf::from("/xdg"))
    );
    assert_eq!(
        data_home(xdg("").as_deref(), xdg("/home").as_deref()),
        Some(PathBuf::from("/home/.local/share"))
    );
    assert_eq!(data_home(None, None), None);
    assert_eq!(
        cache_home(xdg("/xdg").as_deref(), xdg("/home").as_deref()),
        Some(PathBuf::from("/xdg"))
    );
    assert_eq!(
        cache_home(None, xdg("/home").as_deref()),
        Some(PathBuf::from("/home/.cache"))
    );
    assert_eq!(
        config_home(xdg("").as_deref(), xdg("/home").as_deref()),
        Some(PathBuf::from("/home/.config"))
    );
    let _ = OsStr::new("");
}

#[test]
fn the_first_name_in_search_order_wins_and_the_rest_shadow() {
    let root = scratch("order");
    let first = root.join("first");
    let second = root.join("second");
    write_look(&first, "same", "");
    write_look(&second, "same", "");
    write_look(&second, "only-second", "");
    let catalog = discover_ordered(&[(first, Origin::User), (second, Origin::Shipped)]);
    let entry = catalog.find("same").expect("the name resolves");
    assert_eq!(entry.origin, Origin::User);
    assert_eq!(
        catalog
            .find("only-second")
            .expect("the other resolves")
            .origin,
        Origin::Shipped
    );
    assert_eq!(catalog.shadowed.len(), 1);
    assert_eq!(catalog.shadowed[0].0, "same");
    assert!(catalog.notes.is_empty(), "notes: {:?}", catalog.notes);
}

#[test]
fn origins_come_from_directories() {
    let root = scratch("origins");
    let user = root.join("user");
    let shipped = root.join("shipped");
    write_look(&user, "mine", "");
    write_look(&shipped, "theirs", "");
    let catalog = discover_ordered(&[(user, Origin::User), (shipped, Origin::Shipped)]);
    assert_eq!(
        catalog.find("mine").expect("user look").origin,
        Origin::User
    );
    assert_eq!(
        catalog.find("theirs").expect("shipped look").origin,
        Origin::Shipped
    );
    assert_eq!(origin_name(Origin::Declared), "declared");
    assert_eq!(origin_name(Origin::Installed), "installed");
}

#[test]
fn an_invalid_manifest_is_a_note_not_a_missing_catalog() {
    let root = scratch("invalid");
    let dir = root.join("looks");
    write_look(&dir, "good", "");
    let broken = dir.join("broken");
    std::fs::create_dir_all(&broken).expect("a look dir");
    std::fs::write(broken.join("look.toml"), "schema = ").expect("a broken look.toml");
    let mismatched = dir.join("other");
    std::fs::create_dir_all(&mismatched).expect("a look dir");
    std::fs::write(mismatched.join("look.toml"), look_toml("some-name"))
        .expect("a mismatched look.toml");
    let catalog = discover_ordered(&[(dir, Origin::User)]);
    assert!(catalog.find("good").is_some());
    assert!(catalog.find("broken").is_none());
    assert!(catalog.find("other").is_none());
    assert_eq!(catalog.notes.len(), 2, "notes: {:?}", catalog.notes);
}

#[test]
fn a_symlinked_look_uses_the_entry_name_not_the_target() {
    // Home-manager/Nix-style catalogs symlink each look into the store:
    // `user-looks/moonrise -> /store/<hash>-moonrise-look`. The name is
    // the entry's own name, never the target's basename.
    let root = scratch("symlink-look");
    let looks = root.join("looks");
    std::fs::create_dir_all(&looks).expect("a looks dir");
    let store = root.join("store");
    let target = store.join("abc123-moonrise-look");
    std::fs::create_dir_all(&target).expect("a store dir");
    std::fs::write(target.join("look.toml"), look_toml("moonrise")).expect("a manifest");
    std::os::unix::fs::symlink(&target, looks.join("moonrise")).expect("a look link");
    let catalog = discover_ordered(&[(looks, Origin::User)]);
    assert!(
        catalog.find("moonrise").is_some(),
        "notes: {:?}",
        catalog.notes
    );
}

#[test]
fn a_per_file_symlink_farm_catalogs_with_store_files() {
    // Home-manager `recursive` farms link each file into the store: the
    // look dir is real, but `look.toml` is a symlink. The manifest
    // follows it, and the entry's files resolve beside the manifest's
    // target, not the entry.
    let root = scratch("file-farm");
    let looks = root.join("looks");
    std::fs::create_dir_all(&looks).expect("a looks dir");
    let store = root.join("store").join("moonrise");
    std::fs::create_dir_all(&store).expect("a store dir");
    std::fs::write(store.join("look.toml"), look_toml("moonrise")).expect("a manifest");
    let entry = looks.join("moonrise");
    std::fs::create_dir_all(&entry).expect("a look dir");
    std::os::unix::fs::symlink(store.join("look.toml"), entry.join("look.toml"))
        .expect("a manifest link");
    let catalog = discover_ordered(&[(looks, Origin::User)]);
    let found = catalog.find("moonrise").expect("the farm look catalogs");
    let canonical_store = std::fs::canonicalize(&store).expect("a canonical store");
    let canonical_entry = std::fs::canonicalize(&entry).expect("a canonical entry");
    assert_eq!(found.manifest_dir, canonical_store);
    assert_eq!(found.dir, canonical_entry, "the entry dir stays the entry");
}

#[test]
fn a_missing_source_dir_is_skipped_quietly() {
    let catalog: Catalog =
        discover_ordered(&[(PathBuf::from("/nonexistent-scoot-looks"), Origin::User)]);
    assert!(catalog.entries.is_empty());
    assert!(catalog.notes.is_empty());
}

#[test]
fn the_same_directory_twice_scans_once() {
    let root = scratch("twice");
    let dir = root.join("looks");
    write_look(&dir, "same", "");
    let catalog = discover_ordered(&[(dir.clone(), Origin::User), (dir, Origin::Shipped)]);
    assert_eq!(catalog.entries.len(), 1);
    assert!(catalog.shadowed.is_empty());
    assert_eq!(catalog.entries[0].origin, Origin::User);
}
