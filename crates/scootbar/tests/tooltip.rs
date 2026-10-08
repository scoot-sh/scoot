//! Tooltips on a headless scoot and on sway: a module's `tooltip` shown in an
//! `xdg_popup` under it after the pointer has rested a delay, gone when it
//! leaves, never with a grab, a keyboard or a click of its own.
//!
//! The module is a `push` one (its tooltip is whatever `scootbar msg set`
//! says), read from the compositor's screenshot (the frame, background and
//! text are known colors) and from the bar's own protocol trace. Waits poll
//! the screenshot every 20 ms with a deadline (no fixed sleeps, but for the
//! few "nothing happens" checks, each said). Skipped without a `scoot`
//! binary (see `common`); `SCOOTBAR_REQUIRE_SCOOT` and
//! `SCOOTBAR_REQUIRE_SWAY` make that a failure.

// Tooltips are shown by the popup machinery out of a `push` module, so
// this file exists only where both do (docs/scootbar/testing.md: the
// feature matrix). The one test that also places the volume module carries
// its own `volume` gate on top.
#![cfg(all(feature = "push", feature = "popup"))]

mod common;
mod lockclient;
#[cfg(feature = "volume")]
mod pulse;
mod vpointer;

use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Reaper, Session, Shot, rgb, wakeups};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const DIM: &str = "#506070";
const HEIGHT: u32 = 40;
/// The tooltip's delay, in the bar's config.
const DELAY: Duration = Duration::from_millis(300);
/// The volume module, centered on the 1600-pixel output. Only the
/// volume-gated test puts it there.
#[cfg(feature = "volume")]
const CENTER: (u32, u32) = (800, 20);
/// Far from the bar and anything under it.
const AWAY: (u32, u32) = (200, 600);
const TIP: &str = "the tooltip";

fn config(tables: &str, bar: &str) -> String {
    config_placed("left = [\"plain\"]\nright = [\"tip\"]\n", tables, bar)
}

/// The config with `lists` (`left = [...]`) placing the named modules.
fn config_placed(lists: &str, tables: &str, bar: &str) -> String {
    format!(
        "{lists}[bar]\nheight = {HEIGHT}\nfont-size = 20\ntooltip-delay = {}\n{bar}\
         [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\ndim = \"{DIM}\"\n{tables}",
        DELAY.as_millis()
    )
}

/// A `tip` module whose tooltip is set by `msg set`, at the right end, and a
/// `plain` one with none, on the left.
const TABLES: &str = "[push.tip]\nplaceholder = \"TIP\"\n[push.plain]\nplaceholder = \"plain\"\n";

struct Rig {
    session: Session,
    bar: Reaper,
    file: PathBuf,
    /// A row through the middle of the bar (the module's row).
    bar_y: u32,
    /// The middle of the `tip` module.
    tip: (u32, u32),
}

impl Rig {
    fn start(tag: &str) -> Option<Self> {
        Self::start_with(tag, TABLES, "", &[], &[])
    }

