#![cfg(target_os = "linux")]
//! `scoot look` against a real headless session: the CLI binary applies two
//! looks through the compositor's live scoped reload, and a pending user
//! edit survives the swap untouched.
//!
//! The session runs the real `scoot` binary (`CARGO_BIN_EXE_scoot`) with a
//! scratch `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
//! `HOME`, and the catalog comes from `SCOOT_LOOKS_DIR` (plain files, no
//! Nix). The CLI targets the session's private state dir (`--state`) and
//! socket (`--socket`) explicitly, so no developer's real look is in reach.
//! The settings bus is neutralized for the CLI child, so the toolkit step
//! can only note, never write outside the test.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, applies take milliseconds, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(60);

/// A hermetic `fc-match` for the `scoot look` children: the CI runners have
/// no JetBrains Mono (and the dev VM has no DejaVu Sans Mono), so the test
/// looks must not depend on host font *names*. The stub answers the test
/// look's families with a real font file and the same family name, and every
/// other family the way real `fc-match` answers a missing one (a fallback
/// file with a different family), so `ProdChecks`' family comparison still
/// refuses unknown fonts. The answered file must really load: the bar's
/// `daemon --check` opens it. It is found without fontconfig (first the
/// real `fc-match`, then a filesystem scan of the usual font roots), so the
/// suite also passes with an empty `FONTCONFIG_FILE`. Written into the
/// scratch dir (removed on drop), prepended to `PATH` for every `scoot`
/// child below.
fn stub_fc_match_dir(scratch: &Scratch) -> PathBuf {
    let font = real_font_file();
    let dir = scratch.dir.join("bin");
    std::fs::create_dir_all(&dir).expect("a stub bin dir");
    let stub = dir.join("fc-match");
    std::fs::write(
        &stub,
        format!(
            r#"#!/bin/sh
# $1 is --format=%{{file}} or --format=%{{family}} (with a trailing newline),
# $2 is the family. Match on the field name, never the exact flag.
fmt="$1"
fam="$2"
case "$fam" in
  "DejaVu Sans"|"DejaVu Sans Mono"|"JetBrains Mono")
    case "$fmt" in
      *{{file}}*) printf '%s\n' "{font}" ;;
      *) printf '%s\n' "$fam" ;;
    esac
    ;;
  *)
    case "$fmt" in
      *{{file}}*) printf '%s\n' "{font}" ;;
      *) printf 'DejaVu Sans\n' ;;
    esac
    ;;
esac
"#,
            font = font.display(),
        ),
    )
    .expect("a stub fc-match");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
    dir
}

/// A `fuzzel --dmenu` stand-in beside the `fc-match` stub: it reads the
/// catalog names on stdin and answers per `$STUB_FUZZEL` — `choose:NAME`
/// prints the choice, `cancel` exits 1 silently (what a real Esc does),
/// `fail` prints a choice and exits 3 (a broken menu: loud, never
/// applied). Unset is a loud error, so a test that forgets the mode fails
/// instead of applying.
fn stub_fuzzel_menu(bin: &std::path::Path) {
    let stub = bin.join("fuzzel");
    std::fs::write(
        &stub,
        r#"#!/bin/sh
mode="${STUB_FUZZEL:-unset}"
case "$mode" in
  choose:*)
    name="${mode#choose:}"
    while IFS= read -r line; do :; done
    printf '%s\n' "$name"
    ;;
  cancel)
    while IFS= read -r line; do :; done
    exit 1
    ;;
  fail)
    while IFS= read -r line; do :; done
    printf 'look-b\n'
    exit 3
    ;;
  *)
    echo "STUB_FUZZEL mode unset (choose:NAME, cancel, fail)" >&2
    exit 2
    ;;
