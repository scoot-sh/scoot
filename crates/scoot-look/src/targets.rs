//! User-owned look targets: render any app's config from a template.
//!
//! A target is a directory under `<config>/scoot/look-targets/<name>/`
//! with a `target.toml` (the output path plus the user's own reload
//! command) and a template file. On every `scoot look apply` the tool
//! substitutes the active look's tokens into each template and writes
//! the result into the rendered look under `targets/<output>`, beside
//! the built-in fragments, under the same atomic swap, cache key and
//! rollback rules.
//!
//! Pure by contract like the rest of this crate: no filesystem, no
//! process spawning. Discovery, the config-home check and running the
//! reload argv live in the caller (`scootctl`), never here.
//!
//! Substitution is pure text replacement with no logic that can run:
//! `{{ palette.background }}` names a token, with optional `| hex`,
//! `| nohash`, `| rgb` or `| rgba(<alpha>)` filters for color forms.
//! There are no conditionals, loops, includes or escapes in v1: a
//! template needing a literal `{{` has none to write (documented in
//! the site page). Unknown tokens and unknown filters refuse at
//! validate time, so a typo never swaps a half-rendered file.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::grammar::{GrammarError, MAX_FILE_BYTES, is_hex_color, is_identifier, is_name};
use crate::resolved::ResolvedLook;

/// The longest `target.toml` this crate parses: the same 64 KiB as every
/// other look file. Theme templates are small; anything larger is a mistake.
pub const MAX_TARGET_TOML_BYTES: usize = MAX_FILE_BYTES;

/// The longest template accepted: the same 64 KiB. Rendered app configs
/// are small (the kitty example is under 2 KiB); anything larger is not a
/// theme fragment but a mistake.
pub const MAX_TEMPLATE_BYTES: usize = MAX_FILE_BYTES;

/// How many reload argv entries a target may declare: a reload names one
/// program plus its arguments, never a script.
pub const MAX_RELOAD_ARGV: usize = 16;

/// The longest one reload argv entry: the same 512 bytes as any other
/// look string bound.
pub const MAX_RELOAD_ARG_BYTES: usize = 512;

/// The default reload wait: the same 30 s as the scoped reloads and the
/// effectful helpers, so a hung child fails the same way everywhere.
pub const DEFAULT_RELOAD_TIMEOUT_SECS: u64 = 30;

/// The longest reload wait a target may declare.
pub const MAX_RELOAD_TIMEOUT_SECS: u64 = 300;

/// A parsed `target.toml`: what one user-owned target says. Every field
/// is validated by [`validate_target`] before anything renders from it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTarget {
    /// Must match the target directory name (`[a-z0-9-]{1,32}`).
    pub name: String,
    /// Where the rendered file lands: relative to the user's config home
    /// (`kitty/colors.conf`), and mirrored under `targets/` in the render.
    pub output: String,
    /// The template file inside the target directory.
    pub template: String,
    /// The user's own reload command, run after a successful swap.
    #[serde(default)]
    pub reload: Option<Reload>,
}

/// The user's own reload: an argv list run directly (never through a
/// shell) with a bounded wait. Failure is a note, never a rollback.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reload {
    /// The program plus its arguments: 1 to 16 entries.
    pub argv: Vec<String>,
    /// How long to wait, in seconds: 1 to 300, default 30.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Parse one `target.toml` file: malformed TOML, an unknown table or key,
/// or any value breaking the grammar refuses. Required-ness of the
/// directory match is [`validate_target`], which sees the directory name.
pub fn parse_target_toml(bytes: &[u8]) -> Result<RawTarget, Vec<GrammarError>> {
    if bytes.len() > MAX_TARGET_TOML_BYTES {
        return Err(vec![GrammarError::new(
            "target.toml",
            format!("file larger than {MAX_TARGET_TOML_BYTES} bytes"),
        )]);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| vec![GrammarError::new("target.toml", "file is not valid UTF-8")])?;
    if text.chars().any(|c| c == '\0') {
        return Err(vec![GrammarError::new(
            "target.toml",
            "NUL is refused everywhere",
        )]);
    }
    let raw: RawTarget =
        toml::from_str(text).map_err(|e| vec![GrammarError::new("target.toml", e.to_string())])?;
    let mut errors = Vec::new();
    validate_target_shape(&raw, &mut errors);
    if errors.is_empty() {
        Ok(raw)
    } else {
        Err(errors)
    }
}

