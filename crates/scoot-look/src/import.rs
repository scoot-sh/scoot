//! `scoot-look import-base16`: the one base16-to-token mapping.
//!
//! A plain-install source of palettes (design section 5.3): what Stylix
//! gives a Nix user, `import-base16` gives anyone with a scheme file. Pure
//! by contract: bytes in, `look.toml` text out. No Wayland, no fontconfig,
//! no image decoder, no process spawning, nothing that needs Nix.
//!
//! # Input
//!
//! A base16 scheme file or the base24 superset, as YAML, or the JSON form
//! the Nix side serializes Stylix into (what `look-stylix-source` feeds
//! this same function). JSON when the first non-whitespace byte is `{`,
//! otherwise YAML. The YAML read is the flat subset scheme files use:
//! `key: value` mappings with `#` comments, single or double quotes, one
//! nested-map level by indentation (the spec-0.11 `palette:` map, the
//! companion maps below). Tabs, lists and flow collections refuse.
//!
//! Slot sources (design section 5.3):
//!
//! | Token | Slot |
//! |---|---|
//! | `background`, `surface` | `base00` |
//! | `foreground` | `base05` |
//! | `dim` | `base03` |
//! | `accent`, `hover`, `focus` | `base0D` |
//! | `border` | `base03` |
//! | `urgent` | `base08` |
//! | `accent-fg` | `base00` (the look default) |
//! | `selection`, `selection-text` | `base02`, `base05` |
//! | `match` | `base0A` |
//! | `[terminal]` | `base00..base0F` in order, only without `extends` |
//!
//! Slots read with or without a leading `#`, any case; anything else
//! refuses naming the slot. `base10..base17` (base24) must be well-formed
//! when present and are otherwise ignored: the mapping above is the whole
//! palette either way. Slots live in a nested `palette:` map (spec-0.11
//! files) or at the top level (older flat files); when both spell a slot
//! the nested one wins.
//!
//! # Companion data, when given
//!
//! `polarity` (or the scheme's `variant`): `dark` or `light`, else derived
//! from `base00` against `base05` (whichever is darker decides; tied, the
//! `0.18` luminance bar). `fonts`: `sansSerif` (or `sans`) goes to `sans`
//! and to `ui`, `monospace` (or `mono`) to `mono`, `ui` to `ui`. `sizes`:
//! `desktop` to `size`, `popups` to `popup-size`, `applications` to
//! `app-size`, `terminal` to `terminal-size`. `opacity`: `popups` and
//! `terminal`; the bar's is left unset, as today. `cursor`: `name` and
//! `size`. `title`, `description`, `license`: overrides for the generated
//! text. Unknown keys are ignored: scheme files vary, and the mapping only
//! reads what it knows.
//!
//! # Output
//!
//! `look.toml` text for a source look of `Origin::User`: a look the user
//! chose, so a contrast shortfall warns and never refuses (the caller
//! measures it, as `validate` does). With `--extends` the output is a
//! child: palette, companion fonts/sizes/opacity/cursor when given, and
//! nothing else (terminal, shape and wallpaper inherit the base, design
//! section 5.4). Without it the output is complete: title, polarity,
//! fonts, `ring-width`, cursor, a flat `background` wallpaper and the
//! sixteen terminal hues. The generated display text is sanitized to the
//! grammar (never refused: the scheme author's punctuation must not fail
//! an import), and every name, identifier, size and color is checked, so
//! the output always re-parses.

use std::collections::BTreeMap;

use crate::contrast::luminance;
use crate::grammar::{GrammarError, MAX_FILE_BYTES, is_identifier, is_name};

mod parse;

use parse::{Node, parse_json, parse_yaml};

/// Options for [`import_base16`]: `--name` and `--extends` as the CLI
/// spells them, plus the input file's stem as the last-resort name.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Explicit look name. Wins over the scheme's slug.
    pub name: Option<String>,
    /// Base look to extend. When set the output omits terminal, shape
    /// and wallpaper and inherits them.
    pub extends: Option<String>,
    /// The input file's stem, used only when the scheme names no slug.
    pub file_stem: Option<String>,
}

