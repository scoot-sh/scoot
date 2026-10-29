use std::os::unix::fs::symlink;
use std::path::Path;

use super::locate_leaf;

fn look() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch dir");
    std::fs::create_dir(dir.path().join("apps")).expect("apps");
    dir
}

fn refusal(base: &Path, rel: &str) -> String {
    locate_leaf(base, rel).expect_err("a symlink refuses")
}

#[test]
fn a_regular_leaf_is_found_and_an_absent_one_is_none() {
    let dir = look();
    assert_eq!(locate_leaf(dir.path(), "apps/helix.toml"), Ok(None));
    std::fs::write(dir.path().join("apps/helix.toml"), "x").expect("leaf");
    assert_eq!(
        locate_leaf(dir.path(), "apps/helix.toml"),
        Ok(Some(dir.path().join("apps/helix.toml")))
    );
    // No `apps/` directory at all is absence, not an error.
    let bare = tempfile::tempdir().expect("scratch");
    assert_eq!(locate_leaf(bare.path(), "apps/helix.toml"), Ok(None));
}

#[test]
fn a_symlink_leaf_refuses_whatever_it_points_at() {
    let dir = look();
    let leaf = dir.path().join("apps/helix.toml");
    std::fs::write(dir.path().join("real.toml"), "x").expect("target");
    std::fs::create_dir(dir.path().join("realdir")).expect("dir");
    for (name, target) in [
        ("file in the look", dir.path().join("real.toml")),
        ("directory in the look", dir.path().join("realdir")),
        ("file outside the look", "/etc/hostname".into()),
        ("nothing (dangling)", dir.path().join("missing")),
        ("itself (a loop)", leaf.clone()),
    ] {
        let _ = std::fs::remove_file(&leaf);
        symlink(&target, &leaf).expect("a link");
        let error = refusal(dir.path(), "apps/helix.toml");
        assert!(error.contains("is a symlink"), "{name}: {error}");
        assert!(error.contains("apps/helix.toml"), "{name}: {error}");
    }
}

#[test]
fn a_symlinked_parent_component_refuses() {
    let dir = tempfile::tempdir().expect("scratch");
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    std::fs::write(elsewhere.path().join("helix.toml"), "x").expect("leaf");
    symlink(elsewhere.path(), dir.path().join("apps")).expect("a link");
    let error = refusal(dir.path(), "apps/helix.toml");
    assert!(error.contains("is a symlink"), "{error}");
    assert!(
        error.ends_with("apps is a symlink"),
        "names the link: {error}"
    );
}

#[test]
fn a_symlinked_look_directory_itself_is_fine() {
    // A home-manager catalog entry is a link to the store: only what the
    // look carries is checked.
    let real = look();
    std::fs::write(real.path().join("apps/helix.toml"), "x").expect("leaf");
    let holder = tempfile::tempdir().expect("scratch");
    let link = holder.path().join("linked");
    symlink(real.path(), &link).expect("a link");
    assert_eq!(
        locate_leaf(&link, "apps/helix.toml"),
        Ok(Some(link.join("apps/helix.toml")))
    );
}

#[test]
fn a_file_where_the_directory_belongs_is_absence() {
    let dir = tempfile::tempdir().expect("scratch");
    std::fs::write(dir.path().join("apps"), "not a dir").expect("file");
    assert_eq!(locate_leaf(dir.path(), "apps/helix.toml"), Ok(None));
}

#[test]
fn an_unreadable_directory_is_a_stat_error_not_absence() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = look();
    std::fs::write(dir.path().join("apps/helix.toml"), "x").expect("leaf");
    let apps = dir.path().join("apps");
    std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    // Root ignores permission bits: probe the premise before asserting.
    let enforced = std::fs::symlink_metadata(apps.join("helix.toml")).is_err();
    let result = locate_leaf(dir.path(), "apps/helix.toml");
    std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o755)).expect("restore");
    if !enforced {
        eprintln!("skipped: permission bits do not bind here (running as root)");
        return;
    }
    let error = result.expect_err("a permission failure is loud");
    assert!(error.contains("cannot stat"), "{error}");
}
