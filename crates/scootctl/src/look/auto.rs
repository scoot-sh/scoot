//! `scoot look auto`: opt-in light/dark switching, run by a systemd user
//! unit. No daemon: a trigger starts a oneshot service that runs one hidden
//! `scoot look _auto-fire` and exits. The trigger is a timer on two
//! wall-clock boundaries (`--schedule`) or a path unit on the dconf
//! database (`--follow-portal`, see [`portal`]); exactly one is installed,
//! and installing one removes the other.
//!
//! What `auto` owns is up to three unit files in `<config>/systemd/user`
//! (`scoot-look-auto.service`, plus `.timer` or `.path`), each marked on
//! line 1. A file there that does not carry the marker (a home-manager link
//! into the store, a unit someone wrote) is never overwritten or removed.
//! `auto --off` disables the trigger, then removes exactly the files that
//! carry the marker. The unit files are the only record of the settings:
//! `auto` with no flags reads them back.
//!
//! One service for both boundaries, not one per look: after a suspend or
//! shutdown that spans both, `Persistent=true` fires every missed timer
//! with no order promised, and two independent `look apply` units would
//! race to a coin-flip final look. The one service reads the wall clock
//! when it runs and applies the look due at that minute, so any number of
//! catch-up runs agree. The service's command line is fixed: this binary,
//! the hidden verb, and words that passed the look-name and `HH:MM`
//! grammars. Nothing a look supplies reaches it.
//!
//! `--sun` is filed in `docs/backlog/core/` and refuses here by name.

pub mod portal;
mod systemctl;
pub mod unit;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::state::{RecordRead, read_capped_follow, read_record, write_atomic};

use super::bounded::run_bounded;
use super::toolkit::{self, CliDconf, Dconf};
use super::{Look, apply, cycle, discover, output, sync};
use crate::cli::Error;
use scoot_look::schema::Polarity;
use systemctl::{CliSystemctl, Systemctl, TimerState};
use unit::{Clock, PATH, Plan, Portal, SERVICE, Schedule, Side, TIMER, Trigger};

/// What one `scoot look auto` invocation asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// No flags: print what is installed.
    Status,
    /// `--off`: remove everything `auto` created.
    Off,
    /// `--light A --dark B` with `--schedule L/D` or `--follow-portal`:
    /// install (or refresh) the trigger.
    On(Trigger),
    /// The hidden `_auto-fire`: apply the look due now (or the setting
    /// asks for). What the service runs.
    Fire(Trigger),
}

/// Parses the words after `auto` (or `_auto-fire`, which takes the same
/// flags except `--off`).
pub fn parse(positionals: &[String], fire: bool) -> Result<Action, Error> {
    let mut light = None;
    let mut dark = None;
    let mut schedule = None;
    let mut portal = false;
    let mut off = false;
    let mut words = positionals.iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (word.as_str(), None),
        };
        let slot = match flag {
            "--light" => &mut light,
            "--dark" => &mut dark,
            "--schedule" => &mut schedule,
            "--off" if inline.is_none() && !fire => {
                off = true;
                continue;
            }
            "--follow-portal" if inline.is_none() => {
                if portal {
                    return Err(Error::Unknown(word.clone()));
                }
                portal = true;
                continue;
            }
            "--sun" => {
                return Err(Error::Invalid {
                    what: "trigger (`scoot look auto` takes --schedule or --follow-portal; --sun is not available yet)",
                    value: word.clone(),
                });
            }
            _ => return Err(Error::Unknown(word.clone())),
        };
        let value = match inline {
            Some(value) => value,
            None => words.next().map(String::as_str).ok_or(Error::Missing(
                "a value after --light, --dark or --schedule",
            ))?,
        };
        if slot.replace(value.to_owned()).is_some() {
            return Err(Error::Unknown(word.clone()));
        }
    }
    let given = light.is_some() || dark.is_some() || schedule.is_some() || portal;
    if off && given {
        return Err(Error::Invalid {
            what: "--off (it takes no other flag)",
            value: positionals.join(" "),
        });
    }
    if off {
        return Ok(Action::Off);
    }
    if !given && !fire {
        return Ok(Action::Status);
    }
    let light = light.ok_or(Error::Missing("--light NAME"))?;
    let dark = dark.ok_or(Error::Missing("--dark NAME"))?;
    let trigger = match (schedule, portal) {
        (Some(_), true) => {
            return Err(Error::Invalid {
                what: "--schedule and --follow-portal (one trigger at a time)",
                value: positionals.join(" "),
            });
        }
        (Some(schedule), false) => Trigger::Schedule(Schedule::parse(&light, &dark, &schedule)?),
        (None, true) => Trigger::Portal(Portal::parse(&light, &dark)?),
        (None, false) => {
            return Err(Error::Missing(
                "--schedule LIGHT_FROM/DARK_FROM or --follow-portal",
            ));
        }
    };
    Ok(if fire {
        Action::Fire(trigger)
    } else {
        Action::On(trigger)
    })
}

