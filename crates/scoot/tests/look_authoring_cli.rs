#![cfg(target_os = "linux")]
//! `scoot look new`, `watch` and `preview --capture` against a real
//! headless session: authoring works with no Nix, a watched save moves the
//! screen, an invalid save refuses loudly with the last good look still
//! live, the cache holds one reused slot per watched look, and a capture
//! writes a real `preview.png`.
//!
//! The session runs the real `scoot` binary (`CARGO_BIN_EXE_scoot`) with a
//! scratch `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
//! `HOME`, and the catalog comes from `SCOOT_LOOKS_DIR` (plain files, no
//! Nix). The CLI targets the session's private state dir (`--state`) and
//! socket (`--socket`) explicitly, so no developer's real look is in reach.
//! The settings bus is neutralized for every CLI child, so the toolkit
//! step can only note, never write outside the test.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{Receiver, channel};
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

/// A real, loadable font file for the stub to answer, found without
/// fontconfig: first the host `fc-match` (the normal case), then a direct
/// scan of the usual font roots. Panics with the roots tried when neither
/// finds one.
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
/// find the stub `fc-match` before any host one.
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

/// A test look with a recolorable flat wallpaper.
fn look_toml(name: &str, background: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "An integration test look."
polarity = "dark"
license = "MIT"

[palette]
background = "{background}"
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
ring-width = 2

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "{background}"
mode = "fill"
"##
    )
}

