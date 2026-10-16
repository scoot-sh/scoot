//! Cycle and preview tests: catalog order, the preview file, and the
//! generation-guarded revert. Hermetic: scratch catalogs, state dirs,
//! caches and config homes, never the environment.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use scoot_look::Origin;

use super::super::apply::{ApplyOptions, Availability, Checks, PREVIEW_SUPERSEDED, apply_with};
use super::super::leaves::ScriptedLeaves;
use super::super::reload::{BarReload, CompositorReload, ReloadError, Reloaded};
use super::super::toolkit::Dconf;
use super::{
    Preview, PreviewRead, Step, clear_preview, cycle_target, keep_preview, random_target,
    read_preview, revert_if_current, reverter_argv, unapply, write_preview,
};
use scoot_look::catalog::{Catalog, discover_ordered};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch dir unique to this test process and call, removed on drop.
fn scratch(tag: &str) -> tempfile::TempDir {
    let id = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
    tempfile::Builder::new()
        .prefix(&format!(
            "scoot-look-cycle-{tag}-{}-{id}-",
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
}

/// Two looks (`look-a`, `look-b`), a cache, a state dir and a config home,
/// all scratch. Every `apply_with` and revert pins `config`, so no test
/// consults the live `$HOME` or `$XDG_CONFIG_HOME`.
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
    Fixture {
        catalog,
        cache: root.join("cache"),
        state: root.join("state"),
        config: root.join("config"),
        _scratch: scratch,
    }
}

fn options() -> ApplyOptions {
    ApplyOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: true,
        trust: false,
        no_helix: false,
        no_btop: false,
        from_compositor: false,
        preview: false,
        only_if_generation: None,
        nix_claims: Vec::new(),
    }
}

fn preview_options() -> ApplyOptions {
    ApplyOptions {
        preview: true,
        ..options()
    }
}

#[derive(Default)]
struct FakeCompositor {
    script: RefCell<VecDeque<Result<Reloaded, ReloadError>>>,
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
        }
    }
}

impl CompositorReload for FakeCompositor {
    fn reload_look(&mut self) -> Result<Reloaded, ReloadError> {
        self.script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(ReloadError::Refused("no scripted reply left".to_owned())))
    }
}

#[derive(Default)]
struct FakeBar {
    display: String,
}

impl BarReload for FakeBar {
    fn display(&self) -> &str {
        &self.display
    }

    fn reload_look(&self) -> Result<bool, ReloadError> {
        Ok(true)
    }
}

#[derive(Default)]
struct FakeChecks;

