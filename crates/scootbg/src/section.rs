//! A `[wallpaper]` section from scoot's config, as `scootbg apply-config`
//! takes it: JSON, validated strictly, and fingerprinted over a canonical
//! encoding (docs/scootbg/backlog/resolved/scoot-integration-done.md).
//!
//! ```text
//! {"image":"/home/me/hills.jpg","mode":"fill",
//!  "output":{"DP-2":{"color":"#101014"}},
//!  "command":"/nix/store/...-scootbg/bin/scootbg"}
//! ```
//!
//! **The schema.** One object. At the top, the wallpaper for every output:
//! `image` (an absolute path, or an `http(s)` URL, downloaded once and
//! cached: `crate::fetch`) or `color` (`#rrggbb`), never both, with `mode`,
//! `fill` and `filter` for an image only, as `scootbg set` takes them, and
//! `sha256` (64 hex digits, pinning a download's bytes) for a URL image
//! only. Neither is nothing: the compositor's own background. `output` is
//! an object of per-output tables, by connector name, each the same six
//! keys and nothing else; an empty one means nothing on that output.
//! **Each table stands alone**, as a `scootbg set` does: an output's image
//! does not take the top level's `mode`. `command` is scoot's (where to
//! find the binary): accepted, and ignored.
//!
//! **Strict.** Anything else is refused, so a typo is an error rather than a
//! setting silently not applied: an unknown key (at either level), a key
//! given twice, a `null` or a value of the wrong type, an array where an
//! object belongs, a relative path (scoot resolves `~/` and relative paths
//! against its config file before sending; scootbg's working directory is
//! not the config's), a URL that is not `http(s)`, a path with a NUL byte or
//! longer than Linux opens (`PATH_MAX`), a URL past [`crate::fetch::MAX_URL`],
//! a malformed color or hash, an unknown mode or filter, an empty
//! output name, more than [`MAX_OUTPUTS`] outputs, or more than
//! [`MAX_SECTION`] bytes of JSON.
//!
//! **The fingerprint** is SHA-256, in lowercase hex, of the canonical
//! encoding: the section re-encoded as compact JSON, keys in byte order at
//! both levels, strings as given (a color's case included) and escaped the
//! way `serde_json` escapes them, `command` left out, and every other key
//! present in the input present in it (an `output` of `{}` included). So the
//! sender's key order never matters (scoot's per-output tables come from a
//! `HashMap`), while a change to any value does, and `command` (a store path
//! that changes with every upgrade under home-manager) never does.

use std::fmt;
use std::path::Path;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, de::value::MapAccessDeserializer};

use crate::color::{Color, ColorError};
use crate::fetch::{Fetch, MAX_URL, is_url};
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::protocol::{DEFAULT_FILL, MAX_REQUEST_LINE, PROTOCOL_VERSION};
use crate::state::Profile;
use crate::state::format::{MAX_OUTPUTS, Pick, Record};

#[cfg(test)]
mod tests;

/// The most JSON a section may be, in bytes: what fits one request line
/// with room for the rest of it. The canonical encoding is never longer
/// than the JSON it came from (it drops whitespace, and escapes nothing a
/// JSON string did not have to escape already).
pub const MAX_SECTION: usize = MAX_REQUEST_LINE - 1024;

/// The longest image path, in bytes: Linux's `PATH_MAX` less its NUL.
pub const MAX_PATH: usize = 4095;

/// A validated section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    all: Table,
    /// `None` when the section has no `output` key; sorted by name.
    outputs: Option<Vec<(String, Table)>>,
}

/// One table: the strings as given, for the canonical encoding, and what
/// they mean.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Table {
    raw: Raw,
    pick: Chosen,
}

/// What one table chooses, validated but not yet resolved: a URL becomes
/// its cache file only in [`Section::record`], once the cache directory is
/// known (resolving it at parse time would freeze one machine's
/// `XDG_CACHE_HOME` into the choice).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chosen {
    Clear,
    Color(Color),
    Image { source: ImageSource, look: Look },
}

/// Where a chosen image comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageSource {
    Path(String),
    Url { url: String, sha256: Option<[u8; 32]> },
}

impl Chosen {
    /// Resolves the choice into a [`Pick`]: a URL into its file in
    /// `cache`.
    fn record(&self, cache: &Path) -> Pick {
        match self {
            Self::Clear => Pick::Clear,
            Self::Color(color) => Pick::Color(*color),
            Self::Image { source, look } => match source {
                ImageSource::Path(path) => Pick::Image {
                    path: path.clone(),
                    look: *look,
                    fetch: None,
                },
                ImageSource::Url { url, sha256 } => Pick::Image {
                    path: crate::fetch::cached_path(cache, url)
                        .to_string_lossy()
                        .into_owned(),
                    look: *look,
                    fetch: Some(Fetch {
                        url: url.clone(),
                        sha256: *sha256,
                    }),
                },
            },
        }
    }
}

