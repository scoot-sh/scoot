//! Tests for the allowlists: presentation passes, commands and geometry
//! refuse.

use super::*;
use std::collections::BTreeMap;

#[test]
fn presentation_passes() {
    assert!(allows("scoot", "appearance.focus_ring_active_color"));
    assert!(allows("scoot", "wallpaper.image"));
    assert!(allows("scoot", "wallpaper.output.HDMI-A-1.color"));
    assert!(allows("scootbar", "colors.accent"));
    assert!(allows("scootbar", "bar.radius"));
    assert!(allows("scootbar", "workspaces.inactive-color"));
    assert!(allows("scootbar", "workspaces.pill-shape"));
    assert!(allows("scootbar", "workspaces.pill-inset"));
    assert!(allows("scootbar", "workspaces.disc"));
    assert!(allows("scootbar", "clock.icon"));
    assert!(allows("scootbar", "power.icon-poweroff"));
    assert!(allows("scootbar", "battery.icon-charging"));
    assert!(allows("scootnotify", "colors.match"));
    assert!(allows("scootlaunch", "icons.theme"));
    assert!(allows("scootlaunch", "colors.match"));
    let err = check_app_value(
        "scootlaunch",
        "icons.theme",
        &toml::Value::String("Adwaita".into()),
    );
    assert!(err.is_none(), "icons.theme refused: {err:?}");
    let err = check_app_value(
        "scootlaunch",
        "icons.theme",
        &toml::Value::String("../escape".into()),
    );
    assert!(err.is_some(), "bad icons.theme passes");
}

