use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use super::{DEFAULT_DISPLAY, PathError, display_name, resolve};

fn name(value: &str) -> Result<String, PathError> {
    display_name(Some(OsStr::new(value)))
}

#[test]
fn a_plain_name_is_kept() {
    assert_eq!(name("wayland-1").unwrap(), "wayland-1");
    assert_eq!(name("wayland-1.nested_2").unwrap(), "wayland-1.nested_2");
}

#[test]
fn unset_and_empty_mean_libwaylands_default() {
    assert_eq!(display_name(None).unwrap(), DEFAULT_DISPLAY);
    assert_eq!(name("").unwrap(), DEFAULT_DISPLAY);
}

#[test]
fn an_absolute_path_uses_its_final_component() {
    assert_eq!(name("/run/user/1000/wayland-3").unwrap(), "wayland-3");
    assert_eq!(name("/tmp/a/b/sway-ipc").unwrap(), "sway-ipc");
    assert_eq!(name("relative/dir/w").unwrap(), "w");
}

#[test]
fn a_path_ending_in_a_slash_names_nothing() {
    assert!(matches!(
        name("/run/user/1000/"),
        Err(PathError::NoDisplayName(_))
    ));
    assert!(matches!(name("/"), Err(PathError::NoDisplayName(_))));
}

#[test]
fn unsafe_bytes_are_replaced() {
    assert_eq!(name("way land*1").unwrap(), "way_land_1");
    // Non-UTF-8 and non-ASCII bytes too, one `_` per byte.
    let raw = OsStr::from_bytes(b"w\xff\xc3\xa9x");
    assert_eq!(display_name(Some(raw)).unwrap(), "w___x");
    // No component can climb out of the runtime dir: `/` never survives
    // and a bare `..` gains the `scootbg-` prefix.
    assert_eq!(name("..").unwrap(), "..");
    let paths = resolve(Some(OsStr::new("..")), Some(OsStr::new("/run/u"))).unwrap();
    assert_eq!(paths.socket, PathBuf::from("/run/u/scootbg-...sock"));
}

#[test]
fn paths_sit_in_the_runtime_dir() {
    let paths = resolve(
        Some(OsStr::new("wayland-1")),
        Some(OsStr::new("/run/user/1000")),
    )
    .unwrap();
    assert_eq!(paths.display, "wayland-1");
    assert_eq!(
        paths.socket,
        PathBuf::from("/run/user/1000/scootbg-wayland-1.sock")
    );
    assert_eq!(
        paths.lock,
        PathBuf::from("/run/user/1000/scootbg-wayland-1.lock")
    );
}

#[test]
fn the_runtime_dir_must_be_set_and_absolute() {
    assert_eq!(resolve(None, None), Err(PathError::NoRuntimeDir));
    assert_eq!(
        resolve(None, Some(OsStr::new(""))),
        Err(PathError::NoRuntimeDir)
    );
    assert!(matches!(
        resolve(None, Some(OsStr::new("run/user"))),
        Err(PathError::RelativeRuntimeDir(_))
    ));
}

#[test]
fn a_path_too_long_for_a_socket_is_refused() {
    let dir = format!("/{}", "d".repeat(80));
    let ok = "w".repeat(107 - dir.len() - "/scootbg-.sock".len());
    assert!(resolve(Some(OsStr::new(&ok)), Some(OsStr::new(&dir))).is_ok());
    let long = format!("{ok}w");
    assert!(matches!(
        resolve(Some(OsStr::new(&long)), Some(OsStr::new(&dir))),
        Err(PathError::TooLong(_))
    ));
}
