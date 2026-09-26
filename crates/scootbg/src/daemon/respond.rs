//! What the daemon answers to each request.

use crate::control::Handler;
use crate::protocol::{self, OutputEntry, PROTOCOL_VERSION, Reply, Request};

/// The request handler for the daemon's connections. Holds what a reply
/// needs from the daemon (nothing yet but whether to stop; the output list
/// arrives with outputs-and-layer-surfaces.md).
#[derive(Debug, Default)]
pub struct Responder {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
}

impl Handler for Responder {
    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>) {
        match protocol::parse(line) {
            Ok(Request::Query) => {
                const NONE: &[OutputEntry] = &[];
                protocol::write_reply(out, &Reply::Outputs { outputs: NONE });
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
