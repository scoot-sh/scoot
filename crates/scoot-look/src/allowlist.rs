//! The per-app key allowlists, in one place (design section 3.4).
//!
//! A look may set colors, geometry, fonts, opacity, the wallpaper and the
//! cursor. It may never set a command, a bind, an autostart entry, a module
//! list, an `exec`/`button`/`push` table, an `on-*` action, `[wallpaper]
//! command`, `[tty]`, `[renderer]`, `[xwayland]` or `[virtual_input]`, nor
//! bar `edge`, `height`, `margin`, `layer`, `exclusive` or `tooltip-delay`.
//! `validate` refuses a non-allowlisted key by name, and each app's
//! look-layer loader refuses it again: the loader never trusts the tool.
//!
//! The allowlist excludes every restart-only key
//! (`crates/scoot/src/compositor/config.rs` refuses `[tty]`, `[renderer]`,
//! `[xwayland]` and `[virtual_input]` by name at reload), so a swap never
//! produces a "takes effect on restart" refusal. It also excludes every key
//! of scootbar's `Bar` that decides whether the bar destroys and recreates
//! its layer surface (`edge`, `height`, `margin`, `layer`, `exclusive`:
//! `crates/scootbar/src/bar.rs:185-197`), so a look never recreates the
//! bar's surface.

use crate::grammar::{
    GrammarError, check_glyph, check_relative_path, is_hex_color, is_identifier, is_image_file_name,
};

/// Dotted keys a look may set in scoot's fragment.
pub const SCOOT_ALLOWLIST: &[&str] = &[
    "appearance.focus_ring_width",
    "appearance.focus_ring_inactive_width",
    "appearance.focus_ring_active_color",
    "appearance.focus_ring_inactive_color",
    "appearance.background_color",
    "appearance.corner_radius",
    "appearance.cursor_size",
    "appearance.cursor_color",
    "appearance.cursor_theme",
    "wallpaper.image",
    "wallpaper.color",
    "wallpaper.mode",
    "wallpaper.fill",
    "wallpaper.transition",
];

/// Dotted keys a look may set in scootbar's fragment: the six color roles,
/// the bar's style (never its geometry or placement), the workspaces
/// module's visual keys, and one glyph per icon key that takes a single
/// string. `workspaces.active-color` / `inactive-color` are the module's
/// own override knobs (`crates/scootbar/src/config.rs:512-518`);
/// `pill-shape`, `pill-radius`, `pill-inset` and `disc` are its pill knobs
/// (the shape-compat refusals stay the loader's). Each `*.icon` key takes
/// exactly one glyph in the app; the array-valued icon levels
/// (`brightness.icon`, `battery.icon`, `network.icon-wifi`) stay user-only
/// in v1: they are not glyph strings. The bar's `font` and
/// `fallback-fonts` are file paths resolved by the tool
/// (`--ui-font-file`), never overrides an author writes (round 2
/// finding 3), so they are absent here; the loader still accepts the
/// renderer's own `bar.font` as a fragment key.
pub const SCOOTBAR_ALLOWLIST: &[&str] = &[
    "colors.background",
    "colors.foreground",
    "colors.accent",
    "colors.hover",
    "colors.dim",
    "colors.urgent",
    "bar.radius",
    "bar.popup-radius",
    "bar.opacity",
    "bar.font-size",
    "bar.padding",
    "bar.spacing",
    "bar.separator",
    "workspaces.active-color",
    "workspaces.inactive-color",
    "workspaces.item-gap",
    "workspaces.display",
    "workspaces.pill-shape",
    "workspaces.pill-radius",
    "workspaces.pill-inset",
    "workspaces.disc",
    "window-title.icon",
    "clock.icon",
    "volume.icon",
    "microphone.icon",
    "network.icon",
    "network.icon-ethernet",
    "network.icon-vpn",
    "network.icon-offline",
    "media.icon",
    "media.icon-playing",
    "media.icon-paused",
    "bluetooth.icon",
    "bluetooth.icon-off",
    "bluetooth.icon-on",
    "bluetooth.icon-connected",
    "power.icon",
    "power.icon-lock",
    "power.icon-logout",
    "power.icon-suspend",
    "power.icon-reboot",
    "power.icon-poweroff",
    "battery.icon-charging",
    "battery.icon-full",
];

