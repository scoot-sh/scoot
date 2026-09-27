//! What the daemon answers to each request.

use crate::control::Handler;
use crate::outputs::Outputs;
use crate::protocol::{
    self, OutputEntry, OutputList, PROTOCOL_VERSION, Reply, Request, SurfaceEntry,
};

/// The request handler for one round of the poll loop: borrows what a
/// reply needs from the daemon's state.
pub struct Responder<'a> {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
    outputs: &'a dyn OutputList,
}

impl<'a> Responder<'a> {
    pub fn new(outputs: &'a dyn OutputList) -> Self {
        Self {
            stop: false,
            outputs,
        }
    }
}

impl Handler for Responder<'_> {
    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>) {
        match protocol::parse(line) {
            Ok(Request::Query) => {
                protocol::write_reply(
                    out,
                    &Reply::Outputs {
                        outputs: self.outputs,
                    },
                );
            }
            Ok(Request::Version) => protocol::write_reply(
                out,
                &Reply::Version {
                    protocol: PROTOCOL_VERSION,
                    version: env!("CARGO_PKG_VERSION"),
                },
            ),
            Ok(Request::Kill) => {
                self.stop = true;
                protocol::write_reply(out, &Reply::Ok);
            }
            Err(error) => protocol::write_reply(out, &Reply::Error { message: &error }),
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
                shows: None,
            });
        }
    }
}
