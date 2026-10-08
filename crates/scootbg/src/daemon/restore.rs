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
use crate::state::{self, Profile, Saved, saver};
use crate::wallpaper::{Image, Wallpaper};

/// State-file warnings printed at start-up before the rest are counted.
const WARNINGS_SHOWN: usize = 8;

/// Reads `profile`'s state file. Returns the table to save into, set to
/// save to that file unless it must not be written (none can be found, it
/// could not be read, or it is a newer scootbg's), and what the file says.
/// Every problem is a warning on stderr; none stops the daemon.
pub fn load(profile: Profile) -> (Saved, Record) {
    let Some(dir) = state::dir_from_env() else {
        warn(format_args!(
            "scootbg: neither XDG_STATE_HOME nor HOME is an absolute path, so there is \
             nowhere to keep state: the wallpaper is not restored, and changes are not saved \
             (set one and restart `scootbg daemon` to save; `scootbg query` says \
             \"saving\":false meanwhile)"
        ));
        return (Saved::new(profile, None, None), Record::default());
    };
    if let Some(why) = saver::exposed(&dir) {
        warn(format_args!(
            "scootbg: warning: the state directory {} is not private to you ({why}); \
             whoever else can write there can change which wallpaper is restored \
             (`chmod 700` it, or remove it and scootbg makes it private)",
            dir.display()
        ));
    }
    let file = dir.join(profile.as_str());
    let parsed = match state::load(&file) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return (Saved::new(profile, Some(file), None), Record::default()),
        Err(error) => {
            warn(format_args!(
                "scootbg: cannot read the state file {path}: {error}; nothing was restored, \
                 and saving is off until the daemon restarts (so as not to write over what \
                 it holds): fix or remove {path}, then restart `scootbg daemon`, to save \
                 again (`scootbg query` says \"saving\":false meanwhile)",
                path = file.display()
            ));
            return (Saved::new(profile, None, None), Record::default());
        }
    };
    // A file far past the limits has one warning per extra line (thousands
    // for 256 KiB of short lines): the first few say what is wrong, and one
    // line counts the rest, so a log is not flooded at every start.
    for warning in parsed.warnings.iter().take(WARNINGS_SHOWN) {
        warn(format_args!(
            "scootbg: state file {}: {warning}",
            file.display()
        ));
    }
    if let Some(more) = parsed
        .warnings
        .len()
        .checked_sub(WARNINGS_SHOWN)
        .filter(|&n| n > 0)
    {
        warn(format_args!(
            "scootbg: state file {}: and {more} more warnings like these",
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
    if parsed.newer {
        warn(format_args!(
            "scootbg: saving is off until the daemon restarts, so a newer scootbg's state \
             is not written over: remove or rename {path} (or run that newer scootbg), \
             then restart `scootbg daemon`, to save again (`scootbg query` says \
             \"saving\":false meanwhile)",
            path = file.display()
        ));
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
    put(state, record, show, Origin::StateFile);
}

/// Where the choices [`put`] puts come from, for its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The state file, at start-up or on adopting a profile.
    StateFile,
    /// A `[wallpaper]` section, through `apply-config`.
    Config,
}

/// [`apply`], from either origin. Returns what could not be shown (an image
/// that is not a regular file now), one line each, also said on stderr;
/// each stays saved all the same. A downloaded image is always put live:
/// its cache file may not be there yet, and the worker fetches it when the
/// outputs are configured; a failed fetch says so then, as a failed draw
/// does.
pub fn put(state: &mut State, record: Record, show: bool, origin: Origin) -> Vec<String> {
    let mut problems = Vec::new();
    let mut put = |output: Option<&str>, pick: Pick| {
        let generation = state.waiters.next_generation();
        let choice: Choice = match pick {
            Pick::Clear => None,
            Pick::Color(color) => Some(Wallpaper::Color(color)),
            Pick::Image { path, look, fetch } => Some(Wallpaper::Image(Arc::new(Image {
                path,
                look,
                animate: true,
                serial: generation,
                fetch,
            }))),
        };
        state.saved.choices.set(output, choice.clone(), generation);
        if !show {
            return;
        }
        if let Some(Wallpaper::Image(image)) = &choice {
            // Remote: recorded live whatever the cache holds (see the
            // function docs); local: a missing file shows the background.
            if image.fetch.is_none() {
                if let Err(why) = present(&image.path) {
                    let problem = format!("{:?} for {}: {why}", image.path, Target(output));
                    let what = match origin {
                        Origin::StateFile => "cannot restore",
                        Origin::Config => "cannot show, from the [wallpaper] section,",
                    };
                    warn(format_args!(
                        "scootbg: {what} {problem}; showing the compositor's own background \
                         there (it stays saved until the next `scootbg set` or `clear` for it)"
                    ));
                    problems.push(problem);
                    // Nothing older shows there instead.
                    state.choices.set(output, None, generation);
                    return;
                }
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
    for (output, workspace, pick, spec) in record.workspaces {
        put_workspace(state, output.as_deref(), &workspace, pick, spec, show);
    }
    problems
}

/// Puts one restored workspace mapping in the saved table and, when
/// `show`, in the daemon's choices: like [`put`], a local image that is
/// gone shows the base wallpaper there (staying saved), and a download is
/// always put live. `spec` is the transition its switches arrive through.
fn put_workspace(
    state: &mut State,
    output: Option<&str>,
    workspace: &str,
    pick: Pick,
    spec: crate::transition::Spec,
    show: bool,
) {
    let generation = state.waiters.next_generation();
    let choice: Choice = match pick {
        Pick::Clear => None,
        Pick::Color(color) => Some(Wallpaper::Color(color)),
        Pick::Image { path, look, fetch } => Some(Wallpaper::Image(Arc::new(Image {
            path,
            look,
            // Like a base image restore: the state file does not persist
            // `animate` (live-only), so a restart re-checks the animation
            // caps and refuses an over-cap mapping until the next `set`.
            animate: true,
            serial: generation,
            fetch,
        }))),
    };
    state
        .saved
        .record_workspace(output, workspace, &choice, spec, generation);
    if !show {
        return;
    }
    if let Some(Wallpaper::Image(image)) = &choice {
        if image.fetch.is_none() {
            if let Err(why) = present(&image.path) {
                warn(format_args!(
                    "scootbg: cannot restore {:?} for workspace {:?}: {why}; showing the \
                     output's own wallpaper there (it stays saved until the next `scootbg \
                     set --workspace` or `clear --workspace` for it)",
                    image.path, workspace,
                ));
                state
                    .choices
                    .set_workspace(output, workspace, None, spec, generation);
                return;
            }
        }
    }
    let rendered = match &choice {
        Some(Wallpaper::Image(image)) => Some(Arc::clone(image)),
        _ => None,
    };
    state
        .choices
        .set_workspace(output, workspace, choice, spec, generation);
    // Where outputs already have a size (a profile adopted at runtime),
    // pre-render the restored image for them, so the first switch is
    // instant; at start-up nothing is configured yet, and the `configure`
    // hook (`daemon::surfaces`) does this instead.
    if let Some(image) = rendered {
        for entry in state.outputs.iter() {
            let name = entry.output.info().name.as_deref();
            let targeted = output.is_none_or(|want| name == Some(want));
            if !targeted {
                continue;
            }
            if let Some(dims) = super::change::image_dims(&entry.output) {
                if entry.objects.canvas.image(image.serial, dims).is_none() {
                    state.images.jobs.render(
                        &image,
                        crate::jobs::Target {
                            output: entry.output.id(),
                            dims,
                        },
                    );
                }
            }
        }
    }
}

/// Whether `path` is a regular file now (following links), and if not,
/// why.
pub fn present(path: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err("not a regular file".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// "every output", or the named one, for a message.
pub struct Target<'a>(pub Option<&'a str>);

impl std::fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("every output"),
            Some(name) => write!(f, "output \"{}\"", name.escape_debug()),
        }
    }
}