/// Dotted keys a look may set in the shell apps' fragments (design section
/// 9): the thirteen color roles, the face, its size, opacity and radius.
pub const SHELL_APP_ALLOWLIST: &[&str] = &[
    "colors.background",
    "colors.foreground",
    "colors.dim",
    "colors.accent",
    "colors.urgent",
    "colors.surface",
    "colors.hover",
    "colors.focus",
    "colors.border",
    "colors.accent-fg",
    "colors.selection",
    "colors.selection-text",
    "colors.match",
    "font",
    "font-size",
    "opacity",
    "radius",
];

/// Whether `key` (an `[apps.<app>]` dotted key) is allowlisted for `app`.
/// Unknown apps are refused: only the apps below read look layers.
pub fn allows(app: &str, key: &str) -> bool {
    if key.is_empty() || key.len() > 128 || !key.is_ascii() {
        return false;
    }
    match app {
        "scoot" => SCOOT_ALLOWLIST.contains(&key) || is_wallpaper_override_key(key),
        "scootbar" => SCOOTBAR_ALLOWLIST.contains(&key),
        "scootnotify" | "scootlaunch" | "scootgreet" => SHELL_APP_ALLOWLIST.contains(&key),
        _ => false,
    }
}

/// A `wallpaper.output.*` / `wallpaper.workspace.*` override key with a
/// strict suffix: output and workspace names are 1 to 64 ASCII letters,
/// digits, spaces and `_.+-`, and the leaf is one of the four
/// non-image look-level wallpaper leaves. The old prefix match accepted
/// any suffix, so a quoted key carrying a newline, `=`, `[` or a quote
/// sailed through and the renderer wrote it raw: a real `[autostart]`
/// table out.
///
/// `image` is not an override leaf (round 2 finding 4): only the
/// top-level `wallpaper.image`, resolved to an absolute path through
/// [`RenderInputs`](crate::render::RenderInputs) `wallpaper_abs`, names
/// an image. An override would render a relative path with nothing to
/// resolve it against, silently yielding no wallpaper, so it refuses.
/// Per-output and per-workspace images belong in the user's own file.
///
/// Only one `.workspace.` layer parses: a second one under a
/// `wallpaper.workspace.` prefix (`wallpaper.workspace.a.workspace.b.*`)
/// is refused (round 2 finding 6). It used to pass and render an
/// unknown key, which discards scoot's whole `[wallpaper]` section.
pub fn is_wallpaper_override_key(key: &str) -> bool {
    /// Wallpaper leaves a look override may set per output or workspace:
    /// `color`, `mode`, `fill` and `transition`. `sha256` pins cannot be
    /// expressed per override, so URL wallpapers are refused there;
    /// `filter` and the transition tunables stay render-time only.
    const LEAVES: &[&str] = &["color", "mode", "fill", "transition"];
    if let Some(rest) = key.strip_prefix("wallpaper.workspace.") {
        // No second `.workspace.` layer here: the output branch below
        // is the only shape with two name segments.
        if rest.contains(".workspace.") {
            return false;
        }
        let Some((name, leaf)) = rest.rsplit_once('.') else {
            return false;
        };
        // A dotted workspace name would split wrongly above; those names
        // are not expressible as overrides, by design.
        if name.contains('.') || !is_output_name(name) {
            return false;
        }
        return LEAVES.contains(&leaf);
    }
    let rest = match key.strip_prefix("wallpaper.output.") {
        Some(rest) => rest,
        None => return false,
    };
    // `wallpaper.output.<OUT>.workspace.<WS>.<LEAF>`: peel one optional
    // `.workspace.<WS>` layer so per-workspace-on-an-output overrides work.
    let rest = match rest.split_once(".workspace.") {
        Some((output, tail)) => {
            if !is_output_name(output) || tail.contains(".workspace.") {
                return false;
            }
            tail
        }
        None => rest,
    };
    let Some((name, leaf)) = rest.rsplit_once('.') else {
        return false;
    };
    // A dotted output name would split wrongly above; those names are not
    // expressible as overrides, by design.
    if name.contains('.') || !is_output_name(name) {
        return false;
    }
    LEAVES.contains(&leaf)
}