/// Read a base16 or base24 scheme and emit a source look's `look.toml`.
///
/// Refuses (naming the field) on: an oversized or non-UTF-8 file, NUL, bad
/// JSON/YAML, a missing or malformed `base00..base0F` slot, a malformed
/// present `base10..base17`, a bad `polarity`/`variant`, companion values
/// outside the grammar, a name that is no `[a-z0-9-]{1,32}`, or a bad
/// `extends`. Never panics on malformed input: every index is checked.
pub fn import_base16(bytes: &[u8], opts: &ImportOptions) -> Result<String, GrammarError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(GrammarError::new(
            "scheme",
            format!("file larger than {MAX_FILE_BYTES} bytes"),
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| GrammarError::new("scheme", "file is not valid UTF-8"))?;
    if text.chars().any(|c| c == '\0') {
        return Err(GrammarError::new("scheme", "NUL is refused everywhere"));
    }
    let doc = if text.bytes().any(|b| !b.is_ascii_whitespace()) {
        let first = text.bytes().find(|b| !b.is_ascii_whitespace());
        if first == Some(b'{') {
            parse_json(text)?
        } else {
            parse_yaml(text)?
        }
    } else {
        return Err(GrammarError::new("scheme", "empty scheme file"));
    };
    let scheme = Scheme::read(&doc)?;
    let name = resolve_name(&scheme, opts)?;
    if let Some(base) = &opts.extends
        && !is_name(base)
    {
        return Err(GrammarError::new("extends", "must match [a-z0-9-]{1,32}"));
    }
    Ok(emit(&scheme, &name, opts.extends.as_deref()))
}

