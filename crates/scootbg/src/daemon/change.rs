//! Changing what outputs show, and answering once they do.
//!
//! [`reconcile`] makes one output's surface show what the choices say, and
//! is called wherever that could have changed: a `set` or `clear`, a
//! `configure` (first, or a resize), `wl_output.done` (a new scale or
//! mode), a buffer released while a draw waited for one. It is idempotent:
//! when the surface already shows the right thing it sends nothing.
//!
//! [`progress`] runs once per loop turn, after events are dispatched: the
//! waiting requests whose outputs now show them go in flight behind one
//! `wl_display.sync`, sent after their commits on the same connection, and
//! its callback queues their replies (`State::ready`), which the loop hands
//! to the connections. See `crate::waiters` for the bookkeeping and its
//! bounds.

use std::sync::Arc;

use wayland_client::{Connection, QueueHandle};

use super::canvas::Drew;
use super::respond::{ChangeError, Changes};
use super::surfaces::{LayerObjects, Objects, RoundTrip};
use super::wayland::{Globals, State};
use crate::choices::{Choice, Choices};
use crate::control::ConnId;
use crate::jobs::{Jobs, Target, Trial};
use crate::outputs::{Entry, Output};
use crate::paint::{self, Plan};
use crate::print::warn;
use crate::protocol::{OutputList, Show};
use crate::waiters::{self, Waiters};
use crate::wallpaper::{Image, Wallpaper};

/// Makes `entry`'s surface show what `choices` says for it. Returns whether
/// it committed. An image with no buffer rendered at the size needed is
/// asked of the worker (`jobs`), and drawn when it lands.
pub fn reconcile(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
) -> bool {
    let info = entry.output.info();
    let wanted = choices.for_output(info.name.as_deref());
    // A render waiting for an image no longer wanted here is dropped now,
    // whether or not anything can be drawn.
    let serial = wanted.and_then(Wallpaper::image).map(|image| image.serial);
    entry.objects.canvas.forget_ready_unless(serial);
    let scale = paint::buffer_scale(globals.path, wanted, info.scale);
    let id = entry.output.id();
    match entry.output.plan(wanted, scale) {
        Plan::Nothing => false,
        Plan::Show(target) => {
            let objects = &mut entry.objects;
            // A configured surface is a live one; `plan` says `Show` only
            // for those.
            let Some(layer) = objects.layer.as_mut() else {
                return false;
            };
            match objects.canvas.show(globals, qh, id, layer, &target) {
                Ok(Drew::Committed) => {
                    entry.output.drew(target);
                    true
                }
                Ok(Drew::Stalled) => false,
                Ok(Drew::NeedsRender(dims)) => {
                    if let Wallpaper::Image(image) = &target.content {
                        jobs.render(image, Target { output: id, dims });
                    }
                    false
                }
                Err(error) => {
                    warn(format_args!(
                        "scootbg: cannot draw on {}: {error}",
                        entry.output.label()
                    ));
                    entry.output.draw_failed();
                    false
                }
            }
        }
        Plan::Clear => {
            let objects = &mut entry.objects;
            if let Some(layer) = objects.layer.take() {
                layer.destroy();
            }
            objects.canvas.clear();
            objects.layer = Some(LayerObjects::create(globals, &objects.output, qh, id));
            entry.output.recreated();
            true
        }
    }
}

/// The buffer size, in pixels, an image would be drawn at on `output` now:
/// its surface size times its integer scale. `None` while it has no size,
/// or if that overflows (the draw then fails and says so).
pub fn image_dims(output: &Output) -> Option<(u32, u32)> {
    let size = output.surface_size()?;
    let scale = output.info().scale.max(1);
    Some((
        size.width.checked_mul(scale)?,
        size.height.checked_mul(scale)?,
    ))
}

/// Sends one sync for the waiting requests whose outputs now show them.
pub fn progress(state: &mut State, conn: &Connection, qh: &QueueHandle<State>) {
    if state.waiters.is_idle() {
        return;
    }
    let State {
        globals,
        outputs,
        choices,
        waiters,
        ..
    } = state;
    let path = globals.path;
    let sync = waiters.resolve(|generation| {
        waiters::outcome(
            generation,
            outputs.iter().map(|entry| {
                let output = &entry.output;
                let info = output.info();
                let wanted = choices.for_output(info.name.as_deref());
                let scale = paint::buffer_scale(path, wanted, info.scale);
                (output.stamp(), output.progress(wanted, scale))
            }),
        )
    });
    if let Some(sync) = sync {
        conn.display().sync(qh, RoundTrip::Replies(sync));
    }
}

/// Takes every queued image request that newer choices have made moot out
/// of the queue: it would change nothing, so it is never decoded. Its
/// reply waits at its own generation, as a superseded color's does, so it
/// comes once what replaced it is on screen (`crate::waiters`).
pub fn sweep(jobs: &mut Jobs<ConnId>, choices: &Choices, waiters: &mut Waiters<ConnId>) {
    jobs.sweep(
        |output, generation| choices.supersedes(output, generation),
        |conn, generation| waiters.push(conn, generation),
    );
}

/// The daemon's state as the request handler sees it.
pub struct Control<'a> {
    pub state: &'a mut State,
    pub qh: &'a QueueHandle<State>,
}

impl Changes for Control<'_> {
    fn outputs(&self) -> &dyn OutputList {
        &self.state.outputs
    }

    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        show: Option<Show<'_>>,
    ) -> Result<(), ChangeError> {
        let State {
            globals,
            outputs,
            choices,
            waiters,
            images,
            ..
        } = &mut *self.state;
        let targets = |entry: &Entry<Objects>| {
            output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
        };
        if output.is_some() && !outputs.iter().any(targets) {
            return Err(ChangeError::UnknownOutput);
        }
        let generation = waiters.next_generation();
        let choice: Choice = match show {
            None => None,
            Some(Show::Color(color)) => Some(Wallpaper::Color(color)),
            Some(Show::Image(request)) => {
                // Nothing changes until it has decoded (`crate::jobs`): a
                // trial, drawn for the outputs configured now.
                let image = Arc::new(Image {
                    path: request.path.into_owned(),
                    look: crate::image::render::Look {
                        mode: request.mode,
                        fill: request.fill,
                        filter: request.filter,
                    },
                    serial: generation,
                });
                let draw_for = outputs
                    .iter()
                    .filter(|entry| targets(entry))
                    .filter_map(|entry| {
                        Some(Target {
                            output: entry.output.id(),
                            dims: image_dims(&entry.output)?,
                        })
                    })
                    .collect();
                let trial = Trial {
                    conn,
                    output: output.map(str::to_owned),
                };
                return images
                    .jobs
                    .trial(image, trial, draw_for)
                    .map_err(|_| ChangeError::Busy);
            }
        };
        choices.set(output, choice, generation);
        sweep(&mut images.jobs, choices, waiters);
        for entry in outputs.iter_mut().filter(|entry| targets(entry)) {
            entry.output.want(generation);
            reconcile(globals, choices, &mut images.jobs, entry, self.qh);
        }
        // Answered by `progress` at the top of the next loop turn, once
        // every targeted output shows it.
        waiters.push(conn, generation);
        Ok(())
    }
}
