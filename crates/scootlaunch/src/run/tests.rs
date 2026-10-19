//! Run mode's `$PATH` scan: precedence, bounds, and the hostile
//! cases (unreadable dirs, broken links, non-executables, duplicates).

use super::*;
use std::os::unix::fs::PermissionsExt as _;

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::TempDir::with_prefix(format!("scootlaunch-run-{tag}-")).expect("scratch makes")
}

fn write_exe(dir: &Path, name: &str) {
    let path = dir.join(name);
    std::fs::write(&path, b"#!/bin/sh\n").expect("a stub writes");
    let mut perms = std::fs::metadata(&path)
        .expect("a stub stats")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("a stub chmods");
}

#[test]
fn finds_executables_and_skips_the_rest() {
    let dir = scratch("mixed");
    write_exe(dir.path(), "alpha");
    std::fs::write(dir.path().join("plain.txt"), b"no exec bit\n").expect("a file writes");
    std::fs::create_dir(dir.path().join("subdir")).expect("a dir makes");
    let names = executable_names(&[dir.path().to_path_buf()]);
    assert_eq!(names, vec!["alpha".to_string()]);
}

#[test]
fn first_dir_wins_for_duplicates() {
    let first = scratch("first");
    let second = scratch("second");
    write_exe(first.path(), "tool");
    write_exe(second.path(), "tool");
    write_exe(second.path(), "other");
    let names = executable_names(&[first.path().to_path_buf(), second.path().to_path_buf()]);
    assert_eq!(names, vec!["tool".to_string(), "other".to_string()]);
}

#[test]
fn unreadable_dirs_and_broken_links_skip_silently() {
    let dir = scratch("hostile");
    write_exe(dir.path(), "real");
    std::os::unix::fs::symlink("no-such-target", dir.path().join("broken")).expect("a link makes");
    let names = executable_names(&[
        PathBuf::from("/nonexistent-scootlaunch-path-dir"),
        dir.path().to_path_buf(),
    ]);
    assert_eq!(names, vec!["real".to_string()]);
}

#[test]
fn symlinks_to_executables_count() {
    let dir = scratch("links");
    write_exe(dir.path(), "real");
    std::os::unix::fs::symlink("real", dir.path().join("alias")).expect("a link makes");
    let mut names = executable_names(&[dir.path().to_path_buf()]);
    names.sort();
    assert_eq!(names, vec!["alias".to_string(), "real".to_string()]);
}

#[test]
fn entries_launch_by_basename_and_key_frecency() {
    let entries = to_entries(&["htop".to_string()]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "run:htop");
    assert_eq!(entries[0].exec, vec!["htop".to_string()]);
    assert!(!entries[0].terminal && !entries[0].dbus_activatable);
}
