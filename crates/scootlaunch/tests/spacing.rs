//! The panel's spacing, read off a real headless screenshot: the search
//! field and the selection highlight share the panel's inner box, the
//! icon sits inside the highlight, the search line carries no row icon,
//! and every row (a blank icon included) shows an icon or a tile.
//!
//! Skipped without a `scoot` binary beside the test binary
//! (`SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure). With
//! `SCOOTLAUNCH_SHOTS=<dir>` set the screenshots are saved there.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::time::Duration;

use harness::{
    Geom, Panel, TERMINAL_ORANGE, add_blank_icon, desktop, desktop_with_icon, fixture_icon_theme,
    render_launch_fragment, rgb, session_with_apps,
};

fn spacing_one(name: &str) {
    let long = "An Application With An Extremely Long Name That Cannot Fit The Panel";
    let bodies: Vec<(&str, String)> = vec![
        (
            "alpha.desktop",
            desktop_with_icon("Alpha", "/usr/bin/alpha", "alpha"),
        ),
        (
            "beta.desktop",
            desktop_with_icon("Beta", "/usr/bin/beta", "beta"),
        ),
        ("foot.desktop", desktop("Foot Client", "/usr/bin/foot")),
        (
            "chrome.desktop",
            desktop_with_icon("Google Chrome", "/usr/bin/chrome", "beta"),
        ),
        (
            "pwa.desktop",
            desktop_with_icon("https://pwa.zoom.us/wc", "/usr/bin/pwa", "blank"),
        ),
        (
            "nvim.desktop",
            desktop_with_icon("Neovim wrapper", "/usr/bin/nvim", "missing-icon"),
        ),
        ("qt.desktop", desktop("Qt6 Settings", "/usr/bin/qt")),
        ("long.desktop", desktop(long, "/usr/bin/long")),
    ];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) = session_with_apps(&format!("spacing-{name}"), &refs) else {
        return;
    };
    let theme = fixture_icon_theme(&data_home);
    add_blank_icon(&data_home);
    let fragment = render_launch_fragment(name);
    let mut table: toml::Table = fragment.parse().expect("launch.toml parses");
    table
        .get_mut("icons")
        .and_then(|icons| icons.as_table_mut())
        .expect("icons table")
        .insert("theme".to_string(), toml::Value::String(theme));
    let fragment = table.to_string();
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), &fragment).unwrap();
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    let colors = table["colors"].as_table().expect("colors").clone();
    let color = |key: &str| rgb(colors[key].as_str().unwrap());
    let (focus, selection) = (color("focus"), color("selection"));
    let em = table["font-size"].as_integer().expect("font-size") as f32;
    let radius = table["radius"].as_integer().expect("radius") as u32;
    panel.wait_shown(focus);
    std::thread::sleep(Duration::from_millis(400));
    panel.save(&format!("spacing-{name}"));
    let shot = panel.shot();
    let (x0, y0, x1, y1) = panel.find_panel(focus);
    let g = Geom::new(em, radius);
    let fill = shot.at((x0 + x1) / 2, y1 - 4);
    assert_ne!(fill, selection, "{name}: the probe sits on panel fill");

    // The highlight spans the inner box: its first and last pixels on
    // the row's middle line sit one inner padding in from the panel's
    // edges, and the search field's ring starts at the same x.
    let row_mid = y0 + g.list_y() + g.row_h / 2;
    let hits = |y: u32, wanted: [u8; 3]| -> Vec<u32> {
        (x0 + 4..=x1 - 4)
            .filter(|&x| shot.at(x, y) == wanted)
            .collect()
    };
    let highlight = hits(row_mid, selection);
    let (left, right) = (*highlight.first().unwrap(), *highlight.last().unwrap());
    assert!(
        left.abs_diff(x0 + g.inner) <= 2 && right.abs_diff(x1 - g.inner) <= 2,
        "{name}: highlight {left}..{right} for inner {}..{}",
        x0 + g.inner,
        x1 - g.inner
    );
    let field_mid = y0 + g.inner + g.row_h / 2;
    let ring = (x0 + 4..=x1)
        .find(|&x| shot.at(x, field_mid) != fill)
        .expect("a field ring");
    assert!(
        ring.abs_diff(left) <= 2,
        "{name}: field starts at {ring}, highlight at {left}"
    );

    // The highlight is behind the icon: the gap between its left edge
    // and the icon is highlight, and the icon square is mostly icon.
    let icon_x = x0 + g.inner + g.lead;
    let icon_y = y0 + g.list_y() + g.row_h.saturating_sub(g.side) / 2;
    assert_eq!(shot.at(left + 3, row_mid), selection, "{name}");
    assert_eq!(shot.at(icon_x - 2, row_mid), selection, "{name}");
    let not_fill = |x: u32, y: u32, bg: [u8; 3]| shot.at(x, y) != bg;
    // The search line shows no row icon: none of the selected entry's
    // orange (Alpha, the first row) anywhere in the field.
    let in_field = (x0 + g.inner..x1 - g.inner)
        .flat_map(|x| (y0 + g.inner..y0 + g.inner + g.row_h).map(move |y| (x, y)))
        .filter(|&(x, y)| shot.at(x, y) == TERMINAL_ORANGE)
        .count();
    assert_eq!(in_field, 0, "{name}: an icon drew in the search field");

    // Every row, the blank icon and the missing one included, shows an
    // icon or a tile: its square is not empty background.
    for row in 0..8 {
        let bg = if row == 0 { selection } else { fill };
        let y = icon_y + row * g.row_h;
        let drawn = (icon_x..icon_x + g.side)
            .flat_map(|x| (y..y + g.side).map(move |y| (x, y)))
            .filter(|&(x, y)| not_fill(x, y, bg))
            .count() as u32;
        assert!(
            drawn >= g.side * g.side / 3,
            "{name}: row {row} icon square holds {drawn} of {} pixels",
            g.side * g.side
        );
    }
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

#[test]
fn spacing_music_desk() {
    spacing_one("music-desk");
}

#[test]
fn spacing_moonrise() {
    spacing_one("moonrise");
}
