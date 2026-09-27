//! `[wallpaper]` from TOML to the JSON `apply-config` gets: what is read,
//! what is refused (and that a refusal costs only the wallpaper), how paths
//! resolve, and that only the keys written are sent.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use crate::compositor::config;
use crate::compositor::keybindings::{Bound, Modifiers};
use crate::compositor::wallpaper::section::{DEFAULT_COMMAND, MAX_JSON};
use crate::compositor::wallpaper::{Section, WallpaperConfig, WallpaperSetting};

/// The file around `[wallpaper]`, as `config.rs` reads it: the table is one
/// optional field among others.
#[derive(Deserialize)]
struct File {
    #[serde(default)]
    wallpaper: Option<WallpaperConfig>,
}

const CONFIG: &str = "/home/me/.config/scoot/config.toml";
const HOME: &str = "/home/me";

fn setting_at(toml: &str, config_path: &Path, home: Option<&OsStr>) -> WallpaperSetting {
    let file: File = toml::from_str(toml).expect("the TOML itself parses");
    WallpaperSetting::resolve(file.wallpaper, config_path, home)
}

fn setting(toml: &str) -> WallpaperSetting {
    setting_at(toml, Path::new(CONFIG), Some(OsStr::new(HOME)))
}

fn section(toml: &str) -> Section {
    match setting(toml) {
        WallpaperSetting::Section(section) => section,
        other => panic!("expected a usable section from {toml:?}, got {other:?}"),
    }
}

fn json(toml: &str) -> serde_json::Value {
    serde_json::from_str(&section(toml).json).expect("the JSON parses")
}

fn problem(toml: &str) -> String {
    match setting(toml) {
        WallpaperSetting::Invalid(problem) => problem,
        other => panic!("expected {toml:?} to be refused, got {other:?}"),
    }
}

#[test]
fn no_table_is_absent() {
    assert_eq!(setting(""), WallpaperSetting::Absent);
    assert_eq!(setting("[layout]\n"), WallpaperSetting::Absent);
}

#[test]
fn an_empty_table_is_the_empty_section() {
    let section = section("[wallpaper]\n");
    assert_eq!(section.json, "{}");
    assert_eq!(section.command, OsString::from(DEFAULT_COMMAND));
}

#[test]
fn every_key_reaches_the_json_and_nothing_else_does() {
    let got = json(
        r##"
[wallpaper]
image = "/srv/hills.jpg"
mode = "fit"
fill = "#101014"
filter = "nearest"
command = "/opt/bin/scootbg"

[wallpaper.output."DP-2"]
color = "#1e1e2e"

[wallpaper.output.HDMI-A-1]
image = "/srv/city.png"
mode = "tile"
fill = "#000000"
filter = "bilinear"
"##,
    );
    assert_eq!(
        got,
        json!({
            "image": "/srv/hills.jpg",
            "mode": "fit",
            "fill": "#101014",
            "filter": "nearest",
            "output": {
                "DP-2": { "color": "#1e1e2e" },
                "HDMI-A-1": {
                    "image": "/srv/city.png",
                    "mode": "tile",
                    "fill": "#000000",
                    "filter": "bilinear"
                }
            }
        }),
        "every key written, `command` left out"
    );
}

/// scootbg fingerprints the section as written: an explicit default is a
/// different section from none, so it must be sent, and an unwritten key
/// must not appear (not even as `null`, which scootbg refuses).
#[test]
fn only_the_keys_written_are_sent() {
    assert_eq!(
        section("[wallpaper]\ncolor = \"#1e1e2e\"\n").json,
        r##"{"color":"#1e1e2e"}"##
    );
    assert_eq!(
        section("[wallpaper]\nimage = \"/a.png\"\nmode = \"fill\"\n").json,
        r#"{"image":"/a.png","mode":"fill"}"#
    );
    assert_eq!(
        section("[wallpaper]\nimage = \"/a.png\"\n").json,
        r#"{"image":"/a.png"}"#
    );
    // `output` written as an empty table is part of the section as written.
    assert_eq!(
        section("[wallpaper]\noutput = {}\n").json,
        r#"{"output":{}}"#
    );
    // An empty output table is "nothing on that output", and kept.
    assert_eq!(
        section("[wallpaper.output.\"DP-2\"]\n").json,
        r#"{"output":{"DP-2":{}}}"#
    );
    // `command` alone is the empty section.
    assert_eq!(section("[wallpaper]\ncommand = \"scootbg\"\n").json, "{}");
}

