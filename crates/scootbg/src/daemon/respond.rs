//! What the daemon answers to each request.

use std::fmt;

use crate::choices::Choices;
use crate::control::{Answer, ConnId, Handler};
use crate::outputs::{Outputs, Size};
use crate::protocol::{
    self, OutputEntry, OutputList, PROTOCOL_VERSION, Reply, Request, RotationInfo, Show, Shows,
    SurfaceEntry, WorkspaceEntry, WorkspaceList,
};
use crate::section::Section;
use crate::state::Profile;
use crate::transition::Spec;
use crate::waiters::Outcome;

/// What the handler needs from the daemon's state: the outputs, for
/// `query`, and a way to change what they show, for `set` and `clear`.
pub trait Changes {
    fn outputs(&self) -> &dyn OutputList;

    /// The live per-workspace mappings, for `query`.
    fn workspaces(&self) -> &dyn WorkspaceList;

    /// Whether changes are saved for the next start (`query`'s `saving`).
    fn saving(&self) -> bool;

    /// The profile restored and saved (`query`'s `profile`).
    fn profile(&self) -> &str;

    /// The slideshow running now (`query`'s `rotation`), if any.
    fn rotation(&self) -> Option<RotationInfo<'_>>;

    /// `apply-config`: adopts `profile` and applies `section` if it changed
    /// since it was last applied (`daemon::config`), registering `conn` to
    /// be answered once every output shows what it should. `Err` is the
    /// reply, at once: what could not be applied (the rest was).
    fn apply_config(
        &mut self,
        conn: ConnId,
        profile: Profile,
        section: &Section,
    ) -> Result<(), String>;

    /// Makes every output (`output` is `None`), or the outputs named
    /// `output`, show `show` (nothing when `None`), arriving through
    /// `transition` (`Spec::none()` for `clear`, which lands at once), and
    /// registers `conn` to be answered once they do, or, for an image that
    /// cannot be shown, with why. `Err` changes nothing.
    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        show: Option<Show<'_>>,
        transition: Spec,
    ) -> Result<(), ChangeError>;

    /// Makes `workspace` show `show` (nothing when `None`, which takes the
    /// mapping back off) on every output (`output` is `None`) or on the
    /// outputs named `output`, arriving through `transition` when the
    /// workspace turns active, and registers `conn` to be answered once it
    /// is preloaded (and shown, where already active), or, for an image
    /// that cannot be shown, with why. `Err` changes nothing.
    fn change_workspace(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        workspace: &str,
        show: Option<Show<'_>>,
        transition: Spec,
    ) -> Result<(), ChangeError>;
}

/// Why a `set`, `clear`, `set-workspace` or `clear-workspace` was
/// refused; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeError {
    /// No output has that name now.
    UnknownOutput,
    /// Too many images already wait to be decoded (`crate::jobs`).
    Busy,
    /// A download, with nowhere to cache it.
    Cache(String),
    /// A slideshow's directory is not one any more.
    NotDirectory(String),
    /// A slideshow's directory is there, but an entry in it could not be
    /// read: the entry's path and the operating system's reason
    /// (permissions, a symlink loop).
    UnreadableDirectory {
        dir: String,
        entry: String,
        detail: String,
    },
    /// A slideshow's directory holds no files.
    EmptyDirectory(String),
    /// A slideshow's directory holds more than
    /// [`crate::rotation::MAX_LISTED`] files (`seen`: one past the cap).
    TooManyFiles { dir: String, seen: usize },
    /// Too many workspace mappings already (`crate::choices`).
    TooManyWorkspaces,
    /// No mapping for that workspace stands to clear.
    UnknownWorkspace,
    /// A slideshow for one workspace: slideshows run on every output (or
    /// one `output`), not per workspace.
    SlideshowWithWorkspace,
}

/// The reply text for a refused change.
struct Refused<'a> {
    error: ChangeError,
    output: Option<&'a str>,
    workspace: Option<&'a str>,
}

