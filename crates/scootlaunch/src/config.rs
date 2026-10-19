//! `launch.toml`: the launcher's own file, beside the bar's.
//!
//! Every key is optional; a missing file is the built-in defaults. The
//! panel refuses a bad file loudly (unlike the notification daemon, there
//! is no running instance a bad file could restart-loop: refusing is
//! safe); `--check` validates without opening. Unknown keys are refused:
//! a typo that silently did nothing would be worse than an error. The
//! file is capped at 64 KiB (the bar's config discipline).
//!
//! The look layer (`<state>/look/launch.toml`) carries colors, the font
//! family, the size, the opacity and the radius; this file carries what
//! the look must never decide: the font *file*, the panel geometry, and
//! (a later ticket) the terminal `Terminal=true` entries run through.
//! Merge order at every open: flags, then this file, then the look, then
//! the built-ins (`dev/design/scootlaunch.md`, Stylix and look
//! integration).

use std::path::PathBuf;

use serde::Deserialize;

/// A config file past this is refused, not read: it is hand-edited text,
/// and anything larger is a mistake, not a config.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// The panel's default width, in ems of the look's font size: wide enough
/// for `Name — GenericName` rows without wrapping on a 1600-pixel output
/// at the default 13 px size.
pub const DEFAULT_WIDTH_EM: u32 = 40;
/// The list's default visible rows under the entry line.
pub const DEFAULT_ROWS: u32 = 10;

/// The launcher's own file: `$XDG_CONFIG_HOME/scoot/launch.toml` beside
/// the bar's, or `~/.config/scoot/launch.toml` without `XDG_CONFIG_HOME`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("scoot").join("launch.toml")
}

/// The built-in defaults every row below documents: a 40-em panel with 10
/// list rows, the first usable well-known font file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// A `.ttf` or `.otf` file. Without it, the first of the well-known
    /// font files is used, as the bar's `--font` does.
    pub font: Option<String>,
    /// The panel width in ems of the look's font size (10 to 200).
    pub width_em: u32,
    /// The list's visible rows under the entry line (1 to 64).
    pub rows: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: None,
            width_em: DEFAULT_WIDTH_EM,
            rows: DEFAULT_ROWS,
        }
    }
}

/// Loads `path`: `Ok` the file's values over the defaults, `Err` why it
/// was refused. A missing file is not refused — it is the defaults, said
/// so by the caller. Anything past [`MAX_CONFIG_BYTES`] is refused before
/// parsing.
pub fn load(path: &std::path::Path) -> Result<Config, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse(&bytes)
}

/// Parses `bytes` as a config file: pure over its input, so tests never
/// touch the filesystem.
pub fn parse(bytes: &[u8]) -> Result<Config, String> {
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(format!(
            "config larger than {} KiB is refused",
            MAX_CONFIG_BYTES / 1024
        ));
    }
    let text = core::str::from_utf8(bytes).map_err(|_| String::from("config is not UTF-8"))?;
    let config: Config =
        toml::from_str(text).map_err(|error| format!("config refused: {error}"))?;
    config.check()
}

impl Config {
    /// Range-checks the parsed values: out-of-range values are refused
    /// with the key named, never clamped silently.
    fn check(self) -> Result<Self, String> {
        if self.width_em < 10 || self.width_em > 200 {
            return Err(format!(
                "config refused: width_em is {} (10 to 200)",
                self.width_em
            ));
        }
        if self.rows == 0 || self.rows > 64 {
            return Err(format!("config refused: rows is {} (1 to 64)", self.rows));
        }
        if let Some(font) = &self.font {
            if font.is_empty() || font.len() > 4096 {
                return Err(String::from(
                    "config refused: font is a path of 1 to 4096 bytes",
                ));
            }
            if font.contains('\0') || font.chars().any(|char| char.is_control()) {
                return Err(String::from(
                    "config refused: font must not contain control characters",
                ));
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests;
