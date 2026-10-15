//! The third-party daemon steps of a swap: `makoctl reload`, the fcitx5
//! classic UI reload over D-Bus, and restarts of the OSD and night-light
//! units (design section 4.7, ticket `look-leaves-daemons`).
//!
//! Every step here is best-effort and reported: a missing binary, a daemon
//! that is not running, or a failed signal is a note, never a refusal.
//! The fragments these signals activate live in the rendered look (`mako`,
//! `wob.ini`, `fcitx5/theme.conf`, `nightlight`); the Nix side points each
//! app at the state link, so by the time these run the link already names
//! the new files.
//!
//! A private look state (a headless or `--private-look` session) runs none
//! of them: unlike the compositor and bar reloads, which address the
//! session's own sockets, these signals always reach the ambient session,
//! so running them from a test would move the user's real desktop.
//!
//! The commands, verified against the pinned sources:
//!
//! - `makoctl reload`: mako 1.11.0 re-parses its config (following the
//!   `include=` to the new fragment) and restyles the notifications
//!   already on screen (`dbus/mako.c`, `config.c`).
//! - `busctl --user call org.fcitx.Fcitx5 /controller
//!   org.fcitx.Fcitx.Controller1 ReloadAddonConfig s classicui`: fcitx5
//!   5.1.21 re-reads the classic UI theme (`dbusmodule.cpp`: the service
//!   is `org.fcitx.Fcitx5`, the object `/controller`). `fcitx5-remote -r`
//!   does not restyle it.
//! - `systemctl --user try-restart scoot-osd` / `scoot-nightlight`:
//!   `try-restart` restarts a running unit and no-ops when it is not
//!   running (a swap never starts what the user never started).

use std::path::Path;

use super::toolkit::find_on_path;

/// The OSD unit (wob): the name the Nix module gives it, restarted on
/// swap so the new `wob.ini` loads. wob reads its config once, at start.
pub const UNIT_OSD: &str = "scoot-osd";
/// The night-light unit: restarted on swap so the wrapper reads the new
/// temperature at start.
pub const UNIT_NIGHTLIGHT: &str = "scoot-nightlight";

/// What the leaf steps did: who reloaded, and the notes for the reply.
#[derive(Debug, Default)]
pub struct LeafReport {
    pub reloaded: Vec<String>,
    pub notes: Vec<String>,
}

/// The leaf backends, so tests script the daemons.
pub trait Leaves {
    /// `makoctl reload`.
    fn mako_reload(&self) -> Result<(), String>;
    /// The fcitx5 `ReloadAddonConfig "classicui"` D-Bus call.
    fn fcitx5_reload(&self) -> Result<(), String>;
    /// `systemctl --user try-restart <unit>`.
    fn unit_try_restart(&self, unit: &str) -> Result<(), String>;
}

/// The real backends: `makoctl`, `busctl` and `systemctl` on `PATH`.
pub struct CliLeaves {
    makoctl: Option<std::path::PathBuf>,
    busctl: Option<std::path::PathBuf>,
    systemctl: Option<std::path::PathBuf>,
}

impl CliLeaves {
    pub fn new() -> Self {
        Self {
            makoctl: find_on_path("makoctl"),
            busctl: find_on_path("busctl"),
            systemctl: find_on_path("systemctl"),
        }
    }
}

