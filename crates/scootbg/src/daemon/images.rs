//! Images on the loop thread: starting the worker's next job, and landing
//! its result. The queue's rules are `crate::jobs`'s; the decoding is
//! `daemon::worker`'s.
//!
//! **Landing** a job looks everything up again: each result carries its
//! output's [`OutputId`](crate::outputs::OutputId), never reused, so an
//! output unplugged while its image decoded is simply not found and its
//! buffer dropped; a buffer for an image no longer wanted there, or for a
//! size the output no longer has, is dropped too (a later draw asks for a
//! new one).
//!
//! A **trial** (a `set` of an image) changes nothing until it lands:
//!
//! - it could not be decoded: its reply is the reason, and every output
//!   keeps what it showed;
//! - newer choices cover everything it asked for (it was superseded while
//!   decoding): its reply is `ok`, and nothing changes;
//! - otherwise it is recorded as the choice ([`Choices::set`], which keeps
//!   any newer choice for a single output), the outputs it now applies to
//!   are stamped with its generation and drawn, and its reply waits for
//!   them like a color's (`crate::waiters`).

use std::io;
use std::sync::Arc;

use wayland_client::QueueHandle;

use super::change::{image_dims, reconcile, sweep};
use super::respond::Ready;
use super::surfaces::Objects;
use super::wayland::State;
use super::worker::{Done, JobError, Rendered, Worker};
use crate::choices::Choices;
use crate::control::ConnId;
use crate::jobs::{Jobs, Target};
use crate::outputs::{Entry, Outputs};
use crate::print::warn;
use crate::waiters::Outcome;
use crate::wallpaper::{Image, Wallpaper};

pub struct Images {
    pub jobs: Jobs<ConnId>,
    pub worker: Worker,
}

impl std::fmt::Debug for Images {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Images")
            .field("jobs", &self.jobs)
            .finish_non_exhaustive()
    }
}

impl Images {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            jobs: Jobs::default(),
            worker: Worker::new()?,
        })
    }
}

/// Whether `entry` should show `image` (it is the choice there).
fn wants(choices: &Choices, entry: &Entry<Objects>, image: &Image) -> bool {
    let name = entry.output.info().name.as_deref();
    choices
        .for_output(name)
        .and_then(Wallpaper::image)
        .is_some_and(|wanted| wanted.serial == image.serial)
}

/// Starts the next job if the worker is free. Returns whether one failed
/// to start (and was landed as failed), which may have queued a reply.
pub fn pump(state: &mut State, qh: &QueueHandle<State>) -> bool {
    let State {
        outputs,
        choices,
        images,
        ..
    } = &mut *state;
    let work = images.jobs.next(|image, target| {
        outputs.iter().any(|entry| {
            entry.output.id() == target.output
                && wants(choices, entry, image)
                && image_dims(&entry.output) == Some(target.dims)
        })
    });
    let Some((image, targets)) = work else {
        return false;
    };
    match images.worker.start(image, targets) {
        Ok(()) => false,
        Err(error) => {
            land(state, Err(error), qh);
            true
        }
    }
}

/// Lands the running job's result (see the module docs).
pub fn land(state: &mut State, done: Done, qh: &QueueHandle<State>) {
    let State {
        globals,
        outputs,
        choices,
        waiters,
        images,
        ready,
        ..
    } = &mut *state;
    let Some(job) = images.jobs.finished() else {
        return;
    };
    let image = job.image;
    let Some(trial) = job.trial else {
        // A render: the image is already the choice.
        match done {
            Ok(rendered) => {
                offer(outputs, choices, &image, rendered);
                for target in &job.targets {
                    if let Some(entry) = outputs.get_mut(target.output) {
                        reconcile(globals, choices, &mut images.jobs, entry, qh);
                    }
                }
            }
            Err(error) => {
                for target in &job.targets {
                    if let Some(entry) = outputs.get_mut(target.output) {
                        if wants(choices, entry, &image) {
                            failed(entry, &image, &error);
                        }
                    }
                }
            }
        }
        return;
    };
    let rendered = match done {
        Ok(rendered) => rendered,
        Err(error) => {
            let message = format!("cannot show {:?}: {error}; nothing was changed", image.path);
            ready.push((trial.conn, Ready::Refused(message)));
            return;
        }
    };
    let generation = image.serial;
    let output = trial.output.as_deref();
    let choice = Some(Wallpaper::Image(Arc::clone(&image)));
    if !choices.set(output, choice, generation) {
        // Superseded while it decoded: what is shown is newer. `rendered`
        // is dropped.
        ready.push((trial.conn, Ready::Done(Outcome::Shown)));
        return;
    }
    // Older image requests this one covers need not be decoded at all.
    sweep(&mut images.jobs, choices, ready);
    let applies = |entry: &Entry<Objects>| {
        output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
            && wants(choices, entry, &image)
    };
    // Stamped before the buffers are offered, which may mark a failed
    // draw (a stamp clears that).
    for entry in outputs.iter_mut().filter(|entry| applies(entry)) {
        entry.output.want(generation);
    }
    offer(outputs, choices, &image, rendered);
    for entry in outputs.iter_mut().filter(|entry| applies(entry)) {
        reconcile(globals, choices, &mut images.jobs, entry, qh);
    }
    waiters.push(trial.conn, generation);
}

/// Hands each rendered buffer to its output, if that output is still
/// there, still wants this image, and is still that size; reports a draw
/// that failed.
fn offer(
    outputs: &mut Outputs<Objects>,
    choices: &Choices,
    image: &Image,
    rendered: Vec<Rendered>,
) {
    for (Target { output, dims }, result) in rendered {
        let Some(entry) = outputs.get_mut(output) else {
            continue;
        };
        if !wants(choices, entry, image) {
            continue;
        }
        match result {
            Ok(buffer) => {
                if image_dims(&entry.output) == Some(dims) {
                    entry.objects.canvas.offer(image.serial, dims, buffer);
                }
            }
            Err(message) => {
                warn(format_args!(
                    "scootbg: cannot draw {:?} on {}: {message}",
                    image.path,
                    entry.output.label()
                ));
                entry.output.draw_failed();
            }
        }
    }
}

/// A render job failed for `entry`: said on stderr, and not retried until
/// the output is reconfigured or targeted again (`Output::draw_failed`).
fn failed(entry: &mut Entry<Objects>, image: &Image, error: &JobError) {
    warn(format_args!(
        "scootbg: cannot draw {:?} on {}: {error}",
        image.path,
        entry.output.label()
    ));
    entry.output.draw_failed();
}
