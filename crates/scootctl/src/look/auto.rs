//! `scoot look auto`: opt-in light/dark switching on a schedule, run by a
//! systemd user timer. No daemon: the timer starts a oneshot service that
//! runs one hidden `scoot look _auto-fire` and exits.
//!
//! What `auto` owns is two unit files in `<config>/systemd/user`
//! (`scoot-look-auto.timer` and `.service`), each marked on line 1. A
//! file there that does not carry the marker (a home-manager link into
//! the store, a unit someone wrote) is never overwritten or removed.
//! `auto --off` disables the timer, then removes exactly the files that
//! carry the marker. The unit files are the only record of the schedule:
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
//! Only `--schedule` exists so far. `--sun` and `--follow-portal` are
//! filed in `docs/backlog/core/` and refuse here by name.

mod systemctl;
pub mod unit;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::state::{RecordRead, read_capped_follow, read_record, write_atomic};

use super::bounded::run_bounded;
use super::{Look, apply, cycle, discover, install, output, sync};
use crate::cli::Error;
use systemctl::{CliSystemctl, Systemctl, TimerState};
use unit::{Clock, Plan, SERVICE, Schedule, Side, TIMER};

/// What one `scoot look auto` invocation asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// No flags: print what is installed.
    Status,
    /// `--off`: remove everything `auto` created.
    Off,
    /// `--light A --dark B --schedule L/D`: install (or refresh) the timer.
    On(Schedule),
    /// The hidden `_auto-fire`: apply the look due now. What the service
    /// runs.
    Fire(Schedule),
}