/// A table's keys as given. Field order is the canonical key order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default, deserialize_with = "present")]
    color: Option<String>,
    #[serde(default, deserialize_with = "present")]
    fill: Option<String>,
    #[serde(default, deserialize_with = "present")]
    filter: Option<String>,
    #[serde(default, deserialize_with = "present")]
    image: Option<String>,
    #[serde(default, deserialize_with = "present")]
    mode: Option<String>,
    #[serde(default, deserialize_with = "present")]
    sha256: Option<String>,
}

/// The top level as given: a table's keys, `output` and `command`.
/// (Spelled out rather than `#[serde(flatten)]`, which does not work with
/// `deny_unknown_fields`.)
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSection {
    #[serde(default, deserialize_with = "present")]
    color: Option<String>,
    #[serde(default, deserialize_with = "present")]
    fill: Option<String>,
    #[serde(default, deserialize_with = "present")]
    filter: Option<String>,
    #[serde(default, deserialize_with = "present")]
    image: Option<String>,
    #[serde(default, deserialize_with = "present")]
    mode: Option<String>,
    #[serde(default, deserialize_with = "present")]
    sha256: Option<String>,
    #[serde(default, deserialize_with = "outputs")]
    output: Option<Vec<(String, Raw)>>,
    /// scoot's; accepted and ignored (never part of the fingerprint).
    #[serde(default, deserialize_with = "present", rename = "command")]
    _command: Option<String>,
}

/// A string that is there: `null` is refused, not read as absent (only a
/// missing key is absent, through `#[serde(default)]`).
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

/// `T` from a JSON object and nothing else: a derived `Deserialize` would
/// also read a struct from an array (`["/a.png"]`).
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Only<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Only<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }
        }
        deserializer.deserialize_map(Only(std::marker::PhantomData))
    }
}

/// `output`: an object of tables, by name, kept sorted by name as they are
/// read (the canonical order; a binary search finds a name given twice). A
/// name given twice, an empty name, or more than [`MAX_OUTPUTS`] names is
/// refused, the last as soon as it is seen, so the work stays bounded.
fn outputs<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<(String, Raw)>>, D::Error> {
    struct Tables;
    impl<'de> Visitor<'de> for Tables {
        type Value = Vec<(String, Raw)>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of per-output tables, by connector name")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut tables: Vec<(String, Raw)> = Vec::new();
            while let Some(name) = map.next_key::<String>()? {
                if name.is_empty() {
                    return Err(de::Error::custom("an output name is empty"));
                }
                let Err(at) = tables.binary_search_by(|(n, _)| n.as_str().cmp(&name)) else {
                    return Err(de::Error::custom(format_args!(
                        "output {name:?} is given twice"
                    )));
                };
                if tables.len() == MAX_OUTPUTS {
                    return Err(de::Error::custom(format_args!(
                        "more than {MAX_OUTPUTS} outputs"
                    )));
                }
                let Object(table) = map.next_value::<Object<Raw>>()?;
                tables.insert(at, (name, table));
            }
            Ok(tables)
        }
    }
    deserializer.deserialize_map(Tables).map(Some)
}

/// Why a section was refused, past what JSON parsing itself says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionError {
    /// Over [`MAX_SECTION`] bytes.
    TooLarge(usize),
    /// Not JSON, or not this schema (serde's message).
    Json(String),
    /// A table with an `image` and a `color`.
    Both(At),
    /// `mode`, `fill` or `filter` in a table without an `image`.
    ImageOnly(At, &'static str),
    RelativePath(At, String),
    PathNul(At),
    PathTooLong(At, usize),
    Color {
        at: At,
        key: &'static str,
        text: String,
        error: ColorError,
    },
    Mode(At, String),
    Filter(At, String),
    /// A `sha256` that is not 64 hex digits.
    Sha(At, String),
    /// A `sha256` with a file: it pins a download. (With a color it is
    /// the generic image-only refusal, like `mode`.)
    ShaImageOnly(At),
    /// An image URL that is not `http(s)`.
    Scheme(At, String),
    /// A URL past [`crate::fetch::MAX_URL`].
    UrlTooLong(At, usize),
    /// A URL with a NUL byte.
    UrlNul(At),
}

