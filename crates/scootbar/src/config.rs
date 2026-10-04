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
use crate::policy::Policy;
use crate::render::Style;
use crate::text::MAX_FALLBACKS;
use crate::theme::Theme;

#[cfg(test)]
mod binding_tests;
mod bindings;
mod custom;
#[cfg(test)]
mod custom_each_tests;
#[cfg(all(test, feature = "button", feature = "push", feature = "exec"))]
mod custom_tests;
#[cfg(any(
    feature = "clock",
    feature = "button",
    feature = "push",
    feature = "exec",
    feature = "volume",
    feature = "microphone",
    feature = "network",
    feature = "battery",
    feature = "brightness",
    feature = "bluetooth",
    feature = "media",
    feature = "window-title",
    feature = "power"
))]
mod icon;
#[cfg(all(test, feature = "clock"))]
mod icon_tests;
mod outputs;
#[cfg(test)]
mod spacing_tests;
#[cfg(test)]
mod tests;

/// The em when `--font-size` is not given, in logical pixels.
pub const DEFAULT_FONT_SIZE: u32 = 14;
/// The largest `--font-size`: past it a glyph is too big to cache (it is
/// still drawn), and no bar wants one that tall.
pub const MAX_FONT_SIZE: u32 = 256;
/// The largest `bar.radius`: half the tallest bar.
pub const MAX_RADIUS: u32 = MAX_HEIGHT / 2;
/// The delay before a tooltip shows when `bar.tooltip-delay` is not given,
/// in milliseconds.
#[cfg(feature = "popup")]
pub const DEFAULT_TOOLTIP_DELAY: u32 = 500;
/// The longest `bar.tooltip-delay`, in milliseconds: ten seconds is already
/// a tooltip nobody waits for.
#[cfg(feature = "popup")]
pub const MAX_TOOLTIP_DELAY: u32 = 10_000;
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
    /// `bar.fallback-fonts`: at most [`MAX_FALLBACKS`] files tried, in
    /// order, for a character the font lacks. No flag: config file only.
    pub fallback_fonts: Vec<PathBuf>,
    /// The em, in logical pixels, 1 to [`MAX_FONT_SIZE`].
    pub font_size: u32,
    /// The bar's corner radius, in logical pixels: 0 to [`MAX_RADIUS`], and
    /// at most half the height.
    pub radius: u32,
    /// The background's alpha: 255 opaque, 0 transparent.
    pub opacity: u8,
    /// `bar.tooltip-delay`: how long the pointer rests on a module with a
    /// tooltip before it shows, in milliseconds; 0 turns tooltips off. No
    /// flag: config file only.
    #[cfg(feature = "popup")]
    pub tooltip_delay: u32,
    pub modules: Settings,
    /// Which outputs get a bar, and their overrides.
    pub outputs: Policy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bar: Bar::default(),
            theme: Theme::default(),
            layout: Layout::default(),
            font: None,
            fallback_fonts: Vec::new(),
            font_size: DEFAULT_FONT_SIZE,
            radius: 0,
            opacity: u8::MAX,
            #[cfg(feature = "popup")]
            tooltip_delay: DEFAULT_TOOLTIP_DELAY,
            modules: Settings::default(),
            outputs: Policy::default(),
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
            separator: self.layout.separator,
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
    /// A refusal at a key that is not one of the fixed ones: a module the
    /// config names (`exec.weather.command`), or a module's icon keys.
    Named {
        path: PathBuf,
        key: String,
        message: String,
    },
    /// The same, in an `[output."NAME"]` table.
    Output {
        path: PathBuf,
        output: String,
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
            Self::Named { path, key, message } => {
                write!(f, "{}: '{key}': {message}", path.display())
            }
            Self::Output {
                path,
                output,
                key,
                message,
            } => write!(
                f,
                "{}: 'output.\"{}\".{key}': {message}",
                path.display(),
                output.escape_debug()
            ),
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
    #[serde(rename = "window-title")]
    window_title: WindowTitleFile,
    /// The volume module's options, and the microphone variant's (the same
    /// keys under `[microphone]`). Without the feature there are none, and
    /// any key is a loud error naming it.
    #[cfg(feature = "volume")]
    volume: VolumeFile,
    #[cfg(feature = "microphone")]
    microphone: VolumeFile,
    /// The battery module's options. Without the feature there are none,
    /// and any key is a loud error naming it.
    #[cfg(feature = "battery")]
    battery: BatteryFile,
    /// The network module's options. Without the `network` feature there
    /// are none, and any key is a loud error naming it.
    network: NetworkFile,
    /// The brightness module's options. Without the `brightness` feature
    /// there are none, and any key is a loud error naming it.
    brightness: BrightnessFile,
    /// The tray module's options. Without the `tray` feature there are
    /// none, and any key is a loud error naming it.
    tray: TrayFile,
    /// The media module's options. Without the `media` feature there are
    /// none, and any key is a loud error naming it.
    media: MediaFile,
    /// The bluetooth module's options. Without the `bluetooth` feature
    /// there are none, and any key is a loud error naming it.
    bluetooth: BluetoothFile,
    /// The power module's options. Without the `power` feature there are
    /// none, and any key is a loud error naming it.
    power: PowerFile,
    /// `[button.NAME]`, `[push.NAME]` and `[exec.NAME]`: modules the file
    /// defines, placed by their names (`custom`).
    #[cfg(feature = "button")]
    button: std::collections::BTreeMap<String, custom::ButtonFile>,
    #[cfg(feature = "push")]
    push: std::collections::BTreeMap<String, custom::PushFile>,
    #[cfg(feature = "exec")]
    exec: std::collections::BTreeMap<String, custom::ExecFile>,
    /// `"all"` or a list of connector names (validated in `outputs`).
    outputs: Option<toml::Value>,
    /// `[output."NAME"]`: what differs on one output.
    output: std::collections::BTreeMap<String, outputs::OutputFile>,
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
    #[serde(rename = "fallback-fonts")]
    fallback_fonts: Option<Vec<String>>,
    #[serde(rename = "font-size")]
    font_size: Option<u32>,
    padding: Option<u32>,
    spacing: Option<u32>,
    separator: Option<u32>,
    /// Without the `popup` feature there is no such key (and so no
    /// tooltip), and `deny_unknown_fields` refuses it loudly.
    #[cfg(feature = "popup")]
    #[serde(rename = "tooltip-delay")]
    tooltip_delay: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ColorsFile {
    background: Option<String>,
    foreground: Option<String>,
    accent: Option<String>,
    hover: Option<String>,
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
    /// One glyph, drawn before the time.
    #[cfg(feature = "clock")]
    icon: Option<String>,
    /// SVG path data, drawn before the time (instead of `icon`).
    #[cfg(feature = "clock")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "clock")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the time (instead of `icon`). Only in a
    /// build with the `icon-image` feature: without it the key is unknown.
    #[cfg(all(feature = "clock", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "clock", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "clock")]
    margin: Option<u32>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "clock")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "clock")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "clock")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "clock")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "clock")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The workspaces module's options. Without the `workspaces` feature there