impl Checks for FakeChecks {
    fn font_file(&self, _family: &str) -> Result<Availability<String>, String> {
        Ok(Availability::Unavailable(
            "no `fc-match` on PATH: font files unchecked".to_owned(),
        ))
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
struct FakeDconf;

impl Dconf for FakeDconf {
    fn available(&self) -> bool {
        false
    }

    fn read(&self, _key: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn write(&self, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn reset(&self, _key: &str) -> Result<(), String> {
        Ok(())
    }
}

fn record_name(state: &Path) -> Option<String> {
    match scoot_look::state::read_record(state) {
        scoot_look::state::RecordRead::Present(record) => Some(record.name),
        scoot_look::state::RecordRead::Missing => None,
        scoot_look::state::RecordRead::Corrupt(reason) => panic!("a corrupt record: {reason}"),
    }
}

fn record_generation(state: &Path) -> Option<u64> {
    match scoot_look::state::read_record(state) {
        scoot_look::state::RecordRead::Present(record) => Some(record.generation),
        scoot_look::state::RecordRead::Missing => None,
        scoot_look::state::RecordRead::Corrupt(reason) => panic!("a corrupt record: {reason}"),
    }
}

/// Applies through the fakes: no compositor, no bars, no toolkit writes.
fn apply(fixture: &Fixture, name: &str, preview: bool) {
    let mut compositor = FakeCompositor::not_running();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let options = if preview {
        preview_options()
    } else {
        options()
    };
    apply_with(
        name,
        &options,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
    )
    .unwrap_or_else(|failure| panic!("{name} applies: {failure}"));
}

#[test]
fn order_wraps_at_both_ends() {
    let fixture = fixture("cycle-order");
    let catalog = &fixture.catalog;
    assert_eq!(
        cycle_target(catalog, Some("look-a"), Step::Next),
        Some("look-b".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, Some("look-b"), Step::Next),
        Some("look-a".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, Some("look-a"), Step::Prev),
        Some("look-b".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, Some("look-b"), Step::Prev),
        Some("look-a".to_owned())
    );
    // A choice that left the catalog starts over, never nowhere.
    assert_eq!(
        cycle_target(catalog, Some("gone"), Step::Next),
        Some("look-a".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, Some("gone"), Step::Prev),
        Some("look-b".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, None, Step::Next),
        Some("look-a".to_owned())
    );
    assert_eq!(
        cycle_target(catalog, None, Step::Prev),
        Some("look-b".to_owned())
    );
    let empty = Catalog::default();
    assert_eq!(cycle_target(&empty, Some("look-a"), Step::Next), None);
    assert_eq!(cycle_target(&empty, None, Step::Prev), None);
}

#[test]
fn random_stays_inside_the_catalog() {
    let fixture = fixture("cycle-random");
    for _ in 0..100 {
        let name = random_target(&fixture.catalog, None).expect("a pick");
        assert!(
            name == "look-a" || name == "look-b",
            "random left the catalog: {name}"
        );
    }
    let empty = Catalog::default();
    assert_eq!(random_target(&empty, None), None);
    assert_eq!(random_target(&empty, Some("look-a")), None);
}

#[test]
fn random_avoids_the_active_look() {
    let fixture = fixture("cycle-random-avoids");
    // Two looks: every draw off the active one is the other, so a second
    // press always moves.
    for _ in 0..200 {
        assert_eq!(
            random_target(&fixture.catalog, Some("look-a")),
            Some("look-b".to_owned())
        );
        assert_eq!(
            random_target(&fixture.catalog, Some("look-b")),
            Some("look-a".to_owned())
        );
    }
    // A choice that left the catalog draws from the whole catalog.
    for _ in 0..100 {
        let name = random_target(&fixture.catalog, Some("gone")).expect("a pick");
        assert!(
            name == "look-a" || name == "look-b",
            "random left the catalog: {name}"
        );
    }
    // One look: the only pick is a stay.
    let scratch = scratch("cycle-random-single");
    let sources = scratch.path().join("looks");
    write_look(&sources, "look-a", "");
    let single = discover_ordered(&[(sources, Origin::User)]);
    assert_eq!(single.entries.len(), 1, "notes: {:?}", single.notes);
    assert_eq!(
        random_target(&single, Some("look-a")),
        Some("look-a".to_owned())
    );
}

#[test]
fn preview_file_round_trips() {
    let scratch = scratch("cycle-preview-file");
    let state = scratch.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    assert_eq!(read_preview(&state), PreviewRead::Missing);
    let preview = Preview {
        revert_to: Some("look-a".to_owned()),
        from_generation: 7,
    };
    write_preview(&state, &preview).expect("a preview write");
    assert_eq!(read_preview(&state), PreviewRead::Present(preview));
    let none = Preview {
        revert_to: None,
        from_generation: 1,
    };
    write_preview(&state, &none).expect("a preview write");
    assert_eq!(read_preview(&state), PreviewRead::Present(none));
    clear_preview(&state);
    assert_eq!(read_preview(&state), PreviewRead::Missing);
    // Clearing nothing is fine.
    clear_preview(&state);
    // Garbage is corruption, and a bad name never records.
    std::fs::write(state.join("look-preview.json"), b"not json").expect("garbage");
    assert!(matches!(read_preview(&state), PreviewRead::Corrupt(_)));
    std::fs::write(
        state.join("look-preview.json"),
        br#"{"revert-to":"LOUD","from-generation":2}"#,
    )
    .expect("a bad name");
    assert!(matches!(read_preview(&state), PreviewRead::Corrupt(_)));
    std::fs::write(state.join("look-preview.json"), br#"{"revert-to":null}"#)
        .expect("a missing generation");
    assert!(matches!(read_preview(&state), PreviewRead::Corrupt(_)));
    let bad = Preview {
        revert_to: Some("../evil".to_owned()),
        from_generation: 2,
    };
    assert!(write_preview(&state, &bad).is_err());
}

#[test]
fn a_preview_reverts_through_the_transaction() {
    let fixture = fixture("cycle-revert");
    apply(&fixture, "look-a", false);
    assert_eq!(record_generation(&fixture.state), Some(1));
    apply(&fixture, "look-b", true);
    assert_eq!(record_name(&fixture.state), Some("look-b".to_owned()));
    assert_eq!(record_generation(&fixture.state), Some(2));
    assert_eq!(
        read_preview(&fixture.state),
        PreviewRead::Present(Preview {
            revert_to: Some("look-a".to_owned()),
            from_generation: 2,
        })
    );
    let mut compositor = FakeCompositor::ok();
    let bar = FakeBar {
        display: "test".to_owned(),
    };
    let bars: Vec<&dyn BarReload> = vec![&bar];
    let outcome = revert_if_current(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a revert");
    assert_eq!(
        outcome,
        super::RevertOutcome::Reverted {
            name: Some("look-a".to_owned())
        }
    );
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
}

#[test]
fn a_preview_never_undoes_a_newer_apply() {
    let fixture = fixture("cycle-superseded");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    // A newer choice before the timer fires: the plain apply clears the
    // dead preview file as it moves the generation.
    apply(&fixture, "look-a", false);
    assert_eq!(record_generation(&fixture.state), Some(3));
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let outcome = revert_if_current(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a late revert");
    assert_eq!(outcome, super::RevertOutcome::NothingPending);
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
    assert_eq!(record_generation(&fixture.state), Some(3));
}

#[test]
fn a_moved_generation_supersedes_a_live_preview_file() {
    // The file survived while the record moved (a reverter racing the
    // newer apply between its check and its clear): the revert stands
    // down and drops the dead file.
    let fixture = fixture("cycle-superseded-file");
    apply(&fixture, "look-a", false);
    write_preview(
        &fixture.state,
        &Preview {
            revert_to: Some("look-b".to_owned()),
            from_generation: 999,
        },
    )
    .expect("a preview write");
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let outcome = revert_if_current(
        999,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a superseded revert");
    assert_eq!(outcome, super::RevertOutcome::Superseded);
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
    assert_eq!(record_generation(&fixture.state), Some(1));
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
}

#[test]
fn a_newer_preview_owns_its_file() {
    // The file moved on while this reverter slept: its `--from` no longer
    // matches, so it touches nothing (the newer reverter owns the file).
    let fixture = fixture("cycle-newer-preview");
    apply(&fixture, "look-a", false);
    let newer = Preview {
        revert_to: Some("look-a".to_owned()),
        from_generation: 2,
    };
    apply(&fixture, "look-b", true);
    assert_eq!(
        read_preview(&fixture.state),
        PreviewRead::Present(newer.clone())
    );
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let outcome = revert_if_current(
        1,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a replaced revert");
    assert_eq!(outcome, super::RevertOutcome::Superseded);
    assert_eq!(read_preview(&fixture.state), PreviewRead::Present(newer));
}

#[test]
fn a_plain_apply_clears_a_dead_preview_file() {
    let fixture = fixture("cycle-plain-clears");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    assert!(matches!(
        read_preview(&fixture.state),
        PreviewRead::Present(_)
    ));
    apply(&fixture, "look-a", false);
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
}

#[test]
fn keep_forgets_a_pending_preview() {
    let fixture = fixture("cycle-keep");
    assert_eq!(
        keep_preview(&fixture.state, Duration::from_millis(500)),
        Err("no timed preview is pending".to_owned())
    );
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    assert_eq!(
        keep_preview(&fixture.state, Duration::from_millis(500)),
        Ok("look-b".to_owned())
    );
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
    assert_eq!(record_name(&fixture.state), Some("look-b".to_owned()));
    assert_eq!(
        keep_preview(&fixture.state, Duration::from_millis(500)),
        Err("no timed preview is pending".to_owned())
    );
}

#[test]
fn keep_refuses_a_superseded_preview() {
    let fixture = fixture("cycle-keep-stale");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    apply(&fixture, "look-a", false);
    // The plain apply already cleared the dead file.
    assert_eq!(read_preview(&fixture.state), PreviewRead::Missing);
    assert_eq!(
        keep_preview(&fixture.state, Duration::from_millis(500)),
        Err("no timed preview is pending".to_owned())
    );
}

#[test]
fn the_generation_guard_refuses_atomically() {
    let fixture = fixture("cycle-guard");
    apply(&fixture, "look-a", false);
    let mut compositor = FakeCompositor::not_running();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let options = ApplyOptions {
        only_if_generation: Some(999),
        ..options()
    };
    let failure = apply_with(
        "look-b",
        &options,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
    )
    .expect_err("a stale revert refuses");
    assert!(
        failure.message.starts_with(PREVIEW_SUPERSEDED),
        "the refusal names itself: {}",
        failure.message
    );
    // Nothing moved: the choice is still the first one.
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
    assert_eq!(record_generation(&fixture.state), Some(1));
}

#[test]
fn unapply_clears_the_layer_and_honors_its_guard() {
    let fixture = fixture("cycle-unapply");
    apply(&fixture, "look-a", false);
    let mut compositor = FakeCompositor::not_running();
    let bars: Vec<&dyn BarReload> = Vec::new();
    // A moved generation clears nothing.
    assert_eq!(
        unapply(
            &fixture.state,
            &mut compositor,
            &bars,
            Duration::from_millis(500),
            Some(999),
            &FakeDconf,
        ),
        Ok(false)
    );
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
    assert_eq!(
        unapply(
            &fixture.state,
            &mut compositor,
            &bars,
            Duration::from_millis(500),
            Some(1),
            &FakeDconf,
        ),
        Ok(true)
    );
    assert_eq!(record_name(&fixture.state), None);
}

#[test]
fn reverter_argv_pins_state_and_socket() {
    let (exe, args) = reverter_argv(
        Path::new("/bin/scoot"),
        Path::new("/state/scoot"),
        Some(Path::new("/run/scoot.sock")),
        5,
        20,
    );
    assert_eq!(exe, PathBuf::from("/bin/scoot"));
    assert_eq!(
        args,
        vec![
            "look",
            "--state",
            "/state/scoot",
            "--socket",
            "/run/scoot.sock",
            "_preview-revert",
            "--from",
            "5",
            "--after",
            "20",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    );
    let (_, args) = reverter_argv(
        Path::new("/bin/scoot"),
        Path::new("/state/scoot"),
        None,
        5,
        20,
    );
    assert!(
        !args.iter().any(|arg| arg == "--socket"),
        "no explicit socket means no flag: {args:?}"
    );
}

#[test]
fn compositor_calls_count_through_a_revert() {
    // The revert drives the same scoped reloads an apply does: one call
    // per attempt here, not a silent skip.
    let fixture = fixture("cycle-calls");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    let calls = Cell::new(0usize);
    struct Counting<'a> {
        calls: &'a Cell<usize>,
    }
    impl CompositorReload for Counting<'_> {
        fn reload_look(&mut self) -> Result<Reloaded, ReloadError> {
            self.calls.set(self.calls.get() + 1);
            Err(ReloadError::NotRunning("no compositor answers".to_owned()))
        }
    }
    let mut compositor = Counting { calls: &calls };
    let bars: Vec<&dyn BarReload> = Vec::new();
    // No compositor runs: the revert still restores the record and link
    // (only the link moves, like an apply with nothing running).
    let outcome = revert_if_current(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a revert with nothing running");
    assert_eq!(
        outcome,
        super::RevertOutcome::Reverted {
            name: Some("look-a".to_owned())
        }
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
}

#[test]
fn lock_contention_is_recognized() {
    assert!(super::is_lock_contention(
        "cannot lock /s/look.lock: another writer holds it (os error 11)"
    ));
    assert!(!super::is_lock_contention(
        "the recorded choice is corrupt (boom)"
    ));
    assert!(!super::is_lock_contention(
        "preview superseded: the choice moved past generation 2 (now 3): not reverting"
    ));
}

#[test]
fn a_revert_waits_out_a_held_lock() {
    let fixture = fixture("cycle-retry-lock");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    // A swap holding the lock past the attempt's own short wait: the
    // retry sleeps it out and reverts, instead of failing loudly while
    // the preview sticks.
    let lock = scoot_look::state::acquire_lock(&fixture.state, Duration::from_secs(5))
        .expect("the test holds the lock");
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(lock);
    });
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let outcome = super::revert_with_retry(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(1),
        200,
        Duration::from_millis(10),
    )
    .expect("the retry waits the holder out");
    holder.join().expect("the holder thread");
    assert_eq!(
        outcome,
        super::RevertOutcome::Reverted {
            name: Some("look-a".to_owned())
        }
    );
    assert_eq!(record_name(&fixture.state), Some("look-a".to_owned()));
}

#[test]
fn a_revert_gives_up_when_the_lock_never_frees() {
    let fixture = fixture("cycle-retry-giveup");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    // Held for the whole call: every attempt contends, and the error
    // names the lock, loudly rather than silently.
    let _lock = scoot_look::state::acquire_lock(&fixture.state, Duration::from_secs(5))
        .expect("the test holds the lock");
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let failure = super::revert_with_retry(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(1),
        3,
        Duration::from_millis(1),
    )
    .expect_err("no attempt can take the lock");
    assert!(
        super::is_lock_contention(&failure),
        "the give-up names the lock: {failure}"
    );
}

#[test]
fn a_revert_does_not_retry_a_refusal() {
    let fixture = fixture("cycle-retry-no-repeat");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    // A corrupt record (not contention): the first attempt fails and no
    // retry follows — a retry could only repeat it.
    std::fs::write(
        fixture.state.join(scoot_look::state::RECORD_FILE),
        b"not json",
    )
    .expect("a corrupt record");
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let started = std::time::Instant::now();
    let failure = super::revert_with_retry(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(1),
        3,
        Duration::from_millis(500),
    )
    .expect_err("a corrupt record refuses");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "a refusal returns at once, never sleeps out retries"
    );
    assert!(
        failure.contains("corrupt"),
        "the refusal names the record: {failure}"
    );
}

/// A settings backend that remembers: reads see prior writes, so the
/// ownership round-trip is observable without touching the real dconf.
#[derive(Default)]
struct RecordingDconf {
    live: RefCell<std::collections::HashMap<String, String>>,
    writes: RefCell<Vec<(String, String)>>,
    resets: RefCell<Vec<String>>,
}

impl Dconf for RecordingDconf {
    fn available(&self) -> bool {
        true
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.live.borrow().get(key).cloned())
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        self.live
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        self.writes
            .borrow_mut()
            .push((key.to_owned(), value.to_owned()));
        Ok(())
    }

    fn reset(&self, key: &str) -> Result<(), String> {
        self.live.borrow_mut().remove(key);
        self.resets.borrow_mut().push(key.to_owned());
        Ok(())
    }
}

#[test]
fn unapply_restores_the_preview_written_toolkit_keys() {
    // A first-choice preview owns toolkit keys (nothing was recorded
    // before it); clearing the layer restores what was live before,
    // instead of stranding the preview's values with no record for the
    // next apply to own.
    let fixture = fixture("cycle-unapply-toolkit");
    let dconf = RecordingDconf::default();
    let mut compositor = FakeCompositor::not_running();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let options = ApplyOptions {
        no_toolkit: false,
        ..options()
    };
    apply_with(
        "look-a",
        &options,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &dconf,
        &ScriptedLeaves::ok(),
    )
    .unwrap_or_else(|failure| panic!("look-a applies: {failure}"));
    assert!(
        !dconf.writes.borrow().is_empty(),
        "the apply owned toolkit keys"
    );
    let mut compositor = FakeCompositor::not_running();
    assert_eq!(
        unapply(
            &fixture.state,
            &mut compositor,
            &bars,
            Duration::from_millis(500),
            Some(1),
            &dconf,
        ),
        Ok(true)
    );
    // Every key the preview wrote was unset before it: each is reset,
    // none keeps the preview's value.
    assert!(
        dconf.live.borrow().is_empty(),
        "no preview value survives: {:?}",
        dconf.live.borrow()
    );
    let mut resets = dconf.resets.borrow().clone();
    resets.sort();
    let mut written: Vec<String> = dconf
        .writes
        .borrow()
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    written.sort();
    assert_eq!(resets, written, "each owned key reset");
    assert_eq!(record_name(&fixture.state), None);
}

/// The six toolkit files a swap owns under the config home (the same set
/// `apply`'s guard asserts: duplicated here so this suite trips on its own
/// if the owned set changes).
const TOOLKIT_RELS: [&str; 6] = [
    "gtk-3.0/gtk.css",
    "gtk-4.0/gtk.css",
    "gtk-3.0/settings.ini",
    "gtk-4.0/settings.ini",
    "qt6ct/qt6ct.conf",
    "fontconfig/conf.d/10-scoot-look.conf",
];

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
fn revert_writes_toolkit_files_inside_the_pinned_config_home() {
    // The regression guard for the real-`~/.config` writes: the revert
    // runs the full transaction (its toolkit step is always on), so it
    // must honor its pinned config home with no home in the environment
    // at all. The child process below reverts under a removed HOME and
    // XDG_CONFIG_HOME; the parent asserts the child wrote exactly the
    // owned files into its pinned dir and the developer's real `~/.config`
    // is unchanged across the run. A child process (not in-process
    // `set_var`, which is `unsafe` under the shared test process) keeps
    // this hermetic under both test runners.
    let sandbox = scratch("cycle-guard-sandbox");
    let root = sandbox.path().to_owned();
    let before = real_toolkit_snapshot();
    let exe = std::env::current_exe().expect("the test binary");
    // The child helper's libtest path (no crate name in harness paths).
    let child = format!(
        "{}::guard_child_reverts_inside_a_sandboxed_home",
        module_path!()
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(module_path!())
    );
    let out = std::process::Command::new(exe)
        .arg("--exact")
        .arg(&child)
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CACHE_HOME", root.join("cache-home"))
        .env("XDG_STATE_HOME", root.join("state-home"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("SCOOT_LOOK_CYCLE_GUARD", "1")
        .env_remove("SCOOT_LOOK_STATE")
        .env_remove("SCOOT_SOCKET")
        .env_remove("SCOOT_LOOKS_DIR")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("the guard child runs");
    assert!(
        out.status.success(),
        "the sandboxed revert failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        real_toolkit_snapshot(),
        before,
        "the real ~/.config changed under a sandboxed run"
    );
}

#[test]
fn guard_child_reverts_inside_a_sandboxed_home() {
    // The working half of
    // `revert_writes_toolkit_files_inside_the_pinned_config_home`: runs
    // only in the re-exec child (marker env set by the parent), where HOME
    // and XDG_CONFIG_HOME are removed. A direct run without the marker
    // returns immediately, so the normal suite never touches the process
    // environment.
    if std::env::var_os("SCOOT_LOOK_CYCLE_GUARD").is_none() {
        return;
    }
    assert!(
        std::env::var_os("HOME").is_none(),
        "the sandbox must remove HOME"
    );
    assert!(
        std::env::var_os("XDG_CONFIG_HOME").is_none(),
        "the sandbox must remove XDG_CONFIG_HOME"
    );
    // The previously leaking path: a preview revert through the
    // transaction, whose toolkit step must land in the pinned config home
    // with no home resolvable at all (a `None` regression would skip the
    // files with "no home", and fail below).
    let fixture = fixture("cycle-guard-revert");
    apply(&fixture, "look-a", false);
    apply(&fixture, "look-b", true);
    std::fs::remove_dir_all(&fixture.config).ok();
    let mut compositor = FakeCompositor::ok();
    let bars: Vec<&dyn BarReload> = Vec::new();
    let outcome = revert_if_current(
        2,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        &mut compositor,
        &bars,
        &FakeChecks,
        &FakeDconf,
        &ScriptedLeaves::ok(),
        Duration::from_millis(500),
    )
    .expect("a revert with no home in the environment");
    assert_eq!(
        outcome,
        super::RevertOutcome::Reverted {
            name: Some("look-a".to_owned())
        }
    );
    for rel in TOOLKIT_RELS {
        assert!(
            fixture.config.join(rel).is_file(),
            "{rel} landed outside the pinned config home"
        );
    }
}