/// A `HashMap` of outputs would serialize in a different order each
/// process; the JSON is the same bytes every time, whatever the file order.
#[test]
fn the_json_is_the_same_bytes_whatever_the_file_order() {
    let a = section(
        "[wallpaper.output.B]\ncolor = \"#000001\"\n[wallpaper.output.A]\ncolor = \"#000002\"\n",
    );
    let b = section(
        "[wallpaper.output.A]\ncolor = \"#000002\"\n[wallpaper.output.B]\ncolor = \"#000001\"\n",
    );
    assert_eq!(a.json, b.json);
    assert_eq!(
        a.json,
        r##"{"output":{"A":{"color":"#000002"},"B":{"color":"#000001"}}}"##
    );
}

/// Strings pass through exactly, escapes and all: a connector name or a
/// path with a quote, a backslash or a non-ASCII character arrives as it
/// was written.
#[test]
fn strings_survive_json_escaping() {
    let section = section(
        r##"
[wallpaper]
image = "/pics/say \"hi\" \\ ünïcode.png"
[wallpaper.output."we\"ird name"]
color = "#abcdef"
"##,
    );
    let got: serde_json::Value = serde_json::from_str(&section.json).unwrap();
    assert_eq!(got["image"], "/pics/say \"hi\" \\ ünïcode.png");
    assert_eq!(got["output"]["we\"ird name"]["color"], "#abcdef");
}

#[test]
fn a_home_relative_image_expands_against_home() {
    assert_eq!(
        json("[wallpaper]\nimage = \"~/Pictures/hills.jpg\"\n")["image"],
        "/home/me/Pictures/hills.jpg"
    );
    // `~` alone is HOME itself (a directory: scootbg will say so, but the
    // expansion is still the right one).
    assert_eq!(json("[wallpaper]\nimage = \"~\"\n")["image"], "/home/me");
    // Repeated slashes after the tilde collapse.
    assert_eq!(
        json("[wallpaper]\nimage = \"~//a.png\"\n")["image"],
        "/home/me/a.png"
    );
}

#[test]
fn a_relative_image_resolves_against_the_config_files_directory() {
    assert_eq!(
        json("[wallpaper]\nimage = \"hills.jpg\"\n")["image"],
        "/home/me/.config/scoot/hills.jpg"
    );
    assert_eq!(
        json("[wallpaper]\nimage = \"./walls/hills.jpg\"\n")["image"],
        "/home/me/.config/scoot/walls/hills.jpg",
        "`.` components are dropped"
    );
    // `..` is kept: resolving it lexically would be wrong across a symlink.
    assert_eq!(
        json("[wallpaper]\nimage = \"../hills.jpg\"\n")["image"],
        "/home/me/.config/scoot/../hills.jpg"
    );
    // `~user/` is not expanded (only `~` and `~/` are): it is a relative
    // path like any other.
    assert_eq!(
        json("[wallpaper]\nimage = \"~alice/a.png\"\n")["image"],
        "/home/me/.config/scoot/~alice/a.png"
    );
    // An absolute path is left alone.
    assert_eq!(
        json("[wallpaper]\nimage = \"/srv/a.png\"\n")["image"],
        "/srv/a.png"
    );
}

#[test]
fn per_output_images_resolve_the_same_way() {
    let got = json(
        "[wallpaper.output.\"DP-2\"]\nimage = \"~/a.png\"\n[wallpaper.output.\"DP-3\"]\nimage = \"b.png\"\n",
    );
    assert_eq!(got["output"]["DP-2"]["image"], "/home/me/a.png");
    assert_eq!(
        got["output"]["DP-3"]["image"],
        "/home/me/.config/scoot/b.png"
    );
}

