//! Tests for the built index: precedence, masking, filters and the
//! JSON shape. Filesystem cases run in a `TempDir` per test with an
//! explicit [`Env`], never the process environment.

use super::*;

fn test_env(home: &std::path::Path, system: &std::path::Path) -> Env {
    Env {
        xdg_data_home: Some(home.to_path_buf()),
        xdg_data_dirs: vec![system.to_path_buf()],
        xdg_current_desktop: vec!["scoot".to_string()],
        xdg_cache_home: None,
        xdg_state_home: None,
        locale_raw: String::new(),
        locale: None,
        path_dirs: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
    }
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn opts() -> BuildOpts {
    BuildOpts {
        use_cache: false,
        lenient_exec: false,
    }
}

const APP: &str = "[Desktop Entry]\nType=Application\nName=App\nExec=app --flag\n";

#[test]
fn first_data_dir_wins() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=Home\nExec=home\n",
    );
    write(
        &system.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=System\nExec=system\n",
    );
    let index = build(&test_env(&home, &system), &opts());
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries[0].name, "Home");
}

#[test]
fn user_hidden_masks_system_entry() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=Home\nHidden=true\n",
    );
    write(&system.join("applications/a.desktop"), APP);
    let index = build(&test_env(&home, &system), &opts());
    assert!(index.entries.is_empty());
    assert!(
        index.notes.iter().any(|note| note.contains("Hidden")),
        "{:?}",
        index.notes
    );
}

#[test]
fn system_hidden_loses_to_user_entry() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(&home.join("applications/a.desktop"), APP);
    write(
        &system.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=System\nHidden=true\n",
    );
    let index = build(&test_env(&home, &system), &opts());
    assert_eq!(index.entries.len(), 1);
    assert!(index.entries[0].hidden.is_none());
}

#[test]
fn unknown_types_are_ignored() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Link\nName=Link\nURL=https://example.com\n",
    );
    write(
        &home.join("applications/b.desktop"),
        "[Desktop Entry]\nName=NoType\nExec=x\n",
    );
    let index = build(&test_env(&home, &system), &opts());
    assert!(index.entries.is_empty());
}

#[test]
fn no_display_and_filters_hide_with_reasons() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=A\nExec=a\nNoDisplay=true\n",
    );
    write(
        &home.join("applications/b.desktop"),
        "[Desktop Entry]\nType=Application\nName=B\nExec=b\nOnlyShowIn=GNOME;\n",
    );
    write(
        &home.join("applications/c.desktop"),
        "[Desktop Entry]\nType=Application\nName=C\nExec=c\nTryExec=scootlaunch-test-no-such-bin;\n",
    );
    write(&home.join("applications/d.desktop"), APP);
    let index = build(&test_env(&home, &system), &opts());
    assert_eq!(index.entries.len(), 4);
    let reasons: Vec<_> = index
        .entries
        .iter()
        .map(|entry| entry.hidden.clone())
        .collect();
    assert_eq!(
        reasons,
        vec![
            Some(filter::HiddenReason::NoDisplay),
            Some(filter::HiddenReason::OnlyShowIn),
            Some(filter::HiddenReason::TryExec),
            None,
        ]
    );
}

#[test]
fn invalid_entries_drop_with_notes() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=A\nExec=foo %x\n",
    );
    write(
        &home.join("applications/b.desktop"),
        "[Desktop Entry]\nType=Application\nExec=noname\n",
    );
    write(
        &home.join("applications/c.desktop"),
        "[Desktop Entry]\nType=Application\nName=C\n",
    );
    let index = build(&test_env(&home, &system), &opts());
    assert!(index.entries.is_empty(), "{:?}", index.entries);
    assert_eq!(index.notes.len(), 3, "{:?}", index.notes);
}

#[test]
fn field_codes_expand_at_index_time() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(
        &home.join("applications/a.desktop"),
        "[Desktop Entry]\nType=Application\nName=My App\nIcon=myicon\nExec=run %c %k %% %f\n",
    );
    let index = build(&test_env(&home, &system), &opts());
    assert_eq!(index.entries.len(), 1);
    let entry = &index.entries[0];
    assert_eq!(
        entry.exec,
        vec![
            "run",
            "My App",
            &home.join("applications/a.desktop").to_string_lossy(),
            "%",
        ]
    );
}

#[test]
fn json_shape_matches_the_agent_schema() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(&home.join("applications/a.desktop"), APP);
    let index = build(&test_env(&home, &system), &opts());
    let json = serde_json::to_string(&JsonEntry::from_entry(&index.entries[0])).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for field in [
        "id",
        "name",
        "keywords",
        "exec",
        "terminal",
        "dbus_activatable",
        "actions",
        "path",
        "hidden",
    ] {
        assert!(value.get(field).is_some(), "missing {field} in {json}");
    }
    assert_eq!(value["id"], "a.desktop");
    assert_eq!(value["hidden"], serde_json::Value::Null);
    // Absent optionals serialize away, not as nulls.
    assert!(value.get("generic_name").is_none(), "{json}");
    assert!(value.get("comment").is_none(), "{json}");
    assert!(value.get("icon").is_none(), "{json}");
}

#[test]
fn entries_sort_by_id() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    write(&home.join("applications/z.desktop"), APP);
    write(&home.join("applications/a.desktop"), APP);
    write(&home.join("applications/sub/m.desktop"), APP);
    let index = build(&test_env(&home, &system), &opts());
    let ids: Vec<_> = index
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a.desktop", "sub-m.desktop", "z.desktop"]);
}

#[test]
fn oversize_files_skip_with_a_note() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let system = root.path().join("system");
    let mut big = String::from("[Desktop Entry]\nType=Application\nName=Big\nExec=big\nComment=");
    big.push_str(&"x".repeat(MAX_FILE));
    write(&home.join("applications/big.desktop"), &big);
    let index = build(&test_env(&home, &system), &opts());
    assert!(index.entries.is_empty());
    assert!(
        index.notes.iter().any(|note| note.contains("larger than")),
        "{:?}",
        index.notes
    );
}

#[test]
fn missing_app_dirs_build_empty() {
    let env = Env {
        xdg_data_home: Some(std::path::PathBuf::from("/nonexistent-scootlaunch-home")),
        xdg_data_dirs: vec![std::path::PathBuf::from("/nonexistent-scootlaunch-system")],
        xdg_current_desktop: Vec::new(),
        xdg_cache_home: None,
        xdg_state_home: None,
        locale_raw: String::new(),
        locale: None,
        path_dirs: Vec::new(),
    };
    let index = build(&env, &opts());
    assert!(index.entries.is_empty());
    assert!(index.notes.is_empty());
}
