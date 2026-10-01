//! What a `subscribe`d connection is told: a module's view changed, an
//! output came or went.
//!
//! **No subscriber, no cost beyond one branch**: [`State::pump_events`]
//! returns at once while the server has none, and nothing is recorded for a
//! subscriber that does not exist (a module's revision is read only when
//! there is someone to tell).
//!
//! **Coalesced to the frame rate.** The loop pumps once a turn, after the
//! draw. A module's revision bumps however often it likes between pumps; the
//! subscriber gets one event for the module's latest view, and no more than
//! one batch every [`FRAME`] (16 ms): a batch held back to its frame is
//! waited for with the loop's poll timeout ([`State::events_timeout`]),
//! which exists only while something is held, so an idle bar sleeps as it
//! always did. A `module` event is the module's `query` entry plus its
//! `type`, once per output that shows it.
//!
//! **A subscriber that cannot keep up is dropped, not buffered**
//! (`control::conn`): each kind's lines go out in one nonblocking write per
//! subscriber.
//!
//! A reload replaces every module, so every module is told again after one:
//! a subscriber learns the new layout from the events, not by asking.

use std::time::{Duration, Instant};

use super::agent;
use super::wayland::State;
use crate::control::Server;
use crate::control::protocol::{Event, EventKind, write_event};
use crate::modules::View;

/// The shortest time between two batches of events: one 60 Hz frame.
pub const FRAME: Duration = Duration::from_millis(16);

/// Output changes held for the next batch, at most: a flood of hotplug
/// events past it drops the oldest, which a subscriber of a storm can live
/// without (it learns the net state from the newest).
const MAX_OUTPUT_CHANGES: usize = 16;

/// An output coming or going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChange {
    pub added: bool,
    pub name: Option<String>,
}

#[derive(Debug, Default)]
pub struct Events {
    /// Whether a subscriber exists, as of the last pump: what `told` and
    /// `outputs` are kept for.
    armed: bool,
    /// Each placed module's revision as last told, by module index; a
    /// length that differs from the modules' (a reload) tells them all.
    told: Vec<u64>,
    outputs: Vec<OutputChange>,
    /// The earliest the next batch may go.
    due: Option<Instant>,
    /// The batch being built, reused.
    modules: Vec<u8>,
    changes: Vec<u8>,
    view: View,
}

impl Events {
    /// The modules were all replaced (a reload): every one is told again.
    pub fn invalidate(&mut self) {
        self.told.clear();
    }

    /// An output was added or removed: told at the next batch, if anyone is
    /// listening. One branch while nobody is.
    pub fn note_output(&mut self, added: bool, name: Option<&str>) {
        if !self.armed {
            return;
        }
        if self.outputs.len() >= MAX_OUTPUT_CHANGES {
            self.outputs.remove(0);
        }
        self.outputs.push(OutputChange {
            added,
            name: name.map(str::to_owned),
        });
    }

    /// Called every loop turn with whether anyone is subscribed and the
    /// modules' revisions: arms on the first subscriber (what is shown then
    /// is the baseline), disarms on the last, and says whether a batch may be
    /// built this turn. **A subscriber joining one already there changes
    /// nothing**: what is owed to the first stays owed.
    fn begin(&mut self, subscribed: bool, revisions: impl ExactSizeIterator<Item = u64>) -> bool {
        if !subscribed {
            if self.armed {
                self.armed = false;
                self.told.clear();
                self.outputs.clear();
                self.due = None;
            }
            return false;
        }
        if !self.armed {
            self.armed = true;
            self.told.clear();
            self.told.extend(revisions);
            self.outputs.clear();
            return false;
        }
        true
    }

    /// Whether anything waits to be told, given the modules' revisions.
    fn pending(&self, revisions: impl ExactSizeIterator<Item = u64>) -> bool {
        !self.outputs.is_empty()
            || self.told.len() != revisions.len()
            || revisions
                .zip(&self.told)
                .any(|(revision, told)| revision != *told)
    }
}

impl State {
    /// Tells the subscribers what changed since the last batch, at most once
    /// a frame.
    pub fn pump_events(&mut self, server: &mut Server, now: &mut Option<Instant>) {
        let modules = &self.content.modules;
        let revisions = || modules.iter().map(|p| p.revision);
        if !self.events.begin(server.subscribers() > 0, revisions())
            || !self.events.pending(revisions())
        {
            return;
        }
        let at = *now.get_or_insert_with(Instant::now);
        if self.events.due.is_some_and(|due| at < due) {
            return;
        }
        self.events.due = Some(at + FRAME);

        let reloaded = self.events.told.len() != modules.len();
        let events = &mut self.events;
        events.modules.clear();
        for (index, placed) in modules.iter().enumerate() {
            if !reloaded && events.told.get(index) == Some(&placed.revision) {
                continue;
            }
            agent::views(
                modules,
                &self.outputs,
                &mut events.view,
                Some(index),
                |view| {
                    write_event(&mut events.modules, &Event::Module(view));
                    events.modules.len() <= agent::MAX_REPLY
                },
            );
        }
        events.changes.clear();
        for change in events.outputs.drain(..) {
            write_event(
                &mut events.changes,
                &Event::Output {
                    change: if change.added { "added" } else { "removed" },
                    name: change.name.as_deref(),
                },
            );
        }
        events.told.clear();
        events.told.extend(revisions());
        server.broadcast(EventKind::Module, &events.modules);
        server.broadcast(EventKind::Output, &events.changes);
    }

    /// How long the loop may sleep for a batch held back to its frame:
    /// `None` unless one is held (and then only as long as it must).
    pub fn events_timeout(&self, now: &mut Option<Instant>) -> Option<Duration> {
        let revisions = self.content.modules.iter().map(|p| p.revision);
        if !self.events.armed || !self.events.pending(revisions) {
            return None;
        }
        let due = self.events.due?;
        let at = *now.get_or_insert_with(Instant::now);
        due.checked_duration_since(at).filter(|d| !d.is_zero())
    }
}

#[cfg(test)]
mod tests;
