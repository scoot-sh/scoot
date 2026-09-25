//! Restoring a reconnected monitor's workspaces.
//!
//! Under `--tty` every DPMS/standby cycle that drops hot-plug detect is an
//! unplug followed by a replug: `State::remove_output` adopts the removed
//! output's workspaces onto the remaining output, and the monitor comes back
//! as a fresh [`OutputId`](scoot_core::OutputId) with an empty workspace.
//! This module is the memory in between: on removal the core's
//! [`EvictedOutput`](scoot_core::EvictedOutput) is filed keyed by the
//! monitor's [`OutputIdentity`](super::output_identity::OutputIdentity), and
//! on add with a matching identity the still-open windows move back.
//!
//! Two maps on [`State`](super::State), both keyed or valued by identity:
//!
//! - `output_identities`: every live output's identity, registered when the
//!   output is created (name-only on the connector-less backends, name plus
//!   EDID under `--tty`) and forgotten on removal. Read only by
//!   `State::remove_output`, so it never disagrees with what is live.
//! - `displaced`: one record per removed output awaiting its monitor's
//!   return. Written only by `State::remove_output`, consumed only by
//!   `State::restore_displaced`. A second removal under the same identity
//!   overwrites the first (the latest state wins); an add with no record
//!   does nothing.

use scoot_core::{EvictedOutput, OutputId};

use super::State;
use super::output_identity::OutputIdentity;

/// One removed output awaiting its monitor's return: what left.
pub(super) struct DisplacedOutput {
    pub(super) evicted: EvictedOutput,
}

impl State {
    /// Records which monitor `id` is, replacing the name-only identity its
    /// creation registered. The `--tty` paths call this once the full
    /// connector identity (name plus EDID) is known: at startup
    /// (`compositor::run`, from each `StartupHead`) and on hotplug (from
    /// each `Change::Added`). Unknown ids are ignored, so a head that failed
    /// to register never leaves a stale entry.
    pub(crate) fn note_output_identity(&mut self, id: OutputId, identity: OutputIdentity) {
        if self.outputs.get(id).is_some() {
            self.output_identities.insert(id, identity);
        }
    }

    /// Files the removed output's identity for a later restore, and answers
    /// it. The identity the output was registered under, or the name-only
    /// one from its `wl_output` name when nothing was registered (a path
    /// that missed its `note_output_identity` still restores by name).
    pub(super) fn take_output_identity(&mut self, id: OutputId) -> OutputIdentity {
        if let Some(identity) = self.output_identities.remove(&id) {
            return identity;
        }
        OutputIdentity::named(
            &self
                .outputs
                .get(id)
                .map(|output| output.name())
                .unwrap_or_default(),
        )
    }

    /// Moves a returning monitor's still-open windows back onto `id`: the
    /// record `State::remove_output` filed under this output's identity, if
    /// any, handed to the core, which restores what is still where the
    /// adoption put it and leaves hand-moved windows and closed ones alone.
    /// A miss (a genuinely new monitor, or nothing ever removed) does
    /// nothing. Runs at the end of every runtime output add, and once more
    /// on the `--tty` hotplug path after the full identity lands -- a hit
    /// consumes the record, so running twice is safe.
    pub(crate) fn restore_displaced(&mut self, id: OutputId) {
        let Some(identity) = self.output_identities.get(&id).cloned() else {
            return;
        };
        let Some(displaced) = self.displaced.remove(&identity) else {
            return;
        };
        let moved = self.world.restore_output(id, displaced.evicted);
        tracing::info!(
            connector = %identity.name,
            output = id.0,
            windows = moved,
            "a display came back; restored its workspaces"
        );
        self.apply();
    }
}
