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
use std::time::Instant;

use wayland_client::{Connection, QueueHandle};

use super::canvas::Drew;
use super::respond::{ChangeError, Changes};
use super::rotation::{Rotation, StartError};
use super::surfaces::{LayerObjects, Objects, RoundTrip};
use super::transition::{self, Transitions};
use super::wayland::{Globals, State};
use crate::choices::{Choice, Choices};
use crate::control::ConnId;
use crate::fetch::Fetch;
use crate::image::render::Look;
use crate::jobs::{Jobs, Target, Trial};
use crate::outputs::{Entry, Output, Outputs};
use crate::paint::{self, Plan};
use crate::print::warn;
use crate::protocol::{OutputList, Show, Source, WorkspaceList};
use crate::transition::Spec;
use crate::waiters::{self, Waiters};
use crate::wallpaper::{Image, Wallpaper};

/// Makes `entry`'s surface show what `choices` says for it. Returns whether
/// it committed. A running or requested transition goes through
/// `transition::hook` first; an image with no buffer rendered at the size
/// needed is asked of the worker (`jobs`), and drawn when it lands.
pub fn reconcile(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    now: Instant,
) -> bool {
    if let Some(committed) = transition::hook(globals, choices, jobs, transitions, entry, qh, now) {
        return committed;
    }
    let info = entry.output.info();
    let wanted = choices.for_output(info.name.as_deref(), entry.output.active_workspace());
    // A render waiting for an image no longer wanted here is dropped now,
    // whether or not anything can be drawn.
    let serial = wanted.and_then(Wallpaper::image).map(|image| image.serial);
    entry.objects.canvas.forget_ready_unless(serial);
    let scale = paint::scale_for(globals.path, wanted, entry.output.scale());
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
                        // Stashed for a workspace switch that came: attach
                        // at once rather than decoding again.
                        if objects.canvas.revive(image.serial, dims) {
                            match objects.canvas.show(globals, qh, id, layer, &target) {
                                Ok(Drew::Committed) => {
                                    entry.output.drew(target);
                                    return true;
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    warn(format_args!(
                                        "scootbg: cannot draw on {}: {error}",
                                        entry.output.label()
                                    ));
                                    entry.output.draw_failed(error.to_string());
                                    return false;
                                }
                            }
                        }
                        jobs.render(image, Target { output: id, dims });
                    }
                    false
                }
                Err(error) => {
                    warn(format_args!(
                        "scootbg: cannot draw on {}: {error}",
                        entry.output.label()
                    ));
                    entry.output.draw_failed(error.to_string());
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
            // Any transition on it ends with its surface: its buffers go
            // too, and the fresh surface draws the normal way.
            transition::drop_output(transitions, entry.output.id(), &mut entry.output);
            entry.output.recreated();
            let creation = entry.output.creation();
            objects.layer = Some(LayerObjects::create(
                globals,
                &objects.output,
                qh,
                id,
                creation,
            ));
            true
        }
    }
}