/// `--config walls.toml` from the shell's directory: the file's directory
/// is made absolute against the working directory, so the image is still an
/// absolute path, as scootbg requires.
#[test]
fn a_relative_config_path_still_gives_an_absolute_image() {
    let cwd = std::env::current_dir().expect("a working directory");
    let setting = setting_at(
        "[wallpaper]\nimage = \"a.png\"\n",
        Path::new("walls.toml"),
        Some(OsStr::new(HOME)),
    );
    let WallpaperSetting::Section(section) = setting else {
        panic!("expected a section, got {setting:?}");
    };
    let got: serde_json::Value = serde_json::from_str(&section.json).unwrap();
    assert_eq!(
        got["image"],
        cwd.join("a.png")
            .to_str()
            .expect("a UTF-8 working directory")
    );
}

#[test]
fn a_tilde_without_home_is_refused_by_name() {
    for home in [None, Some(OsStr::new(""))] {
        let setting = setting_at(
            "[wallpaper]\nimage = \"~/a.png\"\n",
            Path::new(CONFIG),
            home,
        );
        let WallpaperSetting::Invalid(problem) = setting else {
            panic!("expected a refusal with HOME {home:?}, got {setting:?}");
        };
        assert!(problem.contains("wallpaper.image"), "{problem}");
        assert!(problem.contains("HOME"), "{problem}");
    }
    // Without a `~` in it, HOME does not matter.
    assert!(matches!(
        setting_at("[wallpaper]\nimage = \"a.png\"\n", Path::new(CONFIG), None),
        WallpaperSetting::Section(_)
    ));
}

#[test]
fn a_path_that_is_not_utf8_is_refused() {
    let home = OsStr::from_bytes(b"/home/\xffme");
    let setting = setting_at(
        "[wallpaper]\nimage = \"~/a.png\"\n",
        Path::new(CONFIG),
        Some(home),
    );
    let WallpaperSetting::Invalid(problem) = setting else {
        panic!("expected a refusal, got {setting:?}");
    };
    assert!(problem.contains("not UTF-8"), "{problem}");
}

#[test]
fn an_empty_image_is_refused_by_name() {
    assert!(problem("[wallpaper]\nimage = \"\"\n").contains("`wallpaper.image` is empty"));
    assert!(
        problem("[wallpaper.output.X]\nimage = \"\"\n")
            .contains("`wallpaper.output.X.image` is empty")
    );
}

#[test]
fn the_command_is_a_name_or_a_resolved_path() {
    let command = |toml: &str| section(toml).command;
    assert_eq!(command("[wallpaper]\n"), OsString::from("scootbg"));
    assert_eq!(
        command("[wallpaper]\ncommand = \"my-scootbg\"\n"),
        OsString::from("my-scootbg"),
        "a bare name is left for PATH"
    );
    assert_eq!(
        command("[wallpaper]\ncommand = \"/nix/store/abc-scootbg/bin/scootbg\"\n"),
        OsString::from("/nix/store/abc-scootbg/bin/scootbg")
    );
    assert_eq!(
        command("[wallpaper]\ncommand = \"~/bin/scootbg\"\n"),
        OsString::from("/home/me/bin/scootbg")
    );
    assert_eq!(
        command("[wallpaper]\ncommand = \"./bin/scootbg\"\n"),
        OsString::from("/home/me/.config/scoot/bin/scootbg"),
        "a relative path with a slash resolves like an image, never against scoot's cwd"
    );
    assert!(problem("[wallpaper]\ncommand = \"\"\n").contains("`wallpaper.command` is empty"));
    assert!(
        problem("[wallpaper]\ncommand = \"scoot\\u0000bg\"\n")
            .contains("`wallpaper.command` contains a NUL byte")
    );
}

