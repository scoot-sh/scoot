//! `watch` tests: one reused slot per save, and a loud refusal that keeps
//! the last good look. Hermetic: scratch catalogs, state dirs, caches and
//! config homes, never the environment.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::Origin;
use scoot_look::catalog::{Catalog, discover_ordered};
use scoot_look::state::{LINK_NAME, RecordRead, read_record};

use super::super::leaves::ScriptedLeaves;
use super::super::reload::{BarReload, CompositorReload, ReloadError, Reloaded};
use super::super::toolkit::Dconf;
use super::{WatchOptions, apply_watched_dir, watch_slot};
use crate::look::apply::{ApplyFailure, ApplyReport, Availability, Checks};

/// A minimal valid look with a recolorable background.
fn look_toml(name: &str, background: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "dark"
license = "MIT"

[palette]
background = "{background}"
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
color = "{background}"
mode = "fill"
"##
    )
}

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("scoot-look-watch-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch dir")
}

fn write_watched(dir: &Path, name: &str, background: &str) -> PathBuf {
    let look = dir.join(name);
    std::fs::create_dir_all(&look).expect("a look dir");
    std::fs::write(look.join("look.toml"), look_toml(name, background)).expect("a look.toml");
    look
}

struct Fixture {
    catalog: Catalog,
    watched: PathBuf,
    cache: PathBuf,
    state: PathBuf,
    config: PathBuf,
    _scratch: tempfile::TempDir,
}

fn fixture(tag: &str) -> Fixture {
    let scratch = scratch(tag);
    let root = scratch.path().to_owned();
    let shipped = root.join("shipped");
    std::fs::create_dir_all(shipped.join("look-base")).expect("a base dir");
    std::fs::write(
        shipped.join("look-base").join("look.toml"),
        look_toml("look-base", "#2B3648"),
    )
    .expect("a base look.toml");
    let watched = write_watched(&root, "mine", "#2B3648");
    let catalog = discover_ordered(&[(shipped, Origin::Shipped)]);
    Fixture {
        catalog,
        watched,
        cache: root.join("cache"),
        state: root.join("state"),
        config: root.join("config"),
        _scratch: scratch,
    }
}

fn options() -> WatchOptions {
    WatchOptions {
        lock_wait: Duration::from_millis(500),
        no_toolkit: true,
        origin: Origin::User,
        nix_claims: Vec::new(),
    }
}

#[derive(Default)]
struct FakeCompositor {
    script: RefCell<VecDeque<Result<Reloaded, ReloadError>>>,
    calls: Cell<usize>,
}

impl FakeCompositor {
    fn not_running() -> Self {
        Self::with_script(vec![Err(ReloadError::NotRunning(
            "no compositor answers".to_owned(),
        ))])
    }
}

impl FakeCompositor {
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
        // Unscripted saves behave like a stopped compositor: skipped with
        // a note, so multi-save tests script nothing per save.
        self.script.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(ReloadError::NotRunning(
                "no compositor answers".to_owned(),
            ))
        })
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

fn record_of(state: &Path) -> scoot_look::state::Record {
    match read_record(state) {
        RecordRead::Present(record) => record,
        other => panic!("a record reads: {other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn save(
    fixture: &Fixture,
    options: &WatchOptions,
    checks: &FakeChecks,
    compositor: &mut FakeCompositor,
) -> Result<ApplyReport, ApplyFailure> {
    let leaves = ScriptedLeaves::ok();
    apply_watched_dir(
        &fixture.watched,
        options,
        &fixture.catalog,
        &fixture.cache,
        &fixture.state,
        Some(&fixture.config),
        compositor,
        &[],
        checks,
        &FakeDconf,
        &leaves,
        &[],
    )
}

fn rendered_dirs(cache: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(cache)
        .expect("a cache dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && !path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with('.'))
                    .unwrap_or(true)
        })
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn a_save_applies_into_one_reused_slot() {
    let fixture = fixture("slot");
    let dir = fixture._scratch.path().join("fonts");
    std::fs::create_dir_all(&dir).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&dir);
    let mut compositor = FakeCompositor::not_running();
    let report = save(&fixture, &options(), &checks, &mut compositor).expect("the first save applies");
    assert_eq!(report.name, "mine");
    assert_eq!(report.generation, 1);
    let slot = fixture.cache.join(watch_slot("mine"));
    assert_eq!(PathBuf::from(&report.rendered), slot);
    assert!(slot.join("scoot.toml").is_file(), "the slot renders");
    assert!(slot.join("bar.toml").is_file(), "the slot renders");
    let record = record_of(&fixture.state);
    assert_eq!(record.rendered, slot.display().to_string());
    assert_eq!(
        std::fs::read_link(fixture.state.join(LINK_NAME)).expect("a link"),
        slot
    );
    assert_eq!(rendered_dirs(&fixture.cache), vec![slot.clone()]);
    // A second save (a new background) re-renders the same slot: the
    // cache holds one directory, not two, and the generation bumps.
    std::fs::write(
        fixture.watched.join("look.toml"),
        look_toml("mine", "#1A2B3C"),
    )
    .expect("an edited look.toml");
    let report = save(&fixture, &options(), &checks, &mut compositor).expect("the edit applies");
    assert_eq!(report.generation, 2);
    assert_eq!(PathBuf::from(&report.rendered), slot);
    assert_eq!(rendered_dirs(&fixture.cache), vec![slot.clone()]);
    let scoot = std::fs::read_to_string(slot.join("scoot.toml")).expect("a fragment");
    assert!(scoot.contains("#1A2B3C"), "the slot carries the edit:\n{scoot}");
}

#[test]
fn an_invalid_save_is_refused_loudly_and_keeps_the_last_good_look() {
    let fixture = fixture("invalid");
    let dir = fixture._scratch.path().join("fonts");
    std::fs::create_dir_all(&dir).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&dir);
    let mut compositor = FakeCompositor::not_running();
    save(&fixture, &options(), &checks, &mut compositor).expect("the first save applies");
    let before = record_of(&fixture.state);
    let slot = fixture.cache.join(watch_slot("mine"));
    let scoot_before = std::fs::read_to_string(slot.join("scoot.toml")).expect("a fragment");
    // A broken save: not a color.
    std::fs::write(
        fixture.watched.join("look.toml"),
        look_toml("mine", "#2B3648").replace("#FFA45C", "amber"),
    )
    .expect("a broken look.toml");
    let failure = save(&fixture, &options(), &checks, &mut compositor)
        .expect_err("an invalid save refuses");
    assert!(failure.message.contains("invalid save"), "loud: {}", failure.message);
    assert!(failure.message.contains("mine"), "names the look kept: {}", failure.message);
    assert!(failure.rolled_back_to.is_none(), "nothing moved: no rollback");
    let after = record_of(&fixture.state);
    assert_eq!(after.rendered, before.rendered, "the record stays");
    assert_eq!(after.generation, before.generation, "no generation bump");
    assert_eq!(
        std::fs::read_link(fixture.state.join(LINK_NAME)).expect("a link"),
        slot,
        "the link stays"
    );
    assert_eq!(
        std::fs::read_to_string(slot.join("scoot.toml")).expect("a fragment"),
        scoot_before,
        "the slot keeps the last good render"
    );
    assert_eq!(rendered_dirs(&fixture.cache), vec![slot]);
}

