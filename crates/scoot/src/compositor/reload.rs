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
//! - `[layout] gap`, `column_widths` and `default_column_width`: applied
//!   through `World::set_config` (which validates like `new` and clamps
//!   every live column preset into a shorter width list, the same clamp
//!   `Config::validated` already applies to `default_column_width` --
//!   clamping rather than a proportional remap, so only a column that would
//!   otherwise index past the end of the new list moves, and no window
//!   silently changes relative size), then `apply()` recomputes the
//!   arrangement and requests a render. New columns take the reloaded
//!   default from their next creation on; `cycle-column-width` steps, and
//!   `set-column-width N` range-checks, against the new list's length from
//!   the same reload on.
//! - `[appearance]` ring width and colors, background, `corner_radius`,
//!   `prefer_no_csd`, and the cursor fields (`cursor_size`, `cursor_color`,
//!   `cursor_theme`): applied -- every reader takes the ring/background
//!   values live from `State::appearance` (the render path,
//!   `XdgDecorationHandler`), and the cursor ones rebuild `Cursor` in place
//!   (`Cursor::rebuild`: new fallback bitmaps, the theme reloaded, the
//!   current shape re-resolved) with `State::appearance` written alongside
//!   so the next reload diffs against what this one did. A render is
//!   requested; the arrangement is untouched (cursor pixels are not
//!   placement), so no `apply()` runs.
//!   `XCURSOR_THEME`/`XCURSOR_SIZE` for future children follow from the same
//!   rebuild -- see `State::spawn`, which exports the live theme per child.
//! - `[output] scale`: re-advertised to every output (bound `wl_output`
//!   clients hear the new integer through the same `set_mode` startup uses)
//!   and re-sent to every live surface (the fractional `preferred_scale`
//!   plus its integer companion, walked over every window, layer, lock and
//!   cursor tree), then re-laid-out: every logical geometry is recomputed
//!   and filed with the core, the arrangement recomputed and the screen
//!   redrawn. Under `--nested` a non-1.0 value refuses -- the host owns the
//!   scale there -- rather than applying.
//! - `[binds]`: rebuilt from defaults plus the file (see
//!   [`keybindings_for`](super::config::keybindings_for)), with the `--tty`
//!   `Ctrl+Alt+F1..F12` recovery bindings layered on last when this session
//!   drives `--tty` -- so a reload can neither strip the recovery path nor
//!   gain it on a backend that never had it. Swapped in whole; a key held
//!   across the swap cannot wedge or drop, because press/release routing
//!   never consults the table mid-hold: the press records its keycode in
//!   `suppressed_keys` (or doesn't), and the release is routed by that set,
//!   not by what the table says now (see `input::key`).
//! - `[tty] gpu`, `[renderer] backend`, `[autostart] commands`: refused when
//!   they differ from what the session runs. Each names something fixed
//!   before the first frame (the device already driven, the renderer with
//!   client textures in it, commands that ran once at startup).
//!
//! Every refusal names the field; nothing is silently ignored. Both lists
//! name only fields that *differed* -- a field the file and the session
//! agree on appears in neither, so two empty lists together mean "the
//! reload changed nothing it was asked to".
//!
//! # While locked
//!
//! A reload applies under session lock, deliberately. Nothing in the applied
//! set can disclose locked content: the ring, background and cursor pixels
//! are all derived from config values and installed theme files, never from
//! a client surface -- and the cursor rebuild replaces only those pixels
//! without touching `status` (which shape shows; the lock reset it to the
//! default at lock time), so a locked frame draws the same shape from new
//! pixels, never new content. Gap, column widths and binds are
//! input-side -- widths only re-derive column frames from config
//! proportions, never from what a client drew -- and binds cannot fire actions while locked anyway
//! (`input::key` forwards them to the lock client). A scale change only
//! re-derives the same geometry the lock path already publishes (lock
//! surfaces are reconfigured to their output's new logical size, exactly as
//! a `--tty` hotplug resize does) and re-sends config-derived scale values
//! to surfaces that keep showing the blanked frame -- no client pixels move
//! across the lock boundary. Refusing under lock
//! would strand an agent that edits the file mid-lock with an error for a
//! request that is safe to serve.

use scoot_ipc::Response;

