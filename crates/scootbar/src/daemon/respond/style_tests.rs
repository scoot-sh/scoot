//! The style-only reload's gate ([`super::style_only`]) and routing
//! ([`super::takes_fast_path`]): which config differences keep every module
//! running, which restart them, and why a left-out module takes the full
//! path even for a style-only file. Plain data comparisons over two
//! [`Config`]s — no compositor, no files, and no Nix — so they run anywhere
//! the crate's unit tests do.

use super::{style_only, takes_fast_path};
use crate::action::Bindings;
use crate::color::Color;
use crate::config::Config;

/// `Config::default`, and it again with `change` applied.
fn changed(change: impl FnOnce(&mut Config)) -> (Config, Config) {
    let old = Config::default();
    let mut new = old.clone();
    change(&mut new);
    (old, new)
}

#[test]
fn an_identical_config_is_style_only() {
    let config = Config::default();
    assert!(style_only(&config, &config));
}

#[test]
fn every_theme_token_alone_is_style_only() {
    let (old, mut new) = changed(|_| {});
    new.theme = crate::theme::Theme {
        background: Color { r: 1, g: 2, b: 3 },
        foreground: Color { r: 4, g: 5, b: 6 },
        accent: Color { r: 7, g: 8, b: 9 },
        hover: Color {
            r: 10,
            g: 11,
            b: 12,
        },
        dim: Color {
            r: 13,
            g: 14,
            b: 15,
        },
        urgent: Color {
            r: 16,
            g: 17,
            b: 18,
        },
    };
    assert_ne!(old.theme, new.theme, "the tokens under test differ");
    assert!(style_only(&old, &new));
}

#[test]
fn bar_style_keys_and_the_font_are_style_only() {
    let changes: [fn(&mut Config); 9] = [
        |new: &mut Config| new.radius += 1,
        |new: &mut Config| new.opacity = new.opacity.saturating_sub(1),
        |new: &mut Config| {
            new.font = Some(std::path::PathBuf::from("/fonts/test.ttf"));
        },
        |new: &mut Config| {
            new.fallback_fonts = vec![std::path::PathBuf::from("/fonts/fallback.ttf")];
        },
        |new: &mut Config| new.font_size += 1,
        |new: &mut Config| new.layout.padding += 1,
        |new: &mut Config| new.layout.spacing += 1,
        |new: &mut Config| new.layout.separator += 1,
        |new: &mut Config| new.layout.margins.push(("clock", 2)),
    ];
    for change in changes {
        let (old, new) = changed(change);
        assert!(style_only(&old, &new));
    }
}

#[test]
#[cfg(feature = "popup")]
fn popup_style_keys_are_style_only() {
    let changes: [fn(&mut Config); 2] = [
        |new: &mut Config| new.popup_radius += 1,
        |new: &mut Config| new.tooltip_delay += 1,
    ];
    for change in changes {
        let (old, new) = changed(change);
        assert!(style_only(&old, &new));
    }
}

#[test]
fn bar_geometry_is_not_style_only() {
    use crate::bar::{Edge, Layer};
    let changes: [fn(&mut Config); 5] = [
        |new: &mut Config| new.bar.height += 1,
        |new: &mut Config| {
            new.bar.edge = match new.bar.edge {
                Edge::Top => Edge::Bottom,
                Edge::Bottom => Edge::Top,
            };
        },
        |new: &mut Config| new.bar.margin.top += 1,
        |new: &mut Config| {
            new.bar.layer = match new.bar.layer {
                Layer::Top => Layer::Overlay,
                _ => Layer::Top,
            };
        },
        |new: &mut Config| new.bar.exclusive = !new.bar.exclusive,
    ];
    for change in changes {
        let (old, new) = changed(change);
        assert!(!style_only(&old, &new));
    }
}

#[test]
fn a_module_list_change_is_not_style_only() {
    let changes: [fn(&mut Config); 3] = [
        |new: &mut Config| new.layout.left.push("clock"),
        |new: &mut Config| new.layout.center.push("clock"),
        |new: &mut Config| new.layout.right.push("clock"),
    ];
    for change in changes {
        let (old, new) = changed(change);
        assert!(!style_only(&old, &new));
    }
}

