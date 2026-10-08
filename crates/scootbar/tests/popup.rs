//! The volume popup on a headless scoot: a click bound to `popup` opens an
//! `xdg_popup` under the module, drawn from the module's own state (a live
//! PulseAudio-protocol server, `pulse/`), a slider drag moves the server's
//! level, and every way out closes it with the bar still alive and no
//! protocol error.
//!
//! The popup is read from the compositor's screenshot (its frame and
//! background are known colors) and the server's own log of what it was
//! asked. Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

// Every test here places the volume module and opens its popup, so this
// file exists only where both do (dev/research/scootbar-testing.md: the feature
// matrix).
#![cfg(all(feature = "volume", feature = "popup"))]

mod common;
mod pulse;
mod vpointer;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use common::{Reaper, Session, Shot, rgb, wakeups};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const ACCENT: &str = "#f9e2af";
const DIM: &str = "#506070";
const HEIGHT: u32 = 40;
/// The module, centered on the 1600-pixel output.
const MODULE: (u32, u32) = (800, 20);
/// The binding that makes a click open the popup.
const BINDING: &str = "on-click = \"popup\"\n";

fn config(extra: &str) -> String {
    format!(
        "[bar]\nheight = {HEIGHT}\nfont-size = 20\ntooltip-delay = 0\n\
         [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\naccent = \"{ACCENT}\"\n\
         dim = \"{DIM}\"\n[volume]\n{extra}"
    )
}

struct Rig {
    session: Session,
    bar: Reaper,
    server: pulse::Server,
    file: PathBuf,
    /// A row through the middle of the bar.
    bar_y: u32,
}

impl Rig {
    fn start(tag: &str) -> Option<Self> {
        Self::start_with(tag, BINDING, &[])
    }

    fn start_with(tag: &str, extra: &str, env: &[(&str, &str)]) -> Option<Self> {
        Self::start_placed(tag, extra, env, &["--center", "volume"])
    }

