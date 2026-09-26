//! The control protocol: one JSON object per line, each way.
//!
//! A request names the protocol it speaks and what it asks for:
//!
//! ```text
//! {"protocol":1,"type":"query"}
//! ```
//!
//! and gets exactly one reply line, an object tagged by `type`:
//!
//! ```text
//! {"type":"outputs","outputs":[]}
//! {"type":"version","protocol":1,"version":"0.1.0"}
//! {"type":"ok"}
//! {"type":"error","message":"..."}
//! ```
//!
//! A request whose `protocol` differs from [`PROTOCOL_VERSION`] is refused
//! with an error reply, so a client and a daemon from different builds
//! disagree loudly rather than misread each other. Unknown fields are
//! ignored, which leaves room for the arguments later requests carry.
//!
//! Parsing borrows from the line and allocates nothing unless the `type`
//! string contains escapes; replies are written straight into the
//! connection's reused output buffer.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};

#[cfg(test)]
mod tests;

/// Bumped whenever a wire change would make an old client or daemon
/// misread a new one.
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line accepted, newline excluded. Requests are small
/// (the largest planned, `apply-config`, carries one config section with
/// paths); anything longer is refused and the connection closed, so a
/// client cannot make the daemon buffer without bound.
pub const MAX_REQUEST_LINE: usize = 64 * 1024;

/// What a client can ask for. The wallpaper-changing requests (`set`,
/// `clear`, `apply-config`) arrive with the CLI ticket
/// (docs/scootbg/backlog/cli-and-ipc.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// What each output shows.
    Query,
    /// Stop the daemon.
    Kill,
    /// The daemon's version and protocol.
    Version,
}

impl Request {
    /// The `type` string on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Kill => "kill",
            Self::Version => "version",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "query" => Some(Self::Query),
            "kill" => Some(Self::Kill),
            "version" => Some(Self::Version),
            _ => None,
        }
    }

    /// The request line a client sends, newline included.
    pub fn line(self) -> String {
        format!(
            "{{\"protocol\":{PROTOCOL_VERSION},\"type\":\"{}\"}}\n",
            self.name()
        )
    }
}

/// Why a request line was refused. Each becomes an error reply; the
/// connection stays open (the next line is still a request).
#[derive(Debug)]
pub enum RequestError {
    Malformed(serde_json::Error),
    NoProtocol,
    WrongProtocol(u32),
    NoType,
    Unknown(String),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed request: {error}"),
            Self::NoProtocol => write!(f, "request has no `protocol` field"),
            Self::WrongProtocol(got) => write!(
                f,
                "protocol mismatch: the request speaks {got}, this daemon speaks \
                 {PROTOCOL_VERSION}"
            ),
            Self::NoType => write!(f, "request has no `type` field"),
            Self::Unknown(name) => write!(f, "unknown request `{name}`"),
        }
    }
}

/// The fields every request has. `type` borrows from the line when it has
/// no escapes; any other field is ignored.
#[derive(Deserialize)]
struct Envelope<'a> {
    protocol: Option<u32>,
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
}

/// Parses one request line (without its newline).
pub fn parse(line: &[u8]) -> Result<Request, RequestError> {
    let envelope: Envelope<'_> = serde_json::from_slice(line).map_err(RequestError::Malformed)?;
    match envelope.protocol {
        None => return Err(RequestError::NoProtocol),
        Some(PROTOCOL_VERSION) => {}
        Some(other) => return Err(RequestError::WrongProtocol(other)),
    }
    let kind = envelope.kind.ok_or(RequestError::NoType)?;
    Request::from_name(&kind).ok_or_else(|| RequestError::Unknown(kind.into_owned()))
}

/// One output in a `query` reply. There are none yet: output tracking
/// arrives with docs/scootbg/backlog/outputs-and-layer-surfaces.md, which
/// gives this its fields. The reply's shape, a list, is fixed now.
#[derive(Debug, Serialize)]
pub enum OutputEntry {}

/// A reply line.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Reply<'a> {
    Ok,
    Version {
        protocol: u32,
        version: &'a str,
    },
    Outputs {
        outputs: &'a [OutputEntry],
    },
    Error {
        #[serde(serialize_with = "display")]
        message: &'a dyn fmt::Display,
    },
}

/// Serializes through `Display` without an intermediate `String`.
fn display<S: Serializer>(value: &&dyn fmt::Display, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(*value)
}

/// Appends `reply` and its newline to `out`.
pub fn write_reply(out: &mut Vec<u8>, reply: &Reply<'_>) {
    let start = out.len();
    if serde_json::to_writer(&mut *out, reply).is_err() {
        // Writing into a `Vec` cannot fail and every field serializes, so
        // this is unreachable; if it ever happened, a fixed line beats a
        // half-written one.
        out.truncate(start);
        out.extend_from_slice(br#"{"type":"error","message":"internal: reply failed to encode"}"#);
    }
    out.push(b'\n');
}
