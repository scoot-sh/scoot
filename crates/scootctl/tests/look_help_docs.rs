//! The looks help/docs drift gate: every `scoot look` command and flag in
//! the parser must appear in `--help` and in the docs, and every docs
//! snippet shape must keep working.
//!
//! Three pins, each independent of the implementation (hardcoded tables,
//! real parsing, real runs), so removing a flag from the help, adding one
//! to the parser without documenting it, or breaking a docs example fails
//! loudly:
//!
//! - `help_names_every_command_and_flag`: the spec table below parses and
//!   is covered by `scoot look --help`.
//! - `usage_flags_all_appear_in_the_table`: every `--flag` the help text
//!   names is in the spec table (an addition to the help without docs
//!   cover fails).
//! - `docs_name_every_command_and_flag`: the looks page names every spec
//!   verb and flag, and every `scoot look <verb>` in the docs parses
//!   (a docs example for a removed command fails).
//! - `docs_snippets_run_against_a_scratch_state`: the docs' command shapes
//!   (list, show, apply, cycle, current, validate, build, sync, toolkit,
//!   new, export, install, wiring, preview, save) run green in a re-exec
//!   child under a scratch HOME/XDG tree, a stub `fc-match`, a neutralized
//!   settings bus and a DejaVu-only `FONTCONFIG_FILE`. Shapes the sandbox
//!   cannot run are excluded by name in the child: `watch` (blocks on save
//!   events), `preview --capture` (needs a Linux compositor beside the
//!   test), `apply --for` (spawns a detached reverter) and `pick` (needs a
//!   Wayland display).
//!
//! The child never touches the real HOME: every XDG dir, TMPDIR and the
//! fontconfig file live under one temp root, following the re-exec guard
//! pattern (`SCOOT_LOOK_GUARD`) from the apply-transaction tests.

use std::path::{Path, PathBuf};

use scootctl::look::{COMMANDS, parse_look, usage};

/// One spec row: the verb, an example argv that must parse, and every
/// flag the verb takes (with a value where the flag takes one).
/// This table is the independent spec: it is handwritten, never derived
/// from the parser or the help text.
type SpecRow = (
    &'static str,
    Vec<&'static str>,
    Vec<(&'static str, Option<&'static str>)>,
);

fn spec() -> Vec<SpecRow> {
    let globals = |extra: Vec<(&'static str, Option<&'static str>)>| {
        let mut flags: Vec<(&'static str, Option<&'static str>)> = vec![
            ("--json", None),
            ("--state", Some("/tmp/state")),
            ("--socket", Some("/tmp/scoot.sock")),
        ];
        flags.extend(extra);
        flags
    };
    vec![
        ("list", vec!["list"], globals(vec![("--names", None)])),
        (
            "list",
            vec!["list", "--names"],
            globals(vec![("--names", None)]),
        ),
        ("current", vec!["current"], globals(vec![])),
        ("show", vec!["show", "moonrise"], globals(vec![])),
        (
            "validate",
            vec!["validate", "./my-look"],
            globals(vec![("--origin", Some("user")), ("--trust", None)]),
        ),
        (
            "build",
            vec!["build", "./my-look", "--out", "/tmp/out"],
            globals(vec![
                ("--out", Some("/tmp/out")),
                ("--origin", Some("user")),
                ("--trust", None),
            ]),
        ),
        (
            "apply",
            vec!["apply", "moonrise"],
            globals(vec![
                ("--for", Some("20")),
                ("--keep", None),
                ("--no-toolkit", None),
                ("--trust", None),
                ("--no-helix", None),
                ("--no-btop", None),
                ("--from-compositor", None),
                ("--only-if-generation", Some("7")),
            ]),
        ),
        ("next", vec!["next"], globals(vec![])),
        ("prev", vec!["prev"], globals(vec![])),
        ("random", vec!["random"], globals(vec![])),
        ("pick", vec!["pick"], globals(vec![])),
        ("sync", vec!["sync"], globals(vec![("--no-toolkit", None)])),
        ("toolkit", vec!["toolkit"], globals(vec![])),
        ("toolkit", vec!["toolkit", "off"], globals(vec![])),
        (
            "new",
            vec!["new", "mine", "--from", "moonrise"],
            globals(vec![("--from", Some("moonrise"))]),
        ),
        (
            "watch",
            vec!["watch", "./my-look"],
            globals(vec![("--no-toolkit", None), ("--origin", Some("user"))]),
        ),
        (
            "preview",
            vec!["preview", "moonrise"],
            globals(vec![
                ("--capture", None),
                ("--out", Some("/tmp/shot.png")),
                ("--width", Some("800")),
                ("--height", Some("600")),
                ("--timeout", Some("30")),
                ("--origin", Some("user")),
            ]),
        ),
        (
            "install",
            vec!["install", "./handed-over"],
            globals(vec![("--trust", None)]),
        ),
        (
            "export",
            vec!["export", "moonrise"],
            globals(vec![("--out", Some("/tmp/moonrise.tar"))]),
        ),
        ("save", vec!["save", "my-tweak"], globals(vec![])),
        ("wiring", vec!["wiring"], globals(vec![])),
    ]
}