/// are none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct WorkspacesFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "workspaces")]
    margin: Option<u32>,
    /// The active workspace's pill: `rect`, `pill` or `circle`.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "pill-shape")]
    pill_shape: Option<String>,
    /// A `rect` pill's corner radius, logical pixels.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "pill-radius")]
    pill_radius: Option<u32>,
    /// The pill's gap from the bar's top and bottom, logical pixels.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "pill-inset")]
    pill_inset: Option<u32>,
    /// Spaces between the workspace numbers.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "item-gap")]
    item_gap: Option<u32>,
    /// What the module shows: `numbers` or `dots`.
    #[cfg(feature = "workspaces")]
    display: Option<String>,
    /// With a `circle` pill: grow the module's own span to the disc.
    #[cfg(feature = "workspaces")]
    disc: Option<bool>,
    /// The active pill's fill, instead of the `accent` token.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "active-color")]
    active_color: Option<String>,
    /// Inactive numbers' ink, instead of the `normal` class's token.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "inactive-color")]
    inactive_color: Option<String>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "workspaces")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "workspaces")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "workspaces")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "workspaces")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "workspaces")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The window-title module's options. Without the `window-title` feature
/// there are none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct WindowTitleFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "window-title")]
    margin: Option<u32>,
    /// Whether the app id is shown after the title.
    #[cfg(feature = "window-title")]
    #[serde(rename = "show-app-id")]
    show_app_id: Option<bool>,
    /// One glyph, drawn before the title whenever a window is focused.
    #[cfg(feature = "window-title")]
    icon: Option<String>,
    /// SVG path data, drawn before the title (instead of `icon`).
    #[cfg(feature = "window-title")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "window-title")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the title (instead of `icon`). Only in a
    /// build with the `icon-image` feature: without it the key is unknown.
    #[cfg(all(feature = "window-title", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "window-title", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "window-title")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The most logical pixels wide the module's span may be.
    #[cfg(feature = "window-title")]
    #[serde(rename = "max-width")]
    max_width: Option<u32>,
    /// What shows when no window is focused (empty takes no space).
    #[cfg(feature = "window-title")]
    placeholder: Option<String>,
    /// Whether a middle click (or the `close` action) may close the
    /// focused window. Off by default: an accidental click loses work.
    #[cfg(feature = "window-title")]
    #[serde(rename = "allow-close")]
    allow_close: Option<bool>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "window-title")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "window-title")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "window-title")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "window-title")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "window-title")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The volume module's options (and the microphone variant's, under
/// `[microphone]`). Without the matching feature there are none, and any
/// key is a loud error naming it.
#[cfg(any(feature = "volume", feature = "microphone"))]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct VolumeFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(any(feature = "volume", feature = "microphone"))]
    margin: Option<u32>,
    /// Percent points per scroll notch and per raise: 1 to 50.
    #[cfg(any(feature = "volume", feature = "microphone"))]
    step: Option<u32>,
    /// The cap a raise stops at, in percent: 100 is full scale, past it
    /// is over-amplification, at most 150.
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "max-volume")]
    max_volume: Option<u32>,
    /// One glyph, drawn before the level (instead of the built-in one for
    /// the level).
    #[cfg(any(feature = "volume", feature = "microphone"))]
    icon: Option<String>,
    /// SVG path data, drawn before the level (instead of `icon`).
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the level (instead of `icon`). Only in a
    /// build with the `icon-image` feature: without it the key is unknown.
    #[cfg(all(
        any(feature = "volume", feature = "microphone"),
        feature = "icon-image"
    ))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(
        any(feature = "volume", feature = "microphone"),
        not(feature = "icon-image")
    ))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(any(feature = "volume", feature = "microphone"))]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The network module's options. Without the `network` feature there are
/// none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct NetworkFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "network")]
    margin: Option<u32>,
    /// The interface to show, by name. Absent is the default route's.
    #[cfg(feature = "network")]
    interface: Option<String>,
    /// Whether the SSID is shown (and reported to `query`): the bar is
    /// visible in screenshots and to an agent's `query`.
    #[cfg(feature = "network")]
    #[serde(rename = "show-ssid")]
    show_ssid: Option<bool>,
    /// The picker: spawned with the scan's SSIDs on stdin when the
    /// module is clicked (or its `menu` action runs).
    #[cfg(feature = "network")]
    #[serde(rename = "menu-command")]
    menu_command: Option<Vec<String>>,
    /// Connecting: spawned with the chosen SSID as its last argument when
    /// the list's `connect N` action runs, never through a shell.
    #[cfg(feature = "network")]
    #[serde(rename = "connect-command")]
    connect_command: Option<Vec<String>>,
    /// One glyph, drawn before the text in every state (instead of the
    /// per-state ones below).
    #[cfg(feature = "network")]
    icon: Option<String>,
    /// SVG path data, drawn before the text in every state (instead of
    /// `icon`).
    #[cfg(feature = "network")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "network")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the text in every state (instead of
    /// `icon`). Only in a build with the `icon-image` feature: without it
    /// the key is unknown.
    #[cfg(all(feature = "network", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "network", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// One glyph each, drawn before the text in that state (instead of
    /// `icon`): the wired interface, the tunnel, and no network. A state
    /// with none shows the static icon.
    #[cfg(feature = "network")]
    #[serde(rename = "icon-ethernet")]
    icon_ethernet: Option<String>,
    /// One glyph for every WiFi signal level, or four (weakest to
    /// strongest), picked by the level. Either is drawn before the SSID
    /// instead of `icon`; without it the SSID stands alone. A static
    /// `icon-path`/`icon-image` icon has no levels.
    #[cfg(feature = "network")]
    #[serde(rename = "icon-wifi")]
    icon_wifi: Option<toml::Value>,
    #[cfg(feature = "network")]
    #[serde(rename = "icon-vpn")]
    icon_vpn: Option<String>,
    #[cfg(feature = "network")]
    #[serde(rename = "icon-offline")]
    icon_offline: Option<String>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "network")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "network")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "network")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "network")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "network")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "network")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The brightness module's options. Without the `brightness` feature there
/// are none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct BrightnessFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "brightness")]
    margin: Option<u32>,
    /// The backlight to follow, by class-directory name. Absent is the
    /// first usable device in sorted name order.
    #[cfg(feature = "brightness")]
    device: Option<String>,
    /// Percent points per scroll notch and per raise.
    #[cfg(feature = "brightness")]
    step: Option<u32>,
    /// One glyph for every level, or four (dim to bright), picked by the
    /// level. Either is drawn before the percent instead of a static
    /// icon; without it the percent stands alone. A static
    /// `icon-path`/`icon-image` icon has no levels.
    #[cfg(feature = "brightness")]
    icon: Option<toml::Value>,
    /// SVG path data, drawn before the percent in every state (instead
    /// of `icon`).
    #[cfg(feature = "brightness")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "brightness")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the percent (instead of `icon`). Only in
    /// a build with the `icon-image` feature: without it the key is
    /// unknown.
    #[cfg(all(feature = "brightness", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "brightness", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "brightness")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "brightness")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "brightness")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "brightness")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "brightness")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "brightness")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The battery module's options. Without the `battery` feature there are