impl fmt::Display for Refused<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.error {
            ChangeError::UnknownOutput => write!(
                f,
                "no output is named {:?} (`scootbg query` lists them); nothing was changed",
                self.output.unwrap_or_default()
            ),
            ChangeError::Busy => write!(
                f,
                "too many images are waiting to be decoded; nothing was changed (try again \
                 once they are shown)"
            ),
            ChangeError::Cache(error) => write!(
                f,
                "cannot cache the download ({error}); nothing was changed"
            ),
            ChangeError::NotDirectory(dir) => {
                write!(
                    f,
                    "{dir:?} is not a directory any more; nothing was changed"
                )
            }
            ChangeError::UnreadableDirectory { dir, entry, detail } => {
                write!(
                    f,
                    "{dir:?} cannot be fully listed: cannot read the entry {entry:?} \
                     ({detail}); nothing was changed"
                )
            }
            ChangeError::EmptyDirectory(dir) => {
                write!(f, "{dir:?} holds no files; nothing was changed")
            }
            ChangeError::TooManyFiles { dir, seen } => {
                write!(
                    f,
                    "{dir:?} holds more than {} files ({seen} seen); nothing was changed",
                    crate::rotation::MAX_LISTED,
                )
            }
            ChangeError::TooManyWorkspaces => write!(
                f,
                "too many workspace wallpapers are mapped (at most {} live; `scootbg query` \
                 lists them under `workspaces`; clear one with `scootbg clear --workspace` \
                 first); nothing was changed",
                crate::choices::MAX_WORKSPACES,
            ),
            ChangeError::UnknownWorkspace => write!(
                f,
                "no wallpaper is mapped for workspace {:?}{}; nothing was changed",
                self.workspace.unwrap_or_default(),
                match self.output {
                    Some(output) => format!(" on output {output:?}"),
                    None => String::new(),
                }
            ),
            ChangeError::SlideshowWithWorkspace => write!(
                f,
                "a slideshow runs on every output (or one `output`), not per workspace; \
                 map an image or a color with `scootbg set --workspace` instead; nothing \
                 was changed"
            ),
        }
    }
}

/// A reply that waited: a change shown (or not), or an image refused with
/// the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Done(Outcome),
    Refused(String),
}

/// The request handler for one round of the poll loop.
pub struct Responder<'a> {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
    wallpaper: &'a mut dyn Changes,
}

impl<'a> Responder<'a> {
    pub fn new(wallpaper: &'a mut dyn Changes) -> Self {
        Self {
            stop: false,
            wallpaper,
        }
    }
}

/// The reply to a `set` or `clear` that waited.
pub fn write_ready(out: &mut Vec<u8>, ready: &Ready) {
    match ready {
        Ready::Done(Outcome::Shown) => protocol::write_reply(out, &Reply::Ok),
        Ready::Done(Outcome::Failed) => protocol::write_reply(
            out,
            &Reply::Error {
                message: &"it could not be drawn on every output it was meant for; \
                           `scootbg query` shows what each output shows, and why a draw \
                           failed (`draw_error`, also on the daemon's stderr)",
            },
        ),
        Ready::Refused(message) => protocol::write_reply(out, &Reply::Error { message }),
    }
}

