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
//! A request type is added within a protocol version when a daemon that
//! predates it refusing it (`unknown request`) is the right outcome, as
//! for `apply-config`, whose client reports that loudly (`crate::apply`).
//!
//! Parsing borrows from the line and allocates nothing unless a string in
//! it contains escapes; replies are written straight into the
//! connection's reused output buffer.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};

use crate::choices::MAX_WORKSPACE_NAME;
use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::outputs::Size;
use crate::rotation::{self, EveryError};
use crate::section::Section;
use crate::state::{Profile, ProfileError};
use crate::transition::{self, Kind, Spec};
use crate::wallpaper::Wallpaper;

#[cfg(test)]
mod tests;

/// Bumped whenever a wire change would make an old client or daemon
/// misread a new one.
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line accepted, newline excluded. Requests are small
/// (the largest, `apply-config`, carries one config section with paths, at
/// most `section::MAX_SECTION` bytes); anything longer is refused and the
/// connection closed, so a client cannot make the daemon buffer without
/// bound.
pub const MAX_REQUEST_LINE: usize = 64 * 1024;

/// What a client can ask for.
///
/// ```text
/// {"protocol":1,"type":"set","color":"#1e1e2e"}
/// {"protocol":1,"type":"set","color":"#1e1e2e","output":"DP-1"}
/// {"protocol":1,"type":"set","image":"/abs/a.jpg","mode":"fit","fill":"#101014","filter":"lanczos3"}
/// {"protocol":1,"type":"set","color":"#101014","transition":"fade","duration-ms":"500","easing":"ease-out"}
/// {"protocol":1,"type":"clear"}
/// {"protocol":1,"type":"clear","output":"DP-1"}
/// {"protocol":1,"type":"apply-config","profile":"scoot","config":{"color":"#1e1e2e"}}
/// ```
///
/// A `set` may carry a transition (`crate::transition`): `transition`
/// (one of `none`, `fade`, `wipe`, `grow`), and, for any kind but `none`,
/// `duration-ms` (milliseconds as digits), `easing` (`linear`, `ease-in`,
/// `ease-out`, `ease-in-out`, `smooth`), `angle` (a wipe's degrees) and
/// `position` (a grow's `X,Y`). Each is a string, parsed strictly; anything
/// absent is the default (`none`, 500 ms, `ease-out`, 0 degrees, the
/// center). Parameters without a `transition` are refused; with an explicit
/// `none` they are ignored. A `set` with no transition carries no
/// transition fields, so an older daemon that predates transitions reads it
/// unchanged, and one with a transition reads it as an instant `set`: it
/// shows the same wallpaper, without the animation.
/// A `clear` with any of them is refused: clearing shows the compositor's
/// own background at once, which there is nothing to blend from or to.
///
/// A `set-workspace` shows `show` on `workspace` (one workspace, by the
/// name the compositor announces: "1", "2", ... on scoot) rather than on
/// every output: while that workspace is active, those outputs show it,
/// arriving through `transition` when the workspace turns active (see
/// `crate::choices`). A `clear-workspace` takes that mapping back off, and
/// refuses transition fields like `clear` does. Both are their own request
/// types, rather than a `workspace` field on `set` and `clear`, so a daemon
/// that predates workspaces refuses them (`unknown request`) instead of
/// misreading one as a wallpaper for every output.
#[derive(Debug, Clone, PartialEq)]
pub enum Request<'a> {
    /// What each output shows.
    Query,
    /// Stop the daemon.
    Kill,
    /// The daemon's version and protocol.
    Version,
    /// Show `show` on every output, or on the outputs named `output`,
    /// arriving through `transition`.
    Set {
        show: Show<'a>,
        output: Option<Cow<'a, str>>,
        transition: Spec,
    },
    /// Show nothing (the compositor's own background) on every output, or
    /// on the outputs named `output`.
    Clear { output: Option<Cow<'a, str>> },
    /// Show `show` on `workspace` (every output, or the outputs named
    /// `output`, while that workspace is active there), arriving through
    /// `transition` when it turns active.
    SetWorkspace {
        show: Show<'a>,
        output: Option<Cow<'a, str>>,
        workspace: Cow<'a, str>,
        transition: Spec,
    },
    /// Take the `workspace` mapping back off (every output, or the outputs
    /// named `output`): those outputs fall back to their own wallpaper.
    ClearWorkspace {
        output: Option<Cow<'a, str>>,
        workspace: Cow<'a, str>,
    },
    /// A `[wallpaper]` section from scoot's config, for `profile`
    /// (`crate::section`, `daemon::config`): adopt the profile, and apply
    /// the section if it changed since it was last applied. Boxed: the
    /// section is the request's largest variant by far.
    ApplyConfig {
        profile: Profile,
        section: Box<Section>,
    },
}