esac
"#,
    )
    .expect("a stub fuzzel");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
}
/// A real, loadable font file for the stub to answer, found without
/// fontconfig: first the host `fc-match` (the normal case), then a direct
/// scan of the usual font roots (an empty `FONTCONFIG_FILE` blinds the
/// former but not the filesystem). Panics with the roots tried when neither
/// finds one: a machine with no font file at all cannot run the bar check
/// either way.
fn real_font_file() -> PathBuf {
    use std::process::Command;
    for family in ["DejaVu Sans", "DejaVu Sans Mono", "sans-serif"] {
        if let Ok(out) = Command::new("fc-match")
            .arg("--format=%{file}\n")
            .arg(family)
            .output()
            && out.status.success()
        {
            let path = String::from_utf8(out.stdout).unwrap_or_default();
            let path = path.trim().to_owned();
            if !path.is_empty() && std::path::Path::new(&path).is_file() {
                return PathBuf::from(path);
            }
        }
    }
    let roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/X11/fonts"),
    ];
    let mut tried = Vec::new();
    for root in &roots {
        tried.push(root.display().to_string());
        if let Some(found) = first_font_under(root, 4) {
            return found;
        }
    }
    panic!(
        "no loadable font file: fc-match found nothing and no .ttf/.otf under {}",
        tried.join(", ")
    );
}

