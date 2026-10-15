//! Pure renderers for the first-party fragments (design section 3.3):
//! `scoot.toml` and `bar.toml` from a resolved look plus the environment's
//! resolved inputs (font files, the wallpaper's absolute path, the target
//! binary's feature set).
//!
//! The rendered fragments must pass the apps' own strict parsers, so the
//! renderers only emit keys those parsers accept today. One known naming
//! divergence: the design calls the cursor fill `cursor_accent` ("after
//! #572"), but the compositor still reads `cursor_color`
//! (`crates/scoot/src/compositor/config.rs:176`), so the renderer emits
//! `cursor_color`. `look-layer-scoot` renames it if it adds the field.

use std::collections::BTreeMap;

use crate::allowlist::check_app_value;
use crate::grammar::{GrammarError, is_hex_color};
use crate::resolved::{ResolvedLook, pt_to_px};
use crate::schema::Polarity;

/// The largest `bar.radius` the renderer emits: half scootbar's default
/// height (`DEFAULT_HEIGHT = 28` in `crates/scootbar/src/bar.rs:28-30`;
/// the `radius <= height / 2` refusal in `config.rs:1069-1081`). A look
/// never sets the bar height, so the height in effect is the default.
/// Overrides parse to 512 (valid under a taller user bar) and clamp
/// here, so the fragment passes scootbar's own parser as rendered.
pub const BAR_RADIUS_LIMIT: i64 = 14;

/// What the environment resolved: the tool (or the Nix build) fills these
/// in; the pure crate never touches the filesystem, fontconfig or the
/// network to find them.
#[derive(Debug, Clone, Default)]
pub struct RenderInputs {
    /// Absolute path of the wallpaper image. Required when the look names an
    /// image wallpaper; unused for a color wallpaper.
    pub wallpaper_abs: Option<String>,
    /// Resolved UI font file for the bar (`--ui-font-file`). When absent
    /// the fragment carries no `font` key and the bar falls back to its
    /// well-known font paths: scootbar reads a file path here, never a
    /// family name (`crates/scootbar/src/config.rs:87-90`).
    pub ui_font_file: Option<String>,
    /// Whether the target scootbar binary has the `popup` feature
    /// (`crates/scootbar/src/config.rs:396-401`): `popup-radius` is only
    /// emitted then, because the file denies unknown keys.
    pub bar_popup: bool,
    /// Override sections (the first dotted segment of an `[apps.scootbar]`
    /// key: `workspaces`, `clock`, ...) the target scootbar binary lacks:
    /// a build without that module's feature denies the whole table
    /// (`unknown field inactive-color, there are no fields` on a minimal
    /// bar), so those tokens are not rendered at all. Sorted, deduplicated
    /// by the tool; the renderer skips them defensively.
    pub bar_drop_sections: Vec<String>,
}

/// Quote one TOML string value. Every input is already grammar-checked, so
/// this is the renderer's own quoting on top, never the only defense.
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_value(v: &toml::Value) -> Result<String, GrammarError> {
    match v {
        toml::Value::String(s) => Ok(toml_str(s)),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => {
            if f.is_finite() {
                Ok(format!("{f:?}"))
            } else {
                Ok("0.0".to_string())
            }
        }
        toml::Value::Boolean(b) => Ok(b.to_string()),
        toml::Value::Array(items) => {
            let mut rendered = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    toml::Value::Array(_) | toml::Value::Table(_) => {
                        return Err(GrammarError::new(
                            "apps",
                            "per-app overrides must be strings, integers, floats, booleans or flat arrays of those",
                        ));
                    }
                    _ => rendered.push(toml_value(item)?),
                }
            }
            Ok(format!("[{}]", rendered.join(", ")))
        }
        _ => Err(GrammarError::new(
            "apps",
            "per-app overrides must be strings, integers, floats, booleans or flat arrays of those",
        )),
    }
}

/// One TOML key: bare when it is already bare, quoted otherwise. The
/// renderer never trusts the grammar alone: even a key that slipped past
/// validation renders quoted, so a newline, `=`, `[` or quote in it can
/// never open a new table or key.
fn toml_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        key.to_string()
    } else {
        toml_str(key)
    }
}

/// A dotted key path: each segment quoted on its own, so `output.My
/// Monitor.image` renders as `output."My Monitor".image`.
fn toml_dotted(path: &str) -> String {
    path.split('.').map(toml_key).collect::<Vec<_>>().join(".")
}

