//! `sync` tests: the home-manager rule against scripted peers. Hermetic:
//! scratch catalogs, state dirs, caches and rendered roots, never the
//! environment.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::Origin;
use scoot_look::state::{LINK_NAME, RecordRead, read_record};

use super::super::apply::{ApplyOptions, Availability, Checks, apply_with};
use super::super::catalog::discover_ordered;
use super::super::catalog::tests::{look_toml, scratch, write_look};
use super::super::leaves::ScriptedLeaves;
use super::super::reload::{BarReload, CompositorReload, ReloadError, Reloaded};
use super::super::toolkit::{COLOR_SCHEME_KEY, Dconf};
use super::{SyncOptions, TargetSkips, parse_declared, sync_with};

struct Fixture {
    catalog: super::super::catalog::Catalog,
    cache: PathBuf,
    state: PathBuf,
    config: PathBuf,
    roots: PathBuf,
    _scratch: tempfile::TempDir,
    _root: PathBuf,
}

/// Two looks, a cache, a state dir, a config home and an (empty)
/// rendered-roots dir, all scratch.
fn fixture(tag: &str) -> Fixture {
    let scratch = scratch(tag);
    let root: PathBuf = scratch.path().to_owned();
    let sources = root.join("looks");
    write_look(&sources, "look-a", "");
    let b = look_toml("look-b").replace("ring-width = 2", "ring-width = 3");
    let bdir = sources.join("look-b");
    std::fs::create_dir_all(&bdir).expect("a look dir");
    std::fs::write(bdir.join("look.toml"), b).expect("a look.toml");
    let catalog = discover_ordered(&[(sources, Origin::Declared)]);
    assert_eq!(catalog.entries.len(), 2, "notes: {:?}", catalog.notes);
    let cache = root.join("cache");
    let state = root.join("state");
    let config = root.join("config");
    let roots = root.join("rendered-roots");
    std::fs::create_dir_all(&roots).expect("roots dir");
    Fixture {
        catalog,
        cache,
        state,
        config,
        roots,
        _scratch: scratch,
        _root: root,
    }
}

fn options() -> SyncOptions {
    SyncOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: false,
    }
}

