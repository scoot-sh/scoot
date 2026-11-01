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
/// 9): the thirteen color roles, the face, its size, opacity, radius,
/// and the icon theme name.
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
    "icons.theme",
];

/// The `shadow` values (`[effects] shadow`, and the notifier's `shadow`
/// override): the depth under a card.
pub const SHADOW_STYLES: &[&str] = &["none", "soft", "deep", "glow"];

/// The `fill` values (`[effects] fill`, and the notifier's `fill` override):
/// how a card's surface color is laid down.
pub const FILL_STYLES: &[&str] = &["solid", "gradient"];

/// The `motion` values (`[effects] motion`, and the notifier's `motion`
/// override): the curve the shell apps' motion follows.
pub const MOTION_CURVES: &[&str] = &["none", "ease-out", "spring"];

/// The `card.enter` and `card.exit` values: how a notification card
/// arrives and leaves.
pub const CARD_MOTIONS: &[&str] = &["slide", "fade", "scale", "drop"];

/// The `card.mark` values: the accent marker a notification card carries.
pub const CARD_MARKS: &[&str] = &["none", "stripe", "dot", "badge"];

/// The `card.shape` values: the card's outline.
pub const CARD_SHAPES: &[&str] = &["rounded", "pill"];

/// The `title.weight` range (CSS-style weights): 100 to 900.
pub const TITLE_WEIGHT_RANGE: (i64, i64) = (100, 900);

/// The `title.spacing` range, in logical pixels per character.
pub const TITLE_SPACING_RANGE: (i64, i64) = (-2, 8);

/// The longest `motion-duration-ms`, matching the shell's own clamp.
pub const MOTION_DURATION_MAX_MS: i64 = 1000;

/// The depth, motion, card-style and typography keys `scootnotify` reads,
/// on top of
/// [`SHELL_APP_ALLOWLIST`]. They are not in the shared list because an app
/// that never reads one would report it as an unknown look key on every
/// load; each app joins as it learns the key (`scootlaunch` reads the
/// first five: [`LAUNCH_DEPTH_KEYS`]).
pub const NOTIFY_DEPTH_KEYS: &[&str] = &[
    "shadow",
    "fill",
    "motion",
    "motion-duration-ms",
    "motion-bounce",
    "card.enter",
    "card.exit",
    "card.mark",
    "card.shape",
    "card.app-accent",
    "title.weight",
    "title.spacing",
];

/// The depth and motion keys `scootlaunch` reads, a subset of
/// [`NOTIFY_DEPTH_KEYS`]: the shadow, the panel fill and the motion. The
/// card and title keys are the notifier's and stay refused here.
pub const LAUNCH_DEPTH_KEYS: &[&str] = &[
    "shadow",
    "fill",
    "motion",
    "motion-duration-ms",
    "motion-bounce",
];

/// The depth keys `scootbar` reads, as dotted look keys under `[bar]`:
/// the shadow under the bar and its popups, how their fill is laid
/// down, and the popup entrance motion. They are not in the shared list
/// for the same reason as the notifier's: an app that never reads one
/// reports it as unknown.
pub const BAR_DEPTH_KEYS: &[&str] = &[
    "bar.shadow",
    "bar.fill",
    "bar.motion",
    "bar.motion-duration-ms",
    "bar.motion-bounce",
];

/// The keys of a look's `[effects]` table, and of the `[effects]` table in
/// a shell app's own config file: the five depth and motion tokens. A
/// subset of every dotless allowlist entry the apps read, so `[effects]`
/// can never reach `font-size`, `opacity` or `radius`.
pub const EFFECTS_TABLE_KEYS: &[&str] = &[
    "shadow",
    "fill",
    "motion",
    "motion-duration-ms",
    "motion-bounce",
];

