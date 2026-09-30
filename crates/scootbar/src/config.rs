//! Everything `scootbar daemon` is told: the config file, with the flags
//! as overrides.
//!
//! The file is `$XDG_CONFIG_HOME/scoot/bar.toml` (`~/.config/scoot/bar.toml`
//! when `XDG_CONFIG_HOME` is unset or empty), or what `--config` names. It
//! is the bar's own file, separate from scoot's `config.toml`, so it works
//! on other compositors and a bar change never breaks scoot. A missing file
//! is the defaults; a malformed one is a loud refusal, and a bad reload
//! keeps the running config.
//!
//! Precedence is defaults, then the file, then the flags: giving any of
//! `--left`/`--center`/`--right` replaces just those sections of the
//! file's layout, and any other flag replaces its own value.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::bar::{Bar, Edge, Layer, MAX_HEIGHT, Margin};
use crate::color::Color;
use crate::layout::{Layout, MAX_GAP, Section, check_placement};
use crate::modules::Settings;
use crate::render::Style;
use crate::theme::Theme;

#[cfg(test)]
mod tests;

/// The em when `--font-size` is not given, in logical pixels.
pub const DEFAULT_FONT_SIZE: u32 = 14;
/// The largest `--font-size`: past it a glyph is too big to cache (it is
/// still drawn), and no bar wants one that tall.
pub const MAX_FONT_SIZE: u32 = 256;
/// The largest `bar.radius`: half the tallest bar.
pub const MAX_RADIUS: u32 = MAX_HEIGHT / 2;
/// The largest config file read: a real one is about a kilobyte, so
/// anything past this is not one. Bounds what a reload buffers.
pub const MAX_FILE: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bar: Bar,
    pub theme: Theme,
    pub layout: Layout,
    /// `--font`; `None` looks in the well-known places (`crate::font`).
    pub font: Option<PathBuf>,
    /// The em, in logical pixels, 1 to [`MAX_FONT_SIZE`].
    pub font_size: u32,
    /// The bar's corner radius, in logical pixels: 0 to [`MAX_RADIUS`], and
    /// at most half the height.
    pub radius: u32,
    /// The background's alpha: 255 opaque, 0 transparent.
    pub opacity: u8,
    pub modules: Settings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bar: Bar::default(),
            theme: Theme::default(),
            layout: Layout::default(),
            font: None,
            font_size: DEFAULT_FONT_SIZE,
            radius: 0,
            opacity: u8::MAX,
            modules: Settings::default(),
        }
    }
}

impl Config {
    pub fn style(&self) -> Style {
        Style {
            theme: self.theme,
            font_size: self.font_size,
            padding: self.layout.padding,
            spacing: self.layout.spacing,
            radius: self.radius,
            opacity: self.opacity,
        }
    }
}

/// The config file's home: `$XDG_CONFIG_HOME/scoot/bar.toml`, or
/// `~/.config/scoot/bar.toml` when `XDG_CONFIG_HOME` is unset or empty.
/// `None` when neither is usable (no `HOME` either): there is then no file
/// to read, only the defaults.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(base.join("scoot").join("bar.toml"))
}

