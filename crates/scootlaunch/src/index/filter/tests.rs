//! Tests for show/hide: the desktop-order rule and `TryExec`.

use super::*;
use crate::index::parse::parse;

fn parse_group(text: &str) -> crate::index::parse::Group {
    parse(text.as_bytes())
        .unwrap()
        .groups
        .into_iter()
        .next()
        .unwrap()
}

fn desktops(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn no_lists_means_shown() {
    let group = parse_group("[Desktop Entry]\nName=Foo\n");
    assert_eq!(show_in_hidden(&group, &desktops(&["scoot"])), None);
    assert_eq!(show_in_hidden(&group, &[]), None);
}

#[test]
fn only_show_in_names_scoot_shows() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nOnlyShowIn=scoot;GNOME;\n");
    assert_eq!(show_in_hidden(&group, &desktops(&["scoot"])), None);
    assert_eq!(show_in_hidden(&group, &desktops(&["GNOME"])), None);
}

#[test]
fn only_show_in_without_us_hides() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nOnlyShowIn=GNOME;\n");
    assert_eq!(
        show_in_hidden(&group, &desktops(&["scoot"])),
        Some(HiddenReason::OnlyShowIn)
    );
    assert_eq!(
        show_in_hidden(&group, &[]),
        Some(HiddenReason::OnlyShowIn),
        "no XDG_CURRENT_DESKTOP with a non-empty OnlyShowIn hides"
    );
}

#[test]
fn not_show_in_names_us_hides() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nNotShowIn=scoot;\n");
    assert_eq!(
        show_in_hidden(&group, &desktops(&["scoot"])),
        Some(HiddenReason::NotShowIn)
    );
    assert_eq!(show_in_hidden(&group, &desktops(&["GNOME"])), None);
}

#[test]
fn first_desktop_in_env_order_decides() {
    // `scoot` names us in NotShowIn, `GNOME` in OnlyShowIn. With
    // `scoot:GNOME`, scoot's refusal comes first and hides; reversed,
    // GNOME's invitation comes first and shows.
    let group = parse_group("[Desktop Entry]\nName=Foo\nOnlyShowIn=GNOME;\nNotShowIn=scoot;\n");
    assert_eq!(
        show_in_hidden(&group, &desktops(&["scoot", "GNOME"])),
        Some(HiddenReason::NotShowIn)
    );
    assert_eq!(show_in_hidden(&group, &desktops(&["GNOME", "scoot"])), None);
}

#[test]
fn unparseable_lists_are_ignored_not_hiding() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nOnlyShowIn=a\\qb;\n");
    assert_eq!(show_in_hidden(&group, &desktops(&["scoot"])), None);
    let group = parse_group("[Desktop Entry]\nName=Foo\nNotShowIn=a\\qb;\n");
    assert_eq!(show_in_hidden(&group, &desktops(&["scoot"])), None);
}

#[test]
fn try_exec_missing_hides_and_present_shows() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nTryExec=scootlaunch-test-no-such-bin\n");
    assert_eq!(try_exec_hidden(&group, &[]), Some(HiddenReason::TryExec));
    // `sh` is on every PATH the tests run under; the dir holding it
    // comes from the test's own PATH lookup, not a fixed prefix.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    assert!(
        !dirs.is_empty(),
        "tests need a PATH to test TryExec against"
    );
    let group = parse_group("[Desktop Entry]\nName=Foo\nTryExec=sh\n");
    assert_eq!(try_exec_hidden(&group, &dirs), None);
}

#[test]
fn try_exec_absolute_path_and_non_executable() {
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("plain");
    std::fs::write(&plain, "x").unwrap();
    let group_text = format!("[Desktop Entry]\nName=Foo\nTryExec={}\n", plain.display());
    let group = parse_group(&group_text);
    assert_eq!(try_exec_hidden(&group, &[]), Some(HiddenReason::TryExec));
    // A directory is never executable-as-a-program.
    let group_text = format!(
        "[Desktop Entry]\nName=Foo\nTryExec={}\n",
        root.path().display()
    );
    let group = parse_group(&group_text);
    assert_eq!(try_exec_hidden(&group, &[]), Some(HiddenReason::TryExec));
}

#[test]
fn try_exec_empty_is_absent() {
    let group = parse_group("[Desktop Entry]\nName=Foo\nTryExec=\n");
    assert_eq!(try_exec_hidden(&group, &[]), None);
    let group = parse_group("[Desktop Entry]\nName=Foo\n");
    assert_eq!(try_exec_hidden(&group, &[]), None);
}

#[test]
fn try_exec_searches_path_in_order() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let bin = second.join("mybin");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let group = parse_group("[Desktop Entry]\nName=Foo\nTryExec=mybin\n");
    assert_eq!(
        try_exec_hidden(&group, std::slice::from_ref(&first)),
        Some(HiddenReason::TryExec)
    );
    assert_eq!(try_exec_hidden(&group, &[first, second]), None);
}