struct Scratch {
    dir: PathBuf,
    tmp: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "scoot-look-authoring-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        // The children's TMPDIR, beside (never under) the scratch dir:
        // unix sockets bind at most 107 bytes, and a TMPDIR nested under
        // the long scratch name pushes the capture session's sockets over
        // it. Short on purpose; the counter keeps parallel tests apart.
        let tmp = std::env::temp_dir().join(format!(
            "slt-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("a scratch tmp dir");
        Self { dir, tmp }
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
        std::fs::remove_dir_all(&self.tmp).ok();
    }
}

struct Session {
    child: Child,
    socket: PathBuf,
    runtime: PathBuf,
    cache: PathBuf,
    config: PathBuf,
    home: PathBuf,
    tmp: PathBuf,
    data_home: PathBuf,
    state_home: PathBuf,
    shipped: PathBuf,
    log: PathBuf,
    path_env: OsString,
}

impl Session {
    fn start(scratch: &Scratch) -> Self {
        let stub = stub_fc_match_dir(scratch);
        let path_env = path_with_stub(&stub);
        let runtime = scratch.sub("runtime");
        let cache = scratch.sub("cache");
        let config = scratch.sub("config");
        let home = scratch.sub("home");
        let tmp = scratch.tmp.clone();
        let data_home = scratch.sub("data-home");
        let state_home = scratch.sub("state-home");
        let shipped = scratch.dir.join("shipped");
        std::fs::create_dir_all(&shipped).expect("a shipped dir");
        for (name, background) in [("look-a", "#2B3648"), ("look-b", "#3C4C5C")] {
            let dir = shipped.join(name);
            std::fs::create_dir_all(&dir).expect("a look dir");
            std::fs::write(dir.join("look.toml"), look_toml(name, background))
                .expect("a look.toml");
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
            .env("TMPDIR", &tmp)
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
            tmp,
            data_home,
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

    /// The base environment for every `scoot look` child: the catalog
    /// from `SCOOT_LOOKS_DIR`, the session's private state and socket, a
    /// neutralized settings bus, and the stub `fc-match` first on PATH.
    fn look_command(&self, args: &[&str]) -> Command {
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
            .env("XDG_DATA_HOME", &self.data_home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("TMPDIR", &self.tmp)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE");
        command
    }

    fn look(&self, args: &[&str]) -> Output {
        self.look_command(args).output().expect("scoot look runs")
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

    fn msg(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("PATH", &self.path_env)
            .env("SCOOT_SOCKET", &self.socket)
            .output()
            .expect("scoot msg runs")
    }

    /// Screenshots the session's output into `to`, waiting until the
    /// pixel at (10, 10) reads `want` (a flat color wallpaper fills the
    /// frame there).
    fn wait_wallpaper_pixel(&self, to: &std::path::Path, want: (u8, u8, u8), what: &str) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let shot = self.msg(&["screenshot", "--no-cursor", "--out", &to.to_string_lossy()]);
            if shot.status.success()
                && let Ok(bytes) = std::fs::read(to)
                && let Ok(got) = pixel_at(&bytes, 10, 10)
                && close_enough(got, want)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the screen never showed {what}: last shot:\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// A `scoot look watch` child with its output streaming: the test reads
/// one `applied:` line per save. Stdout also carries each save's
/// companion lines (`reloaded:`, `shadowed keys:`); refusals and notes
/// go to stderr, loudly, and keep the last good look.
struct Watch {
    child: Child,
    lines: Receiver<(bool, String)>,
}

impl Watch {
    fn spawn(session: &Session, dir: &std::path::Path) -> Self {
        let mut child = session
            .look_command(&["watch", &dir.to_string_lossy()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("a watch child");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (send, lines) = channel();
        let send_err = send.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send((true, line)).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if send_err.send((false, line)).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// The next `applied:` line for a good save. A save also prints its
    /// companion lines (`reloaded:`, `shadowed keys:`) on stdout, which
    /// belong to the previous save when they arrive first, so they are
    /// skipped; anything else on stdout is unexpected and panics.
    fn next_line(&self, what: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok((true, line)) if line.contains("applied:") => return line,
                Ok((true, line))
                    if line.starts_with("reloaded:") || line.starts_with("shadowed keys:") =>
                {
                    continue;
                }
                Ok((true, line)) => panic!("unexpected watch output for {what}: {line}"),
                Ok((false, _)) => continue,
                Err(_) => panic!("no watch output for {what}"),
            }
        }
    }

    /// The next stderr line holding `needle`: refusals land here loudly.
    fn next_stderr(&self, needle: &str, what: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok((false, line)) if line.contains(needle) => return line,
                Ok(_) => continue,
                Err(_) => panic!("no watch refusal for {what}"),
            }
        }
    }

    /// No stdout line arrives within `within`: a refused save applies
    /// nothing, so the loop prints nothing to stdout.
    fn no_stdout_within(&self, within: Duration, what: &str) {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok((true, line)) => panic!("unexpected watch output for {what}: {line}"),
                Ok((false, _)) => continue,
                Err(_) => return,
            }
        }
    }

    fn kill(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Whether two colors match within the smoke test's own tolerance (±2 per
/// channel).
fn close_enough(got: (u8, u8, u8), want: (u8, u8, u8)) -> bool {
    got.0.abs_diff(want.0) <= 2 && got.1.abs_diff(want.1) <= 2 && got.2.abs_diff(want.2) <= 2
}

/// The pixel at (`x`, `y`) of a headless screenshot PNG.
fn pixel_at(png: &[u8], x: u32, y: u32) -> Result<(u8, u8, u8), String> {
    use png::ColorType;
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("no PNG size")?];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|error| error.to_string())?;
    let channels = match info.color_type {
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        _ => return Err(format!("unexpected color type {:?}", info.color_type)),
    };
    let width = info.width;
    let at = (y * width + x) as usize * channels;
    let pixel = buf.get(at..at + channels).ok_or("pixel outside the shot")?;
    let gray = pixel[0];
    Ok(match channels {
        1 | 2 => (gray, gray, gray),
        _ => (pixel[0], pixel[1], pixel[2]),
    })
}

fn hex(hex: &str) -> (u8, u8, u8) {
    assert_eq!(hex.len(), 7, "a #rrggbb color");
    assert!(hex.starts_with('#'), "a #rrggbb color");
    (
        u8::from_str_radix(&hex[1..3], 16).expect("hex"),
        u8::from_str_radix(&hex[3..5], 16).expect("hex"),
        u8::from_str_radix(&hex[5..7], 16).expect("hex"),
    )
}

#[test]
fn new_starts_a_user_look_that_applies() {
    let scratch = Scratch::new("new");
    let session = Session::start(&scratch);
    // `new` writes under the scratch data home, so the author's real
    // looks are never in reach.
    let user_looks = session.data_home.join("scoot/user-looks");
    let created = session.check_look(&["new", "mine", "--from", "look-a", "--json"]);
    let created: serde_json::Value = serde_json::from_str(&created).expect("JSON from new");
    assert_eq!(created["name"], "mine");
    assert_eq!(created["extends"], "look-a");
    assert_eq!(created["valid"], true);
    let manifest = PathBuf::from(created["created"].as_str().expect("a path"));
    assert_eq!(manifest, user_looks.join("mine"));
    let text = std::fs::read_to_string(manifest.join("look.toml")).expect("a manifest");
    assert!(text.contains("extends = \"look-a\""), "{text}");
    // The new look resolves through the catalog and applies.
    session.check_look(&["validate", &manifest.to_string_lossy()]);
    let applied = session.check_look(&["apply", "mine", "--json"]);
    let applied: serde_json::Value = serde_json::from_str(&applied).expect("JSON from apply");
    assert_eq!(applied["applied"], "mine");
    // An installed look (someone else's) is never a base: refused loudly,
    // nothing written.
    let theirs = session.data_home.join("scoot/installed-looks/theirs");
    std::fs::create_dir_all(&theirs).expect("an installed dir");
    std::fs::write(theirs.join("look.toml"), look_toml("theirs", "#2B3648"))
        .expect("an installed look.toml");
    let out = session.look(&["new", "second", "--from", "theirs"]);
    assert!(!out.status.success(), "an installed base refuses");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("never extended"),
        "loud about installed bases: {stderr}"
    );
    assert!(!user_looks.join("second").exists(), "nothing was written");
    // An unknown base refuses the same way.
    let out = session.look(&["new", "third", "--from", "nosuchlook"]);
    assert!(!out.status.success(), "an unknown base refuses");
    assert!(!user_looks.join("third").exists(), "nothing was written");
}

#[test]
fn watch_follows_three_edits_and_refuses_an_invalid_save() {
    let scratch = Scratch::new("watch");
    let session = Session::start(&scratch);
    // The watched look lives in the scratch data home's user-looks:
    // the author's real files are never in reach.
    let watched = session.data_home.join("scoot/user-looks/mine");
    std::fs::create_dir_all(&watched).expect("a look dir");
    std::fs::write(watched.join("look.toml"), look_toml("mine", "#2B3648")).expect("a look.toml");
    let mut watch = Watch::spawn(&session, &watched);
    // The initial apply: the screen follows from the start.
    let first = watch.next_line("the initial apply");
    assert!(first.contains("applied: mine"), "the loop applies: {first}");
    let shot = scratch.dir.join("shot.png");
    session.wait_wallpaper_pixel(&shot, hex("#2B3648"), "the initial background");
    // Three edits: the screen follows each one.
    for (background, label) in [
        ("#123456", "edit one"),
        ("#654321", "edit two"),
        ("#1A2B3C", "edit three"),
    ] {
        std::fs::write(watched.join("look.toml"), look_toml("mine", background))
            .expect("an edited look.toml");
        let line = watch.next_line(label);
        assert!(line.contains("applied: mine"), "{label} applies: {line}");
        session.wait_wallpaper_pixel(&shot, hex(background), label);
    }
    let generation = || {
        let current = session.check_look(&["current"]);
        current.lines().next().expect("a current line").to_owned()
    };
    assert!(
        generation().contains("generation 4"),
        "three saves bump to 4: {}",
        generation()
    );
    // An invalid save: refused loudly (on stderr), the last good look
    // stays live and on screen, and no stdout line follows it.
    std::fs::write(
        watched.join("look.toml"),
        look_toml("mine", "#1A2B3C").replace("#FFA45C", "amber"),
    )
    .expect("an invalid look.toml");
    let refused = watch.next_stderr("refused:", "the invalid save");
    assert!(refused.contains("invalid save"), "loud: {refused}");
    watch.no_stdout_within(Duration::from_secs(2), "the invalid save");
    assert!(
        generation().contains("generation 4"),
        "no bump on refusal: {}",
        generation()
    );
    session.wait_wallpaper_pixel(&shot, hex("#1A2B3C"), "the last good background");
    // A fix applies again.
    std::fs::write(watched.join("look.toml"), look_toml("mine", "#0E0E0E"))
        .expect("a fixed look.toml");
    let line = watch.next_line("the fix");
    assert!(line.contains("applied: mine"), "the fix applies: {line}");
    session.wait_wallpaper_pixel(&shot, hex("#0E0E0E"), "the fixed background");
    // One reused slot: the cache holds exactly it, whatever the saves.
    let mut rendered: Vec<String> = std::fs::read_dir(session.cache.join("scoot/rendered"))
        .expect("a render cache")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    rendered.sort();
    assert_eq!(
        rendered,
        vec!["mine-watch"],
        "one reused slot: {rendered:?}"
    );
    watch.kill();
}

#[test]
fn preview_capture_writes_a_png_from_a_private_session() {
    let scratch = Scratch::new("capture");
    let session = Session::start(&scratch);
    // Bare preview reports the missing image first.
    let before = session.check_look(&["preview", "look-a", "--json"]);
    let before: serde_json::Value = serde_json::from_str(&before).expect("JSON from preview");
    assert_eq!(before["name"], "look-a");
    assert_eq!(before["exists"], false);
    // Capture renders look-a in a private session and writes the PNG.
    let png = scratch.dir.join("look-a-preview.png");
    let captured = session.check_look(&[
        "preview",
        "look-a",
        "--capture",
        "--out",
        &png.to_string_lossy(),
        "--width",
        "800",
        "--height",
        "600",
    ]);
    assert!(captured.contains("preview:"), "a capture line: {captured}");
    let bytes = std::fs::read(&png).expect("a preview.png");
    assert!(
        bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
        "a PNG"
    );
    assert!(
        pixel_at(&bytes, 10, 10)
            .map(|pixel| close_enough(pixel, hex("#2B3648")))
            .unwrap_or(false),
        "the capture shows look-a's background"
    );
    // Capturing into the source dir is the default: the author's session
    // is untouched (its state dir holds no new record).
    let dir = scratch.dir.join("mine");
    std::fs::create_dir_all(&dir).expect("a look dir");
    std::fs::write(dir.join("look.toml"), look_toml("mine", "#654321")).expect("a look.toml");
    session.check_look(&["preview", &dir.to_string_lossy(), "--capture"]);
    let bytes = std::fs::read(dir.join("preview.png")).expect("a default preview.png");
    assert!(
        bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
        "a PNG"
    );
    assert!(
        pixel_at(&bytes, 10, 10)
            .map(|pixel| close_enough(pixel, hex("#654321")))
            .unwrap_or(false),
        "the capture shows the directory look's background"
    );
}
