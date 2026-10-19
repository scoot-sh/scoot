//! `notify.toml`: the daemon's own file, beside the bar's.
//!
//! Every key is optional; a missing file is the built-in defaults. A bad
//! file at daemon start runs on those defaults and reports the error (an
//! exit would restart-loop with no notification daemon at all); `--check`
//! and a later `msg reload` refuse loudly instead. Unknown keys are
//! refused: a typo that silently did nothing would be worse than an
//! error. The file is capped at 64 KiB (the bar's config discipline).

use std::path::PathBuf;

use serde::Deserialize;

/// A config file past this is refused, not read: it is hand-edited text,
/// and anything larger is a mistake, not a config.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// The daemon's own file: `$XDG_CONFIG_HOME/scoot/notify.toml` beside the
/// bar's, or `~/.config/scoot/notify.toml` without `XDG_CONFIG_HOME`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("scoot").join("notify.toml")
}

/// The built-in defaults every row below documents: low 5 s, normal 10 s,
/// critical never (the spec's urgency rule), at most 128 live, a
/// per-sender bucket of 20 per second bursting to 40; a 360 px card
/// stack at the top right with a 12 px margin, at most 5 cards, bodies
/// wrapped to 5 lines, and expiry paused after 60 s without input.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Low-urgency expiry for `expire_timeout = -1`, in milliseconds.
    pub timeout_low_ms: u32,
    /// Normal-urgency expiry for `expire_timeout = -1`, in milliseconds.
    pub timeout_normal_ms: u32,
    /// A positive `expire_timeout` is honored up to this, in
    /// milliseconds (1 h: the design's clamp).
    pub timeout_max_ms: u32,
    /// At most this many live notifications; past it the oldest
    /// non-critical one closes with reason 4.
    pub max_live: u32,
    /// Per-sender token-bucket refill, notifications per second.
    pub rate_per_sec: u32,
    /// Per-sender burst: past it a `Notify` still gets its id, but the
    /// card goes to history unshown.
    pub rate_burst: u32,
    /// The card width, in logical pixels.
    pub width_px: u32,
    /// At most this many cards on screen; past it a "+N more" pill
    /// covers the rest.
    pub max_visible: u32,
    /// The stack's distance from the screen edges, in logical pixels.
    pub margin_px: u32,
    /// The screen corner the stack anchors to.
    pub anchor: Anchor,
    /// The output the stack shows on: `None` is a null output, the
    /// compositor's choice (today its primary; the
    /// `layer-shell-default-output` ticket moves that to the output the
    /// user is on). A name pins that output; when it is gone the stack
    /// falls back to the compositor's choice.
    pub output: Option<String>,
    /// Body lines per card before the ellipsis.
    pub body_lines: u32,
    /// The urgency border width, in logical pixels (0 draws none).
    pub border_px: u32,
    /// The card corner radius, in logical pixels.
    pub radius: u32,
    /// Seconds without input after which every expiry clock pauses (0
    /// disables): `ext-idle-notify` v2 counts only real input, so a
    /// playing video does not count as presence. Someone who walked
    /// away comes back to their notifications.
    pub pause_when_idle_secs: u32,
}

/// The screen corner the stack anchors to: the layer-shell anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    /// Top left.
    TopLeft,
    /// Top right (the default).
    #[default]
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    BottomRight,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout_low_ms: 5_000,
            timeout_normal_ms: 10_000,
            timeout_max_ms: 3_600_000,
            max_live: 128,
            rate_per_sec: 20,
            rate_burst: 40,
            width_px: 360,
            max_visible: 5,
            margin_px: 12,
            anchor: Anchor::TopRight,
            output: None,
            body_lines: 5,
            border_px: 2,
            radius: 8,
            pause_when_idle_secs: 60,
        }
    }
}

/// Loads `path`: `Ok` the file's values over the defaults, `Err` why it
/// was refused (missing is not refused — it is the defaults, said so by
/// the caller deciding nothing was read). Anything past
/// [`MAX_CONFIG_BYTES`] is refused before parsing.
pub fn load(path: &std::path::Path) -> Result<Config, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse(&bytes)
}

/// Parses `bytes` as a config file: the fuzz target's check and the
/// corpus replay call this, so no I/O hides behind it.
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
    /// Range-checks the parsed values: out-of-range numbers are refused
    /// with the key named, never clamped silently.
    fn check(self) -> Result<Self, String> {
        if self.max_live == 0 || self.max_live > 1024 {
            return Err(format!(
                "config refused: max_live is {} (1 to 1024)",
                self.max_live
            ));
        }
        if self.rate_per_sec == 0 || self.rate_per_sec > 1000 {
            return Err(format!(
                "config refused: rate_per_sec is {} (1 to 1000)",
                self.rate_per_sec
            ));
        }
        if self.rate_burst == 0 || self.rate_burst > 1000 {
            return Err(format!(
                "config refused: rate_burst is {} (1 to 1000)",
                self.rate_burst
            ));
        }
        if self.timeout_max_ms == 0 || self.timeout_max_ms > 86_400_000 {
            return Err(format!(
                "config refused: timeout_max_ms is {} (1 to 86400000)",
                self.timeout_max_ms
            ));
        }
        if self.timeout_low_ms > self.timeout_max_ms {
            return Err(format!(
                "config refused: timeout_low_ms is {} (past timeout_max_ms {})",
                self.timeout_low_ms, self.timeout_max_ms
            ));
        }
        if self.timeout_normal_ms > self.timeout_max_ms {
            return Err(format!(
                "config refused: timeout_normal_ms is {} (past timeout_max_ms {})",
                self.timeout_normal_ms, self.timeout_max_ms
            ));
        }
        if self.width_px < 10 || self.width_px > 1024 {
            return Err(format!(
                "config refused: width_px is {} (10 to 1024)",
                self.width_px
            ));
        }
        if self.max_visible == 0 || self.max_visible > 16 {
            return Err(format!(
                "config refused: max_visible is {} (1 to 16)",
                self.max_visible
            ));
        }
        if self.margin_px > 128 {
            return Err(format!(
                "config refused: margin_px is {} (0 to 128)",
                self.margin_px
            ));
        }
        if let Some(output) = &self.output
            && (output.is_empty()
                || output.len() > 256
                || output.bytes().any(|byte| byte < 0x20 || byte == 0x7f))
        {
            return Err(format!(
                "config refused: output is not a usable output name (1 to 256 printable bytes)"
            ));
        }
        if self.body_lines == 0 || self.body_lines > 16 {
            return Err(format!(
                "config refused: body_lines is {} (1 to 16)",
                self.body_lines
            ));
        }
        if self.border_px > 8 {
            return Err(format!(
                "config refused: border_px is {} (0 to 8)",
                self.border_px
            ));
        }
        if self.radius > 64 {
            return Err(format!(
                "config refused: radius is {} (0 to 64)",
                self.radius
            ));
        }
        if self.pause_when_idle_secs > 3600 {
            return Err(format!(
                "config refused: pause_when_idle_secs is {} (0 to 3600)",
                self.pause_when_idle_secs
            ));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests;
