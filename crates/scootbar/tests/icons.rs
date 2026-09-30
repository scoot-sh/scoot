//! A clock icon from a fallback symbol font, on headless scoot: a config
//! names the seven-segment font as primary and a symbol font (every glyph a
//! bar) as the one fallback, and the clock draws the icon's bar, then the
//! primary's digits, and a reload naming a fallback that is not a font is
//! refused with the running bar untouched.
//!
//! Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

mod common;

use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{Reaper, Session, rgb, testfont};

const ICON: char = '\u{f0e65}';
const BAR: &str = "#102030";
const EM: u32 = 50;
const HEIGHT: u32 = 60;
const BASELINE: i64 = 45;
const TZ: &str = "IST-5:30";

fn config(session: &Session, fallback: &std::path::Path) -> std::path::PathBuf {
    let path = session.runtime_dir().join("bar.toml");
    let text = format!(
        "center = [\"clock\"]\n[bar]\nheight = {HEIGHT}\nfont-size = {EM}\nfont = \"{}\"\n\
         fallback-fonts = [\"{}\"]\n[colors]\nbackground = \"{BAR}\"\nforeground = \"#f0f0f0\"\n\
         [clock]\nformat = \"%H\"\nicon = \"{ICON}\"\n",
        session.font().display(),
        fallback.display()
    );
    std::fs::write(&path, text).unwrap();
    path
}

fn hour() -> i64 {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    (t + 5 * 3600 + 1800).rem_euclid(86_400) / 3600
}

#[test]
fn a_clock_icon_comes_from_the_fallback_font_and_a_bad_fallback_is_refused() {
    let Some(session) = Session::scoot("icons", 1, "") else {
        return;
    };
    let symbols = session.runtime_dir().join("symbols.ttf");
    std::fs::write(&symbols, testfont::build_symbols([ICON])).unwrap();
    let path = config(&session, &symbols);
    let log = std::fs::File::create(session.bar_log()).unwrap();
    let mut bar = Reaper(
        session
            .scootbar()
            .env("TZ", TZ)
            .arg("daemon")
            .arg("--config")
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let background = rgb(BAR);
    session.wait_for(&mut bar.0, "the icon and the hour", |session| {
        let shot = session.scoot_screenshot(1);
        let ink = |x: i64, y: i64| {
            x >= 0
                && y >= 0
                && (x as u32) < shot.width
                && (y as u32) < HEIGHT
                && shot.at(x as u32, y as u32) != background
        };
        let columns: Vec<i64> = (0..i64::from(shot.width))
            .filter(|&x| (0..i64::from(HEIGHT)).any(|y| ink(x, y)))
            .collect();
        let (&left, &right) = (columns.first()?, columns.last()?);
        // The icon's bar starts 5 pixels into its cell; the digits are
        // the two cells after the icon and its space.
        let origin = left - 5;
        let unit = f64::from(EM) / 1000.0;
        let at = |x: f64, y: f64| {
            ink(
                origin + (x * unit).floor() as i64,
                BASELINE - (y * unit).floor() as i64,
            )
        };
        let is_bar = at(300.0, 350.0) && !at(150.0, 50.0) && !at(300.0, 650.0);
        let digits = testfont::decode(ink, origin + 60, right, BASELINE, f64::from(EM));
        let h = hour();
        let want = |h: i64| format!("{:02}", h.rem_euclid(24));
        (is_bar && (digits == want(h) || digits == want(h - 1))).then_some(())
    });
    // A reload whose fallback is no font is refused; the bar stands.
    let junk = session.runtime_dir().join("junk.ttf");
    std::fs::write(&junk, b"not a font").unwrap();
    config(&session, &junk);
    let out = session
        .scootbar()
        .arg("msg")
        .arg("reload")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("fallback font") && said.contains("junk.ttf"),
        "{said}"
    );
    assert!(bar.0.try_wait().unwrap().is_none());
}