/// The first `.ttf`/`.otf` file under `root`, depth-first, at most `depth`
/// levels down (symlinks followed; the depth cap bounds any loop).
fn first_font_under(root: &std::path::Path, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.is_file()
            && let Some(ext) = path.extension().and_then(|ext| ext.to_str())
            && (ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"))
        {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter()
        .find_map(|dir| first_font_under(&dir, depth - 1))
}

/// This process's `PATH` with `stub` first, so the `scoot look` children
/// find the stub `fc-match` before any host one. `scootbg`/`scootbar` still
/// resolve beside the binary, so only the font lookup is stubbed.
fn path_with_stub(stub: &std::path::Path) -> OsString {
    let mut out = stub.as_os_str().to_owned();
    if let Some(path) = std::env::var_os("PATH")
        && !path.is_empty()
    {
        out.push(":");
        out.push(path);
    }
    out
}

fn scoot() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scoot"))
}

fn look_toml(name: &str, ring_width: i64) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "An integration test look."
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
mono = "JetBrains Mono"
size = 10

[shape]
radius = 14
ring-width = {ring_width}

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "#2B3648"
mode = "fill"
"##
    )
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "scoot-look-apply-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        Self { dir }
    }

    fn sub(&self, name: &str) -> PathBuf {
        let dir = self.dir.join(name);
        std::fs::create_dir_all(&dir).expect("a scratch subdir");
        if name == "runtime" {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .expect("an owner-only runtime dir");
        }
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

struct Session {
    child: Child,
    socket: PathBuf,
    runtime: PathBuf,
    cache: PathBuf,
    config: PathBuf,
    home: PathBuf,
    state_home: PathBuf,
    shipped: PathBuf,
    log: PathBuf,
    path_env: OsString,
}

impl Session {
    fn start(scratch: &Scratch) -> Self {
        let stub = stub_fc_match_dir(scratch);
        stub_fuzzel_menu(&stub);
        let path_env = path_with_stub(&stub);
        let runtime = scratch.sub("runtime");
        let cache = scratch.sub("cache");
        let config = scratch.sub("config");
        let home = scratch.sub("home");
        let state_home = scratch.sub("state-home");
        let shipped = scratch.dir.join("shipped");
        std::fs::create_dir_all(&shipped).expect("a shipped dir");
        for (name, ring) in [("look-a", 2), ("look-b", 3)] {
            let dir = shipped.join(name);
            std::fs::create_dir_all(&dir).expect("a look dir");
            std::fs::write(dir.join("look.toml"), look_toml(name, ring)).expect("a look.toml");
        }
        let socket = runtime.join("scoot.sock");
        let log = scratch.dir.join("scoot.log");
        let log_file = std::fs::File::create(&log).expect("a log file");
        let child = Command::new(scoot())
            .arg("--headless")
            .arg("--socket")
            .arg(&socket)
            .arg("--")
            .arg("sleep")
            .arg("120")
            .env("XDG_STATE_HOME", &state_home)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &home)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .env_remove("SCOOT_LOOKS_DIR")
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .expect("a headless scoot");
        let mut session = Self {
            child,
            socket,
            runtime,
            cache,
            config,
            home,
            state_home,
            shipped,
            log,
            path_env,
        };
        session.wait_until_up();
        session
    }

    fn wait_until_up(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let up = Command::new(scoot())
                .arg("msg")
                .arg("version")
                .env("SCOOT_SOCKET", &self.socket)
                .env("PATH", &self.path_env)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if up {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the headless session never answered IPC; log:\n{}",
                self.log_tail()
            );
            assert!(
                self.child.try_wait().expect("waitable").is_none(),
                "the headless session exited during startup; log:\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut tail = text.lines().rev().take(20).collect::<Vec<_>>();
        tail.reverse();
        tail.join("\n")
    }

    /// The session's private look state dir: the one `scoot-look-*/` entry
    /// under the runtime dir.
    fn private_dir(&self) -> PathBuf {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.runtime).expect("readable runtime dir") {
            let entry = entry.expect("an entry");
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("scoot-look-")
            {
                found.push(entry.path());
            }
        }
        assert_eq!(found.len(), 1, "one private dir, not {found:?}");
        found.pop().expect("one private dir")
    }

    /// One `scoot look ...` invocation against this session: the catalog
    /// from `SCOOT_LOOKS_DIR`, the session's private state and socket, a
    /// neutralized settings bus (the toolkit step notes, never writes).
    /// `PATH` carries the stub `fc-match` first, so the font check never
    /// reaches host fonts.
    fn look(&self, args: &[&str]) -> Output {
        self.look_with(args, &[])
    }

    /// One `scoot look ...` invocation with extra environment (which wins
    /// over the session's): the pick tests drive the stub menu through
    /// `$STUB_FUZZEL` and take `fuzzel` off PATH this way.
    fn look_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(scoot());
        command
            .arg("look")
            .args(args)
            .arg("--state")
            .arg(self.private_dir())
            .arg("--socket")
            .arg(&self.socket)
            .env("PATH", &self.path_env)
            .env("SCOOT_LOOKS_DIR", &self.shipped)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE");
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().expect("scoot look runs")
    }

    fn msg(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("PATH", &self.path_env)
            .env("SCOOT_SOCKET", &self.socket)
            .output()
            .expect("scoot msg runs")
    }

    /// A live Wayland display for the pick tests: binds `name` under the
    /// session's runtime dir (where `look_with` points `XDG_RUNTIME_DIR`)
    /// and returns the held listener with the `WAYLAND_DISPLAY` value to
    /// pass. The caller holds the listener while `scoot look` runs: the
    /// probe dials this socket, never a compositor. Dropping it first is
    /// itself a dead display.
    fn display(&self, name: &str) -> (std::os::unix::net::UnixListener, String) {
        use std::os::unix::net::UnixListener;
        let held = UnixListener::bind(self.runtime.join(name)).expect("a live display socket");
        (held, name.to_owned())
    }

    fn check_look(&self, args: &[&str]) -> String {
        let out = self.look(args);
        assert!(
            out.status.success(),
            "`scoot look {}` failed:\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF-8 stdout")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[test]
fn offline_commands_list_show_validate_and_build() {
    let scratch = Scratch::new("offline");
    let session = Session::start(&scratch);
    let list = session.check_look(&["list"]);
    assert!(list.contains("look-a"), "the catalog lists look-a:\n{list}");
    assert!(list.contains("look-b"), "the catalog lists look-b:\n{list}");
    let show = session.check_look(&["show", "look-a", "--json"]);
    let show: serde_json::Value = serde_json::from_str(&show).expect("JSON from show");
    assert_eq!(show["name"], "look-a");
    assert_eq!(show["palette"]["background"], "#2B3648");
    session.check_look(&["validate", "look-a"]);
    let dir = session.shipped.join("look-a");
    session.check_look(&["validate", &dir.to_string_lossy()]);
    let out = scratch.dir.join("built");
    session.check_look(&["build", "look-a", "--out", &out.to_string_lossy()]);
    for file in [
        "scoot.toml",
        "bar.toml",
        "fuzzel.ini",
        "swaylock",
        "greet.toml",
        "render.json",
    ] {
        assert!(out.join(file).is_file(), "build writes {file}");
    }
    let current = session.check_look(&["current", "--json"]);
    let current: serde_json::Value = serde_json::from_str(&current).expect("JSON from current");
    assert_eq!(current["active"], serde_json::Value::Null);
}

#[test]
fn json_failure_shapes_are_documents_on_stdout() {
    // `--json` always prints a document on stdout, even when the run
    // fails (exit 1, with the text error on stderr): `validate` with
    // effectful refusals, and `apply` with an unknown or refused look.
    let scratch = Scratch::new("json-fail");
    let session = Session::start(&scratch);
    // An invalid look directory: parses, but names a font that is not
    // installed, so validation fails with errors (not a bare refusal).
    let bad = scratch.dir.join("look-bad");
    std::fs::create_dir_all(&bad).expect("a look dir");
    let mut toml = look_toml("look-bad", 2);
    toml = toml.replace("DejaVu Sans", "ScootNoSuchFont");
    toml = toml.replace("JetBrains Mono", "ScootNoSuchFontMono");
    std::fs::write(bad.join("look.toml"), toml).expect("a look.toml");
    let out = session.look(&["validate", &bad.to_string_lossy(), "--json"]);
    assert_eq!(out.status.code(), Some(1), "validate fails");
    let doc: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).expect("UTF-8")).expect("a JSON doc");
    assert_eq!(doc["valid"], false);
    assert!(
        doc["errors"].as_array().expect("an errors list").len() >= 3,
        "font refusals land in errors: {doc}"
    );
    // An unknown look: apply refuses, with the failure document.
    let out = session.look(&["apply", "nosuchlook", "--json"]);
    assert_eq!(out.status.code(), Some(1), "apply fails");
    let doc: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).expect("UTF-8")).expect("a JSON doc");
    assert_eq!(doc["applied"], false);
    assert_eq!(doc["target"], "nosuchlook");
    assert!(
        doc["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown look"),
        "{doc}"
    );
}
#[test]
fn a_swap_applies_two_looks_through_the_scoped_reload() {
    let scratch = Scratch::new("swap");
    let session = Session::start(&scratch);
    let first = session.check_look(&["apply", "look-a", "--json"]);
    let first: serde_json::Value = serde_json::from_str(&first).expect("JSON from apply");
    assert_eq!(first["applied"], "look-a");
    assert_eq!(first["generation"], 1);
    assert!(
        first["reloaded"]
            .as_array()
            .expect("a reloaded list")
            .iter()
            .any(|entry| entry == "compositor"),
        "the running compositor reloads: {first}",
    );
    let private = session.private_dir();
    let record = std::fs::read_to_string(private.join("look.json")).expect("a record");
    assert!(
        record.contains("\"name\":\"look-a\"") || record.contains("\"name\": \"look-a\""),
        "{record}"
    );
    let link = std::fs::read_link(private.join("look")).expect("a link");
    assert!(
        link.is_dir(),
        "the link points at a render: {}",
        link.display()
    );
    // The swap renders the next-launch leaves too: the launcher and the
    // locker read them at their next start, and the login screen reads
    // its fragment at system build time.
    for file in [
        "scoot.toml",
        "bar.toml",
        "fuzzel.ini",
        "swaylock",
        "greet.toml",
        "render.json",
    ] {
        assert!(link.join(file).is_file(), "a swap renders {file}");
    }
    let second = session.check_look(&["apply", "look-b", "--json"]);
    let second: serde_json::Value = serde_json::from_str(&second).expect("JSON from apply");
    assert_eq!(second["applied"], "look-b");
    assert_eq!(second["generation"], 2);
    let current = session.check_look(&["current"]);
    assert!(
        current.contains("look-b"),
        "current names the swap:\n{current}"
    );
}

