//! `--follow-portal` tests: the unit files, the install/switch/remove
//! lifecycle against a recording manager, and the run itself against a
//! scripted dconf driving the real toolkit step. Nothing here touches a
//! real systemd or dconf.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use scoot_look::schema::Polarity;
use scoot_look::state::Record;
use scoot_look::{Origin, ResolvedLook};

use super::portal::{Outcome, Session, dconf_db, follow, parse_setting};
use super::systemctl::{Systemctl, TimerState};
use super::unit::{
    PATH, Plan, Portal, SERVICE, Schedule, Side, TIMER, Trigger, path_text, read_service,
    read_watch,
};
use super::{install, remove, status};
use crate::look::toolkit::{
    COLOR_SCHEME_KEY, Dconf, PREFER_DARK, PREFER_LIGHT, apply_toolkit, want_for_polarity,
};

fn portal() -> Portal {
    Portal::parse("dawn", "moonrise").expect("valid")
}

fn plan() -> Plan {
    Plan {
        trigger: Trigger::Portal(portal()),
        exe: PathBuf::from("/usr/bin/scoot"),
        state: None,
        watch: Some(PathBuf::from("/home/me/.config/dconf/user")),
    }
}

fn schedule_plan() -> Plan {
    Plan {
        trigger: Trigger::Schedule(
            Schedule::parse("dawn", "moonrise", "07:00/19:30").expect("valid"),
        ),
        exe: PathBuf::from("/usr/bin/scoot"),
        state: None,
        watch: None,
    }
}

// ---- the setting ----

#[test]
fn the_setting_reads_as_the_portal_would() {
    assert_eq!(parse_setting("'prefer-dark'"), Some(Side::Dark));
    assert_eq!(parse_setting("'prefer-light'"), Some(Side::Light));
    // What GNOME's own Light switch writes.
    assert_eq!(parse_setting("'default'"), Some(Side::Light));
    assert_eq!(parse_setting(" 'prefer-dark'\n"), Some(Side::Dark));
    for none in [
        "",
        "prefer-dark",
        "'prefer-darker'",
        "''",
        "1",
        "'PREFER-DARK'",
    ] {
        assert_eq!(parse_setting(none), None, "`{none}`");
    }
}

#[test]
fn the_database_is_the_default_profile_under_the_config_home() {
    assert_eq!(
        dconf_db(Path::new("/home/me/.config")),
        PathBuf::from("/home/me/.config/dconf/user")
    );
}

// ---- unit files ----

#[test]
fn units_match_the_golden_files() {
    assert_eq!(
        plan().service().unwrap(),
        include_str!("golden/service-portal.golden")
    );
    assert_eq!(
        plan().trigger_file().unwrap(),
        (PATH, include_str!("golden/path.golden").to_owned())
    );
}

#[test]
fn the_path_unit_says_only_what_it_means() {
    let text = plan().trigger_file().unwrap().1;
    assert!(text.contains("PathChanged=/home/me/.config/dconf/user\n"));
    // `PathExists=` would fire for as long as the file exists, and
    // `MakeDirectory=` would create the database path as a directory.
    for banned in [
        "PathExists",
        "PathModified",
        "MakeDirectory",
        "DirectoryNotEmpty",
    ] {
        assert!(!text.contains(banned), "{banned}");
    }
    assert!(text.contains("WantedBy=paths.target\n"));
    // Only the portal service lifts the start limit.
    assert!(
        plan()
            .service()
            .unwrap()
            .contains("StartLimitIntervalSec=0\n")
    );
    assert!(!schedule_plan().service().unwrap().contains("StartLimit"));
}

#[test]
fn a_path_with_a_percent_is_doubled_and_read_back() {
    let watch = Path::new("/home/a b/c%h/.config/dconf/user");
    let text = path_text(watch).unwrap();
    assert!(text.contains("PathChanged=/home/a b/c%%h/.config/dconf/user\n"));
    assert_eq!(read_watch(&text).as_deref(), Some(watch));
}

