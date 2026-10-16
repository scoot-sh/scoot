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
    /// Leave the cursor out of the scoot fragment: no `cursor_theme`,
    /// `cursor_size` or `cursor_color` keys. The Nix theme target
    /// `targets.cursor.enable = false` ("leave the cursor alone
    /// entirely") reaches the runtime this way: `scoot look sync`
    /// re-renders without the triple, so the look layer carries no
    /// cursor and the compositor keeps its own. Absent (false) renders
    /// exactly what it always has.
    pub omit_cursor: bool,
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
    // An opted-out cursor (`RenderInputs::omit_cursor`) leaves no cursor
    // keys at all: the layer then carries no cursor and whatever the
    // compositor already has (its own config, its built-in fallback)
    // stands.
    if !inputs.omit_cursor {
        appearance.insert("cursor_theme".to_string(), toml_str(&look.cursor.theme));
        appearance.insert("cursor_size".to_string(), look.cursor.size.to_string());
        if look.cursor.accent != "none" {
            appearance.insert("cursor_color".to_string(), toml_str(&look.cursor.accent));
        }
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

/// The GTK theme name from the look's polarity (Adwaita ships inside
/// GTK itself: no extra theme package, nothing invented). Matches the
/// home-manager profile's `gtkTheme` (`nix/modules/theme-home.nix`), so a
/// look renders the same theme name with and without Nix.
pub fn gtk_theme_name(dark: bool) -> &'static str {
    if dark { "Adwaita-dark" } else { "Adwaita" }
}

/// The Qt style from the look's polarity (the Adwaita-Qt style in the
/// look's variant, capitalized exactly as the plugin registers it).
/// Matches the profile's `qtStyle`.
pub fn qt_style_name(dark: bool) -> &'static str {
    if dark { "Adwaita-Dark" } else { "Adwaita" }
}

/// A `#rrggbb` token as opaque `#AARRGGBB` (what qt6ct color scheme
/// entries spell, lowercase like its shipped schemes).
pub fn qt_argb(color: &str) -> String {
    format!("ff{}", color.trim_start_matches('#').to_lowercase())
}

/// The same token translucent (placeholder text, the stock-scheme shape:
/// `#80ffffff` on dark schemes, `#80000000` on light ones).
pub fn qt_argb_soft(color: &str) -> String {
    format!("80{}", color.trim_start_matches('#').to_lowercase())
}

/// A font description for GTK and dconf (`Family 11`): the family plus
/// the app size, without a trailing `.0`.
pub fn font_desc(family: &str, size: f64) -> String {
    if size.fract() == 0.0 {
        format!("{family} {}", size as i64)
    } else {
        format!("{family} {size}")
    }
}

/// Render the GTK 3 `gtk.css`: the look's selection as direct rules.
/// GTK 3's parser dies on a `:root` block (`Theme parsing error:
/// gtk.css:18:1: Invalid name of pseudo-class`, measured live against
/// 3.24.52), and user `@define-color theme_*` overrides have no visible
/// effect on its embedded Adwaita (three stock roles sampled against the
/// look's tokens), so this file carries neither: just the selection
/// rules, with literal hexes, proven amber live on `gtk3-widget-factory`.
/// New GTK 3 apps take the look's selection; running ones keep theirs.
pub fn render_gtk3_css(look: &ResolvedLook) -> String {
    let p = &look.palette;
    let mut out = format!(
        "/* Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply. */\n",
        toml_str(&look.name)
    );
    out.push_str("entry selection,\n.view selection,\ntextview selection,\nrow:selected {\n");
    out.push_str(&format!("  background-color: {};\n", p.accent));
    out.push_str(&format!("  color: {};\n", p.accent_fg));
    out.push_str("}\n");
    out
}