/// Which table, for a message: the top level, or an output's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At(Option<String>);

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => f.write_str("[wallpaper]"),
            Some(name) => write!(f, "[wallpaper.output.{name:?}]"),
        }
    }
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(len) => write!(
                f,
                "the section is {len} bytes of JSON; at most {MAX_SECTION} are taken"
            ),
            Self::Json(message) => write!(f, "{message}"),
            Self::Both(at) => write!(f, "{at}: `image` or `color`, not both"),
            Self::ImageOnly(at, key) => {
                write!(f, "{at}: `{key}` applies to an `image`, and there is none")
            }
            Self::RelativePath(at, path) => write!(
                f,
                "{at}: the image path {path:?} is not absolute (scoot makes `~/` and \
                 relative paths absolute before it runs apply-config)"
            ),
            Self::PathNul(at) => write!(f, "{at}: the image path has a NUL byte"),
            Self::PathTooLong(at, len) => write!(
                f,
                "{at}: the image path is {len} bytes, longer than Linux opens ({MAX_PATH})"
            ),
            Self::Color {
                at,
                key,
                text,
                error,
            } => write!(f, "{at}: `{key}` {text:?}: {error}"),
            Self::Mode(at, mode) => write!(
                f,
                "{at}: unknown mode {mode:?}: fill, fit, stretch, center or tile"
            ),
            Self::Filter(at, filter) => write!(
                f,
                "{at}: unknown filter {filter:?}: lanczos3, catmull-rom, bilinear or nearest"
            ),
            Self::Sha(at, text) => write!(
                f,
                "{at}: the sha256 {text:?} is not 64 hex digits (as `sha256sum` prints)"
            ),
            Self::ShaImageOnly(at) => write!(
                f,
                "{at}: `sha256` pins a downloaded image, and this one is not a URL"
            ),
            Self::Scheme(at, url) => {
                let scheme = url.split("://").next().unwrap_or_default();
                if scheme == "file" {
                    write!(
                        f,
                        "{at}: the image {url:?} is a `file://` URL: give the path itself \
                         (it starts with `/`)"
                    )
                } else {
                    write!(
                        f,
                        "{at}: the image URL scheme {scheme:?} is not fetched (`http` and \
                         `https` are)"
                    )
                }
            }
            Self::UrlTooLong(at, len) => write!(
                f,
                "{at}: the image URL is {len} bytes, longer than the {} taken",
                crate::fetch::MAX_URL
            ),
            Self::UrlNul(at) => write!(f, "{at}: the image URL has a NUL byte"),
        }
    }
}

impl std::error::Error for SectionError {}

impl Section {
    /// Parses and validates `json`, a whole section.
    pub fn parse(json: &[u8]) -> Result<Self, SectionError> {
        if json.len() > MAX_SECTION {
            return Err(SectionError::TooLarge(json.len()));
        }
        serde_json::from_slice::<Self>(json).map_err(|error| SectionError::Json(error.to_string()))
    }

    fn validate(raw: RawSection) -> Result<Self, SectionError> {
        let table = Raw {
            color: raw.color,
            fill: raw.fill,
            filter: raw.filter,
            image: raw.image,
            mode: raw.mode,
            sha256: raw.sha256,
        };
        let all = Table::validate(table, At(None))?;
        let outputs = match raw.output {
            None => None,
            // Sorted by name as read (`outputs`).
            Some(tables) => {
                let mut valid = Vec::with_capacity(tables.len());
                for (name, table) in tables {
                    let table = Table::validate(table, At(Some(name.clone())))?;
                    valid.push((name, table));
                }
                Some(valid)
            }
        };
        Ok(Self { all, outputs })
    }

    /// The canonical encoding (see the module docs).
    pub fn canonical(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        let mut first = true;
        self.all.raw.encode(&mut out, &mut first);
        if let Some(outputs) = &self.outputs {
            key(&mut out, &mut first, "output");
            out.push('{');
            let mut first_output = true;
            for (name, table) in outputs {
                key(&mut out, &mut first_output, name);
                out.push('{');
                let mut first_key = true;
                table.raw.encode(&mut out, &mut first_key);
                out.push('}');
            }
            out.push('}');
        }
        out.push('}');
        out
    }

    /// The fingerprint: SHA-256 of [`Section::canonical`], in hex.
    pub fn fingerprint(&self) -> String {
        crate::sha256::hex(self.canonical().as_bytes())
    }

    /// Nothing to show anywhere: no `image` or `color` at the top, and no
    /// per-output table (`{}`, or only `command`).
    pub fn is_empty(&self) -> bool {
        self.all.pick == Chosen::Clear && self.outputs.as_ref().is_none_or(Vec::is_empty)
    }

    /// What it chooses, as the state file would record it: the choice for
    /// every output (nothing, when it names none), then each output's.
    /// `cache` turns a URL into its file; a file choice needs no cache.
    pub fn record(&self, cache: &Path) -> Record {
        Record {
            profile: None,
            fingerprint: None,
            all: Some(self.all.pick.record(cache)),
            named: self
                .outputs
                .iter()
                .flatten()
                .map(|(name, table)| (name.clone(), table.pick.record(cache)))
                .collect(),
        }
    }

