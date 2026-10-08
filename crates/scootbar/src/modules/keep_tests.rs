//! An unchanged `exec` keeps its child across a reload: [`Module::keeps`]
//! and the old bar handed to [`start`], through real children (`sh`,
//! `sleep`, which every test machine has). Needs the `exec` feature: real
//! commands to keep, and `Kind::Exec` to compare against.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::PollFlags;

use super::custom::{Custom, Kind, intern};
use super::exec::{Restart, Settings};
use super::harness::drive_placed;
use super::payload::Format;
use super::{Module, OutputView, Placed, Settings as Bar, Sources, Update, View, start};
use crate::action::{Action, Bindings, Trigger};
use crate::layout::Layout;
use crate::policy::{BarOverride, Override, Policy, Sections};

/// A scratch directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("scootbar-keep-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Fast restarts, so a test does not wait seconds.
const FAST: Restart = Restart {
    first: Duration::from_millis(20),
    max: Duration::from_millis(80),
    stable: Duration::from_secs(10),
};

fn settings(script: &str) -> Settings {
    Settings {
        command: vec!["sh".into(), "-c".into(), script.into()],
        format: Format::Text,
        placeholder: "ph".into(),
        restart: FAST,
        icon: None,
        show_text: true,
    }
}

fn table(id: &'static str, settings: &Settings) -> Custom {
    Custom {
        id,
        kind: Kind::Exec(settings.clone()),
    }
}

fn bar_settings(id: &'static str, settings: &Settings) -> Bar {
    Bar {
        custom: vec![table(id, settings)],
        ..Default::default()
    }
}

fn bar_settings_bound(id: &'static str, settings: &Settings, bindings: Bindings) -> Bar {
    let mut bar = bar_settings(id, settings);
    bar.bindings = vec![(id, bindings)];
    bar
}

fn layout(left: Vec<&'static str>, center: Vec<&'static str>, right: Vec<&'static str>) -> Layout {
    Layout {
        left,
        center,
        right,
        ..Layout::default()
    }
}

/// Starts the bar's modules, with no old bar: what a start-up hands
/// [`start`].
fn start_fresh(layout: &Layout, settings: &Bar) -> Vec<Placed> {
    let mut said = Vec::new();
    let placed = start(layout, settings, &mut Vec::new(), &mut |id, why| {
        said.push(format!("{id}: {why}"))
    });
    assert!(said.is_empty(), "unexpected unavailable: {said:?}");
    placed
}

/// Starts what `layout` places with the running bar's modules handed over:
/// what a reload hands [`start`].
fn reload(layout: &Layout, settings: &Bar, old: &mut Vec<Placed>) -> Vec<Placed> {
    let mut said = Vec::new();
    let placed = start(layout, settings, old, &mut |id, why| {
        said.push(format!("{id}: {why}"))
    });
    assert!(said.is_empty(), "unexpected unavailable: {said:?}");
    placed
}

fn text(placed: &Placed) -> String {
    let mut view = View::default();
    placed.module.view(&OutputView { name: None }, &mut view);
    view.text().to_owned()
}

