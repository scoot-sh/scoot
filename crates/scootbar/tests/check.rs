//! `scootbar daemon --check`: validates the config (and the flags over it)
//! and the font, with no compositor and no control socket, giving the
//! errors a real start gives. Needs nothing from the machine: no `scoot`,
//! no Wayland display, and a font of its own, so it never skips.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::Scratch;

/// `scootbar daemon ARGS` with nothing in its environment that could name a
/// compositor, a config home or a font, and an empty runtime directory.
fn run(scratch: &Scratch, args: &[&str]) -> Output {
    let run = scratch.0.join("run");
    std::fs::create_dir_all(&run).unwrap();
    Command::new(common::scootbar_bin())
        .arg("daemon")
        .args(args)
        .env_clear()
        .env("XDG_RUNTIME_DIR", &run)
        .env("XDG_CONFIG_HOME", scratch.0.join("config"))
        .env("HOME", &scratch.0)
        .output()
        .unwrap()
}

fn write(scratch: &Scratch, name: &str, text: &str) -> std::path::PathBuf {
    let path = scratch.0.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn toml_with_font(scratch: &Scratch) -> String {
    format!("[bar]\nfont = {:?}\n", scratch.font())
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn left_alone(dir: &Path) -> bool {
    std::fs::read_dir(dir.join("run")).unwrap().next().is_none()
}

#[test]
fn a_good_file_is_ok_and_leaves_nothing_behind() {
    let scratch = Scratch::new("ck-good");
    let file = write(&scratch, "good.toml", &toml_with_font(&scratch));
    let out = run(&scratch, &["--check", "--config", file.to_str().unwrap()]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ok\n");
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
    // No control socket, no lock: it never got as far as claiming one.
    assert!(left_alone(&scratch.0));
}

#[test]
fn the_flag_may_come_anywhere_among_the_others() {
    let scratch = Scratch::new("ck-order");
    let file = write(&scratch, "good.toml", &toml_with_font(&scratch));
    let out = run(
        &scratch,
        &[
            "--height",
            "40",
            "--config",
            file.to_str().unwrap(),
            "--check",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn a_bad_file_is_refused_with_the_error_a_start_gives() {
    let scratch = Scratch::new("ck-bad");
    let file = write(&scratch, "bad.toml", "[colors]\nbackgroun = \"#101010\"\n");
    let path = file.to_str().unwrap();
    let check = run(&scratch, &["--check", "--config", path]);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    assert!(stderr(&check).contains("backgroun"), "{}", stderr(&check));
    assert!(check.stdout.is_empty());
    // The daemon, with no compositor to reach, stops at the same file with
    // the same words: that is what "the same errors" means.
    let start = run(&scratch, &["--config", path]);
    assert_eq!(start.status.code(), Some(1));
    assert_eq!(stderr(&check), stderr(&start));
    assert!(left_alone(&scratch.0));
}

#[test]
fn a_missing_file_is_a_refusal_naming_it() {
    let scratch = Scratch::new("ck-missing");
    let path = scratch.0.join("nope.toml");
    let out = run(&scratch, &["--check", "--config", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("nope.toml"), "{}", stderr(&out));
    assert!(out.stdout.is_empty());
}

// A build with no module draws no text and so needs no font.
#[cfg(feature = "clock")]
#[test]
fn a_font_that_cannot_load_is_refused_as_a_start_refuses_it() {
    let scratch = Scratch::new("ck-font");
    let file = write(
        &scratch,
        "font.toml",
        "[bar]\nfont = \"/nonexistent/no-such.ttf\"\n",
    );
    let path = file.to_str().unwrap();
    let check = run(&scratch, &["--check", "--config", path]);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    assert!(stderr(&check).contains("no-such.ttf"), "{}", stderr(&check));
    assert_eq!(stderr(&check), stderr(&run(&scratch, &["--config", path])));
}

#[cfg(feature = "clock")]
#[test]
fn the_flags_are_applied_over_the_file_before_it_judges() {
    let scratch = Scratch::new("ck-flags");
    // The file's font is unusable; the flag's, which wins, is fine.
    let file = write(&scratch, "f.toml", "[bar]\nfont = \"/nonexistent/x.ttf\"\n");
    let font = scratch.font();
    let out = run(
        &scratch,
        &[
            "--check",
            "--config",
            file.to_str().unwrap(),
            "--font",
            font.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn a_bar_with_no_module_needs_no_font() {
    let scratch = Scratch::new("ck-nomod");
    let file = write(
        &scratch,
        "empty.toml",
        "left = []\ncenter = []\nright = []\n",
    );
    let out = run(&scratch, &["--check", "--config", file.to_str().unwrap()]);
    assert!(out.status.success(), "{}", stderr(&out));
}