/// What a `set` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show<'a> {
    Color(Color),
    Image(ImageRequest<'a>),
    /// A slideshow from a directory: `dir`'s regular files in turn, one
    /// every `every_secs` seconds (`crate::rotation`). The first file goes
    /// through the normal image path (decoded before the reply; when it
    /// cannot be shown the show still starts, said in the reply, and that
    /// file fails its turns until the next rotation); later ones advance
    /// the same way, with no reply.
    Slideshow(SlideshowRequest<'a>),
}

/// A slideshow, as a `set` asks for it. On the wire `mode`, `fill` and
/// `filter` may be left out, for `fill`, `#000000` and `lanczos3`;
/// `shuffle` may be left out whenever the order stays sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideshowRequest<'a> {
    /// Absolute: the daemon refuses anything else, since its working
    /// directory is not the client's.
    pub dir: Cow<'a, str>,
    /// Seconds between images: whole minutes, at least one minute. On the wire as
    /// seconds with an `s` (`"1800s"`), read by the same
    /// [`parse_every`](crate::rotation::parse_every) the CLI uses, so one
    /// grammar names every pace.
    pub every_secs: u64,
    /// Cycle in a shuffled order (shuffled once, when set).
    pub shuffle: bool,
    pub mode: Mode,
    /// Behind a letterboxed or centred image, and under transparency.
    pub fill: Color,
    pub filter: Filter,
}

/// An image, as a `set` asks for it. On the wire `mode`, `fill` and
/// `filter` may be left out, for `fill`, `#000000` and `lanczos3`; `sha256`
/// may be left out whenever no hash is pinned. `animate` may be left out,
/// for checking animation caps: `animate:false` shows the first frame
/// without checking them (`--no-animate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest<'a> {
    /// A file's path, or the URL, as a download: never confused, so a
    /// pinned hash cannot end up verifying the wrong thing.
    pub source: Source<'a>,
    pub mode: Mode,
    /// Behind a letterboxed or centred image, and under transparency.
    pub fill: Color,
    pub filter: Filter,
    /// Whether an animated image goes through the animation caps (at
    /// most 64 frames and 64 MiB of frames; past them the `set` is
    /// refused). Old daemons ignore it (unknown fields) and show the
    /// first frame, which is what `false` asks for. Frame-by-frame
    /// playback is a follow-up: every value shows the first frame today.
    pub animate: bool,
}

