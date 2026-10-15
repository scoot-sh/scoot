//! Swap tests: the transaction against scripted peers. Hermetic: scratch
//! catalogs, state dirs and caches, never the environment (except
//! `shadowed_from_files_in`, which takes its config home explicitly).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::state::{LINK_NAME, RecordRead, read_record};

use super::{
    ApplyOptions, Availability, BuildTarget, Checks, apply_with, build_to_out,
    shadowed_from_files_in, validate_dir, validate_entry,
};
use scoot_look::Origin;

use super::super::catalog::discover_ordered;
use super::super::catalog::tests::{look_toml, scratch, write_look};
use super::super::reload::{BarReload, CompositorReload, ReloadError, Reloaded};
use super::super::toolkit::Dconf;
use crate::look::catalog::Catalog;

struct Fixture {
    catalog: Catalog,
    cache: PathBuf,
    state: PathBuf,
    config: PathBuf,
    _root: PathBuf,
}

/// Two looks (`look-a` ring width 2, `look-b` ring width 3), a cache, a
/// state dir and a config home, all scratch.
fn fixture(tag: &str) -> Fixture {
    let root = scratch(tag);
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
        _root: root,
    }
}

fn options() -> ApplyOptions {
    ApplyOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: false,
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
    script: RefCell<VecDeque<Result<bool, String>>>,
    calls: Cell<usize>,
}

impl FakeBar {
    fn with_script(display: &str, script: Vec<Result<bool, String>>) -> Self {
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

    fn reload_look(&self) -> Result<bool, String> {
        self.calls.set(self.calls.get() + 1);
        self.script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("no scripted reply left".to_owned()))
    }
}

#[derive(Default)]
struct FakeChecks {
    font_dir: Option<PathBuf>,
    font_refuse: Option<String>,
    bar_refuse: Option<String>,
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
        if let Some(refusal) = &self.bar_refuse {
            return Err(refusal.clone());
        }
        assert!(fragment.is_file(), "the bar fragment renders");
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

