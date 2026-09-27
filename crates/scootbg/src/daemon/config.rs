//! `apply-config` in the daemon: scoot's `[wallpaper]` section, and the
//! rule it keeps, "whichever you changed last wins"
//! (docs/scootbg/backlog/scoot-integration.md).
//!
//! Every `apply-config` carries a profile and a section. The daemon:
//!
//! 1. **Adopts the profile**, if it is not the one it has: reads that
//!    profile's state file ([`restore::load`]) and saves there from now
//!    on, whatever profile it started with, so which process won the socket
//!    never decides where state lives. Nothing older than the adoption
//!    shows or is saved in the new profile: an every-output `clear` is
//!    chosen at the adoption's generation (and every later choice is newer
//!    still), and [`Saved::adopted_at`](crate::state::Saved::adopted_at)
//!    keeps an image `set` sent before it, landing after, out of the new
//!    file. The old profile's writer finishes on its own thread
//!    (`state::saver`), and is kept to be waited for on the way out.
//! 2. **Compares fingerprints**: the section's (`crate::section`) with the
//!    one the profile's state recorded, which is the last section applied
//!    *from the config*. Different: the section is applied like a `set` of
//!    everything (an empty one clears), and its fingerprint recorded in the
//!    same write. The same: nothing changes, or, on adopting, the profile's
//!    saved choices are shown, a later `scootbg set` included.
//! 3. **Answers once every output shows what it should**: every output is
//!    stamped and reconciled (which also retries a draw that failed), and
//!    the reply waits like a `set`'s (`crate::waiters`). An image in the
//!    section that is not a file is an error reply at once, after the rest
//!    is applied; its choice is saved all the same, like a restored image
//!    that has gone (`daemon::restore`).
//!
//! Only this module sets a fingerprint (`Saved::applied_config`), so a
//! `set` after an `apply-config` survives restarts and unchanged
//! re-applies until the section itself changes.
//!
//! A daemon *started* by `apply-config` does the same at start-up, before
//! it serves anyone and before any output is configured ([`start`]), so it
//! never shows the saved state first and then the section.

use wayland_client::QueueHandle;

use super::change::{reconcile, sweep};
use super::restore::{self, Origin};
use super::wayland::State;
use crate::control::ConnId;
use crate::print::warn;
use crate::section::Section;
use crate::state::Profile;
use crate::state::format::Record;

/// The section a daemon started by `apply-config` starts from.
#[derive(Debug, Clone)]
pub struct Start {
    pub section: Section,
    pub fingerprint: String,
}

impl Start {
    pub fn new(section: Section) -> Self {
        let fingerprint = section.fingerprint();
        Self {
            section,
            fingerprint,
        }
    }
}

/// Start-up: `record` is the profile's state file, loaded and not shown.
/// Shows the section if it changed since it was last applied, else what
/// the file says.
pub fn start(state: &mut State, record: Record, start: &Start) {
    if state.saved.fingerprint() == Some(start.fingerprint.as_str()) {
        restore::apply(state, record, true);
    } else {
        // Its problems were said on stderr; nobody waits for a reply.
        put(state, &start.section, start.fingerprint.clone());
    }
}

/// A running daemon's `apply-config` from `conn` (see the module docs).
/// `Err` is the reply to send at once.
pub fn apply(
    state: &mut State,
    qh: &QueueHandle<State>,
    conn: ConnId,
    profile: Profile,
    section: &Section,
) -> Result<(), String> {
    let fingerprint = section.fingerprint();
    let adopted = (*state.saved.profile() != profile).then(|| adopt(state, profile));
    let problems = if state.saved.fingerprint() == Some(fingerprint.as_str()) {
        if let Some(record) = adopted {
            restore::apply(state, record, true);
        }
        Vec::new()
    } else {
        put(state, section, fingerprint)
    };
    let State {
        globals,
        outputs,
        choices,
        waiters,
        images,
        ..
    } = state;
    // Image `set`s still queued that this made moot are never decoded.
    sweep(&mut images.jobs, choices, waiters);
    // Newer than every choice just made: every output is waited for.
    let generation = waiters.next_generation();
    for entry in outputs.iter_mut() {
        entry.output.want(generation);
        reconcile(globals, choices, &mut images.jobs, entry, qh);
    }
    if problems.is_empty() {
        waiters.push(conn, generation);
        return Ok(());
    }
    Err(format!(
        "the section was applied, except: {}; there, the compositor's own background \
         shows until the file is back (it stays saved)",
        problems.join("; ")
    ))
}

/// Puts `section`'s choices in place and records its fingerprint; returns
/// what could not be shown.
fn put(state: &mut State, section: &Section, fingerprint: String) -> Vec<String> {
    let problems = restore::put(state, section.record(), true, Origin::Config);
    state.saved.applied_config(fingerprint);
    problems
}

/// Switches to `profile`'s state (see the module docs); returns its file's
/// record, not shown yet.
fn adopt(state: &mut State, profile: Profile) -> Record {
    warn(format_args!(
        "scootbg: apply-config: restoring and saving profile {:?} from now on (was {:?})",
        profile.as_str(),
        state.saved.profile().as_str()
    ));
    let (saved, record) = restore::load(profile);
    let generation = state.waiters.next_generation();
    state.choices.set(None, None, generation);
    let old = std::mem::replace(&mut state.saved, saved);
    state.saved.adopted_at(generation);
    state.retired = Some(old);
    record
}