/// The directory-independent shape checks: the name grammar, the output
/// and template path grammar, the reload argv grammar. The
/// directory-name match and the config-home containment live with the
/// caller, beside the filesystem they need.
pub fn validate_target_shape(raw: &RawTarget, errors: &mut Vec<GrammarError>) {
    if !is_name(&raw.name) {
        errors.push(GrammarError::new("name", "must match [a-z0-9-]{1,32}"));
    }
    if let Some(e) = check_target_path("output", &raw.output) {
        errors.push(e);
    }
    if let Some(e) = check_target_path("template", &raw.template) {
        errors.push(e);
    }
    if let Some(reload) = &raw.reload {
        validate_reload(reload, errors);
    }
}

/// Whether `dir_name` is what this target says it is: the site page says
/// a target's `name` matches its directory name, and the tool enforces it
/// where the directory is known. Pure over the already-read directory
/// name, so it stays testable.
pub fn check_name_matches_dir(raw: &RawTarget, dir_name: &str) -> Option<GrammarError> {
    if raw.name != dir_name {
        Some(GrammarError::new(
            "name",
            format!("must match the target directory name `{dir_name}`"),
        ))
    } else {
        None
    }
}

/// One relative path inside a target (`output` under the config home,
/// `template` inside the target directory): relative, normalized, no
/// `..`, no leading `/`, no backslash, no hidden (dot-leading) segment.
/// An `output` must also be ASCII: one file can have two Unicode
/// spellings (NFC `caf\u{e9}` vs NFD `cafe\u{301}`), which would stage
/// as silent duplicates past a byte compare — and no normalization crate
/// sits in the workspace lock tree, so the refusal happens here instead.
/// A template has no duplicate comparison run over it, so Unicode names
/// stay allowed there.
/// The containment itself (staying under the config home through
/// symlinks) is the caller's, beside the filesystem.
pub fn check_target_path(field: &str, value: &str) -> Option<GrammarError> {
    if value.is_empty() || value.len() > 512 {
        return Some(GrammarError::new(field, "path must be 1 to 512 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Some(GrammarError::new(
            field,
            "path must not contain control characters",
        ));
    }
    if field == "output" && !value.is_ascii() {
        return Some(GrammarError::new(
            field,
            "path must be ASCII: one file can have two Unicode spellings",
        ));
    }
    if value.starts_with('/') {
        return Some(GrammarError::new(
            field,
            "path must be relative, not absolute",
        ));
    }
    if value.contains('\\') {
        return Some(GrammarError::new(field, "path must not contain `\\`"));
    }
    for segment in value.split('/') {
        // Normalized: no `.`, `..`, `//` or trailing `/`, and no
        // hidden files or directories (a target never writes a dotfile).
        if segment.is_empty() || segment == "." {
            return Some(GrammarError::new(
                field,
                "path must be normalized: no `.`, `//` or trailing `/`",
            ));
        }
        if segment == ".." {
            return Some(GrammarError::new(field, "path must not contain `..`"));
        }
        if segment.starts_with('.') {
            return Some(GrammarError::new(
                field,
                "path must not name hidden files or directories",
            ));
        }
        if segment.len() > 64 {
            return Some(GrammarError::new(
                field,
                "each path segment must be at most 64 bytes",
            ));
        }
    }
    None
}

