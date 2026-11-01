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
fn the_depth_keys_belong_to_the_notifier_and_the_launcher() {
    for key in ["shadow", "fill"] {
        assert!(allows("scootnotify", key));
        assert!(allows("scootlaunch", key), "{key}");
        // The greeter reads neither yet, and the bar reads its own dotted
        // keys (`bar.shadow`), never the dotless ones: a key they would
        // report as unknown is refused at the look, not at every load.
        assert!(!allows("scootgreet", key), "{key}");
        assert!(!allows("scootbar", key), "{key}");
    }
    // The bar's depth keys live under `[bar]`, sharing the look's
    // allowlist (never a copy): `bar.shadow` and `bar.fill`.
    for key in BAR_DEPTH_KEYS {
        assert!(allows("scootbar", key), "{key}");
        assert!(!allows("scootgreet", key), "{key}");
        assert!(!allows("scoot", key), "{key}");
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
    // The launcher takes the same values, and refuses the same bad ones.
    for style in SHADOW_STYLES {
        assert!(check_app_value("scootlaunch", "shadow", &text(style)).is_none());
    }
    for style in FILL_STYLES {
        assert!(check_app_value("scootlaunch", "fill", &text(style)).is_none());
    }
    assert!(check_app_value("scootlaunch", "shadow", &text("huge")).is_some());
    assert!(check_app_value("scootlaunch", "fill", &text("rainbow")).is_some());
    assert!(check_app_value("scootlaunch", "shadow", &toml::Value::Integer(1)).is_some());
    assert!(check_app_value("scootgreet", "shadow", &text("soft")).is_some());
    // The bar takes the same values under its own dotted keys, and refuses
    // the same bad ones. Unknown or refused tokens stay reported, never fatal.
    for style in SHADOW_STYLES {
        assert!(check_app_value("scootbar", "bar.shadow", &text(style)).is_none());
    }
    for style in FILL_STYLES {
        assert!(check_app_value("scootbar", "bar.fill", &text(style)).is_none());
    }
    assert!(check_app_value("scootbar", "bar.shadow", &text("huge")).is_some());
    assert!(check_app_value("scootbar", "bar.fill", &text("rainbow")).is_some());
    assert!(check_app_value("scootbar", "bar.shadow", &toml::Value::Integer(1)).is_some());
    assert!(check_app_value("scootbar", "shadow", &text("soft")).is_some());
}

#[test]
fn the_motion_keys_belong_to_the_notifier_and_the_launcher() {
    for key in ["motion", "motion-duration-ms", "motion-bounce"] {
        for app in ["scootnotify", "scootlaunch"] {
            assert!(allows(app, key), "{app} {key}");
        }
        for other in ["scootgreet", "scootbar"] {
            assert!(!allows(other, key), "{other} {key}");
        }
    }
    // The card transitions are the notifier's alone.
    for key in ["card.enter", "card.exit"] {
        assert!(allows("scootnotify", key), "{key}");
        for other in ["scootlaunch", "scootgreet", "scootbar"] {
            assert!(!allows(other, key), "{other} {key}");
        }
    }
    let text = |s: &str| toml::Value::String(s.to_string());
    for curve in MOTION_CURVES {
        for app in ["scootnotify", "scootlaunch"] {
            assert!(check_app_value(app, "motion", &text(curve)).is_none());
        }
    }
    assert!(check_app_value("scootlaunch", "motion", &text("bouncy")).is_some());
    for style in CARD_MOTIONS {
        for key in ["card.enter", "card.exit"] {
            assert!(check_app_value("scootnotify", key, &text(style)).is_none());
        }
    }
    for (key, bad) in [
        ("motion", "bouncy"),
        ("motion", "Spring"),
        ("card.enter", "none"),
        ("card.exit", "spin"),
    ] {
        assert!(
            check_app_value("scootnotify", key, &text(bad)).is_some(),
            "{key}={bad}"
        );
    }
}

#[test]
fn motion_numbers_are_range_checked() {
    let int = toml::Value::Integer;
    let float = toml::Value::Float;
    for app in ["scootnotify", "scootlaunch"] {
        for ok in [0, 1, 380, 1000] {
            assert!(check_app_value(app, "motion-duration-ms", &int(ok)).is_none());
        }
        for bad in [-1, 1001, i64::MAX, i64::MIN] {
            assert!(
                check_app_value(app, "motion-duration-ms", &int(bad)).is_some(),
                "{app} {bad}"
            );
        }
        assert!(check_app_value(app, "motion-bounce", &float(0.5)).is_none());
        assert!(check_app_value(app, "motion-bounce", &float(1.5)).is_some());
    }
    assert!(
        check_app_value(
            "scootnotify",
            "motion-duration-ms",
            &toml::Value::String("fast".into())
        )
        .is_some()
    );
    for ok in [0.0, 0.12, 1.0] {
        assert!(check_app_value("scootnotify", "motion-bounce", &float(ok)).is_none());
    }
    for bad in [-0.1, 1.01, f64::NAN, f64::INFINITY] {
        assert!(
            check_app_value("scootnotify", "motion-bounce", &float(bad)).is_some(),
            "{bad}"
        );
    }
}

#[test]
fn a_bad_bounce_says_bounce_not_opacity() {
    let reason = check_app_value("scootnotify", "motion-bounce", &toml::Value::Integer(2))
        .expect("refused")
        .to_string();
    assert!(reason.contains("0 to 1"), "{reason}");
    assert!(!reason.contains("opaque"), "{reason}");
    // The bounds are the same as an opacity's: whole 0 and 1 pass.
    for ok in [0, 1] {
        assert!(
            check_app_value("scootnotify", "motion-bounce", &toml::Value::Integer(ok)).is_none()
        );
    }
}

#[test]
fn the_card_style_and_title_keys_belong_to_the_notifier_alone() {
    for key in [
        "card.mark",
        "card.shape",
        "card.app-accent",
        "title.weight",
        "title.spacing",
    ] {
        assert!(allows("scootnotify", key), "{key}");
        for other in ["scootlaunch", "scootgreet", "scootbar", "scoot"] {
            assert!(!allows(other, key), "{other} {key}");
        }
    }
    let text = |s: &str| toml::Value::String(s.to_string());
    for mark in CARD_MARKS {
        assert!(check_app_value("scootnotify", "card.mark", &text(mark)).is_none());
    }
    for shape in CARD_SHAPES {
        assert!(check_app_value("scootnotify", "card.shape", &text(shape)).is_none());
    }
    for (key, bad) in [
        ("card.mark", "ribbon"),
        ("card.mark", "Stripe"),
        ("card.mark", ""),
        ("card.shape", "round"),
        ("card.shape", "64"),
    ] {
        assert!(
            check_app_value("scootnotify", key, &text(bad)).is_some(),
            "{key}={bad}"
        );
    }
}

#[test]
fn the_app_accent_is_a_boolean_and_nothing_else() {
    for ok in [true, false] {
        assert!(
            check_app_value("scootnotify", "card.app-accent", &toml::Value::Boolean(ok)).is_none()
        );
    }
    for bad in [
        toml::Value::Integer(1),
        toml::Value::String("true".into()),
        toml::Value::Float(1.0),
    ] {
        assert!(check_app_value("scootnotify", "card.app-accent", &bad).is_some());
    }
}

#[test]
fn title_numbers_are_range_checked() {
    let int = toml::Value::Integer;
    for ok in [100, 400, 700, 900] {
        assert!(check_app_value("scootnotify", "title.weight", &int(ok)).is_none());
    }
    for bad in [-1, 0, 99, 901, i64::MAX, i64::MIN] {
        assert!(
            check_app_value("scootnotify", "title.weight", &int(bad)).is_some(),
            "{bad}"
        );
    }
    for ok in [-2, 0, 1, 8] {
        assert!(check_app_value("scootnotify", "title.spacing", &int(ok)).is_none());
    }
    for bad in [-3, 9, i64::MAX, i64::MIN] {
        assert!(
            check_app_value("scootnotify", "title.spacing", &int(bad)).is_some(),
            "{bad}"
        );
    }
    assert!(check_app_value("scootnotify", "title.spacing", &toml::Value::Float(1.5)).is_some());
}

#[test]
fn the_shared_radius_bound_is_unchanged_for_every_app() {
    // Pill is a notifier token, not a wider radius: nothing that reads
    // `radius` (the shell apps, scoot's corner_radius, the config) sees a
    // new bound.
    for app in ["scootnotify", "scootlaunch", "scootgreet"] {
        assert!(check_app_value(app, "radius", &toml::Value::Integer(64)).is_none());
        assert!(check_app_value(app, "radius", &toml::Value::Integer(65)).is_some());
    }
}

#[test]
fn user_tables_map_to_the_dotted_look_keys() {
    for key in EFFECTS_TABLE_KEYS {
        assert_eq!(user_effect_key("scootnotify", "effects", key), Some(*key));
        assert_eq!(user_effect_key("scootlaunch", "effects", key), Some(*key));
    }
    assert_eq!(
        user_effect_key("scootnotify", "card", "mark"),
        Some("card.mark")
    );
    assert_eq!(
        user_effect_key("scootnotify", "title", "weight"),
        Some("title.weight")
    );
    // Every key the notifier reads from the look is reachable from its
    // user tables, and nothing else is: one list, two entry points.
    for dotted in NOTIFY_DEPTH_KEYS {
        let (table, key) = dotted.split_once('.').unwrap_or(("effects", dotted));
        assert_eq!(user_effect_key("scootnotify", table, key), Some(*dotted));
    }
    // The bar's user `[effects]` shares the look's allowlist: `shadow` and
    // `fill` map to the bar's dotted keys, never a copy.
    assert_eq!(
        user_effect_key("scootbar", "effects", "shadow"),
        Some("bar.shadow")
    );
    assert_eq!(
        user_effect_key("scootbar", "effects", "fill"),
        Some("bar.fill")
    );
}

#[test]
fn user_tables_never_reach_the_looks_own_keys() {
    // `[effects]` is the five tokens, not the dotless presentation keys.
    for key in ["font", "font-size", "opacity", "radius", "card.mark"] {
        assert_eq!(
            user_effect_key("scootnotify", "effects", key),
            None,
            "{key}"
        );
        assert_eq!(
            user_effect_key("scootlaunch", "effects", key),
            None,
            "{key}"
        );
    }
    // The look's color and icon tables are not user tables.
    for table in ["colors", "icons", "apps", "", "Effects"] {
        assert_eq!(user_effect_key("scootnotify", table, "theme"), None);
        assert_eq!(user_effect_key("scootnotify", table, "accent"), None);
    }
    // The launcher has no cards or titles; the bar has no cards, titles or
    // motion (its entrance lands later). Apps outside the shell trio read none.
    assert_eq!(user_effect_key("scootlaunch", "card", "mark"), None);
    assert_eq!(user_effect_key("scootlaunch", "title", "weight"), None);
    assert_eq!(user_effect_key("scootbar", "card", "mark"), None);
    assert_eq!(user_effect_key("scootbar", "title", "weight"), None);
    assert_eq!(user_effect_key("scootbar", "effects", "motion"), None);
    assert_eq!(
        user_effect_key("scootbar", "effects", "motion-duration-ms"),
        None
    );
    assert_eq!(
        user_effect_key("scootbar", "effects", "motion-bounce"),
        None
    );
    assert_eq!(user_effect_key("scootbar", "bar", "shadow"), None);
    for app in ["scootgreet", "scoot", "nonesuch"] {
        assert_eq!(user_effect_key(app, "effects", "shadow"), None, "{app}");
    }
    // The bar's own depth keys map; nothing else does.
    for key in ["font", "font-size", "opacity", "radius", "card.mark"] {
        assert_eq!(user_effect_key("scootbar", "effects", key), None, "{key}");
    }
    // Hostile keys: long, non-ASCII, dotted.
    assert_eq!(
        user_effect_key("scootnotify", "card", &"x".repeat(5000)),
        None
    );
    assert_eq!(user_effect_key("scootnotify", "card", "mark\u{e9}"), None);
    assert_eq!(user_effect_key("scootnotify", "card", "x.mark"), None);
    assert_eq!(user_effect_key("scootnotify", "effects", "card.mark"), None);
}

#[test]
fn font_file_is_the_tools_key_never_an_author_override() {
    for app in ["scootnotify", "scootlaunch", "scootgreet", "scootbar"] {
        assert!(!allows(app, FONT_FILE_KEY), "{app}");
        assert!(
            check_app_value(app, FONT_FILE_KEY, &toml::Value::String("/x.ttf".into())).is_some()
        );
    }
}

#[test]
fn a_font_file_value_is_an_absolute_path_string() {
    let text = |s: &str| toml::Value::String(s.to_owned());
    assert!(check_font_file(&text("/nix/store/x/DejaVuSans.ttf")).is_none());
    assert!(check_font_file(&text("/a b/C d.otf")).is_none());
    for bad in ["", "relative.ttf", "./x.ttf", "~/x.ttf", "/x\0y.ttf"] {
        assert!(check_font_file(&text(bad)).is_some(), "{bad:?}");
    }
    assert!(check_font_file(&text(&format!("/{}", "a".repeat(FONT_FILE_MAX)))).is_some());
    assert!(check_font_file(&toml::Value::Integer(1)).is_some());
    assert!(check_font_file(&toml::Value::Array(Vec::new())).is_some());
}