/// Render the GTK 4 `gtk.css`: the look's accent and surfaces as
/// `@define-color` names plus the `:root` vars libadwaita widgets
/// actually paint with. The `:root` defaults live in libadwaita's own
/// stylesheet at THEME priority, so names alone never reach them; the
/// literal hexes (never `@`-references, which resolve per provider) make
/// the win unconditional. Measured live: libadwaita paints the look's
/// accent and surfaces from this file, while plain GTK 4 widgets keep
/// the stock accent (their theme does not read these names). Semantic
/// colors (destructive, success, warning, error) stay the toolkit
/// defaults: the look names no hues for them.
pub fn render_gtk4_css(look: &ResolvedLook) -> String {
    let p = &look.palette;
    let mut out = format!(
        "/* Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply. */\n",
        toml_str(&look.name)
    );
    for (name, color) in [
        ("accent_bg_color", &p.accent),
        ("accent_fg_color", &p.accent_fg),
        ("accent_color", &p.accent),
        ("window_bg_color", &p.background),
        ("window_fg_color", &p.foreground),
        ("view_bg_color", &p.background),
        ("view_fg_color", &p.foreground),
        ("headerbar_bg_color", &p.background),
        ("headerbar_fg_color", &p.foreground),
        ("headerbar_border_color", &p.dim),
        ("popover_bg_color", &p.background),
        ("popover_fg_color", &p.foreground),
        ("card_bg_color", &p.background),
        ("card_fg_color", &p.foreground),
        ("sidebar_bg_color", &p.background),
        ("sidebar_fg_color", &p.foreground),
    ] {
        out.push_str(&format!("@define-color {name} {color};\n"));
    }
    out.push_str(":root {\n");
    for (name, color) in [
        ("--accent-bg-color", &p.accent),
        ("--accent-fg-color", &p.accent_fg),
        ("--accent-color", &p.accent),
        ("--window-bg-color", &p.background),
        ("--window-fg-color", &p.foreground),
        ("--view-bg-color", &p.background),
        ("--view-fg-color", &p.foreground),
        ("--headerbar-bg-color", &p.background),
        ("--headerbar-fg-color", &p.foreground),
        ("--headerbar-border-color", &p.dim),
        ("--headerbar-backdrop-color", &p.background),
        ("--popover-bg-color", &p.background),
        ("--popover-fg-color", &p.foreground),
        ("--card-bg-color", &p.background),
        ("--card-fg-color", &p.foreground),
        ("--sidebar-bg-color", &p.background),
        ("--sidebar-fg-color", &p.foreground),
        ("--sidebar-backdrop-color", &p.background),
        ("--dialog-bg-color", &p.background),
        ("--dialog-fg-color", &p.foreground),
    ] {
        out.push_str(&format!("  {name}: {color};\n"));
    }
    out.push_str("}\n");
    out
}