/// Split `[apps.<app>]` overrides by their first dotted segment, checking
/// each key's name and value against the allowlist's kinds. The loader
/// re-checks; the renderer never trusts the tool, not even itself: the
/// parse gate refuses bad values, but a library caller can hand-build a
/// `ResolvedLook` past it, so this path checks values too, not only
/// names. Values render through [`toml_value`]; sections map each key
/// once, so a repeated key is one entry, never two.
fn override_sections(
    app: &str,
    apps: &BTreeMap<String, BTreeMap<String, toml::Value>>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, GrammarError> {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    if let Some(table) = apps.get(app) {
        for (key, value) in table {
            if let Some(e) = check_app_value(app, key, value) {
                return Err(e);
            }
            let (section, rest) = key.split_once('.').unwrap_or(("", key));
            sections
                .entry(section.to_string())
                .or_default()
                .insert(rest.to_string(), toml_value(value)?);
        }
    }
    Ok(sections)
}

/// Render the scoot fragment (`scoot.toml`): `[appearance]` colors and
/// widths, the cursor triple, `corner_radius`, and `[wallpaper]` as one
/// unit. Never autostart, binds, window rules, `[tty]`, `[renderer]`,
/// `[xwayland]` or `[virtual_input]`.
///
/// Overrides merge into the base tables: each section is emitted once, so
/// an override of `appearance.background_color` replaces the derived value
/// instead of opening a duplicate `[appearance]` table.
pub fn render_scoot(look: &ResolvedLook, inputs: &RenderInputs) -> Result<String, GrammarError> {
    let p = &look.palette;
    // A hand-built `ResolvedLook` skips the name grammar, so the comment
    // quotes rather than interpolates: a newline in the name stays one line.
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n",
        toml_str(&look.name)
    );
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let appearance = sections.entry("appearance".to_string()).or_default();
    appearance.insert(
        "focus_ring_width".to_string(),
        look.shape.ring_width.to_string(),
    );
    appearance.insert(
        "focus_ring_inactive_width".to_string(),
        look.shape.ring_inactive_width.to_string(),
    );
    appearance.insert("focus_ring_active_color".to_string(), toml_str(&p.focus));
    appearance.insert("focus_ring_inactive_color".to_string(), toml_str(&p.border));
    appearance.insert("background_color".to_string(), toml_str(&p.background));
    appearance.insert("corner_radius".to_string(), look.shape.radius.to_string());
    appearance.insert("cursor_theme".to_string(), toml_str(&look.cursor.theme));
    appearance.insert("cursor_size".to_string(), look.cursor.size.to_string());
    if look.cursor.accent != "none" {
        appearance.insert("cursor_color".to_string(), toml_str(&look.cursor.accent));
    }
    if let Some(wallpaper) = &look.wallpaper {
        let table = sections.entry("wallpaper".to_string()).or_default();
        if let Some(color) = &wallpaper.color {
            table.insert("color".to_string(), toml_str(color));
        } else {
            let abs = inputs.wallpaper_abs.as_ref().ok_or_else(|| {
                GrammarError::new(
                    "wallpaper.image",
                    "an image wallpaper needs its resolved absolute path",
                )
            })?;
            table.insert("image".to_string(), toml_str(abs));
        }
        table.insert(
            "mode".to_string(),
            toml_str(wallpaper.mode.map(|m| m.as_str()).unwrap_or("fill")),
        );
        table.insert(
            "fill".to_string(),
            toml_str(
                &wallpaper
                    .fill
                    .clone()
                    .unwrap_or_else(|| p.background.clone()),
            ),
        );
        table.insert(
            "transition".to_string(),
            toml_str(wallpaper.transition.map(|t| t.as_str()).unwrap_or("none")),
        );
    }
    for (section, keys) in override_sections("scoot", &look.apps)? {
        sections.entry(section).or_default().extend(keys);
    }
    for (section, keys) in &sections {
        out.push_str(&format!("\n[{}]\n", toml_key(section)));
        for (key, value) in keys {
            out.push_str(&format!("{} = {value}\n", toml_dotted(key)));
        }
    }
    Ok(out)
}

