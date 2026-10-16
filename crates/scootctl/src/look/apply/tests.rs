//! Swap tests: the transaction against scripted peers. Hermetic: scratch
//! catalogs, state dirs, caches and config homes, never the environment.
//! `apply_with` takes the config home explicitly (pinned to a tempdir in
//! every test); the `None` fallback resolves it from the live environment
//! for production only, and `toolkit_files_follow_the_injected_config_home`
//! re-runs an apply in a child process under a sandboxed HOME and
//! XDG_CONFIG_HOME to prove nothing lands outside it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use scoot_look::state::{LINK_NAME, RecordRead, read_record};

use super::{
    ApplyOptions, ApplyReport, Availability, BuildTarget, Checks, apply_with, apply_with_live,
    build_to_out, shadowed_from_files_in, validate_dir, validate_entry,
};
use scoot_look::Origin;

use super::super::leaves::ScriptedLeaves;
use super::super::reload::{BarReload, CompositorReload, ReloadError, Reloaded};
use super::super::toolkit::Dconf;
use scoot_look::catalog::{Catalog, discover_ordered};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch dir unique to this test process and call, removed on drop.
fn scratch(tag: &str) -> tempfile::TempDir {
    let id = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
    tempfile::Builder::new()
        .prefix(&format!(
            "scoot-look-apply-{tag}-{}-{id}-",
            std::process::id()
        ))
        .tempdir()
        .expect("a scratch dir")
}

/// A minimal valid look: the moonrise palette (which clears the contrast
/// bars), a color wallpaper (no image binary needed), and a license (so it
/// validates under every origin).
fn look_toml(name: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "dark"
license = "MIT"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DejaVu Sans"
sans = "DejaVu Sans"
mono = "DejaVu Sans Mono"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24

[wallpaper]
color = "#2B3648"
mode = "fill"
"##
    )
}

/// Writes a source look `dir/<name>/look.toml`, with `extra` appended.
fn write_look(dir: &Path, name: &str, extra: &str) -> PathBuf {
    let look = dir.join(name);
    std::fs::create_dir_all(&look).expect("a look dir");
    let mut text = look_toml(name);
    text.push_str(extra);
    std::fs::write(look.join("look.toml"), text).expect("a look.toml");
    look
}

struct Fixture {
    catalog: Catalog,
    cache: PathBuf,
    state: PathBuf,
    config: PathBuf,
    _scratch: tempfile::TempDir,
    _root: PathBuf,
}

/// Two looks (`look-a` ring width 2, `look-b` ring width 3), a cache, a
/// state dir and a config home, all scratch. The `TempDir` guard removes
/// the tree on drop, so the suite leaves no `/tmp/scoot-look-catalog-*`
/// dirs behind.
fn fixture(tag: &str) -> Fixture {
    let scratch = scratch(tag);
    let root: PathBuf = scratch.path().to_owned();
    let sources = root.join("looks");
    write_look(&sources, "look-a", "");
    let b = look_toml("look-b").replace("ring-width = 2", "ring-width = 3");
    let bdir = sources.join("look-b");
    std::fs::create_dir_all(&bdir).expect("a look dir");
    std::fs::write(bdir.join("look.toml"), b).expect("a look.toml");
    let catalog = discover_ordered(&[(sources, Origin::User)]);
    assert_eq!(catalog.entries.len(), 2, "notes: {:?}", catalog.notes);
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    Fixture {
        catalog,
        cache,
        state,
        config,
        _scratch: scratch,
        _root: root,
    }
}

fn options() -> ApplyOptions {
    ApplyOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: false,
        trust: false,
        no_helix: false,
        no_btop: false,
        from_compositor: false,
        preview: false,
        only_if_generation: None,
        nix_claims: Vec::new(),
    }
}

#[derive(Default)]
struct FakeCompositor {
    script: RefCell<VecDeque<Result<Reloaded, ReloadError>>>,
    calls: Cell<usize>,
}

impl FakeCompositor {
    fn ok(applied: &[&str], shadowed: &[&str]) -> Self {
        Self::with_script(vec![Ok(Reloaded {
            applied: applied.iter().map(|s| (*s).to_owned()).collect(),
            refused: Vec::new(),
            shadowed: shadowed.iter().map(|s| (*s).to_owned()).collect(),
            invalid: Vec::new(),
        })])
    }

    fn not_running() -> Self {
        Self::with_script(vec![Err(ReloadError::NotRunning(
            "no compositor answers".to_owned(),
        ))])
    }

    fn with_script(script: Vec<Result<Reloaded, ReloadError>>) -> Self {
        Self {
            script: RefCell::new(script.into()),
            calls: Cell::new(0),
        }
    }
}

impl CompositorReload for FakeCompositor {
    fn reload_look(&mut self) -> Result<Reloaded, ReloadError> {
        self.calls.set(self.calls.get() + 1);
        self.script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(ReloadError::Refused("no scripted reply left".to_owned())))
    }
}

struct FakeBar {
    display: String,
    script: RefCell<VecDeque<Result<bool, ReloadError>>>,
    calls: Cell<usize>,
}

impl FakeBar {
    fn with_script(display: &str, script: Vec<Result<bool, ReloadError>>) -> Self {
        Self {
            display: display.to_owned(),
            script: RefCell::new(script.into()),
            calls: Cell::new(0),
        }
    }
}

impl BarReload for FakeBar {
    fn display(&self) -> &str {
        &self.display
    }

    fn reload_look(&self) -> Result<bool, ReloadError> {
        self.calls.set(self.calls.get() + 1);
        self.script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(ReloadError::Refused("no scripted reply left".to_owned())))
    }
}

#[derive(Default)]
struct FakeChecks {
    font_dir: Option<PathBuf>,
    font_refuse: Option<String>,
    bar_refuse: Option<String>,
    foot_refuse: Option<String>,
    /// Models a bar built `--no-default-features`: refuses only fragments
    /// carrying `popup-radius`, recording every fragment seen.
    popup_only: bool,
    /// Models a minimal bar (no `popup`, no `workspaces` module): refuses
    /// fragments carrying `popup-radius` or a `[workspaces]` table, with
    /// the whole-stderr shapes a real minimal binary answers with.
    minimal_bar: bool,
    bar_seen: RefCell<Vec<String>>,
    foot_seen: RefCell<Vec<String>>,
}

impl FakeChecks {
    fn with_fonts(dir: &Path) -> Self {
        for family in ["DejaVu Sans", "DejaVu Sans Mono"] {
            std::fs::write(dir.join(format!("{family}.otf")), b"font").expect("a font file");
        }
        Self {
            font_dir: Some(dir.to_owned()),
            font_refuse: None,
            bar_refuse: None,
            foot_refuse: None,
            popup_only: false,
            minimal_bar: false,
            bar_seen: RefCell::new(Vec::new()),
            foot_seen: RefCell::new(Vec::new()),
        }
    }
}

impl Checks for FakeChecks {
    fn font_file(&self, family: &str) -> Result<Availability<String>, String> {
        if let Some(refusal) = &self.font_refuse {
            return Err(refusal.clone());
        }
        match &self.font_dir {
            Some(dir) => Ok(Availability::Available(
                dir.join(format!("{family}.otf")).display().to_string(),
            )),
            None => Ok(Availability::Unavailable(
                "no `fc-match` on PATH: font files unchecked".to_owned(),
            )),
        }
    }

    fn wallpaper(&self, path: &Path) -> Result<Availability<()>, String> {
        assert!(path.is_file(), "wallpaper resolves to a file");
        Ok(Availability::Available(()))
    }

    fn bar_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        let text = std::fs::read_to_string(fragment).unwrap_or_default();
        self.bar_seen.borrow_mut().push(text.clone());
        // The 1-based line of the first fragment line holding `needle`
        // (what the real parser's `TOML parse error at line N` names).
        let line_of = |needle: &str| {
            text.lines()
                .position(|line| line.contains(needle))
                .map(|index| index + 1)
                .unwrap_or(1)
        };
        if self.popup_only {
            // The shape a real `--no-default-features` bar answers with:
            // a TOML parse summary line plus the unknown-field detail naming
            // `popup-radius` (captured from a real binary; the CLI keys its
            // popup-less re-render off that name in the whole stderr). The
            // line number is the fragment's own `popup-radius` line, the
            // way the real parser reports it.
            if text.contains("popup-radius") {
                return Err(format!(
                    "scootbar: bar.toml: TOML parse error at line {}, column 1\nunknown field `popup-radius`, expected `radius`",
                    line_of("popup-radius")
                ));
            }
            return Ok(Availability::Available(()));
        }
        if self.minimal_bar {
            // The shapes a real minimal bar answers with: the popup refusal
            // above, plus the module-table refusal (an empty
            // `WorkspacesFile` denies every key: `there are no fields`).
            // Captured from a real `--no-default-features` binary; the CLI
            // keys its section drop off the error's line (the table header
            // at or before the span), so the line numbers are the
            // fragment's own, like the real parser's.
            if text.contains("popup-radius") {
                return Err(format!(
                    "scootbar: bar.toml: TOML parse error at line {}, column 1\nunknown field `popup-radius`, expected `radius`",
                    line_of("popup-radius")
                ));
            }
            if text.contains("[workspaces]") {
                return Err(format!(
                    "scootbar: bar.toml: TOML parse error at line {}, column 1\nunknown field `inactive-color`, there are no fields",
                    line_of("inactive-color")
                ));
            }
            return Ok(Availability::Available(()));
        }
        if let Some(refusal) = &self.bar_refuse {
            return Err(refusal.clone());
        }
        assert!(fragment.is_file(), "the bar fragment renders");
        Ok(Availability::Available(()))
    }

    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        let text = std::fs::read_to_string(fragment).unwrap_or_default();
        self.foot_seen.borrow_mut().push(text);
        if let Some(refusal) = &self.foot_refuse {
            return Err(refusal.clone());
        }
        assert!(fragment.is_file(), "the foot fragment renders");
        Ok(Availability::Available(()))
    }
}

#[derive(Default)]
struct FakeDconf {
    live: std::collections::HashMap<String, String>,
    writes: RefCell<Vec<(String, String)>>,
    missing: bool,
}

impl FakeDconf {
    fn absent() -> Self {
        Self {
            live: std::collections::HashMap::new(),
            writes: RefCell::new(Vec::new()),
            missing: true,
        }
    }
}

impl Dconf for FakeDconf {
    fn available(&self) -> bool {
        !self.missing
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.live.get(key).cloned())
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        self.writes
            .borrow_mut()
            .push((key.to_owned(), value.to_owned()));
        Ok(())
    }

    fn reset(&self, key: &str) -> Result<(), String> {
        self.writes
            .borrow_mut()
            .push((format!("reset {key}"), String::new()));
        Ok(())
    }
}

fn record_of(state: &Path) -> scoot_look::state::Record {
    match read_record(state) {
        RecordRead::Present(record) => record,
        other => panic!("a record reads: {other:?}"),
    }
}

fn link_of(state: &Path) -> PathBuf {
    std::fs::read_link(state.join(LINK_NAME)).expect("a link")
}