#[test]
fn a_swap_never_applies_a_pending_user_edit() {
    let scratch = Scratch::new("scope");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // A pending edit: a new bind in the user's own file, with no reload of
    // its own. The swap below must not apply it.
    let config = session.config.join("scoot/config.toml");
    std::fs::create_dir_all(config.parent().expect("a parent")).expect("a config dir");
    std::fs::write(&config, "[binds]\n\"super+F12\" = \"close\"\n").expect("a pending bind");
    session.check_look(&["apply", "look-b"]);
    let binds = session.msg(&["binds", "--json"]);
    assert!(binds.status.success(), "binds answers");
    let before = String::from_utf8(binds.stdout).expect("UTF-8 binds");
    assert!(
        !before.contains("super+F12"),
        "the swap applied a pending edit:\n{before}",
    );
    // A plain reload applies what the swap held back.
    let reload = session.msg(&["reload"]);
    assert!(reload.status.success(), "a plain reload answers");
    let binds = session.msg(&["binds", "--json"]);
    let after = String::from_utf8(binds.stdout).expect("UTF-8 binds");
    assert!(
        after.contains("super+F12"),
        "the plain reload did not apply the pending bind:\n{after}",
    );
}

#[test]
fn a_swap_renders_the_terminal_fragment_for_new_foot() {
    // End to end through the real session and the real CLI: a look with
    // a `[terminal]` table applies, the rendered `foot.ini` carries the
    // look's hues in foot's order, and the reply walks the user through
    // the one-time `include` setup until it exists. Where foot is
    // installed (the dev VM carries foot 1.28.0) its own
    // `--check-config` parses the fragment; a box without one notes and
    // keeps the fragment.
    let scratch = Scratch::new("foot");
    let session = Session::start(&scratch);
    let dir = session.shipped.join("look-t");
    std::fs::create_dir_all(&dir).expect("a look dir");
    let mut toml = look_toml("look-t", 2);
    toml.push_str(
        "[terminal]\ncolors = [\"#1a2230\", \"#c96a63\", \"#8fa382\", \"#e8a34c\", \"#7c8fb0\", \"#9a7b94\", \"#7e9bb0\", \"#c9bfae\", \"#5e6b7f\", \"#e87f6a\", \"#a9bd9b\", \"#ffd54a\", \"#93a8c8\", \"#b595ad\", \"#a8c4d8\", \"#f6eedc\"]\n",
    );
    std::fs::write(dir.join("look.toml"), toml).expect("a look.toml");
    let link_path = session.private_dir().join("look/foot.ini");
    // No foot.ini yet: the reply carries the one-time setup hint.
    let out = session.look(&["apply", "look-t"]);
    assert!(
        out.status.success(),
        "apply failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains(&*link_path.to_string_lossy()),
        "no setup hint names the link path:\n{stderr}"
    );
    // The rendered fragment holds the look's hues in foot's order, in
    // both themes with the look's polarity selected: a light look (and a
    // user's own opposite `initial-color-theme`) can no longer leave a new
    // terminal on the wrong palette.
    let rendered = std::fs::read_link(session.private_dir().join("look")).expect("a link");
    let foot_ini = rendered.join("foot.ini");
    let foot = std::fs::read_to_string(&foot_ini).expect("a foot.ini");
    assert!(foot.contains("[colors-dark]"), "wrong section:\n{foot}");
    assert!(foot.contains("[colors-light]"), "missing mirror:\n{foot}");
    assert!(
        foot.contains("initial-color-theme=dark"),
        "wrong theme:\n{foot}"
    );
    assert!(foot.contains("regular0=1a2230"), "ansi order lost:\n{foot}");
    assert!(foot.contains("bright7=f6eedc"), "bright hues lost:\n{foot}");
    match Command::new("foot")
        .arg("--check-config")
        .arg("--config")
        .arg(&foot_ini)
        .output()
    {
        Ok(check) => assert!(
            check.status.success(),
            "foot refuses its own fragment:\n{}",
            String::from_utf8_lossy(&check.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("cannot run foot: {error}"),
    }
    // A user `[main]` holding the opposite theme with the fragment
    // included still parses: both themes carry the look's hues, so foot
    // shows them whichever theme is in effect. Where foot is installed
    // (the dev VM carries foot 1.28.0) its own parser is the judge; a box
    // without one skips this half and keeps the fragment.
    for user_first in [true, false] {
        let user = "[main]\ninitial-color-theme=light\n".to_owned();
        let include = format!("[main]\ninclude={}\n", link_path.display());
        let (head, tail) = if user_first {
            (user, include)
        } else {
            (include, user)
        };
        let adversarial = session.config.join(format!(
            "adversarial-{}.ini",
            if user_first { "first" } else { "last" }
        ));
        std::fs::write(&adversarial, format!("{head}{tail}")).expect("an adversarial ini");
        match Command::new("foot")
            .arg("--check-config")
            .arg("--config")
            .arg(&adversarial)
            .output()
        {
            Ok(check) => assert!(
                check.status.success(),
                "foot refuses the opposite-theme include (user first: {user_first}):\n{}",
                String::from_utf8_lossy(&check.stderr)
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot run foot: {error}"),
        }
    }
    // With the include in place, the reply says new terminals follow the
    // look instead of repeating the setup hint.
    let foot_home = session.config.join("foot");
    std::fs::create_dir_all(&foot_home).expect("a foot dir");
    std::fs::write(
        foot_home.join("foot.ini"),
        format!("[main]\ninclude={}\n", link_path.display()),
    )
    .expect("a foot.ini");
    let out = session.look(&["apply", "look-t"]);
    assert!(
        out.status.success(),
        "re-apply failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("new terminals read the look's colors"),
        "no follow note:\n{stderr}"
    );
    assert!(
        !stderr.contains("once foot.ini includes"),
        "a setup hint survives the include:\n{stderr}"
    );
}
/// The active look's name from `current --json` (None when nothing applied).
fn active_name(session: &Session) -> Option<String> {
    let out = session.look(&["current", "--json"]);
    assert!(out.status.success(), "current answers");
    let doc: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).expect("UTF-8")).expect("JSON");
    doc.get("active")
        .and_then(|active| active.get("name"))
        .and_then(|name| name.as_str())
        .map(str::to_owned)
}

/// The active generation from `current --json`.
fn active_generation(session: &Session) -> Option<u64> {
    let out = session.look(&["current", "--json"]);
    assert!(out.status.success(), "current answers");
    let doc: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).expect("UTF-8")).expect("JSON");
    doc.get("active")
        .and_then(|active| active.get("generation"))
        .and_then(|generation| generation.as_u64())
}

