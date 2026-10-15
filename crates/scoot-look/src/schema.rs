//! The `look.toml` schema v1 (design section 3.2): parse plus pure
//! validation of keys *and* values.
//!
//! Two types carry a look: [`RawLook`] is exactly what one `look.toml` file
//! says (every inheritable token optional, so a child with
//! `extends = "base"` only writes what it overrides), and
//! [`ResolvedLook`](crate::resolved::ResolvedLook) is the same look after
//! `extends` is followed and every default applied.
//!
//! [`parse_look_toml`] refuses anything malformed before anything downstream
//! reads it: the file cap, the schema version, unknown tables and keys, and
//! every value against the [`grammar`](crate::grammar) rules. What it cannot
//! check without the world (does the font resolve, do the wallpaper bytes
//! decode, does the rendered fragment pass the app's own strict parser)
//! belongs to the caller, never here.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::grammar::{
    GrammarError, MAX_FILE_BYTES, check_display_text, check_glyph, check_relative_path,
    check_wallpaper_url, is_cursor_accent, is_hex_color, is_identifier, is_name, is_nix_attr_path,
    is_permitted_look_file, is_sha256_hex,
};

/// The only schema version this crate reads.
pub const SCHEMA_VERSION: u32 = 1;

/// How deep `extends` may chain (section 3.5).
pub const MAX_EXTENDS_DEPTH: u32 = 8;

/// Where a source look came from. The directory records the origin, and the
/// origin decides trust: installed looks (someone else's) can be applied but
/// never extended, and may not take the name of any look of the first kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `looks/` in the repo, shipped with scoot.
    Shipped,
    /// A Nix-declared look (`programs.scoot.looks`): the user's own choice.
    Declared,
    /// `user-looks/`: made with `look new`, `import-base16` or `save`.
    User,
    /// `installed-looks/`: someone else's; applied but never extended.
    Installed,
}

impl Origin {
    /// Contrast shortfalls below the bar refuse shipped and installed looks
    /// and warn for looks the user's own configuration produced (a
    /// synthesized Stylix look, an `import-base16` result, a declared path).
    pub fn contrast_refuses(self) -> bool {
        matches!(self, Origin::Shipped | Origin::Installed)
    }

    /// `license` is required for shipped and installed looks.
    pub fn license_required(self) -> bool {
        matches!(self, Origin::Shipped | Origin::Installed)
    }

    /// Only the first kind may be a base for `extends` or `new --from`.
    pub fn extendable(self) -> bool {
        !matches!(self, Origin::Installed)
    }
}

/// A parsed, validated, unresolved source look: exactly what one `look.toml`
/// file says, before `extends` is followed. Every inheritable token is
/// optional so an extending child writes only its overrides; required-ness
/// is checked at resolution, when the chain is complete.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLook {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub polarity: Option<Polarity>,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub credits: Vec<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub palette: RawPalette,
    #[serde(default)]
    pub terminal: Option<Terminal>,
    #[serde(default)]
    pub fonts: RawFonts,
    #[serde(default)]
    pub opacity: Opacity,
    #[serde(default)]
    pub shape: RawShape,
    #[serde(default)]
    pub cursor: RawCursor,
    #[serde(default)]
    pub wallpaper: Option<Wallpaper>,
    #[serde(default)]
    pub icons: Option<Icons>,
    #[serde(default)]
    pub night: Option<Night>,
    #[serde(default)]
    pub apps: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    Dark,
    Light,
}

