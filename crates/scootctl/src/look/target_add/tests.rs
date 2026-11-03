//! Tests for `scoot look target add`: the foreign-pack gate, the review,
//! and the all-or-nothing add.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::look::targets::discover_in;

fn known() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("palette.background".to_owned(), "#0E0E0E".to_owned()),
        ("palette.accent".to_owned(), "#FF9A30".to_owned()),
        ("fonts.mono".to_owned(), "Mono".to_owned()),
    ])
}

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("scoot-target-add-")
        .tempdir()
        .expect("a scratch dir")
}

/// Writes a pack target: `target.toml` plus `colors.tmpl`.
fn write_target(pack: &Path, name: &str, output: &str, template: &str, extra: &str) -> PathBuf {
    let dir = pack.join(name);
    std::fs::create_dir_all(&dir).expect("a target dir");
    std::fs::write(
        dir.join("target.toml"),
        format!("name = \"{name}\"\noutput = \"{output}\"\ntemplate = \"colors.tmpl\"\n{extra}"),
    )
    .expect("a manifest");
    std::fs::write(dir.join("colors.tmpl"), template).expect("a template");
    dir
}

const TEMPLATE: &str = "bg {{ palette.background }}\nfg {{ palette.accent | nohash }}\n";

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().to_owned(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn a_pack_adds_the_exact_reviewed_bytes_and_then_discovers_and_renders() {
    let src = scratch();
    let home = scratch();
    write_target(src.path(), "alpha", "alpha/colors.conf", TEMPLATE, "");
    write_target(
        src.path(),
        "beta",
        "beta/colors.conf",
        TEMPLATE,
        "[reload]\nargv = [\"beta\", \"--reload\"]\ntimeout_secs = 5\n",
    );
    std::fs::write(src.path().join("README.md"), "docs, never copied").unwrap();
    let targets = read_pack(src.path(), home.path(), Some(&known())).expect("a clean pack");
    assert_eq!(targets.len(), 2);
    add_into(&targets, home.path()).expect("adds");
    let root = targets_root(home.path());
    assert_eq!(
        std::fs::read(root.join("alpha/colors.tmpl")).unwrap(),
        TEMPLATE.as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("beta/target.toml")).unwrap(),
        std::fs::read(src.path().join("beta/target.toml")).unwrap()
    );
    assert!(!root.join("README.md").exists());
    let defs = discover_in(home.path()).expect("the result loads");
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[1].reload.as_ref().unwrap().argv, ["beta", "--reload"]);
    let files = crate::look::targets::render_all(&defs, &fixture_look()).expect("renders");
    assert_eq!(files.len(), 2);
    // No staging directory is left beside look-targets/.
    let leftovers: Vec<_> = std::fs::read_dir(root.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["look-targets"], "{leftovers:?}");
}

fn fixture_look() -> scoot_look::ResolvedLook {
    // The shipped moonrise look, straight from the repo.
    let looks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../looks");
    let catalog = scoot_look::catalog::discover_ordered(&[(looks, scoot_look::Origin::Shipped)]);
    scoot_look::resolve(&catalog.sources(), "moonrise").expect("resolves")
}

#[test]
fn a_single_target_directory_is_a_pack_of_one() {
    let src = scratch();
    let home = scratch();
    let dir = write_target(src.path(), "solo", "solo/x.conf", TEMPLATE, "");
    let targets = read_pack(&dir, home.path(), Some(&known())).expect("reads");
    assert_eq!(targets[0].name, "solo");
}

#[test]
fn nothing_is_written_by_reading_a_pack() {
    let src = scratch();
    let home = scratch();
    write_target(src.path(), "alpha", "alpha/colors.conf", TEMPLATE, "");
    read_pack(src.path(), home.path(), Some(&known())).expect("reads");
    assert!(
        tree_bytes(home.path()).is_empty(),
        "review mode leaves the config home untouched"
    );
}

#[test]
fn an_existing_name_refuses_and_leaves_existing_targets_alone() {
    let src = scratch();
    let home = scratch();
    write_target(
        &targets_root(home.path()),
        "alpha",
        "mine/x.conf",
        "mine\n",
        "",
    );
    let before = tree_bytes(home.path());
    write_target(src.path(), "zeta", "zeta/x.conf", TEMPLATE, "");
    write_target(src.path(), "alpha", "alpha/x.conf", TEMPLATE, "");
    let targets = read_pack(src.path(), home.path(), Some(&known())).unwrap();
    let error = add_into(&targets, home.path()).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(tree_bytes(home.path()), before, "nothing changed");
}