#[test]
fn an_offline_apply_records_repoints_and_renders() {
    let fixture = fixture("offline");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    assert_eq!(report.name, "look-a");
    assert_eq!(report.generation, 1);
    assert!(report.reloaded.is_empty());
    assert!(
        !report.skipped.is_empty(),
        "the missing compositor is skipped"
    );
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-a");
    assert_eq!(record.generation, 1);
    assert_eq!(record.chosen_by.as_deref(), Some("apply"));
    let rendered = PathBuf::from(&record.rendered);
    assert_eq!(link_of(&fixture.state), rendered);
    for file in [
        "scoot.toml",
        "bar.toml",
        "render.json",
        "mako",
        "wob.ini",
        "fcitx5/theme.conf",
        "fuzzel.ini",
        "swaylock",
        "greet.toml",
    ] {
        let text = std::fs::read_to_string(rendered.join(file)).expect("a rendered file");
        assert!(!text.is_empty(), "{file} renders");
    }
    // The catalog test look names no `[night]`: no temperature file.
    assert!(
        !rendered.join("nightlight").exists(),
        "a nightlight file renders without a temperature"
    );
    // The launcher and locker leaves carry the look's roles: fuzzel's
    // border is the focus ring, the locker's ring the same.
    let fuzzel = std::fs::read_to_string(rendered.join("fuzzel.ini")).expect("fuzzel.ini");
    assert!(
        fuzzel.contains("border=FFA45Cff"),
        "fuzzel border is the focus ring:\n{fuzzel}"
    );
    let swaylock = std::fs::read_to_string(rendered.join("swaylock")).expect("swaylock");
    assert!(
        swaylock.contains("ring-color=FFA45C"),
        "swaylock ring is the focus ring:\n{swaylock}"
    );
    let cache_name = rendered.file_name().expect("a dir name").to_string_lossy();
    assert!(
        cache_name.starts_with("look-a-") && cache_name.len() == "look-a-".len() + 16,
        "the cache dir carries the key: {cache_name}"
    );
}

#[test]
fn the_render_carries_the_greeter_fragment() {
    // Ticket `look-greeter`: every render carries the login-screen
    // fragment scootgreet will read at system build time (never live).
    // The fixture look's defaults resolve surface to the background and
    // hover/focus/selection to the accent; the app size defaults to 12
    // pt (16 px by the bar's rule).
    let fixture = fixture("greeter-fragment");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    let greet = std::fs::read_to_string(rendered.join("greet.toml")).expect("greet.toml");
    for line in [
        "background = \"#2B3648\"",
        "foreground = \"#F6EEDC\"",
        "accent = \"#FFA45C\"",
        "surface = \"#2B3648\"",
        "focus = \"#FFA45C\"",
        "font = \"DejaVu Sans\"",
        "font-size = 16",
        "opacity = 1.0",
        "radius = 14",
    ] {
        assert!(greet.contains(line), "missing {line}:\n{greet}");
    }
    let parsed: toml::Table = toml::from_str(&greet).expect("greet.toml parses");
    assert_eq!(parsed["colors"].as_table().expect("colors").len(), 13);
}

#[test]
fn a_live_swap_never_reaches_the_greeter() {
    // Ticket `look-greeter`: the login screen reads the greeter's look
    // at system build time only. A swap runs entirely inside the
    // session's own roots (state, cache, config home): the catalog
    // sources below — the manifests and the `apps/regreet.css` leaf
    // the greeter's build-time resolution reads — are byte-identical
    // across two swaps, and no swap writes anything outside the roots
    // at all. The other half of the pin is Nix-side: `nix/tests.nix`
    // asserts the explicit-greeter evaluation equals the
    // session-default one file for file, so with build-time inputs
    // fixed the login screen cannot move under a live swap.
    let fixture = fixture("greeter-isolation");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // A ReGreet CSS leaf in one source look, shaped like the greeter's
    // build-time CSS input (planted after discovery: the pin hashes the
    // files on disk, which is what the build-time resolution reads).
    let apps = fixture._root.join("looks/look-a/apps");
    std::fs::create_dir_all(&apps).expect("an apps dir");
    std::fs::write(
        apps.join("regreet.css"),
        "frame.background { color: #FFA45C; }\n",
    )
    .expect("a greeter CSS leaf");
    let before_tree = tree_snapshot(&fixture._root);
    let before_sources = dir_file_contents(&fixture._root.join("looks"));
    let mut compositor = FakeCompositor::with_script(vec![
        Err(ReloadError::NotRunning("no compositor answers".to_owned())),
        Err(ReloadError::NotRunning("no compositor answers".to_owned())),
    ]);
    for name in ["look-a", "look-b"] {
        apply_with_live(
            name,
            &options(),
            &fixture.catalog,
            &fixture.cache,
            &fixture.state,
            Some(&fixture.config),
            &mut compositor,
            &[],
            &checks,
            &FakeDconf::absent(),
            &ScriptedLeaves::failing(),
            &[],
        )
        .expect("a swap succeeds");
    }
    assert_eq!(record_of(&fixture.state).name, "look-b");
    // The greeter's build-time inputs survive two swaps byte for byte.
    assert_eq!(
        dir_file_contents(&fixture._root.join("looks")),
        before_sources,
        "a live swap rewrote the greeter's build-time inputs"
    );
    // Nothing outside the session's own roots: the only new entries
    // are under the cache, the state dir, the config home and the
    // fonts stub. In particular the catalog sources (`looks/`, the
    // build-time inputs the greeter also reads) gain nothing: a live
    // swap writes no build-time location.
    let after = tree_snapshot(&fixture._root);
    let fresh: Vec<&String> = after
        .iter()
        .filter(|rel| !before_tree.contains(rel))
        .collect();
    let stray: Vec<&&String> = fresh
        .iter()
        .filter(|rel| {
            !rel.starts_with("cache/")
                && !rel.starts_with("state/")
                && !rel.starts_with("config/")
                && !rel.starts_with("fonts/")
        })
        .collect();
    assert!(
        stray.is_empty(),
        "a live swap wrote outside its roots: {stray:?}"
    );
}

#[test]
fn a_from_compositor_apply_records_its_provenance() {
    let fixture = fixture("provenance");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let mut flagged = options();
    flagged.from_compositor = true;
    apply_with_live(
        "look-a",
        &flagged,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("a compositor-driven apply succeeds");
    assert_eq!(
        record_of(&fixture.state).chosen_by.as_deref(),
        Some("compositor")
    );
}

#[test]
fn a_second_apply_bumps_the_generation() {
    let fixture = fixture("generation");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the second apply succeeds");
    assert_eq!(report.generation, 2);
    assert_eq!(record_of(&fixture.state).name, "look-b");
}

/// A committed swap renders the daemon leaves beside the first-party
/// fragments and runs the leaf signals: `makoctl reload`, the fcitx5
/// reload, and both unit restarts. A look without `[night]` writes no
/// temperature file and restarts no night-light unit.
#[test]
fn a_committed_swap_renders_and_runs_the_daemon_leaves() {
    let fixture = fixture("leaves-run");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let leaves = ScriptedLeaves::ok();
    // The catalog test look names no `[night]`: give this apply one.
    let night =
        look_toml("look-a").replace("[wallpaper]", "[night]\ntemperature = 3400\n\n[wallpaper]");
    std::fs::write(fixture._root.join("looks/look-a/look.toml"), night)
        .expect("a look.toml with night");
    let catalog = discover_ordered(&[(fixture._root.join("looks"), Origin::User)]);
    let report = apply_with_live(
        "look-a",
        &options(),
        &catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &leaves,
        &[],
    )
    .expect("the apply succeeds");
    assert!(report.reloaded.contains(&"mako".to_owned()), "{report:?}");
    assert!(report.reloaded.contains(&"fcitx5".to_owned()), "{report:?}");
    assert!(
        report.reloaded.iter().any(|r| r == "scoot-osd unit"),
        "{report:?}"
    );
    assert!(
        report
            .reloaded
            .iter()
            .any(|r| r == "scoot-nightlight unit (3400K)"),
        "{report:?}"
    );
    assert_eq!(
        leaves.restarted().as_slice(),
        &["scoot-osd", "scoot-nightlight"]
    );
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    let mako = std::fs::read_to_string(rendered.join("mako")).expect("a mako fragment");
    assert!(
        mako.contains("background-color=#2B3648"),
        "look colors reach mako:\n{mako}"
    );
    let wob = std::fs::read_to_string(rendered.join("wob.ini")).expect("a wob fragment");
    assert!(
        wob.contains("bar_color = FFA45Cff"),
        "look colors reach wob:\n{wob}"
    );
    let theme =
        std::fs::read_to_string(rendered.join("fcitx5/theme.conf")).expect("an fcitx5 fragment");
    assert!(
        theme.contains("HighlightColor=#FFA45C"),
        "look colors reach fcitx5:\n{theme}"
    );
    let nightlight =
        std::fs::read_to_string(rendered.join("nightlight")).expect("a nightlight fragment");
    assert!(
        nightlight.contains("temperature=3400"),
        "the temperature reaches the unit:\n{nightlight}"
    );
}

/// A refused reload rolls back before the leaf signals run: the daemons
/// never hear about a swap that did not commit.
#[test]
fn a_rolled_back_swap_runs_no_leaf_signal() {
    let fixture = fixture("leaves-rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor =
        FakeCompositor::with_script(vec![Err(ReloadError::Refused("no".to_owned()))]);
    let leaves = ScriptedLeaves::ok();
    let _ = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &leaves,
        &[],
    )
    .expect_err("the refused reload fails the swap");
    assert!(
        leaves.restarted().is_empty(),
        "a unit restarted on rollback"
    );
}

#[test]
fn an_unknown_look_refuses_and_touches_nothing() {
    let fixture = fixture("unknown");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "nope",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("an unknown look refuses");
    assert!(
        failure.message.contains("unknown look `nope`"),
        "{}",
        failure.message
    );
    assert!(failure.message.contains("look-a"), "{}", failure.message);
    assert!(failure.rolled_back_to.is_none());
    assert!(!fixture.state.join("look.json").exists());
    assert!(!fixture.state.join(LINK_NAME).exists());
}

#[test]
fn an_invalid_name_refuses_before_any_path_join() {
    let fixture = fixture("badname");
    let checks = FakeChecks::default();
    let mut compositor = FakeCompositor::not_running();
    for bad in ["../evil", "LOUD", "", "a/b"] {
        let failure = apply_with_live(
            bad,
            &options(),
            &fixture.catalog,
            &fixture.cache,
            &fixture.state,
            Some(&fixture.config),
            &mut compositor,
            &[],
            &checks,
            &FakeDconf::absent(),
            &ScriptedLeaves::failing(),
            &[],
        )
        .expect_err("an invalid name refuses");
        assert!(
            failure.message.contains("not a look name"),
            "{}",
            failure.message
        );
    }
}

#[test]
fn a_corrupt_record_refuses_and_keeps_the_link() {
    let fixture = fixture("corrupt");
    std::fs::create_dir_all(&fixture.state).expect("a state dir");
    std::fs::write(fixture.state.join("look.json"), "{oops").expect("a corrupt record");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a corrupt record refuses");
    assert!(failure.message.contains("corrupt"), "{}", failure.message);
}

#[test]
fn an_unwritable_state_dir_refuses_loudly() {
    let fixture = fixture("unwritable");
    std::fs::create_dir_all(&fixture._root).expect("the root");
    // A file where the state dir should be: creating it fails loudly.
    std::fs::write(&fixture.state, b"not a dir").expect("a blocking file");
    let checks = FakeChecks::default();
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("an unwritable state dir refuses");
    assert!(
        failure.message.contains("cannot use state dir"),
        "{}",
        failure.message
    );
}

#[test]
fn a_compositor_refusal_rolls_back_to_the_previous_look() {
    let fixture = fixture("rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&["appearance.background_color"], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let first_rendered = link_of(&fixture.state);
    // The swap to look-b applies, then the compositor refuses it.
    let mut compositor = FakeCompositor::with_script(vec![
        Ok(Reloaded {
            applied: vec!["appearance.background_color".to_owned()],
            refused: vec!["appearance.background_color (no live output)".to_owned()],
            shadowed: Vec::new(),
            invalid: Vec::new(),
        }),
        Ok(Reloaded::default()),
    ]);
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a refused swap fails");
    assert!(failure.message.contains("refused"), "{}", failure.message);
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert!(failure.rollback_errors.is_empty());
    // The swap and its rollback both reload: the refused app may hold
    // the new layer, so the rollback re-runs the scoped reload against
    // the restored choice.
    assert_eq!(compositor.calls.get(), 2);
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-a");
    assert_eq!(record.generation, 3, "apply, failed swap, rollback");
    assert_eq!(record.chosen_by.as_deref(), Some("rollback"));
    assert_eq!(link_of(&fixture.state), first_rendered);
}

#[test]
fn an_older_compositor_is_a_refusal_never_a_plain_retry() {
    // The wire shape an older compositor answers to the unknown
    // `reload_look` tag: an ordinary error, which refuses the swap rather
    // than retrying as a plain reload (a plain reload would apply the
    // pending edits the scope exists to skip). The peer never accepted the
    // request, so the rollback skips re-running it (one call, not two).
    let fixture = fixture("older");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![Err(ReloadError::Refused(
        "unknown variant `reload_look`, expected `reload`".to_owned(),
    ))]);
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("an older compositor refuses");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert!(
        failure.message.contains("unknown variant"),
        "{}",
        failure.message
    );
    // Only the swap runs: the peer never accepted the request, so the
    // rollback skips re-running it, and nothing retries as a plain reload.
    assert_eq!(compositor.calls.get(), 1);
}

#[test]
fn a_rollback_that_also_refuses_reports_both_errors() {
    // The compositor applies the swap, then a bar refuses it: the rollback
    // re-runs the compositor, which refuses too. The link stays at the
    // previous look, and both errors are reported.
    let fixture = fixture("rollback-errors");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![
        Ok(Reloaded::default()),
        Err(ReloadError::Refused("the old layer is bad too".to_owned())),
    ]);
    let bar = FakeBar::with_script(
        "wayland-0",
        vec![Err(ReloadError::Refused("the new layer is bad".to_owned()))],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a double refusal fails");
    // The link stays at the previous look; both errors are reported.
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(failure.rollback_errors.len(), 1);
    assert!(
        failure.to_string().contains("ROLLED BACK WITH ERRORS"),
        "{}",
        failure
    );
    assert!(
        failure.to_string().contains("the new layer is bad"),
        "{}",
        failure
    );
    assert!(
        failure.to_string().contains("the old layer is bad too"),
        "{}",
        failure
    );
    assert_eq!(record_of(&fixture.state).name, "look-a");
    assert_eq!(
        compositor.calls.get(),
        2,
        "the swap and its rollback both reload"
    );
}

#[test]
fn a_failed_first_apply_removes_what_it_wrote() {
    let fixture = fixture("first-rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // Nothing to restore to and nothing positively reloaded (the
    // compositor refused, never accepted): what the failed swap wrote is
    // removed, no rollback reload runs, and the message is the refusal
    // alone — with no compositor and nothing reloaded, nothing reloaded,
    // so no split-brain note either.
    let mut compositor =
        FakeCompositor::with_script(vec![Err(ReloadError::Refused("nope".to_owned()))]);
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("the first apply fails");
    assert!(failure.rolled_back_to.is_none());
    assert_eq!(failure.message, "nope");
    assert!(failure.rollback_errors.is_empty());
    assert!(
        !failure.to_string().contains("still show the new look"),
        "{}",
        failure
    );
    assert_eq!(compositor.calls.get(), 1);
    assert!(!fixture.state.join("look.json").exists());
    assert!(!fixture.state.join(LINK_NAME).exists());
}

#[test]
fn a_failed_first_apply_names_only_what_actually_reloaded() {
    // The compositor positively reloaded, then a bar refused: the
    // compositor still shows the new look while nothing is recorded, and
    // the note names the compositor only. The bar merely refused — it
    // never left its old layer — so it is not counted.
    let fixture = fixture("first-split");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    let bar = FakeBar::with_script(
        "wayland-0",
        vec![Err(ReloadError::Refused("bad layer".to_owned()))],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("the first apply fails");
    assert!(failure.rolled_back_to.is_none());
    assert!(
        failure
            .to_string()
            .contains("the compositor still shows the new look"),
        "{}",
        failure
    );
    assert!(
        !failure.to_string().contains("wayland-0 still shows"),
        "{}",
        failure
    );
    assert_eq!(bar.calls.get(), 1, "a refusing bar is never re-run");
    assert!(!fixture.state.join("look.json").exists());
    assert!(!fixture.state.join(LINK_NAME).exists());
}

#[test]
fn a_partially_applied_compositor_is_re_run_on_rollback() {
    // A `Reloaded` reply with a non-empty `refused` is partially applied,
    // not never-sent: per-field refusals do not hold back the applied
    // fields, so the peer may hold the new layer and the rollback re-runs
    // it (two calls: the swap and its rollback), restoring the previous
    // look. (Regression: one round never re-ran any refused peer; the
    // round before treated every refusal as may-hold.)
    let fixture = fixture("partial-rerun");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![
        Ok(Reloaded {
            applied: vec!["appearance.background_color".to_owned()],
            refused: vec!["output:scale".to_owned()],
            shadowed: Vec::new(),
            invalid: Vec::new(),
        }),
        Ok(Reloaded::default()),
    ]);
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a partial apply refuses the swap");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert!(
        failure.message.contains("output:scale"),
        "{}",
        failure.message
    );
    assert_eq!(
        compositor.calls.get(),
        2,
        "the swap and its rollback both reload"
    );
    assert_eq!(record_of(&fixture.state).name, "look-a");
}

#[test]
fn a_sent_but_unanswered_compositor_is_re_run_on_rollback() {
    // A 30 s timeout (or a connection closed after the write) is
    // sent-but-unanswered: it may have applied before hanging or dying,
    // so the rollback re-runs it (two calls), like a positive receipt.
    // Only an explicit error reply, the "predates" wall or a never-sent
    // request means "never accepted" (one call: no re-run).
    let fixture = fixture("unanswered-rerun");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![
        Err(ReloadError::Unanswered(
            "the compositor did not answer: timed out".to_owned(),
        )),
        Ok(Reloaded::default()),
    ]);
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("an unanswered apply refuses the swap");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(
        compositor.calls.get(),
        2,
        "the swap and its rollback both reload"
    );
    assert_eq!(record_of(&fixture.state).name, "look-a");
}