/// Render the Qt color scheme (`qt/scoot-look.conf`): the look's palette
/// in qt6ct's own `[ColorScheme]` file shape (pinned qt6ct 0.11
/// `loadColorScheme`: `active_colors`, `inactive_colors`,
/// `disabled_colors`, each 21 entries in `QPalette::ColorRole` order).
/// Moved from the home-manager profile (`nix/modules/theme-home.nix`)
/// unchanged, so a look renders the same scheme with and without Nix.
///
/// Text roles carry the look's foreground, surfaces its background,
/// Highlight and Link its accent with the background on top (the AA
/// pairing the contrast checks measure); inactive mirrors active the way
/// the stock schemes do, and disabled drops the text roles to the look's
/// dim. No `[Fonts]` group: absent keys fall back to the application
/// font, which follows fontconfig.
///
/// One measured limit, not a rendering gap: under the Adwaita style the
/// look's palette never reaches QWidget apps (adwaita-qt's `polish`
/// replaces it with its fixed one, proven live against qtbase 6.11.2,
/// qt6ct 0.11 and adwaita-qt 1.4.2). The scheme still ships: a style that
/// honors `custom_palette` (Fusion does) paints it, and new processes read
/// it either way.
pub fn render_qt_scheme(look: &ResolvedLook) -> String {
    let p = &look.palette;
    let fg = qt_argb(&p.foreground);
    let bg = qt_argb(&p.background);
    let accent = qt_argb(&p.accent);
    let accent_fg = qt_argb(&p.background);
    let dim = qt_argb(&p.dim);
    let soft_fg = qt_argb_soft(&p.foreground);
    let soft_dim = qt_argb_soft(&p.dim);
    // Role order 0-20: WindowText Button Light Midlight Dark Mid Text
    // BrightText ButtonText Base Window Shadow Highlight
    // HighlightedText Link LinkVisited AlternateBase NoRole
    // ToolTipBase ToolTipText PlaceholderText.
    let active = [
        &fg, &bg, &fg, &dim, &bg, &dim, &fg, &accent, &fg, &bg, &bg, &dim, &accent, &accent_fg,
        &accent, &accent, &bg, &fg, &bg, &fg, &soft_fg,
    ];
    let disabled = [
        &dim, &bg, &fg, &dim, &bg, &dim, &dim, &dim, &dim, &bg, &bg, &dim, &dim, &dim, &accent,
        &accent, &bg, &fg, &bg, &dim, &soft_dim,
    ];
    let row = |colors: &[&String]| {
        colors
            .iter()
            .map(|c| format!("#{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let active_row = row(&active);
    let disabled_row = row(&disabled);
    format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n\
         [ColorScheme]\n\
         active_colors={active_row}\n\
         inactive_colors={active_row}\n\
         disabled_colors={disabled_row}\n",
        toml_str(&look.name),
    )
}

/// Render the qt6ct config: the Adwaita Qt style in the look's polarity
/// plus the Adwaita icon theme, with `custom_palette` pointing at the
/// scheme. `scheme_path` is the absolute path the entry carries (on a
/// plain install, through the state link, so a swap needs no rewrite).
/// Moved from the profile's `qtConf`, minus its `~` (qt6ct's own
/// `resolvePath` expands `~`; an absolute path needs no expansion).
/// Without both keys the platform theme keeps its default light palette.
pub fn render_qt6ct_conf(look: &ResolvedLook, scheme_path: &str) -> Result<String, GrammarError> {
    if scheme_path.is_empty()
        || scheme_path.len() > 4096
        || scheme_path.contains('\0')
        || scheme_path.chars().any(|c| c.is_control() || c == '"')
    {
        return Err(GrammarError::new(
            "color_scheme_path",
            "the Qt scheme path must be 1 to 4096 bytes with no control characters or quotes",
        ));
    }
    let dark = look.polarity == Polarity::Dark;
    Ok(format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n\
         [Appearance]\n\
         style={}\n\
         icon_theme={}\n\
         standard_dialogs=default\n\
         custom_palette=true\n\
         color_scheme_path={scheme_path}\n",
        toml_str(&look.name),
        qt_style_name(dark),
        look.icons_theme,
    ))
}

/// Render the GTK `settings.ini` (both versions, same content): the
/// theme, the icon theme, the look's sans, cursor and dark preference.
/// Moved from the profile's `gtkIni`, except the font size: the profile
/// hardcodes 11 pt, while the renderer uses the look's `fonts.app-size`
/// token (default 12), so a look controls its app text on both paths.
/// Newly started apps read it; running GTK 3 apps follow the same values
/// live through dconf instead.
pub fn render_gtk_ini(look: &ResolvedLook) -> String {
    let dark = look.polarity == Polarity::Dark;
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n\
         [Settings]\n",
        toml_str(&look.name)
    );
    for (key, value) in [
        ("gtk-theme-name", gtk_theme_name(dark).to_string()),
        ("gtk-icon-theme-name", look.icons_theme.clone()),
        (
            "gtk-font-name",
            font_desc(&look.fonts.sans, look.fonts.app_size),
        ),
        ("gtk-cursor-theme-name", look.cursor.theme.clone()),
        ("gtk-cursor-theme-size", look.cursor.size.to_string()),
        (
            "gtk-application-prefer-dark-theme",
            if dark { "true" } else { "false" }.to_string(),
        ),
        ("gtk-xft-antialias", "1".to_string()),
        ("gtk-xft-hinting", "1".to_string()),
    ] {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

/// Render the fontconfig default (`10-scoot-look.conf`): the proportional
/// sans for sans-serif (GTK/Qt apps, the greeter) and the terminal face
/// for monospace. Written as `match` rules with strong bindings, not
/// `<alias>` blocks. Family names pass the identifier grammar (no
/// `<>&"'` possible), so no XML escaping is needed on top.
pub fn render_fontconfig(look: &ResolvedLook) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>\n",
    );
    out.push_str(&format!(
        "  <!-- Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply. -->\n",
        toml_str(&look.name)
    ));
    for (generic, family) in [
        ("sans-serif", &look.fonts.sans),
        ("monospace", &look.fonts.mono),
    ] {
        out.push_str("  <match target=\"pattern\">\n");
        out.push_str(&format!(
            "    <test name=\"family\"><string>{generic}</string></test>\n"
        ));
        out.push_str(&format!(
            "    <edit name=\"family\" mode=\"prepend\" binding=\"strong\"><string>{family}</string></edit>\n"
        ));
        out.push_str("  </match>\n");
    }
    out.push_str("</fontconfig>\n");
    out
}