/// How a unit slot reads: nothing there, ours (with its text), or not
/// ours. A symlink is never ours, whatever it points at: home-manager
/// links units into the store, and writing through one would try to
/// modify the store.
#[derive(Debug, PartialEq, Eq)]
enum Slot {
    Absent,
    Ours(String),
    Foreign,
}

fn inspect(path: &Path) -> Result<Slot, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Ok(Slot::Foreign),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Slot::Absent),
        Err(error) => return Err(format!("cannot look at {}: {error}", path.display())),
    }
    match read_capped_follow(path) {
        Ok(text) if unit::is_ours(&text) => Ok(Slot::Ours(text)),
        Ok(_) => Ok(Slot::Foreign),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn foreign(path: &Path) -> String {
    format!(
        "{} exists and was not written by `scoot look auto` (a home-manager link or a unit of your own): \
         move it aside, then run `scoot look auto` again",
        path.display()
    )
}

/// The config home systemd reads user units from: `$XDG_CONFIG_HOME`
/// when it is absolute (the XDG spec says to ignore a relative one, and
/// the manager does), else `~/.config`.
pub fn config_home_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let xdg = xdg.filter(|dir| Path::new(dir).is_absolute());
    scoot_look::catalog::config_home(xdg, home)
}

fn user_config_home() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME");
    config_home_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        home.as_deref(),
    )
    .ok_or_else(|| "no home resolves: set XDG_CONFIG_HOME or HOME".to_owned())
}

/// The user unit directory under a config home.
pub fn unit_dir(config_home: &Path) -> PathBuf {
    config_home.join("systemd/user")
}

/// What an install did.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    /// The unit files rewritten (empty on a byte-identical re-run).
    pub wrote: Vec<&'static str>,
    /// The other mode's trigger, removed because this one replaces it.
    pub replaced: Option<&'static str>,
}

/// Writes the pair and starts the trigger. Order: ask the manager first
/// (no manager, nothing written), refuse a foreign file in either slot we
/// would write before writing any, retire the other mode's trigger, write
/// the service then the trigger, reload, enable, and restart only when
/// something changed (so a re-run with the same arguments is a no-op the
/// manager never sees).
pub fn install(dir: &Path, plan: &Plan, manager: &dyn Systemctl) -> Result<Installed, String> {
    manager.available()?;
    let (trigger_name, text) = plan.trigger_file()?;
    let files = [(SERVICE, plan.service()?), (trigger_name, text)];
    let mut stale = Vec::new();
    for (name, text) in &files {
        match inspect(&dir.join(name))? {
            Slot::Foreign => return Err(foreign(&dir.join(name))),
            Slot::Ours(existing) if existing == *text => {}
            Slot::Ours(_) | Slot::Absent => stale.push(*name),
        }
    }
    // The other mode's trigger would keep starting the service with the
    // wrong expectations. Ours: disable it, then delete it. A foreign file
    // in that slot is not ours to touch and is not a conflict either.
    let (_, other) = plan.trigger.unit();
    let replaced = match inspect(&dir.join(other))? {
        Slot::Ours(_) => {
            manager.disable_now(other)?;
            remove_file(&dir.join(other))?;
            Some(other)
        }
        Slot::Absent | Slot::Foreign => None,
    };
    if !stale.is_empty() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    for (name, text) in &files {
        if stale.contains(name) {
            write_atomic(dir, name, text.as_bytes())?;
        }
    }
    if !stale.is_empty() || replaced.is_some() {
        manager.daemon_reload()?;
    }
    manager.enable_now(trigger_name)?;
    if !stale.is_empty() {
        manager.restart(trigger_name)?;
    }
    Ok(Installed {
        wrote: stale,
        replaced,
    })
}