    /// With `placement` as the bar's own flags (where the module goes, which
    /// edge).
    fn start_placed(
        tag: &str,
        extra: &str,
        env: &[(&str, &str)],
        placement: &[&str],
    ) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
        let file = session.runtime_dir().join("bar.toml");
        fs::write(&file, config(extra)).unwrap();
        let address = server.address();
        let mut vars = vec![("PULSE_SERVER", address.as_str())];
        vars.extend_from_slice(env);
        let mut args = vec!["--config", file.to_str().unwrap()];
        args.extend_from_slice(placement);
        let bar = Reaper(session.bar_with_env(&args, &vars));
        let mut rig = Self {
            session,
            bar,
            server,
            file,
            bar_y: if placement.contains(&"bottom") {
                1000 - HEIGHT / 2
            } else {
                HEIGHT / 2
            },
        };
        rig.wait_shown();
        Some(rig)
    }

    fn shot(&self) -> Shot {
        self.session.scoot_screenshot(1)
    }

    /// The module has drawn (the bar's pixel under it is not the plain
    /// background anywhere in its span).
    fn wait_shown(&mut self) {
        let y = self.bar_y;
        self.session
            .wait_for(&mut self.bar.0, "the volume module shown", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600).any(|x| shot.at(x, y) == rgb(FG)).then_some(())
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

    fn move_to(&self, (x, y): (u32, u32)) {
        self.ipc(&format!(r#"{{"type":"pointer_move","x":{x},"y":{y}}}"#));
    }

    fn button(&self, pressed: bool) {
        self.ipc(&format!(
            r#"{{"type":"pointer_button","button":"left","pressed":{pressed}}}"#
        ));
    }

    fn key(&self, keys: &str) {
        self.ipc(&format!(r#"{{"type":"key","keys":"{keys}"}}"#));
    }

    /// Whether the popup's background is on screen under the module.
    fn popup_shown(&self) -> bool {
        self.shot().at(MODULE.0, HEIGHT + 2) == rgb(BAR)
    }

    fn wait_popup(&mut self, shown: bool, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            if self.popup_shown() == shown {
                return;
            }
            if std::time::Instant::now() > deadline {
                self.save("failure");
                panic!("never {what}; scootbar said: {}", self.session.bar_stderr());
            }
            self.alive();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The output's screenshot as a PNG at `$SCOOTBAR_POPUP_SHOTS/NAME.png`,
    /// when that directory is set: the evidence for the pixels the
    /// assertions read.
    fn save(&self, name: &str) {
        use base64::Engine as _;
        let Some(dir) = std::env::var_os("SCOOTBAR_POPUP_SHOTS") else {
            return;
        };
        let reply = self
            .session
            .scoot_ipc(r#"{"type":"screenshot","output":1,"cursor":false}"#);
        let png = base64::engine::general_purpose::STANDARD
            .decode(reply["png"].as_str().unwrap())
            .unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(PathBuf::from(dir).join(format!("{name}.png")), png).unwrap();
    }

    fn alive(&mut self) {
        assert!(
            self.bar.0.try_wait().unwrap().is_none(),
            "scootbar exited: {}",
            self.session.bar_stderr()
        );
    }

    /// The bar's stderr names no error.
    fn clean(&self) {
        let log = self.session.bar_stderr();
        assert!(!log.contains("error"), "{log}");
        assert!(!log.contains("protocol"), "{log}");
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn a_click_opens_the_popup_under_the_module_and_a_second_click_closes_it() {
    let Some(mut rig) = Rig::start("popup-toggle") else {
        return;
    };
    assert!(!rig.popup_shown());
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown after a click");
    rig.save("popup-open");
    // Idle with it open: nothing is drawn and nothing wakes.
    rig.alive();
    // The second click on the module is the toggle.
    rig.click(MODULE);
    rig.wait_popup(false, "the popup closed by a second click");
    rig.alive();
    rig.clean();
    // And it opens again.
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown again");
}

#[test]
fn escape_and_a_click_outside_close_it() {
    let Some(mut rig) = Rig::start("popup-dismiss") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    rig.key("Escape");
    rig.wait_popup(false, "the popup closed by Escape");
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown again");
    // Far from the bar and the popup.
    rig.click((200, 600));
    rig.wait_popup(false, "the popup closed by a click outside it");
    rig.alive();
    rig.clean();
}

/// Where the slider is on screen: its row's middle, and the rail's ends,
/// found from the pixels (the accent fills the rail up to the knob).
struct Slider {
    y: u32,
    left: u32,
    right: u32,
}

impl Slider {
    fn find(shot: &Shot) -> Option<Self> {
        let (mut top, mut bottom, mut at) = (u32::MAX, 0, 0);
        for y in HEIGHT..HEIGHT + 400 {
            for x in 600..1000 {
                if shot.at(x, y) == rgb(ACCENT) {
                    top = top.min(y);
                    bottom = bottom.max(y);
                    at = x;
                }
            }
        }
        if top == u32::MAX {
            return None;
        }
        let y = (top + bottom) / 2;
        let mut left = at;
        while left > 0 && shot.at(left - 1, y) != rgb(BAR) {
            left -= 1;
        }
        let mut right = at;
        while right + 1 < shot.width && shot.at(right + 1, y) != rgb(BAR) {
            right += 1;
        }
        Some(Self {
            y,
            left,
            right: right + 1,
        })
    }

    /// The x a pointer must be at to mean `percent` of 100.
    fn x(&self, percent: u32) -> u32 {
        let width = self.right - self.left;
        self.left + (width * percent + 50) / 100
    }

    /// Whether the rail is the accent at `percent` of its length.
    fn filled_at(&self, shot: &Shot, percent: u32) -> bool {
        shot.at(self.x(percent), self.y) == rgb(ACCENT)
    }
}

impl Rig {
    fn slider(&mut self) -> Slider {
        self.session
            .wait_for(&mut self.bar.0, "the slider drawn", |session| {
                Slider::find(&session.scoot_screenshot(1))
            })
    }
}

#[test]
fn the_slider_shows_the_level_and_a_drag_sets_it_once_per_value() {
    let Some(mut rig) = Rig::start("popup-slider") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    let slider = rig.slider();
    rig.save("popup-slider-49");
    // 49 percent: filled at 30, not at 70.
    let shot = rig.shot();
    assert!(slider.filled_at(&shot, 30) && !slider.filled_at(&shot, 70));

    // A press sets, a drag follows, a release ends: each new value once.
    rig.move_to((slider.x(80), slider.y));
    rig.button(true);
    // The same place again: no new value, nothing sent.
    rig.move_to((slider.x(80), slider.y));
    rig.move_to((slider.x(20), slider.y));
    // Past the track's ends, still on the popup: clamped to them.
    rig.move_to((slider.right + 6, slider.y));
    rig.move_to((slider.left.saturating_sub(6), slider.y));
    rig.button(false);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rig.server.sets_percent().last() != Some(&0) {
        assert!(
            std::time::Instant::now() < deadline,
            "the sets never ended at 0: {:?}; scootbar said: {}",
            rig.server.sets_percent(),
            rig.session.bar_stderr()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let sets = rig.server.sets_percent();
    // The press set 80, the drag ended at the left clamp: what lies between
    // may be coalesced by the module (one set in flight, the latest queued),
    // and a value is never sent twice running (a motion to where the
    // pointer already was sent nothing).
    assert_eq!(sets.first(), Some(&80), "{sets:?}");
    assert!(sets.len() <= 4, "{sets:?}");
    assert!(sets.windows(2).all(|pair| pair[0] != pair[1]), "{sets:?}");
    rig.alive();
    rig.clean();
    // The popup follows what the server now says.
    rig.save("popup-slider-dragged-to-0");
    rig.session
        .wait_for(&mut rig.bar.0, "the slider empty", |session| {
            let shot = session.scoot_screenshot(1);
            (!slider.filled_at(&shot, 30)).then_some(())
        });
}

#[test]
fn a_press_past_either_end_of_the_track_is_the_end() {
    let Some(mut rig) = Rig::start("popup-clamp") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    let slider = rig.slider();
    rig.click((slider.right + 6, slider.y));
    rig.session.wait_for(&mut rig.bar.0, "full scale", |_| {
        (rig.server.volume_percent() == 100).then_some(())
    });
    rig.click((slider.left.saturating_sub(6), slider.y));
    rig.session.wait_for(&mut rig.bar.0, "zero", |_| {
        (rig.server.volume_percent() == 0).then_some(())
    });
    // The popup is still up, and shows zero.
    rig.alive();
    assert!(rig.popup_shown());
}

#[test]
fn the_slider_follows_a_level_changed_elsewhere() {
    let Some(mut rig) = Rig::start("popup-external") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    let slider = rig.slider();
    rig.server.set_externally(90);
    rig.session
        .wait_for(&mut rig.bar.0, "the slider at 90", |session| {
            slider
                .filled_at(&session.scoot_screenshot(1), 80)
                .then_some(())
        });
    rig.server.set_externally(10);
    rig.session
        .wait_for(&mut rig.bar.0, "the slider at 10", |session| {
            (!slider.filled_at(&session.scoot_screenshot(1), 30)).then_some(())
        });
    rig.alive();
}

#[test]
fn the_button_mutes_on_release_and_a_press_that_slides_off_does_nothing() {
    let Some(mut rig) = Rig::start("popup-button") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    let slider = rig.slider();
    // The button is the row below the slider's.
    let row = (20.0f32 * 1.9).ceil() as u32;
    let (x, y) = (800, slider.y + row);
    rig.move_to((x, y));
    rig.button(true);
    rig.move_to((x, y + 3 * row));
    rig.button(false);
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.server.mutes().is_empty(), "{:?}", rig.server.mutes());
    rig.click((x, y));
    rig.session.wait_for(&mut rig.bar.0, "the mute", |_| {
        (rig.server.mutes() == [true]).then_some(())
    });
    rig.alive();
    // The popup stays open after the action.
    assert!(rig.popup_shown());
}

#[test]
fn the_keyboard_is_taken_only_while_a_popup_is_open() {
    let Some(mut rig) = Rig::start_with("popup-keyboard", BINDING, &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    let asks = |rig: &Rig| rig.session.bar_stderr().matches(".get_keyboard(").count();
    assert_eq!(asks(&rig), 0, "the bar took a keyboard with no popup open");
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    assert_eq!(asks(&rig), 1);
    // The grab is asked with the press's serial, before the keyboard.
    assert!(rig.session.bar_stderr().contains(".grab("));
    rig.key("Escape");
    rig.wait_popup(false, "the popup closed");
    let log = rig.session.bar_stderr();
    // The keyboard is given back with the popup.
    let keyboard = log
        .lines()
        .find_map(|l| l.split(".get_keyboard(wl_keyboard@").nth(1))
        .and_then(|rest| rest.split(')').next())
        .unwrap()
        .to_owned();
    assert!(
        log.contains(&format!("wl_keyboard@{keyboard}.release()")),
        "the keyboard was not released"
    );
    // And a second popup takes a second, not the first again.
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown again");
    assert_eq!(asks(&rig), 2);
}

#[test]
fn it_is_idle_while_open_and_while_closed() {
    let Some(mut rig) = Rig::start("popup-idle") else {
        return;
    };
    let pid = rig.bar.0.id();
    let settle = |pid: u32| {
        let mut last = wakeups(pid);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = wakeups(pid);
            if now == last {
                return last;
            }
            last = now;
        }
    };
    let closed = settle(pid);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakeups(pid), closed, "woke with no popup open");
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    let open = settle(pid);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakeups(pid), open, "woke while a popup sat open");
    rig.click(MODULE);
    rig.wait_popup(false, "the popup closed");
    let after = settle(pid);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakeups(pid), after, "woke after a popup closed");
}

#[test]
fn the_popup_goes_with_its_module_the_bar_hiding_and_a_reload() {
    let Some(mut rig) = Rig::start("popup-ends") else {
        return;
    };
    // The server going away: the module has nothing, so no popup.
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    rig.server.stop();
    rig.wait_popup(false, "the popup closed with its module");
    rig.alive();
    rig.clean();

    let Some(mut rig) = Rig::start("popup-hide") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    assert!(rig.msg(&["hide"]).status.success());
    rig.wait_popup(false, "the popup closed by hiding the bar");
    rig.alive();
    assert!(rig.msg(&["show"]).status.success());
    rig.session
        .wait_for(&mut rig.bar.0, "the bar back", |session| {
            (session.scoot_screenshot(1).at(5, 5) == rgb(BAR)).then_some(())
        });
    rig.click(MODULE);
    rig.wait_popup(true, "a popup on the bar made again");

    // A reload with the popup open.
    assert!(rig.msg(&["reload"]).status.success());
    rig.wait_popup(false, "the popup closed by a reload");
    rig.alive();
    rig.clean();
}

/// `xdg_wm_base` binds in the trace: the requests that bind it, not the
/// registry's listing of it.
fn xdg_binds(log: &str) -> usize {
    log.lines()
        .filter(|line| line.contains(".bind(") && line.contains("\"xdg_wm_base\""))
        .count()
}

#[test]
fn a_bar_with_no_popup_binding_binds_nothing_for_popups_until_invoke_asks() {
    // No `on-click = "popup"`: the module has a popup the config never asked
    // for, so the bar holds no popup global and no keyboard (a user who
    // never opts in pays nothing, not even a global that could be pinged).
    let Some(mut rig) = Rig::start_with("popup-invoke", "", &[("WAYLAND_DEBUG", "1")]) else {
        return;
    };
    assert_eq!(xdg_binds(&rig.session.bar_stderr()), 0);
    // A click does what it always did: mute.
    rig.click(MODULE);
    rig.session.wait_for(&mut rig.bar.0, "the mute", |_| {
        (rig.server.mutes() == [true]).then_some(())
    });
    assert!(!rig.popup_shown());
    assert_eq!(xdg_binds(&rig.session.bar_stderr()), 0);

    // `invoke` opens it anyway, binding the global for it.
    let out = rig.msg(&["invoke", "volume", "popup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    rig.wait_popup(true, "the popup opened by invoke");
    let log = rig.session.bar_stderr();
    assert_eq!(xdg_binds(&log), 1, "{log}");
    // No input serial to grab with, so no grab and no keyboard.
    assert!(!log.contains(".grab("), "{log}");
    assert!(!log.contains(".get_keyboard("), "{log}");
    let out = rig.msg(&["invoke", "volume", "popup"]);
    assert!(out.status.success());
    rig.wait_popup(false, "the popup closed by a second invoke");
    rig.alive();
    rig.clean();
}

#[test]
fn it_opens_on_the_press_and_the_release_that_follows_does_nothing() {
    let Some(mut rig) = Rig::start("popup-press") else {
        return;
    };
    rig.move_to(MODULE);
    rig.button(true);
    // Open with the button still down: a compositor's grab check needs it.
    rig.wait_popup(true, "the popup shown on the press");
    rig.button(false);
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.popup_shown(), "the release closed it");
    assert!(rig.server.mutes().is_empty());
    rig.alive();
    rig.clean();
}

#[test]
fn a_click_on_the_bar_beside_the_module_closes_it_without_acting() {
    let Some(mut rig) = Rig::start("popup-bare") else {
        return;
    };
    rig.click(MODULE);
    rig.wait_popup(true, "the popup shown");
    rig.click((100, 20));
    rig.wait_popup(false, "the popup closed by a click on bare bar");
    // The dismissal was the click's whole effect: nothing was muted.
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.server.mutes().is_empty());
    rig.alive();
    rig.clean();
    let _ = &rig.file;
}

/// On sway, the second layer-shell compositor scootbar is checked on: a
/// headless sway has no input device (so no pointer, and no click), which
/// leaves `invoke` as the way to open the popup, with no grab. What it
/// shows is that the `xdg_popup` parented to the layer surface maps there
/// too, and that it goes with its output without a protocol error.
#[test]
fn on_sway_it_opens_and_goes_with_its_output() {
    let Some(session) = Session::sway("popup-sway", 2) else {
        return;
    };
    let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
    let file = session.runtime_dir().join("bar.toml");
    fs::write(&file, config("")).unwrap();
    let address = server.address();
    let mut bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap(), "--center", "volume"],
        &[("PULSE_SERVER", address.as_str())],
    ));
    let names = |session: &Session| {
        let mut names: Vec<String> = session
            .sway_usable()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names
    };
    let outputs = names(&session);
    assert_eq!(outputs.len(), 2);
    // The popup's background is on screen under the module: the bar's
    // color where sway's backdrop is black.
    let popup_on = |session: &Session, name: &str| {
        let shot = session.screencopy(name);
        shot.at(shot.width / 2, HEIGHT + 2) == rgb(BAR)
    };
    // Wait for the module on the first output.
    session.wait_for(&mut bar.0, "the volume module shown", |session| {
        let shot = session.screencopy(&outputs[0]);
        (0..shot.width)
            .any(|x| shot.at(x, HEIGHT / 2) == rgb(FG))
            .then_some(())
    });
    let msg = |args: &[&str]| session.scootbar().arg("msg").args(args).output().unwrap();
    let opened = msg(&["invoke", "volume", "popup", "--output", &outputs[0]]);
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    session.wait_for(&mut bar.0, "the popup on sway", |session| {
        popup_on(session, &outputs[0]).then_some(())
    });
    // Not on the other output.
    assert!(!popup_on(&session, &outputs[1]));
    // Unplugged with the popup open: the bar lives, the other bar too.
    session.swaymsg(&["output", &outputs[0], "unplug"]);
    session.wait_for(&mut bar.0, "the output gone", |session| {
        (names(session).len() == 1).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    let log = session.bar_stderr();
    assert!(!log.contains("error") && !log.contains("protocol"), "{log}");
    // The popup can be opened on the one that is left.
    let again = msg(&["invoke", "volume", "popup"]);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let left = names(&session).remove(0);
    session.wait_for(&mut bar.0, "the popup on the remaining output", |session| {
        popup_on(session, &left).then_some(())
    });
    drop(server);
}

/// The compositor fits the popup to the output: a module at the right end
/// puts a popup wider than the room to its right back on screen (sliding
/// along the bar), flush with the output's edge.
#[test]
fn at_the_right_edge_it_slides_back_on_screen() {
    let Some(mut rig) = Rig::start_placed("popup-edge", BINDING, &[], &["--right", "volume"])
    else {
        return;
    };
    let desktop = rig.shot().at(800, HEIGHT + 60);
    let out = rig.msg(&["invoke", "volume", "popup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    rig.session
        .wait_for(&mut rig.bar.0, "the popup shown", |session| {
            (session.scoot_screenshot(1).at(1590, HEIGHT + 2) == rgb(BAR)).then_some(())
        });
    let shot = rig.shot();
    // 262 wide: its frame is the last column and the one 261 columns left of
    // it, and the pixel beyond that is the desktop.
    assert_eq!(shot.at(1599, 100), rgb(DIM));
    assert_eq!(shot.at(1599 - 261, 100), rgb(DIM));
    assert_eq!(shot.at(1599 - 262, 100), desktop);
    rig.alive();
    rig.clean();
}

/// A bar along the bottom opens its popup upward, anchored to the bar's top.
#[test]
fn a_bottom_bar_opens_it_above() {
    let Some(mut rig) = Rig::start_placed(
        "popup-bottom",
        BINDING,
        &[],
        &["--center", "volume", "--edge", "bottom"],
    ) else {
        return;
    };
    let out = rig.msg(&["invoke", "volume", "popup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The bar is rows 960..1000, the popup (124 tall) ends where it starts.
    rig.session
        .wait_for(&mut rig.bar.0, "the popup above the bar", |session| {
            (session.scoot_screenshot(1).at(800, 960 - 3) == rgb(BAR)).then_some(())
        });
    let shot = rig.shot();
    assert_eq!(shot.at(800, 959), rgb(DIM), "the popup's bottom frame");
    assert_eq!(shot.at(800, 960 - 124), rgb(DIM), "its top frame");
    assert_ne!(shot.at(800, 960 - 125), rgb(DIM));
    rig.alive();
}

/// A click on sway: a virtual pointer gives a headless sway the pointer it
/// lacks, and its clicks carry real serials. Opened by the press it grabs,
/// stays open through the release, closes on a press on the module again and
/// on a click outside it (the compositor's `popup_done`).
///
/// What this does **not** show is that the serial is the right one: wlroots
/// does not validate an `xdg_popup` grab's serial (changing the one sent to a
/// bogus number left this test passing, measured), so the press-time open is
/// checked here for the grab and the dismissal, not for the serial.
#[test]
fn on_sway_a_click_opens_it_with_a_grab_and_outside_closes_it() {
    let Some(session) = Session::sway("popup-sway-click", 1) else {
        return;
    };
    let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
    let file = session.runtime_dir().join("bar.toml");
    fs::write(&file, config(BINDING)).unwrap();
    let address = server.address();
    let mut bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap(), "--center", "volume"],
        &[("PULSE_SERVER", address.as_str()), ("WAYLAND_DEBUG", "1")],
    ));
    let name = session.sway_usable().remove(0).0;
    session.wait_for(&mut bar.0, "the volume module shown", |session| {
        let shot = session.screencopy(&name);
        (0..shot.width)
            .any(|x| shot.at(x, HEIGHT / 2) == rgb(FG))
            .then_some(())
    });
    let (width, height) = {
        let shot = session.screencopy(&name);
        (shot.width, shot.height)
    };
    let popup_on = |session: &Session| {
        let shot = session.screencopy(&name);
        shot.at(shot.width / 2, HEIGHT + 2) == rgb(BAR)
    };
    let mut pointer = vpointer::VirtualPointer::new(&session.wayland_socket(), (width, height));
    // The device gives the seat its pointer capability, and the bar takes a
    // `wl_pointer` when it hears so: moving before that is an `enter` it
    // never gets.
    session.wait_for(&mut bar.0, "the bar to take the pointer", |session| {
        session.bar_stderr().contains(".get_pointer(").then_some(())
    });
    let module = (width / 2, HEIGHT / 2);
    pointer.move_to(module.0, module.1);
    // The press opens it, with the button still held.
    pointer.button(true);
    session.wait_for(&mut bar.0, "the popup on the press", |session| {
        popup_on(session).then_some(())
    });
    pointer.button(false);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        popup_on(&session),
        "wlroots dismissed the grab: popup_done in the trace? {}",
        session.bar_stderr().contains("popup_done")
    );
    let log = session.bar_stderr();
    assert!(log.contains(".grab("), "{log}");
    assert!(!log.contains("popup_done"), "{log}");
    // A press on the module again closes it (the toggle), and the release
    // does not open it again.
    pointer.button(true);
    session.wait_for(
        &mut bar.0,
        "the popup closed by a second press",
        |session| (!popup_on(session)).then_some(()),
    );
    pointer.button(false);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!popup_on(&session));
    // Open once more and click outside it: the compositor dismisses it.
    pointer.button(true);
    session.wait_for(&mut bar.0, "the popup shown again", |session| {
        popup_on(session).then_some(())
    });
    pointer.button(false);
    pointer.move_to(width / 2, height / 2 + 200);
    pointer.button(true);
    session.wait_for(
        &mut bar.0,
        "the popup closed by a click outside",
        |session| (!popup_on(session)).then_some(()),
    );
    pointer.button(false);
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    drop(server);
}