/// Why the config file was refused. Every variant names the file; value
/// errors name the key too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Reading the file failed.
    Read { path: PathBuf, error: String },
    /// Larger than [`MAX_FILE`].
    TooLarge { path: PathBuf },
    /// Not TOML, or a TOML refusal (an unknown key, a wrong type): the
    /// message names the key and the line.
    Parse { path: PathBuf, error: String },
    /// TOML but not a valid value: `key` is the dotted key (`bar.height`).
    Value {
        path: PathBuf,
        key: &'static str,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "{}: {error}", path.display()),
            Self::TooLarge { path } => write!(
                f,
                "{}: larger than the {} bytes a config file may be, refusing",
                path.display(),
                MAX_FILE
            ),
            Self::Parse { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Value { path, key, message } => {
                write!(f, "{}: '{key}': {message}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

/// What starting (or reloading) with the config file gives: the file's
/// config, and the path a reload re-reads (even when the file was absent:
/// it may exist by then).
pub struct Startup {
    pub config: Config,
    /// `None` only when no path is usable at all (no `HOME` either).
    pub file: Option<PathBuf>,
    /// Whether `config` came from the file: the flags then overlay onto
    /// it section by section, rather than `into_config`'s whole-layout
    /// rule over the defaults.
    pub from_file: bool,
}

/// Reads the file `explicit` names, or the default file: absent is the
/// defaults (for the default path only; an explicit `--config` that names
/// nothing is a refusal), malformed is a loud error.
pub fn load_startup(explicit: Option<&Path>) -> Result<Startup, Error> {
    match explicit {
        Some(path) => Ok(Startup {
            config: read_file(path)?,
            file: Some(path.to_owned()),
            from_file: true,
        }),
        None => match default_path() {
            None => Ok(Startup {
                config: Config::default(),
                file: None,
                from_file: false,
            }),
            Some(path) if !path.exists() => Ok(Startup {
                config: Config::default(),
                file: Some(path),
                from_file: false,
            }),
            Some(path) => Ok(Startup {
                config: read_file(&path)?,
                file: Some(path),
                from_file: true,
            }),
        },
    }
}

/// Re-reads the file a reload re-reads: it must exist and parse, else the
/// running config stands and the error answers the reload.
pub fn reload(file: Option<&Path>) -> Result<Config, Error> {
    match file {
        Some(path) => read_file(path),
        None => Err(Error::Read {
            path: PathBuf::from("bar.toml"),
            error: "no config file to reload: no usable config home, \
                    start with `--config PATH`"
                .to_owned(),
        }),
    }
}

/// Reads and validates `path` in full before returning anything.
fn read_file(path: &Path) -> Result<Config, Error> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| Error::Read {
        path: path.to_owned(),
        error: error.to_string(),
    })?;
    let mut buf = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut buf)
        .map_err(|error| Error::Read {
            path: path.to_owned(),
            error: error.to_string(),
        })?;
    if buf.len() as u64 > MAX_FILE {
        return Err(Error::TooLarge {
            path: path.to_owned(),
        });
    }
    let parsed: File = toml::from_slice(&buf).map_err(|error| Error::Parse {
        path: path.to_owned(),
        error: error.to_string(),
    })?;
    parsed.into_config(path)
}

/// The file's schema: every section optional (absent is the defaults), and
/// every table denies unknown keys, so a misspelled key is a loud error
/// naming it rather than a silently ignored one.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct File {
    bar: BarFile,
    left: Option<Vec<String>>,
    center: Option<Vec<String>>,
    right: Option<Vec<String>>,
    colors: ColorsFile,
    clock: ClockFile,
    workspaces: WorkspacesFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct BarFile {
    edge: Option<String>,
    layer: Option<String>,
    exclusive: Option<bool>,
    height: Option<u32>,
    margin: Option<toml::Value>,
    radius: Option<u32>,
    opacity: Option<toml::Value>,
    font: Option<String>,
    #[serde(rename = "font-size")]
    font_size: Option<u32>,
    padding: Option<u32>,
    spacing: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ColorsFile {
    background: Option<String>,
    foreground: Option<String>,
    accent: Option<String>,
    dim: Option<String>,
    urgent: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ClockFile {
    /// Without the `clock` feature there is no such key: `deny_unknown`
    /// fields then refuses it loudly, as the missing `--clock-format` flag
    /// would.
    #[cfg(feature = "clock")]
    format: Option<String>,
}

/// Reserved for the workspaces module's future options: empty, so any key
/// in it is a loud error naming the key.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct WorkspacesFile {}

impl File {
    /// Validates every value and fills the defaults: `Err` names the
    /// dotted key and says what it takes.
    fn into_config(self, path: &Path) -> Result<Config, Error> {
        let defaults = Config::default();
        let bar = &self.bar;
        let edge = match &bar.edge {
            None => defaults.bar.edge,
            Some(text) => Edge::parse(text).ok_or_else(|| {
                value(
                    path,
                    "bar.edge",
                    format_args!("takes top or bottom, not `{text}`"),
                )
            })?,
        };
        let layer = match &bar.layer {
            None => defaults.bar.layer,
            Some(text) => Layer::parse(text).ok_or_else(|| {
                value(
                    path,
                    "bar.layer",
                    format_args!("takes bottom, top or overlay, not `{text}`"),
                )
            })?,
        };
        let exclusive = bar.exclusive.unwrap_or(defaults.bar.exclusive);
        let height = match bar.height {
            None => defaults.bar.height,
            Some(height) if (1..=MAX_HEIGHT).contains(&height) => height,
            Some(height) => {
                return Err(value(
                    path,
                    "bar.height",
                    format_args!(
                        "takes a whole number of logical pixels from 1 to {MAX_HEIGHT}, \
                         not `{height}`"
                    ),
                ));
            }
        };
        let margin = match &bar.margin {
            None => defaults.bar.margin,
            Some(margin) => parse_margin(margin)
                .map_err(|message| value(path, "bar.margin", format_args!("{message}")))?,
        };
        let radius = match bar.radius {
            None => defaults.radius,
            Some(radius) if radius <= MAX_RADIUS && radius <= height / 2 => radius,
            Some(radius) => {
                return Err(value(
                    path,
                    "bar.radius",
                    format_args!(
                        "takes a whole number of logical pixels from 0 to half the height \
                         ({}), not `{radius}`",
                        (height / 2).min(MAX_RADIUS)
                    ),
                ));
            }
        };
        let opacity = match &bar.opacity {
            None => defaults.opacity,
            Some(opacity) => parse_opacity(opacity).ok_or_else(|| {
                value(
                    path,
                    "bar.opacity",
                    format_args!("takes a number from 0 (transparent) to 1 (opaque)"),
                )
            })?,
        };
        let font = bar.font.as_ref().map(PathBuf::from);
        let font_size = match bar.font_size {
            None => defaults.font_size,
            Some(size) if (1..=MAX_FONT_SIZE).contains(&size) => size,
            Some(size) => {
                return Err(value(
                    path,
                    "bar.font-size",
                    format_args!(
                        "takes a whole number of logical pixels from 1 to {MAX_FONT_SIZE}, \
                         not `{size}`"
                    ),
                ));
            }
        };
        let padding = gap(path, "bar.padding", bar.padding, defaults.layout.padding)?;
        let spacing = gap(path, "bar.spacing", bar.spacing, defaults.layout.spacing)?;
        let layout = self.layout(path, padding, spacing, &defaults.layout)?;
        let theme = self.theme(path, &defaults.theme)?;
        #[cfg_attr(not(feature = "clock"), allow(unused_mut))]
        let mut modules = defaults.modules;
        #[cfg(feature = "clock")]
        if let Some(format) = &self.clock.format {
            use crate::modules::clock::format::Format;
            modules.clock.format = Format::parse(format).map_err(|error| {
                value(
                    path,
                    "clock.format",
                    format_args!("{}: {error}", format.escape_debug()),
                )
            })?;
        }
        Ok(Config {
            bar: Bar {
                edge,
                height,
                margin,
                layer,
                exclusive,
            },
            theme,
            layout,
            font,
            font_size,
            radius,
            opacity,
            modules,
        })
    }

    /// The module lists: giving any of the three sets the whole layout, as
    /// the flags do (a part not given is empty).
    fn layout(
        &self,
        path: &Path,
        padding: u32,
        spacing: u32,
        defaults: &Layout,
    ) -> Result<Layout, Error> {
        let layout = if self.left.is_none() && self.center.is_none() && self.right.is_none() {
            Layout {
                padding,
                spacing,
                ..defaults.clone()
            }
        } else {
            Layout {
                left: ids(path, Section::Left, &self.left)?,
                center: ids(path, Section::Center, &self.center)?,
                right: ids(path, Section::Right, &self.right)?,
                padding,
                spacing,
            }
        };
        check_placement(&layout).map_err(|(section, error)| Error::Value {
            path: path.to_owned(),
            key: section_key(section),
            message: error.to_string(),
        })?;
        Ok(layout)
    }

    fn theme(&self, path: &Path, defaults: &Theme) -> Result<Theme, Error> {
        let colors = &self.colors;
        let color = |key: &'static str, text: Option<&String>, fallback: Color| match text {
            None => Ok(fallback),
            Some(text) => Color::parse(text).map_err(|error| Error::Value {
                path: path.to_owned(),
                key,
                message: format!("{text:?}: {error}"),
            }),
        };
        Ok(Theme {
            background: color(
                "colors.background",
                colors.background.as_ref(),
                defaults.background,
            )?,
            foreground: color(
                "colors.foreground",
                colors.foreground.as_ref(),
                defaults.foreground,
            )?,
            accent: color("colors.accent", colors.accent.as_ref(), defaults.accent)?,
            dim: color("colors.dim", colors.dim.as_ref(), defaults.dim)?,
            urgent: color("colors.urgent", colors.urgent.as_ref(), defaults.urgent)?,
        })
    }
}

/// A value error at `key`: what it takes, in `message`.
fn value(path: &Path, key: &'static str, message: impl fmt::Display) -> Error {
    Error::Value {
        path: path.to_owned(),
        key,
        message: message.to_string(),
    }
}

/// A `left`/`center`/`right` list: every id must name a module of this
/// build.
fn ids(
    path: &Path,
    section: Section,
    ids: &Option<Vec<String>>,
) -> Result<Vec<&'static str>, Error> {
    let mut placed = Vec::new();
    for id in ids.as_ref().map(Vec::as_slice).unwrap_or_default() {
        let Some(spec) = crate::modules::find(id) else {
            let mut has = String::new();
            if crate::modules::REGISTRY.is_empty() {
                has.push_str(" none");
            }
            for spec in crate::modules::REGISTRY {
                has.push(' ');
                has.push_str(spec.id);
            }
            return Err(Error::Value {
                path: path.to_owned(),
                key: section_key(section),
                message: format!(
                    "no module `{}` in this build (it has:{has})",
                    id.escape_debug()
                ),
            });
        };
        if placed.len() >= crate::layout::MAX_MODULES {
            return Err(Error::Value {
                path: path.to_owned(),
                key: section_key(section),
                message: format!("at most {} modules", crate::layout::MAX_MODULES),
            });
        }
        placed.push(spec.id);
    }
    Ok(placed)
}

/// The dotted key a section's list lives under.
fn section_key(section: Section) -> &'static str {
    match section {
        Section::Left => "left",
        Section::Center => "center",
        Section::Right => "right",
    }
}