fn remove_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

/// What `--off` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub removed: Vec<&'static str>,
    /// Files left because they are not ours.
    pub left: Vec<PathBuf>,
    /// The manager was not asked (none answers); files only.
    pub manager_skipped: bool,
}

/// Disables the triggers, then deletes the marked files, then reloads, in
/// that order (a deleted file would leave a dangling `*.target.wants`
/// link). Nothing installed is success with nothing removed. With no
/// answering manager the files still go: there is nothing running to
/// stop, and a leftover file would start a trigger the next time a manager
/// appears.
pub fn remove(dir: &Path, manager: &dyn Systemctl) -> Result<Removed, String> {
    let mut slots = Vec::new();
    for name in [TIMER, PATH, SERVICE] {
        slots.push((name, inspect(&dir.join(name))?));
    }
    let mut out = Removed::default();
    let ours: Vec<&'static str> = slots
        .iter()
        .filter(|(_, slot)| matches!(slot, Slot::Ours(_)))
        .map(|(name, _)| *name)
        .collect();
    out.left = slots
        .iter()
        .filter(|(_, slot)| *slot == Slot::Foreign)
        .map(|(name, _)| dir.join(name))
        .collect();
    if ours.is_empty() {
        return Ok(out);
    }
    let live = manager.available().is_ok();
    if live {
        for trigger in [TIMER, PATH] {
            if ours.contains(&trigger) {
                manager.disable_now(trigger)?;
            }
        }
    } else {
        out.manager_skipped = true;
    }
    for name in ours {
        remove_file(&dir.join(name))?;
        out.removed.push(name);
    }
    if live {
        manager.daemon_reload()?;
    }
    Ok(out)
}

/// What `auto` with no flags found.
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    /// `None`: no file of ours. `Some(Err)`: ours but unreadable.
    pub plan: Option<Result<Plan, String>>,
    /// The files of ours do not make one consistent install: a trigger
    /// without its service or the reverse, both triggers, or the trigger
    /// the service was written for missing.
    pub partial: bool,
    /// Foreign files in our slots.
    pub foreign: Vec<PathBuf>,
    /// The trigger unit of ours and what the manager says about it.
    pub trigger: Option<(&'static str, Result<TimerState, String>)>,
}

pub fn status(dir: &Path, manager: &dyn Systemctl) -> Result<Status, String> {
    let service = inspect(&dir.join(SERVICE))?;
    let timer = inspect(&dir.join(TIMER))?;
    let path = inspect(&dir.join(PATH))?;
    let foreign: Vec<PathBuf> = [(SERVICE, &service), (TIMER, &timer), (PATH, &path)]
        .iter()
        .filter(|(_, slot)| **slot == Slot::Foreign)
        .map(|(name, _)| dir.join(name))
        .collect();
    let timer_ours = matches!(timer, Slot::Ours(_));
    let path_ours = matches!(path, Slot::Ours(_));
    let mut plan = match &service {
        Slot::Ours(text) => Some(unit::read_service(text)),
        _ if timer_ours || path_ours => Some(Err("the service file is missing".to_owned())),
        _ => None,
    };
    // The unit the install is about: what the service was written for,
    // else whichever trigger is there.
    let expected = match &plan {
        Some(Ok(plan)) => Some(plan.trigger.unit().0),
        _ if timer_ours && !path_ours => Some(TIMER),
        _ if path_ours && !timer_ours => Some(PATH),
        _ => None,
    };
    if let (Some(Ok(plan)), Slot::Ours(text)) = (&mut plan, &path) {
        plan.watch = unit::read_watch(text);
    }
    let partial = match (&service, expected) {
        (Slot::Ours(_), Some(unit)) => {
            let have = |name: &str| (name == TIMER && timer_ours) || (name == PATH && path_ours);
            !have(unit) || (timer_ours && path_ours)
        }
        (Slot::Ours(_), None) => true,
        (_, _) => timer_ours || path_ours,
    };
    let trigger = expected
        .filter(|unit| (*unit == TIMER && timer_ours) || (*unit == PATH && path_ours))
        .map(|unit| (unit, manager.timer_state(unit)));
    Ok(Status {
        plan,
        partial,
        foreign,
        trigger,
    })
}