/// none, and any key is a loud error naming it.
#[cfg(feature = "battery")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct BatteryFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "battery")]
    margin: Option<u32>,
    /// The percent at or below which the class turns `warn`.
    #[cfg(feature = "battery")]
    #[serde(rename = "warn-below")]
    warn_below: Option<u32>,
    /// The percent at or below which the class turns `urgent`, and the
    /// downward crossing that fires `on-low`.
    #[cfg(feature = "battery")]
    #[serde(rename = "urgent-below")]
    urgent_below: Option<u32>,
    /// Which batteries the level comes from: `combine` (the mean) or
    /// `first` (the first in sorted name order).
    #[cfg(feature = "battery")]
    batteries: Option<String>,
    /// One glyph for every level, or five (empty to full), picked by the
    /// level. Either is drawn before the percent instead of a static
    /// icon; without it the percent stands alone. A static
    /// `icon-path`/`icon-image` icon has no levels.
    #[cfg(feature = "battery")]
    icon: Option<toml::Value>,
    /// SVG path data, drawn before the percent in every state (instead
    /// of `icon`).
    #[cfg(feature = "battery")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "battery")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the percent in every state (instead of
    /// `icon`). Only in a build with the `icon-image` feature: without
    /// it the key is unknown.
    #[cfg(all(feature = "battery", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "battery", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// One glyph, drawn before the percent while charging (instead of
    /// the level's glyph).
    #[cfg(feature = "battery")]
    #[serde(rename = "icon-charging")]
    icon_charging: Option<String>,
    /// One glyph, drawn before the percent while full (instead of the
    /// level's glyph, and instead of the charging one).
    #[cfg(feature = "battery")]
    #[serde(rename = "icon-full")]
    icon_full: Option<String>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "battery")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The low-battery hook: `{ exec = [...] }`, run once per downward
    /// crossing of `urgent-below`.
    #[cfg(feature = "battery")]
    #[serde(rename = "on-low")]
    on_low: Option<toml::Value>,
    /// The interaction keys (`bindings`): a command, since the module
    /// defines no actions of its own.
    #[cfg(feature = "battery")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "battery")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "battery")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "battery")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "battery")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The tray module's options. Without the `tray` feature there are none,
/// and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct TrayFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "tray")]
    margin: Option<u32>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "tray")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "tray")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "tray")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "tray")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "tray")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The media module's options. Without the `media` feature there are none,