/// Where a requested image comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source<'a> {
    /// Absolute: the daemon refuses anything else, since its working
    /// directory is not the client's.
    Path(Cow<'a, str>),
    /// Downloaded once and cached (`crate::fetch`).
    Url {
        url: Cow<'a, str>,
        sha256: Option<[u8; 32]>,
    },
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
            Self::SetWorkspace { .. } => "set-workspace",
            Self::ClearWorkspace { .. } => "clear-workspace",
            Self::ApplyConfig { .. } => "apply-config",
        }
    }

    /// The request line a client sends, newline included. Strings are
    /// JSON-escaped, so any output name or path round-trips. A `set` with
    /// no transition (`Spec::none`) carries no transition fields, so an
    /// older daemon reads it unchanged.
    pub fn line(&self) -> String {
        if let Self::ApplyConfig { profile, section } = self {
            return section.request_line(profile);
        }
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
            directory: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            every: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            shuffle: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            sha256: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            mode: Option<Mode>,
            #[serde(skip_serializing_if = "Option::is_none")]
            fill: Option<Color>,
            #[serde(skip_serializing_if = "Option::is_none")]
            filter: Option<Filter>,
            #[serde(skip_serializing_if = "Option::is_none")]
            output: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            workspace: Option<&'r str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            transition: Option<&'static str>,
            #[serde(skip_serializing_if = "Option::is_none", rename = "duration-ms")]
            duration_ms: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            easing: Option<&'static str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            angle: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            position: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            animate: Option<&'static str>,
        }
        let mut line = Line {
            protocol: PROTOCOL_VERSION,
            kind: self.name(),
            color: None,
            image: None,
            directory: None,
            every: None,
            shuffle: None,
            sha256: None,
            mode: None,
            fill: None,
            filter: None,
            output: None,
            workspace: None,
            transition: None,
            duration_ms: None,
            easing: None,
            angle: None,
            position: None,
            animate: None,
        };
        // One `set`'s transition fields, shared by `set` and
        // `set-workspace`: only owned strings and `&'static` names cross
        // into the line, so no lifetime ties to the request. The show
        // itself is filled inline per arm, as it always was: its strings
        // borrow the request, which the compiler threads on its own.
        fn fill_transition(line: &mut Line<'_>, transition: &Spec) {
            if transition.kind != Kind::None {
                line.transition = Some(transition.kind.name());
                line.duration_ms = Some(transition.duration_ms.to_string());
                line.easing = Some(transition.easing.name());
                line.angle = Some(transition.angle_deg.to_string());
                line.position = Some(format!("{},{}", transition.pos.0, transition.pos.1));
            }
        }
        match self {
            Self::Set {
                show,
                output,
                transition,
            } => {
                line.output = output.as_deref();
                fill_transition(&mut line, transition);
                match show {
                    Show::Color(color) => line.color = Some(*color),
                    Show::Slideshow(slideshow) => {
                        line.directory = Some(&slideshow.dir);
                        line.every = Some(format!("{}s", slideshow.every_secs));
                        if slideshow.shuffle {
                            line.shuffle = Some(true);
                        }
                        line.mode = Some(slideshow.mode);
                        line.fill = Some(slideshow.fill);
                        line.filter = Some(slideshow.filter);
                    }
                    Show::Image(image) => {
                        match &image.source {
                            Source::Path(path) => line.image = Some(path),
                            Source::Url { url, sha256 } => {
                                line.image = Some(url);
                                line.sha256 = sha256
                                    .as_ref()
                                    .map(|hash| crate::sha256::hex_bytes(hash.as_slice()));
                            }
                        }
                        line.mode = Some(image.mode);
                        line.fill = Some(image.fill);
                        line.filter = Some(image.filter);
                        if !image.animate {
                            line.animate = Some("false");
                        }
                    }
                }
            }
            Self::SetWorkspace {
                show,
                output,
                workspace,
                transition,
            } => {
                line.output = output.as_deref();
                let workspace: &str = workspace;
                line.workspace = Some(workspace);
                fill_transition(&mut line, transition);
                match show {
                    Show::Color(color) => line.color = Some(*color),
                    Show::Slideshow(slideshow) => {
                        line.directory = Some(&slideshow.dir);
                        line.every = Some(format!("{}s", slideshow.every_secs));
                        if slideshow.shuffle {
                            line.shuffle = Some(true);
                        }
                        line.mode = Some(slideshow.mode);
                        line.fill = Some(slideshow.fill);
                        line.filter = Some(slideshow.filter);
                    }
                    Show::Image(image) => {
                        match &image.source {
                            Source::Path(path) => line.image = Some(path),
                            Source::Url { url, sha256 } => {
                                line.image = Some(url);
                                line.sha256 = sha256
                                    .as_ref()
                                    .map(|hash| crate::sha256::hex_bytes(hash.as_slice()));
                            }
                        }
                        line.mode = Some(image.mode);
                        line.fill = Some(image.fill);
                        line.filter = Some(image.filter);
                        if !image.animate {
                            line.animate = Some("false");
                        }
                    }
                }
            }
            Self::Clear { output } => line.output = output.as_deref(),
            Self::ClearWorkspace { output, workspace } => {
                line.output = output.as_deref();
                let workspace: &str = workspace;
                line.workspace = Some(workspace);
            }
            Self::Query | Self::Kill | Self::Version | Self::ApplyConfig { .. } => {}
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
    /// `set` with neither a `color`, an `image` nor a `directory`.
    NoTarget,
    /// `set` with more than one of a `color`, an `image` and a `directory`.
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
    /// A `sha256` that is not 64 hex digits.
    BadSha(String),
    /// A `sha256` with a file: it pins a download, and a file is already
    /// here to be read.
    ShaWithFile,
    /// A `sha256` with a directory: it pins a download, and a directory is
    /// listed, not downloaded.
    ShaWithDirectory,
    /// An `every` without a `directory`: it paces a slideshow, and this
    /// `set` names no directory.
    EveryWithoutDirectory,
    /// A `directory` without an `every`: a directory is only ever a
    /// slideshow, which needs its pace.
    EveryMissing,
    /// An `every` that is not a rotation pace.
    BadEvery(EveryError),
    /// A `shuffle` without a `directory`: it shuffles a slideshow, and this
    /// `set` names none.
    ShuffleWithoutDirectory,
    /// An `animate` with a `directory`: stilling is per image, and a
    /// slideshow steps through many (each checked like one `set`).
    AnimateWithSlideshow,
    /// An image URL with a NUL byte.
    UrlNul,
    /// `apply-config` whose `profile` or `config` is not one (serde's
    /// message, from `crate::section`'s strict parse).
    BadApply(serde_json::Error),
    /// `apply-config` without a `profile`.
    NoProfile,
    /// `apply-config` with a profile name that cannot be one.
    BadProfile(ProfileError),
    /// `apply-config` without a `config`.
    NoConfig,
    /// A transition that is not one (`crate::transition` says what each
    /// takes).
    BadTransition(transition::ParseError),
    /// A `clear` with transition fields: clearing shows the compositor's
    /// own background at once, which there is nothing to blend from or to.
    TransitionWithClear,
    /// A `set-workspace` or `clear-workspace` without a `workspace`.
    NoWorkspace,
    /// A `set-workspace` or `clear-workspace` with an empty `workspace`.
    EmptyWorkspace,
    /// A `set-workspace` or `clear-workspace` with a `workspace` past
    /// [`MAX_WORKSPACE_NAME`](crate::choices::MAX_WORKSPACE_NAME) bytes.
    WorkspaceTooLong,
    /// A `set-workspace` or `clear-workspace` with a `workspace` holding
    /// a NUL byte.
    WorkspaceNul,
    /// A `set-workspace` with a slideshow (`directory`/`every`/`shuffle`):
    /// slideshows run on every output (or one `output`), not per workspace.
    SlideshowWithWorkspace,
    /// A `clear` with `animate`: clearing shows nothing, which has no
    /// frames.
    AnimateWithClear,
    /// An `animate` that is neither `true` nor `false`.
    BadAnimate(String),
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
                "`set` needs a `color` (\"#rrggbb\"), an `image` (an absolute path, or an \
                 `http(s)` URL) or a `directory` (an absolute path, with an `every`)"
            ),
            Self::Both => write!(
                f,
                "`set` takes a `color`, an `image` or a `directory`, not more than one"
            ),
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
            Self::BadSha(text) => write!(
                f,
                "the sha256 {text:?} is not 64 hex digits (as `sha256sum` prints)"
            ),
            Self::ShaWithFile => write!(
                f,
                "`sha256` pins a downloaded image, and this one is a file"
            ),
            Self::ShaWithDirectory => write!(
                f,
                "`sha256` pins a downloaded image, and this one is a directory"
            ),
            Self::EveryWithoutDirectory => write!(
                f,
                "`every` paces a slideshow, and this `set` names no directory"
            ),
            Self::EveryMissing => write!(
                f,
                "a `directory` is a slideshow, which needs an `every` (such as `\"1800s\"`)"
            ),
            Self::BadEvery(error) => write!(f, "{error}"),
            Self::ShuffleWithoutDirectory => write!(
                f,
                "`shuffle` shuffles a slideshow, and this `set` names no directory"
            ),
            Self::AnimateWithSlideshow => write!(
                f,
                "`animate` stills one image, and a slideshow steps through a directory \
                 (each step checked like one `set`)"
            ),
            Self::UrlNul => write!(f, "the image URL has a NUL byte"),
            Self::BadApply(error) => write!(f, "bad apply-config request: {error}"),
            Self::NoProfile => write!(f, "`apply-config` needs a `profile`"),
            Self::BadProfile(error) => write!(f, "`apply-config`: {error}"),
            Self::NoConfig => write!(
                f,
                "`apply-config` needs a `config` object (`{{}}` when the section is absent)"
            ),
            Self::BadTransition(error) => write!(f, "{error}"),
            Self::TransitionWithClear => write!(
                f,
                "`clear` shows the compositor's own background at once: there is nothing to \
                 blend from or to, so it takes no transition"
            ),
            Self::NoWorkspace => write!(
                f,
                "`set-workspace` and `clear-workspace` need a `workspace` (the name the \
                 compositor announces: \"1\", \"2\", ... on scoot)"
            ),
            Self::EmptyWorkspace => write!(f, "the `workspace` is empty"),
            Self::WorkspaceTooLong => {
                write!(f, "the `workspace` is past {MAX_WORKSPACE_NAME} bytes")
            }
            Self::WorkspaceNul => write!(f, "the `workspace` has a NUL byte"),
            Self::SlideshowWithWorkspace => write!(
                f,
                "a slideshow runs on every output (or one `output`), not per workspace; \
                 map an image or a color with `set-workspace` instead"
            ),
            Self::AnimateWithClear => write!(
                f,
                "`clear` shows the compositor's own background at once: there is nothing to \
                 animate, so it takes no `animate`"
            ),
            Self::BadAnimate(text) => write!(
                f,
                "`animate` is `true` or `false` (absent animates), not {text:?}"
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
    directory: Option<Cow<'a, str>>,
    #[serde(borrow)]
    every: Option<Cow<'a, str>>,
    shuffle: Option<bool>,
    #[serde(borrow)]
    sha256: Option<Cow<'a, str>>,
    #[serde(borrow)]
    mode: Option<Cow<'a, str>>,
    #[serde(borrow)]
    fill: Option<Cow<'a, str>>,
    #[serde(borrow)]
    filter: Option<Cow<'a, str>>,
    #[serde(borrow)]
    output: Option<Cow<'a, str>>,
    #[serde(borrow)]
    workspace: Option<Cow<'a, str>>,
    #[serde(borrow)]
    transition: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "duration-ms")]
    duration_ms: Option<Cow<'a, str>>,
    #[serde(borrow)]
    easing: Option<Cow<'a, str>>,
    #[serde(borrow)]
    angle: Option<Cow<'a, str>>,
    #[serde(borrow)]
    position: Option<Cow<'a, str>>,
    #[serde(borrow)]
    animate: Option<Cow<'a, str>>,
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
                envelope.directory,
                envelope.every,
                envelope.shuffle,
                envelope.sha256,
                envelope.mode,
                envelope.fill,
                envelope.filter,
                envelope.animate,
            )?;
            let transition = transition(
                envelope.transition,
                envelope.duration_ms,
                envelope.easing,
                envelope.angle,
                envelope.position,
            )?;
            Ok(Request::Set {
                show,
                output: envelope.output,
                transition,
            })
        }
        "clear" => {
            if envelope.transition.is_some()
                || envelope.duration_ms.is_some()
                || envelope.easing.is_some()
                || envelope.angle.is_some()
                || envelope.position.is_some()
            {
                return Err(RequestError::TransitionWithClear);
            }
            if envelope.animate.is_some() {
                return Err(RequestError::AnimateWithClear);
            }
            Ok(Request::Clear {
                output: envelope.output,
            })
        }
        "set-workspace" => {
            // A slideshow runs on every output (or one `output`), not per
            // workspace: refused before the per-image `animate` check, like
            // the CLI's `--workspace` check wins over `--no-animate`.
            if envelope.directory.is_some() {
                return Err(RequestError::SlideshowWithWorkspace);
            }
            let show = show(
                envelope.color,
                envelope.image,
                envelope.directory,
                envelope.every,
                envelope.shuffle,
                envelope.sha256,
                envelope.mode,
                envelope.fill,
                envelope.filter,
                envelope.animate,
            )?;
            if matches!(show, Show::Slideshow(_)) {
                return Err(RequestError::SlideshowWithWorkspace);
            }
            let transition = transition(
                envelope.transition,
                envelope.duration_ms,
                envelope.easing,
                envelope.angle,
                envelope.position,
            )?;
            Ok(Request::SetWorkspace {
                show,
                output: envelope.output,
                workspace: workspace(envelope.workspace)?,
                transition,
            })
        }
        "clear-workspace" => {
            if envelope.transition.is_some()
                || envelope.duration_ms.is_some()
                || envelope.easing.is_some()
                || envelope.angle.is_some()
                || envelope.position.is_some()
            {
                return Err(RequestError::TransitionWithClear);
            }
            if envelope.animate.is_some() {
                return Err(RequestError::AnimateWithClear);
            }
            Ok(Request::ClearWorkspace {
                output: envelope.output,
                workspace: workspace(envelope.workspace)?,
            })
        }
        "apply-config" => apply_config(line),
        _ => Err(RequestError::Unknown(kind.into_owned())),
    }
}

