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
//! **Sharing.** The worker draws each size once; the pixels are wrapped
//! for the compositor once and offered to every output of that size
//! (`daemon::canvas::Pixels`), each showing them through its own
//! `wl_buffer` (`crate::share`). A render asked for an output whose size
//! another output already shows the image at (one plugged in later, a
//! surface re-created) is served from those pixels in [`pump`], with no
//! decode at all.
//!
//! A **trial** (a `set` of an image) changes nothing until it lands:
//!
//! - it could not be decoded: its reply is the reason, and every output
//!   keeps what it showed;
//! - newer choices cover everything it asked for (it was superseded while
//!   decoding): nothing changes, and its reply is `ok` once what replaced
//!   it is on screen, as for a superseded color;
//! - otherwise it is recorded as the choice ([`Choices::set`], which keeps
//!   any newer choice for a single output), the outputs it now applies to
//!   are stamped with its generation and drawn, and its reply waits for
//!   them like a color's (`crate::waiters`).

use std::io;
use std::rc::Rc;
use std::sync::Arc;

use wayland_client::QueueHandle;

use super::canvas::{self, Content};
use super::change::{image_dims, reconcile, sweep};
use super::respond::Ready;
use super::surfaces::Objects;
use super::wayland::{Globals, State};
use super::worker::{Done, JobError, Rendered, Worker};
use crate::choices::Choices;
use crate::control::ConnId;
use crate::jobs::{Jobs, Target, Trial};
use crate::outputs::{Entry, Outputs};
use crate::print::warn;
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

