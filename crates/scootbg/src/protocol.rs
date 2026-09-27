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
//! Parsing borrows from the line and allocates nothing unless a string in
//! it contains escapes; replies are written straight into the
//! connection's reused output buffer.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};

use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::outputs::Size;
use crate::wallpaper::Wallpaper;

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

/// What a client can ask for. `apply-config` arrives with the scoot
/// integration (docs/scootbg/backlog/scoot-integration.md).
///
/// ```text
/// {"protocol":1,"type":"set","color":"#1e1e2e"}
/// {"protocol":1,"type":"set","color":"#1e1e2e","output":"DP-1"}
/// {"protocol":1,"type":"set","image":"/abs/a.jpg","mode":"fit","fill":"#101014","filter":"lanczos3"}
/// {"protocol":1,"type":"clear"}
/// {"protocol":1,"type":"clear","output":"DP-1"}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request<'a> {
    /// What each output shows.
    Query,
    /// Stop the daemon.
    Kill,
    /// The daemon's version and protocol.
    Version,
    /// Show `show` on every output, or on the outputs named `output`.
    Set {
        show: Show<'a>,
        output: Option<Cow<'a, str>>,
    },
    /// Show nothing (the compositor's own background) on every output, or
    /// on the outputs named `output`.
    Clear { output: Option<Cow<'a, str>> },
}

/// What a `set` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show<'a> {
    Color(Color),
    Image(ImageRequest<'a>),
}

/// An image, as a `set` asks for it. On the wire `mode`, `fill` and
/// `filter` may be left out, for `fill`, `#000000` and `lanczos3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest<'a> {
    /// Absolute: the daemon refuses anything else, since its working
    /// directory is not the client's.
    pub path: Cow<'a, str>,
    pub mode: Mode,
    /// Behind a letterboxed or centred image, and under transparency.
    pub fill: Color,
    pub filter: Filter,
}

/// The fill color when a request names none.
pub const DEFAULT_FILL: Color = Color { r: 0, g: 0, b: 0 };

impl Request<'_> {
    /// The `type` string on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Kill => "kill",
            Self::Version => "version",
            Self::Set { .. } => "set",
            Self::Clear { .. } => "clear",
        }
    }

    /// The request line a client sends, newline included. Strings are
    /// JSON-escaped, so any output name or path round-trips.
    pub fn line(&self) -> String {
        #[derive(Serialize)]
        struct Line<'r> {
            protocol: u32,
            #[serde(rename = "type")]
            kind: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            color: Option<Color>,
            #[serde(skip_serializing_if = "Option::is_none")]
            image: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            mode: Option<Mode>,
            #[serde(skip_serializing_if = "Option::is_none")]
            fill: Option<Color>,
            #[serde(skip_serializing_if = "Option::is_none")]
            filter: Option<Filter>,
            #[serde(skip_serializing_if = "Option::is_none")]
            output: Option<&'r str>,
        }
        let mut line = Line {
            protocol: PROTOCOL_VERSION,
            kind: self.name(),
            color: None,
            image: None,
            mode: None,
            fill: None,
            filter: None,
            output: None,
        };
        match self {
            Self::Set { show, output } => {
                line.output = output.as_deref();
                match show {
                    Show::Color(color) => line.color = Some(*color),
                    Show::Image(image) => {
                        line.image = Some(&image.path);
                        line.mode = Some(image.mode);
                        line.fill = Some(image.fill);
                        line.filter = Some(image.filter);
                    }
                }
            }
            Self::Clear { output } => line.output = output.as_deref(),
            Self::Query | Self::Kill | Self::Version => {}
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
    /// `set` with neither a `color` nor an `image`.
    NoTarget,
    /// `set` with both.
    Both,
    /// `set` with a `color` that is not `#rrggbb`.
    BadColor {
        text: String,
        error: ColorError,
    },
    /// An image `fill` that is not `#rrggbb`.
    BadFill {
        text: String,
        error: ColorError,
    },
    BadMode(String),
    BadFilter(String),
    /// `mode`, `fill` or `filter` with a color.
    ImageOnly(&'static str),
    /// An image path that is not absolute.
    RelativePath(String),
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
            Self::NoTarget => write!(
                f,
                "`set` needs a `color` (\"#rrggbb\") or an `image` (an absolute path)"
            ),
            Self::Both => write!(f, "`set` takes a `color` or an `image`, not both"),
            Self::BadColor { text, error } => write!(f, "bad color {text:?}: {error}"),
            Self::BadFill { text, error } => write!(f, "bad fill color {text:?}: {error}"),
            Self::BadMode(mode) => write!(
                f,
                "unknown mode {mode:?}: fill, fit, stretch, center or tile"
            ),
            Self::BadFilter(filter) => write!(
                f,
                "unknown filter {filter:?}: lanczos3, catmull-rom, bilinear or nearest"
            ),
            Self::ImageOnly(field) => write!(f, "`{field}` applies to an image, not a color"),
            Self::RelativePath(path) => write!(
                f,
                "the image path {path:?} is not absolute (the daemon's working directory \
                 is not yours; `scootbg set` makes a path absolute before sending it)"
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
    color: Option<Cow<'a, str>>,
    #[serde(borrow)]
    image: Option<Cow<'a, str>>,
    #[serde(borrow)]
    mode: Option<Cow<'a, str>>,
    #[serde(borrow)]
    fill: Option<Cow<'a, str>>,
    #[serde(borrow)]
    filter: Option<Cow<'a, str>>,
    #[serde(borrow)]
    output: Option<Cow<'a, str>>,
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
        "kill" => Ok(Request::Kill),
        "version" => Ok(Request::Version),
        "set" => {
            let show = show(
                envelope.color,
                envelope.image,
                envelope.mode,
                envelope.fill,
                envelope.filter,
            )?;
            Ok(Request::Set {
                show,
                output: envelope.output,
            })
        }
        "clear" => Ok(Request::Clear {
            output: envelope.output,
        }),
        _ => Err(RequestError::Unknown(kind.into_owned())),
    }
}

/// What a `set` asks to show, from its fields.
fn show<'a>(
    color: Option<Cow<'a, str>>,
    image: Option<Cow<'a, str>>,
    mode: Option<Cow<'a, str>>,
    fill: Option<Cow<'a, str>>,
    filter: Option<Cow<'a, str>>,
) -> Result<Show<'a>, RequestError> {
    match (color, image) {
        (Some(_), Some(_)) => Err(RequestError::Both),
        (None, None) => Err(RequestError::NoTarget),
        (Some(text), None) => {
            for (field, given) in [("mode", &mode), ("fill", &fill), ("filter", &filter)] {
                if given.is_some() {
                    return Err(RequestError::ImageOnly(field));
                }
            }
            Color::parse(&text)
                .map(Show::Color)
                .map_err(|error| RequestError::BadColor {
                    text: text.into_owned(),
                    error,
                })
        }
        (None, Some(path)) => {
            if !path.starts_with('/') {
                return Err(RequestError::RelativePath(path.into_owned()));
            }
            let mode = match mode {
                None => Mode::default(),
                Some(name) => Mode::from_name(&name)
                    .ok_or_else(|| RequestError::BadMode(name.into_owned()))?,
            };
            let filter = match filter {
                None => Filter::default(),
                Some(name) => Filter::from_name(&name)
                    .ok_or_else(|| RequestError::BadFilter(name.into_owned()))?,
            };
            let fill = match fill {
                None => DEFAULT_FILL,
                Some(text) => Color::parse(&text).map_err(|error| RequestError::BadFill {
                    text: text.into_owned(),
                    error,
                })?,
            };
            Ok(Show::Image(ImageRequest {
                path,
                mode,
                fill,
                filter,
            }))
        }
    }
}

