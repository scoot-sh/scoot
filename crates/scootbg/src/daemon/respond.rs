//! What the daemon answers to each request.

use std::fmt;

use crate::control::{Answer, ConnId, Handler};
use crate::outputs::{Outputs, Size};
use crate::protocol::{
    self, OutputEntry, OutputList, PROTOCOL_VERSION, Reply, Request, Show, Shows, SurfaceEntry,
};
use crate::section::Section;
use crate::state::Profile;
use crate::waiters::Outcome;

/// What the handler needs from the daemon's state: the outputs, for
/// `query`, and a way to change what they show, for `set` and `clear`.
pub trait Changes {
    fn outputs(&self) -> &dyn OutputList;

    /// Whether changes are saved for the next start (`query`'s `saving`).
    fn saving(&self) -> bool;

    /// The profile restored and saved (`query`'s `profile`).
    fn profile(&self) -> &str;

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
    /// `output`, show `show` (nothing when `None`), and registers `conn` to
    /// be answered once they do, or, for an image that cannot be shown,
    /// with why. `Err` changes nothing.
    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        show: Option<Show<'_>>,
    ) -> Result<(), ChangeError>;
}

/// Why a `set` or `clear` was refused; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeError {
    /// No output has that name now.
    UnknownOutput,
    /// Too many images already wait to be decoded (`crate::jobs`).
    Busy,
    /// A download, with nowhere to cache it.
    Cache(String),
}

/// The reply text for a refused change.
struct Refused<'a> {
    error: ChangeError,
    output: Option<&'a str>,
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
        let (output, show) = match protocol::parse(line) {
            Ok(Request::Query) => {
                protocol::write_reply(
                    out,
                    &Reply::Outputs {
                        outputs: self.wallpaper.outputs(),
                        saving: self.wallpaper.saving(),
                        profile: self.wallpaper.profile(),
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
            Ok(Request::Set { show, output }) => (output, Some(show)),
            Ok(Request::Clear { output }) => (output, None),
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
        match self.wallpaper.change(conn, output.as_deref(), show) {
            Ok(()) => Answer::Later,
            Err(error) => {
                let refused = Refused {
                    error,
                    output: output.as_deref(),
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
            });
        }
    }
}