fn is_hex6(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

struct Scheme {
    slots: BTreeMap<String, String>,
    display_name: Option<String>,
    slug_hint: Option<String>,
    author: Option<String>,
    polarity: Option<String>,
    fonts: BTreeMap<String, String>,
    sizes: BTreeMap<String, f64>,
    opacity: BTreeMap<String, f64>,
    cursor_name: Option<String>,
    cursor_size: Option<i64>,
    title: Option<String>,
    description: Option<String>,
    license: Option<String>,
}

impl Scheme {
    fn get<'a>(doc: &'a BTreeMap<String, Node>, key: &str) -> Option<&'a Node> {
        doc.get(key)
    }

    /// A slot from the nested `palette:` map first, else the top level.
    fn slot(doc: &BTreeMap<String, Node>, slot: &str) -> Option<String> {
        let nested = doc
            .get("palette")
            .and_then(Node::as_map)
            .and_then(|m| m.get(slot))
            .and_then(Node::as_str);
        nested
            .or_else(|| doc.get(slot).and_then(Node::as_str))
            .map(str::trim)
            .map(str::to_string)
    }

    fn read(doc: &BTreeMap<String, Node>) -> Result<Scheme, GrammarError> {
        let mut slots = BTreeMap::new();
        for n in 0..16 {
            let slot = format!("base{n:02X}");
            let raw = Self::slot(doc, &slot).ok_or_else(|| {
                GrammarError::new(&slot, format!("required slot `{slot}` is missing"))
            })?;
            slots.insert(slot.clone(), normalize_hex(&slot, &raw)?);
        }
        // base10..base17 (the base24 superset) must be well-formed
        // when present and are otherwise ignored: the palette mapping
        // above is the whole palette either way.
        for n in 0x10..0x18 {
            let slot = format!("base{n:02X}");
            if let Some(raw) = Self::slot(doc, &slot) {
                normalize_hex(&slot, &raw)?;
            }
        }
        let str_key = |key: &str| {
            Self::get(doc, key)
                .and_then(Node::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let mut fonts = BTreeMap::new();
        if let Some(map) = Self::get(doc, "fonts").and_then(Node::as_map) {
            for (slot, keys) in [
                ("sans", &["sansSerif", "sans"][..]),
                ("mono", &["monospace", "mono"][..]),
                ("ui", &["ui", "sansSerif", "sans"][..]),
            ] {
                for key in keys {
                    if let Some(v) = map.get(*key).and_then(Node::as_str) {
                        let v = v.trim().to_string();
                        if !v.is_empty() {
                            if !is_identifier(&v) {
                                return Err(GrammarError::new(
                                    format!("fonts.{key}"),
                                    "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
                                ));
                            }
                            fonts.insert(slot.to_string(), v);
                            break;
                        }
                    }
                }
            }
        }
        let mut sizes = BTreeMap::new();
        if let Some(map) = Self::get(doc, "sizes").and_then(Node::as_map) {
            for (from, to) in [
                ("desktop", "size"),
                ("popups", "popup-size"),
                ("applications", "app-size"),
                ("terminal", "terminal-size"),
            ] {
                if let Some(node) = map.get(from) {
                    let v = node.as_num().ok_or_else(|| {
                        GrammarError::new(format!("sizes.{from}"), "must be a number")
                    })?;
                    if !(4.0..=192.0).contains(&v) {
                        return Err(GrammarError::new(
                            format!("sizes.{from}"),
                            "must be 4 to 192 points",
                        ));
                    }
                    sizes.insert(to.to_string(), v);
                }
            }
        }
        let mut opacity = BTreeMap::new();
        if let Some(map) = Self::get(doc, "opacity").and_then(Node::as_map) {
            for key in ["popups", "terminal"] {
                if let Some(node) = map.get(key) {
                    let v = node.as_num().ok_or_else(|| {
                        GrammarError::new(format!("opacity.{key}"), "must be a number")
                    })?;
                    if !(0.0..=1.0).contains(&v) {
                        return Err(GrammarError::new(
                            format!("opacity.{key}"),
                            "must be 0.0 to 1.0",
                        ));
                    }
                    opacity.insert(key.to_string(), v);
                }
            }
        }
        let (cursor_name, cursor_size) = match Self::get(doc, "cursor").and_then(Node::as_map) {
            Some(map) => {
                let name = match map.get("name").and_then(Node::as_str) {
                    Some(v) if !v.trim().is_empty() => {
                        let v = v.trim().to_string();
                        if !is_identifier(&v) {
                            return Err(GrammarError::new(
                                "cursor.name",
                                "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit",
                            ));
                        }
                        Some(v)
                    }
                    _ => None,
                };
                let size = match map.get("size") {
                    Some(node) => {
                        let v = node
                            .as_num()
                            .ok_or_else(|| GrammarError::new("cursor.size", "must be a number"))?;
                        if v.fract() != 0.0 || !(4.0..=256.0).contains(&v) {
                            return Err(GrammarError::new("cursor.size", "must be 4 to 256"));
                        }
                        Some(v as i64)
                    }
                    None => None,
                };
                (name, size)
            }
            None => (None, None),
        };
        for (field, value, max) in [
            ("title", str_key("title"), 64),
            ("description", str_key("description"), 200),
            ("license", str_key("license"), 200),
        ] {
            if let Some(v) = value
                && let Some(e) = crate::grammar::check_display_text(field, &v, max)
            {
                return Err(e);
            }
        }
        Ok(Scheme {
            slots,
            display_name: str_key("name").or_else(|| str_key("scheme")),
            slug_hint: str_key("slug"),
            author: str_key("author"),
            polarity: match str_key("polarity").or_else(|| str_key("variant")) {
                Some(p) if p == "dark" || p == "light" => Some(p),
                Some(bad) => {
                    return Err(GrammarError::new(
                        "polarity",
                        format!("`{bad}`: must be `dark` or `light`"),
                    ));
                }
                None => None,
            },
            fonts,
            sizes,
            opacity,
            cursor_name,
            cursor_size,
            title: str_key("title"),
            description: str_key("description"),
            license: str_key("license"),
        })
    }
}

fn normalize_hex(slot: &str, raw: &str) -> Result<String, GrammarError> {
    let hex = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if !is_hex6(hex) {
        return Err(GrammarError::new(
            slot,
            "must be six hex digits, with or without a leading `#`",
        ));
    }
    Ok(format!("#{}", hex.to_ascii_lowercase()))
}

fn resolve_name(scheme: &Scheme, opts: &ImportOptions) -> Result<String, GrammarError> {
    if let Some(name) = &opts.name {
        if !is_name(name) {
            return Err(GrammarError::new("name", "must match [a-z0-9-]{1,32}"));
        }
        return Ok(name.clone());
    }
    for hint in [&scheme.slug_hint, &scheme.display_name, &opts.file_stem] {
        if let Some(hint) = hint
            && !hint.trim().is_empty()
        {
            let slug = slugify(hint);
            if !slug.is_empty() {
                if slug.len() > 32 {
                    return Err(GrammarError::new(
                        "name",
                        format!("derived name `{slug}` is longer than 32 bytes: pass --name"),
                    ));
                }
                return Ok(slug);
            }
        }
    }
    Err(GrammarError::new(
        "name",
        "no scheme name found: pass --name",
    ))
}

/// Lowercase, anything outside `[a-z0-9-]` becomes `-`, squeezed and
/// trimmed. `"Solarized Dark"` becomes `solarized-dark`.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_dash = false;
        } else if c == '-' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Display text the grammar refuses is sanitized, never refused: the