#[test]
fn unknown_keys_are_refused_by_name() {
    let got = problem("[wallpaper]\nimgae = \"/a.png\"\n");
    assert!(got.contains("unknown key `wallpaper.imgae`"), "{got}");
    let got = problem("[wallpaper.output.\"DP-2\"]\ncommand = \"x\"\n");
    assert!(
        got.contains("unknown key `wallpaper.output.DP-2.command`"),
        "an output's table takes the five keys only: {got}"
    );
    let got = problem("[wallpaper.output.\"a b\"]\nzoom = 2\n");
    assert!(got.contains("`wallpaper.output.\"a b\".zoom`"), "{got}");
}

#[test]
fn every_problem_is_named_not_just_the_first() {
    let got = problem("[wallpaper]\nimgae = \"/a.png\"\nmode = 3\ncolour = \"#fff\"\n");
    assert!(got.contains("wallpaper.imgae"), "{got}");
    assert!(
        got.contains("`wallpaper.mode` must be a string, not a number"),
        "{got}"
    );
    assert!(got.contains("wallpaper.colour"), "{got}");
}

#[test]
fn values_of_the_wrong_type_are_refused_by_name() {
    for (toml, needle) in [
        (
            "[wallpaper]\nimage = 3\n",
            "`wallpaper.image` must be a string, not a number",
        ),
        (
            "[wallpaper]\nmode = true\n",
            "`wallpaper.mode` must be a string, not a boolean",
        ),
        (
            "[wallpaper]\nfill = 1.5\n",
            "`wallpaper.fill` must be a string, not a number",
        ),
        (
            "[wallpaper]\ncolor = [\"#fff\"]\n",
            "`wallpaper.color` must be a string, not an array",
        ),
        (
            "[wallpaper]\ncommand = { a = 1 }\n",
            "`wallpaper.command` must be a string, not a table",
        ),
        (
            "[wallpaper]\nfilter = 1979-05-27\n",
            "`wallpaper.filter` must be a string",
        ),
        (
            "[wallpaper]\noutput = \"DP-2\"\n",
            "`wallpaper.output` must be a table of tables",
        ),
        (
            "[wallpaper]\noutput = [1]\n",
            "`wallpaper.output` must be a table of tables",
        ),
        (
            "[wallpaper.output]\nDP-2 = 5\n",
            "`wallpaper.output.DP-2` must be a table, not a number",
        ),
        (
            "[wallpaper.output]\nDP-2 = \"#fff\"\n",
            "`wallpaper.output.DP-2` must be a table, not a string",
        ),
        (
            "[wallpaper.output.X]\nimage = false\n",
            "`wallpaper.output.X.image` must be a string",
        ),
        ("wallpaper = 3\n", "`wallpaper` must be a table"),
        ("wallpaper = \"x\"\n", "`wallpaper` must be a table"),
        (
            "[[wallpaper]]\nimage = \"/a.png\"\n",
            "`wallpaper` must be a table",
        ),
    ] {
        let got = problem(toml);
        assert!(got.contains(needle), "{toml:?}: {got}");
    }
}

/// Nesting under `[wallpaper]` as deep as `toml` allows is drained without
/// building anything (`IgnoredAny`), and refused, never a crash. The depth
/// is `toml`'s own limit, so the file parses; one more level and the TOML
/// itself is malformed (the whole-file case `config.rs` owns).
#[test]
fn deep_nesting_inside_the_section_is_refused_without_a_crash() {
    let depth = 70;
    let value = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let got = problem(&format!("[wallpaper]\nimage = {value}\n"));
    assert!(
        got.contains("`wallpaper.image` must be a string, not an array"),
        "{got}"
    );
    let got = problem(&format!("[wallpaper]\nnope = {value}\n"));
    assert!(got.contains("unknown key `wallpaper.nope`"), "{got}");
    // Deep tables in an output's place.
    let dotted = (0..40)
        .map(|i| format!("k{i}"))
        .collect::<Vec<_>>()
        .join(".");
    let got = problem(&format!("[wallpaper.output.X.{dotted}]\n"));
    assert!(got.contains("unknown key `wallpaper.output.X.k0`"), "{got}");
}

