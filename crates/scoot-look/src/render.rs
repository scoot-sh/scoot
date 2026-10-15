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

use crate::allowlist::allows;
use crate::grammar::GrammarError;
use crate::resolved::{ResolvedLook, pt_to_px};

/// What the environment resolved: the tool (or the Nix build) fills these
/// in; the pure crate never touches the filesystem, fontconfig or the
/// network to find them.
#[derive(Debug, Clone, Default)]
pub struct RenderInputs {
    /// Absolute path of the wallpaper image. Required when the look names an
    /// image wallpaper; unused for a color wallpaper.
    pub wallpaper_abs: Option<String>,
    /// Resolved UI font file for the bar. When absent the family name is
    /// emitted and the bar resolves it.
    pub ui_font_file: Option<String>,
    /// Whether the target scootbar binary has the `popup` feature
    /// (`crates/scootbar/src/config.rs:396-401`): `popup-radius` is only
    /// emitted then, because the file denies unknown keys.
    pub bar_popup: bool,
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

fn toml_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => toml_str(s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => {
            if f.is_finite() {
                format!("{f:?}")
            } else {
                "0.0".to_string()
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        _ => "\"<unsupported>\"".to_string(),
    }
}

/// Split `[apps.<app>]` overrides into sections by their first dotted
/// segment, refusing any key no allowlist holds. The loader re-checks; the
/// renderer never trusts the tool, not even itself.
fn override_sections(
    app: &str,
    apps: &BTreeMap<String, BTreeMap<String, toml::Value>>,
) -> Result<BTreeMap<String, Vec<(String, String)>>, GrammarError> {
    let mut sections: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    if let Some(table) = apps.get(app) {
        for (key, value) in table {
            if !allows(app, key) {
                return Err(GrammarError::new(
                    format!("apps.{app}[{key}]"),
                    format!("key `{key}` is not in the `{app}` look allowlist"),
                ));
            }
            let (section, rest) = key.split_once('.').unwrap_or(("", key));
            sections
                .entry(section.to_string())
                .or_default()
                .push((rest.to_string(), toml_value(value)));
        }
    }
    Ok(sections)
}

/// Render the scoot fragment (`scoot.toml`): `[appearance]` colors and
/// widths, the cursor triple, `corner_radius`, and `[wallpaper]` as one
/// unit. Never autostart, binds, window rules, `[tty]`, `[renderer]`,
/// `[xwayland]` or `[virtual_input]`.
pub fn render_scoot(look: &ResolvedLook, inputs: &RenderInputs) -> Result<String, GrammarError> {
    let p = &look.palette;
    let mut out = format!(
        "# Rendered from look \"{}\" by scoot-look. Do not edit: re-rendered on every apply.\n",
        look.name
    );
    out.push_str("[appearance]\n");
    out.push_str(&format!("focus_ring_width = {}\n", look.shape.ring_width));
    out.push_str(&format!(
        "focus_ring_inactive_width = {}\n",
        look.shape.ring_inactive_width
    ));
    out.push_str(&format!(
        "focus_ring_active_color = {}\n",
        toml_str(&p.focus)
    ));
    out.push_str(&format!(
        "focus_ring_inactive_color = {}\n",
        toml_str(&p.border)
    ));
    out.push_str(&format!("background_color = {}\n", toml_str(&p.background)));
    out.push_str(&format!("corner_radius = {}\n", look.shape.radius));
    out.push_str(&format!(
        "cursor_theme = {}\n",
        toml_str(&look.cursor.theme)
    ));
    out.push_str(&format!("cursor_size = {}\n", look.cursor.size));
    if look.cursor.accent != "none" {
        out.push_str(&format!(
            "cursor_color = {}\n",
            toml_str(&look.cursor.accent)
        ));
    }
    if let Some(wallpaper) = &look.wallpaper {
        out.push_str("\n[wallpaper]\n");
        if let Some(color) = &wallpaper.color {
            out.push_str(&format!("color = {}\n", toml_str(color)));
        } else {
            let abs = inputs.wallpaper_abs.as_ref().ok_or_else(|| {
                GrammarError::new(
                    "wallpaper.image",
                    "an image wallpaper needs its resolved absolute path",
                )
            })?;
            out.push_str(&format!("image = {}\n", toml_str(abs)));
        }
        out.push_str(&format!(
            "mode = {}\n",
            toml_str(wallpaper.mode.map(|m| m.as_str()).unwrap_or("fill"))
        ));
        out.push_str(&format!(
            "fill = {}\n",
            toml_str(
                &wallpaper
                    .fill
                    .clone()
                    .unwrap_or_else(|| p.background.clone())
            )
        ));
        out.push_str(&format!(
            "transition = {}\n",
            toml_str(wallpaper.transition.map(|t| t.as_str()).unwrap_or("none"))
        ));
    }
    for (section, keys) in override_sections("scoot", &look.apps)? {
        out.push_str(&format!("\n[{section}]\n"));
        for (key, value) in keys {
            out.push_str(&format!("{key} = {value}\n"));
        }
    }
    Ok(out)
}

/// Render the scootbar fragment (`bar.toml`): `[colors]`, the bar's style,
/// and allowlisted per-module visual keys. Never geometry, placement,
/// module lists or actions.
pub fn render_bar(look: &ResolvedLook, inputs: &RenderInputs) -> Result<String, GrammarError> {
    let p = &look.palette;
    let mut out = format!(
        "# Rendered from look \"{}\" by scoot-look. Do not edit: re-rendered on every apply.\n",
        look.name
    );
    out.push_str("[colors]\n");
    for (key, color) in [
        ("background", &p.background),
        ("foreground", &p.foreground),
        ("accent", &p.accent),
        ("hover", &p.hover),
        ("dim", &p.dim),
        ("urgent", &p.urgent),
    ] {
        out.push_str(&format!("{key} = {}\n", toml_str(color)));
    }
    out.push_str("\n[bar]\n");
    out.push_str(&format!("radius = {}\n", look.shape.radius));
    if inputs.bar_popup {
        out.push_str(&format!("popup-radius = {}\n", look.shape.radius));
    }
    out.push_str(&format!("opacity = {:?}\n", look.opacity.bar));
    let font = inputs
        .ui_font_file
        .clone()
        .unwrap_or_else(|| look.fonts.ui.clone());
    out.push_str(&format!("font = {}\n", toml_str(&font)));
    out.push_str(&format!("font-size = {}\n", pt_to_px(look.fonts.size)));
    for (section, keys) in override_sections("scootbar", &look.apps)? {
        if section == "colors" || section == "bar" {
            for (key, value) in keys {
                out.push_str(&format!("{section}.{key} = {value}\n"));
            }
        } else {
            out.push_str(&format!("\n[{section}]\n"));
            for (key, value) in keys {
                out.push_str(&format!("{key} = {value}\n"));
            }
        }
    }
    Ok(out)
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
    format!(
        "{{\"name\":{},\"source_hash\":{},\"wallpaper\":{},\"ui_font\":{},\"bar_popup\":{}}}",
        json_str(name),
        json_str(hash),
        json_str(wallpaper),
        json_str(font),
        inputs.bar_popup
    )
}

#[cfg(test)]
mod tests;