/// The dotted look key a user-file table entry stands for: `[effects] k`
/// is `k`, `[card] k` is `card.k` and `[title] k` is `title.k`. `None`
/// when `app` does not read that key, or `table` is not one of the three
/// (`[colors]` and `[icons]` belong to the look and are unreachable
/// here). The user-file layer of `scootnotify` and `scootlaunch` asks this
/// so its tables share the look's allowlist, never a copy of it. For
/// `scootbar` `[effects] shadow` is `bar.shadow` (and `fill` likewise):
/// the bar's user file keeps its `[bar]` shape while sharing the same
/// allowlist the look layer is checked against.
pub fn user_effect_key(app: &str, table: &str, key: &str) -> Option<&'static str> {
    if !reads_depth(app) || key.len() > 64 || !key.is_ascii() {
        return None;
    }
    if app == "scootbar" {
        return match (table, key) {
            ("effects", "shadow") => Some("bar.shadow"),
            ("effects", "fill") => Some("bar.fill"),
            ("effects", "motion") => Some("bar.motion"),
            ("effects", "motion-duration-ms") => Some("bar.motion-duration-ms"),
            ("effects", "motion-bounce") => Some("bar.motion-bounce"),
            _ => None,
        };
    }
    let reads = |dotted: &str| match app {
        "scootnotify" => NOTIFY_DEPTH_KEYS.iter().copied().find(|k| *k == dotted),
        _ => LAUNCH_DEPTH_KEYS.iter().copied().find(|k| *k == dotted),
    };
    match table {
        "effects" => EFFECTS_TABLE_KEYS
            .iter()
            .copied()
            .find(|k| *k == key)
            .and_then(reads),
        "card" | "title" => reads(&format!("{table}.{key}")),
        _ => None,
    }
}

/// Whether `app` reads the shared depth and motion keys
/// ([`LAUNCH_DEPTH_KEYS`], [`BAR_DEPTH_KEYS`] for the bar): the apps that
/// draw a surface of their own with a shadow and a fill. The renderer and
/// the value kinds both ask this, so an app joins in one place.
pub fn reads_depth(app: &str) -> bool {
    matches!(app, "scootnotify" | "scootlaunch" | "scootbar")
}