impl Polarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Dark => "dark",
            Polarity::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPalette {
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub foreground: Option<String>,
    #[serde(default)]
    pub dim: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub urgent: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub hover: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default, rename = "accent-fg")]
    pub accent_fg: Option<String>,
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default, rename = "selection-text")]
    pub selection_text: Option<String>,
    #[serde(default)]
    pub r#match: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    pub colors: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFonts {
    #[serde(default)]
    pub ui: Option<String>,
    #[serde(default)]
    pub sans: Option<String>,
    #[serde(default)]
    pub mono: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default, rename = "popup-size")]
    pub popup_size: Option<f64>,
    #[serde(default, rename = "app-size")]
    pub app_size: Option<f64>,
    #[serde(default, rename = "terminal-size")]
    pub terminal_size: Option<f64>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub nix: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opacity {
    #[serde(default)]
    pub bar: Option<f64>,
    #[serde(default)]
    pub popups: Option<f64>,
    #[serde(default)]
    pub terminal: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawShape {
    #[serde(default)]
    pub radius: Option<i64>,
    #[serde(default, rename = "ring-width")]
    pub ring_width: Option<i64>,
    #[serde(default, rename = "ring-inactive-width")]
    pub ring_inactive_width: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCursor {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wallpaper {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub mode: Option<WallpaperMode>,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub transition: Option<WallpaperTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperMode {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

impl WallpaperMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WallpaperMode::Fill => "fill",
            WallpaperMode::Fit => "fit",
            WallpaperMode::Stretch => "stretch",
            WallpaperMode::Center => "center",
            WallpaperMode::Tile => "tile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperTransition {
    None,
    Fade,
    Wipe,
    Grow,
}

impl WallpaperTransition {
    pub fn as_str(self) -> &'static str {
        match self {
            WallpaperTransition::None => "none",
            WallpaperTransition::Fade => "fade",
            WallpaperTransition::Wipe => "wipe",
            WallpaperTransition::Grow => "grow",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icons {
    pub theme: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Night {
    pub temperature: i64,
}

/// Parse and purely validate one `look.toml` file.
///
/// Refuses: an oversized file, malformed TOML, an unknown schema, unknown
/// tables or keys, and any value that breaks the section 3.5 grammar.
/// Everything it checks is in the bytes; required-ness of inherited tokens
/// is checked at resolution, and origin-dependent rules (the `license`
/// requirement, contrast refusal vs warning, installed never extended) are
/// the caller's, via [`validate_for_origin`].
pub fn parse_look_toml(bytes: &[u8]) -> Result<RawLook, Vec<GrammarError>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(vec![GrammarError::new(
            "look.toml",
            format!("file larger than {MAX_FILE_BYTES} bytes"),
        )]);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| vec![GrammarError::new("look.toml", "file is not valid UTF-8")])?;
    if text.chars().any(|c| c == '\0') {
        return Err(vec![GrammarError::new(
            "look.toml",
            "NUL is refused everywhere",
        )]);
    }
    let raw: RawLook =
        toml::from_str(text).map_err(|e| vec![GrammarError::new("look.toml", e.to_string())])?;
    let mut errors = Vec::new();
    validate_raw(&raw, &mut errors);
    if errors.is_empty() {
        Ok(raw)
    } else {
        Err(errors)
    }
}

fn validate_raw(raw: &RawLook, errors: &mut Vec<GrammarError>) {
    if raw.schema != SCHEMA_VERSION {
        errors.push(GrammarError::new(
            "schema",
            format!(
                "unknown schema {}; this crate reads schema {SCHEMA_VERSION}",
                raw.schema
            ),
        ));
    }
    if !is_name(&raw.name) {
        errors.push(GrammarError::new("name", "must match [a-z0-9-]{1,32}"));
    }
    if let Some(title) = &raw.title
        && let Some(e) = check_display_text("title", title, 64)
    {
        errors.push(e);
    }
    if let Some(description) = &raw.description
        && let Some(e) = check_display_text("description", description, 200)
    {
        errors.push(e);
    }
    if let Some(base) = &raw.extends
        && !is_name(base)
    {
        errors.push(GrammarError::new("extends", "must match [a-z0-9-]{1,32}"));
    }
    if raw.extends.as_deref() == Some(raw.name.as_str()) {
        errors.push(GrammarError::new("extends", "a look cannot extend itself"));
    }
    if let Some(license) = &raw.license
        && let Some(e) = check_display_text("license", license, 200)
    {
        errors.push(e);
    }
    for (i, credit) in raw.credits.iter().enumerate() {
        if let Some(e) = check_display_text(&format!("credits[{i}]"), credit, 200) {
            errors.push(e);
        }
    }
    if let Some(preview) = &raw.preview
        && let Some(e) = check_relative_path("preview", preview)
    {
        errors.push(e);
    }

    validate_palette(&raw.palette, errors);
    if let Some(terminal) = &raw.terminal {
        if terminal.colors.len() != 16 {
            errors.push(GrammarError::new(
                "terminal.colors",
                format!(
                    "must hold exactly 16 colors, found {}",
                    terminal.colors.len()
                ),
            ));
        }
        for (i, color) in terminal.colors.iter().enumerate() {
            if !is_hex_color(color) {
                errors.push(GrammarError::new(
                    format!("terminal.colors[{i}]"),
                    "must be exactly #rrggbb",
                ));
            }
        }
    }
    validate_fonts(&raw.fonts, errors);
    validate_opacity(&raw.opacity, errors);
    validate_shape(&raw.shape, errors);
    validate_cursor(&raw.cursor, errors);
    if let Some(wallpaper) = &raw.wallpaper {
        validate_wallpaper(wallpaper, errors);
    }
    if let Some(icons) = &raw.icons
        && !is_identifier(&icons.theme)
    {
        errors.push(GrammarError::new(
            "icons.theme",
            "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
        ));
    }
    if let Some(night) = &raw.night
        && !(1000..=10000).contains(&night.temperature)
    {
        errors.push(GrammarError::new(
            "night.temperature",
            "must be 1000 to 10000",
        ));
    }
    validate_apps(&raw.apps, errors);
}

fn validate_palette(p: &RawPalette, errors: &mut Vec<GrammarError>) {
    for (field, value) in [
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
        if let Some(v) = value
            && !is_hex_color(v)
        {
            errors.push(GrammarError::new(field, "must be exactly #rrggbb"));
        }
    }
}

fn validate_fonts(fonts: &RawFonts, errors: &mut Vec<GrammarError>) {
    for (field, value) in [
        ("fonts.ui", &fonts.ui),
        ("fonts.sans", &fonts.sans),
        ("fonts.mono", &fonts.mono),
    ] {
        if let Some(v) = value
            && !is_identifier(v)
        {
            errors.push(GrammarError::new(
                field,
                "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
            ));
        }
    }
    for (field, size) in [
        ("fonts.size", &fonts.size),
        ("fonts.popup-size", &fonts.popup_size),
        ("fonts.app-size", &fonts.app_size),
        ("fonts.terminal-size", &fonts.terminal_size),
    ] {
        if let Some(pt) = size
            && !(4.0..=192.0).contains(pt)
        {
            errors.push(GrammarError::new(field, "must be 4 to 192 points"));
        }
    }
    for (slot, path) in &fonts.files {
        if !matches!(slot.as_str(), "ui" | "sans" | "mono") {
            errors.push(GrammarError::new(
                format!("fonts.files.{slot}"),
                "carrier slot must be ui, sans or mono",
            ));
        }
        if let Some(e) = check_relative_path(&format!("fonts.files.{slot}"), path) {
            errors.push(e);
        } else if !is_permitted_look_file(path.rsplit('/').next().unwrap_or(path)) {
            errors.push(GrammarError::new(
                format!("fonts.files.{slot}"),
                "bundled fonts must be .ttf or .otf",
            ));
        }
    }
    for (slot, attr) in &fonts.nix {
        if !matches!(slot.as_str(), "ui" | "sans" | "mono") {
            errors.push(GrammarError::new(
                format!("fonts.nix.{slot}"),
                "carrier slot must be ui, sans or mono",
            ));
        }
        if !is_nix_attr_path(attr) {
            errors.push(GrammarError::new(
                format!("fonts.nix.{slot}"),
                "must be a Nix attribute path of one to eight [A-Za-z_][A-Za-z0-9_'-]{0,63} segments",
            ));
        }
    }
}

fn validate_opacity(opacity: &Opacity, errors: &mut Vec<GrammarError>) {
    for (field, value) in [
        ("opacity.bar", &opacity.bar),
        ("opacity.popups", &opacity.popups),
        ("opacity.terminal", &opacity.terminal),
    ] {
        if let Some(v) = value
            && !(0.0..=1.0).contains(v)
        {
            errors.push(GrammarError::new(field, "must be 0.0 to 1.0"));
        }
    }
}

fn validate_shape(shape: &RawShape, errors: &mut Vec<GrammarError>) {
    if let Some(radius) = shape.radius
        && !(0..=64).contains(&radius)
    {
        errors.push(GrammarError::new("shape.radius", "must be 0 to 64"));
    }
    for (field, width) in [
        ("shape.ring-width", &shape.ring_width),
        ("shape.ring-inactive-width", &shape.ring_inactive_width),
    ] {
        if let Some(w) = width
            && !(0..=64).contains(w)
        {
            errors.push(GrammarError::new(field, "must be 0 to 64"));
        }
    }
}

fn validate_cursor(cursor: &RawCursor, errors: &mut Vec<GrammarError>) {
    if let Some(theme) = &cursor.theme
        && !is_identifier(theme)
    {
        errors.push(GrammarError::new(
            "cursor.theme",
            "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
        ));
    }
    // `Appearance::MIN_CURSOR_SIZE..=MAX_CURSOR_SIZE`
    // (`crates/scoot/src/compositor/decorations.rs:339,360`).
    if let Some(size) = cursor.size
        && !(4..=256).contains(&size)
    {
        errors.push(GrammarError::new("cursor.size", "must be 4 to 256"));
    }
    if let Some(accent) = &cursor.accent
        && !is_cursor_accent(accent)
    {
        errors.push(GrammarError::new(
            "cursor.accent",
            "must be exactly #rrggbb or \"none\"",
        ));
    }
    if let Some(body) = &cursor.body
        && !is_hex_color(body)
    {
        errors.push(GrammarError::new("cursor.body", "must be exactly #rrggbb"));
    }
}

fn validate_wallpaper(wallpaper: &Wallpaper, errors: &mut Vec<GrammarError>) {
    match (&wallpaper.image, &wallpaper.color) {
        (Some(_), Some(_)) => {
            errors.push(GrammarError::new(
                "wallpaper",
                "image and color are mutually exclusive: set one, never both",
            ));
        }
        (None, None) => {
            errors.push(GrammarError::new(
                "wallpaper",
                "one of image or color is required",
            ));
        }
        _ => {}
    }
    if let Some(image) = &wallpaper.image {
        if image.starts_with("https://") {
            if let Some(e) = check_wallpaper_url("wallpaper.image", image) {
                errors.push(e);
            }
            match &wallpaper.sha256 {
                Some(pin) if is_sha256_hex(pin) => {}
                _ => errors.push(GrammarError::new(
                    "wallpaper.sha256",
                    "a URL wallpaper requires a sha256 pin of exactly 64 hex digits",
                )),
            }
        } else if image.contains("://") {
            errors.push(GrammarError::new(
                "wallpaper.image",
                "URL wallpapers must use https://; anything else must be a path inside the look directory",
            ));
        } else if let Some(e) = check_relative_path("wallpaper.image", image) {
            errors.push(e);
        } else if !is_permitted_look_file(image.rsplit('/').next().unwrap_or(image)) {
            errors.push(GrammarError::new(
                "wallpaper.image",
                "file wallpapers must be .png, .jpg, .jpeg, .webp or .gif",
            ));
        }
        if wallpaper.sha256.is_some() && !image.starts_with("https://") {
            errors.push(GrammarError::new(
                "wallpaper.sha256",
                "sha256 pins only accompany URL wallpapers",
            ));
        }
    }
    if let Some(color) = &wallpaper.color
        && !is_hex_color(color)
    {
        errors.push(GrammarError::new(
            "wallpaper.color",
            "must be exactly #rrggbb",
        ));
    }
    if let Some(fill) = &wallpaper.fill
        && !is_hex_color(fill)
    {
        errors.push(GrammarError::new(
            "wallpaper.fill",
            "must be exactly #rrggbb",
        ));
    }
}

fn validate_apps(
    apps: &BTreeMap<String, BTreeMap<String, toml::Value>>,
    errors: &mut Vec<GrammarError>,
) {
    for (app, table) in apps {
        if !is_name(app) {
            errors.push(GrammarError::new(
                format!("apps.{app}"),
                "app name must match [a-z0-9-]{1,32}",
            ));
            continue;
        }
        for (key, value) in table {
            let field = format!("apps.{app}[{key}]");
            if key.is_empty() || key.len() > 128 {
                errors.push(GrammarError::new(&field, "key must be 1 to 128 bytes"));
                continue;
            }
            match value {
                toml::Value::String(s) => {
                    if s.contains('\0') || s.chars().any(|c| c.is_control()) {
                        errors.push(GrammarError::new(
                            &field,
                            "must not contain control characters",
                        ));
                    } else if s.len() > 512 {
                        errors.push(GrammarError::new(&field, "must be at most 512 bytes"));
                    } else if (key.ends_with(".glyph") || key.ends_with("-glyph"))
                        && let Some(e) = check_glyph(&field, s)
                    {
                        errors.push(e);
                    }
                }
                toml::Value::Integer(_) | toml::Value::Float(_) | toml::Value::Boolean(_) => {}
                _ => errors.push(GrammarError::new(
                    &field,
                    "per-app overrides must be strings, integers, floats or booleans",
                )),
            }
        }
    }
}

/// Origin-dependent rules: the `license` requirement for shipped and
/// installed looks. Contrast refusal-vs-warning and the installed-never-
/// extended rule live with their callers ([`contrast`](crate::contrast),
/// [`resolve`](crate::resolve)), beside the data they need.
pub fn validate_for_origin(raw: &RawLook, origin: Origin) -> Vec<GrammarError> {
    let mut errors = Vec::new();
    if origin.license_required() {
        match &raw.license {
            Some(license) if !license.is_empty() => {}
            _ => errors.push(GrammarError::new(
                "license",
                "shipped and installed looks require a license",
            )),
        }
    }
    errors
}

#[cfg(test)]
pub(crate) mod tests;
