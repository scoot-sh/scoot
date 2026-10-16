//! The looks picker against a live bar on headless scoot: the bar shows the
//! recorded look, a `scoot look apply` from outside moves it through the
//! look-scoped reload, and picking through the bar (`invoke looks popup`,
//! then `invoke looks "apply N"`) runs a real `scoot look apply` that swaps
//! the desktop and moves the bar text.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// The bar text, the picker and the staged `scoot look apply` all belong to
// the `looks` module with popups (dev/research/scootbar-testing.md: the
// feature matrix).
#![cfg(all(feature = "looks", feature = "popup"))]

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use common::{Reaper, Session};
use serde_json::Value;

const HEIGHT: u32 = 28;

fn look_toml(name: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
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
radius = 0
ring-width = 2

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "#2B3648"
mode = "fill"
"##
    )
}

fn bar_toml() -> String {
    format!(
        "[bar]\nheight = {HEIGHT}\nfont-size = 20\n\
         [looks]\non-click = \"popup\"\n"
    )
}

/// The `scoot` binary beside the test's `scootbar`: what the picker's
/// staged `scoot look apply` resolves through PATH.
fn scoot_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("SCOOTBAR_TEST_SCOOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        return path;
    }
    let sibling = PathBuf::from(env!("CARGO_BIN_EXE_scootbar")).with_file_name("scoot");
    assert!(
        sibling.is_file(),
        "no scoot binary beside {} (build `-p scoot`, or set SCOOTBAR_TEST_SCOOT)",
        env!("CARGO_BIN_EXE_scootbar")
    );
    sibling
}

struct Rig {
    session: Session,
    bar: Reaper,
    scoot: PathBuf,
    looks_dir: PathBuf,
    /// The headless session's private look state: the one `scoot-look-*`
    /// dir in its runtime dir. The CLI targets it with `--state`; the bar
    /// reads it through `SCOOT_LOOK_STATE`.
    state: PathBuf,
    path: String,
}