#[test]
fn a_nix_declared_symlink_name_is_not_written_through() {
    let src = scratch();
    let home = scratch();
    let store = scratch();
    write_target(store.path(), "alpha", "mine/x.conf", "mine\n", "");
    let root = targets_root(home.path());
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(store.path().join("alpha"), root.join("alpha")).unwrap();
    write_target(src.path(), "alpha", "alpha/x.conf", TEMPLATE, "");
    let targets = read_pack(src.path(), home.path(), Some(&known())).unwrap();
    let error = add_into(&targets, home.path()).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(
        std::fs::read_to_string(store.path().join("alpha/colors.tmpl")).unwrap(),
        "mine\n"
    );
}

#[test]
fn an_output_collision_with_an_existing_target_refuses_case_insensitively() {
    let src = scratch();
    let home = scratch();
    write_target(
        &targets_root(home.path()),
        "mine",
        "Kitty/colors.conf",
        "mine\n",
        "",
    );
    write_target(src.path(), "theirs", "kitty/colors.conf", TEMPLATE, "");
    let targets = read_pack(src.path(), home.path(), Some(&known())).unwrap();
    let before = tree_bytes(home.path());
    let error = add_into(&targets, home.path()).unwrap_err();
    assert!(error.contains("already writes"), "{error}");
    assert_eq!(tree_bytes(home.path()), before);
}

#[test]
fn two_pack_targets_writing_one_output_refuse() {
    let src = scratch();
    let home = scratch();
    write_target(src.path(), "one", "x/colors.conf", TEMPLATE, "");
    write_target(src.path(), "two", "X/colors.conf", TEMPLATE, "");
    let error = read_pack(src.path(), home.path(), Some(&known())).unwrap_err();
    assert!(error.contains("same output"), "{error}");
}

#[test]
fn an_invalid_second_target_lands_nothing() {
    let src = scratch();
    let home = scratch();
    write_target(src.path(), "aaa", "a/x.conf", TEMPLATE, "");
    write_target(src.path(), "bbb", "b/x.conf", "{{ palette.backround }}", "");
    let error = read_pack(src.path(), home.path(), Some(&known())).unwrap_err();
    assert!(error.contains("unknown token"), "{error}");
    assert!(tree_bytes(home.path()).is_empty());
}

#[test]
fn an_unclosed_token_and_an_unknown_filter_refuse() {
    let src = scratch();
    let home = scratch();
    write_target(src.path(), "t", "t/x", "{{ palette.background", "");
    assert!(read_pack(src.path(), home.path(), Some(&known())).is_err());
    std::fs::remove_dir_all(src.path().join("t")).unwrap();
    write_target(src.path(), "u", "u/x", "{{ palette.accent | glow }}", "");
    assert!(read_pack(src.path(), home.path(), Some(&known())).is_err());
}

#[test]
fn symlinks_hard_links_and_extra_files_in_a_pack_refuse() {
    let home = scratch();
    // A symlinked template.
    let src = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::remove_file(dir.join("colors.tmpl")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.join("colors.tmpl")).unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("symlink"), "{error}");
    // A symlinked manifest.
    let src = scratch();
    let aside = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    let real = aside.path().join("real.toml");
    std::fs::rename(dir.join("target.toml"), &real).unwrap();
    std::os::unix::fs::symlink(&real, dir.join("target.toml")).unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("symlink"), "{error}");
    // A symlinked target directory inside the pack.
    let src = scratch();
    let elsewhere = scratch();
    write_target(elsewhere.path(), "t", "t/x", TEMPLATE, "");
    std::os::unix::fs::symlink(elsewhere.path().join("t"), src.path().join("t")).unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("not a target directory"), "{error}");
    // A hard-linked template.
    let src = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::hard_link(dir.join("colors.tmpl"), src.path().join("twin")).unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(
        error.contains("hard link") || error.contains("not a target"),
        "{error}"
    );
    // A stray file beside the template (macOS tar leaves `._*`).
    let src = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::write(dir.join("._colors.tmpl"), "junk").unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("unexpected file"), "{error}");
    // A stray file beside the target directories.
    let src = scratch();
    write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::write(src.path().join("evil.sh"), "#!/bin/sh").unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("not a target directory"), "{error}");
}

#[test]
fn a_symlinked_pack_root_and_a_look_directory_refuse() {
    let home = scratch();
    let real = scratch();
    write_target(real.path(), "t", "t/x", TEMPLATE, "");
    let holder = scratch();
    let link = holder.path().join("link");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    assert!(read_pack(&link, home.path(), None).is_err());
    std::fs::write(real.path().join("look.toml"), "name = \"x\"").unwrap();
    let error = read_pack(real.path(), home.path(), None).unwrap_err();
    assert!(error.contains("is a look"), "{error}");
}