/// Parses the words after `auto` (or `_auto-fire`, which takes the same
/// three flags and nothing else).
pub fn parse(positionals: &[String], fire: bool) -> Result<Action, Error> {
    let mut light = None;
    let mut dark = None;
    let mut schedule = None;
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
            "--sun" | "--follow-portal" => {
                return Err(Error::Invalid {
                    what: "trigger (`scoot look auto` takes --schedule only so far; --sun and --follow-portal are not available yet)",
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
    let given = light.is_some() || dark.is_some() || schedule.is_some();
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
    let schedule = schedule.ok_or(Error::Missing("--schedule LIGHT_FROM/DARK_FROM"))?;
    let schedule = Schedule::parse(&light, &dark, &schedule)?;
    Ok(if fire {
        Action::Fire(schedule)
    } else {
        Action::On(schedule)
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

/// The user unit directory under a config home.
pub fn unit_dir(config_home: &Path) -> PathBuf {
    config_home.join("systemd/user")
}

/// What an install did.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    /// The unit files rewritten (empty on a byte-identical re-run).
    pub wrote: Vec<&'static str>,
}

/// Writes the pair and starts the timer. Order: ask the manager first
/// (no manager, nothing written), refuse a foreign file before writing
/// either, write the service then the timer, reload, enable, and restart
/// only when something changed (so a re-run with the same arguments is a
/// no-op the manager never sees).
pub fn install(dir: &Path, plan: &Plan, manager: &dyn Systemctl) -> Result<Installed, String> {
    manager.available()?;
    let files = [(SERVICE, plan.service()?), (TIMER, plan.timer())];
    let mut stale = Vec::new();
    for (name, text) in &files {
        match inspect(&dir.join(name))? {
            Slot::Foreign => return Err(foreign(&dir.join(name))),
            Slot::Ours(existing) if existing == *text => {}
            Slot::Ours(_) | Slot::Absent => stale.push(*name),
        }
    }
    if !stale.is_empty() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    for (name, text) in &files {
        if stale.contains(name) {
            write_atomic(dir, name, text.as_bytes())?;
        }
    }
    if !stale.is_empty() {
        manager.daemon_reload()?;
    }
    manager.enable_now(TIMER)?;
    if !stale.is_empty() {
        manager.restart(TIMER)?;
    }
    Ok(Installed { wrote: stale })
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

/// Disables the timer, then deletes the marked files, then reloads, in
/// that order (a deleted file would leave a dangling `timers.target.wants`
/// link). Nothing installed is success with nothing removed. With no
/// answering manager the files still go: there is nothing running to
/// stop, and a leftover file would start a timer the next time a manager
/// appears.
pub fn remove(dir: &Path, manager: &dyn Systemctl) -> Result<Removed, String> {
    let mut slots = Vec::new();
    for name in [TIMER, SERVICE] {
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
        if ours.contains(&TIMER) {
            manager.disable_now(TIMER)?;
        }
    } else {
        out.manager_skipped = true;
    }
    for name in ours {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => out.removed.push(name),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot remove {}: {error}",
                    dir.join(name).display()
                ));
            }
        }
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
    /// Only one of the two files is there (a hand-delete, a failed run).
    pub partial: bool,
    /// Foreign files in our slots.
    pub foreign: Vec<PathBuf>,
    pub timer: Option<Result<TimerState, String>>,
}

pub fn status(dir: &Path, manager: &dyn Systemctl) -> Result<Status, String> {
    let service = inspect(&dir.join(SERVICE))?;
    let timer = inspect(&dir.join(TIMER))?;
    let foreign: Vec<PathBuf> = [(SERVICE, &service), (TIMER, &timer)]
        .iter()
        .filter(|(_, slot)| **slot == Slot::Foreign)
        .map(|(name, _)| dir.join(name))
        .collect();
    let partial = matches!(
        (&service, &timer),
        (Slot::Ours(_), Slot::Absent | Slot::Foreign)
            | (Slot::Absent | Slot::Foreign, Slot::Ours(_))
    );
    let plan = match &service {
        Slot::Ours(text) => Some(unit::read_service(text)),
        _ if timer != Slot::Absent && matches!(timer, Slot::Ours(_)) => {
            Some(Err("the service file is missing".to_owned()))
        }
        _ => None,
    };
    let timer_state = matches!(timer, Slot::Ours(_)).then(|| manager.timer_state(TIMER));
    Ok(Status {
        plan,
        partial,
        foreign,
        timer: timer_state,
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
        Action::On(schedule) => run_on(look, schedule),
        Action::Fire(schedule) => run_fire(look, schedule),
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

fn run_on(look: &Look, schedule: &Schedule) -> Result<(), String> {
    // A look that is not in the catalog refuses now, not at the first
    // boundary, when nobody is watching the journal.
    let catalog = discover();
    for name in [&schedule.light, &schedule.dark] {
        if catalog.find(name).is_none() {
            return Err(apply::unknown_look(name, &catalog));
        }
    }
    let exe = std::env::current_exe()
        .map(|exe| stable_exe(&exe, std::env::var_os("PATH").as_deref()))
        .map_err(|error| format!("cannot find the scoot binary to put in the unit: {error}"))?;
    // The service runs in the user manager's environment, which need not
    // hold this shell's `SCOOT_LOOK_STATE` or `XDG_STATE_HOME`: the
    // resolved directory is written into the unit, so the timer swaps the
    // state the sessions read.
    let state = apply::resolve_state_dir(look.state.as_deref())?;
    let state = std::path::absolute(&state)
        .map_err(|error| format!("cannot resolve the state dir {}: {error}", state.display()))?;
    let plan = Plan {
        schedule: schedule.clone(),
        exe,
        state: Some(state),
    };
    let dir = unit_dir(&install::config_home_dir()?);
    let done = install(&dir, &plan, &CliSystemctl)?;
    let mut notes = Vec::new();
    let mut lines = vec![format!(
        "auto switching on: {} from {}, {} from {} (local time)",
        schedule.light, schedule.light_from, schedule.dark, schedule.dark_from
    )];
    lines.push(if done.wrote.is_empty() {
        "units already up to date".to_owned()
    } else {
        format!("wrote {} in {}", done.wrote.join(", "), dir.display())
    });
    if let Ok(Some(declared)) = sync::read_declared()
        && let Some(pinned) = declared.default
    {
        notes.push(format!(
            "`{pinned}` is the look your Nix configuration declares: the schedule swaps the live choice, and the declaration wins again only when you change it there"
        ));
    }
    if std::env::var_os("SCOOT_LOOKS_DIR").is_some_and(|dir| !dir.is_empty()) {
        notes.push(
            "SCOOT_LOOKS_DIR is set in this shell, but the timer runs in the user manager's environment and will not see it"
                .to_owned(),
        );
    }
    emit(
        look,
        serde_json::json!({
            "enabled": true,
            "light": schedule.light,
            "dark": schedule.dark,
            "light_from": schedule.light_from.to_string(),
            "dark_from": schedule.dark_from.to_string(),
            "wrote": done.wrote,
            "unit_dir": dir,
        }),
        &lines,
        &notes,
    )?;
    // Apply what is due now, or the screen stays wrong until the next
    // boundary. The install already stands if this fails.
    run_fire(look, schedule).map_err(|error| {
        format!("the timer is installed, but the look due now was not applied: {error}")
    })
}

fn run_off(look: &Look) -> Result<(), String> {
    let dir = unit_dir(&install::config_home_dir()?);
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
    let dir = unit_dir(&install::config_home_dir()?);
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
            let s = &plan.schedule;
            lines.push("auto switching: on".to_owned());
            lines.push(format!("light: {} from {}", s.light, s.light_from));
            lines.push(format!("dark:  {} from {}", s.dark, s.dark_from));
            let now = local_clock().ok();
            // The state the timer swaps, not necessarily this shell's.
            let state = plan.state.as_deref().or(look.state.as_deref());
            let current = active_look(state).0.unwrap_or_else(|| "(none)".to_owned());
            if let Some(now) = now {
                let side = s.due(now);
                lines.push(format!(
                    "due now ({now}): {} ({}); active: {current}",
                    side_word(side),
                    s.look_for(side)
                ));
            }
            lines.push(format!("runs: {}", plan.exec_start().unwrap_or_default()));
            if !plan.exe.exists() {
                notes.push(format!(
                    "{} is gone: run `scoot look auto` with the same arguments to point the timer at the installed binary",
                    plan.exe.display()
                ));
            }
            doc = serde_json::json!({
                "enabled": true,
                "light": s.light,
                "dark": s.dark,
                "light_from": s.light_from.to_string(),
                "dark_from": s.dark_from.to_string(),
                "due": now.map(|n| side_word(s.due(n))),
                "active": current,
                "exe": plan.exe,
                "exe_exists": plan.exe.exists(),
            });
        }
    }
    match &found.timer {
        Some(Ok(t)) => {
            let next = if t.next.is_empty() { "none" } else { &t.next };
            lines.push(format!(
                "timer: {} ({}), next: {next}",
                t.active,
                if t.enabled.is_empty() {
                    "?"
                } else {
                    &t.enabled
                }
            ));
            doc["timer"] = serde_json::json!({
                "active": t.active, "enabled": t.enabled, "next": t.next
            });
        }
        Some(Err(why)) => notes.push(format!("the timer's state is unknown: {why}")),
        None => {}
    }
    if found.partial {
        notes.push(
            "only one of the two unit files is present: run `scoot look auto` with the same arguments again"
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

#[cfg(test)]
mod tests;
