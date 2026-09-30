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

/// A config with the clock showing the hour and `icon_keys` after it, in a
/// bar of `HEIGHT` logical pixels and an em of `ART_EM`, on the seven-segment
/// font (no fallback: a path or image icon needs none).
fn art_config(session: &Session, icon_keys: &str) -> std::path::PathBuf {
    let path = session.runtime_dir().join("bar.toml");
    let text = format!(
        "center = [\"clock\"]\n[bar]\nheight = {HEIGHT}\nfont-size = {ART_EM}\nfont = \"{}\"\n\
         [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\n\
         [clock]\nformat = \"%H\"\n{icon_keys}\n",
        session.font().display(),
    );
    std::fs::write(&path, text).unwrap();
    path
}

const ART_EM: u32 = 40;
const FG: &str = "#f0f0f0";

/// The icon's square and the hour after it, read from a screenshot at
/// `scale`: `ink_color` is the color the square must be (the theme's
/// foreground for a path icon, the picture's own for an image), and the
/// square is `ART_EM × scale` device pixels on a side, centered in the
/// bar's rows, with the digits after it and a space.
fn check_square(session: &Session, scale: f64, ink_color: [u8; 3]) -> Option<()> {
    let shot = session.scoot_screenshot(1);
    let background = rgb(BAR);
    let side = (f64::from(ART_EM) * scale).round() as i64;
    let rows = (f64::from(HEIGHT) * scale).round() as i64;
    let ink = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as u32) < shot.width
            && y < rows
            && shot.at(x as u32, y as u32) != background
    };
    let columns: Vec<i64> = (0..i64::from(shot.width))
        .filter(|&x| (0..rows).any(|y| ink(x, y)))
        .collect();
    let (&left, &right) = (columns.first()?, columns.last()?);
    let top = (rows - side) / 2;
    let at = |x: i64, y: i64| shot.at(x as u32, y as u32);
    // The square: whole, in the wanted color, its four corners and its
    // middle; and background one pixel outside each edge.
    let whole = [
        (left, top),
        (left + side - 1, top),
        (left, top + side - 1),
        (left + side - 1, top + side - 1),
        (left + side / 2, top + side / 2),
    ]
    .iter()
    .all(|&(x, y)| at(x, y) == ink_color);
    let outside = [
        (left - 1, top + side / 2),
        (left + side, top + side / 2),
        (left + side / 2, top - 1),
        (left + side / 2, top + side),
    ]
    .iter()
    .all(|&(x, y)| x < 0 || y < 0 || at(x, y) == background);
    if !(whole && outside) {
        return None;
    }
    // The digits: a space (0.6 em) after the square, then two cells.
    let em = f64::from(side as i32);
    let gap = (0.6 * em).round() as i64;
    let baseline = ((rows - side) as f64 / 2.0 + 0.8 * em).round() as i64;
    let digits = testfont::decode(ink, left + side + gap - 1, right, baseline, em);
    let h = hour();
    let want = |h: i64| format!("{:02}", h.rem_euclid(24));
    (digits == want(h) || digits == want(h - 1)).then_some(())
}

fn run_art(tag: &str, output: &str, scale: f64, keys: &str, color: [u8; 3]) {
    let Some(session) = Session::scoot(tag, 1, output) else {
        return;
    };
    let path = art_config(&session, keys);
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
    session.wait_for(&mut bar.0, "the icon and the hour", |session| {
        check_square(session, scale, color)
    });
    // A reload with a path that is not one is refused, naming the key, and
    // the bar stands with what it had.
    art_config(&session, "icon-path = \"M0 0 L\"");
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
        said.contains("clock.icon-path") && said.contains("at byte"),
        "{said}"
    );
    assert!(bar.0.try_wait().unwrap().is_none());
    assert!(check_square(&session, scale, color).is_some());
}

/// A path icon on headless scoot at scale 1 and at 1.5: a full-viewbox
/// square, drawn in the foreground token at exactly the em, next to the
/// hour.
#[test]
fn a_path_icon_is_drawn_at_the_outputs_real_scale_in_the_foreground_token() {
    let keys = "icon-path = \"M0 0H24V24H0z\"";
    run_art("path-1x", "", 1.0, keys, rgb(FG));
    run_art("path-1_5x", "[output]\nscale = 1.5\n", 1.5, keys, rgb(FG));
}

/// A PNG icon: a solid red picture, scaled to the em at each scale, its own
/// color (not the foreground token).
#[cfg(feature = "icon-image")]
#[test]
fn a_png_icon_is_scaled_to_the_outputs_real_scale_and_keeps_its_own_colors() {
    let scratch = std::env::temp_dir().join(format!("scootbar-icons-png-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let file = scratch.join("red.png");
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 16, 16);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[0xff, 0x00, 0x00, 0xff].repeat(256))
        .unwrap();
    std::fs::write(&file, out).unwrap();
    let keys = format!("icon-image = \"{}\"", file.display());
    run_art("png-1x", "", 1.0, &keys, [0xff, 0, 0]);
    run_art(
        "png-1_5x",
        "[output]\nscale = 1.5\n",
        1.5,
        &keys,
        [0xff, 0, 0],
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