/// The workspace a `set-workspace` or `clear-workspace` names: present,
/// non-empty, short, and without a NUL byte (which JSON can carry as an
/// escape, and nothing downstream wants).
fn workspace(workspace: Option<Cow<'_, str>>) -> Result<Cow<'_, str>, RequestError> {
    let workspace = workspace.ok_or(RequestError::NoWorkspace)?;
    if workspace.is_empty() {
        return Err(RequestError::EmptyWorkspace);
    }
    if workspace.len() > MAX_WORKSPACE_NAME {
        return Err(RequestError::WorkspaceTooLong);
    }
    if workspace.contains('\0') {
        return Err(RequestError::WorkspaceNul);
    }
    Ok(workspace)
}

/// `apply-config`'s own fields, read in a second pass over the line (only
/// for this request, which comes once per scoot reload), so that a `config`
/// on any other request stays an ignored unknown field, and the section's
/// strict parse (unknown keys, duplicates) sees the object itself.
fn apply_config(line: &[u8]) -> Result<Request<'static>, RequestError> {
    #[derive(Deserialize)]
    struct Apply<'a> {
        #[serde(borrow)]
        profile: Option<Cow<'a, str>>,
        config: Option<Section>,
    }
    let apply: Apply<'_> = serde_json::from_slice(line).map_err(RequestError::BadApply)?;
    let profile = apply.profile.ok_or(RequestError::NoProfile)?;
    let profile = Profile::parse(&profile).map_err(RequestError::BadProfile)?;
    let section = apply.config.ok_or(RequestError::NoConfig)?;
    Ok(Request::ApplyConfig {
        profile,
        section: Box::new(section),
    })
}

