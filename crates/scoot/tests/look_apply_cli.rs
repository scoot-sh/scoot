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

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, applies take milliseconds, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(60);

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
mono = "DejaVu Sans Mono"
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
}

impl Session {
    fn start(scratch: &Scratch) -> Self {
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
            if entry.file_name().to_string_lossy().starts_with("scoot-look-") {
                found.push(entry.path());
            }
        }
        assert_eq!(found.len(), 1, "one private dir, not {found:?}");
        found.pop().expect("one private dir")
    }

    /// One `scoot look ...` invocation against this session: the catalog
    /// from `SCOOT_LOOKS_DIR`, the session's private state and socket, a
    /// neutralized settings bus (the toolkit step notes, never writes).
    fn look(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("look")
            .args(args)
            .arg("--state")
            .arg(self.private_dir())
            .arg("--socket")
            .arg(&self.socket)
            .env("SCOOT_LOOKS_DIR", &self.shipped)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .output()
            .expect("scoot look runs")
    }

    fn msg(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("SCOOT_SOCKET", &self.socket)
            .output()
            .expect("scoot msg runs")
    }

    fn check_look(&self, args: &[&str]) -> String {
        let out = self.look(args);
        assert(
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
    assert(list.contains("look-a"), "the catalog lists look-a:\n{list}");
    assert(list.contains("look-b"), "the catalog lists look-b:\n{list}");
    let show = session.check_look(&["show", "look-a", "--json"]);
    let show: serde_json::Value = serde_json::from_str(&show).expect("JSON from show");
    assert_eq!(show["name"], "look-a");
    assert_eq!(show["palette"]["background"], "#2B3648");
    session.check_look(&["validate", "look-a"]);
    let dir = session.shipped.join("look-a");
    session.check_look(&["validate", &dir.to_string_lossy()]);
    let out = scratch.dir.join("built");
    session.check_look(&["build", "look-a", "--out", &out.to_string_lossy()]);
    for file in ["scoot.toml", "bar.toml", "render.json"] {
        assert(
            out.join(file).is_file(),
            "build writes {file}",
        );
    }
    let current = session.check_look(&["current", "--json"]);
    let current: serde_json::Value = serde_json::from_str(&current).expect("JSON from current");
    assert_eq!(current["active"], serde_json::Value::Null);
}

#[test]
fn a_swap_applies_two_looks_through_the_scoped_reload() {
    let scratch = Scratch::new("swap");
    let session = Session::start(&scratch);
    let first = session.check_look(&["apply", "look-a", "--json"]);
    let first: serde_json::Value = serde_json::from_str(&first).expect("JSON from apply");
    assert_eq!(first["applied"], "look-a");
    assert_eq!(first["generation"], 1);
    assert(
        first["reloaded"].as_array().expect("a reloaded list").iter().any(|entry| entry == "compositor"),
        "the running compositor reloads: {first}",
    );
    let private = session.private_dir();
    let record = std::fs::read_to_string(private.join("look.json")).expect("a record");
    assert(record.contains("\"name\":\"look-a\"") || record.contains("\"name\": \"look-a\""), "{record}");
    let link = std::fs::read_link(private.join("look")).expect("a link");
    assert(link.is_dir(), "the link points at a render: {}", link.display());
    assert(link.join("scoot.toml").is_file());
    let second = session.check_look(&["apply", "look-b", "--json"]);
    let second: serde_json::Value = serde_json::from_str(&second).expect("JSON from apply");
    assert_eq!(second["applied"], "look-b");
    assert_eq!(second["generation"], 2);
    let current = session.check_look(&["current"]);
    assert(current.contains("look-b"), "current names the swap:\n{current}");
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
    assert(binds.status.success(), "binds answers");
    let before = String::from_utf8(binds.stdout).expect("UTF-8 binds");
    assert(
        !before.contains("super+F12"),
        "the swap applied a pending edit:\n{before}",
    );
    // A plain reload applies what the swap held back.
    let reload = session.msg(&["reload"]);
    assert(reload.status.success(), "a plain reload answers");
    let binds = session.msg(&["binds", "--json"]);
    let after = String::from_utf8(binds.stdout).expect("UTF-8 binds");
    assert(
        after.contains("super+F12"),
        "the plain reload did not apply the pending bind:\n{after}",
    );
}