/// and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct MediaFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "media")]
    margin: Option<u32>,
    /// The player to prefer, by its short name (`spotify`).
    #[cfg(feature = "media")]
    player: Option<String>,
    /// The most logical pixels wide the module's span may be.
    #[cfg(feature = "media")]
    #[serde(rename = "max-width")]
    max_width: Option<u32>,
    /// One glyph, drawn before the line in both states (instead of the
    /// per-state ones below, and instead of the built-in play and pause
    /// vectors).
    #[cfg(feature = "media")]
    icon: Option<String>,
    /// SVG path data, drawn before the line in both states (instead of
    /// `icon`).
    #[cfg(feature = "media")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "media")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the line in both states (instead of
    /// `icon`). Only in a build with the `icon-image` feature: without it
    /// the key is unknown.
    #[cfg(all(feature = "media", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "media", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// One glyph each, drawn before the line in that state (instead of
    /// `icon`): playing and paused. A state with none shows the static
    /// icon (a stopped player shows nothing, so there is no third key).
    #[cfg(feature = "media")]
    #[serde(rename = "icon-playing")]
    icon_playing: Option<String>,
    #[cfg(feature = "media")]
    #[serde(rename = "icon-paused")]
    icon_paused: Option<String>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "media")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "media")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "media")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "media")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "media")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "media")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

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
        let fallback_fonts: Vec<PathBuf> = match &bar.fallback_fonts {
            None => Vec::new(),
            Some(paths) if paths.len() <= MAX_FALLBACKS => {
                paths.iter().map(PathBuf::from).collect()
            }
            Some(paths) => {
                return Err(value(
                    path,
                    "bar.fallback-fonts",
                    format_args!(
                        "takes at most {MAX_FALLBACKS} font files, not {}",
                        paths.len()
                    ),
                ));
            }
        };
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
        #[cfg(feature = "popup")]
        let tooltip_delay = match bar.tooltip_delay {
            None => defaults.tooltip_delay,
            Some(delay) if delay <= MAX_TOOLTIP_DELAY => delay,
            Some(delay) => {
                return Err(value(
                    path,
                    "bar.tooltip-delay",
                    format_args!(
                        "takes whole milliseconds from 0 (no tooltips) to \
                         {MAX_TOOLTIP_DELAY}, not `{delay}`"
                    ),
                ));
            }
        };
        let padding = gap(path, "bar.padding", bar.padding, defaults.layout.padding)?;
        let spacing = gap(path, "bar.spacing", bar.spacing, defaults.layout.spacing)?;
        let separator = gap(path, "bar.separator", bar.separator, 0)?;
        if separator > spacing {
            return Err(value(
                path,
                "bar.separator",
                format_args!(
                    "is a line drawn in the gap between modules, so it takes at most \
                     bar.spacing ({spacing}), not `{separator}`"
                ),
            ));
        }
        // The modules the file defines by name, before the lists that place
        // them.
        #[allow(unused_mut)]
        let mut tables = custom::Tables::default();
        #[cfg(feature = "button")]
        {
            tables.button = Some(&self.button);
        }
        #[cfg(feature = "push")]
        {
            tables.push = Some(&self.push);
        }
        #[cfg(feature = "exec")]
        {
            tables.exec = Some(&self.exec);
        }
        let defined = custom::read(path, &tables)?;
        let known = defined.names();
        #[allow(unused_mut)]
        let mut margins: Vec<(&'static str, u32)> = defined.margins.clone();
        #[cfg(feature = "clock")]
        if let Some(margin) = self.clock.margin {
            let margin = gap(path, "clock.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::clock::ID, margin));
            }
        }
        #[cfg(feature = "workspaces")]
        if let Some(margin) = self.workspaces.margin {
            let margin = gap(path, "workspaces.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::workspaces::ID, margin));
            }
        }
        #[cfg(feature = "window-title")]
        if let Some(margin) = self.window_title.margin {
            let margin = gap(path, "window-title.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::window_title::ID, margin));
            }
        }
        #[cfg(feature = "volume")]
        if let Some(margin) = self.volume.margin {
            let margin = gap(path, "volume.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::volume::ID, margin));
            }
        }
        #[cfg(feature = "microphone")]
        if let Some(margin) = self.microphone.margin {
            let margin = gap(path, "microphone.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::microphone::ID, margin));
            }
        }
        #[cfg(feature = "network")]
        if let Some(margin) = self.network.margin {
            let margin = gap(path, "network.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::network::ID, margin));
            }
        }
        #[cfg(feature = "brightness")]
        if let Some(margin) = self.brightness.margin {
            let margin = gap(path, "brightness.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::brightness::ID, margin));
            }
        }
        #[cfg(feature = "battery")]
        if let Some(margin) = self.battery.margin {
            let margin = gap(path, "battery.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::battery::ID, margin));
            }
        }
        #[cfg(feature = "tray")]
        if let Some(margin) = self.tray.margin {
            let margin = gap(path, "tray.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::tray::ID, margin));
            }
        }
        #[cfg(feature = "media")]
        if let Some(margin) = self.media.margin {
            let margin = gap(path, "media.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::media::ID, margin));
            }
        }
        #[cfg(feature = "bluetooth")]
        if let Some(margin) = self.bluetooth.margin {
            let margin = gap(path, "bluetooth.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::bluetooth::ID, margin));
            }
        }
        #[cfg(feature = "power")]
        if let Some(margin) = self.power.margin {
            let margin = gap(path, "power.margin", Some(margin), 0)?;
            if margin > 0 {
                margins.push((crate::modules::power::ID, margin));
            }
        }
        let layout = self.layout(
            path,
            Gaps {
                padding,
                spacing,
                separator,
                margins,
            },
            &defaults.layout,
            &known,
        )?;
        let theme = self.theme(path, &defaults.theme)?;
        let outputs = outputs::policy(path, self.outputs.as_ref(), &self.output, &known)?;
        // Only what is placed starts, so only what is placed is counted.
        check_exec_placed(path, &outputs.to_start(&layout), &defined)?;
        #[allow(unused_mut)]
        let mut modules = defaults.modules;
        modules.bindings.extend(defined.bindings);
        modules.custom = defined.modules;
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
        #[cfg(feature = "workspaces")]
        {
            use crate::modules::workspaces::{Display, Shape};
            let pill = &mut modules.workspaces.pill;
            if let Some(text) = &self.workspaces.pill_shape {
                pill.shape = Shape::parse(text).ok_or_else(|| {
                    value(
                        path,
                        "workspaces.pill-shape",
                        format_args!("takes rect, pill or circle, not `{}`", text.escape_debug()),
                    )
                })?;
            }
            if let Some(radius) = self.workspaces.pill_radius {
                pill.radius = gap(path, "workspaces.pill-radius", Some(radius), 0)?;
                if pill.shape != Shape::Rect {
                    return Err(value(
                        path,
                        "workspaces.pill-radius",
                        format_args!(
                            "rounds a `rect` pill; the `pill` and `circle` shapes are already \
                             as round as they fit (drop it, or set pill-shape = \"rect\")"
                        ),
                    ));
                }
            }
            if let Some(inset) = self.workspaces.pill_inset {
                pill.inset = gap(path, "workspaces.pill-inset", Some(inset), 0)?;
            }
            if let Some(spaces) = self.workspaces.item_gap {
                let max = crate::modules::workspaces::MAX_ITEM_GAP;
                if !(1..=max).contains(&spaces) {
                    return Err(value(
                        path,
                        "workspaces.item-gap",
                        format_args!(
                            "takes a whole number of spaces from 1 to {max}, not {spaces}"
                        ),
                    ));
                }
                modules.workspaces.item_gap = spaces;
            }
            modules.workspaces.active_color = color_opt(
                path,
                "workspaces.active-color",
                self.workspaces.active_color.as_ref(),
            )?;
            modules.workspaces.inactive_color = color_opt(
                path,
                "workspaces.inactive-color",
                self.workspaces.inactive_color.as_ref(),
            )?;
            if let Some(text) = &self.workspaces.display {
                modules.workspaces.display = Display::parse(text).ok_or_else(|| {
                    value(
                        path,
                        "workspaces.display",
                        format_args!("takes numbers or dots, not `{}`", text.escape_debug()),
                    )
                })?;
            }
            if let Some(disc) = self.workspaces.disc {
                modules.workspaces.disc = disc;
            }
            if modules.workspaces.disc {
                if modules.workspaces.pill.shape != Shape::Circle {
                    return Err(value(
                        path,
                        "workspaces.disc",
                        "grows the circle to a disc; it needs pill-shape = \"circle\"",
                    ));
                }
                if modules.workspaces.display != Display::Numbers {
                    return Err(value(
                        path,
                        "workspaces.disc",
                        "grows the circle to a disc; dots have no pill to grow",
                    ));
                }
            }
        }
        #[cfg(feature = "clock")]
        if let Some(icon) = icon::clock(path, &self.clock)? {
            modules.clock.icon = Some(icon);
        }
        #[cfg(feature = "clock")]
        {
            let clock = &self.clock;
            let read = bindings::read(
                crate::modules::clock::ID,
                [
                    clock.on_click.as_ref(),
                    clock.on_right_click.as_ref(),
                    clock.on_middle_click.as_ref(),
                    clock.on_scroll_up.as_ref(),
                    clock.on_scroll_down.as_ref(),
                ],
            )
            .map_err(|(trigger, message)| {
                value(
                    path,
                    CLOCK_KEYS[trigger as usize],
                    format_args!("{message}"),
                )
            })?;
            if !read.is_empty() {
                modules.bindings.push((crate::modules::clock::ID, read));
            }
        }
        #[cfg(feature = "workspaces")]
        {
            let workspaces = &self.workspaces;
            let read = bindings::read(
                crate::modules::workspaces::ID,
                [
                    workspaces.on_click.as_ref(),
                    workspaces.on_right_click.as_ref(),
                    workspaces.on_middle_click.as_ref(),
                    workspaces.on_scroll_up.as_ref(),
                    workspaces.on_scroll_down.as_ref(),
                ],
            )
            .map_err(|(trigger, message)| {
                value(
                    path,
                    WORKSPACES_KEYS[trigger as usize],
                    format_args!("{message}"),
                )
            })?;
            if !read.is_empty() {
                modules
                    .bindings
                    .push((crate::modules::workspaces::ID, read));
            }
        }
        #[cfg(feature = "window-title")]
        {
            use crate::modules::window_title::{DEFAULT_MAX_WIDTH, MAX_MAX_WIDTH};
            let title = &mut modules.window_title;
            if let Some(show) = self.window_title.show_app_id {
                title.show_app_id = show;
            }
            if let Some(width) = self.window_title.max_width {
                if !(1..=MAX_MAX_WIDTH).contains(&width) {
                    return Err(value(
                        path,
                        "window-title.max-width",
                        format_args!(
                            "takes a whole number of logical pixels from 1 to {MAX_MAX_WIDTH}, \
                             not `{width}` (the default is {DEFAULT_MAX_WIDTH})"
                        ),
                    ));
                }
                title.max_width = width;
            }
            if let Some(placeholder) = &self.window_title.placeholder {
                if placeholder.len() > crate::modules::MAX_TEXT {
                    return Err(value(
                        path,
                        "window-title.placeholder",
                        format_args!(
                            "takes at most {} bytes, not {}",
                            crate::modules::MAX_TEXT,
                            placeholder.len()
                        ),
                    ));
                }
                title.placeholder = placeholder.clone();
            }
            if let Some(allow) = self.window_title.allow_close {
                title.allow_close = allow;
            }
            if let Some(icon) =
                icon::window_title(&self.window_title).map_err(|(key, message)| Error::Named {
                    path: path.to_owned(),
                    key,
                    message,
                })?
            {
                title.icon = Some(icon);
            }
            if let Some(show) = self.window_title.show_text {
                title.show_text = show;
            }
            let table = &self.window_title;
            let read = bindings::read(
                crate::modules::window_title::ID,
                [
                    table.on_click.as_ref(),
                    table.on_right_click.as_ref(),
                    table.on_middle_click.as_ref(),
                    table.on_scroll_up.as_ref(),
                    table.on_scroll_down.as_ref(),
                ],
            )
            .map_err(|(trigger, message)| {
                value(
                    path,
                    WINDOW_TITLE_KEYS[trigger as usize],
                    format_args!("{message}"),
                )
            })?;
            // A `close` binding with closing off would be a click that
            // does nothing: refuse it loudly, naming the key.
            if !title.allow_close {
                for trigger in crate::action::Trigger::ALL {
                    let is_close = read.get(trigger).is_some_and(|action| {
                        *action
                            == crate::action::Action::Module(crate::action::ModuleAction {
                                name: std::borrow::Cow::Borrowed("close"),
                                arg: None,
                            })
                    });
                    if is_close {
                        return Err(value(
                            path,
                            WINDOW_TITLE_KEYS[trigger as usize],
                            format_args!(
                                "`close` needs window-title.allow-close = true (closing a window \
                                 by an accidental click loses work)"
                            ),
                        ));
                    }
                }
            }
            if !read.is_empty() {
                modules
                    .bindings
                    .push((crate::modules::window_title::ID, read));
            }
        }
        #[cfg(feature = "volume")]
        {
            apply_volume(
                path,
                "volume",
                crate::modules::volume::ID,
                VOLUME_KEYS,
                &self.volume,
                &mut modules.volume,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "microphone")]
        {
            apply_volume(
                path,
                "microphone",
                crate::modules::microphone::ID,
                MICROPHONE_KEYS,
                &self.microphone,
                &mut modules.microphone,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "network")]
        {
            apply_network(
                path,
                &self.network,
                &mut modules.network,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "brightness")]
        {
            apply_brightness(
                path,
                &self.brightness,
                &mut modules.brightness,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "battery")]
        {
            apply_battery(
                path,
                &self.battery,
                &mut modules.battery,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "tray")]
        {
            apply_tray(path, &self.tray, &mut modules.bindings)?;
        }
        #[cfg(feature = "media")]
        {
            apply_media(path, &self.media, &mut modules.media, &mut modules.bindings)?;
        }
        #[cfg(feature = "bluetooth")]
        {
            apply_bluetooth(
                path,
                &self.bluetooth,
                &mut modules.bluetooth,
                &mut modules.bindings,
            )?;
        }
        #[cfg(feature = "power")]
        {
            apply_power(path, &self.power, &mut modules.power, &mut modules.bindings)?;
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
            fallback_fonts,
            font_size,
            radius,
            opacity,
            #[cfg(feature = "popup")]
            tooltip_delay,
            modules,
            outputs,
        })
    }

    /// The module lists: giving any of the three sets the whole layout, as
    /// the flags do (a part not given is empty).
    fn layout(
        &self,
        path: &Path,
        gaps: Gaps,
        defaults: &Layout,
        known: &[&'static str],
    ) -> Result<Layout, Error> {
        let Gaps {
            padding,
            spacing,
            separator,
            margins,
        } = gaps;
        let layout = if self.left.is_none() && self.center.is_none() && self.right.is_none() {
            Layout {
                padding,
                spacing,
                separator,
                margins,
                ..defaults.clone()
            }
        } else {
            Layout {
                left: ids(path, Section::Left, &self.left, known)?,
                center: ids(path, Section::Center, &self.center, known)?,
                right: ids(path, Section::Right, &self.right, known)?,
                padding,
                spacing,
                separator,
                margins,
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
        let color = |key: &'static str, text: Option<&String>, fallback: Color| {
            color_opt(path, key, text).map(|color| color.unwrap_or(fallback))
        };
        // Unset, the tint follows a custom accent: it was the accent
        // before it had a token of its own.
        let accent = color("colors.accent", colors.accent.as_ref(), defaults.accent)?;
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
            accent,
            hover: color_opt(path, "colors.hover", colors.hover.as_ref())?.unwrap_or(accent),
            dim: color("colors.dim", colors.dim.as_ref(), defaults.dim)?,
            urgent: color("colors.urgent", colors.urgent.as_ref(), defaults.urgent)?,
        })
    }
}