/// Whether `key` (an `[apps.<app>]` dotted key) is allowlisted for `app`.
/// Unknown apps are refused: only the apps below read look layers.
pub fn allows(app: &str, key: &str) -> bool {
    if key.is_empty() || key.len() > 128 || !key.is_ascii() {
        return false;
    }
    match app {
        "scoot" => SCOOT_ALLOWLIST.contains(&key) || is_wallpaper_override_key(key),
        "scootbar" => SCOOTBAR_ALLOWLIST.contains(&key) || BAR_DEPTH_KEYS.contains(&key),
        "scootnotify" => SHELL_APP_ALLOWLIST.contains(&key) || NOTIFY_DEPTH_KEYS.contains(&key),
        "scootlaunch" => SHELL_APP_ALLOWLIST.contains(&key) || LAUNCH_DEPTH_KEYS.contains(&key),
        "scootgreet" => SHELL_APP_ALLOWLIST.contains(&key),
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
    /// `0.0..=1.0`, like [`ValueKind::Opacity`], for a value that is a
    /// fraction and not an opacity (`motion-bounce`): same check, its own
    /// message.
    Unit,
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
    /// A TOML boolean. `workspaces.disc` and the notifier's
    /// `card.app-accent` take one today; the kind
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
            "bar.shadow" => Some(ValueKind::Enum(SHADOW_STYLES)),
            "bar.fill" => Some(ValueKind::Enum(FILL_STYLES)),
            "bar.motion" => Some(ValueKind::Enum(MOTION_CURVES)),
            "bar.motion-duration-ms" => Some(ValueKind::IntRange {
                min: 0,
                max: MOTION_DURATION_MAX_MS,
            }),
            "bar.motion-bounce" => Some(ValueKind::Unit),
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
            "font" | "icons.theme" => Some(ValueKind::Identifier),
            "font-size" => Some(ValueKind::IntRange { min: 1, max: 256 }),
            "opacity" => Some(ValueKind::Opacity),
            "radius" => Some(ValueKind::IntRange { min: 0, max: 64 }),
            "shadow" if reads_depth(app) => Some(ValueKind::Enum(SHADOW_STYLES)),
            "fill" if reads_depth(app) => Some(ValueKind::Enum(FILL_STYLES)),
            "motion" if reads_depth(app) => Some(ValueKind::Enum(MOTION_CURVES)),
            "motion-duration-ms" if reads_depth(app) => Some(ValueKind::IntRange {
                min: 0,
                max: MOTION_DURATION_MAX_MS,
            }),
            "motion-bounce" if reads_depth(app) => Some(ValueKind::Unit),
            "card.enter" | "card.exit" if app == "scootnotify" => {
                Some(ValueKind::Enum(CARD_MOTIONS))
            }
            "card.mark" if app == "scootnotify" => Some(ValueKind::Enum(CARD_MARKS)),
            "card.shape" if app == "scootnotify" => Some(ValueKind::Enum(CARD_SHAPES)),
            "card.app-accent" if app == "scootnotify" => Some(ValueKind::Bool),
            "title.weight" if app == "scootnotify" => Some(ValueKind::IntRange {
                min: TITLE_WEIGHT_RANGE.0,
                max: TITLE_WEIGHT_RANGE.1,
            }),
            "title.spacing" if app == "scootnotify" => Some(ValueKind::IntRange {
                min: TITLE_SPACING_RANGE.0,
                max: TITLE_SPACING_RANGE.1,
            }),
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
        (ValueKind::Unit, toml::Value::Float(f)) => {
            (!f.is_finite() || !(0.0..=1.0).contains(f))
                .then(|| refused("must be a number from 0 to 1".to_string()))?
        }
        (ValueKind::Unit, toml::Value::Integer(i)) => {
            (*i != 0 && *i != 1).then(|| refused("must be a number from 0 to 1".to_string()))?
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
            ValueKind::Unit => "must be a number from 0 to 1".to_string(),
            ValueKind::Enum(options) => format!("must be one of {}", options.join(", ")),
            ValueKind::Identifier => "must be an identifier string".to_string(),
            ValueKind::Glyph => "must be a glyph string".to_string(),
            ValueKind::ImagePath => "must be a relative image path string".to_string(),
            ValueKind::FontList => "must be an array of up to 2 identifier strings".to_string(),
            ValueKind::Bool => "must be a boolean".to_string(),
        }),
    }
}

/// The dotless key the tool renders into `launch.toml` and `notify.toml`
/// (not `greet.toml`, which draws the sans): the file `scoot look apply`
/// resolved the look's `fonts.ui` to, or the fallback its note names when
/// that family is not installed. It is the tool's, like the bar's `font`:
/// not in any allowlist, so `[apps.<app>]` cannot set it. The apps that read it check
/// the value with [`check_font_file`].
pub const FONT_FILE_KEY: &str = "font-file";

/// The longest `font-file` value, in bytes: Linux's `PATH_MAX`.
pub const FONT_FILE_MAX: usize = 4096;

/// Check a fragment's `font-file` value: an absolute path in a string, at
/// most [`FONT_FILE_MAX`] bytes, no NUL. Whether the file exists and is a
/// font is the reading app's to find out (it falls back with a note); only
/// the shape is checked here.
pub fn check_font_file(value: &toml::Value) -> Option<GrammarError> {
    let reason = match value {
        toml::Value::String(path) if path.is_empty() || path.len() > FONT_FILE_MAX => {
            "must be 1 to 4096 bytes"
        }
        toml::Value::String(path) if !path.starts_with('/') => "must be an absolute path",
        toml::Value::String(path) if path.contains('\0') => "must not contain NUL",
        toml::Value::String(_) => return None,
        _ => "must be a path string",
    };
    Some(GrammarError::new(FONT_FILE_KEY, reason))
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
