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
//!    (`state::saver`), and is kept to be waited for on the way out, or
//!    taken back if that profile is adopted again while it still writes,
//!    so two writers never write one file.
//! 2. **Compares fingerprints**: the section's (`crate::section`) with the
//!    one the profile's state recorded, which is the last section applied
//!    *from the config*. Different: the section is applied like a `set` of
//!    everything (an empty one clears), and its fingerprint recorded in the
//!    same write. The same: nothing changes, or, on adopting, the profile's
//!    saved choices are shown, a later `scootbg set` included; and either
//!    way the section's images that are not showing are checked again
//!    ([`recheck`]).
//! 3. **Answers once every output shows what it should**: every output is
//!    stamped and reconciled (which also retries a draw that failed), and
//!    the reply waits like a `set`'s (`crate::waiters`). An image in the
//!    section that is not a file is an error reply at once, after the rest
//!    is applied; its choice is saved all the same, like a restored image
//!    that has gone (`daemon::restore`). An unchanged section reports it
//!    too, every time until the file is back, so the exit status of every
//!    `apply-config` says whether all of the section shows: a daemon an
//!    `apply-config` started has already put the section when that
//!    command's own request arrives, unchanged by then, and this is how its
//!    missing images still reach the first reply.
//!
//! **A missing image put back.** An image the section chose that was not
//! a file when it was put is saved all the same, and shows nothing. An
//! unchanged `apply-config` looks at each such entry again ([`recheck`]):
//! if its saved choice is still the section's own (the same path and
//! look), the live one is not, and the file is there now, the saved choice
//! is put back as the live one, at the generation it was made at
//! ([`Choices::fill`](crate::choices::Choices::fill)), exactly as if the
//! file had been there all along. **A `set` made since always stands:**
//! any `set` or `clear` for that output, or for every output, replaced the
//! saved choice, so the entry is no longer the section's and is left
//! alone; and filling at the old generation, not a new one, makes the
//! image no newer than it was, so it cannot undo a later request that is
//! still on its way (an image `set` decoding). That is the precedence rule
//! unchanged: the section's choice is filled in only where it is still
//! the last thing chosen there.
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
use crate::choices::Choice;
use crate::control::ConnId;
use crate::print::warn;
use crate::section::Section;
use crate::state::Profile;
use crate::state::format::{Pick, Record};
use crate::wallpaper::Wallpaper;

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
        recheck(state, section)
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

/// For an unchanged section: each image it chose that is saved as its own
/// but not live is put back if its file is there now, and reported if not
/// (see the module docs). Quiet on stderr: the reply says it.
fn recheck(state: &mut State, section: &Section) -> Vec<String> {
    let mut problems = Vec::new();
    let record = section.record();
    let entries = record.all.iter().map(|pick| (None, pick)).chain(
        record
            .named
            .iter()
            .map(|(name, pick)| (Some(name.as_str()), pick)),
    );
    for (output, pick) in entries {
        let Pick::Image { path, look } = pick else {
            continue;
        };
        let is_it = |choice: Option<&Choice>| {
            matches!(
                choice,
                Some(Some(Wallpaper::Image(image))) if image.path == *path && image.look == *look
            )
        };
        let saved = state.saved.choices.exact(output);
        if !is_it(saved) || is_it(state.choices.exact(output)) {
            // Replaced by a `set` since, or showing.
            continue;
        }
        match restore::present(path) {
            Ok(()) => {
                let saved = saved.cloned().flatten();
                state.choices.fill(output, saved);
            }
            Err(why) => problems.push(format!("{path:?} for {}: {why}", restore::Target(output))),
        }
    }
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
    // Adopted again while its old writer still writes (A, B, A within one
    // save): wait for that write, bounded, so the file read below is the
    // newest. This stalls the loop only then, and only as long as the disk
    // takes to finish a write of a few hundred bytes.
    if let Some(old) = state.retired.iter().find(|old| *old.profile() == profile) {
        if !old.flush(super::SAVE_GRACE) {
            warn(format_args!(
                "scootbg: the state file of profile {:?} was still being written after {} s; \
                 reading it as it is",
                profile.as_str(),
                super::SAVE_GRACE.as_secs()
            ));
        }
    }
    let (mut saved, record) = restore::load(profile);
    let generation = state.waiters.next_generation();
    state.choices.set(None, None, generation);
    // Writers with nothing left to write are dropped: they have nothing to
    // lose. One still stuck on this profile (the wait above ran out) is
    // taken back rather than a second one started (`Saved::take_writer`),
    // so one file never has two writers racing their renames.
    state.retired.retain(|old| !old.idle());
    if let Some(index) = state
        .retired
        .iter()
        .position(|old| old.profile() == saved.profile())
    {
        let mut old = state.retired.swap_remove(index);
        if !saved.take_writer(&mut old) {
            state.retired.push(old);
        }
    }
    let old = std::mem::replace(&mut state.saved, saved);
    state.saved.adopted_at(generation);
    state.retired.push(old);
    record
}