/// The `LOOK_USAGE` line spelling one verb (`next|prev|random` share
/// theirs). Flag cover is checked against this line, not the whole
/// page, so a flag documented under the wrong command fails.
fn usage_line_for<'a>(usage: &'a str, verb: &str) -> &'a str {
    usage
        .lines()
        .find(|line| {
            line.split_whitespace().any(|word| {
                word == verb
                    || word
                        .split('|')
                        .any(|alt| alt.trim_matches(|c| c == '[' || c == ']') == verb)
            })
        })
        .unwrap_or_else(|| panic!("no usage line for `{verb}`"))
}

#[test]
fn help_names_every_command_and_flag() {
    let text = usage();
    for verb in COMMANDS {
        assert!(text.contains(verb), "--help hides `{verb}`");
    }
    // No `_` names leak: the hidden revert child is spawned, never typed.
    assert!(
        !text.contains("_preview-revert"),
        "--help leaks the hidden child"
    );
    for (verb, argv, flags) in spec() {
        let parsed = parse_look(argv.iter().map(|arg| (*arg).to_owned()));
        assert!(
            parsed.is_ok(),
            "`scoot look {}` refuses: {:?}",
            argv.join(" "),
            parsed.err()
        );
        // The example parses; now every flag of the verb parses too, on
        // the verb's own argv. A flag the parser dropped fails here, not
        // in a user's shell.
        for (flag, value) in &flags {
            if argv.contains(flag) {
                continue;
            }
            let mut full: Vec<String> = argv.iter().map(|arg| (*arg).to_owned()).collect();
            full.push((*flag).to_owned());
            if let Some(value) = value {
                full.push((*value).to_owned());
            }
            let parsed = parse_look(full.clone());
            assert!(
                parsed.is_ok(),
                "`scoot look {}` refuses: {:?}",
                full.join(" "),
                parsed.err()
            );
        }
        // And the verb's own usage line names each flag: a flag filed
        // under the wrong command fails here. The globals ride the
        // shared paragraph below the lines, so they check once.
        let line = usage_line_for(&text, verb);
        for (flag, _) in &flags {
            if matches!(*flag, "--json" | "--state" | "--socket") {
                assert!(text.contains(flag), "--help hides `{flag}`");
            } else {
                assert!(line.contains(flag), "`{verb}` hides `{flag}`");
            }
        }
    }
}

#[test]
fn usage_flags_all_appear_in_the_table() {
    // Every `--flag` the help text names must be covered by the spec
    // table above: a help addition without spec (and docs) cover fails
    // here. The `--help`/`-h` spellings and `scoot look` itself are not
    // flags.
    let text = usage();
    let mut tabled: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, _, flags) in spec() {
        tabled.extend(flags.iter().map(|(flag, _)| (*flag).to_owned()));
    }
    // Measurement words, not flags.
    tabled.insert("--look".to_owned());
    let mut unseen = Vec::new();
    for token in
        text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == '/')
    {
        let token = token.trim_matches(|c| {
            c == '.' || c == ';' || c == ':' || c == '`' || c == '\'' || c == '[' || c == ']'
        });
        if token.starts_with("--") && token.len() > 2 && token != "--help" {
            let flag = token.trim_end_matches([']', '=']);
            if !tabled.contains(flag) {
                unseen.push(flag.to_owned());
            }
        }
    }
    // `--json` rides every command; the table rows carry it already, so it
    // needs no special case beyond the filter above.
    assert!(
        unseen.is_empty(),
        "help names flags the spec table misses: {unseen:?}"
    );
}

