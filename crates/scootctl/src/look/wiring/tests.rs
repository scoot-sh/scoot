//! The wiring check against scratch dirs: statuses, real paths, and the
//! absent-leaf silence. No environment is read: every dir is explicit.

use super::{WiringInput, WiringStatus, check_wiring};
use scoot_look::state::Record;

fn record(name: &str, rendered: &std::path::Path) -> Record {
    Record {
        name: name.to_owned(),
        rendered: rendered.display().to_string(),
        generation: 7,
        chosen_by: Some("cli".to_owned()),
        declared_default: None,
        toolkit: None,
        dconf_wrote: std::collections::BTreeMap::new(),
        dconf_orig: std::collections::BTreeMap::new(),
    }
}

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("scoot-wiring-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch dir")
}

fn status_of(items: &[super::WiringItem], app: &str) -> WiringStatus {
    items
        .iter()
        .find(|item| item.app == app)
        .unwrap_or_else(|| panic!("a {app} item"))
        .status
}

fn detail_of(items: &[super::WiringItem], app: &str) -> String {
    items
        .iter()
        .find(|item| item.app == app)
        .unwrap_or_else(|| panic!("a {app} item"))
        .detail
        .clone()
}

#[test]
fn missing_wiring_names_the_exact_fix_with_real_paths() {
    let root = scratch("missing");
    let state = root.path().join("state");
    let rendered = root.path().join("rendered");
    let config = root.path().join("config");
    let data = root.path().join("data");
    for dir in [&state, &rendered, &config, &data] {
        std::fs::create_dir_all(dir).expect("a dir");
    }
    // A look with every leaf: foot, mako, wob, fcitx5, helix, btop,
    // starship, fuzzel, swaylock.
    std::fs::write(rendered.join("foot.ini"), "leaf").expect("foot");
    std::fs::write(rendered.join("mako"), "leaf").expect("mako");
    std::fs::write(rendered.join("wob.ini"), "leaf").expect("wob");
    std::fs::create_dir_all(rendered.join("fcitx5")).expect("fcitx5 dir");
    std::fs::write(rendered.join("fcitx5/theme.conf"), "leaf").expect("fcitx5");
    std::fs::create_dir_all(rendered.join("apps")).expect("apps");
    for leaf in ["apps/helix.toml", "apps/btop.theme", "apps/starship.toml"] {
        std::fs::write(rendered.join(leaf), "leaf").expect("a leaf");
    }
    std::fs::write(rendered.join("fuzzel.ini"), "leaf").expect("fuzzel");
    std::fs::write(rendered.join("swaylock"), "leaf").expect("swaylock");
    let rec = record("moonrise", &rendered);
    let input = WiringInput {
        state_dir: &state,
        record: &rec,
        rendered: &rendered,
        config_home: Some(&config),
        data_home: Some(&data),
    };
    let items = check_wiring(&input);
    for app in ["foot", "mako", "fcitx5", "helix", "btop", "starship"] {
        assert_eq!(status_of(&items, app), WiringStatus::Missing, "{app}");
        let detail = detail_of(&items, app);
        assert!(!detail.contains('~'), "a real path, not `~`: {detail}");
        assert!(
            detail.contains(&state.display().to_string())
                || detail.contains(&config.display().to_string())
                || detail.contains(&data.display().to_string()),
            "the detail points at the scratch dirs: {detail}"
        );
    }
    for app in ["wob", "nightlight", "fuzzel", "swaylock"] {
        assert_eq!(status_of(&items, app), WiringStatus::Manual, "{app}");
    }
}