/// One output or workspace name inside a wallpaper override key.
fn is_output_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b.iter().all(|c| {
            c.is_ascii_alphanumeric()
                || *c == b' '
                || *c == b'_'
                || *c == b'.'
                || *c == b'+'
                || *c == b'-'
        })
        && name != "."
        && name != ".."
}

/// The value kind one allowlisted `[apps.<app>]` key takes. Bounds mirror
/// the app's own parser, so a look override the validator accepts is one
/// the loader accepts: scoot's `Appearance::clamped` and `MIN_`/`MAX_CURSOR_SIZE`
/// (`crates/scoot/src/compositor/decorations.rs:339,360,408-468`), the
/// wallpaper leaves of `crates/scoot/src/compositor/wallpaper/section.rs:88-122`,
/// and scootbar's `MAX_RADIUS`/`MAX_FONT_SIZE`/`MAX_GAP`/`MAX_FALLBACKS`
/// (`crates/scootbar/src/config.rs`, `bar.rs:MAX_HEIGHT`, `text.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Exactly `#rrggbb`.
    Color,
    /// A TOML integer in `min..=max`.
    IntRange { min: i64, max: i64 },
    /// `0.0..=1.0`: a float, or an integer `0` or `1`, like the bar's own
    /// `parse_opacity`.
    Opacity,
    /// One of the named strings.
    Enum(&'static [&'static str]),
    /// A font family, cursor or icon theme name: the identifier grammar.
    Identifier,
    /// A module glyph: the glyph grammar.
    Glyph,
    /// A wallpaper file inside the look dir: a relative path with a
    /// permitted image suffix, never a URL (`sha256` pins cannot be
    /// expressed per override).
    ImagePath,
    /// At most two identifiers (the bar's `MAX_FALLBACKS`).
    FontList,
    /// A TOML boolean. Only `workspaces.disc` takes one today; the kind
    /// exists so adding one cannot fall back to an unchecked scalar.
    Bool,
}