fn apply_options() -> ApplyOptions {
    ApplyOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: false,
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
    fn ok() -> Self {
        Self::with_script(vec![Ok(Reloaded {
            applied: vec!["appearance".to_owned()],
            refused: Vec::new(),
            shadowed: Vec::new(),
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
}

impl FakeChecks {
    fn with_fonts(dir: &Path) -> Self {
        for family in ["DejaVu Sans", "DejaVu Sans Mono"] {
            std::fs::write(dir.join(format!("{family}.otf")), b"font").expect("a font file");
        }
        Self {
            font_dir: Some(dir.to_owned()),
        }
    }
}

impl Checks for FakeChecks {
    fn font_file(&self, family: &str) -> Result<Availability<String>, String> {
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
        assert!(fragment.is_file(), "the bar fragment renders");
        Ok(Availability::Available(()))
    }

    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        assert!(fragment.is_file(), "the foot fragment renders");
        Ok(Availability::Available(()))
    }
}

#[derive(Default)]
struct FakeDconf {
    live: Option<String>,
    writes: RefCell<Vec<(String, String)>>,
    missing: bool,
}

impl FakeDconf {
    fn absent() -> Self {
        Self {
            live: None,
            writes: RefCell::new(Vec::new()),
            missing: true,
        }
    }
}

impl Dconf for FakeDconf {
    fn available(&self) -> bool {
        !self.missing
    }

    fn read(&self, _key: &str) -> Result<Option<String>, String> {
        Ok(self.live.clone())
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

/// Seeds the state with `apply` (a live choice with no declared
/// default), returning the render dir.
fn seed(fixture: &Fixture, name: &str, checks: &dyn Checks) -> PathBuf {
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
        name,
        &apply_options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::ok(),
    )
    .expect("seeding apply succeeds");
    PathBuf::from(report.rendered)
}

fn record_of(state: &Path) -> scoot_look::state::Record {
    match read_record(state) {
        RecordRead::Present(record) => record,
        other => panic!("a record reads: {other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_sync(
    declared: Option<Option<String>>,
    claims: Vec<String>,
    fixture: &Fixture,
    roots: &[PathBuf],
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn Checks,
    dconf: &dyn Dconf,
) -> Result<super::super::apply::ApplyReport, super::super::apply::ApplyFailure> {
    sync_with(
        declared,
        claims,
        TargetSkips::default(),
        &options(),
        &fixture.catalog,
        &fixture.cache,
        roots,
        &fixture.state,
        Some(&fixture.config),
        compositor,
        bars,
        checks,
        dconf,
    )
}

#[test]
fn rule_one_starts_on_the_declared_default() {
    let fixture = fixture("rule-one");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let report = run_sync(
        Some(Some("look-b".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("rule one syncs");
    assert_eq!(report.name, "look-b");
    assert_eq!(report.generation, 1);
    assert!(!report.removed);
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-b");
    assert_eq!(record.chosen_by.as_deref(), Some("sync"));
    assert_eq!(record.declared_default.as_deref(), Some("look-b"));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("declared default")),
        "names the reason: {:?}",
        report.notes
    );
}

#[test]
fn rule_one_null_with_no_state_does_nothing() {
    let fixture = fixture("rule-one-null");
    let checks = FakeChecks::default();
    let mut compositor = FakeCompositor::not_running();
    let report = run_sync(
        Some(None),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("null with no state succeeds");
    assert!(!report.removed);
    assert!(matches!(read_record(&fixture.state), RecordRead::Missing));
    assert!(!fixture.state.join(LINK_NAME).exists());
}

#[test]
fn unmanaged_leaves_the_choice_alone() {
    let fixture = fixture("unmanaged");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let before = seed(&fixture, "look-a", &checks);
    let mut compositor = FakeCompositor::ok();
    let report = run_sync(
        None,
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("unmanaged sync succeeds");
    assert_eq!(report.name, "look-a");
    assert_eq!(compositor.calls.get(), 0, "no reload runs");
    let record = record_of(&fixture.state);
    assert_eq!(record.generation, 1, "the record is untouched");
    assert_eq!(PathBuf::from(record.rendered), before);
}

#[test]
fn rule_two_follows_a_changed_default() {
    let fixture = fixture("rule-two");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    seed(&fixture, "look-a", &checks);
    let mut compositor = FakeCompositor::ok();
    let report = run_sync(
        Some(Some("look-b".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("rule two syncs");
    assert_eq!(report.name, "look-b");
    assert_eq!(report.generation, 2);
    assert_eq!(compositor.calls.get(), 1, "the new look reloads in");
    let record = record_of(&fixture.state);
    assert_eq!(record.declared_default.as_deref(), Some("look-b"));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("declaration wins")),
        "names the reason: {:?}",
        report.notes
    );
}

#[test]
fn rule_two_null_removes_the_layer() {
    let fixture = fixture("rule-two-null");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    seed(&fixture, "look-a", &checks);
    let mut compositor = FakeCompositor::ok();
    let report = run_sync(
        Some(None),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("null removes the layer");
    assert!(report.removed);
    assert_eq!(report.name, "look-a", "names what was removed");
    assert_eq!(compositor.calls.get(), 1, "peers reload the removal in");
    assert!(!fixture.state.join(LINK_NAME).exists(), "the link is gone");
    assert!(
        matches!(read_record(&fixture.state), RecordRead::Missing),
        "the record is gone"
    );
}

#[test]
fn removal_rolls_back_on_a_refused_reload() {
    // Commit only after every reload succeeds: a refused removal
    // restores the record and the link, and re-runs nothing (nothing
    // completed).
    let fixture = fixture("remove-rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let rendered = seed(&fixture, "look-a", &checks);
    let mut compositor = FakeCompositor::with_script(vec![Err(ReloadError::Refused(
        "no layer for you".to_owned(),
    ))]);
    let failure = run_sync(
        Some(None),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect_err("a refused removal fails");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-a");
    assert_eq!(PathBuf::from(record.rendered), rendered);
}

#[test]
fn rule_three_falls_back_when_the_live_choice_left() {
    // The pure rule-3 shape: sync records declared look-b, the user
    // applies look-a by hand (a live choice that carries the recorded
    // default), then a generation drops look-a. The declaration did not
    // change, so rule 2 stays quiet and rule 3 falls back, saying so.
    let fixture = fixture("rule-three");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    run_sync(
        Some(Some("look-b".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("first sync");
    let mut compositor = FakeCompositor::not_running();
    apply_with(
        "look-a",
        &apply_options(),
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
        &ScriptedLeaves::ok(),
    )
    .expect("a manual apply carries the recorded default");
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-a");
    assert_eq!(record.declared_default.as_deref(), Some("look-b"));
    // A new generation whose catalog dropped look-a: rediscover over
    // look-b alone (a fresh dir, so the scan sees one look).
    let sources = fixture._root.join("looks");
    let gen2 = fixture._root.join("looks2");
    std::fs::create_dir_all(&gen2).expect("gen2 dir");
    std::os::unix::fs::symlink(sources.join("look-b"), gen2.join("look-b")).expect("gen2 look");
    let catalog = discover_ordered(&[(gen2, Origin::Declared)]);
    assert_eq!(catalog.entries.len(), 1);
    let mut compositor = FakeCompositor::ok();
    let report = sync_with(
        Some(Some("look-b".to_owned())),
        Vec::new(),
        TargetSkips::default(),
        &options(),
        &catalog,
        &fixture.cache,
        std::slice::from_ref(&fixture.roots),
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("rule three syncs");
    assert_eq!(report.name, "look-b");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("falling back")),
        "says so: {:?}",
        report.notes
    );
}

#[test]
fn rule_three_unknown_declared_refuses() {
    let fixture = fixture("rule-three-unknown");
    let checks = FakeChecks::default();
    let mut compositor = FakeCompositor::not_running();
    let failure = run_sync(
        Some(Some("look-c".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect_err("an unknown declared default refuses");
    assert!(
        failure.message.contains("look-c") && failure.message.contains("look-a"),
        "names the look and the catalog: {}",
        failure.message
    );
    assert!(
        matches!(read_record(&fixture.state), RecordRead::Missing),
        "nothing is touched"
    );
}

#[test]
fn rule_four_keeps_an_unchanged_choice_quiet() {
    // Seed through sync itself (so the record carries the declared
    // default), then sync again: the same catalog re-resolves to the
    // same cache render, so no reload runs and the record is untouched.
    let fixture = fixture("rule-four");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let first = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("first sync");
    let mut compositor = FakeCompositor::ok();
    let report = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("second sync");
    assert_eq!(report.name, "look-a");
    assert_eq!(report.generation, first.generation, "no new generation");
    assert_eq!(compositor.calls.get(), 0, "no reload runs");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("already active")),
        "says so: {:?}",
        report.notes
    );
}

#[test]
fn a_catalog_render_is_adopted_then_kept() {
    // The Nix layout: `rendered/<name>` beside the sources. The first
    // sync points the link at the catalog entry (not the cache); the
    // second finds it already active and reloads nothing.
    let fixture = fixture("catalog-render");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    // Render once through the cache path, then publish it as the
    // catalog's render (same bytes a derivation would have built: the
    // render.json names this source hash).
    let mut compositor = FakeCompositor::not_running();
    let first = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("first sync renders fresh");
    assert!(
        first.rendered.contains("cache"),
        "fresh means cache: {}",
        first.rendered
    );
    let published = fixture.roots.join("look-a");
    std::fs::create_dir_all(&published).expect("publish dir");
    let from = PathBuf::from(&first.rendered);
    for file in ["scoot.toml", "bar.toml", "render.json"] {
        std::fs::copy(from.join(file), published.join(file)).expect("publish file");
    }
    let mut compositor = FakeCompositor::ok();
    let second = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("second sync adopts the catalog render");
    assert_eq!(second.rendered, published.display().to_string());
    assert_eq!(second.generation, first.generation + 1);
    assert_eq!(compositor.calls.get(), 1, "the adopted render reloads in");
    assert!(
        second
            .notes
            .iter()
            .any(|note| note.contains("catalog render")),
        "says so: {:?}",
        second.notes
    );
    let mut compositor = FakeCompositor::ok();
    let third = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("third sync keeps");
    assert_eq!(third.generation, second.generation, "no new generation");
    assert_eq!(compositor.calls.get(), 0, "no reload runs");
}

#[test]
fn a_stale_catalog_render_falls_back_to_cache() {
    // A catalog render whose source hash no longer matches is not
    // trusted: sync renders fresh and says so, instead of pointing the
    // link at stale bytes.
    let fixture = fixture("stale-render");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let published = fixture.roots.join("look-a");
    std::fs::create_dir_all(&published).expect("publish dir");
    std::fs::write(
        published.join("render.json"),
        r#"{"name":"look-a","source_hash":"deadbeef","wallpaper":"","ui_font":"","bar_popup":true,"bar_dropped":[]}"#,
    )
    .expect("stale render.json");
    std::fs::write(published.join("scoot.toml"), b"[appearance]\n").expect("fragment");
    std::fs::write(published.join("bar.toml"), b"[colors]\n").expect("fragment");
    let mut compositor = FakeCompositor::not_running();
    let report = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("sync falls back");
    assert!(
        report.rendered.contains("cache"),
        "fresh means cache: {}",
        report.rendered
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("rendering fresh")),
        "says so: {:?}",
        report.notes
    );
}

#[test]
fn a_claimed_key_is_left_alone() {
    // The module's claims file owns color-scheme: sync applies the look
    // without writing the key, however the live value reads.
    let fixture = fixture("claims");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let dconf = FakeDconf::default();
    let mut compositor = FakeCompositor::not_running();
    let report = run_sync(
        Some(Some("look-a".to_owned())),
        vec![COLOR_SCHEME_KEY.to_owned()],
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &dconf,
    )
    .expect("sync applies around the claim");
    assert_eq!(report.name, "look-a");
    let writes = dconf.writes.borrow();
    assert_eq!(
        writes.len(),
        5,
        "every key but the claimed one writes: {writes:?}"
    );
    assert!(
        writes.iter().all(|(key, _)| key != COLOR_SCHEME_KEY),
        "the claimed key is never written: {writes:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("owned by the Nix configuration")),
        "says so: {:?}",
        report.notes
    );
    let record = record_of(&fixture.state);
    assert!(
        !record.dconf_wrote.contains_key("color-scheme"),
        "no ownership is recorded"
    );
    assert_eq!(record.dconf_wrote.len(), 5);
}

#[test]
fn a_failed_switch_rolls_back_to_the_live_choice() {
    // Commit only after every reload succeeds: a refused switch
    // restores the record and the link, and re-runs the peers that may
    // hold the new layer.
    let fixture = fixture("switch-rollback");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let rendered = seed(&fixture, "look-a", &checks);
    let mut compositor = FakeCompositor::ok();
    let bar = FakeBar::with_script(
        "way-0",
        vec![Err(ReloadError::Refused("bar says no".to_owned()))],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = run_sync(
        Some(Some("look-b".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &bars,
        &checks,
        &FakeDconf::absent(),
    )
    .expect_err("a refused switch fails");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(
        compositor.calls.get(),
        2,
        "the rollback re-runs the compositor"
    );
    let record = record_of(&fixture.state);
    assert_eq!(record.name, "look-a");
    assert_eq!(PathBuf::from(record.rendered), rendered);
}

#[test]
fn a_corrupt_record_refuses_before_anything_moves() {
    let fixture = fixture("corrupt");
    std::fs::create_dir_all(&fixture.state).expect("state dir");
    std::fs::write(fixture.state.join("look.json"), b"{oops").expect("corrupt record");
    let checks = FakeChecks::default();
    let mut compositor = FakeCompositor::not_running();
    let failure = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect_err("a corrupt record refuses");
    assert!(
        failure.message.contains("corrupt"),
        "says so: {}",
        failure.message
    );
}

#[test]
fn declared_files_parse() {
    // `parse_declared` over scratch files: a name, an empty (null)
    // default, claims lines, and the two refusals.
    let scratch = scratch("declared-files");
    let root = scratch.path();
    let claims = root.join("claims");
    std::fs::write(&claims, "  \n/org/gnome/desktop/interface/gtk-theme\n").expect("claims");
    let default = root.join("look-default");
    std::fs::write(&default, "moonrise\n").expect("default");
    let parsed = parse_declared(&default, &claims)
        .expect("parses")
        .expect("managed");
    assert_eq!(parsed.default.as_deref(), Some("moonrise"));
    assert_eq!(
        parsed.claims,
        vec!["/org/gnome/desktop/interface/gtk-theme".to_owned()]
    );
    std::fs::write(&default, "").expect("empty default");
    let parsed = parse_declared(&default, &claims)
        .expect("parses")
        .expect("managed");
    assert_eq!(parsed.default, None);
    std::fs::write(&default, "LOUD\n").expect("bad default");
    let error = parse_declared(&default, &claims).expect_err("refuses a bad name");
    assert!(error.contains("LOUD"), "names it: {error}");
    std::fs::remove_file(&default).expect("remove default");
    assert!(parse_declared(&default, &claims).expect("parses").is_none());
}

#[test]
fn target_skips_parse_one_name_per_line() {
    let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(TargetSkips::from_lines(&[]), TargetSkips::default());
    assert_eq!(
        TargetSkips::from_lines(&lines("cursor\n")),
        TargetSkips {
            cursor: true,
            fonts: false,
        }
    );
    assert_eq!(
        TargetSkips::from_lines(&lines("cursor\nfonts\n")),
        TargetSkips {
            cursor: true,
            fonts: true,
        }
    );
    // Blanks and unknown names never refuse: a newer writer's target
    // does not break an older sync.
    assert_eq!(
        TargetSkips::from_lines(&lines("  \ncursor\nwallpaper\n")),
        TargetSkips {
            cursor: true,
            fonts: false,
        }
    );
    assert!(!TargetSkips::default().any());
    assert!(
        (TargetSkips {
            cursor: true,
            fonts: false,
        })
        .any()
    );
}

/// [`Checks`] that refuses every font: proves a fonts-skipped sync
/// never resolves a face (a missing face must not refuse a sync that
/// themes no font). Bar and foot fragments pass; the fixture looks
/// carry a color wallpaper, so no wallpaper resolves.
struct FontRefusingChecks;

impl Checks for FontRefusingChecks {
    fn font_file(&self, family: &str) -> Result<Availability<String>, String> {
        Err(format!("font `{family}` is not installed"))
    }

    fn wallpaper(&self, path: &Path) -> Result<Availability<()>, String> {
        assert!(path.is_file(), "wallpaper resolves to a file");
        Ok(Availability::Available(()))
    }

    fn bar_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        assert!(fragment.is_file(), "the bar fragment renders");
        Ok(Availability::Available(()))
    }

    fn foot_fragment(&self, fragment: &Path) -> Result<Availability<()>, String> {
        assert!(fragment.is_file(), "the foot fragment renders");
        Ok(Availability::Available(()))
    }
}

fn sync_with_skips(
    declared: Option<Option<String>>,
    skips: TargetSkips,
    fixture: &Fixture,
    roots: &[PathBuf],
    compositor: &mut dyn CompositorReload,
    checks: &dyn Checks,
    dconf: &dyn Dconf,
) -> Result<super::super::apply::ApplyReport, super::super::apply::ApplyFailure> {
    sync_with(
        declared,
        Vec::new(),
        skips,
        &options(),
        &fixture.catalog,
        &fixture.cache,
        roots,
        &fixture.state,
        Some(&fixture.config),
        compositor,
        &[],
        checks,
        dconf,
    )
}

#[test]
fn cursor_skip_renders_fresh_without_cursor_keys() {
    // The bug shape: a catalog render exists carrying the cursor keys,
    // and the cursor target is off. Sync must not point the link at
    // it: it renders fresh without the triple instead.
    let fixture = fixture("cursor-skip");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let first = run_sync(
        Some(Some("look-a".to_owned())),
        Vec::new(),
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf::absent(),
    )
    .expect("first sync renders fresh");
    let published = fixture.roots.join("look-a");
    std::fs::create_dir_all(&published).expect("publish dir");
    let from = PathBuf::from(&first.rendered);
    for file in ["scoot.toml", "bar.toml", "render.json"] {
        std::fs::copy(from.join(file), published.join(file)).expect("publish file");
    }
    let published_scoot =
        std::fs::read_to_string(published.join("scoot.toml")).expect("published fragment");
    assert!(
        published_scoot.contains("cursor_theme"),
        "the catalog render carries the cursor"
    );
    let mut compositor = FakeCompositor::ok();
    let report = sync_with_skips(
        Some(Some("look-a".to_owned())),
        TargetSkips {
            cursor: true,
            fonts: false,
        },
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &checks,
        &FakeDconf::absent(),
    )
    .expect("cursor-skipped sync succeeds");
    assert!(
        report.rendered.contains("cache"),
        "skipped renders fresh, never the catalog entry: {}",
        report.rendered
    );
    let scoot =
        std::fs::read_to_string(PathBuf::from(&report.rendered).join("scoot.toml")).expect("fresh");
    for key in ["cursor_theme", "cursor_size", "cursor_color"] {
        assert!(!scoot.contains(key), "the layer carries {key}:\n{scoot}");
    }
    assert!(
        scoot.contains("background_color"),
        "the rest still renders:\n{scoot}"
    );
    // The bar font is not skipped: it still renders.
    let bar =
        std::fs::read_to_string(PathBuf::from(&report.rendered).join("bar.toml")).expect("bar");
    assert!(
        bar.contains("font = \""),
        "only the cursor is skipped:\n{bar}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("cursor target is off")),
        "says so: {:?}",
        report.notes
    );
    assert!(
        report
            .notes
            .iter()
            .all(|note| !note.contains("catalog render")),
        "no catalog render is used: {:?}",
        report.notes
    );
    assert_eq!(compositor.calls.get(), 1, "the filtered layer reloads in");
}

#[test]
fn fonts_skip_never_resolves_a_face_and_renders_no_bar_font() {
    // Every font refuses, and the fonts target is off: sync still
    // succeeds, the bar fragment carries no `font` key, and the
    // toolkit writes no font name, no font line and no fontconfig file.
    let fixture = fixture("fonts-skip");
    let checks = FontRefusingChecks;
    let dconf = FakeDconf::default();
    let mut compositor = FakeCompositor::ok();
    let report = sync_with_skips(
        Some(Some("look-a".to_owned())),
        TargetSkips {
            cursor: false,
            fonts: true,
        },
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &checks,
        &dconf,
    )
    .expect("fonts-skipped sync succeeds despite refusing fonts");
    let bar =
        std::fs::read_to_string(PathBuf::from(&report.rendered).join("bar.toml")).expect("bar");
    assert!(
        !bar.contains("font = \""),
        "the layer carries a bar font:\n{bar}"
    );
    // The cursor is not skipped: it still renders and writes.
    let scoot =
        std::fs::read_to_string(PathBuf::from(&report.rendered).join("scoot.toml")).expect("fresh");
    assert!(
        scoot.contains("cursor_theme"),
        "only the fonts are skipped:\n{scoot}"
    );
    let writes = dconf.writes.borrow();
    assert!(
        writes.iter().all(|(key, _)| !key.ends_with("/font-name")),
        "font-name written: {writes:?}"
    );
    assert!(
        writes.iter().any(|(key, _)| key.ends_with("/color-scheme")),
        "the rest still writes: {writes:?}"
    );
    assert!(
        !fixture
            .config
            .join("fontconfig/conf.d/10-scoot-look.conf")
            .is_file(),
        "the fontconfig file is written"
    );
    let ini = std::fs::read_to_string(fixture.config.join("gtk-3.0/settings.ini")).expect("ini");
    assert!(
        !ini.contains("gtk-font-name"),
        "the ini names a font:\n{ini}"
    );
    assert!(
        ini.contains("gtk-cursor-theme-name"),
        "only the font line leaves the ini:\n{ini}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("fonts target is off")),
        "says so: {:?}",
        report.notes
    );
}

#[test]
fn cursor_skip_leaves_the_cursor_dconf_keys_and_ini_lines_alone() {
    let fixture = fixture("cursor-skip-toolkit");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let dconf = FakeDconf::default();
    let mut compositor = FakeCompositor::ok();
    let report = sync_with_skips(
        Some(Some("look-a".to_owned())),
        TargetSkips {
            cursor: true,
            fonts: false,
        },
        &fixture,
        std::slice::from_ref(&fixture.roots),
        &mut compositor,
        &checks,
        &dconf,
    )
    .expect("cursor-skipped sync succeeds");
    let writes = dconf.writes.borrow();
    for key in ["cursor-theme", "cursor-size"] {
        let suffix = format!("/{key}");
        assert!(
            writes
                .iter()
                .all(|(written, _)| !written.ends_with(&*suffix)),
            "{key} written: {writes:?}"
        );
    }
    assert!(
        writes.iter().any(|(key, _)| key.ends_with("/font-name")),
        "only the cursor keys stay: {writes:?}"
    );
    let ini = std::fs::read_to_string(fixture.config.join("gtk-3.0/settings.ini")).expect("ini");
    assert!(
        !ini.contains("gtk-cursor-theme-name") && !ini.contains("gtk-cursor-theme-size"),
        "the ini names a cursor:\n{ini}"
    );
    assert!(
        ini.contains("gtk-font-name"),
        "only the cursor lines leave the ini:\n{ini}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("cursor-theme left alone")),
        "says so: {:?}",
        report.notes
    );
}
