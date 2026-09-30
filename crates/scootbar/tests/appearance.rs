//! The bar's shape on a headless scoot, end to end: what a click in a cut
//! corner reaches (the input region), where text sits against the
//! corners, and the spacing options on screenshots.
//!
//! Skipped without a `scoot` binary (see `common`); `SCOOTBAR_REQUIRE_SCOOT`
//! makes that a failure.

mod common;

use common::{Reaper, Session, foot_missing};
use serde_json::Value;

const BAR: &str = "#c03020";

/// Writes the bar's config into the session's scratch directory and
/// returns the flags that read it.
fn config(session: &Session, toml: &str) -> [String; 2] {
    let path = session.runtime_dir().join("appearance.toml");
    std::fs::write(&path, toml).unwrap();
    ["--config".to_owned(), path.to_string_lossy().into_owned()]
}

fn windows(session: &Session) -> Vec<Value> {
    let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
    reply["windows"].as_array().cloned().unwrap_or_default()
}

fn focused_id(session: &Session) -> Option<u64> {
    windows(session)
        .iter()
        .find(|w| w["focused"] == true)
        .and_then(|w| w["id"].as_u64())
}

fn click(session: &Session, x: i64, y: i64) {
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
}

/// A click in a corner the rounded bar cuts reaches the window behind it,
/// while one on the bar's flat part does not: scoot honors the surface's
/// input region, so the corners are not swallowed by an invisible bar.
/// The control is a bar with no radius, which swallows the same click.
#[test]
fn a_click_in_a_cut_corner_reaches_the_window_behind() {
    if foot_missing() {
        return;
    }
    for (radius, reaches) in [(60, true), (0, false)] {
        let Some(session) = Session::scoot(&format!("input-{radius}"), 1, "") else {
            return;
        };
        // A tall bar over the windows (`exclusive = false`, on the `top`
        // layer), its corners cut by half its height.
        let flags = config(
            &session,
            &format!("[bar]\nheight = 120\nradius = {radius}\nexclusive = false\n"),
        );
        let mut bar = Reaper(session.bar(&["--background", BAR, &flags[0], &flags[1]]));
        for _ in 0..2 {
            let reply =
                session.scoot_ipc(r#"{"type":"action","action":"spawn","command":["foot"]}"#);
            assert_eq!(reply["type"], "ok", "{reply}");
            let want = windows(&session).len() + 1;
            session.wait_for(&mut bar.0, "a window mapped", |session| {
                (windows(session).len() >= want).then_some(())
            });
        }
        let all = windows(&session);
        let left = all
            .iter()
            .min_by_key(|w| w["rect"]["x"].as_i64())
            .unwrap()
            .clone();
        let right = focused_id(&session).unwrap();
        assert_ne!(left["id"].as_u64(), Some(right), "{all:?}");
        let (x, y) = (
            left["rect"]["x"].as_i64().unwrap(),
            left["rect"]["y"].as_i64().unwrap(),
        );
        // Wait for the bar to have its shape before clicking.
        session.wait_for(&mut bar.0, "the bar drawn", |session| {
            (session.scoot_screenshot(1).at(400, 10) == common::rgb(BAR)).then_some(())
        });
        // The flat top of the bar: swallowed, focus stays on the right one.
        click(&session, x + 200, y + 2);
        // Absence cannot be waited for: give the click time to have moved
        // focus, were it going to.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            focused_id(&session),
            Some(right),
            "the flat part is the bar's"
        );
        // The window's own corner, under the bar's cut one.
        click(&session, x + 1, y + 1);
        let got = session.wait_for(&mut bar.0, "the click landed", |session| {
            let now = focused_id(session);
            (now != Some(right) || !reaches).then_some(now)
        });
        if reaches {
            assert_eq!(got, left["id"].as_u64(), "a cut corner reaches the window");
        } else {
            assert_eq!(got, Some(right), "a square bar swallows its corner");
        }
    }
}

// The tests below place the clock, so they exist only where it does.
#[cfg(feature = "clock")]
const FG: &str = "#f0f0f0";

#[cfg(feature = "clock")]
/// The leftmost column holding a pixel of exactly `FG` (the test font's
/// segments are whole pixels, so ink is exact) in the top `rows` rows.
fn leftmost_ink(shot: &common::Shot, rows: u32) -> Option<u32> {
    (0..shot.width).find(|&x| (0..rows).any(|y| shot.at(x, y) == common::rgb(FG)))
}

#[cfg(feature = "clock")]
/// Starts a 60-high bar showing the clock at the left, in the test font
/// at 50 pixels to the em, with `toml` as its config, and returns the
/// leftmost ink column once the clock is up, or `None` where there is no
/// scoot to run it on (the test is then skipped, as `Session::scoot` says).
fn clock_ink(tag: &str, toml: &str) -> Option<u32> {
    let session = Session::scoot(tag, 1, "")?;
    let flags = config(&session, toml);
    let mut bar = Reaper(session.bar(&[
        "--background",
        BAR,
        "--foreground",
        FG,
        "--font-size",
        "50",
        "--left",
        "clock",
        "--center=",
        &flags[0],
        &flags[1],
    ]));
    Some(session.wait_for(&mut bar.0, "the clock drawn", |session| {
        leftmost_ink(&session.scoot_screenshot(1), 60)
    }))
}

/// The first module starts past the corner: with a radius of 30 on a
/// 60-high bar and padding 8, its span starts at radius - padding / 2 = 26
/// (its pill, half a padding into it, at the radius), so its ink sits 26
/// pixels further in than on the same bar with square corners, and past
/// the 30-pixel corner square.
#[test]
#[cfg(feature = "clock")]
fn text_stays_out_of_a_rounded_bars_corners() {
    let Some(square) = clock_ink("ink-square", "[bar]\nheight = 60\npadding = 8\n") else {
        return;
    };
    let round = clock_ink(
        "ink-round",
        "[bar]\nheight = 60\nradius = 30\npadding = 8\n",
    )
    .expect("scoot was there for the first bar");
    assert_eq!(round, square + 26, "square {square}, round {round}");
    assert!(round >= 30, "ink in the corner square");
}

/// A module's `margin` moves it in by that much, through the config file
/// and the real bar.
#[test]
#[cfg(feature = "clock")]
fn a_module_margin_moves_it_in_from_the_edge() {
    let Some(plain) = clock_ink("margin-none", "[bar]\nheight = 60\n") else {
        return;
    };
    let margined = clock_ink("margin-12", "[bar]\nheight = 60\n[clock]\nmargin = 12\n")
        .expect("scoot was there for the first bar");
    assert_eq!(margined, plain + 12);
}
