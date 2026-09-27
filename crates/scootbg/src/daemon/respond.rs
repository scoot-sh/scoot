//! What the daemon answers to each request.

use std::fmt;

use crate::choices::Choice;
use crate::control::{Answer, ConnId, Handler};
use crate::outputs::Outputs;
use crate::protocol::{
    self, OutputEntry, OutputList, PROTOCOL_VERSION, Reply, Request, Shows, SurfaceEntry,
};
use crate::waiters::Outcome;

/// What the handler needs from the daemon's state: the outputs, for
/// `query`, and a way to change what they show, for `set` and `clear`.
pub trait Wallpaper {
    fn outputs(&self) -> &dyn OutputList;

    /// Makes every output (`output` is `None`), or the outputs named
    /// `output`, show `choice`, and registers `conn` to be answered once
    /// they do. `Err` changes nothing.
    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        choice: Choice,
    ) -> Result<(), ChangeError>;
}

/// Why a `set` or `clear` was refused; nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeError {
    /// No output has that name now.
    UnknownOutput,
}

/// The reply text for a refused change.
struct Refused<'a> {
    error: ChangeError,
    output: Option<&'a str>,
}

impl fmt::Display for Refused<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.error {
            ChangeError::UnknownOutput => write!(
                f,
                "no output is named {:?} (`scootbg query` lists them); nothing was changed",
                self.output.unwrap_or_default()
            ),
        }
    }
}

/// The request handler for one round of the poll loop.
pub struct Responder<'a> {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
    wallpaper: &'a mut dyn Wallpaper,
}

impl<'a> Responder<'a> {
    pub fn new(wallpaper: &'a mut dyn Wallpaper) -> Self {
        Self {
            stop: false,
            wallpaper,
        }
    }
}

/// The reply to a `set` or `clear` that waited.
pub fn write_outcome(out: &mut Vec<u8>, outcome: Outcome) {
    match outcome {
        Outcome::Shown => protocol::write_reply(out, &Reply::Ok),
        Outcome::Failed => protocol::write_reply(
            out,
            &Reply::Error {
                message: &"the color could not be drawn on every output it was meant for \
                           (the daemon's stderr says why); `scootbg query` shows what each \
                           output shows",
            },
        ),
    }
}

impl Handler for Responder<'_> {
    fn handle(&mut self, conn: ConnId, line: &[u8], out: &mut Vec<u8>) -> Answer {
        let (output, choice) = match protocol::parse(line) {
            Ok(Request::Query) => {
                protocol::write_reply(
                    out,
                    &Reply::Outputs {
                        outputs: self.wallpaper.outputs(),
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
            Ok(Request::Set { color, output }) => (output, Some(color)),
            Ok(Request::Clear { output }) => (output, None),
            Err(error) => {
                protocol::write_reply(out, &Reply::Error { message: &error });
                return Answer::Now;
            }
        };
        match self.wallpaper.change(conn, output.as_deref(), choice) {
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
                },
                shows: output.shows().map(|color| Shows { color }),
            });
        }
    }
}
