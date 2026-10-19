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
/// wrapped to 5 lines, expiry paused after 60 s without input; a history
/// ring of 64, critical cards showing through DND, and no rules.
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
    /// card is counted unshown, never stored.
    pub rate_burst: u32,
    /// At most this many closed or held cards kept in the in-memory
    /// history ring (0 keeps none). The ring survives a reload, never a
    /// restart.
    pub history_max: u32,
    /// Whether critical cards show while DND holds everything else.
    pub dnd_allows_critical: bool,
    /// The `[[rule]]` tables, in file order: every matching rule
    /// applies, and later keys win (the design's rule grammar).
    pub rule: Vec<Rule>,
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

/// At most this many `[[rule]]` tables: past it the file is refused,
/// not silently truncated.
pub const MAX_RULES: usize = 32;

/// The most argv words one rule's `exec` may carry: an argv, never a
/// shell line, so a bound on words bounds the spawn, not prose.
pub const MAX_EXEC_ARGV: usize = 16;

/// One argv word past this is refused: commands are short paths and
/// flags, not documents.
pub const MAX_EXEC_WORD: usize = 4096;

/// A rule's urgency, matching or setting: the spec's §7 three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleUrgency {
    Low,
    Normal,
    Critical,
}

/// One `[[rule]]` table: the match keys narrow, the action keys steer.
/// Every key is optional; a rule with no match key matches everything
/// (a catch-all, usually last). Every matching rule applies, and for the
/// scalar actions the later rule wins; every matching `exec` runs,
/// bounded by the spawner's four running children.
///
/// Thematch keys are `app_name` (exact), `desktop_entry` (exact),
/// `category` (exact, or a `class.` prefix when it ends with a dot:
/// `category = "device."` matches `device.added`), `urgency` (exact)
/// and `summary_contains` (a plain substring of the summary, never a
/// regex). The action keys are `set_urgency` (the design's `urgency`
/// action, renamed: one table cannot carry the match `urgency` and the
/// action under the same name), `timeout_ms` (0 never expires, otherwise
/// milliseconds, honored as written), `hide` (straight to history,
/// never on screen), `history` (false never enters history),
/// `output` (pins this card's output, over the file's), `dnd_bypass`
/// (shows through DND like a critical card) and `exec` (an argv run
/// with the notification's fields in `SCOOTNOTIFY_*` variables, under
/// the spawner's bounds: argv not shell, its own process group, reaped
/// by pidfd, at most 4 running).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Rule {
    /// Matches this app name exactly.
    pub app_name: Option<String>,
    /// Matches this desktop entry exactly.
    pub desktop_entry: Option<String>,
    /// Matches this category exactly, or by prefix past a trailing dot.
    pub category: Option<String>,
    /// Matches this urgency exactly.
    pub urgency: Option<RuleUrgency>,
    /// Matches when the summary contains this substring.
    pub summary_contains: Option<String>,
    /// Sets the card's urgency.
    pub set_urgency: Option<RuleUrgency>,
    /// Overrides the resolved expiry: 0 never, otherwise milliseconds.
    pub timeout_ms: Option<u32>,
    /// Hides the card: straight to history, never on screen.
    pub hide: bool,
    /// False keeps the card out of history (held and closed alike).
    pub history: bool,
    /// Pins this card's output, over the file's `output`.
    pub output: Option<String>,
    /// Shows through DND, like a critical card does by default.
    pub dnd_bypass: bool,
    /// An argv run on arrival, never through a shell.
    pub exec: Option<Vec<String>>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            app_name: None,
            desktop_entry: None,
            category: None,
            urgency: None,
            summary_contains: None,
            set_urgency: None,
            timeout_ms: None,
            hide: false,
            history: true,
            output: None,
            dnd_bypass: false,
            exec: None,
        }
    }
}

impl Rule {
    /// Range-checks one `[[rule]]` table: bounded match strings, a
    /// bounded `timeout_ms`, a usable `output`, and an `exec` argv that
    /// is really an argv (words, each short and NUL-free). `Err` names
    /// the rule by its file order (1-based) and the key.
    fn check(&self, index: usize) -> Result<(), String> {
        let name = format!("rule {}", index + 1);
        for (key, value) in [
            ("app_name", self.app_name.as_deref()),
            ("desktop_entry", self.desktop_entry.as_deref()),
            ("category", self.category.as_deref()),
            ("summary_contains", self.summary_contains.as_deref()),
        ] {
            if let Some(value) = value {
                if value.is_empty() || value.len() > 128 {
                    return Err(format!(
                        "config refused: {name} {key} is {} bytes (1 to 128)",
                        value.len()
                    ));
                }
                if value.bytes().any(|byte| byte == 0 || byte == 0x7f)
                    || value.contains('\n')
                    || value.contains('\r')
                {
                    return Err(format!(
                        "config refused: {name} {key} carries a control character"
                    ));
                }
            }
        }
        if let Some(timeout) = self.timeout_ms
            && timeout > 86_400_000
        {
            return Err(format!(
                "config refused: {name} timeout_ms is {timeout} (0 to 86400000)"
            ));
        }
        if let Some(output) = &self.output
            && (output.is_empty()
                || output.len() > 256
                || output.bytes().any(|byte| byte < 0x20 || byte == 0x7f))
        {
            return Err(format!(
                "config refused: {name} output is not a usable output name (1 to 256 printable bytes)"
            ));
        }
        if let Some(argv) = &self.exec {
            if argv.is_empty() || argv.len() > MAX_EXEC_ARGV {
                return Err(format!(
                    "config refused: {name} exec carries {} words (1 to {MAX_EXEC_ARGV})",
                    argv.len()
                ));
            }
            for word in argv {
                if word.is_empty() || word.len() > MAX_EXEC_WORD {
                    return Err(format!(
                        "config refused: {name} exec carries a {}-byte word (1 to {MAX_EXEC_WORD})",
                        word.len()
                    ));
                }
                if word.bytes().any(|byte| byte == 0) {
                    return Err(format!(
                        "config refused: {name} exec carries a NUL byte"
                    ));
                }
            }
        }
        Ok(())
    }
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
            history_max: 64,
            dnd_allows_critical: true,
            rule: Vec::new(),
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
        if self.history_max > 256 {
            return Err(format!(
                "config refused: history_max is {} (0 to 256)",
                self.history_max
            ));
        }
        if self.rule.len() > MAX_RULES {
            return Err(format!(
                "config refused: {} [[rule]] tables (at most {MAX_RULES})",
                self.rule.len()
            ));
        }
        for (index, rule) in self.rule.iter().enumerate() {
            rule.check(index)?;
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
            return Err(
                "config refused: output is not a usable output name (1 to 256 printable bytes)"
                    .to_string(),
            );
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
