//! The `[wallpaper]` config section: scoot hands it to scootbg, the
//! wallpaper daemon, by running `scootbg apply-config`.
//!
//! One command covers everything (`docs/scootbg/README.md`, the
//! `apply-config` section): at startup, and on every reload, while the
//! section exists, scoot runs `COMMAND apply-config --profile PROFILE JSON`
//! with the section as JSON; on a reload that removed it, the same with
//! `{}`; at startup without one, nothing. scootbg starts its daemon if none
//! runs, and applies the section only if it changed since the last one for
//! that profile, which is what lets a later `scootbg set` survive restarts
//! and unrelated reloads. A reload with the section unchanged is still a
//! run: it costs one short process and brings back a daemon that crashed.
//!
//! # Never waiting on it
//!
//! `apply-config` replies once scootbg's change is on screen, which takes
//! this compositor processing scootbg's commits: waiting for it on the loop
//! thread would deadlock. So it is spawned and left to the `SIGCHLD` reaper
//! (`child_reaper.rs`), which hands its exit status back here to be logged.
//! Its stdio is scoot's, as for any spawned child: its stderr (and the
//! daemon's, which it passes on) lands in scoot's log, never in a pipe scoot
//! would have to drain. Nothing here runs per frame or per input event:
//! only at startup, on a reload, on a child's exit and on one timer per
//! run.
//!
//! # One at a time
//!
//! See [`queue`]: runs never overlap (two would race, and the older section
//! could land last), only the newest waiting section runs next, a run is
//! waited on for 40 s at most, and a failed run's section stays queued and
//! is retried on the next trigger.
//!
//! # The profile
//!
//! `scoot`, or `scoot-nested` under `--nested`, so a nested session and its
//! host keep their own saved wallpaper (see scootbg's Restore section).
//! Fixed at startup.

mod queue;
mod section;

#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::time::Instant;

use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

pub use queue::Waited;
use queue::{Ended, Exit, Queue};
pub use section::{Section, WallpaperConfig, WallpaperSetting};

use super::State;

/// The profile a session on its own seat or display uses.
pub const PROFILE: &str = "scoot";
/// The profile under `--nested`, apart from its host's.
pub const PROFILE_NESTED: &str = "scoot-nested";

/// `State`'s wallpaper half.
#[derive(Debug)]
pub struct Wallpaper {
    queue: Queue,
    profile: &'static str,
    /// The section last handed over from the config (`None`: none, or the
    /// file never had a usable one): what a reload diffs against, and whose
    /// `command` sends the `{}` when a reload removes the section.
    live: Option<Section>,
    /// The timer that abandons the run in flight at its deadline.
    timer: Option<RegistrationToken>,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            queue: Queue::default(),
            profile: PROFILE,
            live: None,
            timer: None,
        }
    }
}

impl Wallpaper {
    /// Whether no `apply-config` is tracked (the reaper's fast path).
    pub fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }

    /// The queue, for the glue's tests.
    #[cfg(test)]
    pub(super) fn queue_mut(&mut self) -> &mut Queue {
        &mut self.queue
    }
}

/// What a reload did to the section, for its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reloaded {
    /// Neither the section nor `command` changed (or the section is still
    /// absent). An unchanged section still runs `apply-config`.
    Unchanged,
    /// Handed to scootbg: added, changed or removed (`values`), and/or a new
    /// `command`.
    Applied { values: bool, command: bool },
    /// The file's section has a problem; the running one is kept.
    Refused(String),
}

impl State {
    /// Startup: runs the section, if the file has a usable one. `nested`
    /// picks the profile. An invalid section is logged and runs nothing:
    /// the session starts with its background color, as with no section.
    pub(super) fn start_wallpaper(&mut self, setting: &WallpaperSetting, nested: bool) {
        self.wallpaper.profile = if nested { PROFILE_NESTED } else { PROFILE };
        match setting {
            WallpaperSetting::Absent => {}
            WallpaperSetting::Invalid(problem) => {
                tracing::error!(
                    %problem,
                    "the config file's [wallpaper] section cannot be used; no wallpaper is set \
                     from it (the rest of the file applies)"
                );
            }
            WallpaperSetting::Section(section) => {
                self.wallpaper.live = Some(section.clone());
                self.submit_wallpaper(section.clone());
            }
        }
    }

    /// A reload: re-runs the section while it exists, `{}` when it went,
    /// and retries a held one otherwise. See [`Reloaded`].
    pub(super) fn reload_wallpaper(&mut self, setting: &WallpaperSetting) -> Reloaded {
        match setting {
            WallpaperSetting::Invalid(problem) => {
                self.wallpaper.queue.trigger();
                self.drive_wallpaper();
                Reloaded::Refused(problem.clone())
            }
            WallpaperSetting::Absent => {
                let Some(live) = self.wallpaper.live.take() else {
                    self.wallpaper.queue.trigger();
                    self.drive_wallpaper();
                    return Reloaded::Unchanged;
                };
                // The removed section's own command: the binary that knows
                // the daemon (a home-manager store path, say).
                self.submit_wallpaper(Section {
                    command: live.command,
                    json: "{}".to_owned(),
                });
                Reloaded::Applied {
                    values: true,
                    command: false,
                }
            }
            WallpaperSetting::Section(section) => {
                let (values, command) = match &self.wallpaper.live {
                    None => (true, false),
                    Some(live) => (live.json != section.json, live.command != section.command),
                };
                self.wallpaper.live = Some(section.clone());
                self.submit_wallpaper(section.clone());
                if values || command {
                    Reloaded::Applied { values, command }
                } else {
                    Reloaded::Unchanged
                }
            }
        }
    }

    fn submit_wallpaper(&mut self, section: Section) {
        self.wallpaper.queue.submit(section.command, section.json);
        self.drive_wallpaper();
    }

