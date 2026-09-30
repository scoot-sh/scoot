//! The appearance hardware test script's refusals: they must exit with their
//! own status and write nothing (its output directory included), so a
//! mistaken `--tty` from inside a desktop cannot touch the seat or the disk.

use std::path::PathBuf;
use std::process::Command;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/scootbar-appearance-hw-test.sh")
}

fn run(env: &[(&str, &str)], out: &std::path::Path) -> (i32, String) {
    let mut command = Command::new("bash");
    command
        .arg(script())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("XDG_RUNTIME_DIR", std::env::temp_dir())
        .env("SCOOTBAR_HW_OUT", out);
    for (k, v) in env {
        command.env(k, v);
    }
    let output = command.output().unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn fresh(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sb-hw-refuse-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn tty_inside_a_graphical_session_is_refused_with_its_own_status_and_writes_nothing() {
    for var in ["WAYLAND_DISPLAY", "DISPLAY"] {
        let out = fresh(var);
        let (code, stderr) = run(&[("SCOOTBAR_HW_MODE", "--tty"), (var, "x")], &out);
        assert_eq!(code, 3, "{stderr}");
        assert!(stderr.contains("refusing --tty"), "{stderr}");
        assert!(!out.exists(), "the refusal created {}", out.display());
    }
}

#[test]
fn a_bad_size_or_mode_is_a_setup_error_before_anything_is_written() {
    for (key, value) in [
        ("SCOOTBAR_HW_SIZE", "size"),
        ("SCOOTBAR_HW_SIZE", "10x"),
        ("SCOOTBAR_HW_SIZE", "1600"),
        ("SCOOTBAR_HW_SIZE", "-1x5"),
        ("SCOOTBAR_HW_MODE", "--x"),
    ] {
        let out = fresh("bad");
        let (code, stderr) = run(&[(key, value)], &out);
        assert_eq!(code, 2, "{key}={value}: {stderr}");
        assert!(stderr.contains(key), "{stderr}");
        assert!(!out.exists());
    }
}

#[test]
fn a_font_that_is_not_a_font_file_is_a_setup_error_before_anything_is_written() {
    let dir = std::env::temp_dir().join(format!("sb-hw-font-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = dir.join("not-a-font.txt");
    std::fs::write(&text, "x").unwrap();
    for value in [
        "/nonexistent/DejaVuSans.ttf",
        dir.to_str().unwrap(),
        text.to_str().unwrap(),
    ] {
        let out = fresh("font");
        let (code, stderr) = run(&[("SCOOTBAR_HW_FONT", value)], &out);
        assert_eq!(code, 2, "SCOOTBAR_HW_FONT={value}: {stderr}");
        assert!(stderr.contains("SCOOTBAR_HW_FONT"), "{stderr}");
        assert!(!out.exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