fn validate_reload(reload: &Reload, errors: &mut Vec<GrammarError>) {
    if reload.argv.is_empty() || reload.argv.len() > MAX_RELOAD_ARGV {
        errors.push(GrammarError::new(
            "reload.argv",
            format!("must hold 1 to {MAX_RELOAD_ARGV} entries"),
        ));
    }
    for (i, arg) in reload.argv.iter().enumerate() {
        let field = format!("reload.argv[{i}]");
        if arg.is_empty() || arg.len() > MAX_RELOAD_ARG_BYTES {
            errors.push(GrammarError::new(
                &field,
                format!("must be 1 to {MAX_RELOAD_ARG_BYTES} bytes"),
            ));
            continue;
        }
        if arg.contains('\0') || arg.chars().any(|c| c.is_control()) {
            errors.push(GrammarError::new(
                &field,
                "must not contain control characters",
            ));
        }
    }
    // A relative program with a `/` in it resolves against nothing the
    // tool defines (the swap runs with the tool's cwd, not the target
    // dir): only a bare name (found on `PATH`) or an absolute path.
    if let Some(program) = reload.argv.first()
        && program.contains('/')
        && !program.starts_with('/')
    {
        errors.push(GrammarError::new(
            "reload.argv[0]",
            "the program must be a bare name on PATH or an absolute path, never a relative path",
        ));
    }
    match reload.timeout_secs {
        Some(secs) if !(1..=MAX_RELOAD_TIMEOUT_SECS).contains(&secs) => {
            errors.push(GrammarError::new(
                "reload.timeout_secs",
                format!("must be 1 to {MAX_RELOAD_TIMEOUT_SECS} seconds"),
            ));
        }
        _ => {}
    }
}

/// The effective reload wait: the declared value or the 30 s default.
pub fn reload_timeout(reload: &Reload) -> u64 {
    reload.timeout_secs.unwrap_or(DEFAULT_RELOAD_TIMEOUT_SECS)
}

