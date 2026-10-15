//! Tests for the allowlists: presentation passes, commands and geometry
//! refuse.

use super::*;
use std::collections::BTreeMap;

#[test]
fn presentation_passes() {
    assert!(allows("scoot", "appearance.focus_ring_active_color"));
    assert!(allows("scoot", "wallpaper.image"));
    assert!(allows("scoot", "wallpaper.output.HDMI-A-1.image"));
    assert!(allows("scootbar", "colors.accent"));
    assert!(allows("scootbar", "bar.radius"));
    assert!(allows("scootbar", "workspaces.inactive-color"));
    assert!(allows("scootnotify", "colors.match"));
}

#[test]
fn commands_binds_and_geometry_refuse() {
    for (app, key) in [
        ("scoot", "autostart.foo"),
        ("scoot", "binds.foo"),
        ("scoot", "wallpaper.command"),
        ("scoot", "tty.gpu"),
        ("scoot", "renderer.backend"),
        ("scoot", "xwayland.enabled"),
        ("scoot", "virtual_input.enabled"),
        ("scootbar", "bar.edge"),
        ("scootbar", "bar.height"),
        ("scootbar", "bar.margin"),
        ("scootbar", "bar.layer"),
        ("scootbar", "bar.exclusive"),
        ("scootbar", "workspaces.on-click"),
        ("scootbar", "exec.foo"),
        ("scootbar", "button.foo"),
        ("unknown-app", "colors.accent"),
    ] {
        assert!(!allows(app, key), "{app} allows {key}");
    }
}

#[test]
fn validate_names_the_key() {
    let apps = BTreeMap::from([(
        "scoot".to_string(),
        BTreeMap::from([("binds.foo".to_string(), toml::Value::String("x".into()))]),
    )]);
    let errors = validate_apps_allowlist(&apps);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].to_string().contains("binds.foo"));
}

#[test]
fn wallpaper_output_suffix_is_strict() {
    // Finding 1: `allows("scoot", k)` accepted any key starting
    // `wallpaper.output.` / `wallpaper.workspace.`, so a quoted key
    // carrying a newline, `=`, `[` or a quote sailed through and the
    // renderer wrote it raw: a real `[autostart]` table out.
    assert!(allows("scoot", "wallpaper.output.HDMI-A-1.image"));
    assert!(allows("scoot", "wallpaper.workspace.2.color"));
    assert!(allows(
        "scoot",
        "wallpaper.output.HDMI-A-1.workspace.2.image"
    ));
    for bad in [
        "wallpaper.output.x = 1\n[autostart]\ncommand",
        "wallpaper.output.\n[autostart]",
        "wallpaper.output.x[evil]",
        "wallpaper.output.x\"evil",
        "wallpaper.output.x=1",
        "wallpaper.output.",
        "wallpaper.output.HDMI-A-1.evil",
        "wallpaper.output.HDMI-A-1.command",
        "wallpaper.output..image",
        "wallpaper.output.HDMI-A-1.",
        "wallpaper.workspace.2.binds",
    ] {
        assert!(!allows("scoot", bad), "allows {bad:?}");
    }
}
