//! The systemd user manager, behind a trait so the unit tests never touch
//! a real one (the same split the toolkit step uses for dconf).

use std::path::Path;
use std::time::Duration;

use super::super::bounded::run_bounded;

/// How long one `systemctl --user` call may take before it is killed.
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the manager says about the timer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimerState {
    /// `active`, `inactive`, ... (`ActiveState`).
    pub active: String,
    /// `enabled`, `disabled`, `not-found`, ... (`UnitFileState`; empty
    /// when the manager does not report one).
    pub enabled: String,
    /// The next elapse in the manager's own words (`NextElapseUSecRealtime`),
    /// empty when none is scheduled.
    pub next: String,
}

/// The calls `look auto` makes. Each returns a message to show on failure.
pub trait Systemctl {
    /// Whether a user manager answers. `Err` is the user-facing reason.
    fn available(&self) -> Result<(), String>;
    fn daemon_reload(&self) -> Result<(), String>;
    /// `enable --now`: links the timer into `timers.target.wants` and
    /// starts it.
    fn enable_now(&self, unit: &str) -> Result<(), String>;
    /// `restart`: picks up a rewritten `OnCalendar=` on a running timer.
    fn restart(&self, unit: &str) -> Result<(), String>;
    /// `disable --now`: stops the timer and removes its link.
    fn disable_now(&self, unit: &str) -> Result<(), String>;
    /// `stop`: for a timer that was never enabled (the one-shot sunrise
    /// timer lives in the runtime directory with no `[Install]`).
    fn stop(&self, unit: &str) -> Result<(), String>;
    fn timer_state(&self, unit: &str) -> Result<TimerState, String>;
}

/// The real thing: `systemctl --user` found on `PATH`.
pub struct CliSystemctl;

impl CliSystemctl {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut full = vec!["--user"];
        full.extend_from_slice(args);
        let out = run_bounded(Path::new("systemctl"), &full, TIMEOUT)
            .map_err(|error| format!("`systemctl {}`: {error}", full.join(" ")))?;
        if !out.status.success() {
            let detail = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "`systemctl {}` failed ({}): {}",
                full.join(" "),
                out.status,
                detail.trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Systemctl for CliSystemctl {
    fn available(&self) -> Result<(), String> {
        self.run(&["show-environment"]).map(drop).map_err(|error| {
            format!(
                "no systemd user manager answers ({error}): `scoot look auto` runs as a user timer and needs one; \
                 inside a container or a session started without systemd, apply looks with `scoot look apply` instead"
            )
        })
    }

    fn daemon_reload(&self) -> Result<(), String> {
        self.run(&["daemon-reload"]).map(drop)
    }

    fn enable_now(&self, unit: &str) -> Result<(), String> {
        self.run(&["enable", "--now", unit]).map(drop)
    }

    fn restart(&self, unit: &str) -> Result<(), String> {
        self.run(&["restart", unit]).map(drop)
    }

    fn disable_now(&self, unit: &str) -> Result<(), String> {
        self.run(&["disable", "--now", unit]).map(drop)
    }

    fn stop(&self, unit: &str) -> Result<(), String> {
        self.run(&["stop", unit]).map(drop)
    }

    fn timer_state(&self, unit: &str) -> Result<TimerState, String> {
        let text = self.run(&[
            "show",
            unit,
            "-p",
            "ActiveState",
            "-p",
            "UnitFileState",
            "-p",
            "NextElapseUSecRealtime",
        ])?;
        Ok(parse_show(&text))
    }
}

/// Reads `systemctl show` output (`Key=value` lines).
pub fn parse_show(text: &str) -> TimerState {
    let mut state = TimerState::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("ActiveState", v)) => state.active = v.to_owned(),
            Some(("UnitFileState", v)) => state.enabled = v.to_owned(),
            Some(("NextElapseUSecRealtime", v)) => state.next = v.to_owned(),
            _ => {}
        }
    }
    state
}
