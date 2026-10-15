//! The style-only reload's gate ([`super::style_only`]): which config
//! differences keep every module running, and which restart them. Plain
//! data comparisons over two [`Config`]s — no compositor, no files, and no
//! Nix — so they run anywhere the crate's unit tests do.

use super::style_only;
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

// ---- The fast path keeps the running modules ----
//
// What `apply_style` does with the module vec: nothing. These tests pin
// that contract through a real child (`sh`, `sleep`, which every test
// machine has): with only the theme changed the gate says style-only, no
// `start` runs, and the child, its shown output and its revision are the
// same ones. Needs the `exec` feature: a real command to keep.

/// A scratch directory, removed on drop.
#[cfg(feature = "exec")]
struct Dir(std::path::PathBuf);

#[cfg(feature = "exec")]
impl Dir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scootbar-style-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn file(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

#[cfg(feature = "exec")]
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether process `pid` is alive, through the shell every test machine
/// has (`kill` is a POSIX special builtin; notably no `/proc`, so this
/// holds on macOS as well as Linux).
#[cfg(feature = "exec")]
fn alive(pid: u32) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("kill -0 {pid}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
#[cfg(feature = "exec")]
fn a_colors_only_reload_keeps_the_child_its_text_and_its_revision() {
    use std::time::Duration;

    use crate::layout::Layout;
    use crate::modules::custom::{Custom, Kind, intern};
    use crate::modules::exec::{Restart, Settings};
    use crate::modules::harness::drive_placed;
    use crate::modules::payload::Format;
    use crate::modules::{OutputView, View, start};

    let dir = Dir::new("keep");
    let pidfile = dir.file("pid");
    let id = intern("style-keep").unwrap();
    let settings = Settings {
        command: vec![
            "sh".into(),
            "-c".into(),
            format!("echo $$ > {}; echo started; sleep 331", pidfile.display()),
        ],
        format: Format::Text,
        placeholder: "ph".into(),
        restart: Restart {
            first: Duration::from_millis(20),
            max: Duration::from_millis(80),
            stable: Duration::from_secs(10),
        },
        icon: None,
        show_text: true,
    };
    let lists = Layout {
        left: vec![id],
        center: vec![],
        right: vec![],
        ..Layout::default()
    };
    let modules = crate::modules::Settings {
        custom: vec![Custom {
            id,
            kind: Kind::Exec(settings),
        }],
        ..Default::default()
    };
    let old_config = Config {
        modules,
        layout: lists,
        ..Default::default()
    };
    // The running bar: the `exec` with its child up and its line shown.
    let mut said = Vec::new();
    let mut old = start(
        &old_config.outputs.to_start(&old_config.layout),
        &old_config.modules,
        &mut Vec::new(),
        &mut |id, why| said.push(format!("{id}: {why}")),
    );
    assert!(said.is_empty(), "unexpected unavailable: {said:?}");
    let text = |placed: &crate::modules::Placed| {
        let mut view = View::default();
        placed.module.view(&OutputView { name: None }, &mut view);
        view.text().to_owned()
    };
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the command wrote its pid")
        .trim()
        .parse()
        .expect("a pid");
    let revision = old[0].revision;
    assert!(revision > 0, "the shown line bumped the revision");
    // Only the theme changed: the gate says style-only, so the fast path
    // starts nothing and hands the vec over as it is.
    let mut new_config = old_config.clone();
    new_config.theme.accent = Color { r: 1, g: 2, b: 3 };
    assert!(style_only(&old_config, &new_config));
    let new = old;
    assert_eq!(new.len(), 1, "no module was replaced");
    assert_eq!(text(&new[0]), "started", "its shown output moved over");
    assert_eq!(new[0].revision, revision, "its revision moved over");
    let pid2: u32 = std::fs::read_to_string(&pidfile)
        .expect("the command wrote its pid")
        .trim()
        .parse()
        .expect("a pid");
    assert_eq!(pid2, pid, "no second child was started");
    assert!(alive(pid), "the child is still running");
    drop(new);
    assert!(!alive(pid), "the kept child dies with its module");
}