/// Render the scootbar fragment (`bar.toml`): `[colors]`, the bar's style,
/// and allowlisted per-module visual keys. Never geometry, placement,
/// module lists or actions.
///
/// Like [`render_scoot`], overrides merge into the base tables: an override
/// of `colors.accent` replaces the derived value in `[colors]` instead of
/// appending a second entry, and `bar.*` overrides land in `[bar]`, never
/// as `bar.bar.*`.
pub fn render_bar(look: &ResolvedLook, inputs: &RenderInputs) -> Result<String, GrammarError> {
    let p = &look.palette;
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n",
        toml_str(&look.name)
    );
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let colors = sections.entry("colors".to_string()).or_default();
    for (key, color) in [
        ("background", &p.background),
        ("foreground", &p.foreground),
        ("accent", &p.accent),
        ("hover", &p.hover),
        ("dim", &p.dim),
        ("urgent", &p.urgent),
    ] {
        colors.insert(key.to_string(), toml_str(color));
    }
    let bar = sections.entry("bar".to_string()).or_default();
    bar.insert("radius".to_string(), look.shape.radius.to_string());
    if inputs.bar_popup {
        bar.insert("popup-radius".to_string(), look.shape.radius.to_string());
    }
    bar.insert("opacity".to_string(), format!("{:?}", look.opacity.bar));
    // scootbar's `font` is a file path, never the family name `fonts.ui`
    // holds (`crates/scootbar/src/config.rs:87-90`, `font.rs:1`): emit
    // it only with a resolved path, so a default `build` never writes a
    // `bar.toml` the font load refuses. Without one the bar falls back
    // to its well-known font paths.
    if let Some(font) = &inputs.ui_font_file {
        bar.insert("font".to_string(), toml_str(font));
    }
    bar.insert(
        "font-size".to_string(),
        pt_to_px(look.fonts.size).to_string(),
    );
    for (section, keys) in override_sections("scootbar", &look.apps)? {
        // Tokens for modules the installed bar lacks never render: the
        // tool drops those sections against the binary's own parser (the
        // way `popup-radius` is dropped for a bar without `popup`), so a
        // shipped look still applies on the lightest bar.
        if inputs
            .bar_drop_sections
            .iter()
            .any(|dropped| dropped.as_str() == section)
        {
            continue;
        }
        sections.entry(section).or_default().extend(keys);
    }
    // Clamp `bar.radius` — derived and override alike — to what scootbar
    // accepts under the height in effect (see [`BAR_RADIUS_LIMIT`]).
    if let Some(bar) = sections.get_mut("bar")
        && let Some(radius) = bar.get("radius")
        && let Ok(n) = radius.parse::<i64>()
        && n > BAR_RADIUS_LIMIT
    {
        bar.insert("radius".to_string(), BAR_RADIUS_LIMIT.to_string());
    }
    for (section, keys) in &sections {
        out.push_str(&format!("\n[{}]\n", toml_key(section)));
        for (key, value) in keys {
            out.push_str(&format!("{} = {value}\n", toml_dotted(key)));
        }
    }
    Ok(out)
}

/// Render the foot fragment (`foot.ini`): `[colors-dark]` for a dark look,
/// `[colors-light]` for a light one (foot 1.28 has no `[colors]`), with the
/// look's 16 ANSI hues as `regular0-7` then `bright0-7` and `alpha` from
/// `opacity.terminal`. `None` when the look has no `[terminal]` table: no
/// fragment is ever synthesized from the palette.
///
/// Every hue is re-checked here, not only at parse: a library caller can
/// hand-build a `ResolvedLook` past the gate, and a newline in a hue would
/// otherwise open a new foot section (foot's `shell` key runs commands, so
/// a section break is a code-execution shape, not a color typo).
pub fn render_foot(look: &ResolvedLook) -> Result<Option<String>, GrammarError> {
    let Some(colors) = &look.terminal else {
        return Ok(None);
    };
    if colors.len() != 16 {
        return Err(GrammarError::new(
            "terminal.colors",
            format!("must hold exactly 16 colors, found {}", colors.len()),
        ));
    }
    for (i, color) in colors.iter().enumerate() {
        if !is_hex_color(color) {
            return Err(GrammarError::new(
                format!("terminal.colors[{i}]"),
                "must be exactly #rrggbb",
            ));
        }
    }
    if !look.opacity.terminal.is_finite() || !(0.0..=1.0).contains(&look.opacity.terminal) {
        return Err(GrammarError::new("opacity.terminal", "must be 0.0 to 1.0"));
    }
    let section = if look.polarity == Polarity::Dark {
        "colors-dark"
    } else {
        "colors-light"
    };
    // A hand-built `ResolvedLook` skips the name grammar, so the comment
    // quotes rather than interpolates: a newline in the name stays one line.
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n",
        toml_str(&look.name)
    );
    out.push_str(&format!(
        "\n[{section}]\nalpha={:.2}\n",
        look.opacity.terminal
    ));
    for (i, color) in colors.iter().enumerate() {
        // Foot's hex carries no `#`; the check above makes the slice safe.
        let key = if i < 8 {
            format!("regular{i}")
        } else {
            format!("bright{}", i - 8)
        };
        out.push_str(&format!("{key}={}\n", &color[1..]));
    }
    Ok(Some(out))
}

/// A cache identity for `render.json`, not security: FNV-1a over the source
/// bytes. No hash crate enters the tree for this.
pub fn source_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn json_str(s: &str) -> String {
    toml_str(s)
}

/// The `render.json` beside the fragments: the source, its hash and the
/// resolved inputs. The compositor reads this (not `<state>/look.json`,
/// which is the tool's record of the live choice) to tell that the layer's
/// identity changed.
pub fn render_json(name: &str, hash: &str, inputs: &RenderInputs) -> String {
    let wallpaper = inputs.wallpaper_abs.as_deref().unwrap_or("");
    let font = inputs.ui_font_file.as_deref().unwrap_or("");
    let mut dropped = inputs.bar_drop_sections.clone();
    dropped.sort();
    dropped.dedup();
    let dropped = dropped
        .iter()
        .map(|section| json_str(section))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"name\":{},\"source_hash\":{},\"wallpaper\":{},\"ui_font\":{},\"bar_popup\":{},\"bar_dropped\":[{dropped}]}}",
        json_str(name),
        json_str(hash),
        json_str(wallpaper),
        json_str(font),
        inputs.bar_popup
    )
}

#[cfg(test)]
mod tests;