#[test]
fn cycle_next_prev_and_random_walk_the_catalog() {
    let scratch = Scratch::new("cycle");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
    session.check_look(&["next"]);
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    // Wraps past the end.
    session.check_look(&["next"]);
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
    // Wraps past the start.
    session.check_look(&["prev"]);
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    session.check_look(&["prev"]);
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
    // `random` never repeats the active look: with two looks it flips.
    session.check_look(&["random"]);
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    session.check_look(&["random"]);
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn list_names_prints_one_name_per_line() {
    let scratch = Scratch::new("list-names");
    let session = Session::start(&scratch);
    let names = session.check_look(&["list", "--names"]);
    let mut lines: Vec<&str> = names.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, ["look-a", "look-b"]);
    let out = session.look(&["list", "--names", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let mut names: Vec<String> =
        serde_json::from_slice(&out.stdout).expect("a JSON array of names");
    names.sort();
    assert_eq!(names, ["look-a".to_owned(), "look-b".to_owned()]);
}

#[test]
fn pick_applies_the_menu_choice() {
    let scratch = Scratch::new("pick-apply");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // Fuzzel is Wayland-only, so `pick` probes the display socket before
    // spawning; the stub menu never dials it.
    let (_live, display) = session.display("wayland-live");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", display.as_str()),
            ("STUB_FUZZEL", "choose:look-b"),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
}

#[test]
fn pick_cancel_applies_nothing_and_succeeds() {
    let scratch = Scratch::new("pick-cancel");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    let (_live, display) = session.display("wayland-live");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", display.as_str()),
            ("STUB_FUZZEL", "cancel"),
        ],
    );
    assert!(out.status.success(), "a cancel succeeds: {out:?}");
    assert!(out.stdout.is_empty(), "a cancel prints nothing: {out:?}");
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_cancel_json_reports_a_null_apply() {
    let scratch = Scratch::new("pick-cancel-json");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    let (_live, display) = session.display("wayland-live");
    let out = session.look_with(
        &["pick", "--json"],
        &[
            ("WAYLAND_DISPLAY", display.as_str()),
            ("STUB_FUZZEL", "cancel"),
        ],
    );
    assert!(out.status.success(), "a cancel succeeds: {out:?}");
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("a JSON cancel document");
    assert_eq!(doc["applied"], serde_json::Value::Null);
    assert_eq!(doc["cancelled"], serde_json::Value::Bool(true));
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_failure_is_loud_and_applies_nothing() {
    let scratch = Scratch::new("pick-fail");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    let (_live, display) = session.display("wayland-live");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", display.as_str()),
            ("STUB_FUZZEL", "fail"),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "a broken menu fails: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("menu"),
        "the failure names the menu: {stderr}"
    );
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_without_a_display_names_it_loudly() {
    let scratch = Scratch::new("pick-no-display");
    let session = Session::start(&scratch);
    // Fuzzel cannot choose with no display, and its empty failure looks
    // exactly like a cancel: refuse loudly instead of succeeding.
    let out = session.look(&["pick"]);
    assert_eq!(out.status.code(), Some(1), "no display fails: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("Wayland display"),
        "the failure names the display: {stderr}"
    );
}

#[test]
fn pick_with_a_dead_display_names_it_loudly() {
    let scratch = Scratch::new("pick-dead-display");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // A stale WAYLAND_DISPLAY (an ssh session, a restarted compositor):
    // no socket under the runtime dir. The stub would choose look-b, so
    // look-a staying active proves the probe refused before spawning it.
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", "wayland-nonexistent"),
            ("STUB_FUZZEL", "choose:look-b"),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "a dead display fails: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("Wayland display"),
        "the failure names the display: {stderr}"
    );
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_with_a_file_for_a_display_names_it_loudly() {
    let scratch = Scratch::new("pick-file-display");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // A plain file where the socket was is not connectable either.
    std::fs::write(session.runtime.join("wayland-file"), "not a socket").expect("a plain file");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", "wayland-file"),
            ("STUB_FUZZEL", "choose:look-b"),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "a file display fails: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("Wayland display"),
        "the failure names the display: {stderr}"
    );
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_with_an_absolute_dead_display_names_it_loudly() {
    let scratch = Scratch::new("pick-absent-display");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // An absolute WAYLAND_DISPLAY names the socket itself, not one under
    // the runtime dir.
    let absent = session.runtime.join("wayland-absent");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", absent.to_str().expect("UTF-8")),
            ("STUB_FUZZEL", "choose:look-b"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an absolute dead display fails: {out:?}"
    );
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("Wayland display"),
        "the failure names the display: {stderr}"
    );
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
}

#[test]
fn pick_without_fuzzel_names_it_loudly() {
    let scratch = Scratch::new("pick-no-fuzzel");
    let session = Session::start(&scratch);
    // A PATH with no menu at all: the failure names `fuzzel`, and nothing
    // applies. The display is live, so the probe passes and the spawn is
    // what fails.
    let empty = scratch.sub("empty-path");
    let path = empty.to_string_lossy().into_owned();
    let (_live, display) = session.display("wayland-live");
    let out = session.look_with(
        &["pick"],
        &[
            ("WAYLAND_DISPLAY", display.as_str()),
            ("PATH", path.as_str()),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "no menu fails: {out:?}");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("`fuzzel`"),
        "the failure names fuzzel: {stderr}"
    );
}

#[test]
fn a_timed_preview_reverts_through_the_live_reload() {
    let scratch = Scratch::new("preview-revert");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    // The CLI returns at once (the timer is detached); the revert lands
    // after the delay through the same scoped reloads an apply uses.
    session.check_look(&["apply", "look-b", "--for", "2"]);
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    let deadline = Instant::now() + PATIENCE;
    loop {
        if active_name(&session).as_deref() == Some("look-a") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the preview never reverted to look-a"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn keep_holds_a_preview_past_its_delay() {
    let scratch = Scratch::new("preview-keep");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    session.check_look(&["apply", "look-b", "--for", "3"]);
    session.check_look(&["apply", "--keep"]);
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    // Past the delay: the kept look stays, and the timer is gone.
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(active_name(&session), Some("look-b".to_owned()));
    let out = session.look(&["apply", "--keep"]);
    assert!(
        !out.status.success(),
        "a second keep refuses: nothing is pending"
    );
}

#[test]
fn a_preview_never_undoes_a_newer_apply_live() {
    let scratch = Scratch::new("preview-newer");
    let session = Session::start(&scratch);
    session.check_look(&["apply", "look-a"]);
    session.check_look(&["apply", "look-b", "--for", "2"]);
    let previewed = active_generation(&session).expect("a generation");
    session.check_look(&["apply", "look-a"]);
    let newer = active_generation(&session).expect("a generation");
    assert!(newer > previewed, "the newer apply moved the generation");
    // Past the first preview's delay: the newer choice stands.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
    assert_eq!(active_generation(&session), Some(newer));
}

#[test]
fn a_first_choice_preview_clears_the_layer_live() {
    // No apply yet: the preview is the first choice, so the revert
    // clears the layer (restoring the toolkit keys the preview wrote)
    // instead of restoring a look.
    let scratch = Scratch::new("preview-first");
    let session = Session::start(&scratch);
    assert_eq!(active_name(&session), None, "nothing applied yet");
    session.check_look(&["apply", "look-a", "--for", "2"]);
    assert_eq!(active_name(&session), Some("look-a".to_owned()));
    let deadline = Instant::now() + PATIENCE;
    loop {
        if active_name(&session).is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the first-choice preview never cleared"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