#[test]
fn bar_visual_keys_take_their_values() {
    // The look-layer-scootbar allowlist: shape enums, the disc boolean,
    // and one glyph per icon key.
    let err = check_app_value(
        "scootbar",
        "workspaces.pill-shape",
        &toml::Value::String("circle".into()),
    );
    assert!(err.is_none(), "pill-shape refused: {err:?}");
    let err = check_app_value(
        "scootbar",
        "workspaces.pill-shape",
        &toml::Value::String("hexagon".into()),
    );
    assert!(err.is_some(), "bad pill-shape passes");
    let err = check_app_value("scootbar", "workspaces.disc", &toml::Value::Boolean(true));
    assert!(err.is_none(), "disc refused: {err:?}");
    let err = check_app_value(
        "scootbar",
        "workspaces.disc",
        &toml::Value::String("yes".into()),
    );
    assert!(err.is_some(), "string disc passes");
    for key in [
        "clock.icon",
        "volume.icon",
        "microphone.icon",
        "network.icon",
        "network.icon-ethernet",
        "network.icon-vpn",
        "network.icon-offline",
        "media.icon",
        "media.icon-playing",
        "media.icon-paused",
        "bluetooth.icon",
        "bluetooth.icon-off",
        "bluetooth.icon-on",
        "bluetooth.icon-connected",
        "power.icon",
        "power.icon-lock",
        "power.icon-logout",
        "power.icon-suspend",
        "power.icon-reboot",
        "power.icon-poweroff",
        "battery.icon-charging",
        "battery.icon-full",
        "window-title.icon",
    ] {
        let err = check_app_value("scootbar", key, &toml::Value::String("●".into()));
        assert!(err.is_none(), "{key} refused: {err:?}");
    }
    // The array-valued icon levels are not glyph strings: not allowlisted.
    for key in ["brightness.icon", "battery.icon", "network.icon-wifi"] {
        assert!(!allows("scootbar", key), "allows {key}");
    }
    // Geometry and actions stay out, with the new keys beside them.
    for key in [
        "bar.edge",
        "bar.height",
        "bar.tooltip-delay",
        "workspaces.pill-shape.foo",
    ] {
        assert!(!allows("scootbar", key), "allows {key}");
    }
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
    assert!(allows("scoot", "wallpaper.output.HDMI-A-1.color"));
    assert!(allows("scoot", "wallpaper.workspace.2.color"));
    assert!(allows(
        "scoot",
        "wallpaper.output.HDMI-A-1.workspace.2.color"
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

#[test]
fn nested_workspace_override_refuses() {
    // Round 2 finding 6: `wallpaper.workspace.a.workspace.b.image`
    // passed the allowlist and rendered an unknown key, which discards
    // scoot's whole `[wallpaper]` section. One `.workspace.` layer only.
    for bad in [
        "wallpaper.workspace.a.workspace.b.image",
        "wallpaper.workspace.a.workspace.b.color",
        "wallpaper.workspace.a.workspace.b.mode",
    ] {
        assert!(!allows("scoot", bad), "allows {bad}");
        assert!(value_kind("scoot", bad).is_none(), "value kind for {bad}");
    }
    // `image` refuses at every level; single-level non-image leaves
    // still pass.
    assert!(!allows("scoot", "wallpaper.workspace.a.image"));
    assert!(allows("scoot", "wallpaper.workspace.a.color"));
}

#[test]
fn override_image_leaves_refuse() {
    // Round 2 finding 4: per-output/workspace `image` overrides
    // rendered a relative path with nothing to resolve it against, so
    // the override silently yielded no wallpaper. Only the top-level
    // `wallpaper.image` (resolved through `RenderInputs.wallpaper_abs`)
    // names an image; overrides take color, mode, fill and transition.
    for key in [
        "wallpaper.output.DP-1.image",
        "wallpaper.workspace.2.image",
        "wallpaper.output.DP-1.workspace.2.image",
    ] {
        assert!(!allows("scoot", key), "allows {key}");
        assert!(value_kind("scoot", key).is_none(), "kind for {key}");
        let err = check_app_value("scoot", key, &toml::Value::String("a.png".into()));
        assert!(err.is_some(), "{key} value passes");
    }
    assert!(allows("scoot", "wallpaper.output.DP-1.color"));
    assert!(allows("scoot", "wallpaper.workspace.2.mode"));
    assert!(allows("scoot", "wallpaper.output.DP-1.workspace.2.fill"));
}

#[test]
fn image_path_refuses_non_images() {
    // Round 2 finding 7: `is_permitted_look_file` also passes
    // `look.toml`, `README.md`, `LICENSE*` and `.ttf`/`.otf`, none of
    // which decodes as a wallpaper.
    for bad in [
        "look.toml",
        "README.md",
        "LICENSE",
        "LICENSE-MIT",
        "COPYING",
        "font.ttf",
        "font.otf",
        "notes.txt",
        "wallpaper",
    ] {
        let err = check_app_value("scoot", "wallpaper.image", &toml::Value::String(bad.into()));
        assert!(err.is_some(), "{bad} passes as an image");
    }
    for good in [
        "a.png",
        "a.jpg",
        "a.jpeg",
        "a.webp",
        "a.gif",
        "A.JPG",
        "dir/a.PNG",
    ] {
        let err = check_app_value(
            "scoot",
            "wallpaper.image",
            &toml::Value::String(good.into()),
        );
        assert!(err.is_none(), "{good} refused: {err:?}");
    }
}

#[test]
fn scootbar_font_keys_refused() {
    // Round 2 finding 3: scootbar's `font` and `fallback-fonts` are
    // file paths, not family names, and only the tool's resolved
    // `--ui-font-file` supplies them. No override spells them.
    for key in ["bar.font", "bar.fallback-fonts"] {
        assert!(!allows("scootbar", key), "allows {key}");
        assert!(
            value_kind("scootbar", key).is_none(),
            "value kind for {key}"
        );
    }
}

#[test]
fn the_depth_keys_belong_to_the_notifier_alone() {
    for key in ["shadow", "fill"] {
        assert!(allows("scootnotify", key));
        // The launcher and the greeter read neither yet: a key they would
        // report as unknown is refused at the look, not at every load.
        assert!(!allows("scootlaunch", key), "{key}");
        assert!(!allows("scootgreet", key), "{key}");
        assert!(!allows("scootbar", key), "{key}");
    }
    let text = |s: &str| toml::Value::String(s.to_string());
    for style in SHADOW_STYLES {
        assert!(check_app_value("scootnotify", "shadow", &text(style)).is_none());
    }
    for style in FILL_STYLES {
        assert!(check_app_value("scootnotify", "fill", &text(style)).is_none());
    }
    for (key, bad) in [
        ("shadow", "huge"),
        ("shadow", "SOFT"),
        ("fill", "rainbow"),
        ("fill", ""),
    ] {
        assert!(
            check_app_value("scootnotify", key, &text(bad)).is_some(),
            "{key}={bad}"
        );
    }
    assert!(check_app_value("scootnotify", "shadow", &toml::Value::Integer(1)).is_some());
    assert!(check_app_value("scootlaunch", "shadow", &text("soft")).is_some());
}