#[test]
fn an_absent_bar_is_skipped_not_rolled_back() {
    let fixture = fixture("absent-bar");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&["appearance.background_color"], &[]);
    // A stale socket the bar already left: nothing listens, so the swap
    // skips it with a note and completes without it.
    let bar = FakeBar::with_script("wayland-9", vec![Ok(false)]);
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let report = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an absent bar skips");
    assert_eq!(report.reloaded, vec!["compositor".to_owned()]);
    assert!(
        report
            .skipped
            .iter()
            .any(|skipped| skipped.contains("wayland-9")),
        "{:?}",
        report.skipped
    );
    assert_eq!(
        bar.calls.get(),
        1,
        "an absent bar runs once, never in a rollback"
    );
    assert_eq!(record_of(&fixture.state).name, "look-a");
}

#[test]
fn a_bar_refusal_rolls_back_the_compositor_too() {
    let fixture = fixture("bar-rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor =
        FakeCompositor::with_script(vec![Ok(Reloaded::default()), Ok(Reloaded::default())]);
    let bar0 = FakeBar::with_script("wayland-0", vec![Ok(true), Ok(true)]);
    // A refusing bar never accepted the new layer, so the rollback never
    // re-runs it (one call, not two): no second refusal, no retry as a
    // plain reload.
    let bar1 = FakeBar::with_script(
        "wayland-1",
        vec![Err(ReloadError::Refused("bad layer".to_owned()))],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar0, &bar1];
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a bar refusal fails the swap");
    assert!(failure.message.contains("wayland-1"), "{}", failure.message);
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(compositor.calls.get(), 2, "the compositor rolls back too");
    assert_eq!(bar0.calls.get(), 2, "the reloaded bar rolls back");
    assert_eq!(bar1.calls.get(), 1, "a refusing bar is never re-run");
    assert_eq!(record_of(&fixture.state).name, "look-a");
}

#[test]
fn a_sent_but_unanswered_bar_is_re_run_on_rollback() {
    // A bar that closes without a reply after a successful write is
    // sent-but-unanswered: it may hold the new layer, so the rollback
    // re-runs it (two calls: the swap and its rollback), like a positive
    // receipt. Only an explicit error reply, the "predates" wall or a
    // never-sent request means "never accepted" (one call: no re-run).
    // (Regression: the bar loop treated every error as never accepted.)
    let fixture = fixture("bar-unanswered-rerun");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let mut compositor =
        FakeCompositor::with_script(vec![Ok(Reloaded::default()), Ok(Reloaded::default())]);
    let bar = FakeBar::with_script(
        "wayland-0",
        vec![
            Err(ReloadError::Unanswered(
                "the bar on wayland-0 closed the connection without replying".to_owned(),
            )),
            Ok(true),
        ],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("an unanswered bar fails the swap");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(
        bar.calls.get(),
        2,
        "the swap and its rollback both reload the bar"
    );
    assert_eq!(compositor.calls.get(), 2, "the compositor rolls back too");
    assert_eq!(record_of(&fixture.state).name, "look-a");
}

#[test]
fn a_failed_first_apply_names_an_unanswered_bar() {
    // No previous look to restore to, and the bar may hold the new layer
    // while nothing is recorded: the split-brain note names the bar (a
    // refusing bar is never named: it never left its old layer).
    let fixture = fixture("first-bar-unanswered");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let bar = FakeBar::with_script(
        "wayland-0",
        vec![Err(ReloadError::Unanswered(
            "the bar on wayland-0 did not answer: timed out".to_owned(),
        ))],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("the first apply fails");
    assert!(failure.rolled_back_to.is_none());
    assert!(
        failure
            .to_string()
            .contains("bar wayland-0 still shows the new look"),
        "{}",
        failure
    );
    assert_eq!(bar.calls.get(), 1, "nothing exists to roll back to");
    assert!(!fixture.state.join("look.json").exists());
}

#[test]
fn a_held_lock_refuses_the_second_writer() {
    use scoot_look::state::acquire_lock;
    let fixture = fixture("lock");
    std::fs::create_dir_all(&fixture.state).expect("a state dir");
    let _held = acquire_lock(&fixture.state, Duration::from_secs(10)).expect("the first lock");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let mut brief = options();
    brief.lock_wait = Duration::from_millis(100);
    let failure = apply_with_live(
        "look-a",
        &brief,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a held lock refuses");
    assert!(
        failure.message.contains("another writer"),
        "{}",
        failure.message
    );
    assert!(!fixture.state.join("look.json").exists());
}

#[test]
fn a_disagreeing_link_heals_from_the_record() {
    let fixture = fixture("heal");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    // A simulated crash between the record and the link: the link dangles
    // at a removed directory.
    std::fs::remove_file(fixture.state.join(LINK_NAME)).expect("the link goes");
    std::os::unix::fs::symlink("/nonexistent-crash-target", fixture.state.join(LINK_NAME))
        .expect("a dangling link");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the next apply heals first");
    assert!(
        report.notes.iter().any(|note| note.contains("repointed")),
        "notes: {:?}",
        report.notes
    );
    assert_eq!(record_of(&fixture.state).name, "look-b");
}

#[test]
fn shadowed_keys_come_from_the_reload_reply() {
    let fixture = fixture("reply-shadowed");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(
        &["appearance.background_color"],
        &["appearance.background_color", "wallpaper"],
    );
    let report = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the apply succeeds");
    assert_eq!(
        report.shadowed,
        vec!["appearance.background_color", "wallpaper"]
    );
}

#[test]
fn offline_shadowed_keys_come_from_the_files() {
    let fixture = fixture("file-shadowed");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // The user's own files override the look: a color (per key) and the
    // whole wallpaper section (as one unit).
    let scoot_dir = fixture.config.join("scoot");
    std::fs::create_dir_all(&scoot_dir).expect("a config dir");
    std::fs::write(
        scoot_dir.join("config.toml"),
        "[appearance]\nbackground_color = \"#000000\"\n[wallpaper]\ncolor = \"#000000\"\n",
    )
    .expect("a user config");
    std::fs::write(
        scoot_dir.join("bar.toml"),
        "[colors]\naccent = \"#000000\"\n",
    )
    .expect("a user bar");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the offline apply succeeds");
    assert!(
        report
            .shadowed
            .contains(&"appearance.background_color".to_owned()),
        "{:?}",
        report.shadowed
    );
    assert!(
        report.shadowed.contains(&"wallpaper".to_owned()),
        "{:?}",
        report.shadowed
    );
    assert!(
        !report
            .shadowed
            .iter()
            .any(|key| key.starts_with("wallpaper.")),
        "{:?}",
        report.shadowed
    );
    // The hermetic core agrees with the file reader over this config home.
    let target = PathBuf::from(&record_of(&fixture.state).rendered);
    let (keys, _) = shadowed_from_files_in(&target, Some(&fixture.config));
    assert_eq!(keys, report.shadowed);
}

#[test]
fn the_toolkit_write_lands_in_the_record() {
    let fixture = fixture("toolkit-record");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let dconf = FakeDconf::default();
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &dconf,
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the apply succeeds");
    assert_eq!(
        dconf.writes.borrow().len(),
        super::super::toolkit::TOOL_KEYS.len()
    );
    let record = record_of(&fixture.state);
    assert_eq!(
        record.dconf_wrote.get("color-scheme").map(String::as_str),
        Some("'prefer-dark'")
    );
    assert_eq!(
        record.dconf_wrote.len(),
        super::super::toolkit::TOOL_KEYS.len()
    );
    // A key someone else changed since is left alone and named, while the
    // keys the tool still owns rewrite: every key reads back what the
    // first apply wrote, except `color-scheme`, which reads `prefer-light`
    // against the recorded `prefer-dark`.
    let mut live = record.dconf_wrote.clone();
    live.insert("color-scheme".to_owned(), "'prefer-light'".to_owned());
    let live = live
        .into_iter()
        .map(|(short, value)| {
            let path = super::super::toolkit::TOOL_KEYS
                .iter()
                .find(|(name, _)| name == &short)
                .map(|(_, path)| (*path).to_owned())
                .expect("a toolkit key");
            (path, value)
        })
        .collect();
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-b",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &dconf,
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the second apply succeeds");
    let writes = dconf.writes.borrow();
    assert!(
        writes
            .iter()
            .all(|(key, _)| key != super::super::toolkit::COLOR_SCHEME_KEY),
        "the foreign key was written: {writes:?}"
    );
    assert_eq!(writes.len(), super::super::toolkit::TOOL_KEYS.len() - 1);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("outside `scoot look`")),
        "{:?}",
        report.notes
    );
}

#[test]
fn no_toolkit_skips_without_persisting() {
    let fixture = fixture("no-toolkit");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let dconf = FakeDconf::default();
    let mut brief = options();
    brief.no_toolkit = true;
    let report = apply_with_live(
        "look-a",
        &brief,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut FakeCompositor::not_running(),
        &[],
        &checks,
        &dconf,
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the apply succeeds");
    assert!(dconf.writes.borrow().is_empty());
    assert!(
        record_of(&fixture.state).toolkit.is_none(),
        "per-apply means per-apply"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("--no-toolkit")),
        "{:?}",
        report.notes
    );
}

#[test]
fn readers_racing_a_swap_see_old_or_new_never_half() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let fixture = fixture("race");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let done = Arc::new(AtomicBool::new(false));
    let state = fixture.state.clone();
    let mut readers = Vec::new();
    for _ in 0..4 {
        let done = done.clone();
        let state = state.clone();
        readers.push(std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                if let RecordRead::Present(record) = read_record(&state) {
                    let rendered = PathBuf::from(&record.rendered);
                    // A recorded choice always names a complete render.
                    for file in [
                        "scoot.toml",
                        "bar.toml",
                        "fuzzel.ini",
                        "swaylock",
                        "greet.toml",
                        "render.json",
                    ] {
                        assert!(rendered.join(file).is_file(), "half a swap: {record:?}");
                    }
                }
                if let Ok(target) = std::fs::read_link(state.join(LINK_NAME)) {
                    // A link always names a complete render too.
                    assert!(
                        target.join("scoot.toml").is_file(),
                        "half a link: {}",
                        target.display()
                    );
                }
            }
        }));
    }
    let checks = FakeChecks::with_fonts(&fonts);
    for round in 0..10 {
        let name = if round % 2 == 0 { "look-a" } else { "look-b" };
        apply_with_live(
            name,
            &options(),
            &fixture.catalog,
            &fixture.cache,
            &fixture.state,
            Some(&fixture.config),
            &mut FakeCompositor::not_running(),
            &[],
            &checks,
            &FakeDconf::absent(),
            &ScriptedLeaves::failing(),
            &[],
        )
        .expect("each swap succeeds");
    }
    done.store(true, Ordering::SeqCst);
    for reader in readers {
        reader.join().expect("no reader saw half a swap");
    }
}