    fn read(&self, _key: &str) -> Option<String> {
        self.live.clone()
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        self.writes
            .borrow_mut()
            .push((key.to_owned(), value.to_owned()));
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
    let report = apply_with(
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
    for file in ["scoot.toml", "bar.toml", "render.json"] {
        let text = std::fs::read_to_string(rendered.join(file)).expect("a rendered file");
        assert!(!text.is_empty(), "{file} renders");
    }
    let cache_name = rendered.file_name().expect("a dir name").to_string_lossy();
    assert!(
        cache_name.starts_with("look-a-") && cache_name.len() == "look-a-".len() + 16,
        "the cache dir carries the key: {cache_name}"
    );
}

#[test]
fn a_second_apply_bumps_the_generation() {
    let fixture = fixture("generation");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    apply_with(
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
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
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
    )
    .expect("the second apply succeeds");
    assert_eq!(report.generation, 2);
    assert_eq!(record_of(&fixture.state).name, "look-b");
}

#[test]
fn an_unknown_look_refuses_and_touches_nothing() {
    let fixture = fixture("unknown");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with(
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
        let failure = apply_with(
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
    let failure = apply_with(
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
    let failure = apply_with(
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
    apply_with(
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
    let failure = apply_with(
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
    // pending edits the scope exists to skip).
    let fixture = fixture("older");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let mut compositor = FakeCompositor::ok(&[], &[]);
    apply_with(
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
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![
        Err(ReloadError::Refused(
            "unknown variant `reload_look`, expected `reload`".to_owned(),
        )),
        Ok(Reloaded::default()),
    ]);
    let failure = apply_with(
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
    )
    .expect_err("an older compositor refuses");
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert!(
        failure.message.contains("unknown variant"),
        "{}",
        failure.message
    );
    // The swap and its rollback are the only two calls: an ordinary
    // error refuses, and nothing retries as a plain reload.
    assert_eq!(compositor.calls.get(), 2);
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
    apply_with(
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
    )
    .expect("the first apply succeeds");
    let mut compositor = FakeCompositor::with_script(vec![
        Ok(Reloaded::default()),
        Err(ReloadError::Refused("the old layer is bad too".to_owned())),
    ]);
    let bar = FakeBar::with_script(
        "wayland-0",
        vec![Err("the new layer is bad".to_owned()), Ok(true)],
    );
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let failure = apply_with(
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
    // Nothing to restore to, so what the failed swap wrote is removed;
    // the rollback reload still runs (and refuses: no layer is no layer).
    let mut compositor = FakeCompositor::with_script(vec![
        Err(ReloadError::Refused("nope".to_owned())),
        Err(ReloadError::Refused("nothing to roll back to".to_owned())),
    ]);
    let failure = apply_with(
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
    )
    .expect_err("the first apply fails");
    assert!(failure.rolled_back_to.is_none());
    assert!(failure.to_string().contains("ROLLED BACK WITH ERRORS"));
    assert_eq!(compositor.calls.get(), 2);
    assert!(!fixture.state.join("look.json").exists());
    assert!(!fixture.state.join(LINK_NAME).exists());
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
    let report = apply_with(
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
    apply_with(
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
    )
    .expect("the first apply succeeds");
    let mut compositor =
        FakeCompositor::with_script(vec![Ok(Reloaded::default()), Ok(Reloaded::default())]);
    let bar0 = FakeBar::with_script("wayland-0", vec![Ok(true), Ok(true)]);
    let bar1 = FakeBar::with_script("wayland-1", vec![Err("bad layer".to_owned()), Ok(true)]);
    let bars: Vec<&dyn BarReload> = vec![&bar0, &bar1];
    let failure = apply_with(
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
    )
    .expect_err("a bar refusal fails the swap");
    assert!(failure.message.contains("wayland-1"), "{}", failure.message);
    assert_eq!(failure.rolled_back_to.as_deref(), Some("look-a"));
    assert_eq!(compositor.calls.get(), 2, "the compositor rolls back too");
    assert_eq!(bar0.calls.get(), 2, "the reloaded bar rolls back");
    assert_eq!(bar1.calls.get(), 2, "the refusing bar rolls back too");
    assert_eq!(record_of(&fixture.state).name, "look-a");
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
    let failure = apply_with(
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
    apply_with(
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
    )
    .expect("the first apply succeeds");
    // A simulated crash between the record and the link: the link dangles
    // at a removed directory.
    std::fs::remove_file(fixture.state.join(LINK_NAME)).expect("the link goes");
    std::os::unix::fs::symlink("/nonexistent-crash-target", fixture.state.join(LINK_NAME))
        .expect("a dangling link");
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
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
    let report = apply_with(
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
    let report = apply_with(
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
    apply_with(
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
    )
    .expect("the apply succeeds");
    assert_eq!(dconf.writes.borrow().len(), 1);
    let record = record_of(&fixture.state);
    assert_eq!(record.color_scheme.as_deref(), Some("'prefer-dark'"));
    // A key someone else changed since is left alone and named.
    let dconf = FakeDconf {
        live: Some("'prefer-light'".to_owned()),
        ..FakeDconf::default()
    };
    let mut compositor = FakeCompositor::not_running();
    let report = apply_with(
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
    )
    .expect("the second apply succeeds");
    assert!(dconf.writes.borrow().is_empty());
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
    let report = apply_with(
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
                    assert!(
                        rendered.join("scoot.toml").is_file(),
                        "half a swap: {record:?}"
                    );
                    assert!(
                        rendered.join("bar.toml").is_file(),
                        "half a swap: {record:?}"
                    );
                    assert!(
                        rendered.join("render.json").is_file(),
                        "half a swap: {record:?}"
                    );
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
        apply_with(
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
    apply_with(
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
    apply_with(
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
    )
    .expect("the apply succeeds");
    assert!(fixture.cache.join("look").is_dir());
    assert!(fixture.cache.join("look.json").is_file());
    assert!(fixture.cache.join("look.lock").is_file());
}

#[test]
fn validate_and_build_cover_a_directory_and_a_name() {
    let fixture = fixture("validate");
    let fonts = fixture._root.join("fonts");
    std::fs::create_dir_all(&fonts).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&fonts);
    let validation = validate_entry(&fixture.catalog, "look-a", &checks).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    assert!(validate_entry(&fixture.catalog, "nope", &checks).is_err());
    let dir = fixture._root.join("looks").join("look-a");
    let validation =
        validate_dir(&dir, Origin::User, &fixture.catalog, &checks).expect("validatable");
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    let out = fixture._root.join("out");
    let report = build_to_out(
        &BuildTarget::Name("look-a".to_owned()),
        &out,
        &fixture.catalog,
        &checks,
    )
    .expect("buildable");
    assert_eq!(report.name, "look-a");
    for file in ["scoot.toml", "bar.toml", "render.json"] {
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
    };
    let mut compositor = FakeCompositor::not_running();
    let failure = apply_with(
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
    };
    let validation = validate_entry(&fixture.catalog, "look-a", &checks).expect("validatable");
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
