//! Re-reading the config file on a live session (`Request::Reload`).
//!
//! Startup reads every setting once and degrades gracefully (see
//! `config.rs`'s module doc); a reload must not do either of those. It
//! loads the file into a temp [`LoadedConfig`](super::config::LoadedConfig)
//! and validates it fully *before* touching live state -- a malformed file,
//! an unknown field, or a vanished path keeps the running config untouched
//! and answers an error, never defaults and never a half-applied session.
//!
//! # What applies, and what refuses
//!
//! - `[layout] gap`: applied through `World::set_config` (which clamps like
//!   `new`), then `apply()` recomputes the arrangement and requests a
//!   render. `column_widths`/`default_column_width` are refused when they
//!   differ: existing columns hold presets into that list, and a shorter
//!   list would index out of range in `arrange` -- so the refusal is
//!   structural, not taste. (The gap-only `Config` handed to `set_config`
//!   keeps the running widths, which is what makes that panic unreachable
//!   rather than merely avoided.)
//! - `[appearance]` ring width and colors, background, `prefer_no_csd`:
//!   applied -- every reader takes them live from `State::appearance` (the
//!   render path, `XdgDecorationHandler`). The cursor fields (`cursor_size`,
//!   `cursor_color`, `cursor_theme`) are refused: `Cursor::new` consumes
//!   them once at startup into a bitmap and a loaded theme, so writing them
//!   here would change nothing.
//! - `[binds]`: rebuilt from defaults plus the file (see
//!   [`keybindings_for`](super::config::keybindings_for)), with the `--tty`
//!   `Ctrl+Alt+F1..F12` recovery bindings layered on last when this session
//!   drives `--tty` -- so a reload can neither strip the recovery path nor
//!   gain it on a backend that never had it. Swapped in whole; a key held
//!   across the swap cannot wedge or drop, because press/release routing
//!   never consults the table mid-hold: the press records its keycode in
//!   `suppressed_keys` (or doesn't), and the release is routed by that set,
//!   not by what the table says now (see `input::key`).
//! - `[output] scale`, `[tty] gpu`, `[renderer] backend`,
//!   `[autostart] commands`: refused when they differ from what the session
//!   runs. Each names something fixed before the first frame (the scale
//!   clients were told at bind time, the device already driven, the renderer
//!   with client textures in it, commands that ran once at startup).
//!
//! Every refusal names the field; nothing is silently ignored. Both lists
//! name only fields that *differed* -- a field the file and the session
//! agree on appears in neither, so two empty lists together mean "the
//! reload changed nothing it was asked to".
//!
//! # While locked
//!
//! A reload applies under session lock, deliberately. Nothing in the applied
//! set can disclose locked content: appearance changes only recolor the
//! ring and background the lock screen already shows (the render path draws
//! the lock surface and nothing else while locked), gap and binds are
//! input-side, and binds cannot fire actions while locked anyway
//! (`input::key` forwards them to the lock client). Refusing under lock
//! would strand an agent that edits the file mid-lock with an error for a
//! request that is safe to serve.

use scoot_ipc::Response;

use super::State;
use super::config::{self, LoadedConfig};

#[cfg(test)]
mod tests;

/// Dotted names for the reply lists. One constant per field rather than
/// `stringify!`-style derives, so renaming a config key without updating its
/// reload name fails loudly at the use site, not silently on the wire.
mod field {
    pub const GAP: &str = "layout.gap";
    pub const COLUMN_WIDTHS: &str = "layout.column_widths";
    pub const DEFAULT_COLUMN_WIDTH: &str = "layout.default_column_width";
    pub const RING_WIDTH: &str = "appearance.focus_ring_width";
    pub const RING_ACTIVE: &str = "appearance.focus_ring_active_color";
    pub const RING_INACTIVE: &str = "appearance.focus_ring_inactive_color";
    pub const BACKGROUND: &str = "appearance.background_color";
    pub const CURSOR_SIZE: &str = "appearance.cursor_size";
    pub const CURSOR_COLOR: &str = "appearance.cursor_color";
    pub const CURSOR_THEME: &str = "appearance.cursor_theme";
    pub const PREFER_NO_CSD: &str = "appearance.prefer_no_csd";
    pub const SCALE: &str = "output.scale";
    pub const GPU: &str = "tty.gpu";
    pub const BACKEND: &str = "renderer.backend";
    pub const AUTOSTART: &str = "autostart.commands";
    pub const BINDS: &str = "binds";
}

impl State {
    /// Serves `Request::Reload`: re-reads [`State::config_path`] and
    /// re-applies what can be re-applied live. See the module doc for the
    /// field list, the failure semantics, and the while-locked decision.
    pub fn reload(&mut self) -> Response {
        let Some(path) = self.config_path.clone() else {
            return Response::error(
                "refused: this session started with no resolvable config path \
                 (neither XDG_CONFIG_HOME nor HOME is set), so there is no \
                 file to reload; keeping the running config",
            );
        };
        let fresh = match config::reload_from(&path, self.tty.is_some()) {
            Ok(fresh) => fresh,
            Err(error) => {
                tracing::error!(%error, "config reload failed");
                return Response::error(error.to_string());
            }
        };
        let report = self.apply_reload(&fresh);
        tracing::info!(
            applied = ?report.applied,
            refused = ?report.refused,
            "config reloaded"
        );
        Response::Reloaded {
            applied: report.applied,
            refused: report.refused,
        }
    }