#[test]
fn a_save_extends_its_base_through_the_catalog() {
    let fixture = fixture("extends");
    // The watched dir extends the catalog's base: only the name, the
    // title and the base live in the file.
    std::fs::write(
        fixture.watched.join("look.toml"),
        "schema = 1\nname = \"mine\"\ntitle = \"Mine\"\nextends = \"look-base\"\n",
    )
    .expect("an extending look.toml");
    let dir = fixture._scratch.path().join("fonts");
    std::fs::create_dir_all(&dir).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&dir);
    let mut compositor = FakeCompositor::not_running();
    let report = save(&fixture, &options(), &checks, &mut compositor).expect("the child applies");
    assert_eq!(report.name, "mine");
    let scoot = std::fs::read_to_string(
        fixture.cache.join(watch_slot("mine")).join("scoot.toml"),
    )
    .expect("a fragment");
    assert!(scoot.contains("#2B3648"), "the base color shows through:\n{scoot}");
}

#[test]
fn a_save_never_extends_an_installed_look() {
    let scratch = scratch("installed-base");
    let root = scratch.path().to_owned();
    let installed = root.join("installed");
    std::fs::create_dir_all(installed.join("theirs")).expect("an installed dir");
    std::fs::write(
        installed.join("theirs").join("look.toml"),
        look_toml("theirs", "#2B3648"),
    )
    .expect("an installed look.toml");
    let watched = write_watched(&root, "mine", "#2B3648");
    std::fs::write(
        watched.join("look.toml"),
        "schema = 1\nname = \"mine\"\ntitle = \"Mine\"\nextends = \"theirs\"\n",
    )
    .expect("an extending look.toml");
    let catalog = discover_ordered(&[(installed, Origin::Installed)]);
    let dir = root.join("fonts");
    std::fs::create_dir_all(&dir).expect("a fonts dir");
    let checks = FakeChecks::with_fonts(&dir);
    let mut compositor = FakeCompositor::not_running();
    let leaves = ScriptedLeaves::ok();
    let failure = apply_watched_dir(
        &watched,
        &options(),
        &catalog,
        &root.join("cache"),
        &root.join("state"),
        Some(&root.join("config")),
        &mut compositor,
        &[],
        &checks,
        &FakeDconf,
        &leaves,
        &[],
    )
    .expect_err("an installed base refuses");
    assert!(
        failure.message.contains("never extended"),
        "loud: {}",
        failure.message
    );
    assert!(
        !root.join("state").join("look.json").exists(),
        "nothing was recorded"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn inotify_sees_a_save_and_a_rename() {
    use super::Watcher;
    use std::io::Write as _;
    let scratch = scratch("inotify");
    let dir = scratch.path().join("look");
    std::fs::create_dir_all(&dir).expect("a watched dir");
    let mut watcher = Watcher::open(&dir).expect("a watcher");
    // A plain write: `close_write`.
    std::fs::write(dir.join("look.toml"), "schema = 1\n").expect("a save");
    assert!(watcher.wait_for_save().expect("no watcher error"), "a save wakes it");
    // An atomic save: temp file plus rename (`moved_to`).
    let tmp = dir.join(".look.toml.tmp.1");
    let mut file = std::fs::File::create(&tmp).expect("a temp file");
    file.write_all(b"schema = 1\n").expect("a write");
    file.sync_all().expect("a sync");
    drop(file);
    std::fs::rename(&tmp, dir.join("look.toml")).expect("a rename");
    assert!(watcher.wait_for_save().expect("no watcher error"), "a rename wakes it");
}

#[cfg(target_os = "linux")]
#[test]
fn inotify_reports_a_removed_directory() {
    use super::Watcher;
    let scratch = scratch("removed");
    let dir = scratch.path().join("look");
    std::fs::create_dir_all(&dir).expect("a watched dir");
    let mut watcher = Watcher::open(&dir).expect("a watcher");
    std::fs::remove_dir_all(&dir).expect("the dir goes away");
    assert!(
        !watcher.wait_for_save().expect("no watcher error"),
        "a removed directory ends the watch"
    );
}