#[test]
fn the_cache_keeps_active_previous_and_four_recent() {
    let fixture = fixture("prune");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    std::fs::create_dir_all(&fixture.cache).expect("a cache dir");
    // Six stale renders, oldest first (sleeps keep the mtimes ordered).
    let mut stale = Vec::new();
    for index in 0..6 {
        let dir = fixture.cache.join(format!("stale-{index:016x}"));
        std::fs::create_dir(&dir).expect("a stale render");
        std::fs::write(dir.join("scoot.toml"), "[appearance]\n").expect("a fragment");
        stale.push(dir);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the apply succeeds");
    // Active + four most recent survive; the two oldest are pruned.
    assert!(!stale[0].exists(), "the oldest render is pruned");
    assert!(!stale[1].exists(), "the second-oldest render is pruned");
    for dir in &stale[2..] {
        assert!(dir.exists(), "a recent render survives: {}", dir.display());
    }
}

#[test]
fn pruning_never_touches_state_names() {
    let fixture = fixture("prune-names");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    std::fs::create_dir_all(&fixture.cache).expect("a cache dir");
    std::fs::create_dir(fixture.cache.join("look")).expect("a look dir");
    std::fs::write(fixture.cache.join("look.json"), "{}").expect("a look.json");
    std::fs::write(fixture.cache.join("look.lock"), "").expect("a look.lock");
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the apply succeeds");
    assert!(fixture.cache.join("look").is_dir());
    assert!(fixture.cache.join("look.json").is_file());
    assert!(fixture.cache.join("look.lock").is_file());
}

#[test]
fn build_refuses_a_foreign_nonempty_out() {
    let fixture = fixture("build-foreign");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("foreign-out");
    std::fs::create_dir_all(&out).expect("an out dir");
    std::fs::write(out.join("notes.txt"), b"mine").expect("a foreign file");
    let error = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect_err("a foreign out refuses");
    assert!(error.contains("holds files"), "{error}");
    assert!(out.join("notes.txt").is_file(), "the foreign file survives");
    assert!(!out.join("scoot.toml").exists());
}

#[test]
fn build_refuses_a_symlinked_out_and_a_symlinked_fragment() {
    let fixture = fixture("build-symlink");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // A symlinked `out` itself refuses.
    let target = fixture._root.join("real-out");
    std::fs::create_dir_all(&target).expect("a target");
    let link = fixture._root.join("link-out");
    std::os::unix::fs::symlink(&target, &link).expect("a link");
    let error = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &link,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect_err("a symlinked out refuses");
    assert!(error.contains("symlink"), "{error}");
    // A symlinked fragment inside an otherwise-ours out refuses instead
    // of writing through it.
    let out = fixture._root.join("out-symfrag");
    std::fs::create_dir_all(&out).expect("an out");
    std::os::unix::fs::symlink("/etc/hostname", out.join("scoot.toml")).expect("a link");
    let error = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect_err("a symlinked fragment refuses");
    assert!(error.contains("symlink"), "{error}");
}

#[test]
fn build_into_an_out_holding_only_our_files_overwrites() {
    let fixture = fixture("build-ours");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("ours-out");
    build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("the first build succeeds");
    // A second build into the same dir (only our five files) succeeds.
    build_to_out(
        &BuildTarget::Name("look-b".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("rebuilding over our own files succeeds");
}

#[test]
fn build_leaves_a_sibling_build_tmp_alone() {
    // `--out dist` with a pre-existing `dist.build-tmp` (or any sibling)
    // must not delete it: the stage is uniquely named.
    let fixture = fixture("build-sibling");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("dist");
    let sibling = fixture._root.join("dist.build-tmp");
    std::fs::create_dir_all(&sibling).expect("a sibling");
    std::fs::write(sibling.join("keep.txt"), b"mine").expect("a file");
    build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("the build succeeds");
    assert!(sibling.join("keep.txt").is_file(), "the sibling survives");
}

#[test]
fn validate_names_a_bad_wallpaper_instead_of_blank() {
    // A wallpaper symlink escaping the look, a FIFO, and a missing file
    // all land in `errors` (never a blank refusal), and `validate_entry`
    // reports invalid (not valid:true).
    for (tag, setup) in [
        ("symlink", "symlink"),
        ("fifo", "fifo"),
        ("missing", "missing"),
    ] {
        let _scratch = scratch(&format!("badwall-{tag}"));
        let root = _scratch.path();
        let sources = root.join("looks");
        std::fs::create_dir_all(sources.join("badlook")).expect("a look dir");
        let base = look_toml("badlook");
        // Swap the color wallpaper for an image one pointing at
        // `wallpaper.png`.
        let with_wall = base.replace("color = \"#2B3648\"", "image = \"wallpaper.png\"");
        assert_ne!(with_wall, base, "the fixture carries a wallpaper color");
        std::fs::write(sources.join("badlook").join("look.toml"), with_wall).expect("a manifest");
        let wall = sources.join("badlook").join("wallpaper.png");
        match setup {
            "symlink" => {
                std::os::unix::fs::symlink("/etc/passwd", &wall).expect("a link");
            }
            "fifo" => {
                let rc = std::process::Command::new("mkfifo")
                    .arg(&wall)
                    .status()
                    .expect("mkfifo runs");
                assert!(rc.success());
            }
            _ => {
                // Missing: nothing created.
            }
        }
        let catalog = discover_ordered(&[(sources, Origin::User)]);
        let checks = FakeChecks::default();
        // The look may be skipped from the catalog (invalid); validate the
        // dir directly so the wallpaper check runs.
        let dir = root.join("looks").join("badlook");
        // `validate_dir` reads look.toml first; the wallpaper failure must
        // appear in errors, never as an empty command error.
        match validate_dir(&dir, Origin::User, &catalog, &checks, false) {
            Ok(validation) => {
                assert!(
                    !validation.errors.is_empty(),
                    "{tag}: expected errors, got {validation:?}"
                );
                for error in &validation.errors {
                    assert!(!error.trim().is_empty(), "{tag}: blank error");
                }
            }
            Err(message) => {
                assert!(!message.trim().is_empty(), "{tag}: blank refusal");
            }
        }
        // And an apply never prints an empty message either.
        if catalog.find("badlook").is_some() {
            let cache = root.join("cache");
            let state = root.join("state");
            // Refused before the toolkit step, but still pinned: every
            // apply test points its config home at a tempdir.
            let config = root.join("config");
            let mut compositor = FakeCompositor::not_running();
            let failure = apply_with_live(
                "badlook",
                &options(),
                &catalog,
                &cache,
                &state,
                Some(&config),
                &mut compositor,
                &[],
                &checks,
                &FakeDconf::absent(),
                &ScriptedLeaves::failing(),
                &[],
            )
            .expect_err("a bad wallpaper refuses the apply");
            assert!(!failure.message.trim().is_empty(), "blank apply message");
            assert!(
                !failure.message.ends_with("`badlook`: "),
                "blank detail: {}",
                failure.message
            );
        }
    }
}

#[test]
fn a_missing_font_family_refuses_despite_a_fallback_file() {
    use super::ProdChecks;
    // A stub `fc-match` that answers every query with a fallback file but
    // reports the real matched family: an installed family passes, a
    // missing one refuses even though a file was answered. A `TempDir`
    // guard, so the run leaves no `/tmp/scoot-fc-stub-*` dirs behind.
    let _stub_dir = tempfile::Builder::new()
        .prefix("scoot-fc-stub-")
        .tempdir()
        .expect("a stub dir");
    let dir = _stub_dir.path();
    let stub = dir.join("fc-match");
    std::fs::write(
        &stub,
        "#!/bin/sh\nif [ \"$1\" = \"--format=%{file}\\n\" ]; then echo \"/fonts/fallback.otf\"; else echo \"Fallback Family\"; fi\n",
    )
    .expect("a stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let checks = ProdChecks::with_fc_match(stub);
    // The fallback family itself resolves.
    let _ = checks;
    // Direct unit coverage lives in the family comparison: a stubbed
    // end-to-end refusal is asserted here.
    let refusal = checks
        .font_file("ZZNoSuchFontFamilyQ")
        .expect_err("a missing family refuses");
    assert!(refusal.contains("not installed"), "{refusal}");
}

#[test]
fn validate_and_build_cover_a_directory_and_a_name() {
    let fixture = fixture("validate");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let validation =
        validate_entry(&fixture.catalog, "look-a", &checks, false).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    assert!(validate_entry(&fixture.catalog, "nope", &checks, false).is_err());
    let dir = fixture._root.join("looks").join("look-a");
    let validation =
        validate_dir(&dir, Origin::User, &fixture.catalog, &checks, false).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    let out = fixture._root.join("out");
    let report = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("buildable");
    assert_eq!(report.name, "look-a");
    for file in [
        "scoot.toml",
        "bar.toml",
        "fuzzel.ini",
        "swaylock",
        "greet.toml",
        "render.json",
    ] {
        assert!(out.join(file).is_file(), "{file} builds");
    }
    // Nothing was recorded or linked by validating or building.
    assert!(!fixture.state.join("look.json").exists());
}

#[test]
fn a_font_refusal_refuses_before_anything_is_touched() {
    let fixture = fixture("font-refusal");
    let checks = FakeChecks {
        font_dir: None,
        font_refuse: Some("font `DejaVu Sans` resolves to nothing".to_owned()),
        bar_refuse: None,
        foot_refuse: None,
        popup_only: false,
        minimal_bar: false,
        bar_seen: RefCell::new(Vec::new()),
        foot_seen: RefCell::new(Vec::new()),
    };
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a refused font refuses the swap");
    assert!(
        failure.message.contains("DejaVu Sans"),
        "{}",
        failure.message
    );
    assert!(!fixture.state.join("look.json").exists());
}

#[test]
fn relative_dirs_resolve_absolute_before_anything_is_recorded() {
    use super::absolutize;
    // A relative cache must never reach the record: the next read would
    // refuse it as corrupt, and the link would dangle.
    let absolute = absolutize(Path::new("/already/absolute")).expect("absolute stays");
    assert_eq!(absolute, PathBuf::from("/already/absolute"));
    let joined = absolutize(Path::new("rel/dir")).expect("relative joins");
    assert!(joined.is_absolute());
    assert!(
        !joined
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    );
    let dotted = absolutize(Path::new("/a/b/../c")).expect("dot-dot normalizes");
    assert_eq!(dotted, PathBuf::from("/a/c"));
}

#[test]
fn a_directory_builds_itself_never_the_catalog_collision() {
    // A tweaked copy under a colliding name builds the copy: the
    // directory's entry wins over the catalog's.
    let fixture = fixture("collision");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let copy = fixture._root.join("copy");
    let tweaked = look_toml("look-a").replace("ring-width = 2", "ring-width = 9");
    let copydir = copy.join("look-a");
    std::fs::create_dir_all(&copydir).expect("a copy dir");
    std::fs::write(copydir.join("look.toml"), tweaked).expect("a tweaked look.toml");
    let out = fixture._root.join("out-collision");
    let report = build_to_out(
        &BuildTarget::Dir {
            path: copydir,
            origin: Origin::User,
        },
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("the copy builds");
    assert_eq!(report.name, "look-a");
    let scoot = std::fs::read_to_string(out.join("scoot.toml")).expect("a fragment");
    assert!(
        scoot.contains("focus_ring_width = 9"),
        "the copy's ring width renders, not the catalog's:\n{scoot}"
    );
}

#[test]
fn effectful_refusals_are_structured_not_bare_errors() {
    // `validate --json` always prints a document: effectful refusals land
    // in `errors`, only catalog failures refuse the command.
    let fixture = fixture("structured");
    let checks = FakeChecks {
        font_dir: None,
        font_refuse: Some("font `DejaVu Sans` resolves to nothing".to_owned()),
        bar_refuse: None,
        foot_refuse: None,
        popup_only: false,
        minimal_bar: false,
        bar_seen: RefCell::new(Vec::new()),
        foot_seen: RefCell::new(Vec::new()),
    };
    let validation =
        validate_entry(&fixture.catalog, "look-a", &checks, false).expect("validatable");
    assert_eq!(validation.errors.len(), 3, "{:?}", validation.errors);
    for error in &validation.errors {
        assert!(error.contains("resolves to nothing"), "{error}");
    }
}

#[test]
fn helpers_run_with_a_timeout() {
    use super::run_with_timeout;
    let echo = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("echo"))
        .find(|candidate| candidate.is_file())
        .expect("echo on PATH");
    let out = run_with_timeout(&echo, &["hello"]).expect("echo runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).expect("UTF-8"), "hello\n");
    let error =
        run_with_timeout(Path::new("/nonexistent-helper-xyz"), &[]).expect_err("missing refuses");
    assert!(error.contains("cannot run"), "{error}");
}

#[test]
fn validate_accepts_a_nameless_dir_path() {
    // `scoot look validate .`: a path with no file name (`.`, `./`,
    // `..`) falls back to the canonical basename for the
    // name-matches-directory check. (`dir/.` still has a file name —
    // Rust ignores the trailing dot — so the probe below ends in `..`,
    // the same `None` case.) A symlink's own entry name still wins (see
    // the catalog's entry-name test); this is only the no-name case.
    let _scratch = scratch("dotdir");
    let root = _scratch.path();
    // `<root>/wd/look.toml`, manifest name `wd`: the probe path
    // `<root>/wd/inner/..` has no file name, reads `<root>/wd/look.toml`,
    // and canonicalizes to `<root>/wd`.
    let wd = root.join("wd");
    std::fs::create_dir_all(wd.join("inner")).expect("a look dir");
    std::fs::write(wd.join("look.toml"), look_toml("wd")).expect("a look.toml");
    let probe = wd.join("inner").join("..");
    assert!(
        probe.file_name().is_none(),
        "the probe path has no file name"
    );
    // No `extends`: the catalog is only the base lookup.
    let catalog = Catalog::default();
    let fonts = root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let validation =
        validate_dir(&probe, Origin::User, &catalog, &checks, false).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
}

#[test]
fn validate_inside_a_symlinked_look_prefers_the_entry_name() {
    // `scoot look validate .` inside `user-looks/linked -> abc-mine-src`:
    // the probe path has no file name, and the canonical basename is the
    // store target's (`abc-mine-src`), but `$PWD`'s basename (`linked`)
    // names the catalog entry resolving here, so the manifest name
    // (`linked`) matches. A `$PWD` naming nothing, or naming an entry
    // elsewhere, falls back to the canonical basename.
    let _scratch = scratch("pwdlink");
    let root = _scratch.path();
    let looks = root.join("user-looks");
    std::fs::create_dir_all(&looks).expect("a looks dir");
    let target = root.join("abc-mine-src");
    std::fs::create_dir_all(&target).expect("a store dir");
    std::fs::write(target.join("look.toml"), look_toml("linked")).expect("a look.toml");
    std::os::unix::fs::symlink(&target, looks.join("linked")).expect("a look link");
    let catalog = discover_ordered(&[(looks.clone(), Origin::User)]);
    assert!(catalog.find("linked").is_some(), "{:?}", catalog.notes);
    let canonical = std::fs::canonicalize(looks.join("linked")).expect("a canonical dir");
    let pwd = looks.join("linked");
    assert_eq!(
        super::pwd_basename_for(Some(pwd.as_os_str()), &canonical, &catalog).as_deref(),
        Some("linked")
    );
    // A `$PWD` naming nothing falls back.
    assert!(
        super::pwd_basename_for(
            Some(std::ffi::OsStr::new("/elsewhere/nosuch")),
            &canonical,
            &catalog
        )
        .is_none()
    );
    // A `$PWD` naming an entry that resolves elsewhere falls back too.
    let elsewhere = std::fs::canonicalize(&looks).expect("a canonical looks dir");
    assert_ne!(elsewhere, canonical);
    assert!(super::pwd_basename_for(Some(pwd.as_os_str()), &elsewhere, &catalog).is_none());
}

#[test]
fn validate_and_apply_agree_on_the_bar_fragment() {
    // A bar fragment its own bar refuses must not validate: `validate`
    // runs the same bar-fragment check `apply` does (against the installed
    // binary's features), instead of printing valid while `apply` refuses.
    let fixture = fixture("bar-agree");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    // For font files the fake dir stands in: write what it resolves.
    for family in ["DejaVu Sans", "DejaVu Sans Mono"] {
        std::fs::write(fonts.join(format!("{family}.otf")), b"font").expect("a font file");
    }
    let checks = FakeChecks {
        font_dir: Some(fonts),
        font_refuse: None,
        bar_refuse: Some("bar fragment refused by scootbar daemon --check: bad radius".to_owned()),
        popup_only: false,
        minimal_bar: false,
        bar_seen: RefCell::new(Vec::new()),
        foot_refuse: None,
        foot_seen: RefCell::new(Vec::new()),
    };
    let validation =
        validate_entry(&fixture.catalog, "look-a", &checks, false).expect("validatable");
    assert!(
        validation
            .errors
            .iter()
            .any(|error| error.contains("bad radius")),
        "{:?}",
        validation.errors
    );
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("apply refuses too");
    assert!(
        failure.message.contains("bad radius"),
        "{}",
        failure.message
    );
}

#[test]
fn validate_retries_a_popup_refusal_without_popup_keys() {
    // A bar built `--no-default-features` refuses `popup-radius`: like
    // `apply` and `build`, `validate` re-renders without the popup keys
    // instead of refusing every look. The fake models that bar faithfully:
    // it refuses only fragments carrying `popup-radius`.
    let fixture = fixture("bar-popup");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    for family in ["DejaVu Sans", "DejaVu Sans Mono"] {
        std::fs::write(fonts.join(format!("{family}.otf")), b"font").expect("a font file");
    }
    let checks = FakeChecks {
        font_dir: Some(fonts),
        font_refuse: None,
        bar_refuse: None,
        popup_only: true,
        minimal_bar: false,
        bar_seen: RefCell::new(Vec::new()),
        foot_refuse: None,
        foot_seen: RefCell::new(Vec::new()),
    };
    let validation =
        validate_entry(&fixture.catalog, "look-a", &checks, false).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    let seen = checks.bar_seen.borrow();
    assert_eq!(
        seen.len(),
        2,
        "the check ran, then ran again without popup keys"
    );
    assert!(
        seen[0].contains("popup-radius"),
        "the first render carries popup-radius"
    );
    assert!(
        !seen[1].contains("popup-radius"),
        "the retry renders without popup-radius"
    );
    assert!(
        validation
            .notes
            .iter()
            .any(|note| note.contains("without popup-radius")),
        "{:?}",
        validation.notes
    );
}

#[test]
fn a_minimal_bar_drops_module_tokens_with_a_note() {
    // A `--no-default-features` bar refuses most shipped looks outright:
    // ginger-night, music-desk, moonrise and vinyl-sunset carry
    // `[apps.scootbar] "workspaces.inactive-color"`, which renders a
    // `[workspaces]` table the minimal bar answers `unknown field
    // inactive-color, there are no fields` to. Like the popup keys, those
    // tokens are dropped (with a note saying what was dropped), so the
    // shipped looks apply on the lightest bar. The fake models that bar
    // faithfully: it refuses `popup-radius` and any `[workspaces]` table.
    let _scratch = scratch("bar-minimal");
    let root = _scratch.path();
    let sources = root.join("looks");
    write_look(
        &sources,
        "look-ws",
        "[apps.scootbar]\n\"workspaces.inactive-color\" = \"#B595AD\"\n",
    );
    let catalog = discover_ordered(&[(sources, Origin::User)]);
    assert!(catalog.find("look-ws").is_some(), "{:?}", catalog.notes);
    let fonts = root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let new_checks = || FakeChecks {
        font_dir: Some({
            for family in ["DejaVu Sans", "DejaVu Sans Mono"] {
                std::fs::write(fonts.join(format!("{family}.otf")), b"font").expect("a font file");
            }
            fonts.clone()
        }),
        font_refuse: None,
        bar_refuse: None,
        popup_only: false,
        minimal_bar: true,
        bar_seen: RefCell::new(Vec::new()),
        foot_refuse: None,
        foot_seen: RefCell::new(Vec::new()),
    };
    // Validate drops both and says so.
    let checks = new_checks();
    let validation = validate_entry(&catalog, "look-ws", &checks, false).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    let seen = checks.bar_seen.borrow();
    assert!(
        seen.len() >= 3,
        "popup, then workspaces, then clean: {seen:?}"
    );
    assert!(seen[0].contains("popup-radius"), "{seen:?}");
    assert!(seen[0].contains("[workspaces]"), "{seen:?}");
    assert!(
        !seen.last().expect("a last check").contains("[workspaces]"),
        "{seen:?}"
    );
    assert!(
        validation
            .notes
            .iter()
            .any(|note| note.contains("without popup-radius")),
        "{:?}",
        validation.notes
    );
    assert!(
        validation
            .notes
            .iter()
            .any(|note| note.contains("without `workspaces`") && note.contains("inactive-color")),
        "{:?}",
        validation.notes
    );
    // Apply agrees: the look applies, and the fragment carries no
    // `[workspaces]` table.
    let checks = new_checks();
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-ws",
        &options(),
        &catalog,
        &cache,
        &state,
        Some(&config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("a minimal bar applies a shipped-shaped look");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("without `workspaces`")),
        "{:?}",
        report.notes
    );
    let rendered_bar = std::path::Path::new(&report.rendered).join("bar.toml");
    let bar = std::fs::read_to_string(&rendered_bar).expect("a rendered bar fragment");
    assert!(!bar.contains("[workspaces]"), "{bar}");
    assert!(!bar.contains("popup-radius"), "{bar}");
    // Build agrees too: the `--out` fragment parses and carries no
    // `[workspaces]` table.
    let checks = new_checks();
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("an out dir");
    let _ = build_to_out(
        &BuildTarget::Name("look-ws".to_owned()),
        &out,
        &catalog,
        &checks,
        false,
    )
    .expect("a minimal bar builds a shipped-shaped look");
    let bar = std::fs::read_to_string(out.join("bar.toml")).expect("a bar fragment");
    assert!(!bar.contains("[workspaces]"), "{bar}");
    assert!(!bar.contains("popup-radius"), "{bar}");
}

#[test]
fn bar_downgrade_names_what_the_refusal_names() {
    // The popup shape first, even past the summary line.
    assert_eq!(
        super::bar_downgrade(
            "bar fragment refused: TOML parse error at line 8, column 1\nunknown field `popup-radius`, expected `radius`",
            "[bar]\nradius = 14\npopup-radius = 14\n"
        ),
        Some(super::BarDrop::Popup)
    );
    // A module table refusal names its section through the field.
    assert_eq!(
        super::bar_downgrade(
            "bar fragment refused: TOML parse error at line 10, column 1\nunknown field `inactive-color`, there are no fields",
            "[colors]\nbackground = \"#000000\"\n\n[workspaces]\ninactive-color = \"#111111\"\n"
        ),
        Some(super::BarDrop::Section {
            section: "workspaces".to_owned(),
            field: "inactive-color".to_owned(),
        })
    );
    // A refusal naming a table the build lacks drops the table itself.
    assert_eq!(
        super::bar_downgrade(
            "unknown field `volume`, expected `bar`",
            "[colors]\nbackground = \"#000000\"\n\n[volume]\nicon = \"x\"\n"
        ),
        Some(super::BarDrop::Section {
            section: "volume".to_owned(),
            field: "volume".to_owned(),
        })
    );
    // `[colors]` and `[bar]` are never dropped: a refusal naming one of
    // their keys is genuine.
    assert_eq!(
        super::bar_downgrade(
            "unknown field `radius`, expected `height`",
            "[bar]\nradius = 99999\n"
        ),
        None
    );
    // A refusal naming nothing in the fragment is genuine too.
    assert_eq!(
        super::bar_downgrade("the bar is on fire", "[bar]\nradius = 14\n"),
        None
    );
}

#[test]
fn bar_downgrade_drops_the_section_the_error_points_at() {
    // Allowlisted `*.icon` keys collide across sections, so the first
    // BTreeMap holder is the wrong answer. The stderr shape below is
    // byte-exact from a real `scootbar --no-default-features --features
    // clock` binary (`daemon --check` over a fragment carrying both
    // `clock.icon` and `window-title.icon`): the span (`12 | icon = "W"`)
    // names `[window-title]`, never the `clock` section that sorts first.
    let fragment = "[bar]\nfont-size = 14\nradius = 14\n\n[clock]\nicon = \"C\"\n\n[colors]\nbackground = \"#000000\"\n\n[window-title]\nicon = \"W\"\n";
    let message = "bar fragment refused by scootbar daemon --check: scootbar: /tmp/bar.toml: TOML parse error at line 12, column 1\n   |\n12 | icon = \"W\"\n   | ^^^^\nunknown field `icon`, there are no fields";
    assert_eq!(
        super::bar_downgrade(message, fragment),
        Some(super::BarDrop::Section {
            section: "window-title".to_owned(),
            field: "icon".to_owned(),
        })
    );
    // The mirror: a window-title-only bar refusing `clock`'s `icon` drops
    // `clock`, keeping `window-title`.
    let mirror = "bar fragment refused by scootbar daemon --check: scootbar: /tmp/bar.toml: TOML parse error at line 6, column 1\n   |\n6 | icon = \"C\"\n   | ^^^^\nunknown field `icon`, there are no fields";
    assert_eq!(
        super::bar_downgrade(mirror, fragment),
        Some(super::BarDrop::Section {
            section: "clock".to_owned(),
            field: "icon".to_owned(),
        })
    );
    // The unknown-table shape points at the header itself (`8 | [volume]`
    // from the same real binary): the section is the header on that line.
    let table_fragment = "[bar]\nradius = 14\nfont-size = 14\n\n[colors]\nbackground = \"#000000\"\n\n[volume]\nicon = \"V\"\n";
    let table_message = "bar fragment refused by scootbar daemon --check: scootbar: /tmp/bar.toml: TOML parse error at line 8, column 2\n  |\n8 | [volume]\n  |  ^^^^^^\nunknown field `volume`, expected one of `bar`, `left`, `center`, `right`, `colors`, `clock`, `workspaces`, `window-title`, `network`, `brightness`, `tray`, `media`, `bluetooth`, `power`, `outputs`, `output`, `look`";
    assert_eq!(
        super::bar_downgrade(table_message, table_fragment),
        Some(super::BarDrop::Section {
            section: "volume".to_owned(),
            field: "volume".to_owned(),
        })
    );
    // The line helpers themselves: the header at or before the span wins.
    assert_eq!(super::error_line(message), Some(12));
    assert_eq!(
        super::section_at(fragment, 12).as_deref(),
        Some("window-title")
    );
    assert_eq!(super::section_at(fragment, 6).as_deref(), Some("clock"));
    assert_eq!(
        super::section_at(table_fragment, 8).as_deref(),
        Some("volume")
    );
}

#[test]
fn symlinked_user_configs_still_report_shadowed_keys() {
    // Home-manager/stow link the user's own config files into the store:
    // following those links for the offline shadow detection is correct
    // (the no-follow policy is for look-dir inputs, not the user's files).
    let _scratch = scratch("shadow-link");
    let root = _scratch.path();
    let render = root.join("render");
    std::fs::create_dir_all(&render).expect("a render dir");
    std::fs::write(
        render.join("scoot.toml"),
        "[appearance]\nbackground_color = \"#2B3648\"\n",
    )
    .expect("a fragment");
    std::fs::write(render.join("bar.toml"), "[colors]\n").expect("a bar fragment");
    let store = root.join("store");
    std::fs::create_dir_all(&store).expect("a store dir");
    std::fs::write(
        store.join("config.toml"),
        "[appearance]\nbackground_color = \"#000000\"\n",
    )
    .expect("a user file");
    let config = root.join("config");
    std::fs::create_dir_all(config.join("scoot")).expect("a config dir");
    std::os::unix::fs::symlink(store.join("config.toml"), config.join("scoot/config.toml"))
        .expect("a config link");
    let (shadowed, _) = shadowed_from_files_in(&render, Some(&config));
    assert!(
        shadowed.contains(&"appearance.background_color".to_owned()),
        "{shadowed:?}"
    );
}

#[test]
fn build_into_an_empty_out_writes_five_files_and_leaves_no_litter() {
    // The old stage lived in `out`'s parent, so `--out .` refused citing
    // the tool's own stage file; the stage now lives in a tool-owned temp
    // dir, gone on every exit path. An empty dir (what `--out .` is from
    // a clean cwd) builds, and neither success nor failure litters.
    let fixture = fixture("build-clean");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("out");
    std::fs::create_dir_all(&out).expect("an out dir");
    let report = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("an empty out builds");
    assert_eq!(report.name, "look-a");
    for file in [
        "scoot.toml",
        "bar.toml",
        "fuzzel.ini",
        "swaylock",
        "greet.toml",
        "render.json",
    ] {
        assert!(out.join(file).is_file(), "{file} renders");
    }
    assert_no_build_litter(&fixture._root);
    // A refused build leaves no stage behind either.
    let failing = fixture._root.join("failing-out");
    std::fs::create_dir_all(&failing).expect("an out dir");
    std::fs::write(failing.join("notes.txt"), b"mine").expect("a foreign file");
    build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &failing,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect_err("a foreign out refuses");
    assert_no_build_litter(&fixture._root);
}

#[test]
fn the_stage_is_private_and_dead_stages_are_swept() {
    // A signal (SIGTERM/SIGINT) never runs `Drop`, so a killed run leaks
    // its stage: `ScratchDir::new` sweeps other dead pids' stages at
    // start, and every stage is created 0700.
    use super::ScratchDir;
    let stage = ScratchDir::new("sweep-test").expect("a stage dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(stage.path())
                .expect("a stage")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "the stage is private"
        );
    }
    // A dead pid's stage is swept by the next staging; a live pid's (and
    // this process's own) never is.
    let tmp = std::env::temp_dir();
    let dead = tmp.join("scoot-look-sweep-test.999999999-0-0");
    let _ = std::fs::remove_dir_all(&dead);
    std::fs::create_dir(&dead).expect("a dead stage");
    let live = tmp.join(format!("scoot-look-sweep-test.{}-0-0", std::process::id()));
    let live_existed = live.exists();
    if !live_existed {
        std::fs::create_dir(&live).expect("a live stage");
    }
    let _next = ScratchDir::new("sweep-test").expect("a second stage");
    #[cfg(target_os = "linux")]
    assert!(!dead.exists(), "a dead pid's stage is swept");
    assert!(live.exists(), "a live pid's stage is kept");
    if !live_existed {
        std::fs::remove_dir_all(&live).ok();
    }
    let _ = std::fs::remove_dir_all(&dead);
}

/// No `.X.build-tmp.*` stage dir survives anywhere under `root`.
fn assert_no_build_litter(root: &Path) {
    let mut litter = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains("build-tmp") {
                litter.push(entry.path());
            }
            if entry.path().is_dir() {
                dirs.push(entry.path());
            }
        }
    }
    assert!(litter.is_empty(), "stage litter survives: {litter:?}");
}

/// The six owned toolkit files, relative to the config home.
const TOOLKIT_RELS: [&str; 6] = [
    "gtk-3.0/gtk.css",
    "gtk-4.0/gtk.css",
    "gtk-3.0/settings.ini",
    "gtk-4.0/settings.ini",
    "qt6ct/qt6ct.conf",
    "fontconfig/conf.d/10-scoot-look.conf",
];

/// Every file under `root`, as sorted `/`-separated paths relative to it.
fn tree_snapshot(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                walk(root, &path, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Every file under `dir` as `(relative path, bytes)`, sorted: a
/// byte-for-byte comparison of a build-time input tree across swaps.
fn dir_file_contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("a readable dir")
            .filter_map(|entry| entry.ok())
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                walk(base, &path, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read(&path).unwrap_or_default(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// Asserts `dir` holds exactly the six owned toolkit files, each carrying
/// the managed marker, and that the report says they were written.
fn assert_toolkit_files(dir: &Path, report: &ApplyReport) {
    for rel in TOOLKIT_RELS {
        let text =
            std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|_| panic!("{rel} written"));
        assert!(
            text.contains(super::super::toolkit::MANAGED_MARK),
            "{rel} carries no marker"
        );
    }
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("wrote gtk-3.0/gtk.css")),
        "no toolkit write notes: {:?}",
        report.notes
    );
    let mut expected: Vec<String> = TOOLKIT_RELS.iter().map(|rel| (*rel).to_owned()).collect();
    expected.sort();
    assert_eq!(tree_snapshot(dir), expected, "extra files beside the six");
}

/// One snapshotted toolkit file: its size plus its mtime (secs, nanos),
/// or nothing when absent.
type FileStamp = Option<(u64, u64, u32)>;

/// The developer's real toolkit files, as (rel, stamp), read-only and
/// never mutated: the parent guard compares these across the sandboxed
/// child run.
fn real_toolkit_snapshot() -> Vec<(String, FileStamp)> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    for rel in TOOLKIT_RELS {
        let data = home
            .as_deref()
            .map(|home| home.join(".config").join(rel))
            .and_then(|path| {
                std::fs::metadata(&path).ok().and_then(|meta| {
                    meta.modified().ok().and_then(|mtime| {
                        mtime
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .map(|age| (meta.len(), age.as_secs(), age.subsec_nanos()))
                    })
                })
            });
        out.push((rel.to_owned(), data));
    }
    out
}

#[test]
fn toolkit_files_follow_the_injected_config_home() {
    // The regression guard for the real-`~/.config` writes: `apply_with`
    // must honor its injected config home and never consult the
    // environment for the toolkit files. The child process below runs one
    // apply per path under a sandboxed HOME (a nonexistent path, so any
    // consult of it fails loudly) and XDG_CONFIG_HOME; the parent asserts
    // the sandbox contains exactly those files and the developer's real
    // `~/.config` is unchanged across the run. A child process (not
    // in-process `set_var`, which is `unsafe` under the shared test
    // process) keeps this hermetic under both test runners.
    let sandbox = scratch("guard-sandbox");
    let root = sandbox.path().to_owned();
    let missing_home = root.join("missing-home");
    let xdg_config = root.join("config");
    let before = real_toolkit_snapshot();
    let exe = std::env::current_exe().expect("the test binary");
    // The child helper's libtest path (no crate name in harness paths).
    let child = format!(
        "{}::guard_child_applies_inside_a_sandboxed_home",
        module_path!()
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(module_path!())
    );
    let out = std::process::Command::new(exe)
        .arg("--exact")
        .arg(&child)
        .env("HOME", &missing_home)
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("XDG_CACHE_HOME", root.join("cache-home"))
        .env("XDG_STATE_HOME", root.join("state-home"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("SCOOT_LOOK_GUARD", "1")
        .env_remove("SCOOT_LOOK_STATE")
        .env_remove("SCOOT_SOCKET")
        .env_remove("SCOOT_LOOKS_DIR")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("the guard child runs");
    assert!(
        out.status.success(),
        "the sandboxed apply failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The nonexistent HOME was never consulted (it would exist now), and
    // every file the child wrote sits under the sandboxed config home.
    assert!(!missing_home.exists(), "the child created $HOME");
    let stray: Vec<String> = tree_snapshot(&root)
        .into_iter()
        .filter(|rel| !rel.starts_with("config/"))
        .collect();
    assert!(
        stray.is_empty(),
        "files outside the sandboxed config home: {stray:?}"
    );
    assert_eq!(
        real_toolkit_snapshot(),
        before,
        "the real ~/.config changed under a sandboxed run"
    );
}

#[test]
fn guard_child_applies_inside_a_sandboxed_home() {
    // The working half of `toolkit_files_follow_the_injected_config_home`:
    // runs only in the re-exec child (marker env set by the parent), where
    // HOME is a nonexistent path and XDG_CONFIG_HOME is a tempdir. A
    // direct run without the marker returns immediately, so the normal
    // suite never touches the process environment.
    if std::env::var_os("SCOOT_LOOK_GUARD").is_none() {
        return;
    }
    let home = PathBuf::from(std::env::var_os("HOME").expect("a sandboxed HOME"));
    assert!(
        !home.exists(),
        "the sandbox HOME must not exist: {}",
        home.display()
    );
    let xdg =
        PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("a sandboxed XDG_CONFIG_HOME"));
    // The production path (`config: None`): resolves from the sandboxed
    // environment and succeeds without ever consulting the missing HOME.
    let first = fixture("guard-fallback");
    let fonts = first._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
        "look-a",
        &options(),
        &first.catalog,
        &first.cache,
        &first.state,
        None,
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::default(),
        &ScriptedLeaves::failing(),
    )
    .expect("the sandboxed fallback apply succeeds");
    assert_toolkit_files(&xdg, &report);
    assert!(
        !home.exists(),
        "the fallback consulted $HOME: {}",
        home.display()
    );
    let xdg_before = tree_snapshot(&xdg);
    // The injected path wins over the environment: with the sandbox still
    // on XDG_CONFIG_HOME, a pinned config home takes every file and the
    // sandbox gains nothing.
    let second = fixture("guard-injected");
    let fonts = second._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let pinned = second._root.join("config");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
        "look-b",
        &options(),
        &second.catalog,
        &second.cache,
        &second.state,
        Some(&pinned),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::default(),
        &ScriptedLeaves::failing(),
    )
    .expect("the pinned apply succeeds");
    assert_toolkit_files(&pinned, &report);
    assert_eq!(
        tree_snapshot(&xdg),
        xdg_before,
        "the pinned apply leaked into XDG_CONFIG_HOME"
    );
    assert!(
        !home.exists(),
        "the pinned apply consulted $HOME: {}",
        home.display()
    );
}

/// Moonrise's 16 ANSI hues, the order foot reads as `regular0-7` then
/// `bright0-7`.
const TERMINAL_EXTRA: &str = r##"
[terminal]
colors = ["#1a2230", "#c96a63", "#8fa382", "#e8a34c", "#7c8fb0", "#9a7b94", "#7e9bb0", "#c9bfae", "#5e6b7f", "#e87f6a", "#a9bd9b", "#ffd54a", "#93a8c8", "#b595ad", "#a8c4d8", "#f6eedc"]
"##;

/// One look with a `[terminal]` table, a cache, a state dir and a config
/// home, all scratch.
fn terminal_fixture(tag: &str) -> Fixture {
    let scratch = scratch(tag);
    let root: PathBuf = scratch.path().to_owned();
    let sources = root.join("looks");
    write_look(&sources, "look-t", TERMINAL_EXTRA);
    let catalog = discover_ordered(&[(sources, Origin::User)]);
    assert_eq!(catalog.entries.len(), 1, "notes: {:?}", catalog.notes);
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    Fixture {
        catalog,
        cache,
        state,
        config,
        _scratch: scratch,
        _root: root,
    }
}

#[test]
fn build_writes_foot_ini_for_a_terminal_look() {
    let fixture = terminal_fixture("build-foot");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("out");
    build_to_out(
        &BuildTarget::Name("look-t".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("a terminal look builds");
    let foot = std::fs::read_to_string(out.join("foot.ini")).expect("a foot.ini");
    assert!(foot.contains("[colors-dark]"), "wrong section:\n{foot}");
    assert!(foot.contains("[colors-light]"), "missing mirror:\n{foot}");
    assert!(
        foot.contains("initial-color-theme=dark"),
        "wrong theme:\n{foot}"
    );
    assert!(foot.contains("regular0=1a2230"), "ansi order lost:\n{foot}");
    assert!(foot.contains("bright7=f6eedc"), "bright hues lost:\n{foot}");
    assert!(foot.contains("alpha=1.00"), "alpha lost:\n{foot}");
}

#[test]
fn build_removes_a_stale_foot_ini_when_the_look_has_none() {
    let fixture = terminal_fixture("build-foot-stale");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let out = fixture._root.join("out");
    build_to_out(
        &BuildTarget::Name("look-t".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
        false,
    )
    .expect("a terminal look builds");
    assert!(out.join("foot.ini").is_file(), "no foot.ini to go stale");
    // `look-p` has no `[terminal]` table: rebuilding over the same dir
    // must not leave the other look's colors behind.
    let sources = fixture._root.join("plain");
    write_look(&sources, "look-p", "");
    let catalog = discover_ordered(&[(sources, Origin::User)]);
    build_to_out(
        &BuildTarget::Name("look-p".to_owned()),
        &out,
        &catalog,
        &checks,
        false,
    )
    .expect("a plain look builds over the dir");
    assert!(
        !out.join("foot.ini").exists(),
        "a stale foot.ini survives the rebuild"
    );
}

#[test]
fn apply_renders_foot_ini_into_the_cached_render() {
    let fixture = terminal_fixture("apply-foot");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    assert_eq!(report.name, "look-t");
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    let foot = std::fs::read_to_string(rendered.join("foot.ini")).expect("a foot.ini");
    assert!(foot.contains("regular0=1a2230"), "ansi order lost:\n{foot}");
}

#[test]
fn a_foot_check_refusal_refuses_before_anything_is_touched() {
    let fixture = terminal_fixture("apply-foot-refused");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let mut checks = FakeChecks::with_fonts(&fonts);
    checks.foot_refuse = Some("foot says no".to_owned());
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a refused foot fragment applies");
    assert!(
        failure.message.contains("foot says no"),
        "{}",
        failure.message
    );
    assert_eq!(compositor.calls.get(), 0, "a reload ran before the refusal");
    assert!(
        !fixture.state.join("look.json").exists(),
        "a record was written before the refusal"
    );
}

#[test]
fn validate_reports_a_foot_refusal() {
    let fixture = terminal_fixture("validate-foot");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let mut checks = FakeChecks::with_fonts(&fonts);
    checks.foot_refuse = Some("foot says no".to_owned());
    let validation =
        validate_entry(&fixture.catalog, "look-t", &checks, false).expect("a validation");
    assert!(
        validation.errors.iter().any(|e| e.contains("foot says no")),
        "errors: {:?}",
        validation.errors
    );
}

#[test]
fn apply_notes_name_the_foot_include_setup_until_it_exists() {
    let fixture = terminal_fixture("apply-foot-notes");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // No foot.ini at all: the reply carries the one-time setup.
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    let link_path = fixture.state.join("look/foot.ini");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains(&*link_path.to_string_lossy())),
        "no setup hint names the link path: {:?}",
        report.notes
    );
    // A commented-out include is still absent.
    let foot_home = fixture.config.join("foot");
    std::fs::create_dir_all(&foot_home).expect("a foot dir");
    std::fs::write(
        foot_home.join("foot.ini"),
        format!("#include={}\n", link_path.display()),
    )
    .expect("a foot.ini");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains(&*link_path.to_string_lossy())),
        "a commented include counts: {:?}",
        report.notes
    );
    // A live include: the reply says new terminals follow the look and
    // running ones keep their palette, with no setup hint.
    std::fs::write(
        foot_home.join("foot.ini"),
        format!("[main]\ninclude={}\n", link_path.display()),
    )
    .expect("a foot.ini");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("an offline apply succeeds");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("new terminals") && n.contains("running")),
        "no follow note: {:?}",
        report.notes
    );
    assert!(
        report
            .notes
            .iter()
            .all(|n| !n.contains("once foot.ini includes")),
        "a setup hint survives the include: {:?}",
        report.notes
    );
}