impl Default for CliLeaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Leaves for CliLeaves {
    fn mako_reload(&self) -> Result<(), String> {
        let program = self
            .makoctl
            .as_ref()
            .ok_or_else(|| "no `makoctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["reload"])?;
        if out.status.success() {
            Ok(())
        } else {
            Err("`makoctl reload` failed (mako not running?)".to_owned())
        }
    }

    fn fcitx5_reload(&self) -> Result<(), String> {
        let program = self
            .busctl
            .as_ref()
            .ok_or_else(|| "no `busctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(
            program,
            &[
                "--user",
                "call",
                "org.fcitx.Fcitx5",
                "/controller",
                "org.fcitx.Fcitx.Controller1",
                "ReloadAddonConfig",
                "s",
                "classicui",
            ],
        )?;
        if out.status.success() {
            Ok(())
        } else {
            Err("fcitx5 `ReloadAddonConfig` failed (fcitx5 not running?)".to_owned())
        }
    }

    fn unit_try_restart(&self, unit: &str) -> Result<(), String> {
        let program = self
            .systemctl
            .as_ref()
            .ok_or_else(|| "no `systemctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["--user", "try-restart", unit])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("`systemctl --user try-restart {unit}` failed"))
        }
    }
}

/// Runs the leaf steps for a swap over the rendered `merged` look.
/// Returns who reloaded and the notes. Never fails the swap.
pub fn apply_leaves(
    state_dir: &Path,
    merged: &scoot_look::ResolvedLook,
    leaves: &dyn Leaves,
) -> LeafReport {
    let mut report = LeafReport::default();
    if super::toolkit::is_private_state(state_dir) {
        report.notes.push(
            "private look state (headless or --private-look): third-party daemons left alone"
                .to_owned(),
        );
        return report;
    }
    match leaves.mako_reload() {
        Ok(()) => report.reloaded.push("mako".to_owned()),
        Err(error) => report.notes.push(format!("mako not reloaded: {error}")),
    }
    match leaves.fcitx5_reload() {
        Ok(()) => report.reloaded.push("fcitx5".to_owned()),
        Err(error) => report.notes.push(format!("fcitx5 not reloaded: {error}")),
    }
    match leaves.unit_try_restart(UNIT_OSD) {
        Ok(()) => report.reloaded.push(format!("{UNIT_OSD} unit")),
        Err(error) => report.notes.push(format!("wob not restarted: {error}")),
    }
    match merged.night_temperature {
        Some(temperature) => match leaves.unit_try_restart(UNIT_NIGHTLIGHT) {
            Ok(()) => report
                .reloaded
                .push(format!("{UNIT_NIGHTLIGHT} unit ({temperature}K)")),
            Err(error) => report.notes.push(format!(
                "nightlight not restarted (look wants {temperature}K): {error}"
            )),
        },
        None => report
            .notes
            .push("the look names no night temperature: nightlight left alone".to_owned()),
    }
    report
}

#[cfg(test)]
mod tests;

/// A scripted [`Leaves`] for tests: each backend pops its next scripted
/// result (unscripted calls succeed), recording every unit restart. Shared
/// by the leaf tests and the apply-transaction tests.
#[cfg(test)]
pub(crate) struct ScriptedLeaves {
    mako: std::cell::RefCell<Vec<Result<(), String>>>,
    fcitx5: std::cell::RefCell<Vec<Result<(), String>>>,
    units: std::cell::RefCell<Vec<Result<(), String>>>,
    restarted: std::cell::RefCell<Vec<String>>,
}

#[cfg(test)]
impl ScriptedLeaves {
    pub(crate) fn ok() -> Self {
        Self {
            mako: std::cell::RefCell::new(vec![Ok(())]),
            fcitx5: std::cell::RefCell::new(vec![Ok(())]),
            units: std::cell::RefCell::new(vec![Ok(()), Ok(())]),
            restarted: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            mako: std::cell::RefCell::new(vec![Err("no `makoctl` on PATH".to_owned())]),
            fcitx5: std::cell::RefCell::new(vec![Err("no `busctl` on PATH".to_owned())]),
            units: std::cell::RefCell::new(vec![
                Err("no `systemctl` on PATH".to_owned()),
                Err("no `systemctl` on PATH".to_owned()),
            ]),
            restarted: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn restarted(&self) -> Vec<String> {
        self.restarted.borrow().clone()
    }
}

#[cfg(test)]
impl Leaves for ScriptedLeaves {
    fn mako_reload(&self) -> Result<(), String> {
        self.mako.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn fcitx5_reload(&self) -> Result<(), String> {
        self.fcitx5.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn unit_try_restart(&self, unit: &str) -> Result<(), String> {
        self.restarted.borrow_mut().push(unit.to_owned());
        self.units.borrow_mut().pop().unwrap_or(Ok(()))
    }
}