fn docs_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../site/src/content/docs/looks/index.md")
}

#[test]
fn docs_name_every_command_and_flag() {
    let docs = std::fs::read_to_string(docs_path()).expect("the looks page");
    for verb in COMMANDS {
        assert!(
            docs.contains(&format!("look {verb}")),
            "docs never say `look {verb}`"
        );
    }
    let mut tabled: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, _, flags) in spec() {
        tabled.extend(flags.iter().map(|(flag, _)| (*flag).to_owned()));
    }
    for flag in &tabled {
        assert!(docs.contains(flag.as_str()), "docs never say `{flag}`");
    }
    // Every `scoot look <verb>` spelled in the docs must still parse: a
    // docs example for a removed or renamed command fails here.
    for (line_no, line) in docs.lines().enumerate() {
        for found in find_look_verbs(line) {
            if found == "look" {
                continue;
            }
            assert!(
                COMMANDS.contains(&found.as_str()),
                "docs line {} names `scoot look {found}`, which parses no more",
                line_no + 1
            );
        }
    }
}

/// The verbs in `scoot look <verb>` spellings on one docs line.
fn find_look_verbs(line: &str) -> Vec<String> {
    let mut verbs = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("scoot look") {
        rest = &rest[at + "scoot look".len()..];
        let word: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !word.is_empty() && !word.starts_with('-') {
            verbs.push(word);
        }
    }
    verbs
}

// --- The runnable snippet gate (re-exec child under a sandbox) ---

/// A scratch look the child applies, validates, builds, previews,
/// exports and installs: DejaVu families only (the stub `fc-match`
/// answers them), a color wallpaper (no image binary needed), and a
/// license (so the export/install roundtrip passes the gate).
fn test_look_toml(name: &str) -> String {
    format!(
        r##"schema = 1
name = "{name}"
title = "Test {name}"
description = "A drift-gate look."
polarity = "dark"
license = "MIT"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DejaVu Sans"
sans = "DejaVu Sans"
mono = "DejaVu Sans Mono"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"

[wallpaper]
color = "#2B3648"
"##
    )
}

fn repo_looks() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../looks")
}

/// A real, loadable font file for the stub `fc-match`: first the host's
/// own answer for DejaVu Sans, then a scan of the usual font roots.
/// Whatever the stub answers must exist: the render cache keys it.
fn real_font_file() -> PathBuf {
    if let Ok(out) = std::process::Command::new("fc-match")
        .arg("--format=%{file}\n")
        .arg("DejaVu Sans")
        .output()
    {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !path.is_empty() && Path::new(&path).is_file() {
            return PathBuf::from(path);
        }
    }
    let mut roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".fonts"));
        roots.push(home.join(".local/share/fonts"));
        roots.push(home.join("Library/Fonts"));
    }
    let mut stack = roots;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "ttf" || ext == "otf" || ext == "ttc")
            {
                return path;
            }
        }
    }
    panic!("no font file anywhere: cannot stub fc-match");
}