/// The dotted keys of the built-in modules' interaction keys, by trigger.
#[cfg(feature = "clock")]
const CLOCK_KEYS: [&str; 5] = [
    "clock.on-click",
    "clock.on-right-click",
    "clock.on-middle-click",
    "clock.on-scroll-up",
    "clock.on-scroll-down",
];
#[cfg(feature = "workspaces")]
const WORKSPACES_KEYS: [&str; 5] = [
    "workspaces.on-click",
    "workspaces.on-right-click",
    "workspaces.on-middle-click",
    "workspaces.on-scroll-up",
    "workspaces.on-scroll-down",
];
#[cfg(feature = "window-title")]
const WINDOW_TITLE_KEYS: [&str; 5] = [
    "window-title.on-click",
    "window-title.on-right-click",
    "window-title.on-middle-click",
    "window-title.on-scroll-up",
    "window-title.on-scroll-down",
];
#[cfg(feature = "volume")]
const VOLUME_KEYS: [&str; 5] = [
    "volume.on-click",
    "volume.on-right-click",
    "volume.on-middle-click",
    "volume.on-scroll-up",
    "volume.on-scroll-down",
];
#[cfg(feature = "microphone")]
const MICROPHONE_KEYS: [&str; 5] = [
    "microphone.on-click",
    "microphone.on-right-click",
    "microphone.on-middle-click",
    "microphone.on-scroll-up",
    "microphone.on-scroll-down",
];
#[cfg(feature = "battery")]
const BATTERY_KEYS: [&str; 5] = [
    "battery.on-click",
    "battery.on-right-click",
    "battery.on-middle-click",
    "battery.on-scroll-up",
    "battery.on-scroll-down",
];