#[test]
fn a_hostile_watch_path_cannot_add_a_line() {
    for bad in [
        "/a\nExecStart=/bin/sh",
        "/a\r/b",
        "/a\u{7}b",
        "relative/dconf",
        "",
    ] {
        assert!(path_text(Path::new(bad)).is_err(), "{bad:?}");
    }
    use std::os::unix::ffi::OsStrExt;
    let raw = Path::new(std::ffi::OsStr::from_bytes(b"/a\xffb"));
    assert!(path_text(raw).is_err());
    let missing = Plan {
        watch: None,
        ..plan()
    };
    assert!(missing.trigger_file().is_err());
}

#[test]
fn the_path_reader_trusts_only_one_changed_line() {
    let good = plan().trigger_file().unwrap().1;
    assert!(read_watch(&good).is_some());
    assert_eq!(read_watch("[Path]\nPathChanged=/x\n"), None, "no marker");
    let twice = good.replace("Unit=", "PathChanged=/other\nUnit=");
    assert_eq!(read_watch(&twice), None);
    assert_eq!(
        read_watch(&good.replace("PathChanged", "PathModified")),
        None
    );
}

#[test]
fn the_service_reader_takes_both_shapes_and_nothing_between() {
    let text = plan().service().unwrap();
    let back = read_service(&text).unwrap();
    assert_eq!(
        back,
        Plan {
            watch: None,
            ..plan()
        }
    );
    assert_eq!(
        read_service(&schedule_plan().service().unwrap()).unwrap(),
        schedule_plan()
    );
    for edit in [
        ("--follow-portal", "--follow-portal --schedule 07:00/19:00"),
        ("--follow-portal", "--schedule"),
        ("--follow-portal", "--follow-portal=1"),
        ("--follow-portal", ""),
        ("--dark moonrise", "--dark dawn"),
        ("--dark moonrise", "--dark ../x"),
    ] {
        assert!(
            read_service(&text.replace(edit.0, edit.1)).is_err(),
            "edit `{}` must be refused",
            edit.1
        );
    }
}

// ---- lifecycle ----

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    fn log(&self, call: String) {
        self.calls.borrow_mut().push(call);
    }
}

impl Systemctl for Fake {
    fn available(&self) -> Result<(), String> {
        self.log("available".into());
        Ok(())
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.log("reload".into());
        Ok(())
    }
    fn enable_now(&self, unit: &str) -> Result<(), String> {
        self.log(format!("enable {unit}"));
        Ok(())
    }
    fn restart(&self, unit: &str) -> Result<(), String> {
        self.log(format!("restart {unit}"));
        Ok(())
    }
    fn disable_now(&self, unit: &str) -> Result<(), String> {
        self.log(format!("disable {unit}"));
        Ok(())
    }
    fn timer_state(&self, unit: &str) -> Result<TimerState, String> {
        self.log(format!("state {unit}"));
        Ok(TimerState {
            active: "active".into(),
            enabled: "enabled".into(),
            next: String::new(),
        })
    }
}

fn unit_dir() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("systemd/user");
    (root, dir)
}

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

#[test]
fn install_writes_service_and_path_and_enables_the_path() {
    let (_root, dir) = unit_dir();
    let manager = Fake::default();
    let done = install(&dir, &plan(), &manager).unwrap();
    assert_eq!(done.wrote, vec![SERVICE, PATH]);
    assert_eq!(done.replaced, None);
    assert_eq!(names(&dir), vec![PATH.to_owned(), SERVICE.to_owned()]);
    assert_eq!(
        manager.calls(),
        [
            "available",
            "reload",
            "enable scoot-look-auto.path",
            "restart scoot-look-auto.path"
        ]
    );
    // A re-run changes nothing and only checks the enable.
    let again = Fake::default();
    assert!(install(&dir, &plan(), &again).unwrap().wrote.is_empty());
    assert_eq!(again.calls(), ["available", "enable scoot-look-auto.path"]);
}

#[test]
fn a_moved_config_home_rewrites_the_path_unit() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let moved = Plan {
        watch: Some(PathBuf::from("/elsewhere/dconf/user")),
        ..plan()
    };
    let done = install(&dir, &moved, &Fake::default()).unwrap();
    assert_eq!(done.wrote, vec![PATH]);
}