#[test]
fn docs_snippets_run_against_a_scratch_state() {
    if std::env::var_os("SCOOT_LOOK_HELP_DOCS_CHILD").is_some() {
        return child_main();
    }
    let root = tempfile::Builder::new()
        .prefix(&format!("scoot-look-docs-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch root");
    let font = real_font_file();
    let exe = std::env::current_exe().expect("the test binary");
    // Run only this test in the child: the re-exec guard inside it
    // takes the snippet branch instead of running the suite again.
    let out = std::process::Command::new(exe)
        .arg("docs_snippets_run_against_a_scratch_state")
        .arg("--exact")
        .arg("--nocapture")
        .env("SCOOT_LOOK_HELP_DOCS_CHILD", "1")
        .env("SCOOT_LOOK_HELP_DOCS_ROOT", root.path())
        .env("SCOOT_LOOK_HELP_DOCS_FONT", &font)
        .env("SCOOT_LOOKS_DIR", repo_looks())
        .output()
        .expect("the child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "docs snippets failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("~/") && !stderr.contains("~/"),
        "a snippet printed a `~` path (unusable under another home):\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The child: a scratch HOME/XDG tree, a stub `fc-match`, a neutralized
/// settings bus and a DejaVu-only fontconfig file, then every docs
/// command shape in order.
fn child_main() {
    let root = PathBuf::from(std::env::var_os("SCOOT_LOOK_HELP_DOCS_ROOT").expect("a root"));
    let font = PathBuf::from(std::env::var_os("SCOOT_LOOK_HELP_DOCS_FONT").expect("a font"));
    let home = root.join("home");
    let run = root.join("run");
    let tmp = root.join("tmp");
    let bin = root.join("bin");
    for dir in [&home, &run, &tmp, &bin] {
        std::fs::create_dir_all(dir).expect("a scratch dir");
    }
    // The stub answers the test look's DejaVu families with the real
    // file and their own names, and every other family the way real
    // `fc-match` answers a missing one (a fallback file with a different
    // family), so the family comparison still refuses unknown fonts.
    std::fs::write(
        bin.join("fc-match"),
        format!(
            "#!/bin/sh\nfmt=\"$1\"\nfam=\"$2\"\ncase \"$fam\" in\n  \"DejaVu Sans\"|\"DejaVu Sans Mono\")\n    case \"$fmt\" in\n      *file*) printf '%s\\n' \"{font}\" ;;\n      *) printf '%s\\n' \"$fam\" ;;\n    esac\n    ;;\n  *)\n    case \"$fmt\" in\n      *file*) printf '%s\\n' \"{font}\" ;;\n      *) printf 'DejaVu Sans\\n' ;;\n    esac\n    ;;\nesac\n",
            font = font.display(),
        ),
    )
    .expect("a stub fc-match");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(bin.join("fc-match"))
            .expect("stub metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(bin.join("fc-match"), perms).expect("stub perms");
    }
    std::fs::write(
        root.join("fonts.conf"),
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>\n  <selectfont>\n    <acceptfont>\n      <pattern><patelt name=\"family\"><string>DejaVu Sans</string></patelt></pattern>\n    </acceptfont>\n  </selectfont>\n</fontconfig>\n",
    )
    .expect("a DejaVu-only fonts.conf");
    // SAFETY: the child is the only test in its process (re-exec), so no
    // other test can observe the sandbox environment.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
        std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
        std::env::set_var("XDG_RUNTIME_DIR", &run);
        std::env::set_var("TMPDIR", &tmp);
        std::env::set_var("FONTCONFIG_FILE", root.join("fonts.conf"));
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "disabled:");
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("SCOOT_SOCKET");
        std::env::remove_var("SCOOT_LOOK_STATE");
        std::env::remove_var("STARSHIP_CONFIG");
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var_os("PATH")
                .unwrap_or_default()
                .to_string_lossy()
        );
        std::env::set_var("PATH", path);
    }
    assert_no_leak();
    // The scratch look under test, plus user config files for `save`.
    // The catalog holds exactly one look: the scratch look as a user
    // look, shadowed by a shipped copy of itself. Cycle steps then stay
    // on it (a one-entry catalog returns its only entry), so `next`,
    // `prev` and `random` run green without host fonts.
    let user_looks = home.join(".local/share/scoot/user-looks/snippet-look");
    std::fs::create_dir_all(&user_looks).expect("a user look dir");
    std::fs::write(user_looks.join("look.toml"), test_look_toml("snippet-look"))
        .expect("a test look");
    let shipped = root.join("shipped/snippet-look");
    std::fs::create_dir_all(&shipped).expect("a shipped dir");
    std::fs::write(shipped.join("look.toml"), test_look_toml("snippet-look"))
        .expect("a shipped copy");
    unsafe {
        std::env::set_var("SCOOT_LOOKS_DIR", root.join("shipped"));
    }
    std::fs::create_dir_all(home.join(".config/scoot")).expect("a config tree");
    std::fs::write(
        home.join(".config/scoot/config.toml"),
        "[appearance]\nbackground_color = \"#2B3648\"\nfocus_ring_active_color = \"#FF9A49\"\n[wallpaper]\ncolor = \"#2B3648\"\n",
    )
    .expect("a compositor file");
    std::fs::write(
        home.join(".config/scoot/bar.toml"),
        "[colors]\nbackground = \"#2B3648\"\nforeground = \"#F6EEDC\"\n",
    )
    .expect("a bar file");
    let out_dir = root.join("built");
    let tarball = root.join("snippet-look.tar");
    // Every docs command shape, in docs order. Each must exit Ok: the
    // reply text is the help/docs contract, asserted above, not here.
    run_ok(&["list"]);
    run_ok(&["list", "--names"]);
    run_ok(&["show", "snippet-look"]);
    run_ok(&["apply", "snippet-look"]);
    run_ok(&["current"]);
    run_ok(&["wiring"]);
    run_ok(&["next"]);
    run_ok(&["prev"]);
    run_ok(&["random"]);
    // `apply --for` is excluded by design (it spawns a detached
    // reverter): bare `--keep` with no pending preview refuses.
    run_err(&["apply", "--keep"]);
    run_ok(&["validate", "snippet-look"]);
    run_ok(&["validate", user_looks.to_str().expect("UTF-8")]);
    run_ok(&[
        "build",
        "snippet-look",
        "--out",
        out_dir.to_str().expect("UTF-8"),
    ]);
    run_ok(&["sync"]);
    run_ok(&["toolkit"]);
    run_ok(&["toolkit", "off"]);
    run_ok(&["toolkit", "on"]);
    run_ok(&["new", "snippet-child", "--from", "snippet-look"]);
    run_ok(&["show", "snippet-child"]);
    run_ok(&["save", "snippet-saved"]);
    run_ok(&["show", "snippet-saved"]);
    run_ok(&[
        "export",
        "snippet-look",
        "--out",
        tarball.to_str().expect("UTF-8"),
    ]);
    run_ok(&["preview", "snippet-look"]);
    // The exported tarball installs back under its own name, so both
    // same-named sources move aside first: an installed look may not
    // take a user or shipped look's name.
    std::fs::remove_dir_all(&user_looks).expect("the user look moves aside");
    std::fs::remove_dir_all(root.join("shipped")).expect("the shipped copy moves aside");
    run_ok(&["install", tarball.to_str().expect("UTF-8")]);
    run_ok(&["show", "snippet-look"]);
    run_ok(&["apply", "snippet-look", "--trust"]);
    // Every refusal shape stays loud and actionable.
    run_err(&["show", "nosuchlook"]);
    run_err(&["apply", "LOUD"]);
    run_err(&["apply", "--for", "20"]);
    run_err(&["new", "x", "--from", "nosuchbase"]);
    run_err(&[
        "build",
        "nosuchlook",
        "--out",
        out_dir.to_str().expect("UTF-8"),
    ]);
    // Nothing escaped the sandbox.
    assert_no_leak();
}