impl Rig {
    fn start(tag: &str) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let scoot = scoot_bin();
        let bin_dir = scoot.parent().unwrap().to_owned();
        // Two source looks, cataloged through `SCOOT_LOOKS_DIR`.
        let looks_dir = session.scratch.0.join("looks");
        for name in ["look-a", "look-b"] {
            let dir = looks_dir.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("look.toml"), look_toml(name)).unwrap();
        }
        // The session's private look state: `--headless` always keeps one,
        // seeded from the (empty) user state. Without `--state` the CLI
        // would act on the developer's real look.
        let mut state = None;
        for entry in fs::read_dir(&session.scratch.0).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap_or_default();
            if name.starts_with("scoot-look-") && entry.file_type().unwrap().is_dir() {
                assert!(state.is_none(), "one private look state, not two");
                state = Some(entry.path());
            }
        }
        let state = state.expect("the headless session seeds a private look state");
        // Hermetic fonts: the runner may have no Nerd Fonts, and the real
        // `fc-match` would resolve the test families to whatever it has.
        // A PATH-prepended stub answers the test families with the test
        // font file (which really loads: the bar's `daemon --check` opens
        // it) and their own names, so the production family comparison
        // still passes. Every other family resolves the way real `fc-match`
        // answers a missing one.
        let font_file = session.font();
        let stub_dir = session.scratch.0.join("stub-bin");
        fs::create_dir_all(&stub_dir).unwrap();
        fs::write(
            stub_dir.join("fc-match"),
            format!(
                "#!/bin/sh\n\
                 fmt=\"${{1:-}}\"\n\
                 fam=\"${{2:-}}\"\n\
                 case \"$fam\" in\n\
                 \"DejaVu Sans\"|\"DejaVu Sans Mono\")\n\
                 case \"$fmt\" in *file*) printf '%s\\n' \"{}\" ;; *) printf '%s\\n' \"$fam\" ;; esac ;;\n\
                 *)\n\
                 case \"$fmt\" in *file*) printf '%s\\n' \"{}\" ;; *) printf 'DejaVu Sans\\n' ;; esac ;;\n\
                 esac\n",
                font_file.display(),
                font_file.display(),
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(stub_dir.join("fc-match"), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let file = session.scratch.0.join("bar.toml");
        fs::write(&file, bar_toml()).unwrap();
        let mut path = stub_dir.into_os_string();
        path.push(":");
        path.push(&bin_dir.into_os_string());
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        let path = path.into_string().unwrap();
        let bar = Reaper(session.bar_with_env(
            &["--config", file.to_str().unwrap(), "--center", "looks"],
            &[
                ("SCOOT_LOOKS_DIR", looks_dir.to_str().unwrap()),
                ("SCOOT_LOOK_STATE", state.to_str().unwrap()),
                ("PATH", &path),
            ],
        ));
        let mut rig = Self {
            session,
            bar,
            scoot,
            looks_dir,
            state,
            path,
        };
        rig.wait_text("no look", "the bar starts with no look");
        Some(rig)
    }

    /// The sandboxed environment the bar runs in: the CLI needs the same
    /// state, runtime and catalog to act on the same session.
    fn cli(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(&self.scoot);
        command
            .args(args)
            .arg("--state")
            .arg(&self.state)
            .env("XDG_RUNTIME_DIR", &self.session.scratch.0)
            .env("XDG_CONFIG_HOME", self.session.scratch.config_home())
            .env("XDG_STATE_HOME", self.session.scratch.state_home())
            .env("XDG_CACHE_HOME", self.session.scratch.cache_home())
            .env("SCOOT_LOOKS_DIR", &self.looks_dir)
            .env("SCOOT_SOCKET", self.session.scratch.0.join("scoot.sock"))
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    fn msg_json(&self, args: &[&str]) -> Value {
        let out = self.msg(args);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn wait_text(&mut self, text: &str, what: &str) {
        // No assert inside: a `msg` that fails (no socket yet) is `None,
        // retried; a dead bar fails through `try_wait` with its stderr.
        self.session.wait_for(&mut self.bar.0, what, |session| {
            let queried = session.scootbar().arg("msg").arg("query").output().unwrap();
            if !queried.status.success() {
                return None;
            }
            let queried: Value = serde_json::from_slice(&queried.stdout).ok()?;
            queried["modules"]
                .as_array()?
                .iter()
                .any(|module| module["id"] == "looks" && module["text"] == text)
                .then_some(())
        });
    }

    /// Opens the picker (which fills its snapshot) and reports the query
    /// after it: the picker's rows live in the `looks` module's `value`.
    fn picker_value(&self) -> Value {
        let opened = self.msg(&["invoke", "looks", "popup"]);
        assert!(opened.status.success(), "{opened:?}");
        self.msg_json(&["query"])
    }
}

#[test]
fn an_outside_apply_moves_the_bar_text() {
    let Some(mut rig) = Rig::start("look-picker-outside") else {
        return;
    };
    let out = rig.cli(&["look", "apply", "look-a"]);
    assert!(out.status.success(), "{out:?}");
    rig.wait_text("look-a", "the bar shows the applied look");
    let current = rig.cli(&["look", "current", "--json"]);
    assert!(current.status.success(), "{current:?}");
    let current: Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(
        current["active"]["name"],
        Value::String("look-a".to_owned())
    );
}

#[test]
fn picking_through_the_bar_applies() {
    let Some(mut rig) = Rig::start("look-picker-pick") else {
        return;
    };
    // Open the picker so it fills its snapshot, then apply its first row.
    let value = rig.picker_value();
    let looks = value["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["id"] == "looks")
        .unwrap();
    let first = looks["value"]["looks"][0].as_str().unwrap().to_owned();
    let out = rig
        .session
        .scootbar()
        .arg("msg")
        .args(["invoke", "looks", "apply", "0"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    rig.wait_text(&first, "the bar shows the picked look");
    // And the recorded choice agrees.
    let current = rig.cli(&["look", "current", "--json"]);
    assert!(current.status.success(), "{current:?}");
    let current: Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(current["active"]["name"], Value::String(first));
}