#[test]
fn switching_modes_retires_the_other_trigger_first() {
    let (_root, dir) = unit_dir();
    install(&dir, &schedule_plan(), &Fake::default()).unwrap();
    let manager = Fake::default();
    let done = install(&dir, &plan(), &manager).unwrap();
    assert_eq!(done.replaced, Some(TIMER));
    assert_eq!(names(&dir), vec![PATH.to_owned(), SERVICE.to_owned()]);
    assert_eq!(
        manager.calls(),
        [
            "available",
            "disable scoot-look-auto.timer",
            "reload",
            "enable scoot-look-auto.path",
            "restart scoot-look-auto.path"
        ]
    );
    // And back.
    let back = Fake::default();
    let done = install(&dir, &schedule_plan(), &back).unwrap();
    assert_eq!(done.replaced, Some(PATH));
    assert_eq!(names(&dir), vec![SERVICE.to_owned(), TIMER.to_owned()]);
    assert!(
        std::fs::read_to_string(dir.join(SERVICE))
            .unwrap()
            .contains("--schedule 07:00/19:30")
    );
}

#[test]
fn a_foreign_file_in_the_other_slot_is_not_touched_or_a_conflict() {
    let (_root, dir) = unit_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(TIMER), "[Timer]\nOnCalendar=daily\n").unwrap();
    let manager = Fake::default();
    let done = install(&dir, &plan(), &manager).unwrap();
    assert_eq!(done.replaced, None);
    assert_eq!(
        std::fs::read_to_string(dir.join(TIMER)).unwrap(),
        "[Timer]\nOnCalendar=daily\n"
    );
    assert!(!manager.calls().iter().any(|c| c.contains("timer")));
    // A foreign file in a slot we would write still refuses.
    let (_root2, dir2) = unit_dir();
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(dir2.join(PATH), "[Path]\nPathExists=/x\n").unwrap();
    assert!(install(&dir2, &plan(), &Fake::default()).is_err());
    assert!(!dir2.join(SERVICE).exists(), "nothing written");
}

#[test]
fn off_removes_all_three_and_only_ours() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    // A stray timer of ours from a hand-copy: still ours, still removed.
    std::fs::write(
        dir.join(TIMER),
        super::unit::timer_text(&Schedule::parse("dawn", "moonrise", "07:00/19:30").unwrap()),
    )
    .unwrap();
    std::fs::write(dir.join("scootbar.service"), "[Service]\n").unwrap();
    let manager = Fake::default();
    let out = remove(&dir, &manager).unwrap();
    assert_eq!(out.removed, vec![TIMER, PATH, SERVICE]);
    assert_eq!(names(&dir), vec!["scootbar.service".to_owned()]);
    assert_eq!(
        manager.calls(),
        [
            "available",
            "disable scoot-look-auto.timer",
            "disable scoot-look-auto.path",
            "reload"
        ]
    );
}

#[test]
fn status_reads_the_portal_back_with_its_watched_file() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let found = status(&dir, &Fake::default()).unwrap();
    assert_eq!(found.plan, Some(Ok(plan())));
    assert!(!found.partial);
    let (unit, state) = found.trigger.unwrap();
    assert_eq!(unit, PATH);
    assert_eq!(state.unwrap().active, "active");
}

#[test]
fn status_calls_an_inconsistent_install_partial() {
    // The service says portal, only the timer is there.
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    std::fs::remove_file(dir.join(PATH)).unwrap();
    std::fs::write(
        dir.join(TIMER),
        super::unit::timer_text(&Schedule::parse("dawn", "moonrise", "07:00/19:30").unwrap()),
    )
    .unwrap();
    let found = status(&dir, &Fake::default()).unwrap();
    assert!(found.partial);
    assert!(found.trigger.is_none());
    // Both triggers.
    install(&dir, &plan(), &Fake::default()).unwrap();
    assert!(!status(&dir, &Fake::default()).unwrap().partial);
    std::fs::write(
        dir.join(TIMER),
        super::unit::timer_text(&Schedule::parse("dawn", "moonrise", "07:00/19:30").unwrap()),
    )
    .unwrap();
    assert!(status(&dir, &Fake::default()).unwrap().partial);
    // A path unit with no service.
    std::fs::remove_file(dir.join(TIMER)).unwrap();
    std::fs::remove_file(dir.join(SERVICE)).unwrap();
    let found = status(&dir, &Fake::default()).unwrap();
    assert!(found.partial);
    assert!(found.plan.unwrap().is_err());
}