/// One `scoot look ...` run that must succeed.
fn run_ok(args: &[&str]) {
    let look = parse_look(args.iter().map(|arg| (*arg).to_owned())).unwrap_or_else(|error| {
        panic!("`scoot look {}` does not parse: {error}", args.join(" "));
    });
    if let Err(error) = scootctl::look::run(&look) {
        panic!("`scoot look {}` failed: {error}", args.join(" "));
    }
}

/// One `scoot look ...` run that must refuse (parse or run).
fn run_err(args: &[&str]) {
    match parse_look(args.iter().map(|arg| (*arg).to_owned())) {
        Err(_) => {}
        Ok(look) => {
            if scootctl::look::run(&look).is_ok() {
                panic!(
                    "`scoot look {}` succeeded, expected a refusal",
                    args.join(" ")
                );
            }
        }
    }
}

/// Every sandboxed dir must resolve under the scratch root: a pinned
/// caller never touches the developer's real HOME.
fn assert_no_leak() {
    let root = PathBuf::from(std::env::var_os("SCOOT_LOOK_HELP_DOCS_ROOT").expect("a root"));
    for var in [
        "HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        let dir = PathBuf::from(std::env::var_os(var).unwrap_or_default());
        assert!(
            dir.starts_with(&root),
            "{var} escapes the sandbox: {}",
            dir.display()
        );
    }
}
