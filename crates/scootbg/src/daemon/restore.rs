//! Start-up: reading the profile's state file (`crate::state`) and showing
//! what it says again.
//!
//! Runs once, after the Wayland connection is made and before the loop
//! serves anyone, so every restored choice is older than any request (each
//! takes a generation from the same counter, `crate::waiters`), and no
//! output is configured yet: the choices are put in place, and each output
//! is drawn when its `configure` comes, as for any choice. An image is
//! therefore decoded once, when its outputs are configured (`crate::jobs`
//! on why that is once), not once to validate it and again to draw it.
//!
//! **Nothing here writes the file.** The saved table ([`Saved::choices`])
//! is filled with everything the file says, whether it is shown or not, so
//! the next `set` writes all of it back: entries for outputs not plugged
//! in, and images that could not be restored, stay until a `set` or
//! `clear` replaces them.
//!
//! **A restored image that cannot be found** (moved, deleted, on a disk
//! not mounted yet) is not made a choice: that output shows the
//! compositor's own background, with a warning, and the daemon starts as
//! usual. Checked by one `stat` here, not by decoding: a file that exists
//! but cannot be decoded fails when it is drawn, and says so then, as any
//! image that fails to draw does.

use std::sync::Arc;

use super::wayland::State;
use crate::choices::Choice;
use crate::print::warn;
use crate::state::format::{Pick, Record};
use crate::state::{self, Profile, Saved};
use crate::wallpaper::{Image, Wallpaper};

/// Reads `profile`'s state file. Returns the table to save into, set to
/// save to that file unless it must not be written (none can be found, it
/// could not be read, or it is a newer scootbg's), and what the file says.
/// Every problem is a warning on stderr; none stops the daemon.
pub fn load(profile: Profile) -> (Saved, Record) {
    let Some(dir) = state::dir_from_env() else {
        warn(format_args!(
            "scootbg: neither XDG_STATE_HOME nor HOME is an absolute path, so there is \
             nowhere to keep state: the wallpaper is not restored, and changes are not saved"
        ));
        return (Saved::new(profile, None, None), Record::default());
    };
    let file = dir.join(profile.as_str());
    let parsed = match state::load(&file) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return (Saved::new(profile, Some(file), None), Record::default()),
        Err(error) => {
            warn(format_args!(
                "scootbg: cannot read the state file {}: {error}; nothing was restored, \
                 and changes are not saved (so as not to write over what it holds)",
                file.display()
            ));
            return (Saved::new(profile, None, None), Record::default());
        }
    };
    for warning in &parsed.warnings {
        warn(format_args!(
            "scootbg: state file {}: {warning}",
            file.display()
        ));
    }
    let record = parsed.record;
    if let Some(named) = record.profile.as_deref() {
        if named != profile.as_str() {
            warn(format_args!(
                "scootbg: state file {} says it is profile {named:?}; it is read as \
                 profile {:?}, its file name",
                file.display(),
                profile.as_str()
            ));
        }
    }
    let file = (!parsed.newer).then_some(file);
    let fingerprint = record.fingerprint.clone();
    (Saved::new(profile, file, fingerprint), record)
}

/// Puts what `record` says in the saved table and, when `show` (not
/// `--no-restore`), in the daemon's choices: the choice for every output
/// first, then each named one, so a named choice stays over the one for
/// every output, as it did when it was made.
pub fn apply(state: &mut State, record: Record, show: bool) {
    let mut put = |output: Option<&str>, pick: Pick| {
        let generation = state.waiters.next_generation();
        let choice: Choice = match pick {
            Pick::Clear => None,
            Pick::Color(color) => Some(Wallpaper::Color(color)),
            Pick::Image { path, look } => Some(Wallpaper::Image(Arc::new(Image {
                path,
                look,
                serial: generation,
            }))),
        };
        state.saved.choices.set(output, choice.clone(), generation);
        if !show {
            return;
        }
        if let Some(Wallpaper::Image(image)) = &choice {
            if let Err(why) = present(&image.path) {
                let target = Target(output);
                warn(format_args!(
                    "scootbg: cannot restore {:?} for {target}: {why}; showing the \
                     compositor's own background there (it stays saved until the next \
                     `scootbg set` or `clear` for it)",
                    image.path
                ));
                return;
            }
        }
        state.choices.set(output, choice, generation);
    };
    if let Some(all) = record.all {
        put(None, all);
    }
    for (name, pick) in record.named {
        put(Some(&name), pick);
    }
}

/// Whether `path` is a regular file now (following links), and if not,
/// why.
fn present(path: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err("not a regular file".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// "every output", or the named one, for a message.
struct Target<'a>(Option<&'a str>);

impl std::fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("every output"),
            Some(name) => write!(f, "output \"{}\"", name.escape_debug()),
        }
    }
}
