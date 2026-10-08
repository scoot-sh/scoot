//! The running slideshow: one directory's files in turn, on the loop's
//! timer (`crate::rotation`).
//!
//! A `set DIR --every` lists the directory once and shows its files one
//! after another: the first through the normal image path (so the reply
//! waits for it, and a file that cannot be shown refuses the whole `set`),
//! the rest here, paced by the poll timeout, with no reply to send. Each
//! step records and chooses its image exactly like a restored one
//! (`daemon::restore`): decoded on demand by the worker, saved for the next
//! start, and reported by `query` as what shows. A step that cannot be
//! drawn fails like any draw (`draw_failed` in `query`, said on stderr)
//! until the next step.
//!
//! One slideshow runs at a time: a new `set`, a `clear` or a changed
//! `apply-config` drops it. With none running the loop's timeout is what
//! it was: no extra wakeups. With one, the timeout is the remaining time
//! to the next step, so the daemon wakes at most once a minute — and needs
//! no timerfd of its own, which also keeps the poll set exactly as small
//! as it was (an extra slot would outgrow a lowered fd limit and fail the
//! whole loop; see the listener-rest test).

use std::sync::Arc;
use std::time::{Duration, Instant};

use wayland_client::QueueHandle;

use super::change::{reconcile, sweep};
use super::wayland::State;
use crate::choices::Choice;
use crate::image::render::Look;
use crate::print::warn;
use crate::protocol::RotationInfo;
use crate::transition::Spec;
use crate::wallpaper::{Image, Wallpaper};

#[cfg(test)]
mod tests;

/// The slideshow running now, if any (`State::rotation`): every listed
/// file, the next one to show, and when.
pub struct Rotation {
    dir: String,
    files: Vec<String>,
    /// The file the next step shows. Starts past the first, which the
    /// request's own image trial shows.
    index: usize,
    every_secs: u64,
    shuffle: bool,
    /// `None` for every output.
    output: Option<String>,
    look: Look,
    transition: Spec,
    /// When the next step shows.
    next: Instant,
}

impl std::fmt::Debug for Rotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rotation")
            .field("dir", &self.dir)
            .field("files", &self.files.len())
            .field("index", &self.index)
            .field("every_secs", &self.every_secs)
            .finish()
    }
}

/// Why a slideshow was refused; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// No directory there now (removed between the CLI's check and this
    /// one, or sent over the protocol directly).
    NotDirectory,
    /// Nothing to cycle through.
    Empty,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotDirectory => write!(f, "not a directory any more; nothing was changed"),
            Self::Empty => write!(f, "the directory holds no files; nothing was changed"),
        }
    }
}

impl Rotation {
    /// Starts cycling `dir` (absolute) every `every_secs` seconds, for
    /// `output` (`None` for every output), each file shown with `look`
    /// through `transition`. Lists once, orders once (shuffled with the
    /// kernel's randomness when asked); the first step is one interval
    /// from `now`. The first file is shown by the request's own image
    /// trial, not here.
    pub fn start(
        dir: &str,
        every_secs: u64,
        shuffle: bool,
        output: Option<&str>,
        look: Look,
        transition: Spec,
        now: Instant,
    ) -> Result<Self, StartError> {
        if !std::fs::metadata(dir).is_ok_and(|meta| meta.is_dir()) {
            return Err(StartError::NotDirectory);
        }
        let listed = crate::rotation::list_dir(std::path::Path::new(dir))
            .map_err(|_| StartError::NotDirectory)?;
        if listed.files.is_empty() {
            return Err(StartError::Empty);
        }
        if listed.skipped_non_utf8 > 0 {
            warn(format_args!(
                "scootbg: rotation: {} file name{} in {dir:?} is not UTF-8, which the control \
                 protocol cannot carry; left out",
                listed.skipped_non_utf8,
                if listed.skipped_non_utf8 == 1 {
                    ""
                } else {
                    "s"
                },
            ));
        }
        let files = crate::rotation::order(listed.files, shuffle, seed());
        let every_secs = debug_every_override(every_secs);
        Ok(Self {
            dir: dir.to_owned(),
            index: 1 % files.len(),
            files,
            every_secs,
            shuffle,
            output: output.map(str::to_owned),
            look,
            transition,
            next: now + Duration::from_secs(every_secs),
        })
    }