    fn start_with(
        tag: &str,
        tables: &str,
        bar: &str,
        placement: &[&str],
        env: &[(&str, &str)],
    ) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let file = session.runtime_dir().join("bar.toml");
        fs::write(&file, config(tables, bar)).unwrap();
        Self::launch(session, file, placement, env, bar.contains("bottom"))
    }

    fn launch(
        session: Session,
        file: PathBuf,
        placement: &[&str],
        env: &[(&str, &str)],
        bottom: bool,
    ) -> Option<Self> {
        let mut args = vec!["--config", file.to_str().unwrap()];
        args.extend_from_slice(placement);
        let bar = Reaper(session.bar_with_env(&args, env));
        let bar_y = if bottom {
            1000 - HEIGHT / 2
        } else {
            HEIGHT / 2
        };
        let mut rig = Self {
            session,
            bar,
            file,
            bar_y,
            tip: (0, bar_y),
        };
        rig.wait_shown();
        // The `tip` module is the one in the right half.
        let shot = rig.shot();
        let xs: Vec<u32> = (1000..1600)
            .filter(|&x| shot.at(x, bar_y) == rgb(FG))
            .collect();
        rig.tip = ((xs[0] + xs[xs.len() - 1]) / 2, bar_y);
        Some(rig)
    }

    fn shot(&self) -> Shot {
        self.session.scoot_screenshot(1)
    }

    /// The bar has drawn a module (its text is somewhere on its row).
    fn wait_shown(&mut self) {
        let y = self.bar_y;
        self.session
            .wait_for(&mut self.bar.0, "a module shown", |session| {
                let shot = session.scoot_screenshot(1);
                (0..1600).any(|x| shot.at(x, y) == rgb(FG)).then_some(())
            });
    }

    fn ipc(&self, request: &str) {
        let reply = self.session.scoot_ipc(request);
        assert_eq!(reply["type"], "ok", "{request}: {reply}");
    }

    fn move_to(&self, (x, y): (u32, u32)) {
        self.ipc(&format!(r#"{{"type":"pointer_move","x":{x},"y":{y}}}"#));
    }

    fn click(&self, (x, y): (u32, u32)) {
        self.ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
        ));
    }

    fn scroll(&self, (x, y): (u32, u32)) {
        self.move_to((x, y));
        self.ipc(r#"{"type":"scroll","dx":0,"dy":15}"#);
    }

    fn key(&self, keys: &str) {
        self.ipc(&format!(r#"{{"type":"key","keys":"{keys}"}}"#));
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    /// Sets the `tip` module's text and tooltip.
    fn set(&self, tooltip: &str) {
        let json = serde_json::json!({"text": "TIP", "tooltip": tooltip}).to_string();
        let out = self.msg(&["set", "tip", &json]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Whether a tooltip's background is on screen under `x`: the bar's
    /// color where the desktop is something else.
    fn tip_at(&self, x: u32) -> bool {
        let y = if self.bar_y < HEIGHT {
            HEIGHT + 2
        } else {
            1000 - HEIGHT - 4
        };
        self.shot().at(x, y) == rgb(BAR)
    }

    fn wait_tip(&mut self, x: u32, shown: bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if self.tip_at(x) == shown {
                return;
            }
            if Instant::now() > deadline {
                self.save("failure");
                panic!("never {what}; scootbar said: {}", self.session.bar_stderr());
            }
            self.alive();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The output as a PNG at `$SCOOTBAR_TOOLTIP_SHOTS/NAME.png`, when that
    /// directory is set: the evidence for the pixels the assertions read.
    fn save(&self, name: &str) {
        use base64::Engine as _;
        let Some(dir) = std::env::var_os("SCOOTBAR_TOOLTIP_SHOTS") else {
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

    fn clean(&self) {
        let log = self.session.bar_stderr();
        assert!(!log.contains("error"), "{log}");
        assert!(!log.contains("protocol"), "{log}");
        assert!(!log.contains("panicked"), "{log}");
    }

    /// The wakeups once the bar has been quiet for half a second.
    fn settle(&self) -> u64 {
        let pid = self.bar.0.id();
        let mut last = wakeups(pid);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = wakeups(pid);
            if now == last {
                return last;
            }
            last = now;
        }
    }

    /// The x of the middle of the `plain` module (on the left).
    fn plain_x(&self) -> u32 {
        let shot = self.shot();
        let y = self.bar_y;
        let xs: Vec<u32> = (0..600).filter(|&x| shot.at(x, y) == rgb(FG)).collect();
        (xs[0] + xs[xs.len() - 1]) / 2
    }

    /// How many `get_popup` requests the bar has made.
    fn popups_made(&self) -> usize {
        popups_in(&self.session.bar_stderr())
    }
}

/// The `xdg_surface.get_popup` requests in a protocol trace (the layer
/// surface's `get_popup`, which adopts each one, is not another popup).
fn popups_in(trace: &str) -> usize {
    trace
        .lines()
        .filter(|line| line.contains("xdg_surface@") && line.contains(".get_popup("))
        .count()
}

/// `xdg_wm_base` binds in the trace: the requests that bind it, not the
/// registry's listing of it.
fn xdg_binds(log: &str) -> usize {
    log.lines()
        .filter(|line| line.contains(".bind(") && line.contains("\"xdg_wm_base\""))
        .count()
}

fn region(shot: &Shot, x: Range<u32>, y: Range<u32>) -> Vec<[u8; 3]> {
    y.flat_map(|y| x.clone().map(move |x| (x, y)))
        .map(|(x, y)| shot.at(x, y))
        .collect()
}

/// How many rows below the bar at `x` are not the desktop's `color`: the
/// tooltip's height.
fn height_below(shot: &Shot, x: u32, desktop: [u8; 3]) -> u32 {
    (HEIGHT..1000)
        .take_while(|&y| shot.at(x, y) != desktop)
        .count() as u32
}

#[test]
fn hovering_shows_it_after_the_delay_and_leaving_hides_it_leaving_nothing_behind() {
    let Some(mut rig) = Rig::start("tip-hover") else {
        return;
    };
    rig.set(TIP);
    // Idle bar, pointer far away: nothing.
    let before = region(&rig.shot(), 1200..1600, HEIGHT..HEIGHT + 120);
    let began = Instant::now();
    rig.move_to(rig.tip);
    rig.wait_tip(
        rig.tip.0,
        true,
        "the tooltip shown after resting on the module",
    );
    assert!(
        began.elapsed() >= DELAY,
        "shown {:?} after the pointer arrived, before the {DELAY:?} delay",
        began.elapsed()
    );
    rig.save("tooltip-open");
    // Its text is in it, in the foreground color, below its top frame.
    let shot = rig.shot();
    assert!(
        (1300..1600).any(|x| (HEIGHT + 4..HEIGHT + 30).any(|y| shot.at(x, y) == rgb(FG))),
        "no text in the tooltip"
    );
    // The frame is the `dim` token: the tooltip's top edge is on the bar's
    // bottom edge.
    assert_eq!(shot.at(rig.tip.0, HEIGHT), rgb(DIM));
    // Leaving the bar hides it, and what it covered is the desktop again
    // (the redraw of a destroyed popup that nothing grabbed).
    rig.move_to(AWAY);
    rig.wait_tip(rig.tip.0, false, "the tooltip hidden after leaving");
    assert_eq!(
        region(&rig.shot(), 1200..1600, HEIGHT..HEIGHT + 120),
        before,
        "stale pixels where the tooltip was"
    );
    rig.alive();
    rig.clean();
}

#[test]
fn a_rapid_hover_in_and_out_leaves_no_tooltip_and_no_leak() {
    let Some(mut rig) = Rig::start_with("tip-flurry", TABLES, "", &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    rig.set(TIP);
    let pid = rig.bar.0.id();
    let before = region(&rig.shot(), 1200..1600, HEIGHT..HEIGHT + 120);
    let plain = rig.plain_x();
    rig.settle();
    let (fds, maps) = (common::open_fds(pid), common::shm_mappings(pid));
    // Across the module and off it, back and forth, faster than the delay:
    // nothing is armed for long enough to show.
    for _ in 0..200 {
        rig.move_to(rig.tip);
        rig.move_to((plain, 20));
        rig.move_to(AWAY);
    }
    assert!(!rig.tip_at(rig.tip.0));
    assert_eq!(
        rig.popups_made(),
        0,
        "a tooltip showed for a pointer that never rested"
    );
    // Then shown and hidden, over and over, each waited for.
    for _ in 0..10 {
        rig.move_to(rig.tip);
        rig.wait_tip(rig.tip.0, true, "shown");
        rig.move_to(AWAY);
        rig.wait_tip(rig.tip.0, false, "hidden");
    }
    assert_eq!(rig.popups_made(), 10);
    rig.alive();
    rig.clean();
    assert_eq!(
        region(&rig.shot(), 1200..1600, HEIGHT..HEIGHT + 120),
        before
    );
    rig.settle();
    assert_eq!(common::open_fds(pid), fds, "a descriptor leaked");
    assert_eq!(common::shm_mappings(pid), maps, "a buffer leaked");
}

#[test]
fn no_timer_runs_off_a_module_with_a_tooltip_and_the_bar_is_idle_with_one_shown() {
    let Some(mut rig) = Rig::start("tip-idle") else {
        return;
    };
    rig.set(TIP);
    let plain = rig.plain_x();
    // Resting on a module with no tooltip, and on bare bar: no timer, so no
    // wakeup, and nothing shown.
    rig.move_to((plain, 20));
    let rest = rig.settle();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        wakeups(rig.bar.0.id()),
        rest,
        "woke on a module with no tooltip"
    );
    assert!(!rig.tip_at(plain));
    rig.move_to((1200, 20));
    let rest = rig.settle();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakeups(rig.bar.0.id()), rest, "woke on bare bar");
    // Shown: nothing more is due, nothing wakes.
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    let shown = rig.settle();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        wakeups(rig.bar.0.id()),
        shown,
        "woke while a tooltip sat shown"
    );
    rig.move_to(AWAY);
    rig.wait_tip(rig.tip.0, false, "hidden");
    let after = rig.settle();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        wakeups(rig.bar.0.id()),
        after,
        "woke after the tooltip went"
    );
}

#[test]
fn a_module_with_no_tooltip_shows_none() {
    let Some(mut rig) = Rig::start("tip-none") else {
        return;
    };
    let plain = rig.plain_x();
    rig.move_to((plain, 20));
    // Longer than three delays: a tooltip would be up by now.
    std::thread::sleep(DELAY * 3);
    assert!(!rig.tip_at(plain));
    // A module whose tooltip goes empty while shown takes it with it.
    rig.set(TIP);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    rig.set("");
    rig.wait_tip(rig.tip.0, false, "gone with its text");
    rig.alive();
    rig.clean();
}

#[test]
fn a_click_closes_it_and_acts_and_it_stays_away_until_the_pointer_leaves() {
    let Some(session) = Session::scoot("tip-click", 1, "") else {
        return;
    };
    let out = session.runtime_dir().join("out");
    fs::create_dir_all(&out).unwrap();
    let tables = format!(
        "[push.tip]\nplaceholder = \"TIP\"\n\
         on-click = {{ exec = [\"sh\", \"-c\", \"echo x >> {}/clicks\"] }}\n",
        out.display()
    );
    let file = session.runtime_dir().join("bar.toml");
    fs::write(&file, config_placed("right = [\"tip\"]\n", &tables, "")).unwrap();
    let Some(mut rig) = Rig::launch(session, file, &[], &[], false) else {
        return;
    };
    rig.set(TIP);
    let clicks = || {
        fs::read_to_string(out.join("clicks"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    };
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    // The click goes through to the bar: the tooltip is not a popup that
    // spends it dismissing itself.
    rig.click(rig.tip);
    rig.wait_tip(rig.tip.0, false, "closed by the click");
    rig.session
        .wait_for(&mut rig.bar.0, "the click's action", |_| {
            (clicks() == 1).then_some(())
        });
    // Pointer still there: three delays later it has not come back.
    std::thread::sleep(DELAY * 3);
    assert!(!rig.tip_at(rig.tip.0), "it came back under a still pointer");
    assert_eq!(clicks(), 1);
    // Off and on again: it does. (The bar is given a moment to see the
    // pointer gone: a leave and an enter in one batch are no absence.)
    rig.move_to(AWAY);
    std::thread::sleep(Duration::from_millis(150));
    rig.move_to(rig.tip);
    rig.wait_tip(
        rig.tip.0,
        true,
        "shown again after the pointer left and came back",
    );
    // A scroll dismisses it the same way.
    rig.scroll(rig.tip);
    rig.wait_tip(rig.tip.0, false, "closed by a scroll");
    std::thread::sleep(DELAY * 3);
    assert!(!rig.tip_at(rig.tip.0));
    rig.alive();
    rig.clean();
}

#[test]
fn it_never_takes_the_keyboard_a_grab_or_the_pointer() {
    let Some(mut rig) = Rig::start_with("tip-nograb", TABLES, "", &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    rig.set(TIP);
    // A bar that has not shown a tooltip has not bound the popup global.
    assert_eq!(xdg_binds(&rig.session.bar_stderr()), 0);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    let log = rig.session.bar_stderr();
    assert_eq!(xdg_binds(&log), 1, "bound for the first tooltip");
    assert_eq!(popups_in(&log), 1, "{log}");
    assert!(!log.contains(".grab("), "a tooltip asked for a grab");
    assert!(
        !log.contains(".get_keyboard("),
        "a tooltip took the keyboard"
    );
    assert!(
        log.contains(".set_input_region("),
        "a tooltip has an input region"
    );
    // The pointer never entered its surface.
    let surface = log
        .lines()
        .find_map(|l| l.split(".get_xdg_surface(").nth(1))
        .and_then(|rest| rest.split("wl_surface@").nth(1))
        .and_then(|rest| rest.split(')').next())
        .expect("the popup's surface in the trace")
        .to_owned();
    assert!(
        !log.lines().any(|l| l.contains("wl_pointer@")
            && l.contains(".enter(")
            && l.contains(&format!("wl_surface@{surface}"))),
        "the pointer entered the tooltip"
    );
    // Escape does nothing (there is no keyboard to hear it).
    rig.key("Escape");
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.tip_at(rig.tip.0));
}

#[test]
fn its_text_follows_the_module_in_place_and_is_made_again_when_the_size_changes() {
    let Some(mut rig) = Rig::start_with("tip-update", TABLES, "", &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    rig.set("clock 10:00");
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    assert_eq!(rig.popups_made(), 1);
    let first = region(&rig.shot(), 1300..1600, HEIGHT..HEIGHT + 60);
    // Same size, other text: redrawn where it is.
    rig.set("clock 10:01");
    let deadline = Instant::now() + Duration::from_secs(8);
    while region(&rig.shot(), 1300..1600, HEIGHT..HEIGHT + 60) == first {
        assert!(Instant::now() < deadline, "the text never changed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        rig.popups_made(),
        1,
        "made again for a text of the same size"
    );
    // Shorter: still in place, at the size it has.
    rig.set("clock 10");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.popups_made(), 1, "made again for a shorter text");
    // Longer: made again, at the new size, with no delay to wait.
    rig.set("clock 10:01 and a good many more words than fit");
    let began = Instant::now();
    rig.session
        .wait_for(&mut rig.bar.0, "made again", |session| {
            (popups_in(&session.bar_stderr()) == 2).then_some(())
        });
    assert!(began.elapsed() < DELAY * 10);
    rig.wait_tip(rig.tip.0, true, "shown at the new size");
    rig.alive();
    rig.clean();
}

#[test]
fn a_long_tooltip_wraps_and_stays_on_screen_at_the_right_edge() {
    let Some(mut rig) = Rig::start("tip-wrap") else {
        return;
    };
    let desktop = rig.shot().at(800, HEIGHT + 100);
    // One line first: its height.
    rig.set("short");
    // The module is at the right end: its middle is near the edge.
    let x = rig.tip.0;
    rig.move_to((x, 20));
    rig.wait_tip(x, true, "shown");
    let one = height_below(&rig.shot(), x, desktop);
    rig.move_to(AWAY);
    rig.wait_tip(x, false, "hidden");
    // 250 bytes of words, wrapped at 30 ems into several lines. (The cap of six
    // is the unit tests': 256 bytes of words cannot reach it.)
    let long = "word ".repeat(50);
    rig.set(&long);
    rig.move_to((x, 20));
    rig.wait_tip(x, true, "the long one shown");
    rig.save("tooltip-wrapped-right-edge");
    let shot = rig.shot();
    let many = height_below(&shot, x, desktop);
    assert!(many > one * 3, "wrapped to {many} rows, one line is {one}");
    assert!(
        many < one * 8,
        "{many} rows: taller than a six-line tooltip"
    );
    // Slid back on screen: its frame is the output's last column.
    assert_eq!(shot.at(1599, HEIGHT + many / 2), rgb(DIM));
    // And no wider than 30 ems (600 px) plus its padding.
    let left = (0..1599)
        .rev()
        .find(|&x| shot.at(x, HEIGHT + many / 2) == rgb(DIM))
        .unwrap();
    assert!(1599 - left < 640, "{} px wide", 1599 - left);
    rig.alive();
    rig.clean();
}

#[test]
fn a_bottom_bars_tooltip_opens_above_it() {
    let Some(mut rig) = Rig::start_with("tip-bottom", TABLES, "edge = \"bottom\"\n", &[], &[])
    else {
        return;
    };
    rig.set(TIP);
    rig.move_to(rig.tip);
    // The bar is rows 960..1000: the tooltip ends where it starts.
    rig.wait_tip(rig.tip.0, true, "shown above the bar");
    let shot = rig.shot();
    assert_eq!(shot.at(rig.tip.0, 959), rgb(DIM), "its bottom frame");
    assert_eq!(shot.at(rig.tip.0, 960 - 3), rgb(BAR));
    rig.alive();
    rig.clean();
}

#[test]
fn a_reload_closes_it_and_the_new_delay_applies() {
    let Some(mut rig) = Rig::start("tip-reload") else {
        return;
    };
    rig.set(TIP);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    // The file now turns tooltips off: reloaded, the one up is gone and none
    // follows.
    fs::write(
        &rig.file,
        config(TABLES, "").replace("tooltip-delay = 300", "tooltip-delay = 0"),
    )
    .unwrap();
    assert!(rig.msg(&["reload"]).status.success());
    rig.wait_tip(rig.tip.0, false, "closed by the reload");
    std::thread::sleep(DELAY * 3);
    assert!(!rig.tip_at(rig.tip.0), "a tooltip with tooltips off");
    // And back on.
    fs::write(&rig.file, config(TABLES, "")).unwrap();
    assert!(rig.msg(&["reload"]).status.success());
    rig.set(TIP);
    rig.move_to(AWAY);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown with tooltips back on");
    rig.alive();
    rig.clean();
}

#[test]
fn hiding_the_bar_or_a_bad_reload_takes_it_down_without_harm() {
    let Some(mut rig) = Rig::start("tip-hide") else {
        return;
    };
    rig.set(TIP);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    assert!(rig.msg(&["hide"]).status.success());
    rig.wait_tip(rig.tip.0, false, "closed by hiding the bar");
    rig.alive();
    assert!(rig.msg(&["show"]).status.success());
    rig.session
        .wait_for(&mut rig.bar.0, "the bar back", |session| {
            (session.scoot_screenshot(1).at(5, 5) == rgb(BAR)).then_some(())
        });
    rig.clean();
}

#[cfg(feature = "volume")]
#[test]
fn under_a_click_popup_there_is_no_tooltip_and_after_it_there_is() {
    let Some(session) = Session::scoot("tip-popup", 1, "") else {
        return;
    };
    let server = pulse::Server::start(&session.runtime_dir().join("pulse"));
    let file = session.runtime_dir().join("bar.toml");
    let tables = format!("{TABLES}[volume]\non-click = \"popup\"\n");
    fs::write(
        &file,
        config_placed("left = [\"tip\", \"plain\"]\n", &tables, ""),
    )
    .unwrap();
    let address = server.address();
    let bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap(), "--center", "volume"],
        &[("PULSE_SERVER", address.as_str())],
    ));
    let mut rig = Rig {
        session,
        bar,
        file,
        bar_y: HEIGHT / 2,
        tip: (0, HEIGHT / 2),
    };
    rig.wait_shown();
    rig.set(TIP);
    // The `tip` module is the left-most of the two on the left.
    let shot = rig.shot();
    let xs: Vec<u32> = (0..600)
        .filter(|&x| shot.at(x, HEIGHT / 2) == rgb(FG))
        .collect();
    let first_run_end = xs
        .windows(2)
        .position(|w| w[1] - w[0] > 12)
        .map_or(xs.len() - 1, |at| at);
    rig.tip = ((xs[0] + xs[first_run_end]) / 2, HEIGHT / 2);
    // The volume popup open (the grab is the click's)...
    rig.click(CENTER);
    rig.wait_tip(CENTER.0, true, "the volume popup shown");
    // ...and the pointer resting on the tip module: no tooltip, however long.
    rig.move_to(rig.tip);
    std::thread::sleep(DELAY * 3);
    assert!(!rig.tip_at(rig.tip.0), "a tooltip under an open popup");
    rig.key("Escape");
    rig.wait_tip(CENTER.0, false, "the popup closed");
    // Closed. (scoot drops the first motion after a grab it ended; two to
    // the bare bar, then back onto the module, which is armed afresh.)
    rig.move_to((400, 20));
    rig.move_to((401, 20));
    std::thread::sleep(Duration::from_millis(100));
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "the tooltip once the popup is gone");
    // A click on the volume module opens its popup, which takes the tooltip
    // down (one popup at a time).
    rig.click(CENTER);
    rig.wait_tip(CENTER.0, true, "the popup opens");
    assert!(!rig.tip_at(rig.tip.0));
    rig.alive();
    rig.clean();
}

#[test]
fn a_session_lock_takes_it_and_none_shows_over_the_lock() {
    let Some(mut rig) = Rig::start("tip-lock") else {
        return;
    };
    rig.set(TIP);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown");
    let locker = lockclient::Locker::start(&rig.session.wayland_socket());
    rig.session
        .wait_for(&mut rig.bar.0, "the session locked", |_| {
            locker.is_locked().then_some(())
        });
    // The lock covers the output, the tooltip with it.
    let covered = |session: &Session| {
        let shot = session.scoot_screenshot(1);
        [
            (rig.tip.0, HEIGHT + 2),
            (rig.tip.0, 20),
            (100, 500),
            (rig.tip.0, HEIGHT + 20),
        ]
        .iter()
        .all(|&(x, y)| shot.at(x, y) == rgb(lockclient::COLOR))
    };
    rig.session.wait_for(
        &mut rig.bar.0,
        "the lock on screen over everything",
        |session| covered(session).then_some(()),
    );
    rig.save("tooltip-under-lock");
    // The pointer rests where the module was, for many delays: no tooltip
    // is drawn over the lock.
    rig.move_to(rig.tip);
    for _ in 0..8 {
        std::thread::sleep(DELAY);
        assert!(covered(&rig.session), "a tooltip over the session lock");
    }
    rig.alive();
    // Unlocked: the bar is back, and a tooltip works again.
    drop(locker);
    rig.session
        .wait_for(&mut rig.bar.0, "the session unlocked", |session| {
            (session.scoot_screenshot(1).at(5, 5) == rgb(BAR)).then_some(())
        });
    rig.move_to(AWAY);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "a tooltip after the unlock");
    rig.alive();
    rig.clean();
}

/// Sway's outputs as `(name, x, y, width, height)`, from `get_outputs`.
fn sway_outputs(session: &Session) -> Vec<(String, u32, u32, u32, u32)> {
    let outputs: serde_json::Value =
        serde_json::from_str(&session.swaymsg(&["-t", "get_outputs", "-r"])).unwrap();
    let num = |rect: &serde_json::Value, key: &str| rect[key].as_u64().unwrap() as u32;
    outputs
        .as_array()
        .unwrap()
        .iter()
        .map(|output| {
            let rect = &output["rect"];
            (
                output["name"].as_str().unwrap().to_owned(),
                num(rect, "x"),
                num(rect, "y"),
                num(rect, "width"),
                num(rect, "height"),
            )
        })
        .collect()
}

/// A tooltip's background under the `tip` module, as `screencopy` of `name`
/// shows it.
fn sway_tip_on(session: &Session, name: &str) -> bool {
    let shot = session.screencopy(name);
    let xs: Vec<u32> = (shot.width / 2..shot.width)
        .filter(|&x| shot.at(x, HEIGHT / 2) == rgb(FG))
        .collect();
    let Some((first, last)) = xs.first().zip(xs.last()) else {
        return false;
    };
    shot.at((first + last) / 2, HEIGHT + 2) == rgb(BAR)
}

/// The middle of the `tip` module on `name`'s bar, in that output's pixels.
fn sway_tip_x(session: &Session, name: &str) -> Option<u32> {
    let shot = session.screencopy(name);
    let xs: Vec<u32> = (shot.width / 2..shot.width)
        .filter(|&x| shot.at(x, HEIGHT / 2) == rgb(FG))
        .collect();
    Some((xs.first()? + xs.last()?) / 2)
}

#[test]
fn on_sway_it_shows_and_goes_with_its_output() {
    let Some(session) = Session::sway("tip-sway", 2) else {
        return;
    };
    let file = session.runtime_dir().join("bar.toml");
    fs::write(&file, config(TABLES, "")).unwrap();
    let mut bar = Reaper(session.bar_with_env(
        &["--config", file.to_str().unwrap()],
        &[("WAYLAND_DEBUG", "1")],
    ));
    let outputs = sway_outputs(&session);
    assert_eq!(outputs.len(), 2);
    // The extent of the layout, for the absolute motion of a virtual
    // pointer, and the output at its left edge.
    let left = outputs.iter().map(|o| o.1).min().unwrap();
    let right = outputs.iter().map(|o| o.1 + o.3).max().unwrap();
    let height = outputs.iter().map(|o| o.2 + o.4).max().unwrap();
    let first = outputs.iter().find(|o| o.1 == left).unwrap().clone();
    let tip_x = session.wait_for(&mut bar.0, "the module shown", |session| {
        sway_tip_x(session, &first.0)
    });
    let json = serde_json::json!({"text": "TIP", "tooltip": TIP}).to_string();
    let set = session
        .scootbar()
        .arg("msg")
        .args(["set", "tip", &json])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let mut pointer =
        vpointer::VirtualPointer::new(&session.wayland_socket(), (right - left, height));
    session.wait_for(&mut bar.0, "the bar to take the pointer", |session| {
        session.bar_stderr().contains(".get_pointer(").then_some(())
    });
    pointer.move_to(tip_x, HEIGHT / 2);
    session.wait_for(&mut bar.0, "the tooltip on sway", |session| {
        sway_tip_on(session, &first.0).then_some(())
    });
    // Unplugged while shown: the bar lives, with no protocol error.
    session.swaymsg(&["output", &first.0, "unplug"]);
    session.wait_for(&mut bar.0, "the output gone", |session| {
        (sway_outputs(session).len() == 1).then_some(())
    });
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    let log = session.bar_stderr();
    assert!(!log.contains("error") && !log.contains("protocol"), "{log}");
    // And the output that is left shows one when the pointer rests on it.
    let left = sway_outputs(&session).remove(0);
    let mut pointer = vpointer::VirtualPointer::new(&session.wayland_socket(), (left.3, left.4));
    let x = sway_tip_x(&session, &left.0).expect("the module on the output that is left");
    pointer.move_to(x, HEIGHT / 2 + 1);
    pointer.move_to(x, HEIGHT / 2);
    session.wait_for(
        &mut bar.0,
        "a tooltip on the output that is left",
        |session| sway_tip_on(session, &left.0).then_some(()),
    );
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
}

#[test]
fn a_scale_change_with_it_shown_leaves_one_at_the_new_scale() {
    let Some(session) = Session::scoot("tip-scale", 1, "") else {
        return;
    };
    let file = session.runtime_dir().join("bar.toml");
    fs::write(&file, config(TABLES, "")).unwrap();
    let Some(mut rig) = Rig::launch(session, file, &[], &[], false) else {
        return;
    };
    rig.set(TIP);
    let desktop = rig.shot().at(100, 500);
    rig.move_to(rig.tip);
    rig.wait_tip(rig.tip.0, true, "shown at scale 1");
    let one = height_below(&rig.shot(), rig.tip.0, desktop);
    // The output goes to scale 2 (scoot's reload applies it): the mode stays
    // 1600 pixels, the bar is 80 of them tall, and whatever the tooltip was
    // made again at, the one that shows with the pointer on the module is at
    // twice the size.
    fs::write(
        rig.session.runtime_dir().join("config.toml"),
        "[output]\nscale = 2\n",
    )
    .unwrap();
    let reply = rig.session.scoot_ipc(r#"{"type":"reload"}"#);
    assert_eq!(reply["type"], "reloaded", "{reply}");
    let x = rig
        .session
        .wait_for(&mut rig.bar.0, "the bar at scale 2", |session| {
            let shot = session.scoot_screenshot(1);
            if shot.at(5, 60) != rgb(BAR) {
                return None;
            }
            let xs: Vec<u32> = (800..1600).filter(|&x| shot.at(x, 40) == rgb(FG)).collect();
            Some((xs.first()? + xs.last()?) / 2)
        });
    rig.move_to((x / 2 - 4, 20));
    rig.move_to((x / 2, 20));
    let deadline = Instant::now() + Duration::from_secs(8);
    let two = loop {
        let shot = rig.shot();
        if shot.at(x, 2 * HEIGHT + 4) == rgb(BAR) {
            break height_below(&shot, x, desktop);
        }
        assert!(
            Instant::now() < deadline,
            "no tooltip at scale 2: {}",
            rig.session.bar_stderr()
        );
        rig.alive();
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        two > one + one / 2,
        "{two} rows at scale 2, {one} at scale 1"
    );
    rig.alive();
    rig.clean();
}