    /// The `apply-config` request line for `profile`, newline included.
    pub fn request_line(&self, profile: &Profile) -> String {
        let mut line = String::with_capacity(128);
        line.push_str("{\"protocol\":");
        line.push_str(&PROTOCOL_VERSION.to_string());
        line.push_str(",\"type\":\"apply-config\",\"profile\":");
        string(&mut line, profile.as_str());
        line.push_str(",\"config\":");
        line.push_str(&self.canonical());
        line.push_str("}\n");
        line
    }
}

impl<'de> Deserialize<'de> for Section {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Object(raw) = Object::<RawSection>::deserialize(deserializer)?;
        Self::validate(raw).map_err(de::Error::custom)
    }
}

impl Table {
    fn validate(raw: Raw, at: At) -> Result<Self, SectionError> {
        let pick = match (&raw.image, &raw.color) {
            (Some(_), Some(_)) => return Err(SectionError::Both(at)),
            (None, color) => {
                for (key, given) in [
                    ("mode", &raw.mode),
                    ("fill", &raw.fill),
                    ("filter", &raw.filter),
                    ("sha256", &raw.sha256),
                ] {
                    if given.is_some() {
                        return Err(SectionError::ImageOnly(at, key));
                    }
                }
                match color {
                    None => Chosen::Clear,
                    Some(text) => Chosen::Color(parse_color(&at, "color", text)?),
                }
            }
            (Some(image), None) => {
                let look = parse_look(&raw, &at)?;
                let source = if is_url(image) {
                    if image.contains('\0') {
                        return Err(SectionError::UrlNul(at));
                    }
                    if image.len() > MAX_URL {
                        return Err(SectionError::UrlTooLong(at, image.len()));
                    }
                    let sha256 = match &raw.sha256 {
                        None => None,
                        Some(text) => Some(
                            crate::fetch::parse_sha256(text)
                                .map_err(|_| SectionError::Sha(at.clone(), text.clone()))?,
                        ),
                    };
                    ImageSource::Url {
                        url: image.clone(),
                        sha256,
                    }
                } else {
                    if image.contains("://") {
                        return Err(SectionError::Scheme(at, image.clone()));
                    }
                    if raw.sha256.is_some() {
                        return Err(SectionError::ShaImageOnly(at));
                    }
                    if !image.starts_with('/') {
                        return Err(SectionError::RelativePath(at, image.clone()));
                    }
                    if image.contains('\0') {
                        return Err(SectionError::PathNul(at));
                    }
                    if image.len() > MAX_PATH {
                        return Err(SectionError::PathTooLong(at, image.len()));
                    }
                    ImageSource::Path(image.clone())
                };
                Chosen::Image { source, look }
            }
        };
        Ok(Self { raw, pick })
    }
}

impl Raw {
    /// Appends the keys given, in canonical order, each preceded by a
    /// comma unless `first`.
    fn encode(&self, out: &mut String, first: &mut bool) {
        for (name, value) in [
            ("color", &self.color),
            ("fill", &self.fill),
            ("filter", &self.filter),
            ("image", &self.image),
            ("mode", &self.mode),
            ("sha256", &self.sha256),
        ] {
            if let Some(value) = value {
                key(out, first, name);
                string(out, value);
            }
        }
    }
}

/// `mode`, `fill` and `filter` as a [`Look`].
fn parse_look(raw: &Raw, at: &At) -> Result<Look, SectionError> {
    let mode = match &raw.mode {
        None => Mode::default(),
        Some(name) => {
            Mode::from_name(name).ok_or_else(|| SectionError::Mode(at.clone(), name.clone()))?
        }
    };
    let filter = match &raw.filter {
        None => Filter::default(),
        Some(name) => {
            Filter::from_name(name).ok_or_else(|| SectionError::Filter(at.clone(), name.clone()))?
        }
    };
    let fill = match &raw.fill {
        None => DEFAULT_FILL,
        Some(text) => parse_color(at, "fill", text)?,
    };
    Ok(Look { mode, fill, filter })
}

fn parse_color(at: &At, key: &'static str, text: &str) -> Result<Color, SectionError> {
    Color::parse(text).map_err(|error| SectionError::Color {
        at: at.clone(),
        key,
        text: text.to_owned(),
        error,
    })
}

/// `,"name":` (no comma when `first`, which it then clears).
fn key(out: &mut String, first: &mut bool, name: &str) {
    if !std::mem::take(first) {
        out.push(',');
    }
    string(out, name);
    out.push(':');
}

/// `value` as a JSON string, escaped as `serde_json` escapes it.
fn string(out: &mut String, value: &str) {
    // Serializing a `&str` cannot fail.
    if let Ok(text) = serde_json::to_string(value) {
        out.push_str(&text);
    }
}
