//! The style-only reload under a look layer, against a live bar on headless
//! scoot: a look swap of colors alone takes the style path (modules kept,
//! repainted), a swap staging a module-affecting key the allowlist permits
//! takes the full path, a refused plain reload leaves the snapshot, the
//! active flag and `last` untouched, and a style reload with a popup open
//! keeps the popup on its surface at the new style.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// Style reloads keep `exec` children alive, so this file exists where
// `exec` does (dev/research/scootbar-testing.md: the feature matrix).
#![cfg(feature = "exec")]

mod common;
#[cfg(all(feature = "volume", feature = "popup"))]
mod pulse;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Reaper, Session, rgb};
use serde_json::Value;

const BAR: &str = "#102030";
const NEW: &str = "#ff0000";
const LAYER_NEW: &str = "#00c000";
const HEIGHT: u32 = 28;

fn out_table(dir: &Path) -> String {
    format!(
        "[exec.out]\ncommand = [\"sh\", \"-c\", \"echo $$ > {}/pid; echo started; sleep 600\"]\n",
        dir.display()
    )
}

fn extra_table(dir: &Path) -> String {
    format!(
        "[exec.extra]\ncommand = [\"sh\", \"-c\", \"echo $$ > {}/extra-pid; echo extra; sleep 601\"]\n",
        dir.display()
    )
}

fn layer_file(body: &str) -> String {
    body.to_owned()
}

struct Rig {
    session: Session,
    bar: Reaper,
    file: PathBuf,
    layer: PathBuf,
    dir: PathBuf,
}

