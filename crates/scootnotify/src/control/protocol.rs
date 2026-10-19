//! The control protocol: one JSON object per line, each way.
//!
//! A request names the protocol it speaks and what it asks for:
//!
//! ```text
//! {"protocol":1,"type":"list"}
//! ```
//!
//! and gets exactly one reply line, an object tagged by `type`:
//!
//! ```text
//! {"type":"notifications","notifications":[{...}]}
//! {"type":"history","history":[{...}]}
//! {"type":"dnd","dnd":"on"}
//! {"type":"version","protocol":2,"version":"0.3.1"}
//! {"type":"subscribed","events":["notify","close"]}
//! {"type":"ok"}
//! {"type":"ok","scope":"look"}
//! {"type":"error","message":"..."}
//! ```
//!
//! A `reload` re-reads the config file and the look layer, and answers
//! applied or refused; with `"scope":"look"` only the look layer is
//! re-read, under no user file at all, so a look swap never applies a
//! half-edited one. Any other scope is refused by name, never silently
//! treated as a plain reload (which would apply those edits).
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
//! (The shape follows scootbar's `control/protocol.rs`: line framing, a
//! versioned envelope, borrowed parse, buffered replies. The requests are
//! the notifier's own. There is deliberately no `reload-look` verb: that
//! verb exists so an old bar never misreads a scoped reload as a plain
//! one, and this socket has no old daemons.)

use std::borrow::Cow;
use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::bus::hints::Urgency;
use crate::model::history::HistoryEntry;
use crate::model::store::{CloseReason, Notification};

/// Bumped whenever a wire change would make an old client or daemon
/// misread a new one. Compatibility rides on this number, never on the
/// crate version. 2 adds `dnd`, `history` and `restore` (with the `dnd`
/// event and reply): a version-1 client asking a version-2 daemon gets
/// its old replies byte-identical, and a version mismatch is refused by
/// name either way.
pub const PROTOCOL_VERSION: u32 = 2;

/// What a client can ask for.
///
/// ```text
/// {"protocol":2,"type":"list"}
/// {"protocol":2,"type":"dismiss","id":3}
/// {"protocol":2,"type":"dismiss","all":true}
/// {"protocol":2,"type":"invoke","id":3}
/// {"protocol":2,"type":"invoke","id":3,"action":"default"}
/// {"protocol":2,"type":"reload"}
/// {"protocol":2,"type":"reload","scope":"look"}
/// {"protocol":2,"type":"subscribe","events":["notify","close"]}
/// {"protocol":2,"type":"dnd"}
/// {"protocol":2,"type":"dnd","mode":"toggle"}
/// {"protocol":2,"type":"history"}
/// {"protocol":2,"type":"restore"}
/// {"protocol":2,"type":"version"}
/// ```
///
/// (`wait` is not a verb: `scootnotify msg wait` subscribes, reads the
/// baseline `list`, and matches client-side.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    /// The live notifications, oldest first, as JSON.
    List,
    /// Close one card (`id`), or every live card (`all`), with reason 2,
    /// as a click would. Unknown ids are refused loudly, never a silent
    /// ok. A request carrying both an `id` and `"all":true` (only over
    /// the raw socket; the CLI refuses the combination) dismisses that id.
    Dismiss { id: Option<u32>, all: bool },
    /// Run card `id`'s `action` as a click would (`default` when absent),
    /// emitting `ActionInvoked` and closing with reason 2 unless the card
    /// is `resident`. Unknown ids and unknown keys are refused loudly.
    Invoke { id: u32, action: Cow<'a, str> },
    /// Re-read the config file and the look layer and live-apply them.
    /// Without a scope both are re-read; with `"look"` only the look
    /// layer is, and the user file is never touched.
    Reload { scope: Option<ReloadScope> },
    /// Dedicate this connection to events of these kinds.
    Subscribe { events: Vec<EventKind> },
    /// The DND mode: read it (no `mode`), or set it (`on`, `off`,
    /// `toggle`). Answers — and, to subscribers, emits — the mode.
    Dnd { mode: Option<DndMode> },
    /// The history ring, oldest first, as JSON.
    History,
    /// Re-show the newest history entry as a new card with a new id.
    /// Refused loudly when history is empty.
    Restore,
    /// The daemon's version and protocol.
    Version,
}

/// How one `dnd` request steers the mode: set it (`on`, `off`), flip it
/// (`toggle`), or read it (absent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndMode {
    On,
    Off,
    Toggle,
}