    /// Starts the queue's next run, if it names one. A spawn that fails is
    /// held for the next trigger, loudly: the session carries on without a
    /// wallpaper.
    fn drive_wallpaper(&mut self) {
        let Some(next) = self.wallpaper.queue.next() else {
            return;
        };
        let command = next.command.clone();
        let mut child = self.session_command(&command);
        child
            .arg("apply-config")
            .arg("--profile")
            .arg(self.wallpaper.profile)
            .arg(&next.json);
        match child.spawn() {
            Ok(child) => {
                let pid = child.id();
                // Dropping a `Child` neither waits for nor kills it: the
                // reaper collects it by pid.
                drop(child);
                self.wallpaper.queue.started(pid, Instant::now());
                tracing::info!(
                    command = %Lossy(&command),
                    profile = self.wallpaper.profile,
                    pid,
                    "handing the [wallpaper] section to scootbg (apply-config)"
                );
                self.arm_wallpaper_timer();
            }
            Err(error) => {
                self.wallpaper.queue.spawn_failed();
                if error.kind() == std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        command = %Lossy(&command),
                        %error,
                        "cannot run scootbg for the [wallpaper] section: the command was not \
                         found. Install scootbg (its own package, like scootctl: the NixOS and \
                         home-manager modules install it for you; from source, `cargo install \
                         --path crates/scootbg`), or set `command` in [wallpaper] to its path. \
                         The session carries on with its background color; the next reload \
                         tries again"
                    );
                } else {
                    tracing::warn!(
                        command = %Lossy(&command),
                        %error,
                        "cannot run scootbg for the [wallpaper] section; the session carries on \
                         with its background color, and the next reload tries again"
                    );
                }
            }
        }
    }

    /// Arms (or re-arms, or clears) the one timer that abandons the run in
    /// flight at its deadline.
    fn arm_wallpaper_timer(&mut self) {
        if let Some(token) = self.wallpaper.timer.take() {
            self.loop_handle.remove(token);
        }
        let Some(deadline) = self.wallpaper.queue.deadline() else {
            return;
        };
        match self.loop_handle.insert_source(
            Timer::from_deadline(deadline),
            |_, _, state: &mut State| {
                state.wallpaper.timer = None;
                state.expire_wallpaper();
                TimeoutAction::Drop
            },
        ) {
            Ok(token) => self.wallpaper.timer = Some(token),
            // Only a loop that is shutting down refuses a timer. Without one
            // the run is waited on until it exits (every `apply-config` but
            // `{}` on a hung disk bounds itself), which is what this bound
            // improves on, not what keeps anything safe.
            Err(error) => tracing::warn!(
                %error,
                "cannot arm the [wallpaper] run's timeout; waiting for it to exit"
            ),
        }
    }

    /// The timer fired: abandons the run in flight if its deadline has
    /// passed (else re-arms: a timer is never early by design, but this does
    /// not depend on it), then starts whatever is next.
    fn expire_wallpaper(&mut self) {
        if let Some(pid) = self.wallpaper.queue.expire(Instant::now()) {
            tracing::warn!(
                pid,
                timeout_s = queue::PATIENCE.as_secs(),
                "scootbg apply-config has not finished; no longer waiting on it (it keeps \
                 running, and its exit is logged when it comes). A hung disk under the state \
                 file is the usual cause"
            );
            self.drive_wallpaper();
        } else {
            self.arm_wallpaper_timer();
        }
    }

    /// Reaps the `apply-config` runs that exited, logs how each ended, and
    /// starts what is next. Called by the reaper on every `SIGCHLD` while a
    /// run is tracked.
    pub(super) fn reap_wallpaper(&mut self) {
        let before = self.wallpaper.queue.running_pid();
        self.wallpaper
            .queue
            .reap(super::child_reaper::wait, log_end);
        if self.wallpaper.queue.running_pid() != before {
            // The run in flight ended: its timer goes with it.
            if let Some(token) = self.wallpaper.timer.take() {
                self.loop_handle.remove(token);
            }
        }
        self.drive_wallpaper();
    }
}

/// Logs how a run ended. 1 is a runtime failure (scootbg's own message is
/// above in this log), 2 a section scootbg refused.
fn log_end(ended: Ended) {
    let Ended {
        pid,
        command,
        exit,
        abandoned,
    } = ended;
    let command = Lossy(&command);
    match exit {
        Exit::Success => {
            tracing::info!(pid, %command, abandoned, "scootbg applied the [wallpaper] section")
        }
        Exit::Code(1) => tracing::warn!(
            pid,
            %command,
            abandoned,
            "scootbg apply-config failed (exit status 1: a runtime failure, which scootbg \
             describes above in this log); the section is tried again on the next reload"
        ),
        Exit::Code(2) => tracing::warn!(
            pid,
            %command,
            abandoned,
            "scootbg refused the [wallpaper] section (exit status 2): a value scoot passes \
             through unchecked (a color, mode or filter), or a scootbg that does not match this \
             scoot; scootbg names the problem above in this log"
        ),
        Exit::Code(code) => tracing::warn!(
            pid,
            %command,
            abandoned,
            code,
            "scootbg apply-config exited with an unexpected status"
        ),
        Exit::Signal(signal) => tracing::warn!(
            pid,
            %command,
            abandoned,
            signal,
            "scootbg apply-config was killed by a signal"
        ),
        Exit::Unknown => tracing::warn!(
            pid,
            %command,
            abandoned,
            "scootbg apply-config was reaped elsewhere; its exit status is unknown"
        ),
    }
}

/// An `OsStr` shown lossily in a log field.
struct Lossy<'a>(&'a OsStr);

impl std::fmt::Display for Lossy<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.to_string_lossy().fmt(f)
    }
}
