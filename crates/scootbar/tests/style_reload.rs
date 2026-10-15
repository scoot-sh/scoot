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
