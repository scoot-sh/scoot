use std::os::unix::fs::PermissionsExt;

use super::ensure;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scoot-x11dir-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_missing_directory_is_created_sticky_and_world_writable() {
    let root = scratch("create");
    let dir = root.join(".X11-unix");
    ensure(&dir).expect("the directory is created");
    // 1777 survives the usual 022 umask only because `ensure` chmods.
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o1777);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_existing_directory_is_left_alone() {
    let root = scratch("existing");
    let dir = root.join(".X11-unix");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    ensure(&dir).expect("an existing directory is success");
    ensure(&dir).expect("and stays one when asked twice");
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o755, "an existing directory is not chmod-ed");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_in_the_way_is_an_error() {
    let root = scratch("file");
    let path = root.join(".X11-unix");
    std::fs::write(&path, b"").unwrap();
    assert!(ensure(&path).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_parent_is_an_error_not_a_tree() {
    let root = scratch("parent");
    assert!(ensure(&root.join("no").join(".X11-unix")).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