/// The `[battery]` table: the thresholds, which batteries feed the level,
/// the low hook and the interaction keys, into the module's settings.
#[cfg(feature = "battery")]
fn apply_battery(
    path: &Path,
    table: &BatteryFile,
    settings: &mut crate::modules::battery::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    use crate::modules::battery::{
        Batteries, DEFAULT_URGENT_BELOW, DEFAULT_WARN_BELOW, MAX_THRESHOLD,
    };
    let named = |key: &str, message: String| Error::Named {
        path: path.to_owned(),
        key: format!("battery.{key}"),
        message,
    };
    if let Some(below) = table.warn_below {
        if below > MAX_THRESHOLD {
            return Err(named(
                "warn-below",
                format!(
                    "takes whole percent from 0 to {MAX_THRESHOLD}, not `{below}` \
                     (the default is {DEFAULT_WARN_BELOW})"
                ),
            ));
        }
        settings.warn_below = below;
    }
    if let Some(below) = table.urgent_below {
        if below > MAX_THRESHOLD {
            return Err(named(
                "urgent-below",
                format!(
                    "takes whole percent from 0 to {MAX_THRESHOLD}, not `{below}` \
                     (the default is {DEFAULT_URGENT_BELOW})"
                ),
            ));
        }
        settings.urgent_below = below;
    }
    // Urgent is checked first, so a warn band below it would never show:
    // refuse the inversion loudly rather than silently drop a class.
    if settings.warn_below < settings.urgent_below {
        return Err(named(
            "warn-below",
            format!(
                "takes at least urgent-below ({}), not `{}` (below it warn is unreachable)",
                settings.urgent_below, settings.warn_below
            ),
        ));
    }
    if let Some(text) = &table.batteries {
        settings.batteries = Batteries::parse(text).ok_or_else(|| {
            named(
                "batteries",
                format!("takes combine or first, not `{}`", text.escape_debug()),
            )
        })?;
    }
    if let Some(given) = &table.on_low {
        let toml::Value::Table(hook) = given else {
            return Err(named(
                "on-low",
                "takes `{ exec = [\"command\", \"arg\"] }`, run once per downward crossing \
                 of battery.urgent-below"
                    .to_owned(),
            ));
        };
        let mut entries = hook.iter();
        let (Some((kind, inner)), None) = (entries.next(), entries.next()) else {
            return Err(named(
                "on-low",
                "takes `{ exec = [\"command\", \"arg\"] }`, with one key".to_owned(),
            ));
        };
        if kind != "exec" {
            return Err(named(
                "on-low",
                format!(
                    "takes `{{ exec = [...] }}`, not `{{{}}}`",
                    kind.escape_debug()
                ),
            ));
        }
        let argv = bindings::exec(inner).map_err(|message| named("on-low", message))?;
        settings.on_low = Some(argv);
    }
    let icons = icon::battery(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    settings.icon_charging = icons.charging;
    settings.icon_full = icons.full;
    if let Some(show) = table.show_text {
        settings.show_text = show;
    }
    let read = bindings::read(
        crate::modules::battery::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| Error::Named {
        path: path.to_owned(),
        key: BATTERY_KEYS[trigger as usize].to_owned(),
        message,
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::battery::ID, read));
    }
    Ok(())
}

/// The `[volume]` (or `[microphone]`) table: `step`, `max-volume`, the
/// icon keys and the interaction keys, into the module's settings.
/// `section` is the table's name, for the errors.
#[cfg(any(feature = "volume", feature = "microphone"))]
fn apply_volume(
    path: &Path,
    section: &'static str,
    id: &'static str,
    keys: [&str; 5],
    table: &VolumeFile,
    settings: &mut crate::modules::volume::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    use crate::modules::volume::{
        DEFAULT_MAX_VOLUME, DEFAULT_STEP, MAX_MAX_VOLUME, MAX_STEP, MIN_MAX_VOLUME,
    };
    let named = |key: &str, message: String| Error::Named {
        path: path.to_owned(),
        key: format!("{section}.{key}"),
        message,
    };
    if let Some(step) = table.step {
        if !(1..=MAX_STEP).contains(&step) {
            return Err(named(
                "step",
                format!(
                    "takes a whole number of percent points from 1 to {MAX_STEP}, not `{step}` \
                     (the default is {DEFAULT_STEP})"
                ),
            ));
        }
        settings.step = step;
    }
    if let Some(max) = table.max_volume {
        if !(MIN_MAX_VOLUME..=MAX_MAX_VOLUME).contains(&max) {
            return Err(named(
                "max-volume",
                format!(
                    "takes whole percent from {MIN_MAX_VOLUME} (full scale) to {MAX_MAX_VOLUME} \
                     (over-amplification), not `{max}` (the default is {DEFAULT_MAX_VOLUME})"
                ),
            ));
        }
        settings.max_volume = max;
    }
    if let Some(icon) = icon::volume(section, table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })? {
        settings.icon = Some(icon);
    }
    let read = bindings::read(
        id,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| Error::Named {
        path: path.to_owned(),
        key: keys[trigger as usize].to_owned(),
        message,
    })?;
    if !read.is_empty() {
        bindings.push((id, read));
    }
    Ok(())
}
#[cfg(feature = "network")]
const NETWORK_KEYS: [&str; 5] = [
    "network.on-click",
    "network.on-right-click",
    "network.on-middle-click",
    "network.on-scroll-up",
    "network.on-scroll-down",
];

/// The `[network]` table: `interface`, `show-ssid`, `menu-command`,
/// `connect-command`, the icon keys and `show-text`, and the interaction
/// keys, into the module's settings.
#[cfg(feature = "network")]
fn apply_network(
    path: &Path,
    table: &NetworkFile,
    settings: &mut crate::modules::network::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    if let Some(interface) = table.interface.as_deref() {
        // `IFNAMSIZ` is 16 with the NUL: longer never matches, and empty
        // never selects.
        if interface.is_empty() || interface.len() > 15 {
            return Err(value(
                path,
                "network.interface",
                format_args!("takes an interface name of 1 to 15 bytes, not `{interface}`"),
            ));
        }
        settings.interface = Some(interface.to_owned());
    }
    if let Some(show) = table.show_ssid {
        settings.show_ssid = show;
    }
    if let Some(command) = table.menu_command.as_deref() {
        if command.iter().any(String::is_empty) {
            return Err(value(
                path,
                "network.menu-command",
                format_args!("takes no empty argument, not `{command:?}`"),
            ));
        }
        settings.menu_command = command.to_owned();
    }
    if let Some(command) = table.connect_command.as_deref() {
        if command.iter().any(String::is_empty) {
            return Err(value(
                path,
                "network.connect-command",
                format_args!("takes no empty argument, not `{command:?}`"),
            ));
        }
        settings.connect_command = command.to_owned();
    }
    let icons = icon::network(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    settings.icon_ethernet = icons.ethernet;
    settings.icon_wifi = icons.wifi;
    settings.icon_vpn = icons.vpn;
    settings.icon_offline = icons.offline;
    if let Some(show) = table.show_text {
        settings.show_text = show;
    }
    let read = bindings::read(
        crate::modules::network::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(
            path,
            NETWORK_KEYS[trigger as usize],
            format_args!("{message}"),
        )
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::network::ID, read));
    }
    Ok(())
}

#[cfg(feature = "brightness")]
const BRIGHTNESS_KEYS: [&str; 5] = [
    "brightness.on-click",
    "brightness.on-right-click",
    "brightness.on-middle-click",
    "brightness.on-scroll-up",
    "brightness.on-scroll-down",
];

/// The `[brightness]` table: `device`, `step` and the interaction keys,
/// into the module's settings.
#[cfg(feature = "brightness")]
fn apply_brightness(
    path: &Path,
    table: &BrightnessFile,
    settings: &mut crate::modules::brightness::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    use crate::modules::brightness::{DEFAULT_STEP, MAX_STEP};
    if let Some(device) = table.device.as_deref() {
        // Empty never selects; a name that could escape the class
        // directory never reaches it (the module skips it), so refuse it
        // here, where the key is named.
        if device.is_empty() || device.contains('/') || device.contains('\0') {
            return Err(value(
                path,
                "brightness.device",
                format_args!("takes a backlight name, not `{device}`"),
            ));
        }
        settings.device = Some(device.to_owned());
    }
    if let Some(step) = table.step {
        if !(1..=MAX_STEP).contains(&step) {
            return Err(value(
                path,
                "brightness.step",
                format_args!(
                    "takes a whole number of percent points from 1 to {MAX_STEP}, not `{step}` \
                     (the default is {DEFAULT_STEP})"
                ),
            ));
        }
        settings.step = step;
    }
    let icons = icon::brightness(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    if let Some(show) = table.show_text {
        settings.show_text = show;
    }
    let read = bindings::read(
        crate::modules::brightness::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(
            path,
            BRIGHTNESS_KEYS[trigger as usize],
            format_args!("{message}"),
        )
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::brightness::ID, read));
    }
    Ok(())
}