/// The `scoot` path the service should run. `current` is this process's
/// own executable, which under Nix is a store path that garbage collection
/// can remove after an upgrade: when a `scoot` on `PATH` resolves to the
/// same file, that name (a profile or system link that survives upgrades)
/// is used instead. Otherwise the running binary is.
pub fn stable_exe(current: &Path, path_var: Option<&OsStr>) -> PathBuf {
    let Ok(real) = std::fs::canonicalize(current) else {
        return current.to_owned();
    };
    for dir in path_var.iter().flat_map(|p| std::env::split_paths(p)) {
        if !dir.is_absolute() {
            continue;
        }
        let candidate = dir.join("scoot");
        if std::fs::canonicalize(&candidate).is_ok_and(|resolved| resolved == real) {
            return candidate;
        }
    }
    current.to_owned()
}

/// The look to apply for `schedule` at `now`, or `None` when the active
/// look already is it (a swap would only churn the reloads). A pending
/// preview never counts as active: its reverter would put the old look
/// back, so the target applies (and keeps) even when the preview is of it.
pub fn fire_target<'a>(
    schedule: &'a Schedule,
    now: Clock,
    current: Option<&str>,
    preview_pending: bool,
) -> Option<&'a str> {
    let target = schedule.look_for(schedule.due(now));
    (preview_pending || current != Some(target)).then_some(target)
}

