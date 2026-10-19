//! The five shipped looks under `looks/`: each validates as a shipped look
//! and renders the first-party fragments plus the greeter's and the launcher's.
//!
//! The gate behind the theming page's "a check renders every look into
//! every target and fails on a missing role": CI runs this on every change
//! under `looks/` (the classify job's path filter). Pure crate calls only:
//! listing, validating and rendering a look is plain files plus this crate,
//! with no Nix helper involved, so this test takes no `PATH` dependency and
//! mutates none (unlike `non_nix.rs`, which scrubs `PATH` to prove the same
//! point for one synthetic look; scrubbing here too would race that test
//! under `cargo test`'s shared process).

use std::collections::BTreeMap;
use std::path::PathBuf;

use scoot_look::{
    Origin, RenderInputs, check_contrast, parse_kv_config, render_bar, render_fuzzel, render_greet,
    render_launch, render_notify, render_scoot, render_swaylock, resolve,
};

/// Every directory under `looks/` with a `look.toml`.
const SHIPPED: &[&str] = &[
    "ginger-night",
    "moonrise",
    "music-desk",
    "radial-burst",
    "vinyl-sunset",
];

fn looks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("looks")
}

fn read_raw(name: &str) -> (scoot_look::RawLook, Vec<u8>) {
    let path = looks_dir().join(name).join("look.toml");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let raw = scoot_look::parse_look_toml(&bytes).unwrap_or_else(|errors| {
        panic!(
            "{name}/look.toml refuses to parse: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    (raw, bytes)
}

fn resolved(name: &str) -> scoot_look::ResolvedLook {
    let (raw, _) = read_raw(name);
    assert!(
        raw.extends.is_none(),
        "{name}: a shipped look stands alone, never extends another"
    );
    if let Some(e) = scoot_look::check_name_matches_dir(&raw, name) {
        panic!("{name}: {e}");
    }
    assert!(
        scoot_look::validate_for_origin(&raw, Origin::Shipped).is_empty(),
        "{name}: shipped origin adds errors"
    );
    assert!(
        scoot_look::validate_apps_allowlist(&raw.apps).is_empty(),
        "{name}: non-allowlisted [apps.*] key"
    );
    let map = BTreeMap::from([(name.to_string(), (raw, Origin::Shipped))]);
    resolve(&map, name).unwrap_or_else(|e| panic!("{name}: does not resolve: {e}"))
}

#[test]
fn shipped_set_is_exactly_the_five_looks() {
    let dir = looks_dir();
    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("looks/ entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| dir.join(name).join("look.toml").is_file())
        .collect();
    found.sort();
    let mut expected: Vec<String> = SHIPPED.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "looks/ holds exactly the five shipped looks"
    );
}

#[test]
fn every_shipped_look_validates_and_renders() {
    for name in SHIPPED {
        let look = resolved(name);
        // A shipped look is refused below a contrast bar: the gate proves
        // all five clear it, with the pair and the ratio named on failure.
        let report = check_contrast(&look, Origin::Shipped);
        let refusals = report.refusals();
        assert!(
            refusals.is_empty(),
            "{name}: contrast refusals: {}",
            refusals
                .iter()
                .map(|p| format!("{} is {} (bar {})", p.name, p.ratio, p.threshold))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let dir = looks_dir().join(name);
        let inputs = RenderInputs {
            wallpaper_abs: look.wallpaper.as_ref().and_then(|w| {
                w.image.as_ref().map(|_| {
                    dir.join("wallpaper.png")
                        .to_str()
                        .expect("look path is UTF-8")
                        .to_string()
                })
            }),
            ..RenderInputs::default()
        };
        let scoot = render_scoot(&look, &inputs)
            .unwrap_or_else(|e| panic!("{name}: scoot.toml does not render: {e}"));
        let bar = render_bar(&look, &inputs)
            .unwrap_or_else(|e| panic!("{name}: bar.toml does not render: {e}"));
        let scoot: toml::Value =
            toml::from_str(&scoot).unwrap_or_else(|e| panic!("{name}: scoot.toml reparse: {e}"));
        let bar: toml::Value =
            toml::from_str(&bar).unwrap_or_else(|e| panic!("{name}: bar.toml reparse: {e}"));
        // D15 decided the radius yes (2026-10-14) with look-layer-scoot's
        // pixman benchmark as the condition: ~+5-6% per frame (~6-12 us at
        // 1, 10 and 50 windows on pixman, release build), acceptable, so the
        // shipped looks carry the examples' radius, locked per look. Bar
        // opacity stays at 1.0: blending a translucent bar over background
        // and windows on pixman was never measured (no layer-surface
        // translucency bench exists), so the examples' opacity stays a
        // comment in each look.toml until that benchmark passes.
        let corners = match *name {
            "ginger-night" => 14,
            "moonrise" => 14,
            "music-desk" => 14,
            "radial-burst" => 16,
            "vinyl-sunset" => 12,
            _ => panic!("unexpected shipped look {name}"),
        };
        assert_eq!(
            scoot["appearance"]["corner_radius"].as_integer(),
            Some(corners),
            "{name}: corner_radius holds the example value"
        );
        assert_eq!(
            bar["bar"]["radius"].as_integer(),
            Some(corners.min(14)),
            "{name}: bar radius holds the example value (clamped to the bar limit)"
        );
        assert_eq!(
            bar["bar"]["opacity"].as_float(),
            Some(1.0),
            "{name}: bar opacity holds at 1.0 until the translucency benchmark passes"
        );
        // Without `--ui-font-file` the bar fragment carries no `font` key
        // (scootbar's is a file path, not the family name), so the bar
        // falls back to its well-known paths.
        let table = bar["bar"]
            .as_table()
            .unwrap_or_else(|| panic!("{name}: bar.toml has no [bar]"));
        assert!(
            !table.contains_key("font") && !table.contains_key("fallback-fonts"),
            "{name}: default render emits no font path"
        );
        // The daemon leaves render for every shipped look: the same
        // mapping the Nix modules used to inline, now from the tokens.
        // music-desk's pins below match the Nix content checks
        // (`nix/tests.nix` "mako config carries the overlay layer"): the
        // eval-time values and the renderer agree.
        let leaves = scoot_look::render_leaf_files(&look)
            .unwrap_or_else(|e| panic!("{name}: leaves do not render: {e}"));
        let leaf = |file: &str| {
            leaves
                .iter()
                .find(|(path, _)| path == file)
                .unwrap_or_else(|| panic!("{name}: no {file} leaf"))
                .1
                .clone()
        };
        let mako = leaf("mako");
        for line in [
            "layer=overlay",
            "[mode=do-not-disturb]",
            "invisible=1",
            "[urgency=critical]",
        ] {
            assert!(
                mako.lines().any(|l| l == line),
                "{name}: mako leaf misses `{line}`:\n{mako}"
            );
        }
        let wob = leaf("wob.ini");
        assert!(
            wob.lines().any(|l| l == "[style.muted]"),
            "{name}: wob leaf misses the muted style:\n{wob}"
        );
        let theme = leaf("fcitx5/theme.conf");
        assert!(
            theme.contains("[InputPanel/Background]"),
            "{name}: fcitx5 leaf misses its sections:\n{theme}"
        );
        if *name == "music-desk" {
            for line in [
                "background-color=#FCFBFB",
                "text-color=#1A2032",
                "border-color=#3D579A",
                "progress-color=over #3D579A",
                "border-color=#EE6F5E",
            ] {
                assert!(
                    mako.lines().any(|l| l == line),
                    "music-desk: mako leaf misses `{line}`:\n{mako}"
                );
            }
        }
        // The launcher and locker leaves render too, and reparse to the
        // same pairs: the gate behind "a check renders every look into
        // every target" covers the next-launch apps as well.
        let fuzzel = render_fuzzel(&look)
            .unwrap_or_else(|e| panic!("{name}: fuzzel.ini does not render: {e}"));
        let swaylock = render_swaylock(&look)
            .unwrap_or_else(|e| panic!("{name}: swaylock does not render: {e}"));
        let fuzzel =
            parse_kv_config(&fuzzel).unwrap_or_else(|e| panic!("{name}: fuzzel.ini reparse: {e}"));
        let swaylock =
            parse_kv_config(&swaylock).unwrap_or_else(|e| panic!("{name}: swaylock reparse: {e}"));
        let colors = fuzzel
            .get("colors")
            .unwrap_or_else(|| panic!("{name}: fuzzel.ini has no [colors]"));
        assert_eq!(
            colors.len(),
            7,
            "{name}: fuzzel.ini carries the seven CLI leaves"
        );
        let global = swaylock
            .get("")
            .unwrap_or_else(|| panic!("{name}: swaylock has section-less pairs"));
        assert_eq!(
            global.len(),
            14,
            "{name}: swaylock carries the fourteen lockTheme leaves"
        );
        // The overlays keep today's borders: fuzzel's border is the
        // focused ring, the locker's ring the same. (Hex case rides
        // through from the source, as in the TOML fragments.)
        let focus = look.palette.focus.trim_start_matches('#').to_string();
        assert!(
            colors
                .get("border")
                .is_some_and(|border| border.eq_ignore_ascii_case(&format!("{focus}ff"))),
            "{name}: fuzzel border is the focus ring: {:?}",
            colors.get("border")
        );
        assert!(
            global
                .get("ring-color")
                .is_some_and(|ring| ring.eq_ignore_ascii_case(&focus)),
            "{name}: swaylock ring is the focus ring: {:?}",
            global.get("ring-color")
        );
        // The greeter fragment renders for every shipped look too: all
        // thirteen shell-app roles, the UI sans, opaque, with the look's
        // radius. scootgreet reads it at system build time, never live.
        let greet = render_greet(&look)
            .unwrap_or_else(|e| panic!("{name}: greet.toml does not render: {e}"));
        let greet: toml::Value =
            toml::from_str(&greet).unwrap_or_else(|e| panic!("{name}: greet.toml reparse: {e}"));
        let colors = greet
            .get("colors")
            .and_then(|colors| colors.as_table())
            .unwrap_or_else(|| panic!("{name}: greet.toml has no [colors]"));
        assert_eq!(
            colors.len(),
            13,
            "{name}: greet.toml carries the thirteen shell-app roles"
        );
        assert_eq!(
            greet.get("opacity").and_then(|opacity| opacity.as_float()),
            Some(1.0),
            "{name}: the login screen stays opaque"
        );
        assert_eq!(
            greet.get("font").and_then(|font| font.as_str()),
            Some(look.fonts.sans.as_str()),
            "{name}: the greeter reads the UI sans"
        );
        // The launcher fragment renders for every shipped look too: all
        // thirteen shell-app roles, the UI font at its popup size, the
        // popups opacity, with the look's radius. scootlaunch reads it
        // at every open.
        let launch = render_launch(&look)
            .unwrap_or_else(|e| panic!("{name}: launch.toml does not render: {e}"));
        let launch: toml::Value =
            toml::from_str(&launch).unwrap_or_else(|e| panic!("{name}: launch.toml reparse: {e}"));
        let colors = launch
            .get("colors")
            .and_then(|colors| colors.as_table())
            .unwrap_or_else(|| panic!("{name}: launch.toml has no [colors]"));
        assert_eq!(
            colors.len(),
            13,
            "{name}: launch.toml carries the thirteen shell-app roles"
        );
        assert_eq!(
            launch.get("font").and_then(|font| font.as_str()),
            Some(look.fonts.ui.as_str()),
            "{name}: the panel reads the UI font"
        );
        // The notification fragment renders for every shipped look too:
        // the same thirteen roles, font, opacity and radius. scootnotify
        // reads it at start and on re-theme.
        let notify = render_notify(&look)
            .unwrap_or_else(|e| panic!("{name}: notify.toml does not render: {e}"));
        let notify: toml::Value =
            toml::from_str(&notify).unwrap_or_else(|e| panic!("{name}: notify.toml reparse: {e}"));
        let colors = notify
            .get("colors")
            .and_then(|colors| colors.as_table())
            .unwrap_or_else(|| panic!("{name}: notify.toml has no [colors]"));
        assert_eq!(
            colors.len(),
            13,
            "{name}: notify.toml carries the thirteen shell-app roles"
        );
    }
}

#[test]
fn shipped_contrast_warnings_are_the_known_dim_and_urgent_pairs() {
    // Design section 3.5: `dim` and `urgent` report but never refuse, and
    // one shipped look sits under 3.0 (music-desk dim 1.57 and urgent
    // 2.88; ratios round to one decimal). vinyl-sunset's 2026-10-17 retheme
    // to the maintainer's image lifted dim to 4.4 and urgent to 5.1, so it
    // no longer warns. Locked here so a retune shows up as a test change;
    // the maintainer call (retune or accept) rides in the PR, not in a
    // refusal.
    let mut below: Vec<(String, String, f64)> = Vec::new();
    for name in SHIPPED {
        let look = resolved(name);
        for pair in check_contrast(&look, Origin::Shipped).warnings() {
            below.push((name.to_string(), pair.name.to_string(), pair.ratio));
        }
    }
    assert_eq!(
        below,
        vec![
            ("music-desk".to_string(), "dim/background".to_string(), 1.6),
            (
                "music-desk".to_string(),
                "urgent/background".to_string(),
                2.9
            ),
        ]
    );
}

#[test]
fn shipped_static_leaves_read_and_shape_check() {
    // The terminal static leaves each shipped look carries: read from
    // `apps/` and shape-checked the way `apply` checks them before
    // writing them into the render. Locked per look, so a leaf added,
    // removed or corrupted shows up as a test change. music-desk ships
    // no btop theme (its tuned theme path is not portable, so its
    // `color_theme` stays commented out) and radial-burst ships no
    // terminal leaves at all.
    let expected: &[(&str, &[&str])] = &[
        (
            "ginger-night",
            &["apps/helix.toml", "apps/btop.theme", "apps/starship.toml"],
        ),
        (
            "moonrise",
            &["apps/helix.toml", "apps/btop.theme", "apps/starship.toml"],
        ),
        ("music-desk", &["apps/helix.toml", "apps/starship.toml"]),
        ("radial-burst", &[]),
        (
            "vinyl-sunset",
            &["apps/helix.toml", "apps/btop.theme", "apps/starship.toml"],
        ),
    ];
    for (name, leaves) in expected {
        let dir = looks_dir().join(name);
        for leaf in scoot_look::STATIC_LEAVES {
            let path = dir.join(leaf.path);
            if leaves.contains(&leaf.path) {
                let bytes = std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("{name}: cannot read {}: {e}", path.display()));
                scoot_look::check_static_leaf(leaf.path, &bytes).unwrap_or_else(|e| {
                    panic!("{name}: {} does not shape-check: {e}", path.display())
                });
                // A Helix theme that loads: the leaf must at least name
                // the selection (an invalid theme falls back to the
                // default with only a log line, proven live against
                // 25.07.1).
                if leaf.path == "apps/helix.toml" {
                    let text = String::from_utf8_lossy(&bytes);
                    assert!(
                        text.contains("ui.selection"),
                        "{name}: helix leaf names no ui.selection"
                    );
                }
            } else {
                assert!(!path.exists(), "{name}: unexpected {}", path.display());
            }
        }
    }
}