// ---- the run ----

/// A dconf database whose every write counts as a change event, even one
/// that writes the value already there: dconf may or may not notice, and
/// the loop guard must not depend on it.
#[derive(Default)]
struct Db {
    live: RefCell<HashMap<String, String>>,
    writes: Cell<u32>,
    missing: bool,
    unreadable: bool,
}

impl Db {
    fn set_scheme(&self, value: &str) {
        self.live
            .borrow_mut()
            .insert(COLOR_SCHEME_KEY.to_owned(), value.to_owned());
    }
    fn scheme(&self) -> Option<String> {
        self.live.borrow().get(COLOR_SCHEME_KEY).cloned()
    }
}

impl Dconf for Db {
    fn available(&self) -> bool {
        !self.missing
    }
    fn read(&self, key: &str) -> Result<Option<String>, String> {
        if self.unreadable {
            return Err("dconf is broken".to_owned());
        }
        Ok(self.live.borrow().get(key).cloned())
    }
    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        self.live
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }
    fn reset(&self, key: &str) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        self.live.borrow_mut().remove(key);
        Ok(())
    }
}

fn look_toml(name: &str, polarity: &str) -> String {
    format!(
        r##"
schema = 1
name = "{name}"
title = "Test {name}"
description = "A test look."
polarity = "{polarity}"
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

fn looks() -> BTreeMap<String, ResolvedLook> {
    let mut raws = BTreeMap::new();
    for (name, polarity) in [
        ("dawn", "light"),
        ("moonrise", "dark"),
        ("midnight", "dark"),
        ("noon", "light"),
    ] {
        let raw = scoot_look::parse_look_toml(look_toml(name, polarity).as_bytes()).unwrap();
        raws.insert(name.to_owned(), (raw, Origin::Shipped));
    }
    raws.keys()
        .map(|name| (name.clone(), scoot_look::resolve(&raws, name).unwrap()))
        .collect()
}

/// The world a run sees: the recorded state, the catalog, and `apply`
/// doing what `look apply` does to dconf (the real toolkit step on a real
/// record). `lookups` counts catalog reads, and `after_apply` lets a test
/// flip the setting mid-swap.
type Hook<'a> = Box<dyn FnMut(&Db) + 'a>;

struct World<'a> {
    db: &'a Db,
    record: Option<Record>,
    looks: BTreeMap<String, ResolvedLook>,
    swaps: Vec<String>,
    lookups: u32,
    broken: Vec<&'static str>,
    after_apply: Option<Hook<'a>>,
}

fn state_dir() -> PathBuf {
    // Not a private state (the leaf must not start with `scoot-look-`).
    std::env::temp_dir().join(format!("scoot-portal-state-{}", std::process::id()))
}

impl<'a> World<'a> {
    fn new(db: &'a Db) -> Self {
        World {
            db,
            record: None,
            looks: looks(),
            swaps: Vec::new(),
            lookups: 0,
            broken: Vec::new(),
            after_apply: None,
        }
    }

    /// `scoot look apply NAME` by hand, or by the service.
    fn apply_look(&mut self, name: &str) {
        let look = self.looks.get(name).expect("a known look").clone();
        let mut record = self.record.take().unwrap_or(Record {
            name: String::new(),
            rendered: String::new(),
            generation: 0,
            chosen_by: None,
            declared_default: None,
            toolkit: None,
            dconf_wrote: BTreeMap::new(),
            dconf_orig: BTreeMap::new(),
        });
        record.name = name.to_owned();
        record.generation += 1;
        apply_toolkit(&state_dir(), &mut record, &look, self.db);
        self.record = Some(record);
        if let Some(hook) = self.after_apply.as_mut() {
            hook(self.db);
        }
    }

    fn polarity(&mut self, name: &str) -> Option<Polarity> {
        self.lookups += 1;
        if self.broken.contains(&name) {
            return None;
        }
        self.looks.get(name).map(|l| l.polarity)
    }
}

impl Session for World<'_> {
    fn active_polarity(&mut self) -> Option<Polarity> {
        let name = self.record.as_ref()?.name.clone();
        self.polarity(&name)
    }
    fn polarity_of(&mut self, look: &str) -> Option<Polarity> {
        self.polarity(look)
    }
    fn apply(&mut self, look: &str) -> Result<(), String> {
        self.swaps.push(look.to_owned());
        self.apply_look(look);
        Ok(())
    }
}

