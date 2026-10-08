//! The images waiting for the worker thread: what to decode and for which
//! outputs, with no threads and no Wayland objects, so every ordering is a
//! unit test. `daemon::images` runs them.
//!
//! Two kinds of job:
//!
//! - A **trial**: a `set` of an image. Nothing is changed until it has
//!   decoded, so a file that is missing, not an image, too large or
//!   corrupt leaves every output as it was and the reply says why. It
//!   renders the outputs it targets as they are when it starts, so the
//!   success path needs no second decode.
//! - A **render**: an image already chosen, needed at a size nobody has
//!   drawn it at: an output plugged in or reconfigured, a new scale. The
//!   decoded source is never kept (tens of MB for nothing), so this decodes
//!   the file again. Renders of the same image merge into one job while it
//!   waits, so it is decoded once for all of them. A target another output
//!   already shows at that size is taken out before the job runs
//!   ([`Jobs::satisfy`]): it shares those pixels, and is not decoded for.
//!
//! **A trial does not start while an output it targets is about to be
//! configured** ([`Jobs::next`]'s `trial_targets`; `daemon::images`
//! decides, from `Output::coming`). Started then, it would decode only to
//! validate the file, and the `configure` would ask for it again: a `set`
//! sent as the daemon starts decoded twice (ticket 8 counted two opens).
//! Held, it decodes once, for all of them; the hold ends within a round
//! trip the daemon has already sent. Renders need no hold: they are asked
//! for only once an output is configured, and outputs configured together
//! (every compositor checked configures a session's outputs in one batch)
//! merge into one job, while one of the same size served later shares
//! the pixels ([`Jobs::satisfy`]).
//!
//! **The worker runs one job at a time, newest first.** A burst of `set`s
//! then shows the last one after one decode, not after all of them: when
//! it succeeds, every older trial it covers is superseded (`Choices`) and
//! never decoded; its reply waits, like a superseded color's, until what
//! replaced it is on screen. If it fails, the next newest runs, so
//! the newest request that *can* be shown wins. A render carries the
//! serial of its image, which is that image's request generation.
//!
//! **Bounded.** A connection has one request in flight, but a client that
//! hangs up can connect again, and the change it asked for still happens,
//! so trials are capped at [`MAX_TRIALS`] queued; one more is refused
//! (nothing changed). Renders number at most one job per chosen image,
//! with at most one target per output.

use std::sync::Arc;

use crate::outputs::OutputId;
use crate::wallpaper::Image;

#[cfg(test)]
mod tests;

/// Trials waiting at once, at most: twice the connection limit, the same
/// bound as the waiting replies (`crate::waiters`).
pub const MAX_TRIALS: usize = 2 * crate::control::MAX_CONNECTIONS;

/// One output to draw an image for, at a buffer size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub output: OutputId,
    pub dims: (u32, u32),
}

/// A trial's request: who to answer, and which outputs it chose for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial<C> {
    pub conn: C,
    /// `None` for every output.
    pub output: Option<String>,
    /// A slideshow's directory, when this trial is its first file: the
    /// show is already armed, so a file that cannot be shown starts it
    /// anyway (said in the reply) rather than changing nothing.
    pub slideshow: Option<String>,
}

#[derive(Debug)]
pub struct Job<C> {
    pub image: Arc<Image>,
    pub targets: Vec<Target>,
    /// `Some` for a trial.
    pub trial: Option<Trial<C>>,
}

impl<C> Job<C> {
    fn serial(&self) -> u64 {
        self.image.serial
    }

    fn covers(&self, serial: u64, target: Target) -> bool {
        self.serial() == serial && self.targets.contains(&target)
    }
}

/// Trials already at [`MAX_TRIALS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

#[derive(Debug)]
pub struct Jobs<C> {
    queue: Vec<Job<C>>,
    /// The job on the worker, while it is there.
    running: Option<Job<C>>,
}

impl<C> Default for Jobs<C> {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            running: None,
        }
    }
}

impl<C> Jobs<C> {
    /// Queues a trial of `image`; what it draws for is decided when it
    /// starts ([`Jobs::next`]).
    pub fn trial(&mut self, image: Arc<Image>, trial: Trial<C>) -> Result<(), Busy> {
        let trials = self.queue.iter().filter(|job| job.trial.is_some()).count();
        if trials >= MAX_TRIALS {
            return Err(Busy);
        }
        self.queue.push(Job {
            image,
            targets: Vec::new(),
            trial: Some(trial),
        });
        Ok(())
    }

