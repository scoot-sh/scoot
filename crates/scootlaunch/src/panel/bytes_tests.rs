//! The panel paints the bytes it always did when no depth or motion
//! token is set: one FNV-1a fingerprint over a spread of looks, scales and
//! contents, recorded on the commit before the effects landed
//! (`4e8293664`) and pinned here. Any change to the default path moves it.

use super::*;
use crate::rank::frecency::Frecency;

pub(super) fn test_text(dir: &std::path::Path) -> Text {
    let path = dir.join("test.ttf");
    std::fs::write(&path, scootui::testfont::build()).expect("font");
    scootui::font::text(Some(&path), &[]).expect("text")
}

pub(super) fn entries(count: usize) -> Vec<Entry> {
    (0..count)
        .map(|n| Entry {
            id: format!("app{n}.desktop"),
            name: format!("Application number {n}"),
            generic_name: None,
            comment: None,
            keywords: Vec::new(),
            icon: None,
            exec: vec!["x".to_owned()],
            terminal: false,
            dbus_activatable: false,
            actions: Vec::new(),
            path: PathBuf::from("/usr/share/applications/x.desktop"),
            workdir: None,
            hidden: None,
        })
        .collect()
}

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// The looks the fingerprint covers: the defaults, a translucent
/// restyled one and a square, large-font one.
fn looks() -> Vec<Look> {
    let translucent = Look {
        opacity: 0.8,
        radius: 16,
        surface: scootui::color::Color {
            r: 30,
            g: 30,
            b: 46,
        },
        focus: scootui::color::Color {
            r: 249,
            g: 226,
            b: 175,
        },
        ..Look::default()
    };
    let square = Look {
        radius: 0,
        font_size_px: 20.0,
        ..Look::default()
    };
    vec![Look::default(), translucent, square]
}

/// The fingerprint of every apps panel draw (three looks, the given
/// scales, four list sizes, an error line) and, if asked, one dmenu draw.
pub(super) fn fingerprint(scales: &[scootui::density::Scale], dmenu: bool) -> u64 {
    let dir = tempfile::tempdir().expect("tmp");
    let mut text = test_text(dir.path());
    let mut icons = crate::icons::Cache::default();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for look in looks() {
        for scale in scales.iter().copied() {
            for count in [0usize, 1, 5, 40] {
                let ui = Ui::open(entries(count), Frecency::empty(), 0, 10);
                let logical = Layout::new(look.font_size_px, look.radius, 1.0);
                let width = (30.0 * look.font_size_px) as u32;
                let height = logical.height(10);
                let factor = scale.factor();
                let width_d = (f64::from(width) * factor).round() as u32;
                let height_d = (f64::from(height) * factor).round() as u32;
                for error in [None, Some("scootlaunch: could not start it")] {
                    let mut pixels = vec![0u8; width_d as usize * height_d as usize * 4];
                    let view = super::PanelView {
                        ui: &ui,
                        look: &look,
                        width,
                        height,
                        rows: 10,
                        error,
                        empty_text: "empty",
                        scale,
                        icon_theme: "Fixture",
                        icon_bases: &[],
                    };
                    super::paint_panel(
                        &mut pixels,
                        (width_d, height_d),
                        &view,
                        &mut text,
                        &mut icons,
                        None,
                    );
                    fnv(&mut hash, &pixels);
                }
            }
        }
    }
    if !dmenu {
        return hash;
    }
    let look = Look::default();
    let lines: Vec<String> = (0..12).map(|n| format!("line {n}")).collect();
    let dui = DmenuUi::open(lines, 6, false, false);
    let spec = DmenuSpec {
        prompt: String::from("> "),
        placeholder: String::new(),
        index: false,
        prompt_color: look.theme.foreground,
        prompt_only: false,
    };
    let layout = Layout::dmenu(look.font_size_px);
    let (width, height) = (400u32, layout.height(6));
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    super::paint_dmenu(
        &mut pixels,
        (width, height),
        &dui,
        &mut text,
        &look,
        width,
        height,
        6,
        scootui::density::Scale::Integer(1),
        &spec,
    );
    fnv(&mut hash, &pixels);
    hash
}

/// The 1x draws: recorded on `4e8293664`, before depth and motion, and
/// unchanged since (the fingerprint then covered the scaled draws too, so
/// this is that value's 1x half, taken on the commit before the launcher
/// followed the compositor's scale).
#[test]
fn the_default_look_paints_the_bytes_it_always_did() {
    let hash = fingerprint(&[scootui::density::Scale::Integer(1)], true);
    assert_eq!(hash, ONE_X, "recorded on 4e8293664");
}

/// The scaled draws, with the rows held to the scaled height: recorded
/// when the launcher began drawing at the compositor's scale, which is the
/// first time a scaled draw reached a screen.
#[test]
fn the_scaled_panel_paints_the_bytes_recorded_with_the_fit() {
    let hash = fingerprint(
        &[
            scootui::density::Scale::Fractional(180),
            scootui::density::Scale::Integer(2),
        ],
        false,
    );
    assert_eq!(hash, SCALED, "recorded with the rows held to the height");
}

const ONE_X: u64 = 0x6226_31d1_2e84_8874;
const SCALED: u64 = 0xa352_c849_814f_5f69;
