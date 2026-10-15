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