#[test]
fn an_oversized_section_is_refused_before_scootbg_sees_it() {
    let long = format!("/{}", "a".repeat(MAX_JSON));
    let got = problem(&format!("[wallpaper]\nimage = \"{long}\"\n"));
    assert!(got.contains(&format!("at most {MAX_JSON}")), "{got}");
    // Just under the bound is sent.
    let fits = format!("/{}", "a".repeat(MAX_JSON - r#"{"image":"/"}"#.len()));
    let section = section(&format!("[wallpaper]\nimage = \"{fits}\"\n"));
    assert_eq!(section.json.len(), MAX_JSON);
}

// -- Through the real loader --------------------------------------------------

fn write_config(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("config.toml");
    fs::write(&path, text).expect("a config file");
    path
}

/// The regression this section was held back for: every other table is
/// `deny_unknown_fields`, and an unknown table used to make startup ignore
/// the whole file. A `[wallpaper]` must leave the binds (and the rest)
/// applied.
#[test]
fn a_file_with_a_wallpaper_section_still_applies_its_binds() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "[layout]\ngap = 3\n[binds]\n\"super+n\" = \"close\"\n[wallpaper]\ncolor = \"#1e1e2e\"\n",
    );
    let loaded = config::load(Some(&path)).expect("the explicit config loads");
    assert_eq!(loaded.config.gap, 3, "the file was not discarded");
    let super_n = Modifiers {
        super_: true,
        ..Modifiers::default()
    };
    assert_eq!(
        loaded
            .keybindings
            .match_key(smithay::input::keyboard::Keysym::n, super_n),
        Some(Bound::Action(scoot_core::Action::CloseFocused)),
        "the bind from the same file applies"
    );
    let WallpaperSetting::Section(section) = &loaded.wallpaper else {
        panic!("expected a section, got {:?}", loaded.wallpaper);
    };
    assert_eq!(section.json, r##"{"color":"#1e1e2e"}"##);
}

/// A mistake inside `[wallpaper]` costs only the wallpaper: the rest of the
/// file still applies at startup.
#[test]
fn a_broken_wallpaper_section_costs_only_the_wallpaper() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "[layout]\ngap = 3\n[wallpaper]\nimgae = \"x\"\ncolor = 7\n",
    );
    let loaded = config::load(Some(&path)).expect("the explicit config loads");
    assert_eq!(loaded.config.gap, 3, "the rest of the file applied");
    let WallpaperSetting::Invalid(problem) = &loaded.wallpaper else {
        panic!("expected a refusal, got {:?}", loaded.wallpaper);
    };
    assert!(problem.contains("wallpaper.imgae"), "{problem}");
    assert!(problem.contains("wallpaper.color"), "{problem}");
}

/// The loader resolves against the real file's directory.
#[test]
fn the_loader_resolves_against_the_files_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), "[wallpaper]\nimage = \"walls/a.png\"\n");
    let loaded = config::load(Some(&path)).unwrap();
    let WallpaperSetting::Section(section) = &loaded.wallpaper else {
        panic!("expected a section, got {:?}", loaded.wallpaper);
    };
    let got: serde_json::Value = serde_json::from_str(&section.json).unwrap();
    assert_eq!(
        got["image"],
        dir.path().join("walls/a.png").to_str().unwrap()
    );
    // And a reload reads the same way.
    let reloaded = config::reload_from(&path, false).unwrap();
    assert_eq!(reloaded.wallpaper, loaded.wallpaper);
}

/// The generated starting config keeps the section commented out, header
/// included: an uncommented empty `[wallpaper]` would clear the wallpaper.
#[test]
fn the_default_config_leaves_the_section_commented_out() {
    let emitted = config::default_config_toml();
    assert!(emitted.contains("# [wallpaper]\n"), "{emitted}");
    let file: File = toml::from_str(&emitted).expect("the emission parses");
    assert!(
        file.wallpaper.is_none(),
        "no live [wallpaper] in the emission"
    );
}