impl DndMode {
    /// The `mode` string on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Toggle => "toggle",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            "toggle" => Some(Self::Toggle),
            _ => None,
        }
    }
}

/// A `reload`'s scope: what it re-reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadScope {
    /// Only the look layer: a look swap that never applies pending edits
    /// to the user file.
    Look,
}

impl ReloadScope {
    /// The `scope` string on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Look => "look",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "look" => Some(Self::Look),
            _ => None,
        }
    }
}

/// What a subscriber can ask to be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// A notification arrived
    /// (`{"type":"notify","notification":{...}}`, the object as `list`
    /// shows it).
    Notify,
    /// A card closed
    /// (`{"type":"close","id":N,"reason":"expired|dismissed|closed|evicted"}`).
    Close,
    /// An action was invoked through `msg invoke` or a click
    /// (`{"type":"invoke","id":N,"action":"..."}`).
    Invoke,
    /// DND changed (`{"type":"dnd","dnd":"on|off"}`).
    Dnd,
}

impl EventKind {
    pub const ALL: [Self; 4] = [Self::Notify, Self::Close, Self::Invoke, Self::Dnd];

    pub fn name(self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::Close => "close",
            Self::Invoke => "invoke",
            Self::Dnd => "dnd",
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
            Self::List => "list",
            Self::Dismiss { .. } => "dismiss",
            Self::Invoke { .. } => "invoke",
            Self::Reload { .. } => "reload",
            Self::Subscribe { .. } => "subscribe",
            Self::Dnd { .. } => "dnd",
            Self::History => "history",
            Self::Restore => "restore",
            Self::Version => "version",
        }
    }

    /// The request line a client sends, newline included. Strings are
    /// JSON-escaped, so any action round-trips.
    pub fn line(&self) -> String {
        #[derive(Serialize)]
        struct Line<'r> {
            protocol: u32,
            #[serde(rename = "type")]
            kind: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            id: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            all: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            action: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            scope: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            mode: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            events: Option<Vec<&'static str>>,
        }
        let mut line = Line {
            protocol: PROTOCOL_VERSION,
            kind: self.name(),
            id: None,
            all: None,
            action: None,
            scope: None,
            mode: None,
            events: None,
        };
        match self {
            Self::List | Self::Version | Self::History | Self::Restore => {}
            Self::Dismiss { id, all } => {
                line.id = *id;
                line.all = (*all).then_some(true);
            }
            Self::Invoke { id, action } => {
                line.id = Some(*id);
                line.action = Some(action.as_ref());
            }
            Self::Reload { scope } => {
                line.scope = scope.map(|scope| scope.name());
            }
            Self::Dnd { mode } => {
                line.mode = mode.map(|mode| mode.name());
            }
            Self::Subscribe { events } => {
                line.events = Some(events.iter().map(|kind| kind.name()).collect());
            }
        }
        // Serializing numbers and strings into a `String` cannot fail; if
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
    /// read the struct from an array such as `[1, "list"]`, which is not
    /// this protocol.
    NotAnObject,
    NoProtocol,
    WrongProtocol(u32),
    NoType,
    Unknown(String),
    /// `dismiss` without an `id` (and without `"all":true`), or `invoke`
    /// without an `id`.
    NoId,
    /// An `id` that is not a whole number that fits a `u32`.
    BadId,
    /// `reload` names a scope that does not exist.
    UnknownScope(String),
    /// `subscribe` names a kind that does not exist.
    UnknownEvent(String),
    /// `dnd` names a mode that does not exist.
    UnknownMode(String),
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
                "`dismiss` needs an `id` (or `\"all\":true`), `invoke` needs an `id`"
            ),
            Self::BadId => write!(f, "`id` takes a whole number from 1 to 4294967295"),
            Self::UnknownScope(scope) => write!(
                f,
                "unknown reload scope `{}` (the scopes are: look)",
                scope.escape_debug()
            ),
            Self::UnknownEvent(kind) => write!(
                f,
                "unknown event kind `{}` (they are: notify, close, invoke, dnd)",
                kind.escape_debug()
            ),
            Self::UnknownMode(mode) => write!(
                f,
                "unknown dnd mode `{}` (it takes: on, off, toggle, or nothing to read)",
                mode.escape_debug()
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
    /// Any JSON number: whether it is a whole number that fits is checked,
    /// so a float or a huge one is refused by name, not as malformed.
    id: Option<Value>,
    all: Option<Value>,
    #[serde(borrow)]
    action: Option<Cow<'a, str>>,
    /// A `reload`'s scope, absent for a plain reload.
    #[serde(borrow)]
    scope: Option<Cow<'a, str>>,
    /// A `dnd` request's mode, absent to read the mode.
    #[serde(borrow)]
    mode: Option<Cow<'a, str>>,
    events: Option<Vec<String>>,
}