/// The dotted interaction keys of `[tray]`, in [`Trigger`] order: what a
/// bad binding names.
#[cfg(feature = "tray")]
const TRAY_KEYS: [&str; 5] = [
    "tray.on-click",
    "tray.on-right-click",
    "tray.on-middle-click",
    "tray.on-scroll-up",
    "tray.on-scroll-down",
];

/// The `[tray]` table: the interaction keys into the module's bindings.
/// The module holds no options of its own.
#[cfg(feature = "tray")]
fn apply_tray(
    path: &Path,
    table: &TrayFile,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    let read = bindings::read(
        crate::modules::tray::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(path, TRAY_KEYS[trigger as usize], format_args!("{message}"))
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::tray::ID, read));
    }
    Ok(())
}

/// The dotted interaction keys of `[media]`, in [`Trigger`] order: what a
/// bad binding names.
#[cfg(feature = "media")]
const MEDIA_KEYS: [&str; 5] = [
    "media.on-click",
    "media.on-right-click",
    "media.on-middle-click",
    "media.on-scroll-up",
    "media.on-scroll-down",
];

/// The `[media]` table: `player`, `max-width` and the interaction keys,
/// into the module's settings and bindings.
#[cfg(feature = "media")]
fn apply_media(
    path: &Path,
    table: &MediaFile,
    settings: &mut crate::modules::media::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    use crate::modules::media::{DEFAULT_MAX_WIDTH, MAX_MAX_WIDTH};
    if let Some(player) = table.player.as_deref() {
        // The short name of a bus name's element: what a player is called
        // on the bus is letters, digits, `_` and `-`, with dots between
        // elements (`vlc`, `org.example.player`); anything else never
        // matches one.
        let name_chars = player
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
        if player.is_empty() || player.len() > 200 || !name_chars {
            return Err(value(
                path,
                "media.player",
                format_args!(
                    "takes a player's name as on the bus (`spotify` for \
                     org.mpris.MediaPlayer2.spotify), letters, digits, `_`, `-` and `.`, \
                     not `{player}`"
                ),
            ));
        }
        settings.player = Some(player.to_owned());
    }
    if let Some(width) = table.max_width {
        if !(1..=MAX_MAX_WIDTH).contains(&width) {
            return Err(value(
                path,
                "media.max-width",
                format_args!(
                    "takes a whole number of logical pixels from 1 to {MAX_MAX_WIDTH}, \
                     not `{width}` (the default is {DEFAULT_MAX_WIDTH})"
                ),
            ));
        }
        settings.max_width = width;
    }
    let icons = icon::media(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    settings.icon_playing = icons.playing;
    settings.icon_paused = icons.paused;
    if let Some(show) = table.show_text {
        settings.show_text = show;
    }
    let read = bindings::read(
        crate::modules::media::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(
            path,
            MEDIA_KEYS[trigger as usize],
            format_args!("{message}"),
        )
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::media::ID, read));
    }
    Ok(())
}

#[cfg(feature = "bluetooth")]
const BLUETOOTH_KEYS: [&str; 5] = [
    "bluetooth.on-click",
    "bluetooth.on-right-click",
    "bluetooth.on-middle-click",
    "bluetooth.on-scroll-up",
    "bluetooth.on-scroll-down",
];

/// The `[bluetooth]` table: `menu-command` and the interaction keys,
/// into the module's settings.
#[cfg(feature = "bluetooth")]
fn apply_bluetooth(
    path: &Path,
    table: &BluetoothFile,
    settings: &mut crate::modules::bluetooth::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    if let Some(command) = table.menu_command.as_deref() {
        if command.iter().any(String::is_empty) {
            return Err(value(
                path,
                "bluetooth.menu-command",
                format_args!("takes no empty argument, not `{command:?}`"),
            ));
        }
        settings.menu_command = command.to_owned();
    }
    let icons = icon::bluetooth(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    settings.icon_off = icons.off;
    settings.icon_on = icons.on;
    settings.icon_connected = icons.connected;
    if let Some(show) = table.show_text {
        settings.show_text = show;
    }
    let read = bindings::read(
        crate::modules::bluetooth::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(
            path,
            BLUETOOTH_KEYS[trigger as usize],
            format_args!("{message}"),
        )
    })?;
    if !read.is_empty() {
        bindings.push((crate::modules::bluetooth::ID, read));
    }
    Ok(())
}

/// The bluetooth module's options. Without the `bluetooth` feature there
/// are none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct BluetoothFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "bluetooth")]
    margin: Option<u32>,
    /// The picker: spawned with the device list on stdin when the
    /// module's `menu` action runs.
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "menu-command")]
    menu_command: Option<Vec<String>>,
    /// One glyph, drawn before the text in every state (instead of the
    /// per-state ones below).
    #[cfg(feature = "bluetooth")]
    icon: Option<String>,
    /// SVG path data, drawn before the text in every state (instead of
    /// `icon`).
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn before the text in every state (instead of
    /// `icon`). Only in a build with the `icon-image` feature: without it
    /// the key is unknown.
    #[cfg(all(feature = "bluetooth", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "bluetooth", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// One glyph each, drawn before the text in that state (instead of
    /// `icon`): every adapter off, one powered with nothing connected,
    /// and a device connected. A state with none shows the static icon.
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "icon-off")]
    icon_off: Option<String>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "icon-on")]
    icon_on: Option<String>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "icon-connected")]
    icon_connected: Option<String>,
    /// Whether the text is drawn beside the icon (default true): false
    /// draws only the icon, with the text moved into the tooltip.
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "bluetooth")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

#[cfg(feature = "power")]
const POWER_KEYS: [&str; 5] = [
    "power.on-click",
    "power.on-right-click",
    "power.on-middle-click",
    "power.on-scroll-up",
    "power.on-scroll-down",
];