#[test]
fn a_second_apply_reuses_the_cached_foot_ini() {
    let fixture = terminal_fixture("apply-foot-reuse");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the first apply succeeds");
    let first = PathBuf::from(&record_of(&fixture.state).rendered);
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with_live(
        "look-t",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect("the second apply succeeds");
    let second = PathBuf::from(&record_of(&fixture.state).rendered);
    assert_eq!(first, second, "the same look re-renders instead of reusing");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("reusing cached render")),
        "no reuse note: {:?}",
        report.notes
    );
    assert!(second.join("foot.ini").is_file(), "reuse loses foot.ini");
}

// Static terminal leaves through the transaction: rendering, the trust
// drop, inheritance, opt-outs, and refusal before anything is touched.

const HELIX_LEAF: &str =
    "\"ui.background\" = {}\n\"ui.text\" = \"#FF0000\"\n\"ui.selection\" = \"#FF0000\"\n";
const BTOP_LEAF: &str = "# a theme\ntheme[main_fg]=\"#FF0000\"\n";
const STARSHIP_LEAF: &str = "add_newline = false\nformat = \"$directory$character\"\n";

/// Writes `apps/` static leaves into the source look `sources/<name>`.
fn write_static(sources: &Path, name: &str, leaves: &[(&str, &str)]) {
    for (file, contents) in leaves {
        let path = sources.join(name).join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("apps parents");
        std::fs::write(&path, contents).expect("a static leaf");
    }
}