    /// Asks for `image` drawn at `target`, unless a job already will.
    pub fn render(&mut self, image: &Arc<Image>, target: Target) {
        let serial = image.serial;
        let covered = self
            .running
            .iter()
            .chain(&self.queue)
            .any(|job| job.covers(serial, target));
        if covered {
            return;
        }
        // One target per output: a newer size replaces an older one.
        let waiting = self
            .queue
            .iter_mut()
            .find(|job| job.trial.is_none() && job.serial() == serial);
        match waiting {
            Some(job) => {
                job.targets.retain(|t| t.output != target.output);
                job.targets.push(target);
            }
            None => self.queue.push(Job {
                image: Arc::clone(image),
                targets: vec![target],
                trial: None,
            }),
        }
    }

    /// Removes the trials that `superseded` (their output and serial) says
    /// newer choices have made moot, calling `answer` with each one's
    /// connection and serial. Not the running one: it is answered when it
    /// lands.
    pub fn sweep(
        &mut self,
        superseded: impl Fn(Option<&str>, u64) -> bool,
        mut answer: impl FnMut(C, u64),
    ) {
        let mut index = 0;
        while index < self.queue.len() {
            let moot = self.queue.get(index).is_some_and(|job| {
                job.trial
                    .as_ref()
                    .is_some_and(|trial| superseded(trial.output.as_deref(), job.serial()))
            });
            if moot {
                let job = self.queue.remove(index);
                if let Some(trial) = job.trial {
                    answer(trial.conn, job.image.serial);
                }
            } else {
                index += 1;
            }
        }
    }

    /// Takes out of the waiting renders every target `shared` says it
    /// served from pixels an output already has (and a render left with
    /// none); returns whether any was. Trials are left alone: nothing has
    /// their image yet.
    pub fn satisfy(&mut self, mut shared: impl FnMut(&Arc<Image>, Target) -> bool) -> bool {
        let mut any = false;
        for Job { image, targets, .. } in self.queue.iter_mut().filter(|job| job.trial.is_none()) {
            targets.retain(|&target| {
                let served = shared(image, target);
                any |= served;
                !served
            });
        }
        if any {
            self.queue
                .retain(|job| job.trial.is_some() || !job.targets.is_empty());
        }
        any
    }

    /// Takes the newest job to run, if none runs now: first dropping the
    /// render targets `wanted` says are no longer wanted (and render jobs
    /// left with none). Returns the job's image and targets for the
    /// worker; the job itself stays here until [`Jobs::finished`].
    ///
    /// A trial's targets are `trial_targets`' answer, asked only when the
    /// trial is the job to run. `None` holds it back: nothing runs this
    /// time, not even an older job (the newest is the one that will be
    /// shown; running an older one first would only delay it), and it is
    /// asked again next time.
    pub fn next(
        &mut self,
        wanted: impl Fn(&Arc<Image>, Target) -> bool,
        trial_targets: impl FnOnce(&Trial<C>) -> Option<Vec<Target>>,
    ) -> Option<(Arc<Image>, Vec<Target>)> {
        if self.running.is_some() {
            return None;
        }
        for job in self.queue.iter_mut().filter(|job| job.trial.is_none()) {
            let image = Arc::clone(&job.image);
            job.targets.retain(|&target| wanted(&image, target));
        }
        self.queue
            .retain(|job| job.trial.is_some() || !job.targets.is_empty());
        let newest = (0..self.queue.len())
            .max_by_key(|&i| self.queue.get(i).map_or(0, |job| job.serial()))?;
        let job = self.queue.get_mut(newest)?;
        if let Some(trial) = &job.trial {
            job.targets = trial_targets(trial)?;
        }
        let job = self.queue.remove(newest);
        let work = (Arc::clone(&job.image), job.targets.clone());
        self.running = Some(job);
        Some(work)
    }

    /// The running job is done: it is handed back to be landed.
    pub fn finished(&mut self) -> Option<Job<C>> {
        self.running.take()
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}
