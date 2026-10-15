//! The five shipped looks under `looks/`: each validates as a shipped look
//! and renders both first-party fragments.
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

use scoot_look::{Origin, RenderInputs, check_contrast, render_bar, render_scoot, resolve};

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
    }
}

#[test]
fn shipped_contrast_warnings_are_the_known_dim_and_urgent_pairs() {
    // Design section 3.5: `dim` and `urgent` report but never refuse, and
    // two shipped looks sit under 3.0 (music-desk dim 1.57 and urgent
    // 2.88, vinyl-sunset dim 2.18; ratios round to one decimal). Locked
    // here so a retune shows up as a test change; the maintainer call
    // (retune or accept) rides in the PR, not in a refusal.
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
            (
                "vinyl-sunset".to_string(),
                "dim/background".to_string(),
                2.2
            ),
        ]
    );
}
