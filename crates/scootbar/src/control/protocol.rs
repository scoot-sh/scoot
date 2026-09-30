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
//! {"type":"modules","modules":[{"id":"clock","section":"center","output":"DP-1","text":"3:07 pm","class":"normal"}]}
//! {"type":"version","protocol":1,"version":"0.1.0"}
//! {"type":"ok"}
//! {"type":"bar","visible":false}
//! {"type":"error","message":"..."}
//! ```
//!
//! A request whose `protocol` differs from [`PROTOCOL_VERSION`] is refused
//! with an error reply, so a client and a daemon from different builds
//! disagree loudly rather than misread each other. Unknown fields are
//! ignored, which leaves room for the arguments later requests carry.
//!
//! Parsing borrows from the line and allocates nothing unless a string in
//! it contains escapes; replies are written straight into the
//! connection's reused output buffer.
//!
//! (The shape follows scootbg's `protocol.rs`: line framing, versioned
//! envelope, borrowed parse, buffered replies. The requests are the bar's
//! own.)

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

/// Bumped whenever a wire change would make an old client or daemon
/// misread a new one.
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line accepted, newline excluded. Requests are small
/// (the largest, `set`, carries one JSON value); anything longer is
/// refused and the connection closed, so a client cannot make the daemon
/// buffer without bound.
pub const MAX_REQUEST_LINE: usize = 64 * 1024;

/// What a client can ask for.
///
/// ```text
/// {"protocol":1,"type":"query"}
/// {"protocol":1,"type":"reload"}
/// {"protocol":1,"type":"hide"}
/// {"protocol":1,"type":"show"}
/// {"protocol":1,"type":"toggle"}
/// {"protocol":1,"type":"version"}
/// {"protocol":1,"type":"kill"}
/// {"protocol":1,"type":"set","id":"clock","value":{...}}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    /// Each placed module's current state, as JSON.
    Query,
    /// Re-read the config file and live-apply it.
    Reload,
    /// Destroy every bar surface and buffer, releasing the exclusive zone.
    Hide,
    /// Make the bars again.
    Show,
    /// [`Request::Hide`] if shown, [`Request::Show`] if hidden.
    Toggle,
    /// Stop the daemon.
    Kill,
    /// The daemon's version and protocol.
    Version,
    /// A value for module `id` ([`crate::modules::Module::on_set`]): no
    /// module takes one yet, so this is refused loudly for every id today,
    /// and is the forward hook for the modules that will.
    Set { id: Cow<'a, str>, value: Value },
}

impl Request<'_> {
    /// The `type` string on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Reload => "reload",
            Self::Hide => "hide",
            Self::Show => "show",
            Self::Toggle => "toggle",
            Self::Kill => "kill",
            Self::Version => "version",
            Self::Set { .. } => "set",
        }
    }

    /// The request line a client sends, newline included. Strings are
    /// JSON-escaped, so any id round-trips; the value goes over raw.
    pub fn line(&self) -> String {
        #[derive(Serialize)]
        struct Line<'r> {
            protocol: u32,
            #[serde(rename = "type")]
            kind: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            id: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            value: Option<&'r Value>,
        }
        let (id, value) = match self {
            Self::Set { id, value } => (Some(id.as_ref()), Some(value)),
            Self::Query
            | Self::Reload
            | Self::Hide
            | Self::Show
            | Self::Toggle
            | Self::Kill
            | Self::Version => (None, None),
        };
        let line = Line {
            protocol: PROTOCOL_VERSION,
            kind: self.name(),
            id,
            value,
        };
        // Serializing strings and numbers into a `String` cannot fail; if
        // it ever did, the empty line gets a "malformed" reply, not a panic.
        let mut text = serde_json::to_string(&line).unwrap_or_default();
        text.push('\n');
        text
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
    /// `set` without an `id`.
    NoId,
    /// `set` without a `value`.
    NoValue,
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
            Self::NoId => write!(
                f,
                "`set` needs an `id` (a module id, as `query` lists them)"
            ),
            Self::NoValue => write!(f, "`set` needs a `value` (a JSON value for the module)"),
        }
    }
}

/// The fields a request may have. Strings borrow from the line when they
/// have no escapes; any other field is ignored.
#[derive(Deserialize)]
struct Envelope<'a> {
    protocol: Option<u32>,
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    value: Option<Value>,
}

/// Parses one request line (without its newline).
pub fn parse(line: &[u8]) -> Result<Request<'_>, RequestError> {
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
    match &*kind {
        "query" => Ok(Request::Query),
        "reload" => Ok(Request::Reload),
        "hide" => Ok(Request::Hide),
        "show" => Ok(Request::Show),
        "toggle" => Ok(Request::Toggle),
        "kill" => Ok(Request::Kill),
        "version" => Ok(Request::Version),
        "set" => {
            let id = envelope.id.ok_or(RequestError::NoId)?;
            let value = envelope.value.ok_or(RequestError::NoValue)?;
            Ok(Request::Set { id, value })
        }
        _ => Err(RequestError::Unknown(kind.into_owned())),
    }
}

/// One placed module's state on one output, as a `query` reply lists it.
/// Every key but `icon` is always present, `null` when not known (yet):
/// `output` is null where the compositor never named the output. `icon`
/// is absent where the module shows none. New keys may be added; none is
/// removed or changes meaning within a protocol version.
#[derive(Debug, Serialize)]
pub struct ModuleView<'a> {
    /// The module id, as `--left`/`--center`/`--right` (and `query`) name
    /// it.
    pub id: &'a str,
    /// Where it is placed: `left`, `center` or `right`.
    pub section: &'static str,
    /// `wl_output.name` (`DP-1`), if the compositor sent one.
    pub output: Option<&'a str>,
    /// What it shows.
    pub text: &'a str,
    /// Its state class: `normal`, `warn`, `urgent` or `muted`.
    pub class: &'static str,
    /// A glyph shown before the text, if it shows one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<char>,
}

/// A reply line.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Reply<'a> {
    Ok,
    /// Whether the bars are shown now, after a `hide`, `show` or `toggle`.
    Bar {
        visible: bool,
    },
    Version {
        protocol: u32,
        version: &'a str,
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