/// The buffer size, in pixels, an image would be drawn at on `output` now:
/// its surface size at its scale, fractional where the compositor said one
/// (`Output::full_buffer`, which the draw's `Drawn::buffer` agrees with by
/// sharing `density::Scale::buffer`). `None` while it has no size, or if
/// that overflows (the draw then fails and says so). A render is asked
/// for, kept and offered only while this still says its size.
pub fn image_dims(output: &Output) -> Option<(u32, u32)> {
    output.full_buffer().map(|buffer| buffer.dims)
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
                let wanted =
                    choices.for_output(output.info().name.as_deref(), output.active_workspace());
                let scale = paint::scale_for(path, wanted, output.scale());
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

    fn workspaces(&self) -> &dyn WorkspaceList {
        &self.state.choices
    }

    fn saving(&self) -> bool {
        self.state.saved.saving()
    }

    fn profile(&self) -> &str {
        self.state.saved.profile().as_str()
    }

    fn rotation(&self) -> Option<crate::protocol::RotationInfo<'_>> {
        self.state.rotation.as_ref().map(|rotation| rotation.info())
    }

    fn apply_config(
        &mut self,
        conn: ConnId,
        profile: crate::state::Profile,
        section: &crate::section::Section,
    ) -> Result<(), String> {
        super::config::apply(self.state, self.qh, conn, profile, section)
    }

    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        show: Option<Show<'_>>,
        transition: Spec,
    ) -> Result<(), ChangeError> {
        let State {
            globals,
            outputs,
            choices,
            waiters,
            images,
            saved,
            transitions,
            rotation,
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
            None => {
                // A `clear` stops a running slideshow: back to the
                // compositor's own background, at once.
                *rotation = None;
                None
            }
            Some(Show::Color(color)) => {
                // A color `set` replaces a running slideshow with itself.
                *rotation = None;
                Some(Wallpaper::Color(color))
            }
            Some(Show::Slideshow(request)) => {
                let look = Look {
                    mode: request.mode,
                    fill: request.fill,
                    filter: request.filter,
                };
                // Moved out once: the refusal below and the trial's
                // slideshow marker both own it.
                let dir = request.dir.into_owned();
                let next = Rotation::start(
                    &dir,
                    request.every_secs,
                    request.shuffle,
                    output,
                    look,
                    transition,
                    Instant::now(),
                )
                .map_err(|error| match error {
                    StartError::NotDirectory => ChangeError::NotDirectory(dir.clone()),
                    StartError::Unreadable { entry, detail } => ChangeError::UnreadableDirectory {
                        dir: dir.clone(),
                        entry,
                        detail,
                    },
                    StartError::Empty => ChangeError::EmptyDirectory(dir.clone()),
                    StartError::TooMany { seen } => ChangeError::TooManyFiles {
                        dir: dir.clone(),
                        seen,
                    },
                })?;
                // Only an accepted slideshow replaces the running one: a
                // refused one changes nothing, running one included. The
                // trial below is what accepts it (too many queued trials
                // refuses), so the running slideshow stays until that
                // succeeds: a refused new `set` leaves the old one showing
                // and advancing.
                let first = next.first().to_owned();
                let started = trial_image(
                    outputs,
                    &mut images.jobs,
                    conn,
                    output,
                    first,
                    None,
                    look,
                    transition,
                    generation,
                    Some(dir),
                );
                if started.is_ok() {
                    *rotation = Some(next);
                }
                return started;
            }
            Some(Show::Image(request)) => {
                // An image `set` replaces a running slideshow with itself.
                *rotation = None;
                // Nothing changes until it has decoded (`crate::jobs`): a
                // trial, drawn for the outputs it targets once none of them
                // is about to be configured (`images::pump`). A download
                // is fetched by the worker thread before it decodes.
                let (path, fetch) = match request.source {
                    Source::Path(path) => (path.into_owned(), None),
                    Source::Url { url, sha256 } => {
                        let fetch = crate::fetch::Fetch {
                            url: url.into_owned(),
                            sha256,
                        };
                        let path = crate::fetch::dir()
                            .map(|dir| crate::fetch::cached_path(&dir, &fetch.url))
                            .map_err(|error| ChangeError::Cache(error.to_string()))?;
                        let path = path.to_string_lossy().into_owned();
                        (path, Some(fetch))
                    }
                };
                return trial_image(
                    outputs,
                    &mut images.jobs,
                    conn,
                    output,
                    path,
                    fetch,
                    Look {
                        mode: request.mode,
                        fill: request.fill,
                        filter: request.filter,
                    },
                    transition,
                    generation,
                    None,
                );
            }
        };
        // Always recorded: nothing is newer than a request made now.
        saved.record(output, &choice, generation);
        choices.set(output, choice, generation);
        sweep(&mut images.jobs, choices, waiters);
        let now = Instant::now();
        for entry in outputs.iter_mut().filter(|entry| targets(entry)) {
            entry.output.want(generation);
            entry.output.request_transition(transition, generation);
            reconcile(
                globals,
                choices,
                &mut images.jobs,
                transitions,
                entry,
                self.qh,
                now,
            );
        }
        // Answered by `progress` at the top of the next loop turn, once
        // every targeted output shows it.
        waiters.push(conn, generation);
        Ok(())
    }

    fn change_workspace(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        workspace: &str,
        show: Option<Show<'_>>,
        transition: Spec,
    ) -> Result<(), ChangeError> {
        let State {
            globals,
            outputs,
            choices,
            waiters,
            images,
            saved,
            transitions,
            workspaces,
            ..
        } = &mut *self.state;
        let targets = |entry: &Entry<Objects>| {
            output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
        };
        if output.is_some() && !outputs.iter().any(targets) {
            return Err(ChangeError::UnknownOutput);
        }
        // At most one workspace image decodes at a time, like any image;
        // the bound is on live mappings, checked as they are made
        // (cleared ones do not count).
        if show.is_some()
            && !choices
                .workspaces()
                .any(|(o, w, ..)| o == output && w == workspace)
            && choices
                .workspaces()
                .filter(|(_, _, choice, ..)| choice.is_some())
                .count()
                >= crate::choices::MAX_WORKSPACES
        {
            return Err(ChangeError::TooManyWorkspaces);
        }
        let generation = waiters.next_generation();
        let choice: Choice = match show {
            None => {
                // Taking the mapping back off: refusing when none stands,
                // so a typo answers loudly rather than clearing nothing.
                let stands = choices
                    .workspaces()
                    .any(|(o, w, choice, ..)| o == output && w == workspace && choice.is_some());
                if !stands {
                    return Err(ChangeError::UnknownWorkspace);
                }
                None
            }
            Some(Show::Color(color)) => Some(Wallpaper::Color(color)),
            Some(Show::Slideshow(_)) => {
                return Err(ChangeError::SlideshowWithWorkspace);
            }
            Some(Show::Image(request)) => {
                // As a base image `set`: nothing changes until it has
                // decoded, and the transition waits with the trial.
                let (path, fetch) = match request.source {
                    Source::Path(path) => (path.into_owned(), None),
                    Source::Url { url, sha256 } => {
                        let fetch = crate::fetch::Fetch {
                            url: url.into_owned(),
                            sha256,
                        };
                        let path = crate::fetch::dir()
                            .map(|dir| crate::fetch::cached_path(&dir, &fetch.url))
                            .map_err(|error| ChangeError::Cache(error.to_string()))?;
                        let path = path.to_string_lossy().into_owned();
                        (path, Some(fetch))
                    }
                };
                let image = Arc::new(Image {
                    path,
                    look: crate::image::render::Look {
                        mode: request.mode,
                        fill: request.fill,
                        filter: request.filter,
                    },
                    serial: generation,
                    fetch,
                });
                let trial = Trial {
                    conn,
                    output: output.map(str::to_owned),
                    slideshow: None,
                    workspace: Some(workspace.to_owned()),
                    transition,
                };
                for entry in outputs.iter_mut().filter(|entry| targets(entry)) {
                    entry.output.request_transition(transition, generation);
                }
                images
                    .jobs
                    .trial(image, trial)
                    .map_err(|_| ChangeError::Busy)?;
                // The mapping lands with the trial: bound by then, so the
                // manager's batches are already arriving when it does.
                super::workspaces::ensure_bound(workspaces, choices, globals, self.qh);
                return Ok(());
            }
        };
        // Pixels stashed for the image this replaces will never be
        // switched to: drop them everywhere now, not when memory runs
        // short. A `None` choice stashes nothing, so clearing one drops
        // nothing.
        if let Some(serial) = choices.workspace_serial(output, workspace) {
            let replaced = choice
                .as_ref()
                .and_then(Wallpaper::image)
                .map(|image| image.serial != serial)
                .unwrap_or(true);
            if replaced {
                for entry in outputs.iter_mut() {
                    entry.objects.canvas.drop_stash_serial(serial);
                }
            }
        }
        // Always recorded: nothing is newer than a request made now.
        saved.record_workspace(output, workspace, &choice, transition, generation);
        choices.set_workspace(output, workspace, choice, transition, generation);
        sweep(&mut images.jobs, choices, waiters);
        let now = Instant::now();
        for entry in outputs.iter_mut().filter(|entry| targets(entry)) {
            entry.output.want(generation);
            entry.output.request_transition(transition, generation);
            reconcile(
                globals,
                choices,
                &mut images.jobs,
                transitions,
                entry,
                self.qh,
                now,
            );
        }
        super::workspaces::ensure_bound(workspaces, choices, globals, self.qh);
        // Answered by `progress` at the top of the next loop turn: at once
        // where the workspace is not active (nothing new shows), once the
        // outputs that are active show it otherwise.
        waiters.push(conn, generation);
        Ok(())
    }
}

/// Queues an image trial: nothing changes until it has decoded, and the
/// reply waits for what replaces it. Shared by an image `set` and a
/// slideshow's first file.
#[allow(clippy::too_many_arguments)]
fn trial_image(
    outputs: &mut Outputs<Objects>,
    jobs: &mut Jobs<ConnId>,
    conn: ConnId,
    output: Option<&str>,
    path: String,
    fetch: Option<Fetch>,
    look: Look,
    transition: Spec,
    generation: u64,
    slideshow: Option<String>,
) -> Result<(), ChangeError> {
    let image = Arc::new(Image {
        path,
        look,
        serial: generation,
        fetch,
    });
    let trial = Trial {
        conn,
        output: output.map(str::to_owned),
        slideshow,
        workspace: None,
        transition,
    };
    // Queued first: a refused trial (too many waiting) changes nothing,
    // not even the outputs' pending transitions.
    jobs.trial(image, trial).map_err(|_| ChangeError::Busy)?;
    // The transition waits with the trial: the pixels landing starts it.
    for entry in outputs
        .iter_mut()
        .filter(|entry| output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name)))
    {
        entry.output.request_transition(transition, generation);
    }
    Ok(())
}