/// Runs the service the way the path unit would, as the worst case: every
/// write a run makes, and the external change that started it, fires the
/// service once more. Returns how many times it ran.
fn settle(world: &mut World, db: &Db, portal: &Portal) -> u32 {
    let mut runs = 0;
    let mut pending = true;
    while pending {
        assert!(runs < 8, "the service keeps firing: a loop");
        let before = db.writes.get();
        follow(portal, db, world).expect("a run");
        runs += 1;
        pending = db.writes.get() != before;
    }
    runs
}

#[test]
fn one_external_flip_causes_exactly_one_swap() {
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    assert_eq!(db.scheme().as_deref(), Some(PREFER_DARK));
    world.swaps.clear();
    // The settings app flips to light.
    db.set_scheme(PREFER_LIGHT);
    let runs = settle(&mut world, &db, &portal());
    assert_eq!(world.swaps, ["dawn"], "exactly one swap");
    assert!(runs <= 2, "{runs} runs");
    assert_eq!(db.scheme().as_deref(), Some(PREFER_LIGHT));
    // And back to dark: one more.
    db.set_scheme(PREFER_DARK);
    settle(&mut world, &db, &portal());
    assert_eq!(world.swaps, ["dawn", "moonrise"]);
}

#[test]
fn the_gnome_light_value_swaps_once_and_stays_put() {
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    world.swaps.clear();
    db.set_scheme("'default'");
    let runs = settle(&mut world, &db, &portal());
    assert_eq!(world.swaps, ["dawn"]);
    assert!(runs <= 3, "{runs} runs");
    // The toolkit step saw the key as set outside and left it: the user's
    // own value stands.
    assert_eq!(db.scheme().as_deref(), Some("'default'"));
    // Unrelated dconf writes later change nothing.
    for _ in 0..5 {
        follow(&portal(), &db, &mut world).unwrap();
    }
    assert_eq!(world.swaps, ["dawn"]);
}

#[test]
fn a_manual_apply_of_the_other_polarity_is_not_reverted() {
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    db.set_scheme(PREFER_LIGHT);
    settle(&mut world, &db, &portal());
    assert_eq!(world.swaps, ["dawn"]);
    // `scoot look apply moonrise` by hand, with the desktop on light. The
    // toolkit step owns the key again (the follow swap recorded it), so
    // the setting follows the look, and no later write reverts the apply.
    world.apply_look("moonrise");
    assert_eq!(db.scheme().as_deref(), Some(PREFER_DARK));
    for _ in 0..5 {
        follow(&portal(), &db, &mut world).unwrap();
    }
    assert_eq!(world.swaps, ["dawn"], "the manual choice stands");
    assert_eq!(world.record.as_ref().unwrap().name, "moonrise");
}

#[test]
fn a_look_of_the_right_polarity_that_is_not_in_the_pair_stays() {
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("midnight");
    assert_eq!(db.scheme().as_deref(), Some(PREFER_DARK));
    for _ in 0..3 {
        follow(&portal(), &db, &mut world).unwrap();
    }
    assert!(world.swaps.is_empty());
    // A flip to light goes to the pair's light look, and noon (light, not
    // in the pair) then stays against further light writes.
    db.set_scheme(PREFER_LIGHT);
    settle(&mut world, &db, &portal());
    assert_eq!(world.swaps, ["dawn"]);
    world.apply_look("noon");
    follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(world.swaps, ["dawn"]);
}

