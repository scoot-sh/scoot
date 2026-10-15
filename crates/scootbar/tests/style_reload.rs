//! The style-only reload against a live bar on headless scoot: a
//! colors-only reload keeps every module running (the `exec` child's pid is
//! unchanged) and repaints at once, while a refused reload — style or full
//! — changes nothing, and what follows it still routes on the running bar.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// Style reloads keep `exec` children alive, so this file exists where
// `exec` does (dev/research/scootbar-testing.md: the feature matrix).
#![cfg(feature = "exec")]

mod common;

use std::fs;
use std::path::PathBuf;
#[cfg(feature = "network")]
use std::process::{Command, Stdio};

use common::{Reaper, Session, rgb};
use serde_json::Value;

const BAR: &str = "#102030";
const NEW: &str = "#ff0000";
const HEIGHT: u32 = 28;

/// The bar's config: `lists` placing the named `exec` modules, `bar_extra`
/// keys under `[bar]`, on `background`.
fn config(lists: &str, tables: &str, bar_extra: &str, background: &str) -> String {
    format!(
        "{lists}\n[bar]\nheight = {HEIGHT}\n{bar_extra}\n[colors]\nbackground = \"{background}\"\n{tables}"
    )
}

fn out_table(dir: &std::path::Path) -> String {
    format!(
        "[exec.out]\ncommand = [\"sh\", \"-c\", \"echo $$ > {}/pid; echo started; sleep 600\"]\n",
        dir.display()
    )
}

fn extra_table(dir: &std::path::Path) -> String {
    format!(
        "[exec.extra]\ncommand = [\"sh\", \"-c\", \"echo $$ > {}/extra-pid; echo extra; sleep 601\"]\n",
        dir.display()
    )
}

struct Rig {
    session: Session,
    bar: Reaper,
    file: PathBuf,
    dir: PathBuf,
}

