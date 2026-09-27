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
//! {"type":"outputs","outputs":[{"name":"DP-1",...}]}
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

use crate::outputs::Size;

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
    /// Valid JSON, but not an object: the derived `Deserialize` would also
    /// read the struct from an array such as `[1, "kill"]`, which is not
    /// this protocol.
    NotAnObject,
    NoProtocol,
    WrongProtocol(u32),
    NoType,
    Unknown(String),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed request: {error}"),
            Self::NotAnObject => write!(f, "a request is one JSON object"),
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
    // JSON whitespace is these four bytes; an object is the only value
    // that starts with `{`. Anything else is refused before serde, which
    // would otherwise accept the array form of the struct.
    let first = line
        .iter()
        .find(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    if first != Some(&b'{') {
        // Keep "malformed" for what is not JSON at all (an empty line
        // included), so the message says what is wrong.
        return Err(
            match serde_json::from_slice::<serde::de::IgnoredAny>(line) {
                Err(error) => RequestError::Malformed(error),
                Ok(_) => RequestError::NotAnObject,
            },
        );
    }
    let envelope: Envelope<'_> = serde_json::from_slice(line).map_err(RequestError::Malformed)?;
    match envelope.protocol {
        None => return Err(RequestError::NoProtocol),
        Some(PROTOCOL_VERSION) => {}
        Some(other) => return Err(RequestError::WrongProtocol(other)),
    }
    let kind = envelope.kind.ok_or(RequestError::NoType)?;
    Request::from_name(&kind).ok_or_else(|| RequestError::Unknown(kind.into_owned()))
}

/// One output in a `query` reply, borrowed from the daemon's state so a
/// reply allocates nothing beyond the connection's output buffer:
///
/// ```text
/// {"name":"DP-1","description":"Dell U2720Q (DP-1)",
///  "mode":{"width":3840,"height":2160},"scale":2,"transform":"normal",
///  "logical":{"width":1920,"height":1080},
///  "surface":{"state":"configured","size":{"width":1920,"height":1080}},
///  "shows":null}
/// ```
///
/// Every key is always present, `null` when not known (yet). New keys may
/// be added; none is removed or changes meaning within a protocol version.
#[derive(Debug, Serialize)]
pub struct OutputEntry<'a> {
    /// The connector name (`wl_output` v4, else `xdg-output`).
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    /// The current mode, in device pixels.
    pub mode: Option<Size>,
    /// `wl_output`'s integer scale.
    pub scale: u32,
    /// `wl_output.transform`: `normal`, `90`, `180`, `270`, `flipped`,
    /// `flipped-90`, ... Rotations count counter-clockwise, as the
    /// protocol does (sway's `transform 90`, clockwise, reports `270`).
    pub transform: &'a str,
    /// The output's size in logical pixels, as well as it is known (see
    /// `Output::logical`): exact once the surface is configured; before
    /// that, at a fractional scale, possibly too small.
    pub logical: Option<Size>,
    pub surface: SurfaceEntry,
    /// What the output shows: `null`, nothing, until colours and images
    /// land (solid-colour.md fills this in).
    pub shows: Option<Nothing>,
}

/// Where an output's wallpaper surface stands: `state` is one of
/// `waiting` (the output has not reported itself yet), `pending` (asked
/// for, not yet sized), `configured`, `closed` (closed by the compositor,
/// being re-created) and `gave-up` (closed twice; not tried again until
/// the output is replugged).
/// `size`, in logical pixels, is set only while `configured`, and even
/// then `null` if the compositor left the size to scootbg before the
/// output reported a mode.
#[derive(Debug, Serialize)]
pub struct SurfaceEntry {
    pub state: &'static str,
    pub size: Option<Size>,
}

/// Nothing is drawn yet, so `shows` has no value but `null`.
#[derive(Debug, Serialize)]
pub enum Nothing {}

/// The daemon's outputs, as a `query` reply lists them.
pub trait OutputList {
    /// Calls `each` with every output's entry, in order.
    fn for_each_entry(&self, each: &mut dyn FnMut(&OutputEntry<'_>));
}

/// For fixed lists, as in tests (a slice cannot be a `dyn` value).
impl<const N: usize> OutputList for [OutputEntry<'_>; N] {
    fn for_each_entry(&self, each: &mut dyn FnMut(&OutputEntry<'_>)) {
        self.iter().for_each(each);
    }
}

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
        #[serde(serialize_with = "list")]
        outputs: &'a dyn OutputList,
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

/// Serializes an [`OutputList`] as a JSON array, entry by entry.
fn list<S: Serializer>(value: &&dyn OutputList, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(None)?;
    let mut failed = None;
    value.for_each_entry(&mut |entry| {
        if failed.is_none() {
            failed = seq.serialize_element(entry).err();
        }
    });
    match failed {
        Some(error) => Err(error),
        None => seq.end(),
    }
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