    /// Diffs a validated `fresh` load against the running session and swaps
    /// in what applies. Pure comparison first, mutation after -- and the
    /// only mutation of live layout/appearance/binds state on this path, so
    /// a failure above (which returns before this runs) cannot half-apply.
    fn apply_reload(&mut self, fresh: &LoadedConfig) -> Report {
        let mut report = Report::default();

        if fresh.config.gap != self.world.config().gap {
            let mut config = self.world.config().clone();
            config.gap = fresh.config.gap;
            self.world.set_config(config);
            report.applied.push(field::GAP.to_owned());
        }
        if fresh.config.column_widths != self.world.config().column_widths {
            report.refused.push(refused(
                field::COLUMN_WIDTHS,
                "startup-only: live columns hold presets into this list",
            ));
        }
        if fresh.config.default_column_width != self.world.config().default_column_width {
            report.refused.push(refused(
                field::DEFAULT_COLUMN_WIDTH,
                "startup-only: new columns take it once, at creation",
            ));
        }

        let appearance = &fresh.appearance;
        let live = self.appearance.clone();
        let mut appearance_changed = false;
        if appearance.focus_ring_width != live.focus_ring_width {
            self.appearance.focus_ring_width = appearance.focus_ring_width;
            report.applied.push(field::RING_WIDTH.to_owned());
            appearance_changed = true;
        }
        if appearance.focus_ring_active_color != live.focus_ring_active_color {
            self.appearance.focus_ring_active_color = appearance.focus_ring_active_color;
            report.applied.push(field::RING_ACTIVE.to_owned());
            appearance_changed = true;
        }
        if appearance.focus_ring_inactive_color != live.focus_ring_inactive_color {
            self.appearance.focus_ring_inactive_color = appearance.focus_ring_inactive_color;
            report.applied.push(field::RING_INACTIVE.to_owned());
            appearance_changed = true;
        }
        if appearance.background_color != live.background_color {
            self.appearance.background_color = appearance.background_color;
            report.applied.push(field::BACKGROUND.to_owned());
            appearance_changed = true;
        }
        if appearance.prefer_no_csd != live.prefer_no_csd {
            self.appearance.prefer_no_csd = appearance.prefer_no_csd;
            report.applied.push(field::PREFER_NO_CSD.to_owned());
            // No re-render needed on its own: this only answers future
            // `zxdg_toplevel_decoration_v1` requests, it repaints nothing.
        }
        if appearance.cursor_size != live.cursor_size {
            report.refused.push(refused(
                field::CURSOR_SIZE,
                "startup-only: the fallback bitmap is built once, at startup",
            ));
        }
        if appearance.cursor_color != live.cursor_color {
            report.refused.push(refused(
                field::CURSOR_COLOR,
                "startup-only: the fallback bitmap is built once, at startup",
            ));
        }
        if appearance.cursor_theme != live.cursor_theme {
            report.refused.push(refused(
                field::CURSOR_THEME,
                "startup-only: the theme is loaded once, at startup",
            ));
        }
        // `Appearance::clamped` bounds the ring against half the gap at
        // load, and the gap may just have moved: re-clamp the applied ring
        // against the applied gap rather than trusting the file's
        // arithmetic. `clamped` warns on its own when it changes anything.
        if appearance_changed {
            let gap = self.world.config().gap;
            self.appearance = self.appearance.clone().clamped(gap);
        }

        if fresh.scale != self.output_scale {
            report.refused.push(refused(
                field::SCALE,
                "startup-only: clients were told the scale at bind time",
            ));
        }
        if fresh.gpu != self.startup_gpu {
            report.refused.push(refused(
                field::GPU,
                "startup-only: the session already drives its device",
            ));
        }
        if fresh.renderer.is_some_and(|kind| kind != self.renderer) {
            report.refused.push(refused(
                field::BACKEND,
                "startup-only: the live renderer holds client textures",
            ));
        }
        if fresh.autostart != self.startup_autostart {
            report.refused.push(refused(
                field::AUTOSTART,
                "startup-only: entries run once, at session start",
            ));
        }

        if !fresh.keybindings.same_bindings_as(&self.keybindings) {
            self.keybindings = fresh.keybindings.clone();
            report.applied.push(field::BINDS.to_owned());
        }

        // Recompute the arrangement and request a render when anything
        // visible moved. A binds-only reload skips it: no placement changed,
        // so there is nothing to configure and nothing dirty.
        if report.applied.iter().any(|name| {
            name == field::GAP
                || name == field::RING_WIDTH
                || name == field::RING_ACTIVE
                || name == field::RING_INACTIVE
                || name == field::BACKGROUND
        }) {
            self.apply();
        }
        report
    }
}

/// A reload's applied-vs-refused lists, in reply order.
#[derive(Debug, Default)]
struct Report {
    applied: Vec<String>,
    refused: Vec<String>,
}

fn refused(field: &str, reason: &str) -> String {
    format!("{field} ({reason})")
}