/// Every token a template may name, as dotted paths to rendered strings.
/// The set is the look's own tokens with the look's own spellings
/// (`palette.accent-fg`, `terminal.color4`, `fonts.mono`): whatever the
/// look carries is what a template can say. Tokens the look does not set
/// (a missing `[terminal]` table, an unset `[effects]` key, no
/// `[night]`) are absent, so naming one refuses instead of rendering an
/// empty string silently.
pub fn target_tokens(look: &ResolvedLook) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    let p = &look.palette;
    for (key, color) in [
        ("palette.background", &p.background),
        ("palette.foreground", &p.foreground),
        ("palette.dim", &p.dim),
        ("palette.accent", &p.accent),
        ("palette.urgent", &p.urgent),
        ("palette.surface", &p.surface),
        ("palette.hover", &p.hover),
        ("palette.focus", &p.focus),
        ("palette.border", &p.border),
        ("palette.accent-fg", &p.accent_fg),
        ("palette.selection", &p.selection),
        ("palette.selection-text", &p.selection_text),
        ("palette.match", &p.r#match),
    ] {
        tokens.insert(key.to_owned(), color.clone());
    }
    if let Some(colors) = &look.terminal {
        for (i, color) in colors.iter().enumerate() {
            tokens.insert(format!("terminal.color{i}"), color.clone());
        }
    }
    for (key, family) in [
        ("fonts.ui", &look.fonts.ui),
        ("fonts.sans", &look.fonts.sans),
        ("fonts.mono", &look.fonts.mono),
    ] {
        tokens.insert(key.to_owned(), family.clone());
    }
    for (key, size) in [
        ("fonts.size", look.fonts.size),
        ("fonts.popup-size", look.fonts.popup_size),
        ("fonts.app-size", look.fonts.app_size),
        ("fonts.terminal-size", look.fonts.terminal_size),
    ] {
        tokens.insert(key.to_owned(), format_number(size));
    }
    for (key, value) in [
        ("opacity.bar", look.opacity.bar),
        ("opacity.popups", look.opacity.popups),
        ("opacity.terminal", look.opacity.terminal),
    ] {
        tokens.insert(key.to_owned(), format_number(value));
    }
    for (key, value) in [
        ("shape.radius", look.shape.radius),
        ("shape.ring-width", look.shape.ring_width),
        ("shape.ring-inactive-width", look.shape.ring_inactive_width),
    ] {
        tokens.insert(key.to_owned(), value.to_string());
    }
    for (key, value) in [
        ("effects.shadow", look.effects.shadow.as_ref()),
        ("effects.fill", look.effects.fill.as_ref()),
        ("effects.motion", look.effects.motion.as_ref()),
    ] {
        if let Some(value) = value {
            tokens.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(ms) = look.effects.motion_duration_ms {
        tokens.insert("effects.motion-duration-ms".to_owned(), ms.to_string());
    }
    if let Some(bounce) = look.effects.motion_bounce {
        tokens.insert("effects.motion-bounce".to_owned(), format_number(bounce));
    }
    tokens.insert("cursor.theme".to_owned(), look.cursor.theme.clone());
    tokens.insert("cursor.size".to_owned(), look.cursor.size.to_string());
    tokens.insert("cursor.accent".to_owned(), look.cursor.accent.clone());
    if let Some(body) = &look.cursor.body {
        tokens.insert("cursor.body".to_owned(), body.clone());
    }
    tokens.insert(
        "polarity".to_owned(),
        match look.polarity {
            crate::schema::Polarity::Dark => "dark".to_owned(),
            crate::schema::Polarity::Light => "light".to_owned(),
        },
    );
    if let Some(wallpaper) = &look.wallpaper {
        if let Some(color) = &wallpaper.color {
            tokens.insert("wallpaper.color".to_owned(), color.clone());
        }
        if let Some(mode) = wallpaper.mode {
            tokens.insert("wallpaper.mode".to_owned(), mode.as_str().to_owned());
        }
        if let Some(fill) = &wallpaper.fill {
            tokens.insert("wallpaper.fill".to_owned(), fill.clone());
        }
        if let Some(transition) = wallpaper.transition {
            tokens.insert(
                "wallpaper.transition".to_owned(),
                transition.as_str().to_owned(),
            );
        }
    }
    tokens.insert("icons.theme".to_owned(), look.icons_theme.clone());
    if let Some(temperature) = look.night_temperature {
        tokens.insert("night.temperature".to_owned(), temperature.to_string());
    }
    tokens
}

/// What kind of value a `[targets.<name>]` key takes, which is also the
/// grammar its value is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideKind {
    /// A palette role or terminal color: exactly `#rrggbb`.
    Color,
    /// A font family (`fonts.ui`, `fonts.sans`, `fonts.mono`): the same
    /// identifier the look's own `[fonts]` table takes.
    Family,
    /// A font size (`fonts.size`, `fonts.popup-size`, `fonts.app-size`,
    /// `fonts.terminal-size`): a TOML number, 4 to 192 points, like the
    /// look's own `[fonts]` table.
    Size,
}

/// A `[targets.<name>]` value as written: a string (colors, families) or a
/// TOML number (sizes). Which one a key takes is [`override_kind`].
#[derive(Debug, Clone, PartialEq)]
pub enum OverrideValue {
    /// A quoted string.
    Text(String),
    /// A TOML integer or float.
    Number(f64),
}

impl<'de> Deserialize<'de> for OverrideValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = OverrideValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(
                    "a quoted string (colors, font families) or a number (font sizes); booleans, arrays, tables and dates are not accepted",
                )
            }
            fn visit_str<E>(self, v: &str) -> Result<OverrideValue, E> {
                Ok(OverrideValue::Text(v.to_owned()))
            }
            fn visit_i64<E>(self, v: i64) -> Result<OverrideValue, E> {
                Ok(OverrideValue::Number(v as f64))
            }
            fn visit_u64<E>(self, v: u64) -> Result<OverrideValue, E> {
                Ok(OverrideValue::Number(v as f64))
            }
            fn visit_f64<E>(self, v: f64) -> Result<OverrideValue, E> {
                Ok(OverrideValue::Number(v))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// The kind of an overridable token, or `None` when the key is not one:
/// the thirteen palette roles and sixteen terminal colors (colors), the
/// three font families and the four font sizes. Every other token
/// (`opacity.*`, `shape.*`, `cursor.*`, `polarity`, ...) stays
/// non-overridable.
pub fn override_kind(key: &str) -> Option<OverrideKind> {
    const PALETTE: [&str; 13] = [
        "palette.background",
        "palette.foreground",
        "palette.dim",
        "palette.accent",
        "palette.urgent",
        "palette.surface",
        "palette.hover",
        "palette.focus",
        "palette.border",
        "palette.accent-fg",
        "palette.selection",
        "palette.selection-text",
        "palette.match",
    ];
    if PALETTE.contains(&key) {
        return Some(OverrideKind::Color);
    }
    match key {
        "fonts.ui" | "fonts.sans" | "fonts.mono" => return Some(OverrideKind::Family),
        "fonts.size" | "fonts.popup-size" | "fonts.app-size" | "fonts.terminal-size" => {
            return Some(OverrideKind::Size);
        }
        _ => {}
    }
    key.strip_prefix("terminal.color")
        .filter(|n| {
            n.parse::<u8>()
                .is_ok_and(|v| v <= 15 && v.to_string() == *n)
        })
        .map(|_| OverrideKind::Color)
}

/// Validates every `[targets.<name>]` table of a look at parse time, the
/// one place `validate`, `build`, `install` and every swap share: the
/// target name grammar, an allowed token per key, and a value held to
/// that token's own grammar (`#rrggbb`, a font identifier, 4 to 192
/// points). A key like `output`, `template` or `reload` is not a token,
/// so a look cannot smuggle a template, output path or reload command in.
/// Whether a user target of that name exists is not checked here: the
/// look is shared, the user's targets are local, and an override for a
/// target the user has not enabled does nothing.
pub fn validate_overrides(
    targets: &BTreeMap<String, BTreeMap<String, OverrideValue>>,
    errors: &mut Vec<GrammarError>,
) {
    for (target, table) in targets {
        if !is_name(target) {
            errors.push(GrammarError::new(
                format!("targets.{target}"),
                "target name must match [a-z0-9-]{1,32}",
            ));
            continue;
        }
        for (key, value) in table {
            let field = format!("targets.{target}[{key}]");
            match (override_kind(key), value) {
                (None, _) => errors.push(GrammarError::new(
                    &field,
                    "not an overridable token: only `palette.*` colors, `terminal.color0` to `terminal.color15`, `fonts.ui`/`fonts.sans`/`fonts.mono` and the four font sizes",
                )),
                (Some(OverrideKind::Color), OverrideValue::Text(v)) if is_hex_color(v) => {}
                (Some(OverrideKind::Color), _) => {
                    errors.push(GrammarError::new(&field, "must be exactly #rrggbb"));
                }
                (Some(OverrideKind::Family), OverrideValue::Text(v)) if is_identifier(v) => {}
                (Some(OverrideKind::Family), _) => errors.push(GrammarError::new(
                    &field,
                    "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
                )),
                (Some(OverrideKind::Size), OverrideValue::Number(pt))
                    if (4.0..=192.0).contains(pt) => {}
                (Some(OverrideKind::Size), _) => {
                    errors.push(GrammarError::new(&field, "must be a number from 4 to 192 points"));
                }
            }
        }
    }
}

/// The token text an already-validated override renders as: the string
/// itself, or a size spelled like the look's own size tokens (`12`, never
/// `12.0`).
pub(crate) fn override_text(value: &OverrideValue) -> String {
    match value {
        OverrideValue::Text(text) => text.clone(),
        OverrideValue::Number(number) => format_number(*number),
    }
}

/// The tokens one target renders against: the look's own, with that
/// target's `[targets.<name>]` overrides replacing (or, for a
/// `terminal.colorN` the look does not set, adding) their keys. Other
/// targets never see them.
pub fn tokens_for_target<'a>(
    base: &'a BTreeMap<String, String>,
    look: &ResolvedLook,
    target: &str,
) -> std::borrow::Cow<'a, BTreeMap<String, String>> {
    match look.target_overrides.get(target) {
        Some(table) if !table.is_empty() => {
            let mut tokens = base.clone();
            for (key, value) in table {
                tokens.insert(key.clone(), value.clone());
            }
            std::borrow::Cow::Owned(tokens)
        }
        _ => std::borrow::Cow::Borrowed(base),
    }
}

