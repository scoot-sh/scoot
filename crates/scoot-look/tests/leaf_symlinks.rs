//! `scoot-look build` refuses a static leaf that is a symlink, in every
//! shape `scoot look apply` does (the same case table lives in
//! `crates/scootctl/src/look/apply/tests.rs`): one rule, enforced in both.

use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

const LOOK: &str = r##"
schema = 1
name = "look-a"
title = "Look A"
description = "Slate navy."
polarity = "dark"
license = "MIT"

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

const HELIX: &str =
    "\"ui.background\" = {}\n\"ui.text\" = \"#FF0000\"\n\"ui.selection\" = \"#FF0000\"\n";

/// A look at `<root>/look-a` with a real `apps/helix.toml`.
fn look(root: &Path) -> std::path::PathBuf {
    let look = root.join("look-a");
    std::fs::create_dir_all(look.join("apps")).expect("apps");
    std::fs::write(look.join("look.toml"), LOOK).expect("manifest");
    std::fs::write(look.join("apps/helix.toml"), HELIX).expect("leaf");
    look
}

fn build(look: &Path, out: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scoot-look"))
        .arg("build")
        .arg(look)
        .arg("--out")
        .arg(out)
        .output()
        .expect("runs")
}

#[test]
fn a_regular_leaf_builds() {
    let root = tempfile::tempdir().expect("scratch");
    let out = root.path().join("out");
    let result = build(&look(root.path()), &out);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(out.join("apps/helix.toml")).expect("copied"),
        HELIX
    );
}

#[test]
fn a_symlinked_leaf_refuses_in_every_shape() {
    for case in [
        "to a file in the look",
        "to a directory in the look",
        "to a file outside",
        "dangling",
        "a loop",
        "a symlinked apps directory",
    ] {
        let root = tempfile::tempdir().expect("scratch");
        let outside = tempfile::tempdir().expect("outside");
        let look = look(root.path());
        let leaf = look.join("apps/helix.toml");
        match case {
            "to a file in the look" => {
                std::fs::rename(&leaf, look.join("real.toml")).expect("move");
                symlink(look.join("real.toml"), &leaf).expect("link");
            }
            "to a directory in the look" => {
                std::fs::remove_file(&leaf).expect("drop");
                symlink(look.join("apps"), &leaf).expect("link");
            }
            "to a file outside" => {
                std::fs::rename(&leaf, outside.path().join("helix.toml")).expect("move");
                symlink(outside.path().join("helix.toml"), &leaf).expect("link");
            }
            "dangling" => {
                std::fs::remove_file(&leaf).expect("drop");
                symlink(look.join("missing"), &leaf).expect("link");
            }
            "a loop" => {
                std::fs::remove_file(&leaf).expect("drop");
                symlink(&leaf, &leaf).expect("link");
            }
            _ => {
                std::fs::rename(look.join("apps"), outside.path().join("apps")).expect("move");
                symlink(outside.path().join("apps"), look.join("apps")).expect("link");
            }
        }
        let out = root.path().join("out");
        let result = build(&look, &out);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success(), "{case}: {result:?}");
        assert!(stderr.contains("is a symlink"), "{case}: {stderr}");
        assert!(
            !out.join("apps/helix.toml").exists(),
            "{case}: nothing copied"
        );
    }
}

#[test]
fn a_symlinked_look_directory_still_builds() {
    let root = tempfile::tempdir().expect("scratch");
    let store = root.path().join("store");
    let real = look(&store);
    let link = root.path().join("linked");
    symlink(&real, &link).expect("link");
    let out = root.path().join("out");
    let result = build(&link, &out);
    assert!(result.status.success(), "{result:?}");
    assert!(out.join("apps/helix.toml").is_file());
}