/// What a `set` asks to show, from its fields: one parameter per field the
/// envelope carries, so nine. Split further only if a tenth arrives.
#[allow(clippy::too_many_arguments)]
fn show<'a>(
    color: Option<Cow<'a, str>>,
    image: Option<Cow<'a, str>>,
    directory: Option<Cow<'a, str>>,
    every: Option<Cow<'a, str>>,
    shuffle: Option<bool>,
    sha256: Option<Cow<'a, str>>,
    mode: Option<Cow<'a, str>>,
    fill: Option<Cow<'a, str>>,
    filter: Option<Cow<'a, str>>,
    animate: Option<Cow<'a, str>>,
) -> Result<Show<'a>, RequestError> {
    let shuffle = shuffle.unwrap_or(false);
    if let Some(dir) = directory {
        if color.is_some() || image.is_some() {
            return Err(RequestError::Both);
        }
        if sha256.is_some() {
            return Err(RequestError::ShaWithDirectory);
        }
        if !dir.starts_with('/') {
            return Err(RequestError::RelativePath(dir.into_owned()));
        }
        let every = every.ok_or(RequestError::EveryMissing)?;
        let every_secs = rotation::parse_every(&every).map_err(RequestError::BadEvery)?;
        if animate.is_some() {
            return Err(RequestError::AnimateWithSlideshow);
        }
        let (mode, fill, filter) = look(mode, fill, filter)?;
        return Ok(Show::Slideshow(SlideshowRequest {
            dir,
            every_secs,
            shuffle,
            mode,
            fill,
            filter,
        }));
    }
    if every.is_some() {
        return Err(RequestError::EveryWithoutDirectory);
    }
    if shuffle {
        return Err(RequestError::ShuffleWithoutDirectory);
    }
    match (color, image) {
        (Some(_), Some(_)) => Err(RequestError::Both),
        (None, None) => Err(RequestError::NoTarget),
        (Some(text), None) => {
            for (field, given) in [
                ("mode", &mode),
                ("fill", &fill),
                ("filter", &filter),
                ("sha256", &sha256),
                ("animate", &animate),
            ] {
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
        (None, Some(image)) => {
            let source = if crate::fetch::is_url(&image) {
                if image.contains('\0') {
                    return Err(RequestError::UrlNul);
                }
                let sha256 = sha256
                    .map(|text| {
                        crate::fetch::parse_sha256(&text)
                            .map_err(|_| RequestError::BadSha(text.into_owned()))
                    })
                    .transpose()?;
                Source::Url { url: image, sha256 }
            } else {
                if !image.starts_with('/') {
                    return Err(RequestError::RelativePath(image.into_owned()));
                }
                if sha256.is_some() {
                    return Err(RequestError::ShaWithFile);
                }
                Source::Path(image)
            };
            let (mode, fill, filter) = look(mode, fill, filter)?;
            let animate = match animate.as_deref() {
                None | Some("true") => true,
                Some("false") => false,
                Some(other) => return Err(RequestError::BadAnimate(other.to_owned())),
            };
            Ok(Show::Image(ImageRequest {
                source,
                mode,
                fill,
                filter,
                animate,
            }))
        }
    }
}

/// An image's `mode`, `fill` and `filter`, as a `set` asks for them: the
/// defaults for what is left out.
fn look(
    mode: Option<Cow<'_, str>>,
    fill: Option<Cow<'_, str>>,
    filter: Option<Cow<'_, str>>,
) -> Result<(Mode, Color, Filter), RequestError> {
    let mode = match mode {
        None => Mode::default(),
        Some(name) => {
            Mode::from_name(&name).ok_or_else(|| RequestError::BadMode(name.into_owned()))?
        }
    };
    let filter = match filter {
        None => Filter::default(),
        Some(name) => {
            Filter::from_name(&name).ok_or_else(|| RequestError::BadFilter(name.into_owned()))?
        }
    };
    let fill = match fill {
        None => DEFAULT_FILL,
        Some(text) => Color::parse(&text).map_err(|error| RequestError::BadFill {
            text: text.into_owned(),
            error,
        })?,
    };
    Ok((mode, fill, filter))
}

/// What a `set` transitions through, from its fields: parsed strictly
/// (`crate::transition::assemble`), so parameters without a `transition`
/// are refused rather than silently not applied.
fn transition(
    kind: Option<Cow<'_, str>>,
    duration_ms: Option<Cow<'_, str>>,
    easing: Option<Cow<'_, str>>,
    angle: Option<Cow<'_, str>>,
    position: Option<Cow<'_, str>>,
) -> Result<Spec, RequestError> {
    let kind = kind
        .map(|name| Kind::parse(&name))
        .transpose()
        .map_err(RequestError::BadTransition)?;
    transition::assemble(
        kind,
        duration_ms.as_deref(),
        easing.as_deref(),
        angle.as_deref(),
        position.as_deref(),
        str::to_owned,
    )
    .map_err(RequestError::BadTransition)
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
///  "draw_failed":false,"draw_error":null,"shows":null,
///  "workspace":"2","transition":null}
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
    /// scale an image is drawn at is `surface.scale`.
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
    /// The last attempt to draw what the output should show failed (the
    /// daemon's stderr says why: an image that cannot be decoded, a buffer
    /// too large): `shows` is then `null`, or what it showed before, not
    /// what was asked. Cleared by the next request for it or a new size.
    pub draw_failed: bool,
    /// Why, while `draw_failed`: the error stderr gives after "cannot draw
    /// ... on OUTPUT: " (such as "no such file", or "shared memory: Cannot
    /// allocate memory (os error 12)"). `null` otherwise. For a person or
    /// an agent to read, not to match: the wording is not part of the
    /// protocol.
    pub draw_error: Option<&'a str>,
    /// What the output shows: `{"color":"#rrggbb"}`,
    /// `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
    /// or `null` for nothing (the compositor's own background, or no
    /// surface yet).
    pub shows: Option<Shows<'a>>,
    /// The workspace active on the output now, by the name the compositor
    /// announces ("1", "2", ... on scoot): `null` while unknown (no
    /// workspace wallpaper mapped, no manager, or no events yet). Added
    /// for per-workspace wallpapers within protocol 1; older clients
    /// ignore it.
    pub workspace: Option<&'a str>,
    /// The transition running on the output now (`none` is never reported:
    /// it lands at once): `fade`, `wipe` or `grow`, else `null`. Added for
    /// transitions within protocol 1; older clients ignore it.
    pub transition: Option<&'a str>,
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
                let mut map =
                    serializer.serialize_map(Some(if image.fetch.is_some() { 5 } else { 4 }))?;
                map.serialize_entry("image", &image.path)?;
                if let Some(fetch) = &image.fetch {
                    map.serialize_entry("url", &fetch.url)?;
                }
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
/// output reported a mode. `scale` is the scale a full-size buffer (an
/// image, or a color on the full-size fallback) is drawn at on the surface
/// (`Output::scale`): `wp_fractional_scale_v1`'s (1.5, say) where the
/// compositor sent one and has a viewporter, unless it is found stale;
/// else the larger of `wl_surface.preferred_buffer_scale` and
/// `wl_output`'s. A color on a single-pixel or 1×1 buffer is drawn at 1
/// whatever this says. `pixels` is the size of a full-size buffer at it (in
/// device pixels: 1601×1001 for scoot's 1067×667 at 1.5). Both `null`
/// whenever `size` is.
#[derive(Debug, Serialize)]
pub struct SurfaceEntry {
    pub state: &'static str,
    pub size: Option<Size>,
    pub scale: Option<crate::density::Scale>,
    pub pixels: Option<Size>,
}

/// A running slideshow, as a `query` reply reports it: what directory it
/// cycles, every how many seconds, shuffled or in order, and how many files
/// that is. Which file shows now is each output's `shows`, as for a `set`.
/// Added within protocol 1; older clients ignore it. Absent while no
/// slideshow runs, so a static wallpaper's reply is byte-for-byte what it
/// was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RotationInfo<'a> {
    pub directory: &'a str,
    pub every_secs: u64,
    pub shuffle: bool,
    pub files: usize,
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

/// One workspace mapping in a `query` reply: the wallpaper for
/// `workspace` on `output` (`null`: every output) while that workspace is
/// active there. Only live mappings are listed (cleared ones, kept for
/// ordering against older requests, show nothing and are listed nowhere).
/// Added for per-workspace wallpapers within protocol 1; older clients
/// ignore the whole list.
#[derive(Debug, Serialize)]
pub struct WorkspaceEntry<'a> {
    /// The connector name the mapping is for, `null` for every output.
    pub output: Option<&'a str>,
    /// The workspace, by the name the compositor announces.
    pub workspace: &'a str,
    /// What those outputs show while the workspace is active: `shows`'s
    /// shape (`{"color":...}` or `{"image":...}`).
    pub shows: Shows<'a>,
}

/// The daemon's workspace mappings, as a `query` reply lists them.
pub trait WorkspaceList {
    /// Calls `each` with every live mapping's entry, in order made.
    fn for_each_entry(&self, each: &mut dyn FnMut(&WorkspaceEntry<'_>));
}

/// For fixed lists, as in tests (a slice cannot be a `dyn` value).
impl<const N: usize> WorkspaceList for [WorkspaceEntry<'_>; N] {
    fn for_each_entry(&self, each: &mut dyn FnMut(&WorkspaceEntry<'_>)) {
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
        /// The live per-workspace mappings (`set-workspace`): empty when
        /// none. Added within protocol 1; older clients ignore it.
        #[serde(serialize_with = "workspace_list")]
        workspaces: &'a dyn WorkspaceList,
        /// Whether `set` and `clear` are saved for the next start
        /// (`crate::state`): `false` when saving is off (no state
        /// directory, or a state file that could not be read or is a newer
        /// scootbg's; the daemon's stderr said why at start-up).
        saving: bool,
        /// The profile whose state the daemon restores and saves: its
        /// `--profile`, or the last one an `apply-config` made it adopt.
        profile: &'a str,
        /// The slideshow running now, if any (`crate::rotation`).
        #[serde(skip_serializing_if = "Option::is_none")]
        rotation: Option<RotationInfo<'a>>,
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

/// Serializes a [`WorkspaceList`] as a JSON array, entry by entry.
fn workspace_list<S: Serializer>(
    value: &&dyn WorkspaceList,
    serializer: S,
) -> Result<S::Ok, S::Error> {
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