/// A whole-number float renders without its `.0` (`10`, never `10.0`),
/// like the GTK font description; anything else renders shortest
/// round-trip (`{:?}`), never NaN or infinite (those refuse upstream).
fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value:?}")
    }
}

/// Substitute every `{{ token }}` / `{{ token | filter }}` in `template`
/// against `tokens`. Pure text replacement: no conditionals, no loops,
/// no includes, no code execution. A template with no tokens renders
/// unchanged. Unknown tokens, unknown filters, filters on non-colors
/// and an unclosed `{{` refuse; a stray `}}` with no opening `{{` stays
/// literal (JSON objects nest `}}`, so refusing those would break real
/// configs).
pub fn render_template(
    template: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<String, GrammarError> {
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err(GrammarError::new(
            "template",
            format!("template larger than {MAX_TEMPLATE_BYTES} bytes"),
        ));
    }
    if template.contains('\0') {
        return Err(GrammarError::new("template", "NUL is refused everywhere"));
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(GrammarError::new(
                "template",
                "unclosed `{{`: a token needs its closing `}}`",
            ));
        };
        let inner = after[..close].trim().to_owned();
        let rendered = render_token(&inner, tokens)?;
        out.push_str(&rendered);
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Render one `{{ ... }}` interior: `path` or `path | filter`.
fn render_token(inner: &str, tokens: &BTreeMap<String, String>) -> Result<String, GrammarError> {
    if inner.is_empty() {
        return Err(GrammarError::new(
            "template",
            "empty `{{ }}`: name a token like `{{ palette.background }}`",
        ));
    }
    let (path, filter) = match inner.split_once('|') {
        Some((path, filter)) => (path.trim(), Some(filter.trim())),
        None => (inner.trim(), None),
    };
    if path.is_empty() {
        return Err(GrammarError::new(
            "template",
            "empty token: name a token like `{{ palette.background }}`",
        ));
    }
    let value = tokens.get(path).ok_or_else(|| {
        GrammarError::new(
            "template",
            format!("unknown token `{path}`: no look token spells that"),
        )
    })?;
    match filter {
        None => Ok(value.clone()),
        Some(filter) => apply_filter(path, value, filter),
    }
}

/// One `| filter` on a token value. Filters only re-spell `#rrggbb`
/// colors (with `#`, without it, as `R,G,B`, as `rgba(R,G,B,alpha)`):
/// a filter on anything else refuses, so a typo in the token name
/// cannot hide behind a filter that would pass any string through.
fn apply_filter(path: &str, value: &str, filter: &str) -> Result<String, GrammarError> {
    match filter {
        "hex" => {
            require_color(path, value)?;
            Ok(value.to_owned())
        }
        "nohash" => Ok(require_color(path, value)?[1..].to_owned()),
        "rgb" => {
            let hex = require_color(path, value)?;
            let (r, g, b) = hex_to_rgb(hex)?;
            Ok(format!("{r},{g},{b}"))
        }
        _ => {
            if let Some(alpha) = filter
                .strip_prefix("rgba(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                let alpha = alpha.trim();
                let parsed: f64 = alpha.parse().map_err(|_| {
                    GrammarError::new(
                        "template",
                        format!("filter `rgba(...)` needs a number from 0 to 1, not `{alpha}`"),
                    )
                })?;
                if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
                    return Err(GrammarError::new(
                        "template",
                        format!("filter `rgba(...)` needs a number from 0 to 1, not `{alpha}`"),
                    ));
                }
                let hex = require_color(path, value)?;
                let (r, g, b) = hex_to_rgb(hex)?;
                return Ok(format!("rgba({r},{g},{b},{alpha})"));
            }
            Err(GrammarError::new(
                "template",
                format!(
                    "unknown filter `{filter}`: one of `hex`, `nohash`, `rgb`, `rgba(<0 to 1>)`"
                ),
            ))
        }
    }
}

/// The `#rrggbb` shape a color filter needs, or the refusal a
/// non-color token earns. The token map only holds validated colors,
/// but a library caller can hand-build past that gate, so the check
/// runs here too rather than slicing blindly.
fn require_color<'a>(path: &str, value: &'a str) -> Result<&'a str, GrammarError> {
    if crate::grammar::is_hex_color(value) {
        Ok(value)
    } else {
        Err(GrammarError::new(
            "template",
            format!("filter on `{path}` needs a `#rrggbb` color"),
        ))
    }
}

fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), GrammarError> {
    let pair = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| {
            GrammarError::new("template", "filter on a color needs `#rrggbb` hex digits")
        })
    };
    Ok((pair(1)?, pair(3)?, pair(5)?))
}

#[cfg(test)]
mod tests;