/// The pid a test command wrote to `file` (`echo $$ > file`).
fn pid_in(file: &Path) -> u32 {
    fs::read_to_string(file)
        .expect("the command wrote its pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// Whether process `pid` is gone from the process table: a zombie is not.
fn gone(pid: u32) -> bool {
    !Path::new(&format!("/proc/{pid}")).exists()
}

/// Until `pid` leaves the process table, or the test fails.
fn until_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gone(pid) {
        assert!(
            Instant::now() < deadline,
            "pid {pid} is still in the process table ({what})"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether a process runs with exactly this command line (its arguments
/// joined by spaces).
fn running(cmdline: &str) -> bool {
    for entry in fs::read_dir("/proc").expect("proc").flatten() {
        let Ok(raw) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let text: Vec<u8> = raw.iter().map(|&b| if b == 0 { b' ' } else { b }).collect();
        if String::from_utf8_lossy(&text).trim_end() == cmdline {
            return true;
        }
    }
    false
}

/// Until a process with exactly this command line runs, or the test fails
/// (a worker the script forked may not have exec'd yet when its first line
/// shows).
fn until_running(cmdline: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running(cmdline) {
        assert!(Instant::now() < deadline, "{cmdline} is not running");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- `keeps`: who continues, who starts over ----

#[test]
fn an_exec_keeps_its_own_unchanged_table_and_nothing_else() {
    let id = intern("keep-unit").unwrap();
    let settings = settings("echo hi; sleep 331");
    let running = super::exec::start(id, &settings).expect("starts");
    assert!(running.keeps(&table(id, &settings)));
    let mut changed = settings.clone();
    changed.command = vec!["sh".into(), "-c".into(), "echo bye; sleep 331".into()];
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed command starts over"
    );
    let mut changed = settings.clone();
    changed.format = Format::Json;
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed format starts over"
    );
    let mut changed = settings.clone();
    changed.placeholder = "waiting".into();
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed placeholder starts over"
    );
    let mut changed = settings.clone();
    changed.restart = Restart::default();
    assert_ne!(changed.restart, FAST, "the restart key under test differs");
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed restart key starts over"
    );
    let mut changed = settings.clone();
    changed.icon = Some(crate::icon::Icon::Glyph('x'));
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed icon starts over"
    );
    let mut changed = settings.clone();
    changed.show_text = false;
    assert!(
        !running.keeps(&table(id, &changed)),
        "a changed show-text starts over"
    );
    #[cfg(feature = "button")]
    assert!(
        !running.keeps(&Custom {
            id,
            kind: Kind::Button(super::button::Settings::new("Go", None)),
        }),
        "another kind starts over"
    );
    #[cfg(feature = "push")]
    assert!(
        !running.keeps(&Custom {
            id,
            kind: Kind::Push(super::push::Settings {
                placeholder: "p".into(),
                icon: None,
                show_text: true,
            }),
        }),
        "another kind starts over"
    );
}

/// A module that says nothing: what the default `keeps` is.
struct Stub;

impl Module for Stub {
    fn sources<'fd>(&'fd self, _sources: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _output: &OutputView<'_>, _view: &mut View) {}
}

#[test]
fn a_module_that_says_nothing_declines_every_table() {
    let stub = Stub;
    let id = intern("keep-stub").unwrap();
    assert!(!stub.keeps(&table(id, &settings("true"))));
}

#[test]
#[cfg(feature = "button")]
fn a_button_starts_fresh() {
    let button = super::button::start(&super::button::Settings::new("Go", None));
    let id = intern("keep-button").unwrap();
    assert!(!button.keeps(&table(id, &settings("true"))));
}

#[test]
#[cfg(feature = "push")]
fn a_push_starts_fresh() {
    let push = super::push::start(&super::push::Settings {
        placeholder: "p".into(),
        icon: None,
        show_text: true,
    });
    let id = intern("keep-push").unwrap();
    assert!(!push.keeps(&table(id, &settings("true"))));
}

// ---- `start` with the old bar: keep, replace, drop, move ----

#[test]
fn an_unchanged_table_keeps_its_child() {
    let dir = Dir::new("pid");
    let pidfile = dir.file("pid");
    let id = intern("keep-pid").unwrap();
    let script = format!("echo $$ > {}; echo started; sleep 331", pidfile.display());
    let settings = settings(&script);
    let placed = layout(vec![id], vec![], vec![]);
    let mut old = start_fresh(&placed, &bar_settings(id, &settings));
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid = pid_in(&pidfile);
    let revision = old[0].revision;
    assert!(revision > 0, "the shown line bumped the revision");
    // The same table, with a binding new since the bar started: the module
    // is kept, and its bindings are refreshed.
    let mut bound = Bindings::default();
    bound.set(Trigger::Click, Action::Exec(vec!["true".into()]));
    let mut stale = old;
    let new = reload(
        &placed,
        &bar_settings_bound(id, &settings, bound.clone()),
        &mut stale,
    );
    assert!(stale.is_empty(), "the unchanged module moved over");
    assert_eq!(new.len(), 1);
    assert_eq!(text(&new[0]), "started", "its shown output moved over");
    assert_eq!(new[0].revision, revision, "its revision moved over");
    assert_eq!(new[0].bindings, bound, "its bindings were refreshed");
    assert_eq!(pid_in(&pidfile), pid, "no second child was started");
    assert!(!gone(pid), "the child is still running");
    drop(new);
    until_gone(pid, "the kept child after the bar dropped it");
}

#[test]
fn a_changed_command_format_placeholder_or_restart_key_replaces_it() {
    for tag in ["command", "format", "placeholder", "restart"] {
        let dir = Dir::new(&format!("replace-{tag}"));
        let pidfile = dir.file("pid");
        let id = intern("replace").unwrap();
        let script = format!("echo $$ > {}; echo started; sleep 332", pidfile.display());
        let settings = settings(&script);
        let placed = layout(vec![id], vec![], vec![]);
        let mut old = start_fresh(&placed, &bar_settings(id, &settings));
        drive_placed(&mut old[0], "started", |p| text(p) == "started");
        let pid = pid_in(&pidfile);
        let mut changed = settings.clone();
        // What the fresh module shows before it runs, and once its new
        // table's child has printed.
        let (now, later) = match tag {
            "command" => {
                changed.command = vec![
                    "sh".into(),
                    "-c".into(),
                    format!("echo $$ > {}; echo moved; sleep 332", pidfile.display()),
                ];
                ("ph", "moved")
            }
            "format" => {
                changed.format = Format::Json;
                ("ph", "ph")
            }
            "placeholder" => {
                changed.placeholder = "waiting".into();
                ("waiting", "started")
            }
            "restart" => {
                changed.restart = Restart::default();
                ("ph", "started")
            }
            _ => unreachable!(),
        };
        let mut stale = old;
        let mut new = reload(&placed, &bar_settings(id, &changed), &mut stale);
        assert_eq!(stale.len(), 1, "{tag}: the changed table is not kept");
        assert_eq!(new.len(), 1);
        assert_eq!(
            text(&new[0]),
            now,
            "{tag}: a fresh module shows its placeholder"
        );
        // The old child runs until its module drops.
        assert!(
            !gone(pid),
            "{tag}: the old child runs until its module drops"
        );
        drop(stale);
        until_gone(pid, "{tag}: the replaced child");
        // The new table starts its own child: a new pid writes the file.
        // The wait reads the shown text, with the pidfile as the identity
        // check only (see a_respawn_wait_on_text_survives_a_slow_first_line):
        // the child writes the pidfile before its first line.
        drive_placed(&mut new[0], "respawned", |p| {
            pid_in(&pidfile) != pid && text(p) == later
        });
        let pid2 = pid_in(&pidfile);
        assert_ne!(pid2, pid, "{tag}: a new child wrote the pidfile");
        assert_eq!(text(&new[0]), later, "{tag}: the new table's child shows");
        drop(new);
        until_gone(pid2, "{tag}: the replacement child");
    }
}

#[test]
fn a_respawn_wait_on_text_survives_a_slow_first_line() {
    let dir = Dir::new("replace-slow-line");
    let pidfile = dir.file("pid");
    let id = intern("replace-slow").unwrap();
    let script = format!("echo $$ > {}; echo started; sleep 337", pidfile.display());
    let settings = settings(&script);
    let placed = layout(vec![id], vec![], vec![]);
    let mut old = start_fresh(&placed, &bar_settings(id, &settings));
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid = pid_in(&pidfile);
    // The replacement child writes its pidfile a whole second before its
    // first line: the pidfile-to-pipe gap the CI flake fell into, widened
    // from microseconds to a certainty.
    let changed = Settings {
        command: vec![
            "sh".into(),
            "-c".into(),
            format!(
                "echo $$ > {}; sleep 1; echo moved; sleep 337",
                pidfile.display()
            ),
        ],
        ..settings.clone()
    };
    let mut stale = old;
    let mut new = reload(&placed, &bar_settings(id, &changed), &mut stale);
    assert_eq!(stale.len(), 1, "the changed table is not kept");
    drop(stale);
    until_gone(pid, "the replaced child");
    // The wait reads the shown text, with the pidfile as the identity
    // check only: the child writes the pidfile before its first line, so
    // a pidfile-only wait returns with the placeholder still shown.
    drive_placed(&mut new[0], "respawned", |p| {
        pid_in(&pidfile) != pid && text(p) == "moved"
    });
    let pid2 = pid_in(&pidfile);
    assert_ne!(pid2, pid, "a new child wrote the pidfile");
    assert_eq!(text(&new[0]), "moved", "the new table's child shows");
    drop(new);
    until_gone(pid2, "the replacement child");
}

#[test]
fn a_removed_module_is_dropped_and_its_group_killed() {
    let dir = Dir::new("remove");
    let pidfile = dir.file("pid");
    let id = intern("remove").unwrap();
    // A worker behind the leader: the group kill must take it too.
    let script = format!(
        "echo $$ > {}; sleep 333 & echo started; wait",
        pidfile.display()
    );
    let settings = settings(&script);
    let modules = bar_settings(id, &settings);
    let mut old = start_fresh(&layout(vec![id], vec![], vec![]), &modules);
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    until_running("sleep 333");
    let pid = pid_in(&pidfile);
    let mut stale = old;
    let new = reload(&layout(vec![], vec![], vec![]), &modules, &mut stale);
    assert!(new.is_empty());
    assert_eq!(
        stale.len(),
        1,
        "the removed module stays for the caller to drop"
    );
    assert!(!gone(pid), "the child runs until its module drops");
    drop(stale);
    let deadline = Instant::now() + Duration::from_secs(5);
    while running("sleep 333") {
        assert!(Instant::now() < deadline, "the worker outlived the module");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(gone(pid), "pid {pid} is still in the process table");
}

#[test]
fn a_module_moved_between_sections_keeps_running() {
    let dir = Dir::new("move-section");
    let pidfile = dir.file("pid");
    let id = intern("move-section").unwrap();
    let script = format!("echo $$ > {}; echo started; sleep 334", pidfile.display());
    let settings = settings(&script);
    let modules = bar_settings(id, &settings);
    let mut old = start_fresh(&layout(vec![id], vec![], vec![]), &modules);
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid = pid_in(&pidfile);
    let revision = old[0].revision;
    let mut stale = old;
    let new = reload(&layout(vec![], vec![], vec![id]), &modules, &mut stale);
    assert!(stale.is_empty(), "the moved module moved over");
    assert_eq!(new.len(), 1);
    assert_eq!(text(&new[0]), "started");
    assert_eq!(new[0].revision, revision);
    assert_eq!(pid_in(&pidfile), pid, "no second child was started");
    assert!(!gone(pid), "the child is still running");
    drop(new);
    until_gone(pid, "the moved child after the bar dropped it");
}

#[test]
fn a_module_moved_between_outputs_keeps_running() {
    let dir = Dir::new("move-output");
    let pidfile = dir.file("pid");
    let id = intern("move-output").unwrap();
    let script = format!("echo $$ > {}; echo started; sleep 335", pidfile.display());
    let settings = settings(&script);
    let modules = bar_settings(id, &settings);
    // Yesterday: the shared lists place it left.
    let before = Policy::default().to_start(&layout(vec![id], vec![], vec![]));
    // Today: the shared lists place nothing; DP-1's override puts it right.
    let mut policy = Policy::default();
    policy.overrides.push(Override {
        name: "DP-1".into(),
        bar: BarOverride::default(),
        font_size: None,
        modules: Some(Sections {
            left: vec![],
            center: vec![],
            right: vec![id],
        }),
    });
    let after = policy.to_start(&layout(vec![], vec![], vec![]));
    let mut old = start_fresh(&before, &modules);
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid = pid_in(&pidfile);
    let mut stale = old;
    let new = reload(&after, &modules, &mut stale);
    assert!(
        stale.is_empty(),
        "the same id in the merged layout is kept across the output move"
    );
    assert_eq!(new.len(), 1);
    assert_eq!(text(&new[0]), "started");
    assert_eq!(pid_in(&pidfile), pid, "no second child was started");
    assert!(!gone(pid), "the child is still running");
    drop(new);
    until_gone(pid, "the moved child after the bar dropped it");
}