impl Rig {
    fn start(tag: &str) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let dir = session.runtime_dir().join("out");
        fs::create_dir_all(&dir).unwrap();
        let file = session.runtime_dir().join("bar.toml");
        let tables = out_table(&dir);
        fs::write(&file, config("right = [\"out\"]\n", &tables, "", BAR)).unwrap();
        let bar = Reaper(session.bar_with_env(&["--config", file.to_str().unwrap()], &[]));
        let mut rig = Self {
            session,
            bar,
            file,
            dir,
        };
        rig.wait_color(BAR, "the bar drawn");
        rig.wait_text("started");
        Some(rig)
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    fn query(&self) -> Value {
        let out = self.msg(&["query"]);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn wait_color(&mut self, color: &str, what: &str) {
        let want = rgb(color);
        self.session.wait_for(&mut self.bar.0, what, |session| {
            (session.scoot_screenshot(1).at(5, 5) == want).then_some(())
        });
    }

    /// Until the `out` module's shown text reads `text`.
    fn wait_text(&mut self, text: &str) {
        self.session
            .wait_for(&mut self.bar.0, "the command's line shown", |session| {
                let queried = session.scootbar().arg("msg").arg("query").output().unwrap();
                assert!(queried.status.success(), "{queried:?}");
                let queried: Value = serde_json::from_slice(&queried.stdout).unwrap();
                let modules = queried["modules"].as_array().unwrap();
                modules
                    .iter()
                    .any(|module| module["id"] == "out" && module["text"] == text)
                    .then_some(())
            });
    }

    fn pid(&self) -> u32 {
        fs::read_to_string(self.dir.join("pid"))
            .expect("the command wrote its pid")
            .trim()
            .parse()
            .expect("a pid")
    }

    fn alive(&mut self) {
        assert!(
            self.bar.0.try_wait().unwrap().is_none(),
            "scootbar exited: {}",
            self.session.bar_stderr()
        );
    }

    fn clean(&self) {
        let log = self.session.bar_stderr();
        assert!(!log.contains("error"), "{log}");
        assert!(!log.contains("protocol"), "{log}");
    }
}

#[test]
fn a_colors_only_reload_keeps_the_child_and_repaints_at_once() {
    let Some(mut rig) = Rig::start("style-keeps") else {
        return;
    };
    let pid = rig.pid();
    fs::write(
        &rig.file,
        config("right = [\"out\"]\n", &out_table(&rig.dir), "", NEW),
    )
    .unwrap();
    let reloaded = rig.msg(&["reload"]);
    assert!(reloaded.status.success(), "{reloaded:?}");
    // The same child, still showing its line: no module restarted.
    assert_eq!(rig.pid(), pid, "no second child was started");
    let queried = rig.query();
    assert_eq!(queried["modules"].as_array().unwrap().len(), 1);
    assert_eq!(queried["modules"][0]["text"], "started");
    // And the new style is on screen: the reload repainted.
    rig.wait_color(NEW, "the reloaded background");
    rig.alive();
    rig.clean();
}

#[test]
fn a_refused_reload_changes_nothing_and_the_next_one_routes_on_the_running_bar() {
    let Some(mut rig) = Rig::start("style-refused") else {
        return;
    };
    let pid = rig.pid();
    // A reload that is both structural (a second module) and unusable (a
    // fallback font that is no font): refused with the running bar
    // untouched — and, what this pins, with `last` untouched too, so the
    // reload after it still routes on the running bar rather than on this
    // refused one.
    let tables = format!("{}{}", out_table(&rig.dir), extra_table(&rig.dir));
    let refused = config(
        "right = [\"out\", \"extra\"]\n",
        &tables,
        "fallback-fonts = [\"/nonexistent/scootbar-test-font.ttf\"]\n",
        NEW,
    );
    fs::write(&rig.file, refused).unwrap();
    let out = rig.msg(&["reload"]);
    assert!(
        !out.status.success(),
        "a missing fallback refuses the reload"
    );
    // Nothing started, nothing moved, nothing repainted.
    assert!(
        !rig.dir.join("extra-pid").exists(),
        "no refused module started"
    );
    assert_eq!(rig.pid(), pid, "the running child is untouched");
    let queried = rig.query();
    assert_eq!(queried["modules"].as_array().unwrap().len(), 1);
    assert_eq!(queried["modules"][0]["text"], "started");
    rig.wait_color(BAR, "the background stands");
    // The same change with a good font: `last` is still the running bar,
    // so the added module list takes the full reload and the new module
    // starts. Had the refused config replaced `last`, only the fallback
    // would differ and the fast path would leave `extra` unstarted.
    fs::write(
        &rig.file,
        config("right = [\"out\", \"extra\"]\n", &tables, "", NEW),
    )
    .unwrap();
    let reloaded = rig.msg(&["reload"]);
    assert!(reloaded.status.success(), "{reloaded:?}");
    rig.wait_color(NEW, "the reloaded background");
    let queried = rig.query();
    let modules = queried["modules"].as_array().unwrap();
    assert_eq!(
        modules.len(),
        2,
        "the full reload started `extra`: {queried}"
    );
    assert_eq!(rig.pid(), pid, "the kept child survived the full reload");
    rig.alive();
    rig.clean();
}

/// A reload retries a module left out as unavailable: the bar runs in a
/// user and network namespace with only `lo`, so the network module is
/// unavailable at start; an interface appears later, and a colors-only
/// reload picks it up. Without the retry the same file stays style-only
/// and the module stays absent.
#[test]
#[cfg(feature = "network")]
fn a_reload_retries_a_module_left_out_as_unavailable() {
    fn tool(name: &str, args: &[&str]) -> bool {
        Command::new(name)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    if !tool("unshare", &["-Urn", "true"]) || !tool("ip", &["-V"]) {
        eprintln!("skipped -- no unshare/ip for the namespace retry test");
        return;
    }
    let Some(session) = Session::scoot("style-retry", 1, "") else {
        return;
    };
    let dir = session.runtime_dir().join("out");
    fs::create_dir_all(&dir).unwrap();
    let file = session.runtime_dir().join("bar.toml");
    let lists = "left = [\"network\"]\nright = [\"out\"]\n";
    let tables = out_table(&dir);
    fs::write(&file, config(lists, &tables, "", BAR)).unwrap();
    // The wrapper runs the daemon in a user and network namespace: only
    // `lo`, down, so `network` starts unavailable. A reader inside the
    // namespace waits on a fifo and brings up a `dummy0` when the test
    // writes to it (`nsenter -U` cannot enter the namespace: setgroups is
    // denied — so the trigger comes from inside).
    let fifo = session.runtime_dir().join("netns-trigger");
    let ack = session.runtime_dir().join("netns-ack");
    let inner = session.runtime_dir().join("netns-inner.sh");
    fs::write(
        &inner,
        format!(
            "#!/bin/sh\n( read _ < \"{fifo}\" && ip link add dummy0 type dummy && ip link set dummy0 up && echo up > \"{ack}\" ) &\nexec {bin} \"$@\"\n",
            fifo = fifo.display(),
            ack = ack.display(),
            bin = common::scootbar_bin().display()
        ),
    )
    .unwrap();
    let wrapper = session.runtime_dir().join("netns-bar.sh");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec unshare -Urn sh {} \"$@\"\n",
            inner.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    for script in [&inner, &wrapper] {
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("mkfifo").arg(&fifo).output().unwrap();
    assert!(
        out.status.success(),
        "mkfifo: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = fs::File::create(session.bar_log()).unwrap();
    let mut daemon = Command::new(&wrapper);
    daemon.arg("daemon").arg("--font").arg(session.font());
    daemon.args(["--config", file.to_str().unwrap()]);
    daemon.env("WAYLAND_DISPLAY", &session.wayland_display);
    daemon.env("XDG_RUNTIME_DIR", session.runtime_dir());
    session.scratch.sandbox(&mut daemon);
    daemon
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    let mut bar = Reaper(daemon.spawn().unwrap());
    let query = || {
        let out = session.scootbar().arg("msg").arg("query").output().unwrap();
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let pid = || {
        fs::read_to_string(dir.join("pid"))
            .expect("the command wrote its pid")
            .trim()
            .parse::<u32>()
            .expect("a pid")
    };
    // The bar is up with its `exec` line, but no network module.
    session.wait_for(&mut bar.0, "the bar drawn", |session| {
        (session.scoot_screenshot(1).at(5, 5) == rgb(BAR)).then_some(())
    });
    session.wait_for(&mut bar.0, "the command's line shown", |_| {
        query()["modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|module| module["id"] == "out" && module["text"] == "started")
            .then_some(())
    });
    assert!(
        query()["modules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|module| module["id"] != "network"),
        "the network module started with only lo"
    );
    assert!(
        session.bar_stderr().contains("left out"),
        "no left-out note: {}",
        session.bar_stderr()
    );
    let kept = pid();
    // An interface appears in the bar's namespace; only then does the
    // reload run, so the retry either sees it or it is a real failure.
    fs::write(&fifo, "go\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if fs::read_to_string(&ack).is_ok_and(|text| text.trim() == "up") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the interface never came up: {}",
            session.bar_stderr()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Colors only otherwise: the retry takes the full path and the network
    // module starts; the kept `exec` child survives it.
    fs::write(&file, config(lists, &tables, "", NEW)).unwrap();
    let out = session
        .scootbar()
        .arg("msg")
        .args(["reload"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    session.wait_for(&mut bar.0, "the retried network module", |_| {
        query()["modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|module| module["id"] == "network")
            .then_some(())
    });
    assert_eq!(pid(), kept, "the kept child survived the retrying reload");
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "scootbar exited: {}",
        session.bar_stderr()
    );
}