impl Handler for Responder<'_> {
    fn handle(&mut self, conn: ConnId, line: &[u8], out: &mut Vec<u8>) -> Answer {
        let (output, workspace, show, transition) = match protocol::parse(line) {
            Ok(Request::Query) => {
                protocol::write_reply(
                    out,
                    &Reply::Outputs {
                        outputs: self.wallpaper.outputs(),
                        workspaces: self.wallpaper.workspaces(),
                        saving: self.wallpaper.saving(),
                        profile: self.wallpaper.profile(),
                        rotation: self.wallpaper.rotation(),
                    },
                );
                return Answer::Now;
            }
            Ok(Request::Version) => {
                protocol::write_reply(
                    out,
                    &Reply::Version {
                        protocol: PROTOCOL_VERSION,
                        version: env!("CARGO_PKG_VERSION"),
                    },
                );
                return Answer::Now;
            }
            Ok(Request::Kill) => {
                self.stop = true;
                protocol::write_reply(out, &Reply::Ok);
                return Answer::Now;
            }
            Ok(Request::Set {
                show,
                output,
                transition,
            }) => (output, None, Some(show), transition),
            Ok(Request::Clear { output }) => (output, None, None, Spec::none()),
            Ok(Request::SetWorkspace {
                show,
                output,
                workspace,
                transition,
            }) => (output, Some(workspace), Some(show), transition),
            Ok(Request::ClearWorkspace { output, workspace }) => {
                (output, Some(workspace), None, Spec::none())
            }
            Ok(Request::ApplyConfig { profile, section }) => {
                return match self.wallpaper.apply_config(conn, profile, &section) {
                    Ok(()) => Answer::Later,
                    Err(message) => {
                        protocol::write_reply(out, &Reply::Error { message: &message });
                        Answer::Now
                    }
                };
            }
            Err(error) => {
                protocol::write_reply(out, &Reply::Error { message: &error });
                return Answer::Now;
            }
        };
        let result = match &workspace {
            Some(workspace) => self.wallpaper.change_workspace(
                conn,
                output.as_deref(),
                workspace,
                show,
                transition,
            ),
            None => self
                .wallpaper
                .change(conn, output.as_deref(), show, transition),
        };
        match result {
            Ok(()) => Answer::Later,
            Err(error) => {
                let refused = Refused {
                    error,
                    output: output.as_deref(),
                    workspace: workspace.as_deref(),
                };
                protocol::write_reply(out, &Reply::Error { message: &refused });
                Answer::Now
            }
        }
    }
}

/// Each output's `query` entry, borrowed from the model: no allocation.
impl<O> OutputList for Outputs<O> {
    fn for_each_entry(&self, each: &mut dyn FnMut(&OutputEntry<'_>)) {
        for entry in self.iter() {
            let output = &entry.output;
            let info = output.info();
            each(&OutputEntry {
                name: info.name.as_deref(),
                description: info.description.as_deref(),
                mode: info.mode,
                scale: info.scale,
                transform: info.transform.name(),
                logical: output.logical(),
                surface: SurfaceEntry {
                    state: output.surface().name(),
                    size: output.surface_size(),
                    scale: output.surface_size().map(|_| output.scale()),
                    pixels: output.full_buffer().map(|buffer| Size {
                        width: buffer.dims.0,
                        height: buffer.dims.1,
                    }),
                },
                draw_failed: output.has_failed(),
                draw_error: output.failure(),
                shows: output.shows().map(Shows),
                workspace: output.active_workspace(),
                transition: output.running().map(|kind| kind.name()),
            });
        }
    }
}

/// Each live workspace mapping's `query` entry, borrowed from the
/// choices: no allocation.
impl WorkspaceList for Choices {
    fn for_each_entry(&self, each: &mut dyn FnMut(&WorkspaceEntry<'_>)) {
        for (output, workspace, choice, ..) in self.workspaces() {
            let Some(wallpaper) = choice else {
                continue;
            };
            each(&WorkspaceEntry {
                output,
                workspace,
                shows: Shows(wallpaper),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeError;

    /// The workspace cap refusal names the cap: with 64 mappings live the
    /// 65th says how many fit, so a client (or a person reading stderr)
    /// knows to clear one rather than retrying. Each mapping can hold an
    /// output-sized buffer (~33 MB at 4K per size, shared across same-size
    /// outputs), so the bound is the memory bound too.
    #[test]
    fn the_workspace_cap_refusal_names_the_cap() {
        let refused = format!(
            "{}",
            super::Refused {
                error: ChangeError::TooManyWorkspaces,
                output: None,
                workspace: None,
            }
        );
        assert!(
            refused.contains(&crate::choices::MAX_WORKSPACES.to_string()),
            "names the cap: {refused}"
        );
        assert!(refused.contains("clear"), "says the remedy: {refused}");
    }
}