#[test]
fn wired_files_report_wired() {
    let root = scratch("wired");
    let state = root.path().join("state");
    let rendered = root.path().join("rendered");
    let config = root.path().join("config");
    let data = root.path().join("data");
    for dir in [&state, &rendered, &config, &data] {
        std::fs::create_dir_all(dir).expect("a dir");
    }
    std::fs::write(rendered.join("foot.ini"), "leaf").expect("foot");
    std::fs::write(rendered.join("mako"), "leaf").expect("mako");
    std::fs::write(rendered.join("wob.ini"), "leaf").expect("wob");
    std::fs::create_dir_all(rendered.join("apps")).expect("apps");
    std::fs::write(rendered.join("apps/helix.toml"), "leaf").expect("helix");
    std::fs::write(rendered.join("apps/btop.theme"), "leaf").expect("btop");
    std::fs::write(rendered.join("apps/starship.toml"), "leaf").expect("starship");
    std::fs::write(rendered.join("fuzzel.ini"), "leaf").expect("fuzzel");
    std::fs::write(rendered.join("swaylock"), "leaf").expect("swaylock");
    // No fcitx5 leaf rendered: fcitx5 reports absent, never missing.
    std::fs::create_dir_all(config.join("foot")).expect("foot dir");
    std::fs::write(
        config.join("foot/foot.ini"),
        "include=/state/look/foot.ini\n",
    )
    .expect("foot.ini");
    std::fs::create_dir_all(config.join("mako")).expect("mako dir");
    std::fs::write(config.join("mako/config"), "include=/state/look/mako\n").expect("mako config");
    std::fs::create_dir_all(config.join("helix/themes")).expect("helix dir");
    std::fs::write(config.join("helix/themes/scoot-look.toml"), "theme").expect("helix theme");
    std::fs::create_dir_all(config.join("btop/themes")).expect("btop dir");
    std::fs::write(config.join("btop/themes/scoot-look.theme"), "theme").expect("btop theme");
    std::fs::write(config.join("starship.toml"), "config").expect("starship");
    let rec = record("moonrise", &rendered);
    let input = WiringInput {
        state_dir: &state,
        record: &rec,
        rendered: &rendered,
        config_home: Some(&config),
        data_home: Some(&data),
    };
    let items = check_wiring(&input);
    for app in ["foot", "mako", "helix", "btop", "starship"] {
        assert_eq!(status_of(&items, app), WiringStatus::Wired, "{app}");
    }
    assert_eq!(status_of(&items, "fcitx5"), WiringStatus::Absent);
}

#[test]
fn commented_includes_do_not_count_as_wired() {
    let root = scratch("commented");
    let state = root.path().join("state");
    let rendered = root.path().join("rendered");
    let config = root.path().join("config");
    let data = root.path().join("data");
    for dir in [&state, &rendered, &config, &data] {
        std::fs::create_dir_all(dir).expect("a dir");
    }
    std::fs::write(rendered.join("foot.ini"), "leaf").expect("foot");
    std::fs::create_dir_all(config.join("foot")).expect("foot dir");
    std::fs::write(
        config.join("foot/foot.ini"),
        "#include=/state/look/foot.ini\n",
    )
    .expect("foot.ini");
    let rec = record("moonrise", &rendered);
    let input = WiringInput {
        state_dir: &state,
        record: &rec,
        rendered: &rendered,
        config_home: Some(&config),
        data_home: Some(&data),
    };
    let items = check_wiring(&input);
    assert_eq!(status_of(&items, "foot"), WiringStatus::Missing);
}

#[test]
fn a_leafless_look_reports_absent_never_missing() {
    let root = scratch("leafless");
    let state = root.path().join("state");
    let rendered = root.path().join("rendered");
    let config = root.path().join("config");
    let data = root.path().join("data");
    for dir in [&state, &rendered, &config, &data] {
        std::fs::create_dir_all(dir).expect("a dir");
    }
    let rec = record("flat", &rendered);
    let input = WiringInput {
        state_dir: &state,
        record: &rec,
        rendered: &rendered,
        config_home: Some(&config),
        data_home: Some(&data),
    };
    let items = check_wiring(&input);
    for app in [
        "foot", "mako", "wob", "fcitx5", "helix", "btop", "starship", "fuzzel", "swaylock",
    ] {
        assert_eq!(status_of(&items, app), WiringStatus::Absent, "{app}");
    }
    // nightlight is executable state, not a look leaf: always a manual
    // step, never absent.
    assert_ne!(
        status_of(&items, "nightlight"),
        WiringStatus::Absent,
        "nightlight stays manual"
    );
}

#[test]
fn starship_config_env_counts_as_wired() {
    let root = scratch("starship-env");
    let state = root.path().join("state");
    let rendered = root.path().join("rendered");
    let config = root.path().join("config");
    for dir in [&state, &rendered, &config] {
        std::fs::create_dir_all(dir).expect("a dir");
    }
    std::fs::create_dir_all(rendered.join("apps")).expect("apps");
    std::fs::write(rendered.join("apps/starship.toml"), "leaf").expect("starship");
    let rec = record("moonrise", &rendered);
    // STARSHIP_CONFIG is read from the process environment: guard it so
    // the suite never depends on the developer's own shell.
    let stash = std::env::var_os("STARSHIP_CONFIG");
    unsafe { std::env::set_var("STARSHIP_CONFIG", root.path().join("starship.toml")) };
    let pointed = root.path().join("starship.toml");
    std::fs::write(&pointed, "config").expect("starship config");
    let input = WiringInput {
        state_dir: &state,
        record: &rec,
        rendered: &rendered,
        config_home: Some(&config),
        data_home: None,
    };
    let items = check_wiring(&input);
    assert_eq!(status_of(&items, "starship"), WiringStatus::Wired);
    match stash {
        Some(value) => unsafe { std::env::set_var("STARSHIP_CONFIG", value) },
        None => unsafe { std::env::remove_var("STARSHIP_CONFIG") },
    }
}