impl Rig {
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
fn a_look_swap_of_colors_only_keeps_modules_and_repaints() {
    let Some(session) = Session::scoot(&common::scootbar_bin(), "look-style-colors", 1, "") else {
        return;
    };
    let dir = session.runtime_dir().join("out");
    fs::create_dir_all(&dir).unwrap();
    let file = session.runtime_dir().join("bar.toml");
    let layer = session.runtime_dir().join("layer.toml");
    fs::write(&layer, "[colors]\nbackground = \"#102030\"\n").unwrap();
    fs::write(
        &file,
        format!(
            "right = [\"out\"]\n[bar]\nheight = {HEIGHT}\n{}\n[look]\nlayer = \"{}\"\n",
            out_table(&dir),
            layer.display()
        ),
    )
    .unwrap();
    let bar = Reaper(session.bar(&["--config", file.to_str().unwrap()]));
    let mut rig = Rig {
        session,
        bar,
        file,
        layer,
        dir,
    };
    rig.wait_color(BAR, "the layered background drawn");
    rig.wait_text("started");
    let pid = rig.pid();
    // The layer swaps colors alone: the scoped reload takes the style path,
    // the child survives and the new style is on screen.
    fs::write(
        &rig.layer,
        layer_file("[colors]\nbackground = \"#ff0000\"\n"),
    )
    .unwrap();
    let out = rig.msg(&["reload", "--look"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(rig.pid(), pid, "no second child was started");
    let queried = rig.query();
    assert_eq!(queried["modules"].as_array().unwrap().len(), 1);
    assert_eq!(queried["modules"][0]["text"], "started");
    rig.wait_color(NEW, "the swapped background");
    rig.alive();
    rig.clean();
}

/// A look swap staging a module-affecting key the allowlist permits (the
/// clock's icon) takes the full path: the icon is live afterwards, which a
/// style-only reload could never apply (it restarts no module).
#[test]
#[cfg(feature = "clock")]
fn a_look_swap_with_a_module_key_takes_the_full_path() {
    let Some(session) = Session::scoot(&common::scootbar_bin(), "look-style-module-key", 1, "") else {
        return;
    };
    let dir = session.runtime_dir().join("out");
    fs::create_dir_all(&dir).unwrap();
    let file = session.runtime_dir().join("bar.toml");
    let layer = session.runtime_dir().join("layer.toml");
    fs::write(&layer, "[colors]\nbackground = \"#102030\"\n").unwrap();
    let tables = format!("{}\n[clock]\nformat = \"%H\"\n", out_table(&dir));
    fs::write(
        &file,
        format!(
            "left = [\"clock\"]\nright = [\"out\"]\n[bar]\nheight = {HEIGHT}\n{tables}\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    )
    .unwrap();
    let bar = Reaper(session.bar(&["--config", file.to_str().unwrap()]));
    let mut rig = Rig {
        session,
        bar,
        file,
        layer,
        dir,
    };
    rig.wait_color(BAR, "the layered background drawn");
    rig.wait_text("started");
    let pid = rig.pid();
    let before = rig.query();
    assert!(
        before["modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|module| module["id"] == "clock" && module["icon"].is_null()),
        "no clock icon before the swap: {before}"
    );
    // Colors plus the clock's icon: not style alone, so the full path, and
    // the icon is live. The kept `exec` child survives the full reload.
    fs::write(
        &rig.layer,
        layer_file("[colors]\nbackground = \"#ff0000\"\n[clock]\nicon = \"X\"\n"),
    )
    .unwrap();
    let out = rig.msg(&["reload", "--look"]);
    assert!(out.status.success(), "{out:?}");
    rig.session
        .wait_for(&mut rig.bar.0, "the swapped clock icon", |session| {
            let queried = session.scootbar().arg("msg").arg("query").output().unwrap();
            assert!(queried.status.success(), "{queried:?}");
            let queried: Value = serde_json::from_slice(&queried.stdout).unwrap();
            queried["modules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|module| module["id"] == "clock" && module["icon"] == "X")
                .then_some(())
        });
    assert_eq!(rig.pid(), pid, "the kept child survived the full reload");
    rig.wait_color(NEW, "the swapped background");
    rig.alive();
    rig.clean();
}

/// A plain reload refused at apply leaves the snapshot, the active flag
/// and `last` untouched: a scoped reload still merges under the old
/// snapshot (the refused file's extra module never appears), and the next
/// good plain reload still routes on the running bar (the refused file's
/// extra module starts on the full path, not the fast one).
#[test]
fn a_refused_plain_reload_leaves_snapshot_active_and_last_untouched() {
    let Some(session) = Session::scoot(&common::scootbar_bin(), "look-style-refused", 1, "") else {
        return;
    };
    let dir = session.runtime_dir().join("out");
    fs::create_dir_all(&dir).unwrap();
    let file = session.runtime_dir().join("bar.toml");
    let layer = session.runtime_dir().join("layer.toml");
    fs::write(&layer, "[colors]\nbackground = \"#102030\"\n").unwrap();
    let tables = out_table(&dir);
    // The user file sets no background: the background is the layer's, so
    // the snapshot is exactly this file.
    fs::write(
        &file,
        format!(
            "right = [\"out\"]\n[bar]\nheight = {HEIGHT}\n{tables}\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    )
    .unwrap();
    let bar = Reaper(session.bar(&["--config", file.to_str().unwrap()]));
    let mut rig = Rig {
        session,
        bar,
        file,
        layer,
        dir,
    };
    rig.wait_color(BAR, "the layered background drawn");
    rig.wait_text("started");
    let pid = rig.pid();
    // Refused: a second module and a new background, but a fallback font
    // that is no font. The layer moves too, so a committed snapshot would
    // show it.
    let refused_tables = format!("{}{}", out_table(&rig.dir), extra_table(&rig.dir));
    fs::write(
        &rig.file,
        format!(
            "right = [\"out\", \"extra\"]\n[bar]\nheight = {HEIGHT}\nfallback-fonts = [\"/nonexistent/scootbar-test-font.ttf\"]\n[colors]\nbackground = \"{NEW}\"\n{refused_tables}\n[look]\nlayer = \"{}\"\n",
            rig.layer.display()
        ),
    )
    .unwrap();
    fs::write(
        &rig.layer,
        layer_file("[colors]\nbackground = \"#00c000\"\n"),
    )
    .unwrap();
    let out = rig.msg(&["reload"]);
    assert!(
        !out.status.success(),
        "a missing fallback refuses the reload"
    );
    assert!(
        !rig.dir.join("extra-pid").exists(),
        "no refused module started"
    );
    assert_eq!(rig.pid(), pid, "the running child is untouched");
    // The snapshot and the active flag stand: a scoped reload merges the
    // new layer under the OLD user file (one module, good font), so it
    // succeeds and the refused file's `extra` module never appears.
    let out = rig.msg(&["reload", "--look"]);
    assert!(out.status.success(), "{out:?}");
    let queried = rig.query();
    assert_eq!(
        queried["modules"].as_array().unwrap().len(),
        1,
        "the scoped reload merged under the old snapshot: {queried}"
    );
    rig.wait_color(LAYER_NEW, "the swapped layer under the old snapshot");
    assert_eq!(rig.pid(), pid, "the running child is untouched");
    // And `last` stands: the same change with a good font still routes on
    // the running bar, so the added module list takes the full reload and
    // `extra` starts. Had the refused config replaced `last`, only the
    // fallback would differ and the fast path would leave `extra`
    // unstarted.
    fs::write(
        &rig.file,
        format!(
            "right = [\"out\", \"extra\"]\n[bar]\nheight = {HEIGHT}\n[colors]\nbackground = \"{NEW}\"\n{refused_tables}\n[look]\nlayer = \"{}\"\n",
            rig.layer.display()
        ),
    )
    .unwrap();
    let out = rig.msg(&["reload"]);
    assert!(out.status.success(), "{out:?}");
    rig.session
        .wait_for(&mut rig.bar.0, "the added module", |session| {
            let queried = session.scootbar().arg("msg").arg("query").output().unwrap();
            assert!(queried.status.success(), "{queried:?}");
            let queried: Value = serde_json::from_slice(&queried.stdout).unwrap();
            queried["modules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|module| module["id"] == "extra")
                .then_some(())
        });
    assert_eq!(rig.pid(), pid, "the kept child survived the full reload");
    rig.wait_color(NEW, "the reloaded background");
    rig.alive();
    rig.clean();
}

/// A style reload with a popup open under a look layer: the popup stays
/// open on its surface at the new style (colors alone still fit, so no
/// second `get_popup`), and the bar stays alive with no protocol error.
#[test]
#[cfg(all(feature = "volume", feature = "popup"))]
fn a_style_reload_with_a_popup_open_under_a_look_layer() {
    use std::time::Duration;
    const BAR_BG: &str = "#102030";
    const FG: &str = "#f0f0f0";
    const POP_HEIGHT: u32 = 40;
    const MODULE: (u32, u32) = (800, 20);
    let Some(session) = Session::scoot(&common::scootbar_bin(), "look-style-popup", 1, "") else {
        return;
    };
    let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
    let file = session.runtime_dir().join("bar.toml");
    let layer = session.runtime_dir().join("layer.toml");
    fs::write(&layer, "[colors]\nbackground = \"#102030\"\n").unwrap();
    fs::write(
        &file,
        format!(
            "[bar]\nheight = {POP_HEIGHT}\nfont-size = 20\ntooltip-delay = 0\n[colors]\nforeground = \"{FG}\"\n[volume]\non-click = \"popup\"\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    )
    .unwrap();
    let address = server.address();
    let bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap(), "--center", "volume"],
        &[("PULSE_SERVER", address.as_str()), ("WAYLAND_DEBUG", "1")],
    ));
    let mut rig = VolumeRig {
        session,
        bar,
        server,
        file,
        layer,
    };
    rig.wait_shown(POP_HEIGHT);
    rig.click(MODULE);
    rig.wait_popup(true, BAR_BG, POP_HEIGHT, MODULE, "the popup shown");
    assert_eq!(popups_in(&rig.session.bar_stderr()), 1);
    // Colors alone under the look layer: the scoped reload restyles the
    // open popup in place — still open, no remake, painting the new style.
    fs::write(&rig.layer, "[colors]\nbackground = \"#ff0000\"\n").unwrap();
    let out = rig.msg(&["reload", "--look"]);
    assert!(out.status.success(), "{out:?}");
    rig.wait_popup(true, NEW, POP_HEIGHT, MODULE, "the popup restyled");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        popups_in(&rig.session.bar_stderr()),
        1,
        "a colors-only restyle keeps the popup's surface"
    );
    rig.alive();
    rig.clean();
}

#[cfg(all(feature = "volume", feature = "popup"))]
fn popups_in(trace: &str) -> usize {
    trace
        .lines()
        .filter(|line| line.contains("xdg_surface@") && line.contains(".get_popup("))
        .count()
}

#[cfg(all(feature = "volume", feature = "popup"))]
struct VolumeRig {
    session: Session,
    bar: Reaper,
    #[allow(dead_code)]
    server: pulse::Server,
    #[allow(dead_code)]
    file: PathBuf,
    layer: PathBuf,
}

#[cfg(all(feature = "volume", feature = "popup"))]
impl VolumeRig {
    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    fn shot(&self) -> common::Shot {
        self.session.scoot_screenshot(1)
    }

    fn wait_shown(&mut self, height: u32) {
        let y = height / 2;
        self.session
            .wait_for(&mut self.bar.0, "the volume module shown", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600)
                    .any(|x| shot.at(x, y) == rgb("#f0f0f0"))
                    .then_some(())
            });
    }

    fn ipc(&self, request: &str) {
        let reply = self.session.scoot_ipc(request);
        assert_eq!(reply["type"], "ok", "{request}: {reply}");
    }

    fn click(&self, (x, y): (u32, u32)) {
        self.ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
        ));
    }

    fn popup_shown(&self, background: &str, height: u32, module: (u32, u32)) -> bool {
        self.shot().at(module.0, height + 2) == rgb(background)
    }

    fn wait_popup(
        &mut self,
        shown: bool,
        background: &str,
        height: u32,
        module: (u32, u32),
        what: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            if self.popup_shown(background, height, module) == shown {
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("never {what}; scootbar said: {}", self.session.bar_stderr());
            }
            self.alive();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
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
