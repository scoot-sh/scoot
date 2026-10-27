#![cfg(target_os = "linux")]
//! IPC look swaps against a real headless session: `scoot msg look-apply`
//! swaps through the compositor's parked child and answers when it lands,
//! and `scoot msg looks` reports the catalog, the active look and the
//! session's state directory.
//!
//! The session runs the real `scoot` binary (`CARGO_BIN_EXE_scoot`) with a
//! scratch `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
//! `HOME`, and the catalog comes from `SCOOT_LOOKS_DIR` (plain files, no
//! Nix) -- set on the session itself, so the compositor's own `looks`
//! answer and the `look apply` children it spawns read the same catalog.
//! `PATH` carries the stub `fc-match` first (as in `look_apply_cli.rs`),
//! also on the session, so the spawned children never reach host fonts.
//! The settings bus is disabled, so the toolkit step can only note.
//!
//! The test looks carry no `[wallpaper]`: with no wallpaper section
//! nothing hands anything to scootbg (which is not beside the test
//! binary), and the output shows the look's background color -- which the
//! pixel checks below read back through `screenshot --no-cursor`.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, applies take seconds at most, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(60);

/// A hermetic `fc-match`, as in `look_apply_cli.rs`: answers the test
/// looks' families with a real font file, everything else the way real
/// `fc-match` answers a missing family.
fn stub_fc_match_dir(scratch: &Scratch) -> PathBuf {
    let font = real_font_file();
    let dir = scratch.dir.join("bin");
    std::fs::create_dir_all(&dir).expect("a stub bin dir");
    let stub = dir.join("fc-match");
    std::fs::write(
        &stub,
        format!(
            r#"#!/bin/sh
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

/// A real, loadable font file for the stub to answer: first the host
/// `fc-match`, then a direct scan of the usual font roots.
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
    let mut roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/X11/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".local/share/fonts"));
        roots.push(home.join(".fonts"));
    }
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

/// A test look with no `[wallpaper]`: the output shows `background`, with
/// nothing handed to scootbg.
fn look_toml(name: &str, background: &str, ring_width: i64) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "An IPC integration test look."
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
ring-width = {ring_width}

