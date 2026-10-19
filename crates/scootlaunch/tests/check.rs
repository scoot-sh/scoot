//! `--check`: validates the panel's config without a compositor.
//!
//! Binary-level (exit codes live in `main`), hermetic (scratch homes,
//! so no test reads the machine's files), and compositor-free: these
//! run anywhere the binary builds, needing no session.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn launch_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootlaunch"))
}

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::TempDir::with_prefix(format!("scootlaunch-check-{tag}-")).expect("scratch makes")
}

fn check(dir: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    Command::new(launch_bin())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .env("XDG_RUNTIME_DIR", dir.path())
        .env_remove("WAYLAND_DISPLAY")
        .args(args)
        .output()
        .expect("the binary runs")
}

#[test]
fn a_valid_file_checks_out() {
    let dir = scratch("valid");
    let file = dir.path().join("launch.toml");
    fs::write(&file, b"font = \"/tmp/a.ttf\"\nwidth_em = 50\nrows = 5\n").unwrap();
    let out = check(&dir, &["--check", "--config", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("valid"),
        "{out:?}"
    );
}

#[test]
fn a_bad_file_is_refused_with_status_1() {
    let dir = scratch("bad");
    let file = dir.path().join("launch.toml");
    fs::write(&file, b"rows = 0\n").unwrap();
    let out = check(&dir, &["--check", "--config", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("rows"),
        "{out:?}"
    );
}

#[test]
fn a_missing_default_is_the_defaults() {
    let dir = scratch("missing");
    let out = check(&dir, &["--check"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("built-in defaults"),
        "{out:?}"
    );
}
