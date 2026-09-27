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

use wayland_client::{Connection, QueueHandle};

use super::canvas::Drew;
use super::respond::{ChangeError, Wallpaper};
use super::surfaces::{LayerObjects, Objects, RoundTrip};
use super::wayland::{Globals, State};
use crate::choices::{Choice, Choices};
use crate::control::ConnId;
use crate::outputs::Entry;
use crate::paint::Plan;
use crate::print::warn;
use crate::protocol::OutputList;
use crate::waiters;

/// Makes `entry`'s surface show what `choices` says for it. Returns whether
/// it committed.
pub fn reconcile(
    globals: &Globals,
    choices: &Choices,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
) -> bool {
    let info = entry.output.info();
    let wanted = choices.for_output(info.name.as_deref());
    let scale = globals.path.buffer_scale(info.scale);
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
            match objects.canvas.show(globals, qh, id, layer, target) {
                Ok(Drew::Committed) => {
                    entry.output.drew(target);
                    true
                }
                Ok(Drew::Stalled) => false,
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
                let progress = output.progress(wanted, path.buffer_scale(info.scale));
                (output.stamp(), progress)
            }),
        )
    });
    if let Some(sync) = sync {
        conn.display().sync(qh, RoundTrip::Replies(sync));
    }
}

/// The daemon's state as the request handler sees it.
pub struct Control<'a> {
    pub state: &'a mut State,
    pub qh: &'a QueueHandle<State>,
}

impl Wallpaper for Control<'_> {
    fn outputs(&self) -> &dyn OutputList {
        &self.state.outputs
    }

    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        choice: Choice,
    ) -> Result<(), ChangeError> {
        let State {
            globals,
            outputs,
            choices,
            waiters,
            ..
        } = &mut *self.state;
        let targets = |entry: &Entry<Objects>| {
            output.is_none_or(|name| entry.output.info().name.as_deref() == Some(name))
        };
        if output.is_some() && !outputs.iter().any(targets) {
            return Err(ChangeError::UnknownOutput);
        }
        let generation = waiters.next_generation();
        choices.set(output, choice);
        for entry in outputs.iter_mut().filter(|entry| targets(entry)) {
            entry.output.want(generation);
            reconcile(globals, choices, entry, self.qh);
        }
        // Answered by `progress` at the top of the next loop turn, once
        // every targeted output shows it.
        waiters.push(conn, generation);
        Ok(())
    }
}