[cursor]
theme = "Mocu"
size = 24
"##
    )
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "scoot-look-ipc-{tag}-{}-{}",
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
    _cache: PathBuf,
    _config: PathBuf,
    _home: PathBuf,
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
        let state_home = scratch.sub("state-home");
        let shipped = scratch.dir.join("shipped");
        std::fs::create_dir_all(&shipped).expect("a shipped dir");
        for (name, background, ring) in [("look-a", "#2B3648", 2), ("look-b", "#3A2530", 3)] {
            let dir = shipped.join(name);
            std::fs::create_dir_all(&dir).expect("a look dir");
            std::fs::write(dir.join("look.toml"), look_toml(name, background, ring))
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
            // The session's own catalog, PATH and bus: the compositor's
            // `looks` answer and the `look apply` children it spawns read
            // the same catalog and stub fonts, and the toolkit step notes.
            .env("SCOOT_LOOKS_DIR", &shipped)
            .env("PATH", &path_env)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .expect("a headless scoot");
        let mut session = Self {
            child,
            socket,
            runtime,
            _cache: cache,
            _config: config,
            _home: home,
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
    /// under the runtime dir. A headless session always keeps one, so
    /// swaps never touch the user state below.
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

    fn msg(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("PATH", &self.path_env)
            .env("SCOOT_SOCKET", &self.socket)
            .output()
            .expect("scoot msg runs")
    }

    fn check_msg(&self, args: &[&str]) -> serde_json::Value {
        let out = self.msg(args);
        assert!(
            out.status.success(),
            "`scoot msg {}` failed:\nstdout:\n{}\nstderr:\n{}\nlog:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            self.log_tail(),
        );
        serde_json::from_slice(&out.stdout).expect("JSON stdout")
    }

    /// One raw request over the socket, for shapes `scoot msg` never
    /// sends (`for_ms`, a path as the name).
    fn raw(&self, line: &str) -> serde_json::Value {
        let mut stream = UnixStream::connect(&self.socket).expect("an IPC connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read timeout");
        stream
            .write_all(line.as_bytes())
            .expect("a written request");
        stream.write_all(b"\n").expect("a newline");
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        reader.read_line(&mut reply).expect("a reply line");
        serde_json::from_str(&reply).expect("a JSON reply")
    }

    /// The screen's center pixel, after the frame settles.
    fn center_pixel(&self, tag: &str) -> (u8, u8, u8) {
        let shot = self.dir_shot(tag);
        let out = self.msg(&["wait-idle", "--quiet-ms", "200"]);
        assert!(
            out.status.success(),
            "wait-idle failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = self.msg(&["screenshot", "--no-cursor", "--out", &shot]);
        assert!(
            out.status.success(),
            "screenshot failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        read_center_pixel(Path::new(&shot))
    }

    fn dir_shot(&self, tag: &str) -> String {
        self.runtime
            .join(format!("shot-{tag}.png"))
            .display()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// Whether a `scoot look apply --from-compositor` child runs anywhere:
/// the compositor spawns one per IPC swap, so a visible one proves a swap
/// is in flight. Reads /proc directly (Linux-only, like this file): no
/// subprocess, no buffering, no new dependency.
fn apply_child_running() -> bool {
    let proc = std::fs::read_dir("/proc");
    let entries = match proc {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        // NUL-separated argv: `scoot look apply <name> --from-compositor`.
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        if String::from_utf8_lossy(&cmdline).contains("--from-compositor") {
            return true;
        }
    }
    false
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    assert_eq!(hex.len(), 6, "a #rrggbb color");
    (
        u8::from_str_radix(&hex[0..2], 16).expect("red"),
        u8::from_str_radix(&hex[2..4], 16).expect("green"),
        u8::from_str_radix(&hex[4..6], 16).expect("blue"),
    )
}

fn read_center_pixel(path: &Path) -> (u8, u8, u8) {
    let file = std::fs::File::open(path).expect("a screenshot file");
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().expect("a PNG reader");
    let mut buf = vec![0; reader.output_buffer_size().expect("a PNG buffer size")];
    let info = reader.next_frame(&mut buf).expect("a PNG frame");
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => panic!("an RGB(A) screenshot, not {other:?}"),
    };
    let (x, y) = (width / 2, height / 2);
    let at = (y * width + x) * channels;
    (buf[at], buf[at + 1], buf[at + 2])
}

#[test]
fn looks_lists_the_catalog_and_reports_no_active_choice() {
    let scratch = Scratch::new("looks");
    let session = Session::start(&scratch);
    let reply = session.check_msg(&["looks"]);
    assert_eq!(reply["type"], "looks");
    let names: Vec<&str> = reply["looks"]
        .as_array()
        .expect("a looks list")
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name"))
        .collect();
    assert!(
        names.contains(&"look-a"),
        "the catalog lists look-a: {reply}"
    );
    assert!(
        names.contains(&"look-b"),
        "the catalog lists look-b: {reply}"
    );
    let shipped = std::fs::canonicalize(&session.shipped).expect("a canonical shipped dir");
    let entry = reply["looks"]
        .as_array()
        .expect("a looks list")
        .iter()
        .find(|entry| entry["name"] == "look-a")
        .expect("a look-a entry");
    assert!(
        entry["source"]
            .as_str()
            .expect("a source")
            .starts_with(shipped.display().to_string().as_str()),
        "the catalog entry names its source dir: {entry}"
    );
    assert_eq!(reply["active"], serde_json::Value::Null);
    assert_eq!(
        reply["state_dir"].as_str().expect("a state dir"),
        session.private_dir().display().to_string().as_str(),
        "a headless session reports its private dir"
    );
}

#[test]
fn look_apply_swaps_and_answers_when_it_lands() {
    let scratch = Scratch::new("apply");
    let session = Session::start(&scratch);
    let reply = session.check_msg(&["look-apply", "look-a"]);
    assert_eq!(reply["type"], "look_applied");
    assert_eq!(reply["name"], "look-a");
    assert!(
        reply["applied"]
            .as_array()
            .expect("an applied list")
            .iter()
            .any(|app| app == "compositor"),
        "the compositor reloaded: {reply}"
    );
    let active = session.check_msg(&["looks"]);
    assert_eq!(active["active"], "look-a");

    // The reply lands with the swap: ring, background and cursor apply
    // synchronously in the compositor, and with no wallpaper section the
    // output shows the look's background. A settled frame then screenshots
    // it.
    assert_eq!(
        session.center_pixel("a"),
        hex_to_rgb("#2B3648"),
        "the screen shows look-a's background"
    );

    let reply = session.check_msg(&["look-apply", "look-b"]);
    assert_eq!(reply["name"], "look-b");
    assert_eq!(
        session.center_pixel("b"),
        hex_to_rgb("#3A2530"),
        "the screen shows look-b's background"
    );
    let active = session.check_msg(&["looks"]);
    assert_eq!(active["active"], "look-b");

    // The user state is untouched: a headless session swaps on its
    // private dir only.
    assert!(
        !session.state_home.join("scoot").join("look.json").exists(),
        "no look.json in the user state"
    );
    // And the private record marks the compositor-driven provenance.
    let record = std::fs::read_to_string(session.private_dir().join("look.json"))
        .expect("a private look.json");
    assert!(record.contains("\"chosen-by\":\"compositor\""), "{record}");
}

#[test]
fn a_refused_swap_changes_nothing() {
    let scratch = Scratch::new("refused");
    let session = Session::start(&scratch);
    session.check_msg(&["look-apply", "look-a"]);
    // An unknown look refuses with nothing touched.
    let out = session.msg(&["look-apply", "nosuchlook"]);
    assert!(!out.status.success(), "an unknown look refuses");
    assert_eq!(session.check_msg(&["looks"])["active"], "look-a");
    // A path is refused by the compositor itself (the CLI never sends
    // one: it refuses at parse). Raw socket, so this exercises the
    // compositor's gate, not the CLI's.
    let reply = session.raw(r#"{"type":"look_apply","name":"../evil"}"#);
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(session.check_msg(&["looks"])["active"], "look-a");
    // No stray record appeared anywhere: the first swap's record still
    // names look-a.
    let record = std::fs::read_to_string(session.private_dir().join("look.json"))
        .expect("a private look.json");
    assert!(record.contains("\"name\":\"look-a\""), "{record}");
}

#[test]
fn a_second_swap_while_one_runs_is_refused() {
    let scratch = Scratch::new("concurrent");
    let session = Session::start(&scratch);
    let lock = session.private_dir().join("look.lock");
    // Hold the state-dir lock in-process so the first swap's child parks
    // in its lock wait: the swap is in flight (the compositor holds it
    // parked) for as long as this guard lives. The guard drops only after
    // the second request has been answered, so no wall-clock lifetime can
    // expire early on a loaded runner (the old `flock sleep 6` holder did:
    // CI run 40437160156 took 7.0s, the holder had exited, the first swap
    // had completed, and the second was NOT refused). Explicit release via
    // drop, never a kill (killing an external `flock` could orphan its
    // `sleep` still holding the lock).
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .expect("the test holds the state-dir lock");
    rustix::fs::flock(&lock_file, rustix::fs::FlockOperation::LockExclusive)
        .expect("an exclusive state-dir lock");
    // The first swap parks; the second meets the one-swap gate.
    let socket = session.socket.clone();
    let scoot = scoot();
    let first = std::thread::spawn(move || {
        Command::new(&scoot)
            .arg("msg")
            .arg("look-apply")
            .arg("look-a")
            .env("SCOOT_SOCKET", &socket)
            .output()
            .expect("the first look-apply runs")
    });
    // Deterministic gate: the swap's child process exists from the spawn,
    // and parking follows on the same loop thread with nothing blocking
    // between -- so a visible `--from-compositor` child proves the first
    // swap is in flight. No grace sleep: spawning the second `scoot msg`
    // below (process spawn plus IPC connect) takes milliseconds, while the
    // park lands microseconds after the spawn, so the park is set before
    // the second request reaches the gate. (The compositor log would do
    // too, but its file writer may buffer; /proc never does.)
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if apply_child_running() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the first swap never spawned its child; log:\n{}",
            session.log_tail()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = session.msg(&["look-apply", "look-b"]);
    assert!(
        !out.status.success(),
        "a second swap while one runs refuses"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already running"),
        "the refusal names the gate: {stderr}"
    );
    // Release the lock now that the second request has been answered: the
    // first swap completes once its child acquires it.
    drop(lock_file);
    let out = first.join().expect("the first swap answers");
    assert!(
        out.status.success(),
        "the first swap lands after the release: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(reply["name"], "look-a");
    assert_eq!(session.check_msg(&["looks"])["active"], "look-a");
}

#[test]
fn a_timed_preview_reverts_unless_a_newer_choice_lands() {
    let scratch = Scratch::new("preview");
    let session = Session::start(&scratch);
    // No recorded choice yet: a preview refuses before anything spawns.
    let reply = session.raw(r#"{"type":"look_apply","name":"look-a","for_ms":1500}"#);
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(
        session.check_msg(&["looks"])["active"],
        serde_json::Value::Null
    );

    session.check_msg(&["look-apply", "look-a"]);
    // A timed preview swaps, then swaps back: the generation the swap
    // writes guards the revert.
    let reply = session.raw(r#"{"type":"look_apply","name":"look-b","for_ms":1500}"#);
    assert_eq!(reply["type"], "look_applied", "{reply}");
    assert_eq!(reply["name"], "look-b");
    assert_eq!(session.check_msg(&["looks"])["active"], "look-b");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if session.check_msg(&["looks"])["active"] == "look-a" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the preview never reverted; log:\n{}",
            session.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