/// The `[power]` table: the icon keys, `rows`, the five `*-command`
/// overrides and the interaction keys, into the module's settings.
#[cfg(feature = "power")]
fn apply_power(
    path: &Path,
    table: &PowerFile,
    settings: &mut crate::modules::power::Settings,
    bindings: &mut Vec<(&'static str, crate::action::Bindings)>,
) -> Result<(), Error> {
    if let Some(rows) = table.rows.as_deref() {
        let mut hidden = [true; 5];
        for name in rows {
            let Some(at) = crate::modules::power::row_index(name) else {
                return Err(value(
                    path,
                    "power.rows",
                    format_args!(
                        "takes the rows to show (`{}`), not `{}`",
                        crate::modules::power::ROW_NAMES.join("`, `"),
                        name.escape_debug(),
                    ),
                ));
            };
            hidden[at] = false;
        }
        settings.hidden = hidden;
    }
    let commands = [
        ("power.lock-command", &table.lock_command),
        ("power.logout-command", &table.logout_command),
        ("power.suspend-command", &table.suspend_command),
        ("power.reboot-command", &table.reboot_command),
        ("power.poweroff-command", &table.poweroff_command),
    ];
    for (at, &(key, given)) in commands.iter().enumerate() {
        if let Some(command) = given {
            // An argv list run directly, never through a shell: the same
            // bounds the interaction keys' `{ exec = [...] }` holds (at
            // most 32 arguments of 4096 bytes, no NUL), so a staged
            // `Action::Exec` always holds its contract.
            let argv = bindings::exec(command)
                .map_err(|message| value(path, key, format_args!("{message}")))?;
            settings.commands[at] = argv;
        }
    }
    let icons = icon::power(table).map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })?;
    settings.icon = icons.icon;
    settings.row_icons = [
        icons.lock,
        icons.logout,
        icons.suspend,
        icons.reboot,
        icons.poweroff,
    ];
    let read = bindings::read(
        crate::modules::power::ID,
        [
            table.on_click.as_ref(),
            table.on_right_click.as_ref(),
            table.on_middle_click.as_ref(),
            table.on_scroll_up.as_ref(),
            table.on_scroll_down.as_ref(),
        ],
    )
    .map_err(|(trigger, message)| {
        value(
            path,
            POWER_KEYS[trigger as usize],
            format_args!("{message}"),
        )
    })?;
    // A click opens the menu only when bound (`on-click = "popup"`, as
    // the volume slider's): like every other module, no keys means no
    // bindings entry at all.
    if !read.is_empty() {
        bindings.push((crate::modules::power::ID, read));
    }
    Ok(())
}

/// The power module's options. Without the `power` feature there are
/// none, and any key is a loud error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct PowerFile {
    /// Extra logical pixels on each side of the module.
    #[cfg(feature = "power")]
    margin: Option<u32>,
    /// One glyph, drawn with no text (instead of the per-row ones below).
    #[cfg(feature = "power")]
    icon: Option<String>,
    /// SVG path data, drawn with no text (instead of `icon`).
    #[cfg(feature = "power")]
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    /// The path's viewbox, `min-x min-y width height`; 24 by 24 if absent.
    #[cfg(feature = "power")]
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// A PNG file, drawn with no text (instead of `icon`). Only in a
    /// build with the `icon-image` feature: without it the key is taken
    /// (its value ignored), so that the refusal can say what is missing.
    #[cfg(all(feature = "power", feature = "icon-image"))]
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    /// Without the feature the key is still taken (its value ignored), so
    /// that the refusal can say what is missing, not "unknown field".
    #[cfg(all(feature = "power", not(feature = "icon-image")))]
    #[serde(rename = "icon-image")]
    icon_image: Option<serde::de::IgnoredAny>,
    /// One glyph each, drawn before its row's label (instead of nothing):
    /// the lock, log-out, suspend, reboot and shut-down rows. A row with
    /// none shows its label alone.
    #[cfg(feature = "power")]
    #[serde(rename = "icon-lock")]
    icon_lock: Option<String>,
    #[cfg(feature = "power")]
    #[serde(rename = "icon-logout")]
    icon_logout: Option<String>,
    #[cfg(feature = "power")]
    #[serde(rename = "icon-suspend")]
    icon_suspend: Option<String>,
    #[cfg(feature = "power")]
    #[serde(rename = "icon-reboot")]
    icon_reboot: Option<String>,
    #[cfg(feature = "power")]
    #[serde(rename = "icon-poweroff")]
    icon_poweroff: Option<String>,
    /// The rows to show: a subset of `lock`, `logout`, `suspend`,
    /// `reboot` and `poweroff` (all of them, when absent).
    #[cfg(feature = "power")]
    rows: Option<Vec<String>>,
    /// Per-row command overrides: argv lists run directly, never through
    /// a shell, instead of the row's own path. `lock-command` unset hides
    /// the lock row (there is no default locker).
    #[cfg(feature = "power")]
    #[serde(rename = "lock-command")]
    lock_command: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "logout-command")]
    logout_command: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "suspend-command")]
    suspend_command: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "reboot-command")]
    reboot_command: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "poweroff-command")]
    poweroff_command: Option<toml::Value>,
    /// The interaction keys (`bindings`): a module action, `{ exec = [...] }`
    /// or `{ scoot = "..." }`.
    #[cfg(feature = "power")]
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[cfg(feature = "power")]
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// The layout's spacings, validated: what [`File::layout`] adds to the
/// module lists.
struct Gaps {
    padding: u32,
    spacing: u32,
    separator: u32,
    margins: Vec<(&'static str, u32)>,
}

/// A value error at `key`: what it takes, in `message`.
fn value(path: &Path, key: &'static str, message: impl fmt::Display) -> Error {
    Error::Value {
        path: path.to_owned(),
        key,
        message: message.to_string(),
    }
}

/// A color the file names, or `None` when it names none: a bad one is a
/// loud error naming its key.
fn color_opt(
    path: &Path,
    key: &'static str,
    text: Option<&String>,
) -> Result<Option<Color>, Error> {
    text.map(|text| {
        Color::parse(text).map_err(|error| Error::Value {
            path: path.to_owned(),
            key,
            message: format!("{text:?}: {error}"),
        })
    })
    .transpose()
}

/// A `left`/`center`/`right` list: every id must name a module of this
/// build.
fn ids(
    path: &Path,
    section: Section,
    ids: &Option<Vec<String>>,
    known: &[&'static str],
) -> Result<Vec<&'static str>, Error> {
    let mut placed = Vec::new();
    for id in ids.as_ref().map(Vec::as_slice).unwrap_or_default() {
        // A built-in module, or one the file defines (`[button.NAME]`, ...).
        let found = crate::modules::find(id)
            .map(|spec| spec.id)
            .or_else(|| known.iter().copied().find(|name| *name == id));
        let Some(found) = found else {
            let mut has = String::new();
            if crate::modules::REGISTRY.is_empty() && known.is_empty() {
                has.push_str(" none");
            }
            for spec in crate::modules::REGISTRY {
                has.push(' ');
                has.push_str(spec.id);
            }
            for name in known {
                has.push(' ');
                has.push_str(name);
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
        placed.push(found);
    }
    Ok(placed)
}

/// At most [`crate::modules::custom::MAX_EXEC`] `exec` modules are placed
/// (on any output): each holds a child, a `timerfd` and up to two polled
/// fds.
fn check_exec_placed(path: &Path, placed: &Layout, defined: &custom::Defined) -> Result<(), Error> {
    let execs = placed
        .placed()
        .filter(|(_, id)| {
            defined
                .modules
                .iter()
                .any(|custom| custom.id == *id && custom.kind.name() == "exec")
        })
        .count();
    if execs > crate::modules::custom::MAX_EXEC {
        return Err(value(
            path,
            "exec",
            format_args!(
                "at most {} exec modules may be placed, not {execs}",
                crate::modules::custom::MAX_EXEC
            ),
        ));
    }
    Ok(())
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