/// Render the foot fragment (`foot.ini`): `[main]` selects the look's
/// polarity as foot's initial theme (`initial-color-theme`: foot starts on
/// `colors-dark` unless it is set to `light`), and both `[colors-dark]`
/// and `[colors-light]` carry the look's 16 ANSI hues as `regular0-7`
/// then `bright0-7`, each with `alpha` from `opacity.terminal` (foot 1.28
/// has no `[colors]`). Both palettes hold the same hues on purpose:
/// whatever theme is in effect when a new terminal starts — the
/// fragment's own `initial-color-theme`, or the user's own `foot.ini`
/// setting when it comes after the include — the look's hues show.
/// `None` when the look has no `[terminal]` table: no fragment is ever
/// synthesized from the palette.
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
    let theme = if look.polarity == Polarity::Dark {
        "dark"
    } else {
        "light"
    };
    // A hand-built `ResolvedLook` skips the name grammar, so the comment
    // quotes rather than interpolates: a newline in the name stays one line.
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n# Both themes carry the look's hues, so new terminals match whatever theme is in effect.\n",
        toml_str(&look.name)
    );
    out.push_str(&format!("\n[main]\ninitial-color-theme={theme}\n"));
    for section in ["colors-dark", "colors-light"] {
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
    }
    Ok(Some(out))
}
/// One `key=value` line for the line-based leaf formats (`fuzzel.ini`,
/// swaylock). Those formats have no string quoting, so the writer is the
/// quoting: a value that would open a new line (or smuggle NUL) is
/// refused, and every other byte renders literally. Keys are the
/// renderer's own fixed spellings, never interpolated, so only values
/// pass through here.
fn kv_line(key: &str, value: &str) -> Result<String, GrammarError> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(GrammarError::new(
            "leaf",
            format!("value for `{key}` must stay on one line"),
        ));
    }
    Ok(format!("{key}={value}\n"))
}

/// A `#rrggbb` token as fuzzel's opaque `rrggbbaa` (its CLI and its
/// config spell colors without the `#`). The grammar guarantees the
/// shape on validated looks; a hand-built look carrying anything else
/// is refused here, so a broken config never reaches the app.
fn fuzzel_color(field: &str, color: &str) -> Result<String, GrammarError> {
    if !is_hex_color(color) {
        return Err(GrammarError::new(
            field,
            "must be exactly #rrggbb to render a leaf config",
        ));
    }
    Ok(format!("{}ff", &color[1..]))
}