/// Reads a `u32` id from its JSON value: whole numbers that fit, nothing
/// else (no floats, no strings, no negatives).
fn read_id(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|id| *id != 0)
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
        "list" => Ok(Request::List),
        "dismiss" => {
            let all = matches!(envelope.all, Some(Value::Bool(true)));
            match envelope.id {
                Some(value) => Ok(Request::Dismiss {
                    id: Some(read_id(&value).ok_or(RequestError::BadId)?),
                    all,
                }),
                None if all => Ok(Request::Dismiss { id: None, all }),
                None => Err(RequestError::NoId),
            }
        }
        "invoke" => {
            let id = match envelope.id {
                Some(value) => read_id(&value).ok_or(RequestError::BadId)?,
                None => return Err(RequestError::NoId),
            };
            // Absent is `default` (what `msg invoke ID` sends); an
            // unknown key is the daemon's to refuse, with the card's keys
            // named.
            Ok(Request::Invoke {
                id,
                action: envelope.action.unwrap_or(Cow::Borrowed("default")),
            })
        }
        "reload" => {
            let scope = match envelope.scope.as_deref() {
                None => None,
                Some(name) => Some(
                    ReloadScope::parse(name)
                        .ok_or_else(|| RequestError::UnknownScope(name.to_owned()))?,
                ),
            };
            Ok(Request::Reload { scope })
        }
        "subscribe" => {
            let mut events = Vec::new();
            match envelope.events {
                // None named: every kind (a bare `scootnotify msg subscribe`).
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
        "dnd" => {
            let mode = match envelope.mode.as_deref() {
                None => None,
                Some(name) => Some(
                    DndMode::parse(name)
                        .ok_or_else(|| RequestError::UnknownMode(name.to_owned()))?,
                ),
            };
            Ok(Request::Dnd { mode })
        }
        "history" => Ok(Request::History),
        "restore" => Ok(Request::Restore),
        "version" => Ok(Request::Version),
        _ => Err(RequestError::Unknown(kind.into_owned())),
    }
}

/// One action, as the JSON view lists it.
#[derive(Debug, Serialize)]
pub struct ActionView<'a> {
    pub key: &'a str,
    pub label: &'a str,
}

/// One icon, as the JSON view lists it: a name, a path, or the size of
/// decoded pixels. Pixels never cross the socket.
#[derive(Debug, Serialize)]
pub struct IconView<'a> {
    /// `name`, `path` or `data`.
    pub kind: &'static str,
    pub value: Cow<'a, str>,
}

/// One live notification, as `list` shows it and `notify` events carry
/// it. Every key but `icon` is always present, `null` where not known:
/// `shown` is null until first presented, `expires` null where the card
/// never expires, `output` null where the compositor chooses. `icon` is
/// absent where the card shows none. New keys may be added; none is
/// removed or changes meaning within a protocol version.
#[derive(Debug, Serialize)]
pub struct NotificationView<'a> {
    pub id: u32,
    pub app_name: &'a str,
    pub desktop_entry: &'a str,
    pub summary: &'a str,
    /// The body with markup stripped.
    pub body: &'a str,
    /// `low`, `normal` or `critical`.
    pub urgency: &'static str,
    pub category: &'a str,
    pub actions: Vec<ActionView<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconView<'a>>,
    /// CLOCK_REALTIME ms at arrival.
    pub created: u64,
    /// CLOCK_REALTIME ms at first presentation, `None` until shown.
    pub shown: Option<u64>,
    /// CLOCK_REALTIME ms when the card expires, `None` where it never
    /// does (or until shown). Hover and input-idle pauses push the real
    /// deadline past this; it is recomputed from the running clock on
    /// every read.
    pub expires: Option<u64>,
    /// The pinned output, or `None` where the compositor chooses.
    pub output: Option<&'a str>,
}