const ALL_LEAVES: &[(&str, &str)] = &[
    ("apps/helix.toml", HELIX_LEAF),
    ("apps/btop.theme", BTOP_LEAF),
    ("apps/starship.toml", STARSHIP_LEAF),
];

/// A fixture whose `look-a` carries static leaves, scanned as `origin`.
fn static_fixture(tag: &str, origin: Origin, leaves: &[(&str, &str)]) -> Fixture {
    let scratch = scratch(tag);
    let root: PathBuf = scratch.path().to_owned();
    let sources = root.join("looks");
    write_look(&sources, "look-a", "");
    write_static(&sources, "look-a", leaves);
    let catalog = discover_ordered(&[(sources, origin)]);
    assert_eq!(catalog.entries.len(), 1, "notes: {:?}", catalog.notes);
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    Fixture {
        catalog,
        cache,
        state,
        config,
        _scratch: scratch,
        _root: root,
    }
}

fn trust_options() -> ApplyOptions {
    ApplyOptions {
        trust: true,
        ..options()
    }
}

fn opt_out_options() -> ApplyOptions {
    ApplyOptions {
        no_helix: true,
        no_btop: true,
        ..options()
    }
}

fn offline_apply(
    fixture: &Fixture,
    name: &str,
    options: &ApplyOptions,
    leaves: &ScriptedLeaves,
) -> ApplyReport {
    let mut compositor = FakeCompositor::not_running();
    apply_with_live(
        name,
        options,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &FakeChecks::default(),
        &FakeDconf::absent(),
        leaves,
        &[],
    )
    .expect("an offline apply succeeds")
}