use super::State;
use super::config::{self, LoadedConfig};
use super::output_scale::integer_scale;

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
    pub const CORNER_RADIUS: &str = "appearance.corner_radius";
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
            // Logged as well as answered: over IPC the client sees the
            // error, but a SIGHUP trigger has no reply channel, so without
            // this nothing would observe the refusal anywhere.
            tracing::error!(
                "config reload refused: no resolvable config path; keeping the running config"
            );
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
    /// only mutation of live layout/appearance/cursor/binds state on this
    /// path, so a failure above (which returns before this runs) cannot
    /// half-apply.
    ///
    /// One applier per field family, in dependency order (layout before
    /// appearance: the ring re-clamp reads the applied gap). Each compares
    /// against the live value -- or, where there is no live value to read
    /// (the device already driven, the entries already run), against the
    /// `startup_*` snapshot `run` wrote once and nothing ever advances --
    /// and writes `State::appearance`/cursor/world/binds alongside every
    /// report entry, so a second reload diffs against what the first applied
    /// and re-reports nothing.
    fn apply_reload(&mut self, fresh: &LoadedConfig) -> Report {
        let mut report = Report::default();
        self.apply_layout_reload(fresh, &mut report);
        self.apply_appearance_reload(fresh, &mut report);
        self.apply_scale_reload(fresh, &mut report);
        self.apply_startup_only_reload(fresh, &mut report);
        self.apply_binds_reload(fresh, &mut report);

        // Recompute the arrangement and request a render when anything
        // visible moved. A binds-only reload skips it: no placement changed,
        // so there is nothing to configure and nothing dirty. A cursor-only
        // reload redraws without re-arranging: cursor pixels are not
        // placement, so `apply()` would reconfigure every window for nothing.
        if report.applied.iter().any(|name| {
            name == field::GAP
                || name == field::COLUMN_WIDTHS
                || name == field::DEFAULT_COLUMN_WIDTH
                || name == field::SCALE
                || name == field::RING_WIDTH
                || name == field::RING_ACTIVE
                || name == field::RING_INACTIVE
                || name == field::BACKGROUND
                || name == field::CORNER_RADIUS
        }) {
            self.apply();
        } else if report.applied.iter().any(|name| {
            name == field::CURSOR_SIZE || name == field::CURSOR_COLOR || name == field::CURSOR_THEME
        }) {
            self.request_render();
        }
        report
    }

    /// `[layout]`: `gap`, `column_widths` and `default_column_width` apply
    /// through one `World::set_config` (which validates like `new` and
    /// clamps live presets into a shorter width list, so `arrange` stays in
    /// range). Each field reports under its own name, but a single swap
    /// serves all three -- widths, default and gap are one `Config`, never
    /// a half-applied session. Compared against the live world config, and
    /// the stored config is exactly what was compared, so the next reload
    /// agrees silently.
    fn apply_layout_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        let live = self.world.config();
        let gap = fresh.config.gap != live.gap;
        let widths = fresh.config.column_widths != live.column_widths;
        let default = fresh.config.default_column_width != live.default_column_width;
        if gap || widths || default {
            let mut config = live.clone();
            config.gap = fresh.config.gap;
            config.column_widths.clone_from(&fresh.config.column_widths);
            config.default_column_width = fresh.config.default_column_width;
            self.world.set_config(config);
            if gap {
                report.applied.push(field::GAP.to_owned());
            }
            if widths {
                report.applied.push(field::COLUMN_WIDTHS.to_owned());
            }
            if default {
                report.applied.push(field::DEFAULT_COLUMN_WIDTH.to_owned());
            }
        }
    }

    /// `[appearance]`: the ring/background/corner/`prefer_no_csd` fields
    /// write `State::appearance` live; the cursor fields additionally
    /// rebuild `Cursor` in place. Compared against the pre-mutation clone,
    /// so every field diffs against the running session even when an
    /// earlier one in this same reload already wrote.
    fn apply_appearance_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
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
        if appearance.corner_radius != live.corner_radius {
            self.appearance.corner_radius = appearance.corner_radius;
            report.applied.push(field::CORNER_RADIUS.to_owned());
            appearance_changed = true;
        }
        if appearance.prefer_no_csd != live.prefer_no_csd {
            self.appearance.prefer_no_csd = appearance.prefer_no_csd;
            report.applied.push(field::PREFER_NO_CSD.to_owned());
            // No re-render needed on its own: this only answers future
            // `zxdg_toplevel_decoration_v1` requests, it repaints nothing.
        }
        // The cursor triple: each field reports under its own name, but one
        // rebuild serves all three -- bitmaps, theme and re-resolution are
        // one atomic swap, never a half-rebuilt cursor. `fresh` is already
        // load-clamped (including `cursor_size`), and what is stored here is
        // exactly what was compared, so the next reload agrees silently.
        // `Theme::load` never fails (an unresolvable name is an empty theme
        // drawn as the fallback shapes), so there is no failure half to
        // guard: the writes below cannot partially happen. Known corner, not
        // a bug: reloading `cursor_theme` back to unset resolves through
        // `$XCURSOR_THEME`, which startup already overwrote with the
        // resolved name -- so the session keeps the startup value rather
        // than re-reading the outer environment. Self-consistent and
        // idempotent; only a restart picks up an externally changed variable.
        let cursor_size = appearance.cursor_size != live.cursor_size;
        let cursor_color = appearance.cursor_color != live.cursor_color;
        let cursor_theme = appearance.cursor_theme != live.cursor_theme;
        if cursor_size || cursor_color || cursor_theme {
            self.appearance.cursor_size = appearance.cursor_size;
            self.appearance.cursor_color = appearance.cursor_color;
            self.appearance
                .cursor_theme
                .clone_from(&appearance.cursor_theme);
            self.cursor.rebuild(
                appearance.cursor_size,
                appearance.cursor_color,
                appearance.cursor_theme.as_deref(),
            );
            if cursor_size {
                report.applied.push(field::CURSOR_SIZE.to_owned());
            }
            if cursor_color {
                report.applied.push(field::CURSOR_COLOR.to_owned());
            }
            if cursor_theme {
                report.applied.push(field::CURSOR_THEME.to_owned());
            }
            appearance_changed = true;
        }
        // `Appearance::clamped` bounds the ring against half the gap at
        // load, and the gap may just have moved: re-clamp the applied ring
        // against the applied gap rather than trusting the file's
        // arithmetic. `clamped` warns on its own when it changes anything.
        // (The cursor size needs no second clamp here: `rebuild` applies
        // the same bound at the allocation, and the stored value is the
        // load-clamped one both sides agree on.)
        if appearance_changed {
            let gap = self.world.config().gap;
            self.appearance = self.appearance.clone().clamped(gap);
        }
    }

    /// `[output] scale`: re-advertised to every output and re-sent to every
    /// live surface, then re-laid-out (see `rescale_outputs` and
    /// `resend_output_scale`). Compared against the live
    /// `State::output_scale`, and that field -- plus its precomputed integer
    /// -- is exactly what is stored, so a second reload agrees silently.
    /// `fresh.scale` is already load-clamped (including the non-finite
    /// fallback), so an out-of-range value applies as its clamped self,
    /// never as a refusal.
    ///
    /// Under `--nested` a differing value refuses instead: the host owns the
    /// scale there (`compositor::run` forced the live value to 1.0 with a
    /// warning), so any difference is a non-1.0 ask by construction.
    fn apply_scale_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        match scale_reload(fresh.scale, self.output_scale, self.host.is_some()) {
            ScaleReload::Agree => {}
            ScaleReload::RefuseNested => {
                report.refused.push(refused(
                    field::SCALE,
                    "refused under --nested: the host compositor owns the window's scale",
                ));
            }
            ScaleReload::Apply => {
                self.output_scale = fresh.scale;
                self.integer_scale = integer_scale(fresh.scale);
                self.rescale_outputs(fresh.scale);
                self.resend_output_scale();
                report.applied.push(field::SCALE.to_owned());
            }
        }
    }

    /// The fields with no live state to compare against: `[tty] gpu`,
    /// `[renderer] backend`, `[autostart] commands`. Each diffs against the
    /// `startup_*` snapshot (or the fixed live value, where the session
    /// carries one) and refuses when it differs -- see the module doc for
    /// why none of these applies live. (`[output] scale` used to refuse
    /// here too; it applies live now, through `apply_scale_reload` above.)
    fn apply_startup_only_reload(&self, fresh: &LoadedConfig, report: &mut Report) {
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
    }

    /// `[binds]`: rebuilt from defaults plus the file (see
    /// [`keybindings_for`](super::config::keybindings_for)), swapped in
    /// whole. A key held across the swap neither wedges nor drops -- see
    /// the module doc.
    fn apply_binds_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        if !fresh.keybindings.same_bindings_as(&self.keybindings) {
            self.keybindings = fresh.keybindings.clone();
            report.applied.push(field::BINDS.to_owned());
        }
    }
}

/// A reload's applied-vs-refused lists, in reply order.
#[derive(Debug, Default)]
struct Report {
    applied: Vec<String>,
    refused: Vec<String>,
}

/// What a reloaded `[output] scale` does: applies live, agrees silently, or
/// refuses under `--nested`.
#[derive(Debug, PartialEq, Eq)]
enum ScaleReload {
    /// The file and the session agree: silent in both lists.
    Agree,
    /// A new value on a backend the compositor scales itself: re-advertise,
    /// re-send, re-lay-out, report applied.
    Apply,
    /// A differing value under `--nested`, where the host owns the scale.
    RefuseNested,
}

/// Diffs a reloaded scale against the live one. Pure so the `--nested`
/// refusal pins without a host connection, which no test harness can fake:
/// `nested` is whether the session presents into a host compositor.
fn scale_reload(fresh: f64, live: f64, nested: bool) -> ScaleReload {
    if fresh == live {
        ScaleReload::Agree
    } else if nested {
        ScaleReload::RefuseNested
    } else {
        ScaleReload::Apply
    }
}

fn refused(field: &str, reason: &str) -> String {
    format!("{field} ({reason})")
}