#[test]
fn hostile_outputs_templates_and_names_refuse() {
    let home = scratch();
    for output in ["../../etc/x", "/etc/x", ".hidden/x", "a/../../x"] {
        let src = scratch();
        write_target(src.path(), "t", output, TEMPLATE, "");
        assert!(
            read_pack(src.path(), home.path(), None).is_err(),
            "{output} must refuse"
        );
    }
    // A nested template path.
    let src = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::write(
        dir.join("target.toml"),
        "name = \"t\"\noutput = \"t/x\"\ntemplate = \"sub/a.tmpl\"\n",
    )
    .unwrap();
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("plain file name"), "{error}");
    // A name that does not match its directory.
    let src = scratch();
    let dir = write_target(src.path(), "t", "t/x", TEMPLATE, "");
    std::fs::rename(&dir, src.path().join("other")).unwrap();
    assert!(read_pack(src.path(), home.path(), None).is_err());
    // A pack past the target cap.
    let src = scratch();
    for i in 0..=MAX_PACK_TARGETS {
        write_target(
            src.path(),
            &format!("t{i}"),
            &format!("o/{i}"),
            TEMPLATE,
            "",
        );
    }
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("more than"), "{error}");
    // An empty pack.
    let src = scratch();
    assert!(read_pack(src.path(), home.path(), None).is_err());
}

#[test]
fn an_output_through_a_symlinked_config_directory_refuses() {
    let src = scratch();
    let home = scratch();
    let outside = scratch();
    std::os::unix::fs::symlink(outside.path(), home.path().join("kitty")).unwrap();
    write_target(src.path(), "k", "kitty/colors.conf", TEMPLATE, "");
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("symlink"), "{error}");
}

#[test]
fn the_review_escapes_terminal_control_bytes() {
    // A hostile pack tries to overwrite the RELOAD line and hide text:
    // ESC, CR, a clear-line sequence, BEL and a bidi override, in the
    // template (the manifest grammar already refuses them in a reload).
    let src = scratch();
    let home = scratch();
    write_target(
        src.path(),
        "evil",
        "evil/x",
        "bg {{ palette.background }}\n\x1b[2K\rRELOAD:   none\u{202e}\x07\n",
        "[reload]\nargv = [\"touch\", \"a\"]\n",
    );
    let targets = read_pack(src.path(), home.path(), Some(&known())).expect("reads");
    let text = review_text(&targets[0]);
    for bad in ['\x1b', '\r', '\u{202e}', '\x07'] {
        assert!(!text.contains(bad), "{bad:?} reached the review:\n{text:?}");
    }
    assert!(text.contains("\\u{1b}[2K\\u{d}RELOAD:   none"), "{text}");
    assert!(
        text.contains("  RELOAD:   touch a (runs after every swap"),
        "{text}"
    );
    // The template can imitate the label, but only behind the `| ` rule
    // prefix; the real line is the one outside the template block.
    let real = text.lines().filter(|l| l.starts_with("  RELOAD:")).count();
    assert_eq!(real, 1, "{text}");
}

#[test]
fn a_control_byte_in_a_reload_argument_refuses_at_read() {
    let src = scratch();
    let home = scratch();
    write_target(
        src.path(),
        "evil",
        "evil/x",
        TEMPLATE,
        "[reload]\nargv = [\"touch\", \"a\\u001b[2K\\rb\"]\n",
    );
    let error = read_pack(src.path(), home.path(), None).unwrap_err();
    assert!(error.contains("control"), "{error}");
}

#[test]
fn escape_keeps_newline_tab_and_unicode_text() {
    assert_eq!(escape_for_terminal("a\tb\nc é"), "a\tb\nc é");
    assert_eq!(escape_for_terminal("\x7f\u{9b}"), "\\u{7f}\\u{9b}");
}

#[test]
fn the_in_tree_example_packs_add_as_one_pack() {
    // Dogfood: the whole starter directory is a valid pack of seven.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = root.join("docs/examples/look-targets");
    let home = scratch();
    let targets = read_pack(&src, home.path(), None).expect("the starter pack reads");
    assert_eq!(targets.len(), 7);
    add_into(&targets, home.path()).expect("adds");
    assert_eq!(discover_in(home.path()).unwrap().len(), 7);
    // Adding it again refuses on the names, changing nothing.
    let before = tree_bytes(home.path());
    assert!(add_into(&targets, home.path()).is_err());
    assert_eq!(tree_bytes(home.path()), before);
}