/// One output in a `query` reply, borrowed from the daemon's state so a
/// reply allocates nothing beyond the connection's output buffer:
///
/// ```text
/// {"name":"DP-1","description":"Dell U2720Q (DP-1)",
///  "mode":{"width":3840,"height":2160},"scale":2,"transform":"normal",
///  "logical":{"width":1920,"height":1080},
///  "surface":{"state":"configured","size":{"width":1920,"height":1080},
///             "scale":2,"pixels":{"width":3840,"height":2160}},
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
    /// `wl_output`'s integer scale: a fractional scale rounded up. The
    /// scale the wallpaper is drawn at is `surface.scale`.
    pub scale: u32,
    /// `wl_output.transform`: `normal`, `90`, `180`, `270`, `flipped`,
    /// `flipped-90`, ... Rotations count counter-clockwise, as the
    /// protocol does (sway's `transform 90`, clockwise, reports `270`).
    pub transform: &'a str,
    /// The output's size in logical pixels, as well as it is known (see
    /// `Output::logical`): exact once the surface is configured; before
    /// that, worked out from the mode and the best scale known, within a
    /// pixel.
    pub logical: Option<Size>,
    pub surface: SurfaceEntry,
    /// What the output shows: `{"color":"#rrggbb"}`,
    /// `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
    /// or `null` for nothing (the compositor's own background, or no
    /// surface yet).
    pub shows: Option<Shows<'a>>,
}

/// What an output shows, when it shows something: an object, which may
/// gain keys within a protocol version.
#[derive(Debug)]
pub struct Shows<'a>(pub &'a Wallpaper);

impl Serialize for Shows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self.0 {
            Wallpaper::Color(color) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("color", color)?;
                map.end()
            }
            Wallpaper::Image(image) => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("image", &image.path)?;
                map.serialize_entry("mode", &image.look.mode)?;
                map.serialize_entry("fill", &image.look.fill)?;
                map.serialize_entry("filter", &image.look.filter)?;
                map.end()
            }
        }
    }
}

/// Where an output's wallpaper surface stands: `state` is one of
/// `waiting` (the output has not reported itself yet), `pending` (asked
/// for, not yet sized), `configured`, `closed` (closed by the compositor,
/// being re-created) and `gave-up` (closed twice; not tried again until
/// the output is replugged).
/// `size`, in logical pixels, is set only while `configured`, and even
/// then `null` if the compositor left the size to scootbg before the
/// output reported a mode. `scale` is the scale the surface is drawn at:
/// `wp_fractional_scale_v1`'s (1.5, say) where the compositor sent one and
/// has a viewporter, else `wl_surface.preferred_buffer_scale`, else
/// `wl_output`'s; `pixels` is the size of a full-size buffer at it (an
/// image's, in device pixels: 1601×1001 for scoot's 1067×667 at 1.5). Both
/// `null` whenever `size` is.
#[derive(Debug, Serialize)]
pub struct SurfaceEntry {
    pub state: &'static str,
    pub size: Option<Size>,
    pub scale: Option<crate::density::Scale>,
    pub pixels: Option<Size>,
}

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
