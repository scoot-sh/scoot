//! Tests for data dirs and the walk: precedence, ids, symlinks,
//! loops, depth, and the bounds. Every filesystem case runs in a
//! `TempDir` that no two tests share (nextest isolates processes;
//! `cargo test` shares one, and `std::env` is process-global, so the
//! helpers here take explicit paths and never touch the environment).

use super::*;
use std::os::unix::fs::symlink;

fn env_with(data_home: &Path, data_dirs: &[&Path]) -> super::super::Env {
    super::super::Env {
        xdg_data_home: Some(data_home.to_path_buf()),
        xdg_data_dirs: data_dirs.iter().map(|path| path.to_path_buf()).collect(),
        xdg_current_desktop: Vec::new(),
        xdg_cache_home: None,
        locale_raw: String::new(),
        locale: None,
        path_dirs: Vec::new(),
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

const APP: &str = "[Desktop Entry]\nType=Application\nName=App\nExec=app\n";

#[test]
fn home_comes_first_and_forms_plain_ids() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(&home.join("applications/a.desktop"), APP);
    write(&system.join("applications/b.desktop"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[&system]));
    assert_eq!(dirs.len(), 2);
    assert_eq!(dirs[0].dir, home.join("applications"));
    let (files, _, _) = walk(&dirs[0]);
    assert_eq!(
        files
            .iter()
            .map(|found| found.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a.desktop"]
    );
}

#[test]
fn subdirectories_form_dashed_ids() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    write(&home.join("applications/foo/bar.desktop"), APP);
    write(&home.join("applications/foo-bar.desktop"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    // Both spellings form the same id; the walk lists both and the
    // index (tested in `super`) keeps the first with a note.
    let (files, _, _) = walk(&dirs[0]);
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|found| found.id == "foo-bar.desktop"));
}

#[test]
fn non_desktop_files_are_silently_skipped() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    write(&home.join("applications/mimeinfo.cache"), "[MIME Cache]\n");
    write(&home.join("applications/icon.png"), "fake");
    write(&home.join("applications/UPPER.DESKTOP"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    let (files, key, notes) = walk(&dirs[0]);
    assert!(files.is_empty());
    assert!(notes.is_empty(), "{notes:?}");
    // …but regular files still join the cache key, so a new file
    // rescans even before it matters.
    assert_eq!(
        key.iter()
            .filter(|entry| matches!(entry, KeyEntry::File { .. }))
            .count(),
        3
    );
}

#[test]
fn relative_and_missing_dirs_are_ignored() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // No `applications/` under home: no dir, no error.
    let mut env = env_with(&home, &[]);
    env.xdg_data_dirs.push(PathBuf::from("relative/path"));
    env.xdg_data_dirs
        .push(PathBuf::from("/nonexistent-scootlaunch-test"));
    let dirs = resolve_app_dirs(&env);
    assert!(dirs.is_empty());
}

#[test]
fn default_data_dirs_apply_when_none_configured() {
    // Cannot assert the machine's real dirs; assert the constant the
    // resolver would use, so a drift edits both.
    assert_eq!(DEFAULT_DATA_DIRS, &["/usr/local/share/", "/usr/share/"]);
}

#[test]
fn symlinked_files_and_dirs_are_followed() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let real = root.path().join("real");
    write(&real.join("linked.desktop"), APP);
    std::fs::create_dir_all(home.join("applications")).unwrap();
    symlink(
        real.join("linked.desktop"),
        home.join("applications/linked.desktop"),
    )
    .unwrap();
    let other = root.path().join("other");
    write(&other.join("deep.desktop"), APP);
    symlink(&other, home.join("applications/sub")).unwrap();
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    let (mut files, _, notes) = walk(&dirs[0]);
    files.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        files
            .iter()
            .map(|found| found.id.as_str())
            .collect::<Vec<_>>(),
        vec!["linked.desktop", "sub-deep.desktop"]
    );
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn symlink_loops_are_cut() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join("applications/loop")).unwrap();
    symlink(
        home.join("applications"),
        home.join("applications/loop/back"),
    )
    .unwrap();
    write(&home.join("applications/top.desktop"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    let (files, _, notes) = walk(&dirs[0]);
    assert_eq!(files.len(), 1);
    assert!(notes.iter().any(|note| note.contains("loop")), "{notes:?}");
}

#[test]
fn dangling_links_are_skipped_with_a_note() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join("applications")).unwrap();
    symlink(
        root.path().join("nowhere.desktop"),
        home.join("applications/gone.desktop"),
    )
    .unwrap();
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    let (files, _, notes) = walk(&dirs[0]);
    assert!(files.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("dangling")),
        "{notes:?}"
    );
}

#[test]
fn depth_is_capped() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let mut deep = home.join("applications");
    for level in 0..12 {
        deep = deep.join(format!("d{level}"));
    }
    write(&deep.join("deep.desktop"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    let (files, _, notes) = walk(&dirs[0]);
    assert!(files.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("deeper")),
        "{notes:?}"
    );
}

#[test]
fn canon_and_key_carry_identity() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    write(&home.join("applications/a.desktop"), APP);
    let dirs = resolve_app_dirs(&env_with(&home, &[]));
    assert_eq!(dirs.len(), 1);
    assert!(dirs[0].canonical.is_absolute());
    let (files, key, _) = walk(&dirs[0]);
    assert_eq!(files.len(), 1);
    assert!(
        key.iter()
            .any(|entry| matches!(entry, KeyEntry::Dir { .. }))
    );
    assert!(
        key.iter()
            .any(|entry| matches!(entry, KeyEntry::File { .. }))
    );
}