/// The JSON view of one live card: `list` shows these, `notify` events
/// carry one. `output` is the pinned output name, if any.
pub fn view<'a>(
    note: &'a Notification,
    now: Instant,
    now_ms: u64,
    output: Option<&'a str>,
) -> NotificationView<'a> {
    NotificationView {
        id: note.id,
        app_name: &note.app_name,
        desktop_entry: &note.desktop_entry,
        summary: &note.summary,
        body: &note.plain_body,
        urgency: match note.urgency {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::Critical => "critical",
        },
        category: &note.category,
        actions: note
            .actions
            .iter()
            .map(|pair| ActionView {
                key: &pair.key,
                label: &pair.label,
            })
            .collect(),
        icon: match &note.icon {
            crate::model::store::Icon::None => None,
            crate::model::store::Icon::Name(name) => Some(IconView {
                kind: "name",
                value: Cow::Borrowed(name),
            }),
            crate::model::store::Icon::Path(path) => Some(IconView {
                kind: "path",
                value: Cow::Borrowed(path),
            }),
            crate::model::store::Icon::Data(image) => Some(IconView {
                kind: "data",
                // The size, never the pixels.
                value: Cow::Owned(format!("{}x{}", image.width, image.height)),
            }),
        },
        created: note.created_ms,
        shown: note.shown_ms,
        expires: note.clock.deadline().map(|at| {
            now_ms.saturating_add(
                at.saturating_duration_since(now)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            )
        }),
        output,
    }
}

/// One history entry, as `history` shows it: what a closed or held
/// card left behind. There is no live id (the old one was invalidated
/// by its `NotificationClosed`, as the spec requires), so `restore`
/// re-shows the newest as a new card. New keys may be added; none is
/// removed or changes meaning within a protocol version.
#[derive(Debug, Serialize)]
pub struct HistoryView<'a> {
    pub app_name: &'a str,
    pub desktop_entry: &'a str,
    pub summary: &'a str,
    /// The body with markup stripped.
    pub body: &'a str,
    /// `low`, `normal` or `critical`.
    pub urgency: &'static str,
    pub category: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconView<'a>>,
    /// CLOCK_REALTIME ms at arrival.
    pub created: u64,
    /// Whether the card was ever on screen.
    pub seen: bool,
}

/// The JSON view of one history entry.
pub fn history_view(entry: &HistoryEntry) -> HistoryView<'_> {
    use crate::model::history::IconName;
    HistoryView {
        app_name: &entry.app_name,
        desktop_entry: &entry.desktop_entry,
        summary: &entry.summary,
        body: &entry.body,
        urgency: match entry.urgency {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::Critical => "critical",
        },
        category: &entry.category,
        icon: match &entry.icon {
            None => None,
            Some(IconName::Name(name)) => Some(IconView {
                kind: "name",
                value: Cow::Borrowed(name),
            }),
            Some(IconName::Path(path)) => Some(IconView {
                kind: "path",
                value: Cow::Borrowed(path),
            }),
        },
        created: entry.created_ms,
        seen: entry.seen,
    }
}

/// An event on a subscribed connection.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event<'a> {
    /// A notification arrived, shown as `list` shows it. Boxed: the
    /// view is the enum's largest variant by far, and events are rare
    /// (bounded by the sender rate limit), so the indirection costs
    /// nothing measurable.
    Notify {
        notification: Box<NotificationView<'a>>,
    },
    /// A card closed: `expired`, `dismissed`, `closed`
    /// (`CloseNotification`) or `evicted` (the live cap).
    Close { id: u32, reason: &'static str },
    /// An action was invoked, through `msg invoke` or a click.
    Invoke { id: u32, action: &'a str },
    /// DND changed, through `msg dnd`, a bind or the bar.
    Dnd { dnd: &'a str },
}

impl CloseReason {
    /// The event's reason string.
    pub fn name(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Dismissed => "dismissed",
            Self::Closed => "closed",
            Self::Evicted => "evicted",
        }
    }
}

/// The last line a subscriber the daemon is about to drop can be sent:
/// the shared connection's, re-exported so the client's and tests name
/// the same line the daemon writes.
pub use scootui::control::DROPPED;

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
    /// The answer to `list`: the live cards, oldest first.
    Notifications {
        notifications: Vec<NotificationView<'a>>,
    },
    /// The answer to `history`: the ring, oldest first.
    History {
        history: Vec<HistoryView<'a>>,
    },
    /// The answer to `dnd`: the mode after the request ran.
    Dnd {
        dnd: &'a str,
    },
    Ok {
        /// The scoped-reload receipt: a look-scoped reload carries one
        /// (`"look"`), so the CLI can tell a scoped reply from a plain one.
        /// Plain reloads leave this absent, keeping their reply
        /// byte-identical (`{"type":"ok"}`).
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<&'a str>,
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