    /// How often it fires, for tests.
    #[cfg(test)]
    pub fn every(&self) -> Duration {
        Duration::from_secs(self.every_secs)
    }

    /// Whether the next step is due.
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }

    /// How long until the next step: the loop's timeout while one runs.
    pub fn remaining(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    /// For `query`: what runs now.
    pub fn info(&self) -> RotationInfo<'_> {
        RotationInfo {
            directory: &self.dir,
            every_secs: self.every_secs,
            shuffle: self.shuffle,
            files: self.files.len(),
        }
    }

    /// The first file: shown by the request's own image trial, which is
    /// what the reply waits for.
    pub fn first(&self) -> &str {
        // `start` refuses an empty listing, so there is always one.
        self.files.first().map(String::as_str).unwrap_or("")
    }
}

/// The interval tests run at: `SCOOTBG_DEBUG_ROTATION_EVERY` seconds in a
/// debug build (so the integration test advances a slideshow without
/// waiting a minute), the request's own otherwise. A release build has no
/// knob: it is compiled out, and short intervals stay refused.
fn debug_every_override(every_secs: u64) -> u64 {
    #[cfg(debug_assertions)]
    {
        if let Some(short) = std::env::var("SCOOTBG_DEBUG_ROTATION_EVERY")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|&secs| secs >= 1)
        {
            crate::print::warn(format_args!(
                "scootbg: debug: SCOOTBG_DEBUG_ROTATION_EVERY shortens the rotation to {short}s"
            ));
            return short;
        }
    }
    every_secs
}

/// A seed no other slideshow draws: random from the kernel (`getrandom(2)`,
/// like `state::saver`), mixed with the time and this process when the
/// kernel refuses.
fn seed() -> u64 {
    let mut bytes = [0u8; 8];
    if rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty()).is_ok() {
        return u64::from_ne_bytes(bytes);
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|age| age.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos ^ ((std::process::id() as u64) << 32) ^ 0x9e3779b97f4a7c15
}

/// Shows the next file when it is due, and re-arms for the interval after:
/// records and chooses it like a restored image (no waiter, no reply), and
/// reconciles what it targets. Called every turn of the loop; cheap when
/// idle (one `Instant` comparison) and a no-op without a slideshow.
pub fn drive_due(state: &mut State, qh: &QueueHandle<State>, now: Instant) {
    let Some(rotation) = state.rotation.as_mut() else {
        return;
    };
    if !rotation.due(now) || rotation.files.is_empty() {
        return;
    }
    let file = rotation.files[rotation.index].clone();
    rotation.index = (rotation.index + 1) % rotation.files.len();
    let (output, look, transition, every_secs) = (
        rotation.output.clone(),
        rotation.look,
        rotation.transition,
        rotation.every_secs,
    );
    rotation.next = now + Duration::from_secs(every_secs);
    let generation = state.waiters.next_generation();
    let choice: Choice = Some(Wallpaper::Image(Arc::new(Image {
        path: file,
        look,
        serial: generation,
        fetch: None,
    })));
    state.saved.record(output.as_deref(), &choice, generation);
    state.choices.set(output.as_deref(), choice, generation);
    let State {
        globals,
        outputs,
        choices,
        waiters,
        images,
        transitions,
        ..
    } = state;
    sweep(&mut images.jobs, choices, waiters);
    for entry in outputs.iter_mut().filter(|entry| {
        output
            .as_deref()
            .is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
    }) {
        entry.output.request_transition(transition, generation);
        reconcile(
            globals,
            choices,
            &mut images.jobs,
            transitions,
            entry,
            qh,
            now,
        );
    }
}