/// The local wall-clock minute, from `date`: scootctl carries no time
/// crate, the zone is the system's (the one `OnCalendar=` uses), and this
/// runs at most twice a day.
fn local_clock() -> Result<Clock, String> {
    let out = run_bounded(Path::new("date"), &["+%H:%M"], Duration::from_secs(5))
        .map_err(|error| format!("cannot read the local time with `date`: {error}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Clock::parse(text.trim())
        .ok_or_else(|| format!("`date +%H:%M` printed `{}`, not HH:MM", text.trim()))
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Light => "light",
        Side::Dark => "dark",
    }
}

pub fn run(look: &Look, action: &Action) -> Result<(), String> {
    match action {
        Action::Status => run_status(look),
        Action::Off => run_off(look),
        Action::On(trigger) => run_on(look, trigger),
        Action::Fire(Trigger::Schedule(schedule)) => run_fire(look, schedule),
        Action::Fire(Trigger::Portal(portal)) => run_fire_portal(look, portal),
    }
}

/// The reply: lines on stdout in human mode, the document in `--json`;
/// notes go to stderr as `note:` lines, or into the document's `notes`.
fn emit(
    look: &Look,
    mut json: serde_json::Value,
    lines: &[String],
    notes: &[String],
) -> Result<(), String> {
    if look.json {
        json["notes"] = serde_json::json!(notes);
        output::print_line(&serde_json::to_string_pretty(&json).unwrap_or_default())
            .map_err(|error| error.to_string())
    } else {
        for line in lines {
            output::print_line(line).map_err(|error| error.to_string())?;
        }
        for note in notes {
            output::warn(format_args!("note: {note}"));
        }
        Ok(())
    }
}

/// The executable to bake into the unit for the running binary `current`:
/// `stable_exe`'s choice, refused when that file is gone. A binary replaced
/// while running reads back as `.../scoot (deleted)`; baked into a unit it
/// would never start.
fn unit_exe(current: &Path, path_var: Option<&OsStr>) -> Result<PathBuf, String> {
    let exe = stable_exe(current, path_var);
    if !exe.exists() {
        return Err(format!(
            "the scoot binary {} no longer exists (replaced while running?): run `scoot look auto` again from the installed one",
            exe.display()
        ));
    }
    Ok(exe)
}

fn run_on(look: &Look, trigger: &Trigger) -> Result<(), String> {
    // A look that is not in the catalog refuses now, not at the first
    // trigger, when nobody is watching the journal.
    let catalog = discover();
    for name in [trigger.light(), trigger.dark()] {
        if catalog.find(name).is_none() {
            return Err(apply::unknown_look(name, &catalog));
        }
    }
    // A portal pair must be a light look and a dark look. With the sides
    // swapped each run would apply the look that writes the opposite
    // setting and undo the user's own toggle.
    if matches!(trigger, Trigger::Portal(_)) {
        let sources = catalog.sources();
        for (name, want) in [
            (trigger.light(), Polarity::Light),
            (trigger.dark(), Polarity::Dark),
        ] {
            let merged = scoot_look::resolve(&sources, name)
                .map_err(|error| format!("cannot resolve `{name}`: {error}"))?;
            if merged.polarity != want {
                return Err(format!(
                    "`{name}` is a {} look: --follow-portal needs a {} look there",
                    merged.polarity.as_str(),
                    want.as_str()
                ));
            }
        }
        if !CliDconf::new().available() {
            return Err(
                "no `dconf` on PATH: --follow-portal reads the desktop's setting with it"
                    .to_owned(),
            );
        }
    }
    let current = std::env::current_exe()
        .map_err(|error| format!("cannot find the scoot binary to put in the unit: {error}"))?;
    let exe = unit_exe(&current, std::env::var_os("PATH").as_deref())?;
    // The service runs in the user manager's environment, which need not
    // hold this shell's `SCOOT_LOOK_STATE` or `XDG_STATE_HOME`: the
    // resolved directory is written into the unit, so the trigger swaps the
    // state the sessions read.
    let state = apply::resolve_state_dir(look.state.as_deref())?;
    let state = std::path::absolute(&state)
        .map_err(|error| format!("cannot resolve the state dir {}: {error}", state.display()))?;
    let config_home = user_config_home()?;
    let plan = Plan {
        trigger: trigger.clone(),
        exe,
        state: Some(state),
        watch: matches!(trigger, Trigger::Portal(_)).then(|| portal::dconf_db(&config_home)),
    };
    let dir = unit_dir(&config_home);
    let done = install(&dir, &plan, &CliSystemctl)?;
    let mut notes = Vec::new();
    let mut lines = vec![match trigger {
        Trigger::Schedule(s) => format!(
            "auto switching on: {} from {}, {} from {} (local time)",
            s.light, s.light_from, s.dark, s.dark_from
        ),
        Trigger::Portal(p) => format!(
            "auto switching on: {} when the desktop is light, {} when it is dark",
            p.light, p.dark
        ),
    }];
    if let Some(watch) = &plan.watch {
        lines.push(format!("watching {}", watch.display()));
    }
    lines.push(if done.wrote.is_empty() {
        "units already up to date".to_owned()
    } else {
        format!("wrote {} in {}", done.wrote.join(", "), dir.display())
    });
    if let Some(gone) = done.replaced {
        lines.push(format!("removed {gone} (replaced by the new trigger)"));
    }
    if let Ok(Some(declared)) = sync::read_declared()
        && let Some(pinned) = declared.default
    {
        notes.push(format!(
            "`{pinned}` is the look your Nix configuration declares: auto switching swaps the live choice, and the declaration wins again only when you change it there"
        ));
    }
    if std::env::var_os("SCOOT_LOOKS_DIR").is_some_and(|dir| !dir.is_empty()) {
        notes.push(
            "SCOOT_LOOKS_DIR is set in this shell, but the service runs in the user manager's environment and will not see it"
                .to_owned(),
        );
    }
    let mut doc = match trigger {
        Trigger::Schedule(s) => serde_json::json!({
            "enabled": true,
            "trigger": "schedule",
            "light": s.light,
            "dark": s.dark,
            "light_from": s.light_from.to_string(),
            "dark_from": s.dark_from.to_string(),
        }),
        Trigger::Portal(p) => serde_json::json!({
            "enabled": true,
            "trigger": "portal",
            "light": p.light,
            "dark": p.dark,
            "watch": plan.watch,
        }),
    };
    doc["wrote"] = serde_json::json!(done.wrote);
    doc["replaced"] = serde_json::json!(done.replaced);
    doc["unit_dir"] = serde_json::json!(dir);
    // Apply what is due now, or the screen stays wrong until the next
    // trigger. The install already stands if this fails. Under `--json`
    // the swap runs first and its outcome rides in this document, so a
    // failed swap still leaves exactly one `auto` document on stdout.
    let swap = |look: &Look| {
        match trigger {
            Trigger::Schedule(s) => run_fire(look, s),
            Trigger::Portal(p) => run_fire_portal(look, p),
        }
        .map_err(|error| format!("the units are installed, but the look was not applied: {error}"))
    };
    if look.json {
        let swapped = swap(look);
        doc["apply_error"] = serde_json::json!(swapped.as_ref().err());
        emit(look, doc, &lines, &notes)?;
        return swapped;
    }
    emit(look, doc, &lines, &notes)?;
    swap(look)
}

fn run_off(look: &Look) -> Result<(), String> {
    let dir = unit_dir(&user_config_home()?);
    let out = remove(&dir, &CliSystemctl)?;
    let mut lines = Vec::new();
    if out.removed.is_empty() && out.left.is_empty() {
        lines.push("auto switching is off: nothing to remove".to_owned());
    } else if !out.removed.is_empty() {
        lines.push(format!(
            "auto switching off: removed {} from {}",
            out.removed.join(", "),
            dir.display()
        ));
    }
    let mut notes = Vec::new();
    if out.manager_skipped {
        notes.push("no systemd user manager answers: removed the files only".to_owned());
    }
    for path in &out.left {
        notes.push(format!("left {}: it is not ours", path.display()));
    }
    emit(
        look,
        serde_json::json!({
            "enabled": false,
            "removed": out.removed,
            "left": out.left,
            "manager_skipped": out.manager_skipped,
        }),
        &lines,
        &notes,
    )
}

fn run_status(look: &Look) -> Result<(), String> {
    let dir = unit_dir(&user_config_home()?);
    let found = status(&dir, &CliSystemctl)?;
    let mut lines = Vec::new();
    let mut notes = Vec::new();
    let mut doc = serde_json::json!({ "enabled": false });
    match &found.plan {
        None => lines.push("auto switching: off".to_owned()),
        Some(Err(why)) => {
            lines.push(format!("auto switching: damaged ({why})"));
            lines.push("run `scoot look auto --off`, then set it up again".to_owned());
            doc = serde_json::json!({ "enabled": true, "error": why });
        }
        Some(Ok(plan)) => {
            lines.push("auto switching: on".to_owned());
            lines.push(format!("light: {}", plan.trigger.light()));
            lines.push(format!("dark:  {}", plan.trigger.dark()));
            // The state the service swaps, not necessarily this shell's.
            let state = plan.state.as_deref().or(look.state.as_deref());
            let current = active_look(state).0.unwrap_or_else(|| "(none)".to_owned());
            doc = serde_json::json!({
                "enabled": true,
                "light": plan.trigger.light(),
                "dark": plan.trigger.dark(),
                "active": current,
                "exe": plan.exe,
                "exe_exists": plan.exe.exists(),
            });
            match &plan.trigger {
                Trigger::Schedule(s) => {
                    lines[1] = format!("light: {} from {}", s.light, s.light_from);
                    lines[2] = format!("dark:  {} from {}", s.dark, s.dark_from);
                    let now = local_clock().ok();
                    if let Some(now) = now {
                        let side = s.due(now);
                        lines.push(format!(
                            "due now ({now}): {} ({}); active: {current}",
                            side_word(side),
                            s.look_for(side)
                        ));
                    }
                    doc["trigger"] = serde_json::json!("schedule");
                    doc["light_from"] = serde_json::json!(s.light_from.to_string());
                    doc["dark_from"] = serde_json::json!(s.dark_from.to_string());
                    doc["due"] = serde_json::json!(now.map(|n| side_word(s.due(n))));
                }
                Trigger::Portal(_) => {
                    lines.push("follows: the desktop's light/dark setting".to_owned());
                    if let Some(watch) = &plan.watch {
                        lines.push(format!("watching: {}", watch.display()));
                    }
                    let setting = CliDconf::new()
                        .read(toolkit::COLOR_SCHEME_KEY)
                        .ok()
                        .flatten();
                    lines.push(format!(
                        "color-scheme: {}; active: {current}",
                        setting.as_deref().unwrap_or("(not set)")
                    ));
                    doc["trigger"] = serde_json::json!("portal");
                    doc["watch"] = serde_json::json!(plan.watch);
                    doc["color_scheme"] = serde_json::json!(setting);
                }
            }
            lines.push(format!("runs: {}", plan.exec_start().unwrap_or_default()));
            if !plan.exe.exists() {
                notes.push(format!(
                    "{} is gone: run `scoot look auto` with the same arguments to point the unit at the installed binary",
                    plan.exe.display()
                ));
            }
        }
    }
    if let Some((unit, state)) = &found.trigger {
        match state {
            Ok(t) => {
                let enabled = if t.enabled.is_empty() {
                    "?"
                } else {
                    &t.enabled
                };
                let mut line = format!("{unit}: {} ({enabled})", t.active);
                if *unit == TIMER {
                    let next = if t.next.is_empty() { "none" } else { &t.next };
                    line.push_str(&format!(", next: {next}"));
                }
                lines.push(line);
                if t.active == "failed" {
                    notes.push(format!(
                        "{unit} has failed (a trigger limit, or its service could not start): \
                         run `systemctl --user status {unit}`, then `scoot look auto` with the same arguments to start it again"
                    ));
                }
                doc[if *unit == TIMER { "timer" } else { "path" }] = serde_json::json!({
                    "active": t.active, "enabled": t.enabled, "next": t.next
                });
            }
            Err(why) => notes.push(format!("the trigger's state is unknown: {why}")),
        }
    }
    if found.partial {
        notes.push(
            "the unit files do not make one install (a missing service or trigger, or both a timer and a path unit): run `scoot look auto` with the same arguments again"
                .to_owned(),
        );
    }
    for path in &found.foreign {
        notes.push(format!("{} is not ours and is left alone", path.display()));
    }
    emit(look, doc, &lines, &notes)
}

/// The recorded active look's name, if any, and whether a timed preview
/// is pending in that state.
fn active_look(state: Option<&Path>) -> (Option<String>, bool) {
    let Ok(dir) = apply::resolve_state_dir(state) else {
        return (None, false);
    };
    let name = match read_record(&dir) {
        RecordRead::Present(record) => Some(record.name),
        _ => None,
    };
    let preview = matches!(cycle::read_preview(&dir), cycle::PreviewRead::Present(_));
    (name, preview)
}

/// What the service runs: apply the look due at this minute unless it is
/// already active. A failure exits non-zero, so the unit shows failed in
/// `systemctl --user status` and the journal.
fn run_fire(look: &Look, schedule: &Schedule) -> Result<(), String> {
    let now = local_clock()?;
    let (current, preview_pending) = active_look(look.state.as_deref());
    let Some(target) = fire_target(schedule, now, current.as_deref(), preview_pending) else {
        return Ok(());
    };
    super::run_apply(
        look,
        Some(target),
        false,
        false,
        false,
        false,
        false,
        None,
        false,
        None,
    )
}

/// What the path service runs: apply the look the desktop's setting asks
/// for unless the active look already has that polarity. Prints one line
/// for the journal either way.
fn run_fire_portal(look: &Look, portal: &Portal) -> Result<(), String> {
    let mut session = RealSession {
        look,
        sources: None,
    };
    let out = portal::follow(portal, &CliDconf::new(), &mut session)?;
    // Stderr, not stdout: under `auto --json` this run precedes the one
    // document on stdout. The journal keeps both streams.
    output::warn(format_args!("follow-portal: {}", out.note));
    Ok(())
}

/// The world as the service sees it: the recorded state, the catalog (read
/// only when the setting disagrees with something worth looking up), and
/// the same swap `scoot look apply NAME` does.
struct RealSession<'a> {
    look: &'a Look,
    sources: Option<std::collections::BTreeMap<String, (scoot_look::RawLook, scoot_look::Origin)>>,
}

impl RealSession<'_> {
    fn polarity(&mut self, name: &str) -> Option<Polarity> {
        let sources = self.sources.get_or_insert_with(|| discover().sources());
        scoot_look::resolve(sources, name).ok().map(|m| m.polarity)
    }
}

impl portal::Session for RealSession<'_> {
    fn active_polarity(&mut self) -> Option<Polarity> {
        let name = active_look(self.look.state.as_deref()).0?;
        self.polarity(&name)
    }

    fn polarity_of(&mut self, look: &str) -> Option<Polarity> {
        self.polarity(look)
    }

    fn apply(&mut self, look: &str) -> Result<(), String> {
        super::run_apply(
            self.look,
            Some(look),
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            None,
        )
    }
}

#[cfg(test)]
mod portal_tests;
#[cfg(test)]
mod tests;