#[test]
fn an_outputs_policy_change_is_not_style_only() {
    let (old, new) = changed(|new| {
        new.outputs.select = crate::policy::Select::Named(vec!["DP-1".to_owned()]);
    });
    assert!(!style_only(&old, &new));
    // A per-output override is policy too: even a font-size override takes
    // the full reload, fail-closed.
    let (old, new) = changed(|new| {
        new.outputs.overrides.push(crate::policy::Override {
            name: "DP-1".to_owned(),
            bar: crate::policy::BarOverride::default(),
            font_size: Some(20),
            modules: None,
        });
    });
    assert!(!style_only(&old, &new));
}

/// Every style field at once is still style-only: this pins the reset
/// list in [`super::style_only`] as complete. A new style field added to
/// the config without listing it there fails this test once the test
/// changes it too — and until then the new field takes the full reload,
/// never the fast path.
#[test]
fn every_style_field_at_once_is_style_only() {
    let (old, new) = changed(|new| {
        new.theme.accent = Color { r: 1, g: 2, b: 3 };
        new.theme.background = Color { r: 4, g: 5, b: 6 };
        new.font = Some(std::path::PathBuf::from("/fonts/test.ttf"));
        new.fallback_fonts = vec![std::path::PathBuf::from("/fonts/fallback.ttf")];
        new.font_size += 1;
        new.radius += 1;
        #[cfg(feature = "popup")]
        {
            new.popup_radius += 1;
            new.tooltip_delay += 1;
        }
        new.opacity = new.opacity.saturating_sub(1);
        new.layout.padding += 1;
        new.layout.spacing += 1;
        new.layout.separator = 1;
        new.layout.margins.push(("clock", 2));
    });
    assert_ne!(old.theme, new.theme, "the theme under test differs");
    assert!(style_only(&old, &new));
}

/// Every non-style field at once takes the full reload: the bar, the
/// module lists, the outputs policy and the module tables.
#[test]
fn every_non_style_field_at_once_is_not_style_only() {
    let (old, new) = changed(|new| {
        new.bar.height += 1;
        new.layout.left.push("clock");
        new.outputs.select = crate::policy::Select::Named(vec!["DP-1".to_owned()]);
        new.modules.bindings.push(("clock", Bindings::default()));
    });
    assert!(!style_only(&old, &new));
}

#[test]
fn a_bindings_change_is_not_style_only() {
    let (old, new) = changed(|new| {
        new.modules.bindings.push(("clock", Bindings::default()));
    });
    assert!(!style_only(&old, &new));
}

#[test]
#[cfg(feature = "clock")]
fn a_module_table_change_is_not_style_only() {
    let (old, new) = changed(|new| {
        new.modules.clock.format =
            crate::modules::clock::format::Format::parse("%H:%M").expect("a valid clock format");
    });
    assert_ne!(
        old.modules.clock.format, new.modules.clock.format,
        "the format under test differs"
    );
    assert!(!style_only(&old, &new));
}

/// The fast-path routing ([`super::takes_fast_path`]): style-only takes it,
/// anything else does not — and a module left out as unavailable at the last
/// start takes the full path even for a style-only file, so the reload
/// retries it.
#[test]
fn a_left_out_module_takes_the_full_path_even_for_a_style_only_file() {
    let (old, new) = changed(|new| {
        new.theme.accent = Color { r: 1, g: 2, b: 3 };
    });
    assert!(style_only(&old, &new), "the file differs in style alone");
    assert!(takes_fast_path(&old, &new, false));
    assert!(
        !takes_fast_path(&old, &new, true),
        "a left-out module is retried, not kept absent"
    );
    let (old, new) = changed(|new| {
        new.bar.height += 1;
    });
    assert!(!style_only(&old, &new));
    assert!(!takes_fast_path(&old, &new, false));
    assert!(!takes_fast_path(&old, &new, true));
}