#[test]
fn nothing_to_follow_never_reads_the_catalog() {
    let db = Db::default();
    let mut world = World::new(&db);
    // Unset.
    let out = follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(out.applied, Vec::<String>::new());
    assert!(out.note.contains("not set"), "{}", out.note);
    // Garbage.
    for junk in ["'prefer-sepia'", "7", "@s ''", "[]"] {
        db.set_scheme(junk);
        let out = follow(&portal(), &db, &mut world).unwrap();
        assert!(
            out.applied.is_empty() && out.note.contains("not a setting"),
            "{junk}"
        );
    }
    assert_eq!(world.lookups, 0, "no catalog read");
    assert_eq!(db.writes.get(), 0);
}

#[test]
fn a_broken_dconf_is_a_loud_error_not_a_swap() {
    let missing = Db {
        missing: true,
        ..Db::default()
    };
    let mut world = World::new(&missing);
    let error = follow(&portal(), &missing, &mut world).unwrap_err();
    assert!(error.contains("no `dconf` on PATH"), "{error}");
    let unreadable = Db {
        unreadable: true,
        ..Db::default()
    };
    let mut world = World::new(&unreadable);
    assert!(follow(&portal(), &unreadable, &mut world).is_err());
    assert!(world.swaps.is_empty());
}

#[test]
fn no_record_yet_applies_the_matching_look() {
    let db = Db::default();
    db.set_scheme(PREFER_DARK);
    let mut world = World::new(&db);
    let out = follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(out.applied, ["moonrise"]);
    // An active look that no longer resolves counts as unknown too.
    let db = Db::default();
    db.set_scheme(PREFER_LIGHT);
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    world.broken.push("moonrise");
    db.set_scheme(PREFER_LIGHT);
    let out = follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(out.applied, ["dawn"]);
}

#[test]
fn a_pair_whose_look_changed_polarity_is_left_alone() {
    // `dark` names a look that is now light, or that is gone.
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("dawn");
    db.set_scheme(PREFER_DARK);
    let swapped = Portal::parse("dawn", "noon").unwrap();
    let out = follow(&swapped, &db, &mut world).unwrap();
    assert!(out.applied.is_empty());
    assert!(out.note.contains("`noon` is light"), "{}", out.note);
    let gone = Portal::parse("dawn", "vanished").unwrap();
    let out = follow(&gone, &db, &mut world).unwrap();
    assert!(out.applied.is_empty());
    assert!(
        out.note.contains("not a look that resolves"),
        "{}",
        out.note
    );
    assert!(world.swaps.is_empty());
}

#[test]
fn a_flip_during_the_swap_is_picked_up_by_the_run_itself() {
    // systemd drops its watch while the service runs, so this change
    // would otherwise be lost until the next write.
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    world.swaps.clear();
    db.set_scheme(PREFER_LIGHT);
    let mut flipped = false;
    world.after_apply = Some(Box::new(move |db| {
        if !flipped {
            flipped = true;
            db.set_scheme(PREFER_DARK);
        }
    }));
    let out: Outcome = follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(world.swaps, ["dawn", "moonrise"]);
    assert_eq!(out.applied, ["dawn", "moonrise"]);
}

#[test]
fn a_setting_that_never_settles_is_bounded() {
    let db = Db::default();
    let mut world = World::new(&db);
    world.apply_look("moonrise");
    world.swaps.clear();
    db.set_scheme(PREFER_LIGHT);
    world.after_apply = Some(Box::new(|db| {
        let next = if db.scheme().as_deref() == Some(PREFER_LIGHT) {
            PREFER_DARK
        } else {
            PREFER_LIGHT
        };
        db.set_scheme(next);
    }));
    let out = follow(&portal(), &db, &mut world).unwrap();
    assert_eq!(world.swaps.len(), 3, "three rounds and no more");
    assert!(out.note.contains("kept changing"), "{}", out.note);
}

#[test]
fn want_for_polarity_is_what_the_setting_reads_back() {
    // The value apply writes parses to the side of the look that wrote it.
    assert_eq!(parse_setting(want_for_polarity(true)), Some(Side::Dark));
    assert_eq!(parse_setting(want_for_polarity(false)), Some(Side::Light));
}
