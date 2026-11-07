//! The moving half of `--sun`: the one-shot timer for the next sunrise or
//! sunset. A systemd calendar cannot say "sunset", so every run of the
//! service (the daily wake, the wake after login, and the one-shot timer
//! itself) computes the next boundary and writes
//! `scoot-look-auto-next.timer` for it, in the user manager's runtime
//! directory (`$XDG_RUNTIME_DIR/systemd/user`, a place systemd reads units
//! from and clears at logout and reboot, which is what a timer for one
//! instant should be). The timer names the instant in UTC, so neither a
//! time zone nor daylight saving enters, and starts the same service, which
//! picks its side from the clock: a boundary that passed while the machine
//! slept is made up by the next run, and any number of runs agree.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use scoot_look::state::write_atomic;
use scoot_look::sun::civil::on_calendar_utc;
use scoot_look::sun::{self, Change, Location};

use super::systemctl::Systemctl;
use super::unit::{MARKER, SERVICE};
use super::{Slot, foreign, inspect, remove_file};

/// The one-shot timer. Its file lives in [`runtime_dir`].
pub const NEXT_TIMER: &str = "scoot-look-auto-next.timer";

/// How far ahead the next boundary is looked for. The daily wake finds a
/// boundary that comes later than this, and a polar day or night has none,
/// so a longer span would only cost time.
pub const HORIZON_SECS: i64 = 36 * 3600;

/// The directory for the one-shot timer: `$XDG_RUNTIME_DIR/systemd/user`.
/// The user manager sets the variable for its services; a shell without it
/// (or with a relative one) cannot say where the manager looks.
pub fn runtime_dir(xdg_runtime: Option<&OsStr>) -> Result<PathBuf, String> {
    match xdg_runtime.map(Path::new) {
        Some(dir) if dir.is_absolute() => Ok(dir.join("systemd/user")),
        _ => Err(
            "XDG_RUNTIME_DIR is not set to an absolute path: --sun keeps its next-boundary timer there (log in through a systemd user session)"
                .to_owned(),
        ),
    }
}

/// The timer's text for the instant `at`; `None` when systemd cannot spell
/// that instant (a year past 9999 or before 0000).
pub fn next_timer_text(at: i64) -> Option<String> {
    let when = on_calendar_utc(at)?;
    Some(format!(
        "# {MARKER}. Rewritten by each run; `scoot look auto --off` removes it.\n\
         [Unit]\n\
         Description=scoot look: the next sunrise or sunset\n\
         \n\
         [Timer]\n\
         OnCalendar={when}\n\
         AccuracySec=1s\n\
         RemainAfterElapse=no\n\
         Unit={SERVICE}\n"
    ))
}

/// What [`arm`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Armed {
    /// The timer waits for this boundary. `rewrote` is false when the
    /// timer already stood for it and was left alone.
    At { change: Change, rewrote: bool },
    /// No boundary within [`HORIZON_SECS`] (a polar day or night): nothing
    /// is armed and the previous side stands.
    Idle,
}

/// Arms the timer for the first sunrise or sunset after `now`. A timer
/// already waiting for that very instant is left alone, so a run that
/// changes nothing never asks the manager to reload.
pub fn arm(
    dir: &Path,
    place: Location,
    now: i64,
    manager: &dyn Systemctl,
) -> Result<Armed, String> {
    let Some(change) = sun::next_change(place, now, HORIZON_SECS) else {
        retire(dir, manager)?;
        return Ok(Armed::Idle);
    };
    let text = next_timer_text(change.at).ok_or_else(|| {
        format!(
            "the next sunrise or sunset ({}) is not a date systemd can name",
            change.at
        )
    })?;
    let path = dir.join(NEXT_TIMER);
    match inspect(&path)? {
        Slot::Foreign => return Err(foreign(&path)),
        Slot::Ours(existing) if existing == text => {
            let waiting = manager
                .timer_state(NEXT_TIMER)
                .is_ok_and(|state| state.active == "active");
            if waiting {
                return Ok(Armed::At {
                    change,
                    rewrote: false,
                });
            }
        }
        Slot::Ours(_) | Slot::Absent => {}
    }
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    write_atomic(dir, NEXT_TIMER, text.as_bytes())?;
    manager.daemon_reload()?;
    manager.restart(NEXT_TIMER)?;
    Ok(Armed::At {
        change,
        rewrote: true,
    })
}

/// Stops the timer and deletes its file, when it is ours; a foreign file of
/// that name is left alone. `true` when something was removed. With no
/// answering manager the file still goes (nothing runs to stop, and the
/// runtime directory is cleared at logout anyway).
pub fn retire(dir: &Path, manager: &dyn Systemctl) -> Result<bool, String> {
    let path = dir.join(NEXT_TIMER);
    if !matches!(inspect(&path)?, Slot::Ours(_)) {
        return Ok(false);
    }
    let live = manager.available().is_ok();
    if live {
        manager.stop(NEXT_TIMER)?;
    }
    remove_file(&path)?;
    if live {
        manager.daemon_reload()?;
    }
    Ok(true)
}