/// Starts the next job if the worker is free, after serving what it can
/// from pixels already drawn ([`share`]). Returns whether a job failed to
/// start (and was landed as failed), which may have queued a reply.
pub fn pump(state: &mut State, qh: &QueueHandle<State>) -> bool {
    share(state, qh);
    let State {
        outputs,
        choices,
        images,
        ..
    } = &mut *state;
    let work = images.jobs.next(
        |image, target| {
            outputs.iter().any(|entry| {
                entry.output.id() == target.output
                    && wants(choices, entry, image)
                    && image_dims(&entry.output) == Some(target.dims)
            })
        },
        |trial| trial_targets(outputs, trial),
    );
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

/// What a trial draws for when it starts: every output it targets that
/// has a size to draw at. `None` (hold it back) while one of them is about
/// to be configured (`Output::coming`), so a `set` sent before the outputs
/// are configured decodes once, for them, rather than once to validate the
/// file and again for their `configure` (`crate::jobs`). The check
/// allocates nothing: it is made on every loop turn a trial waits.
fn trial_targets(outputs: &Outputs<Objects>, trial: &Trial<ConnId>) -> Option<Vec<Target>> {
    let targeted = |entry: &&Entry<Objects>| {
        trial
            .output
            .as_deref()
            .is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
    };
    if outputs.iter().filter(targeted).any(|e| e.output.coming()) {
        return None;
    }
    Some(
        outputs
            .iter()
            .filter(targeted)
            .filter_map(|entry| {
                Some(Target {
                    output: entry.output.id(),
                    dims: image_dims(&entry.output)?,
                })
            })
            .collect(),
    )
}

/// Serves each waiting render from pixels an output already has for that
/// image at that size (on screen, kept, or waiting to be), so the output
/// shares them rather than decode the file again; then draws the outputs
/// served. Nothing to do, and no cost, while no render waits.
fn share(state: &mut State, qh: &QueueHandle<State>) {
    let State {
        globals,
        outputs,
        choices,
        images,
        ..
    } = &mut *state;
    let served = images.jobs.satisfy(|image, target| {
        let Some(pixels) = outputs
            .iter()
            .find_map(|entry| entry.objects.canvas.image(image.serial, target.dims))
            .map(Rc::clone)
        else {
            return false;
        };
        // Still there, still wanting it, still that size; anything else is
        // for `next` to drop.
        match outputs.get_mut(target.output) {
            Some(entry)
                if wants(choices, entry, image)
                    && image_dims(&entry.output) == Some(target.dims) =>
            {
                entry.objects.canvas.offer(pixels);
                true
            }
            _ => false,
        }
    });
    if served {
        // Idempotent: an output with nothing to change sends nothing.
        for entry in outputs.iter_mut() {
            reconcile(globals, choices, &mut images.jobs, entry, qh);
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
        saved,
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
                offer(
                    globals,
                    qh,
                    outputs,
                    choices,
                    &image,
                    &job.targets,
                    rendered,
                );
                for target in &job.targets {
                    if let Some(entry) = outputs.get_mut(target.output) {
                        reconcile(globals, choices, &mut images.jobs, entry, qh);
                    }
                }
            }
            Err(error) => {
                // One download, one line: a fetch served every target, so
                // it is said once here, not once per output below. Nothing
                // still wants it (superseded while it downloaded): silence,
                // as for any superseded job.
                if let JobError::Fetch(fetch) = &error {
                    let mut names: Vec<String> = Vec::new();
                    for target in &job.targets {
                        if let Some(entry) = outputs.get_mut(target.output) {
                            if wants(choices, entry, &image) {
                                names.push(entry.output.label().to_string());
                            }
                        }
                    }
                    if !names.is_empty() {
                        warn(format_args!(
                            "scootbg: cannot show {} on {}: {fetch}; showing the compositor's \
                             own background there",
                            describe(&image),
                            names.join(", "),
                        ));
                    }
                }
                for target in &job.targets {
                    if let Some(entry) = outputs.get_mut(target.output) {
                        if wants(choices, entry, &image) {
                            if error.is_fetch() {
                                entry.output.draw_failed(error.to_string());
                            } else {
                                failed(entry, &image, &error);
                            }
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
            let message = format!(
                "cannot show {}: {error}; nothing was changed",
                describe(&image)
            );
            ready.push((trial.conn, Ready::Refused(message)));
            return;
        }
    };
    let generation = image.serial;
    let output = trial.output.as_deref();
    let choice = Some(Wallpaper::Image(Arc::clone(&image)));
    // Saved by the same newest-wins rule the choices follow, so it is
    // saved exactly when it is recorded below.
    saved.record(output, &choice, generation);
    if !choices.set(output, choice, generation) {
        // Superseded while it decoded: what is shown is newer, and
        // `rendered` is dropped. Answered, as a superseded color is, once
        // what replaced it is on screen.
        waiters.push(trial.conn, generation);
        return;
    }
    // Older image requests this one covers need not be decoded at all.
    sweep(&mut images.jobs, choices, waiters);
    let applies = |entry: &Entry<Objects>| {
        output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
            && wants(choices, entry, &image)
    };
    // Stamped before the buffers are offered, which may mark a failed
    // draw (a stamp clears that).
    for entry in outputs.iter_mut().filter(|entry| applies(entry)) {
        entry.output.want(generation);
    }
    offer(
        globals,
        qh,
        outputs,
        choices,
        &image,
        &job.targets,
        rendered,
    );
    for entry in outputs.iter_mut().filter(|entry| applies(entry)) {
        reconcile(globals, choices, &mut images.jobs, entry, qh);
    }
    waiters.push(trial.conn, generation);
}

/// Hands each rendered size to the job's targets of that size that are
/// still there, still want this image, and are still that size: wrapped
/// for the compositor once, and shared by all of them (a buffer nobody
/// takes is dropped). Reports a draw that failed.
fn offer(
    globals: &Globals,
    qh: &QueueHandle<State>,
    outputs: &mut Outputs<Objects>,
    choices: &Choices,
    image: &Image,
    targets: &[Target],
    rendered: Vec<Rendered>,
) {
    for (dims, result) in rendered {
        let targeted = |entry: &Entry<Objects>| {
            targets
                .iter()
                .any(|t| t.output == entry.output.id() && t.dims == dims)
                && wants(choices, entry, image)
        };
        // Still that size: a buffer for a size gone stale is dropped.
        let takes =
            |entry: &Entry<Objects>| targeted(entry) && image_dims(&entry.output) == Some(dims);
        let shared = match result {
            Ok(buffer) if outputs.iter().any(takes) => {
                canvas::pixels(globals, qh, buffer, Content::Image(image.serial))
                    .map_err(|error| error.to_string())
            }
            Ok(_) => continue,
            Err(message) => Err(message),
        };
        match shared {
            Ok(pixels) => {
                for entry in outputs.iter_mut().filter(|entry| takes(entry)) {
                    entry.objects.canvas.offer(Rc::clone(&pixels));
                }
            }
            Err(message) => {
                for entry in outputs.iter_mut().filter(|entry| targeted(entry)) {
                    warn(format_args!(
                        "scootbg: cannot draw {:?} on {}: {message}",
                        image.path,
                        entry.output.label()
                    ));
                    entry.output.draw_failed(message.clone());
                }
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
    entry.output.draw_failed(error.to_string());
}

/// The image, for a message: the URL for a download, else the path.
fn describe(image: &Image) -> String {
    match &image.fetch {
        Some(fetch) => format!("{:?}", fetch.url),
        None => format!("{:?}", image.path),
    }
}