/// The value kind for an allowlisted key, or `None` when the key is not
/// allowlisted for the app.
pub fn value_kind(app: &str, key: &str) -> Option<ValueKind> {
    match app {
        "scoot" => match key {
            "appearance.focus_ring_width" | "appearance.focus_ring_inactive_width" => {
                Some(ValueKind::IntRange { min: 0, max: 64 })
            }
            "appearance.focus_ring_active_color"
            | "appearance.focus_ring_inactive_color"
            | "appearance.background_color"
            | "appearance.cursor_color" => Some(ValueKind::Color),
            "appearance.corner_radius" => Some(ValueKind::IntRange { min: 0, max: 64 }),
            "appearance.cursor_size" => Some(ValueKind::IntRange { min: 4, max: 256 }),
            "appearance.cursor_theme" => Some(ValueKind::Identifier),
            "wallpaper.image" => Some(ValueKind::ImagePath),
            "wallpaper.color" | "wallpaper.fill" => Some(ValueKind::Color),
            "wallpaper.mode" => Some(ValueKind::Enum(&[
                "fill", "fit", "stretch", "center", "tile",
            ])),
            "wallpaper.transition" => Some(ValueKind::Enum(&["none", "fade", "wipe", "grow"])),
            _ if is_wallpaper_override_key(key) => match key.rsplit('.').next() {
                Some("color") | Some("fill") => Some(ValueKind::Color),
                Some("mode") => Some(ValueKind::Enum(&[
                    "fill", "fit", "stretch", "center", "tile",
                ])),
                Some("transition") => Some(ValueKind::Enum(&["none", "fade", "wipe", "grow"])),
                _ => None,
            },
            _ => None,
        },
        "scootbar" => match key {
            "colors.background" | "colors.foreground" | "colors.accent" | "colors.hover"
            | "colors.dim" | "colors.urgent" => Some(ValueKind::Color),
            "bar.radius" | "bar.popup-radius" => Some(ValueKind::IntRange { min: 0, max: 512 }),
            "bar.opacity" => Some(ValueKind::Opacity),
            "bar.font-size" => Some(ValueKind::IntRange { min: 1, max: 256 }),
            "bar.padding" | "bar.spacing" | "bar.separator" => {
                Some(ValueKind::IntRange { min: 0, max: 1024 })
            }
            "workspaces.active-color" | "workspaces.inactive-color" => Some(ValueKind::Color),
            "workspaces.item-gap" => Some(ValueKind::IntRange { min: 0, max: 1024 }),
            "workspaces.display" => Some(ValueKind::Enum(&["numbers", "dots"])),
            "workspaces.pill-shape" => Some(ValueKind::Enum(&["rect", "pill", "circle", "square"])),
            "workspaces.pill-radius" | "workspaces.pill-inset" => {
                Some(ValueKind::IntRange { min: 0, max: 1024 })
            }
            "workspaces.disc" => Some(ValueKind::Bool),
            "window-title.icon"
            | "clock.icon"
            | "volume.icon"
            | "microphone.icon"
            | "network.icon"
            | "network.icon-ethernet"
            | "network.icon-vpn"
            | "network.icon-offline"
            | "media.icon"
            | "media.icon-playing"
            | "media.icon-paused"
            | "bluetooth.icon"
            | "bluetooth.icon-off"
            | "bluetooth.icon-on"
            | "bluetooth.icon-connected"
            | "power.icon"
            | "power.icon-lock"
            | "power.icon-logout"
            | "power.icon-suspend"
            | "power.icon-reboot"
            | "power.icon-poweroff"
            | "battery.icon-charging"
            | "battery.icon-full" => Some(ValueKind::Glyph),
            _ => None,
        },
        "scootnotify" | "scootlaunch" | "scootgreet" => match key {
            "colors.background"
            | "colors.foreground"
            | "colors.dim"
            | "colors.accent"
            | "colors.urgent"
            | "colors.surface"
            | "colors.hover"
            | "colors.focus"
            | "colors.border"
            | "colors.accent-fg"
            | "colors.selection"
            | "colors.selection-text"
            | "colors.match" => Some(ValueKind::Color),
            "font" => Some(ValueKind::Identifier),
            "font-size" => Some(ValueKind::IntRange { min: 1, max: 256 }),
            "opacity" => Some(ValueKind::Opacity),
            "radius" => Some(ValueKind::IntRange { min: 0, max: 64 }),
            _ => None,
        },
        _ => None,
    }
}

