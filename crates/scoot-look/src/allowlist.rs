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

use crate::grammar::GrammarError;

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
/// the bar's style (never its geometry or placement), and per-module visual
/// keys. `workspaces.active-color` / `inactive-color` are the module's own
/// override knobs (`crates/scootbar/src/config.rs:512-518`);
/// `window-title.icon` is its one-glyph knob.
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
    "bar.font",
    "bar.fallback-fonts",
    "bar.font-size",
    "bar.padding",
    "bar.spacing",
    "bar.separator",
    "workspaces.active-color",
    "workspaces.inactive-color",
    "workspaces.item-gap",
    "workspaces.display",
    "window-title.icon",
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
    match app {
        "scoot" => {
            SCOOT_ALLOWLIST.contains(&key)
                || key.starts_with("wallpaper.output.")
                || key.starts_with("wallpaper.workspace.")
        }
        "scootbar" => SCOOTBAR_ALLOWLIST.contains(&key),
        "scootnotify" | "scootlaunch" | "scootgreet" => SHELL_APP_ALLOWLIST.contains(&key),
        _ => false,
    }
}

/// Refuse every `[apps.<app>]` key no allowlist holds, by name.
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