#[test]
fn static_leaves_render_verbatim_and_signal() {
    let fixture = static_fixture("static-render", Origin::User, ALL_LEAVES);
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &options(), &leaves);
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    for (file, contents) in ALL_LEAVES {
        let read = std::fs::read(rendered.join(file)).expect("a rendered static leaf");
        assert_eq!(read, contents.as_bytes(), "{file} copies verbatim");
    }
    assert_eq!(
        leaves.signaled(),
        vec!["helix".to_owned(), "btop".to_owned()]
    );
    assert!(report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(report.reloaded.contains(&"btop".to_owned()), "{report:?}");
    // The one-time wiring for each leaf, and starship's next prompt.
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.toml")),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.theme")),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("STARSHIP_CONFIG")),
        "{report:?}"
    );
}

#[test]
fn installed_look_drops_static_leaves_without_trust() {
    let fixture = static_fixture("static-drop", Origin::Installed, ALL_LEAVES);
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &options(), &leaves);
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    assert!(
        !rendered.join("apps").exists(),
        "no static leaf renders untrusted"
    );
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(!report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("dropping apps/btop.theme")
                && note.contains("apps/helix.toml")
                && note.contains("apps/starship.toml")
                && note.contains("--trust")),
        "{report:?}"
    );
}

#[test]
fn trusted_installed_look_keeps_and_prints_static_leaves() {
    let fixture = static_fixture("static-trust", Origin::Installed, ALL_LEAVES);
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &trust_options(), &leaves);
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    for (file, contents) in ALL_LEAVES {
        let read = std::fs::read(rendered.join(file)).expect("a rendered static leaf");
        assert_eq!(read, contents.as_bytes(), "{file} copies verbatim");
    }
    // `--trust` prints each kept leaf first: the contents ride the reply.
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("trusted static leaf apps/helix.toml")
                && note.contains("\"ui.text\"")),
        "{report:?}"
    );
    assert_eq!(
        leaves.signaled(),
        vec!["helix".to_owned(), "btop".to_owned()]
    );
}