/// A `padding`/`spacing` value: 0 to [`MAX_GAP`].
fn gap(path: &Path, key: &'static str, given: Option<u32>, fallback: u32) -> Result<u32, Error> {
    match given {
        None => Ok(fallback),
        Some(gap) if gap <= MAX_GAP => Ok(gap),
        Some(gap) => Err(Error::Value {
            path: path.to_owned(),
            key,
            message: format!(
                "takes a whole number of logical pixels from 0 to {MAX_GAP}, not `{gap}`"
            ),
        }),
    }
}

/// `bar.margin`: an integer (every side) or the CSS shorthand string the
/// `--margin` flag takes. TOML has no convenient tuple for the shorthand,
/// and an integer is what a file wants to say for one value.
fn parse_margin(value: &toml::Value) -> Result<Margin, String> {
    match value {
        toml::Value::Integer(sides) => {
            let sides = u32::try_from(*sides).map_err(|_| {
                "takes one to four whole numbers separated by commas, or one number alone"
                    .to_owned()
            })?;
            Margin::parse(&sides.to_string()).map_err(|error| error.to_string())
        }
        toml::Value::String(text) => Margin::parse(text).map_err(|error| error.to_string()),
        _ => Err(
            "takes one to four whole numbers separated by commas, or one number alone".to_owned(),
        ),
    }
}

/// `bar.opacity`: a number from 0 to 1 (an integer is 0 or 1), as alpha
/// 0 to 255, rounded. NaN and out-of-range values are refused.
fn parse_opacity(value: &toml::Value) -> Option<u8> {
    let opacity = match value {
        toml::Value::Float(float) => *float,
        toml::Value::Integer(integer) => *integer as f64,
        _ => return None,
    };
    (0.0..=1.0)
        .contains(&opacity)
        .then(|| (opacity * 255.0).round() as u8)
}