/// A `#rrggbb` token as swaylock's `rrggbb` (its parser also takes a
/// leading `#` and an alpha pair, but the looks are opaque hex).
fn swaylock_color(field: &str, color: &str) -> Result<String, GrammarError> {
    if !is_hex_color(color) {
        return Err(GrammarError::new(
            field,
            "must be exactly #rrggbb to render a leaf config",
        ));
    }
    Ok(color[1..].to_string())
}

/// Render the launcher fragment (`fuzzel.ini`): the seven roles the
/// inlined CLI flags carry today (`fuzzel-theme.nix`), as a `[colors]`
/// table. No `[main]` section: the font, line count and every command
/// key (`launch-prefix`, `terminal`) stay the user's own config, exactly
/// what the flags leave alone today.
pub fn render_fuzzel(look: &ResolvedLook) -> Result<String, GrammarError> {
    let p = &look.palette;
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n",
        toml_str(&look.name)
    );
    out.push_str("\n[colors]\n");
    for (key, color) in [
        ("background", &p.background),
        ("text", &p.foreground),
        ("prompt", &p.foreground),
        ("match", &p.r#match),
        ("selection", &p.selection),
        ("selection-text", &p.selection_text),
        ("border", &p.focus),
    ] {
        let rendered = fuzzel_color(key, color)?;
        out.push_str(&kv_line(key, &rendered)?);
    }
    Ok(out)
}

/// Render the locker fragment (`swaylock`): the fourteen `key=value`
/// leaves the store config carries today (`idle-home.nix` `lockTheme`),
/// section-less, in swaylock's spelling.
pub fn render_swaylock(look: &ResolvedLook) -> Result<String, GrammarError> {
    let p = &look.palette;
    let mut out = format!(
        "# Rendered from look {} by scoot-look. Do not edit: re-rendered on every apply.\n",
        toml_str(&look.name)
    );
    out.push('\n');
    for (key, color) in [
        ("color", &p.background),
        ("inside-color", &p.background),
        ("ring-color", &p.focus),
        ("ring-ver-color", &p.accent),
        ("ring-clear-color", &p.hover),
        ("ring-wrong-color", &p.urgent),
        ("inside-ver-color", &p.accent),
        ("inside-clear-color", &p.hover),
        ("inside-wrong-color", &p.urgent),
        ("key-hl-color", &p.accent),
        ("bs-hl-color", &p.accent),
        ("text-color", &p.foreground),
        ("line-color", &p.border),
        ("separator-color", &p.border),
    ] {
        let rendered = swaylock_color(key, color)?;
        out.push_str(&kv_line(key, &rendered)?);
    }
    Ok(out)
}

/// Parse a rendered leaf fragment back: `[section]` headers, `key=value`
/// lines, `#` comments and blank lines. Anything else (a bare word, a
/// second `=`-less line, a broken header) is refused. The section-less
/// pairs of a swaylock fragment read under `""`. Strict by design: the
/// fragments never emit anything else, so a reparse failure names a
/// renderer bug, and `validate` runs this over exactly what `apply`
/// writes.
pub fn parse_kv_config(
    text: &str,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, GrammarError> {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current = String::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') || line.len() < 3 {
                return Err(GrammarError::new(
                    "leaf",
                    format!("line {} is not a section: {line}", index + 1),
                ));
            }
            current = line[1..line.len() - 1].to_string();
            if current.is_empty() || current.contains(['\n', '\r', '\0', '[', ']']) {
                return Err(GrammarError::new(
                    "leaf",
                    format!("line {} is not a section: {line}", index + 1),
                ));
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(GrammarError::new(
                "leaf",
                format!("line {} is not key=value: {line}", index + 1),
            ));
        };
        if key.is_empty() || key.contains(['\n', '\r', '\0']) {
            return Err(GrammarError::new(
                "leaf",
                format!("line {} has a bad key: {line}", index + 1),
            ));
        }
        sections
            .entry(current.clone())
            .or_default()
            .insert(key.to_string(), value.to_string());
    }
    Ok(sections)
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