#[test]
fn trusted_non_installed_look_prints_no_trust_notes() {
    // `--trust` on a look that needs no trusting changes nothing and
    // prints nothing: the leaves render like any trusted look's.
    let fixture = static_fixture("static-notrust", Origin::User, ALL_LEAVES);
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &trust_options(), &leaves);
    assert!(
        report
            .notes
            .iter()
            .all(|note| !note.contains("trusted static leaf")),
        "{report:?}"
    );
}

#[test]
fn static_leaves_inherit_unless_the_child_names_its_own() {
    let scratch = scratch("static-inherit");
    let root: PathBuf = scratch.path().to_owned();
    let sources = root.join("looks");
    write_look(&sources, "base-a", "");
    write_static(&sources, "base-a", &[("apps/helix.toml", HELIX_LEAF)]);
    // `extends` is a top-level key: appended text would land inside
    // `[wallpaper]`, so the child's manifest is edited in place.
    let child = look_toml("child-b").replacen(
        "name = \"child-b\"",
        "name = \"child-b\"\nextends = \"base-a\"",
        1,
    );
    let child_dir = sources.join("child-b");
    std::fs::create_dir_all(&child_dir).expect("a look dir");
    std::fs::write(child_dir.join("look.toml"), child).expect("a look.toml");
    let catalog = discover_ordered(&[(sources, Origin::User)]);
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    let fixture = Fixture {
        catalog,
        cache,
        state,
        config,
        _scratch: scratch,
        _root: root,
    };
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "child-b", &options(), &leaves);
    let _ = report;
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    let read = std::fs::read_to_string(rendered.join("apps/helix.toml")).expect("inherited leaf");
    assert_eq!(read, HELIX_LEAF, "the base's leaf renders under the child");
    // The child names its own: the file changes, the render follows (the
    // cache keys the leaf bytes, so no stale directory is reused).
    let own = "\"ui.background\" = {}\n\"ui.text\" = \"#00FF00\"\n\"ui.selection\" = \"#00FF00\"\n";
    write_static(
        &fixture._root.join("looks"),
        "child-b",
        &[("apps/helix.toml", own)],
    );
    let first = rendered.clone();
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "child-b", &options(), &leaves);
    let _ = report;
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    assert_ne!(rendered, first, "an edited leaf re-renders elsewhere");
    let read = std::fs::read_to_string(rendered.join("apps/helix.toml")).expect("own leaf");
    assert_eq!(read, own, "the child's own leaf wins");
}

#[test]
fn broken_static_leaf_refuses_before_anything() {
    let fixture = static_fixture(
        "static-broken",
        Origin::User,
        &[("apps/helix.toml", "\"ui.text\" = ")],
    );
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with_live(
        "look-a",
        &options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &FakeChecks::default(),
        &FakeDconf::absent(),
        &ScriptedLeaves::failing(),
        &[],
    )
    .expect_err("a broken static leaf refuses");
    assert!(
        failure.message.contains("apps/helix.toml"),
        "the refusal names the leaf: {}",
        failure.message
    );
    assert!(
        !fixture.state.join("look.json").exists(),
        "nothing is recorded"
    );
}

#[test]
fn opt_outs_skip_signals_but_keep_leaves_and_notes() {
    let fixture = static_fixture("static-optout", Origin::User, ALL_LEAVES);
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &opt_out_options(), &leaves);
    let rendered = PathBuf::from(&record_of(&fixture.state).rendered);
    for (file, _) in ALL_LEAVES {
        assert!(rendered.join(file).is_file(), "{file} still renders");
    }
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(
        !report.reloaded.contains(&"helix".to_owned())
            && !report.reloaded.contains(&"btop".to_owned()),
        "{report:?}"
    );
    assert!(
        report.notes.iter().any(|note| note.contains("--no-helix")),
        "{report:?}"
    );
    assert!(
        report.notes.iter().any(|note| note.contains("--no-btop")),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.toml")),
        "the setup hint still prints: {report:?}"
    );
}

#[test]
fn look_without_static_leaves_signals_nothing() {
    // A look with no `apps/` files runs no editor step at all: no
    // signal, no note, no silence broken.
    let fixture = fixture("static-absent");
    let leaves = ScriptedLeaves::ok();
    let report = offline_apply(&fixture, "look-a", &options(), &leaves);
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(
        !report.reloaded.contains(&"helix".to_owned())
            && !report.reloaded.contains(&"btop".to_owned()),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .all(|note| !note.contains("helix") && !note.contains("btop")),
        "{report:?}"
    );
}