/// scheme author's punctuation must not fail an import. Forbidden
/// characters become `-`, controls become a space, truncated to the
/// byte cap on a char boundary.
fn sanitize_display(s: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    for c in s.chars() {
        if c == '\0' || c.is_control() || c == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&c) {
            out.push(' ');
        } else if matches!(c, '=' | '[' | ']' | '"' | '\\') {
            out.push('-');
        } else {
            out.push(c);
        }
        if out.len() >= max_bytes {
            break;
        }
    }
    while out.len() > max_bytes {
        out.pop();
    }
    out.trim().to_string()
}

fn toml_str(s: &str) -> String {
    // Every string reaching here is grammar-safe (names, identifiers,
    // hex) or sanitized display text: no `"`, no `\`, no controls.
    debug_assert!(!s.contains(['"', '\\']) && !s.chars().any(|c| c.is_control()));
    format!("\"{s}\"")
}

fn toml_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

#[allow(clippy::too_many_lines)]
fn emit(scheme: &Scheme, name: &str, extends: Option<&str>) -> String {
    let s = |slot: &str| scheme.slots[slot].clone();
    // Design section 5.3: the one mapping. `surface` stays `base00`
    // because Stylix's own mako and fuzzel targets draw their cards on
    // `base00`; `selection`/`selection-text`/`match` take the base16
    // selection meanings, as Stylix's fuzzel target draws.
    let background = s("base00");
    let foreground = s("base05");
    let dim = s("base03");
    let accent = s("base0D");
    let urgent = s("base08");
    let title = scheme
        .title
        .clone()
        .unwrap_or_else(|| sanitize_display(scheme.display_name.as_deref().unwrap_or(name), 64));
    let title = sanitize_display(&title, 64);
    let description = scheme.description.clone().unwrap_or_else(|| {
        match (&scheme.display_name, &scheme.author) {
            (Some(n), Some(a)) => format!("Base16 {n} by {a}, imported as a scoot look."),
            (Some(n), None) => format!("Base16 {n}, imported as a scoot look."),
            (None, Some(a)) => format!("Base16 scheme by {a}, imported as a scoot look."),
            (None, None) => "Imported from a base16 scheme.".to_string(),
        }
    });
    let description = sanitize_display(&description, 200);
    let polarity = match scheme.polarity.as_deref() {
        Some("dark") => "dark".to_string(),
        Some("light") => "light".to_string(),
        _ => {
            if luminance(&background) <= luminance(&foreground) {
                "dark".to_string()
            } else {
                "light".to_string()
            }
        }
    };
    let mut out = String::new();
    out.push_str("schema = 1\n");
    out.push_str(&format!("name = {}\n", toml_str(name)));
    out.push_str(&format!("title = {}\n", toml_str(&title)));
    out.push_str(&format!("description = {}\n", toml_str(&description)));
    out.push_str(&format!("polarity = \"{polarity}\"\n"));
    if let Some(base) = extends {
        out.push_str(&format!("extends = {}\n", toml_str(base)));
    }
    if let Some(license) = &scheme.license {
        out.push_str(&format!("license = {}\n", toml_str(license)));
    }
    if let Some(author) = &scheme.author {
        let credit = sanitize_display(
            &format!(
                "Base16 {} by {}.",
                scheme.display_name.as_deref().unwrap_or(name),
                author
            ),
            200,
        );
        out.push_str(&format!("credits = [{}]\n", toml_str(&credit)));
    }
    out.push_str("\n[palette]\n");
    out.push_str(&format!("background = {}\n", toml_str(&background)));
    out.push_str(&format!("foreground = {}\n", toml_str(&foreground)));
    out.push_str(&format!("dim = {}\n", toml_str(&dim)));
    out.push_str(&format!("accent = {}\n", toml_str(&accent)));
    out.push_str(&format!("urgent = {}\n", toml_str(&urgent)));
    out.push_str(&format!("surface = {}\n", toml_str(&background)));
    out.push_str(&format!("hover = {}\n", toml_str(&accent)));
    out.push_str(&format!("focus = {}\n", toml_str(&accent)));
    out.push_str(&format!("border = {}\n", toml_str(&dim)));
    out.push_str(&format!("accent-fg = {}\n", toml_str(&background)));
    out.push_str(&format!("selection = {}\n", toml_str(&s("base02"))));
    out.push_str(&format!("selection-text = {}\n", toml_str(&foreground)));
    out.push_str(&format!("match = {}\n", toml_str(&s("base0A"))));
    if extends.is_none() {
        out.push_str("\n[terminal]\ncolors = [");
        for (i, n) in (0..16).enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&toml_str(&s(&format!("base{n:02X}"))));
        }
        out.push_str("]\n");
    }
    // Fonts: the companion's `sansSerif` feeds `sans` and `ui`
    // (as scootbar.nix does today), `monospace` feeds `mono`.
    let ui = scheme.fonts.get("ui").cloned();
    let sans = scheme.fonts.get("sans").cloned();
    let mono = scheme.fonts.get("mono").cloned();
    let size = scheme.sizes.get("size").copied();
    let has_fonts = ui.is_some() || sans.is_some() || mono.is_some() || size.is_some();
    if extends.is_none() || has_fonts {
        out.push_str("\n[fonts]\n");
        out.push_str(&format!(
            "ui = {}\n",
            toml_str(ui.as_deref().or(sans.as_deref()).unwrap_or("DejaVu Sans"))
        ));
        out.push_str(&format!(
            "sans = {}\n",
            toml_str(sans.as_deref().or(ui.as_deref()).unwrap_or("DejaVu Sans"))
        ));
        out.push_str(&format!(
            "mono = {}\n",
            toml_str(mono.as_deref().unwrap_or("DejaVu Sans Mono"))
        ));
        let size = size.unwrap_or(10.0);
        out.push_str(&format!("size = {}\n", toml_num(size)));
        for key in ["popup-size", "app-size", "terminal-size"] {
            if let Some(v) = scheme.sizes.get(key) {
                out.push_str(&format!("{key} = {}\n", toml_num(*v)));
            }
        }
    }
    if !scheme.opacity.is_empty() {
        out.push_str("\n[opacity]\n");
        for key in ["popups", "terminal"] {
            if let Some(v) = scheme.opacity.get(key) {
                out.push_str(&format!("{key} = {}\n", toml_num(*v)));
            }
        }
    }
    if extends.is_none() {
        out.push_str("\n[shape]\nring-width = 2\n");
        let theme = scheme.cursor_name.as_deref().unwrap_or("Mocu");
        out.push_str(&format!("\n[cursor]\ntheme = {}\n", toml_str(theme)));
        if let Some(size) = scheme.cursor_size {
            out.push_str(&format!("size = {size}\n"));
        }
        out.push_str(&format!(
            "\n[wallpaper]\ncolor = {}\n",
            toml_str(&background)
        ));
    } else if scheme.cursor_name.is_some() || scheme.cursor_size.is_some() {
        out.push_str("\n[cursor]\n");
        if let Some(cursor_name) = &scheme.cursor_name {
            out.push_str(&format!("theme = {}\n", toml_str(cursor_name)));
        }
        if let Some(size) = scheme.cursor_size {
            out.push_str(&format!("size = {size}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
