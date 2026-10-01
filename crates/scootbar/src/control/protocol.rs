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
/// {"protocol":1,"type":"query","id":"status"}
/// {"protocol":1,"type":"layout"}
/// {"protocol":1,"type":"invoke","id":"volume","action":"raise","arg":5,"output":"DP-1"}
/// {"protocol":1,"type":"subscribe","events":["module","output"]}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    /// Each placed module's current state, as JSON; only module `id`'s when
    /// one is named.
    Query { id: Option<Cow<'a, str>> },
    /// Each placed module's rectangle on every output, as last drawn.
    Layout,
    /// Run module `id`'s action `action` (a module-defined action, or a
    /// trigger: `click`, `right-click`, `middle-click`, `scroll-up`,
    /// `scroll-down`, which runs the configured binding) exactly as a click
    /// or scroll would, with `arg` (a whole number, a scroll's steps) on
    /// `output` (by default the first that shows the module).
    Invoke {
        id: Cow<'a, str>,
        action: Cow<'a, str>,
        arg: Option<i32>,
        output: Option<Cow<'a, str>>,
    },
    /// Dedicate this connection to events of these kinds.
    Subscribe { events: Vec<EventKind> },
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
    /// A value for module `id` ([`crate::modules::Module::on_set`]): only a
    /// `push` module takes one; any other is refused loudly.
    Set { id: Cow<'a, str>, value: Value },
}

/// What a subscriber can ask to be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// A module's view changed (`{"type":"module", ...}`, as `query` lists it).
    Module,
    /// An output was added or removed (`{"type":"output", ...}`).
    Output,
}

impl EventKind {
    pub const ALL: [Self; 2] = [Self::Module, Self::Output];

    pub fn name(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Output => "output",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

impl Request<'_> {
    /// The `type` string on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Query { .. } => "query",
            Self::Layout => "layout",
            Self::Invoke { .. } => "invoke",
            Self::Subscribe { .. } => "subscribe",
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
            #[serde(skip_serializing_if = "Option::is_none")]
            action: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            arg: Option<i32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            output: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            events: Option<Vec<&'static str>>,
        }
        let mut line = Line {
            protocol: PROTOCOL_VERSION,
            kind: self.name(),
            id: None,
            value: None,
            action: None,
            arg: None,
            output: None,
            events: None,
        };
        match self {
            Self::Set { id, value } => {
                line.id = Some(id.as_ref());
                line.value = Some(value);
            }
            Self::Query { id } => line.id = id.as_deref(),
            Self::Invoke {
                id,
                action,
                arg,
                output,
            } => {
                line.id = Some(id.as_ref());
                line.action = Some(action.as_ref());
                line.arg = *arg;
                line.output = output.as_deref();
            }
            Self::Subscribe { events } => {
                line.events = Some(events.iter().map(|kind| kind.name()).collect());
            }
            Self::Layout
            | Self::Reload
            | Self::Hide
            | Self::Show
            | Self::Toggle
            | Self::Kill
            | Self::Version => {}
        }
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
    /// `invoke` without an `action`.
    NoAction,
    /// `invoke`'s `arg` is not a whole number that fits.
    BadArg,
    /// `subscribe` names a kind that does not exist.
    UnknownEvent(String),
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
                "`set` and `invoke` need an `id` (a module id, as `query` lists them)"
            ),
            Self::NoValue => write!(f, "`set` needs a `value` (a JSON value for the module)"),
            Self::NoAction => write!(
                f,
                "`invoke` needs an `action` (a module's action, or click, right-click, \
                 middle-click, scroll-up or scroll-down)"
            ),
            Self::BadArg => write!(f, "`arg` takes a whole number"),
            Self::UnknownEvent(kind) => write!(
                f,
                "unknown event kind `{}` (they are: module, output)",
                kind.escape_debug()
            ),
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
    /// Present even when `null` (a `push` module's way to clear): serde
    /// would read `"value": null` as absent into an `Option<Value>`.
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
    #[serde(borrow)]
    action: Option<Cow<'a, str>>,
    /// Any JSON number: whether it is a whole number that fits is checked,
    /// so a float or a huge one is refused by name, not as malformed.
    arg: Option<Value>,
    #[serde(borrow)]
    output: Option<Cow<'a, str>>,
    events: Option<Vec<String>>,
}

/// A value that is there, whatever it is, `null` included.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
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
        "query" => Ok(Request::Query { id: envelope.id }),
        "layout" => Ok(Request::Layout),
        "invoke" => {
            let id = envelope.id.ok_or(RequestError::NoId)?;
            let action = envelope.action.ok_or(RequestError::NoAction)?;
            let arg = match envelope.arg {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_i64()
                        .and_then(|n| i32::try_from(n).ok())
                        .ok_or(RequestError::BadArg)?,
                ),
            };
            Ok(Request::Invoke {
                id,
                action,
                arg,
                output: envelope.output,
            })
        }
        "subscribe" => {
            let mut events = Vec::new();
            match envelope.events {
                // None named: every kind (a bare `scootbar msg subscribe`).
                None => events.extend(EventKind::ALL),
                Some(names) if names.is_empty() => events.extend(EventKind::ALL),
                Some(names) => {
                    for name in names {
                        let kind =
                            EventKind::parse(&name).ok_or(RequestError::UnknownEvent(name))?;
                        if !events.contains(&kind) {
                            events.push(kind);
                        }
                    }
                }
            }
            Ok(Request::Subscribe { events })
        }
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
    /// Its tooltip, where it has one (absent while empty).
    #[serde(skip_serializing_if = "is_empty")]
    pub tooltip: &'a str,
    /// What it holds that is not text, where it has something (the
    /// workspaces module: `{"active": 2, "workspaces": [1, 2, 3]}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// One rectangle, logical pixels in the compositor's global space (the
/// space `scoot msg pointer` takes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// One module's place on one output's bar.
#[derive(Debug, Serialize)]
pub struct PlacedRect<'a> {
    pub id: &'a str,
    pub section: &'static str,
    #[serde(flatten)]
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// An event on a subscribed connection.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event<'a> {
    /// A module's view changed, shown as `query` lists it.
    Module(ModuleView<'a>),
    /// An output came or went.
    Output {
        change: &'static str,
        name: Option<&'a str>,
    },
}

/// Appends `event` and its newline to `out`.
pub fn write_event(out: &mut Vec<u8>, event: &Event<'_>) {
    let start = out.len();
    if serde_json::to_writer(&mut *out, event).is_err() {
        out.truncate(start);
        return;
    }
    out.push(b'\n');
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
    /// The answer to `subscribe`: the kinds this connection now carries.
    Subscribed {
        events: Vec<&'static str>,
    },
    Error {
        #[serde(serialize_with = "display")]
        message: &'a dyn fmt::Display,
    },
}

/// For `skip_serializing_if`, which hands over a reference to the field.
fn is_empty(text: &&str) -> bool {
    text.is_empty()
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