/// Check one `[apps.<app>]` override value against its key's kind: the
/// allowlist by name, then the value by kind. Every key the allowlist holds
/// has a kind, so there is no key whose value gets only the generic string
/// backstops. The `*.icon` keys are glyph-checked: each takes one string,
/// and the array-valued icon levels are not allowlisted at all.
pub fn check_app_value(app: &str, key: &str, value: &toml::Value) -> Option<GrammarError> {
    let field = format!("apps.{app}[{key}]");
    let refused = |reason: String| Some(GrammarError::new(field.clone(), reason));
    let kind = match value_kind(app, key) {
        Some(kind) => kind,
        None => {
            return refused(format!("key `{key}` is not in the `{app}` look allowlist"));
        }
    };
    match (kind, value) {
        (ValueKind::Color, toml::Value::String(s)) => {
            (!is_hex_color(s)).then(|| refused("must be exactly #rrggbb".to_string()))?
        }
        (ValueKind::IntRange { min, max }, toml::Value::Integer(i)) => {
            (!(min..=max).contains(i)).then(|| {
                refused(format!("must be a whole number from {min} to {max}"))
            })?
        }
        (ValueKind::Opacity, toml::Value::Float(f)) => {
            (!f.is_finite() || !(0.0..=1.0).contains(f)).then(|| {
                refused("must be a number from 0 (transparent) to 1 (opaque)".to_string())
            })?
        }
        (ValueKind::Opacity, toml::Value::Integer(i)) => {
            (*i != 0 && *i != 1).then(|| {
                refused("must be a number from 0 (transparent) to 1 (opaque)".to_string())
            })?
        }
        (ValueKind::Enum(options), toml::Value::String(s)) => {
            (!options.contains(&s.as_str())).then(|| {
                refused(format!("must be one of {}", options.join(", ")))
            })?
        }
        (ValueKind::Identifier, toml::Value::String(s)) => {
            (!is_identifier(s)).then(|| {
                refused(
                    "must be an identifier: ASCII letters, digits, space and -_.+, 1 to 64 bytes, starting with a letter or digit"
                        .to_string(),
                )
            })?
        }
        (ValueKind::Glyph, toml::Value::String(s)) => check_glyph(&field, s),
        (ValueKind::ImagePath, toml::Value::String(s)) => {
            if s.contains("://") {
                return refused(
                    "URL wallpapers cannot be overridden per app: a sha256 pin cannot be expressed here"
                        .to_string(),
                );
            }
            if let Some(e) = check_relative_path(&field, s) {
                return Some(e);
            }
            let base = s.rsplit('/').next().unwrap_or(s);
            if !is_image_file_name(base) {
                return refused(
                    "file wallpapers must be .png, .jpg, .jpeg, .webp or .gif".to_string(),
                );
            }
            None
        }
        (ValueKind::FontList, toml::Value::Array(items)) => {
            if items.len() > 2 {
                return refused("takes at most 2 font files".to_string());
            }
            for item in items {
                match item {
                    toml::Value::String(s) if is_identifier(s) => {}
                    _ => {
                        return refused(
                            "must be an array of up to 2 identifier strings".to_string()
                        );
                    }
                }
            }
            None
        }
        (ValueKind::Bool, toml::Value::Boolean(_)) => None,
        _ => refused(match kind {
            ValueKind::Color => "must be exactly #rrggbb".to_string(),
            ValueKind::IntRange { min, max } => {
                format!("must be a whole number from {min} to {max}")
            }
            ValueKind::Opacity => {
                "must be a number from 0 (transparent) to 1 (opaque)".to_string()
            }
            ValueKind::Enum(options) => format!("must be one of {}", options.join(", ")),
            ValueKind::Identifier => "must be an identifier string".to_string(),
            ValueKind::Glyph => "must be a glyph string".to_string(),
            ValueKind::ImagePath => "must be a relative image path string".to_string(),
            ValueKind::FontList => "must be an array of up to 2 identifier strings".to_string(),
            ValueKind::Bool => "must be a boolean".to_string(),
        }),
    }
}

/// Refuse every `[apps.<app>]` key no allowlist holds, by name. A
/// name-only pass over already-parsed tables; [`parse_look_toml`](crate::schema::parse_look_toml)
/// enforces the same rule (plus per-key values) itself, so there is no
/// unvalidated path through the crate.
pub fn validate_apps_allowlist(
    apps: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, toml::Value>>,
) -> Vec<GrammarError> {
    let mut errors = Vec::new();
    for (app, table) in apps {
        for key in table.keys() {
            if !allows(app, key) {
                errors.push(GrammarError::new(
                    format!("apps.{app}[{key}]"),
                    format!("key `{key}` is not in the `{app}` look allowlist"),
                ));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests;
