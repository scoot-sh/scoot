//! Tests for the daemon leaf renderers: golden fragments for the minimal
//! look, and a hostile value in every interpolated field refused by each
//! renderer (the grammar and the quoting both hold).

use std::collections::BTreeMap;

use super::*;
use crate::resolve::resolve;
use crate::schema::{Origin, parse_look_toml};

fn moonrise() -> ResolvedLook {
    let text = crate::schema::tests::minimal_toml();
    let raw = parse_look_toml(text.as_bytes()).expect("minimal parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    resolve(&map, "moonrise").expect("minimal resolves")
}

#[test]
fn mako_fragment() {
    // The minimal look: background #2B3648, foreground #F6EEDC, accent
    // (and default focus) #FFA45C, urgent #E87F6A.
    let frag = render_mako(&moonrise()).expect("renders");
    let expected = concat!(
        "# Rendered from look \"moonrise\" by scoot-look (mako). Do not edit: re-rendered on every apply.\n",
        "layer=overlay\n",
        "background-color=#2B3648\n",
        "text-color=#F6EEDC\n",
        "border-color=#FFA45C\n",
        "progress-color=over #FFA45C\n",
        "\n[mode=do-not-disturb]\n",
        "invisible=1\n",
        "\n[urgency=critical]\n",
        "border-color=#E87F6A\n",
    );
    assert_eq!(frag, expected, "mako fragment changed");
}

#[test]
fn wob_fragment() {
    let frag = render_wob(&moonrise()).expect("renders");
    let expected = concat!(
        "# Rendered from look \"moonrise\" by scoot-look (wob.ini). Do not edit: re-rendered on every apply.\n",
        "max = 100\n",
        "width = 400\n",
        "height = 40\n",
        "anchor = bottom\n",
        "margin = 48\n",
        "overflow_mode = nowrap\n",
        "background_color = 2B3648ff\n",
        "bar_color = FFA45Cff\n",
        "border_color = FFA45Cff\n",
        "overflow_background_color = 2B3648ff\n",
        "overflow_bar_color = E87F6Aff\n",
        "overflow_border_color = E87F6Aff\n",
        "\n[style.muted]\n",
        "background_color = 2B3648ff\n",
        "bar_color = 9C8B95ff\n",
        "border_color = 9C8B95ff\n",
    );
    assert_eq!(frag, expected, "wob fragment changed");
    assert!(
        !frag.lines().any(|line| line.starts_with("timeout")),
        "timeout is the launcher's line, never the fragment's:\n{frag}"
    );
}

#[test]
fn fcitx5_fragment() {
    let frag = render_fcitx5_theme(&moonrise()).expect("renders");
    let expected = concat!(
        "# Rendered from look \"moonrise\" by scoot-look (fcitx5 classic UI theme). Do not edit: re-rendered on every apply.\n",
        "[Metadata]\n",
        "Name=Scoot Look\n",
        "Version=1.0\n",
        "Author=scoot\n",
        "Description=The desktop look's palette as an fcitx5 classic UI theme.\n",
        "ScaleWithDPI=True\n",
        "\n",
        "[InputPanel]\n",
        "Font=Sans 11\n",
        "NormalColor=#F6EEDC\n",
        "HighlightColor=#FFA45C\n",
        "HighlightBackgroundColor=#2B3648\n",
        "HighlightCandidateColor=#2B3648\n",
        "\n[InputPanel/Background]\n",
        "Color=#2B3648\n",
        "BorderColor=#FFA45C\n",
        "\n[InputPanel/Highlight]\n",
        "Color=#FFA45C\n",
        "\n[Menu]\n",
        "Font=Sans 10\n",
        "NormalColor=#F6EEDC\n",
        "\n[Menu/Background]\n",
        "Color=#2B3648\n",
        "\n[Menu/Highlight]\n",
        "Color=#FFA45C\n",
    );
    assert_eq!(frag, expected, "fcitx5 fragment changed");
}

#[test]
fn nightlight_fragment() {
    let frag = render_nightlight("moonrise", 3400).expect("renders");
    assert_eq!(
        frag,
        "# Rendered from look \"moonrise\" by scoot-look (nightlight). Do not edit: re-rendered on every apply.\ntemperature=3400\n"
    );
    for bad in [999, 10001, 0, -1] {
        assert!(
            render_nightlight("moonrise", bad).is_err(),
            "temperature {bad} renders"
        );
    }
}

/// Every string the renderers interpolate is a hex color; a hostile one
/// (past a hand-built look, skipping the parse gate) refuses in every
/// renderer instead of landing raw in a line-based format.
#[test]
fn hostile_colors_refused_by_every_renderer() {
    for hostile in [
        "#2B3648\nbackground-color=#000000",
        "#2B3648\n[urgency=critical]",
        "over #FFA45C",
        "#FFA45",
        "#FFA45CG",
        "red",
        "#2B3648\"",
        "#2B3648\\",
        "#2B3648=",
        "#2B3648[",
        "",
    ] {
        for field in [
            "background",
            "foreground",
            "dim",
            "accent",
            "urgent",
            "surface",
            "hover",
            "focus",
            "border",
            "accent_fg",
            "selection",
            "selection_text",
            "r#match",
        ] {
            let mut look = moonrise();
            match field {
                "background" => look.palette.background = hostile.to_string(),
                "foreground" => look.palette.foreground = hostile.to_string(),
                "dim" => look.palette.dim = hostile.to_string(),
                "accent" => look.palette.accent = hostile.to_string(),
                "urgent" => look.palette.urgent = hostile.to_string(),
                "surface" => look.palette.surface = hostile.to_string(),
                "hover" => look.palette.hover = hostile.to_string(),
                "focus" => look.palette.focus = hostile.to_string(),
                "border" => look.palette.border = hostile.to_string(),
                "accent_fg" => look.palette.accent_fg = hostile.to_string(),
                "selection" => look.palette.selection = hostile.to_string(),
                "selection_text" => look.palette.selection_text = hostile.to_string(),
                _ => look.palette.r#match = hostile.to_string(),
            }
            // A hostile value never renders anywhere: the used roles
            // refuse, and the unused ones are still safe because they
            // never reach a fragment. Assert the three config
            // renderers, which interpolate palette roles.
            for (what, result) in [
                ("mako", render_mako(&look).map(|_| ())),
                ("wob", render_wob(&look).map(|_| ())),
                ("fcitx5", render_fcitx5_theme(&look).map(|_| ())),
            ] {
                let uses = matches!(
                    (what, field),
                    (
                        "mako",
                        "background" | "foreground" | "focus" | "accent" | "urgent"
                    ) | ("wob", "background" | "accent" | "focus" | "urgent" | "dim")
                        | ("fcitx5", "background" | "foreground" | "accent" | "focus")
                );
                if uses {
                    assert!(
                        result.is_err(),
                        "{what} renders hostile {field} {hostile:?}"
                    );
                }
            }
        }
    }
}

/// A hostile look name (past a hand-built look) stays one quoted line in
/// every fragment header: no second line, no section, no key.
#[test]
fn hostile_name_stays_one_quoted_line() {
    let mut look = moonrise();
    look.name = "x\n[urgency=critical]\nborder-color=#000000".to_string();
    for frag in [
        render_mako(&look).expect("renders"),
        render_wob(&look).expect("renders"),
        render_fcitx5_theme(&look).expect("renders"),
    ] {
        let header = frag.lines().next().unwrap_or_default();
        assert!(
            header.starts_with("# Rendered from look \"x\\n"),
            "name renders raw:\n{frag}"
        );
        assert!(
            !frag.contains("[urgency=critical]\nborder-color=#000000"),
            "injection:\n{frag}"
        );
    }
    let frag = render_nightlight("x\ntemperature=9999", 3400).expect("renders");
    assert!(
        frag.lines().count() == 2,
        "name breaks the nightlight file:\n{frag}"
    );
}

/// Every rendered line is single-line `key=value` (or a section/header):
/// no renderer ever emits a raw newline inside a value.
#[test]
fn fragments_hold_no_raw_value_newlines() {
    let look = moonrise();
    for frag in [
        render_mako(&look).expect("renders"),
        render_wob(&look).expect("renders"),
        render_fcitx5_theme(&look).expect("renders"),
        render_nightlight("moonrise", 3400).expect("renders"),
    ] {
        for line in frag.lines() {
            assert!(
                line.is_empty()
                    || line.starts_with('#')
                    || line.starts_with('[')
                    || line.contains('=')
                    || line.contains(" = "),
                "bare line: {line:?}"
            );
        }
    }
}
