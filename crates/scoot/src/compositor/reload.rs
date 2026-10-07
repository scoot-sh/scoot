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
//! - `[output] scale` and each `[[outputs]]` entry's `scale`: every output's
//!   scale is re-decided (its entry's, else the default -- see
//!   `output_config.rs`), re-advertised on every output
//!   (bound `wl_output` clients hear the current mode, scale and `done`
//!   through the same `set_mode` startup uses, whether or not that output's
//!   own scale moved) and re-sent to every live surface (the
//!   fractional `preferred_scale` plus its integer companion, each told its
//!   own output's scale, walked over every window, layer, lock and cursor
//!   tree), then re-laid-out: every logical geometry is recomputed and filed
//!   with the core, the arrangement recomputed and the screen redrawn.
//!   Reported per field -- `output.scale` for the default and
//!   `outputs.<name>.scale` for an entry -- even when the output it names is
//!   not connected (the value is stored, and applies when it is). Under
//!   `--nested` any change refuses -- the host owns the scale there --
//!   rather than applying.
//! - `[[outputs]]` `mode`: refused by name (`outputs.<name>.mode`), pending a
//!   restart. Applying it live would be a modeset on a driven head, which
//!   no reload does (see `output_config.rs`); the session keeps the mode it
//!   started with, and that is also the mode a replugged monitor comes back
//!   at.
//! - `[binds]`: rebuilt from defaults plus the file (see
//!   [`keybindings_for`](super::config::keybindings_for)), with the `--tty`
//!   `Ctrl+Alt+F1..F12` recovery bindings layered on last when this session
//!   drives `--tty` -- so a reload can neither strip the recovery path nor
//!   gain it on a backend that never had it. Swapped in whole; a key held
//!   across the swap cannot wedge or drop, because press/release routing
//!   never consults the table mid-hold: the press records its keycode in
//!   `suppressed_keys` (or doesn't), and the release is routed by that set,
//!   not by what the table says now (see `input::key`).
//! - `[tty] gpu`, `[renderer] backend`, `[xwayland] enabled`,
//!   `[virtual_input] enabled` and `[virtual_input] binds`: refused when
//!   they differ from what the session runs, naming restart as the remedy.
//!   Each names something fixed before the first frame (the device already
//!   driven, the renderer with client textures in it, the X server started
//!   once or never, the globals advertised once or never, the trust
//!   boundary a connected remote already holds) that no live swap can reach
//!   proportionate to its risk -- rebuilding any of them mid-session is a
//!   restart keeping clients, every step fallible mid-flight -- so the
//!   refusal stands and says so. (`binds` is read on every virtual key, so
//!   it *could* flip live like `[binds]`; it refuses with the section
//!   instead, so the whole `[virtual_input]` contract is one restart-only
//!   gate and a mid-hold flip can never strand virtual suppressed state --
//!   see `virtual_input.rs`.)
//! - `[xwayland] fractional`: applied -- the stored choice is swapped and
//!   the X scale re-chosen through the one chooser
//!   (`State::refit_xwayland`), exactly like a scale change: the client
//!   scale, the XSETTINGS toolkits read and every open X window's
//!   configure follow when the choice moves the X scale, and nothing is
//!   sent when it does not (scale 1 and integer scales draw the same
//!   either way). A value that names neither `sharp` nor `light` is refused
//!   by name and the session keeps its own, like `[floating] modifier`.
//!   Applies under lock too: only scale numbers move, never client pixels
//!   (see "While locked" below).
//! - `[autostart] commands`: the spawn delta applies -- entries the session
//!   has not seen run once each, in file order, through the same `act` path
//!   startup drains. New `Spawn` entries only: a reloaded non-spawn action
//!   (`quit` included) is refused by name and never acted on. Seen means "in
//!   the `startup_autostart` snapshot", by value per occurrence: an edited
//!   entry is new, a removed-then-re-added entry runs again, and duplicates
//!   count per occurrence. A spawn the OS refuses (a missing program) is
//!   refused by name and stays pending -- retried on the next reload, never
//!   silently dropped -- so `applied` means the entry started, not merely
//!   that it was attempted. The snapshot advances past decided entries only
//!   (accepted spawns, refused non-spawns), which is why a second identical
//!   reload is silent.
//!
//! - `[floating] modifier`: applied -- the next Mod+press reads it (a drag
//!   under way when the reload lands carries on). A value that is not a
//!   modifier is refused by name, and the session keeps its own.
//! - `[floating] auto` and `[[window_rule]]`: applied -- swapped in whole
//!   and read at every window's first commit from then on. Windows already
//!   mapped are not re-decided (rules apply at map time; see
//!   `floating.rs`), so nothing re-arranges. A rule that parses but cannot
//!   be used (no matcher, a bad size, ...) is refused by its position in the
//!   file, e.g. `window_rule #3 (sets neither float nor size): skipped as
//!   unusable`, and the rest apply.
//!
//! - `[wallpaper]`: handed to `scootbg apply-config` again on every reload
//!   while the section exists, and `{}` on the reload that removes it (see
//!   `wallpaper.rs`). Reported applied as `wallpaper` when the section's
//!   values differ from what the session last handed over, and as
//!   `wallpaper.command` when `command` does (both, when both do) --
//!   "applied" meaning handed over, never waited on. A section with a
//!   problem (inside `[wallpaper]`, an unknown key no longer fails the
//!   whole reload) is refused by name on every reload that finds it, and
//!   the running one is kept and re-run.
//!
//! Every refusal names the field; nothing is silently ignored. Both lists
//! name only fields that *differed* -- a field the file and the session
//! agree on appears in neither, so two empty lists together mean "the
//! reload changed nothing it was asked to". Two exceptions are refused on
//! every reload that finds them: an unusable window rule, and a
//! `[wallpaper]` section with a problem (neither is ever in effect, so each
//! always differs from what the file says).
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
//! proportions, never from what a client drew -- and the only binds that fire while locked are
//! `spawn`s flagged `allow_when_locked` (`input::key` forwards every other
//! bind to the lock client), which draw nothing and take no focus over the
//! lock surface. A scale change only
//! re-derives the same geometry the lock path already publishes (lock
//! surfaces are reconfigured to their output's new logical size, exactly as
//! a `--tty` hotplug resize does) and re-sends config-derived scale values
//! to surfaces that keep showing the blanked frame -- no client pixels move
//! across the lock boundary. Refusing under lock
//! would strand an agent that edits the file mid-lock with an error for a
//! request that is safe to serve.
//!
//! `[wallpaper]` applies under lock too: scootbg draws on the background
//! layer, which a locked frame does not show, so the new wallpaper
//! discloses nothing and is simply there on unlock.
//!
//! Autostart is the deliberate exception: a reload under lock neither runs
//! new spawn entries -- a spawned program at lock time could disclose a
//! window onto, or interfere with, the locked session -- nor drops them.
//! The snapshot freezes, the reply refuses the field as skipped-while-locked,
//! and the first unlocked reload decides what is still pending: new spawns
//! run, non-spawns refuse by name. Deferred, not denied. (An empty delta --
//! a pure removal, or nothing new at all -- stays silent even under lock;
//! there is nothing actionable to skip.)

use scoot_core::Action;
use scoot_ipc::Response;

use super::State;
use super::config::{self, LoadedConfig};
use super::output_config::EntriesDiff;
use super::wallpaper::Reloaded;

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
    pub const RING_INACTIVE_WIDTH: &str = "appearance.focus_ring_inactive_width";
    pub const RING_ACTIVE: &str = "appearance.focus_ring_active_color";
    pub const RING_INACTIVE: &str = "appearance.focus_ring_inactive_color";
    pub const BACKGROUND: &str = "appearance.background_color";
    pub const CORNER_RADIUS: &str = "appearance.corner_radius";
    pub const CURSOR_SIZE: &str = "appearance.cursor_size";
    pub const CURSOR_COLOR: &str = "appearance.cursor_color";
    pub const CURSOR_THEME: &str = "appearance.cursor_theme";
    pub const CURSOR_HIDE: &str = "appearance.cursor_hide_after_ms";
    pub const PREFER_NO_CSD: &str = "appearance.prefer_no_csd";
    pub const SCALE: &str = "output.scale";
    /// `outputs.<name>.scale` / `outputs.<name>.mode`: per output, so the
    /// name is built from the entry (see `output_field`).
    pub const OUTPUTS: &str = "outputs";
    pub const GPU: &str = "tty.gpu";
    pub const BACKEND: &str = "renderer.backend";
    pub const XWAYLAND: &str = "xwayland.enabled";
    /// `[virtual_input] enabled`: whether the virtual-pointer and
    /// virtual-keyboard globals are advertised.
    pub const VIRTUAL_INPUT: &str = "virtual_input.enabled";
    /// `[virtual_input] binds`: whether virtual-keyboard keys run binds.
    pub const VIRTUAL_INPUT_BINDS: &str = "virtual_input.binds";
    /// `[xwayland] fractional`: what X draws at a fractional scale.
    pub const FRACTIONAL: &str = "xwayland.fractional";
    pub const AUTOSTART: &str = "autostart.commands";
    pub const FLOATING_AUTO: &str = "floating.auto";
    pub const FLOATING_MODIFIER: &str = "floating.modifier";
    pub const WINDOW_RULES: &str = "window_rule";
    pub const BINDS: &str = "binds";
    pub const WALLPAPER: &str = "wallpaper";
    pub const WALLPAPER_COMMAND: &str = "wallpaper.command";
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
    /// (the device already driven), against the `startup_*` snapshot `run`
    /// wrote once and nothing ever advances -- and writes
    /// `State::appearance`/cursor/world/binds alongside every report entry,
    /// so a second reload diffs against what the first applied and
    /// re-reports nothing. (`startup_autostart` is the one snapshot that
    /// advances: every *unlocked* reload moves it past the entries the reload
    /// decided (accepted spawns, refused non-spawns) -- a failed spawn stays
    /// pending and retries, so decided entries never re-report; a *locked*
    /// reload freezes it, deferring the delta to the first unlocked reload.)
    fn apply_reload(&mut self, fresh: &LoadedConfig) -> Report {
        let mut report = Report::default();
        self.apply_layout_reload(fresh, &mut report);
        self.apply_appearance_reload(fresh, &mut report);
        self.apply_output_reload(fresh, &mut report);
        self.apply_xwayland_fractional_reload(fresh, &mut report);
        self.apply_device_reload(fresh, &mut report);
        self.apply_autostart_reload(fresh, &mut report);
        self.apply_floating_reload(fresh, &mut report);
        self.apply_binds_reload(fresh, &mut report);
        self.apply_wallpaper_reload(fresh, &mut report);

        // Recompute the arrangement and request a render when anything
        // visible moved. A binds-only reload skips it: no placement changed,
        // so there is nothing to configure and nothing dirty. A cursor-only
        // reload redraws without re-arranging: cursor pixels are not
        // placement, so `apply()` would reconfigure every window for nothing.
        if report.applied.iter().any(|name| {
            name == field::GAP
                || name == field::COLUMN_WIDTHS
                || name == field::DEFAULT_COLUMN_WIDTH
                || name == field::RING_WIDTH
                || name == field::RING_INACTIVE_WIDTH
                || name == field::RING_ACTIVE
                || name == field::RING_INACTIVE
                || name == field::BACKGROUND
                || name == field::CORNER_RADIUS
        }) || report.rescaled
        {
            self.apply();
        }
        // A rebuilt cursor is a cursor change like any other: a redraw where
        // frames draw it, and news for every capture that asked for the
        // pointer (`State::cursor_changed`) -- on its own path so a reload
        // that also moved placement pays both, and one that changed only the
        // cursor re-serves no capture that did not ask for it.
        if report.applied.iter().any(|name| {
            name == field::CURSOR_SIZE || name == field::CURSOR_COLOR || name == field::CURSOR_THEME
        }) {
            self.cursor_changed();
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
        if appearance.focus_ring_inactive_width != live.focus_ring_inactive_width {
            self.appearance.focus_ring_inactive_width = appearance.focus_ring_inactive_width;
            report.applied.push(field::RING_INACTIVE_WIDTH.to_owned());
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
            // The overlay-plane twins are keyed by the old images' ids: drop
            // them now rather than leaving stale dma-bufs cached per output
            // until the bound turns over (see `render::cursor_plane`).
            #[cfg(feature = "gpu-scanout")]
            if let Some(tty) = self.tty.as_mut() {
                tty.clear_cursor_overlays();
            }
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
        // load-clamped one both sides agree on. The hide delay's bound --
        // the one-day cap in `Appearance::clamped`, which every load and
        // reload runs through -- likewise already holds on `fresh`.)
        if appearance_changed {
            let gap = self.world.config().gap;
            self.appearance = self.appearance.clone().clamped(gap);
        }
        // The hide delay is not placement and rebuilds nothing: it only
        // moves the deadline the cursor-hide timer serves. Re-timed here
        // rather than left to the next `apply()`, so disabling -- or
        // shortening or lengthening -- the delay takes effect on this
        // reload: `update_cursor_hide` would leave an armed deadline
        // computed under the old delay in place.
        if appearance.cursor_hide_after_ms != live.cursor_hide_after_ms {
            self.appearance.cursor_hide_after_ms = appearance.cursor_hide_after_ms;
            report.applied.push(field::CURSOR_HIDE.to_owned());
            self.retime_cursor_hide(std::time::Instant::now());
        }
    }

    /// `[output] scale` and `[[outputs]]`: every output's scale re-decided
    /// and re-applied live, a changed entry mode refused (see the module
    /// doc). Compared against the live [`State::default_scale`] and
    /// [`State::output_entries`], and what is stored is exactly what was
    /// compared (with the live modes kept), so a second reload agrees
    /// silently. `fresh` is already load-clamped (including the non-finite
    /// fallbacks), so an out-of-range value applies as its clamped self,
    /// never as a refusal.
    ///
    /// The outputs are re-laid-out only when an output's scale actually
    /// moves: a changed entry for a monitor that is not connected, or a
    /// default every connected output overrides, is stored and reported
    /// without re-announcing anything.
    ///
    /// Every live `output-scale` scale ([`State::runtime_scales`]) is
    /// dropped here, file changed or not: a reload means "the file's
    /// scales", and a reload that fails to load never gets this far, so it
    /// keeps them. A dropped scale that differs from the file's is a scale
    /// the reload moved, so it reports as `outputs.<name>.scale` in
    /// `applied` like a changed entry (once per name, sorted, whether or
    /// not that monitor is connected now: a replug would have come back at
    /// it); one equal to the file's moved nothing and reports nothing.
    ///
    /// Under `--nested` every difference refuses instead: the host owns the
    /// scale there (`compositor::run` forced the live value to 1.0 with a
    /// warning and ignored every entry), so nothing is stored.
    fn apply_output_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        let nested = self.host.is_some();
        let default = scale_reload(fresh.scale, self.default_scale, nested);
        let entries = self.output_entries.diff(&fresh.outputs);
        report
            .refused
            .extend(output_refusals(&default, &entries, nested));
        if nested {
            return;
        }
        // Live IPC scales (`output-scale`) end here: a reload goes back to
        // the config file's scales, which is what `configured_scale` reads
        // once the map is empty. Each one the file disagrees with is a
        // move this reload makes, reported unless a changed entry for the
        // same name already reports it. Sorted, because the map's order is
        // not the caller's to see. A reload is a cold path; the `Vec` is
        // empty, and allocates nothing, without a live scale.
        let mut dropped: Vec<String> = self
            .runtime_scales
            .drain()
            .filter(|(name, scale)| {
                *scale != fresh.outputs.scale_for(name, fresh.scale)
                    && !entries.scales.contains(name)
            })
            .map(|(name, _)| name)
            .collect();
        dropped.sort_unstable();
        if default == ScaleReload::Agree && entries.scales.is_empty() && dropped.is_empty() {
            return;
        }
        self.default_scale = fresh.scale;
        self.output_entries = fresh.outputs.with_modes_of(&self.output_entries);
        if default == ScaleReload::Apply {
            report.applied.push(field::SCALE.to_owned());
        }
        for name in entries.scales.iter().chain(&dropped) {
            report.applied.push(output_field(name, "scale"));
        }
        let moved = self.outputs.iter().any(|output| {
            self.configured_scale(&output.name()) != super::output_scale::scale_of(output)
        });
        if moved {
            // Re-chooses the X scale too, once the new layout is in place
            // (see `xwayland/scale.rs`).
            self.rescale_outputs();
            self.resend_output_scale();
            report.rescaled = true;
        }
    }

    /// `[xwayland] fractional`: swapped live and re-applied through the one
    /// X-scale chooser (`State::refit_xwayland`), exactly like a scale
    /// change -- see the module doc. Compared against the live choice, and
    /// what is stored is exactly what was compared, so a second reload
    /// agrees silently. A reload that moves the X scale needs no `apply()`:
    /// the logical layout is untouched, and `refit_xwayland` reconfigures
    /// every managed X window itself.
    fn apply_xwayland_fractional_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        if let Some(name) = &fresh.invalid_xwayland_fractional {
            report.refused.push(format!(
                "{} (`{name}` is not one of sharp or light; kept {})",
                field::FRACTIONAL,
                self.xwayland_fractional.name()
            ));
            return;
        }
        if fresh.xwayland_fractional != self.xwayland_fractional {
            self.xwayland_fractional = fresh.xwayland_fractional;
            report.applied.push(field::FRACTIONAL.to_owned());
            #[cfg(feature = "xwayland")]
            self.refit_xwayland();
        }
    }

    /// The fields with no live state to compare against: `[tty] gpu`,
    /// `[renderer] backend` and `[xwayland] enabled`. Each diffs against the
    /// `startup_*` snapshot (or the fixed live value, where the session
    /// carries one) and refuses when it differs -- see the module doc for
    /// why none applies live.
    /// (`[output] scale` used to refuse here too; it applies live now,
    /// through `apply_output_reload` above. `[autostart]` runs its spawn
    /// delta instead, through `apply_autostart_reload` below.)
    fn apply_device_reload(&self, fresh: &LoadedConfig, report: &mut Report) {
        if fresh.gpu != self.startup_gpu {
            report.refused.push(refused(
                field::GPU,
                "takes effect on restart: the session already drives its device",
            ));
        }
        if fresh.renderer.is_some_and(|kind| kind != self.renderer) {
            report.refused.push(refused(
                field::BACKEND,
                "takes effect on restart: the live renderer holds client textures",
            ));
        }
        // The X server starts once at startup (or never), so flipping the
        // knob later cannot do what it says: starting one mid-session would
        // hand existing children a `DISPLAY` some of them already read as
        // absent, and stopping one would orphan every connected X client.
        // Compared against the request snapshot, not `xdisplay` liveness --
        // an enabled-but-crashed server still refuses, because "start one
        // now" is the same unstartable ask.
        if fresh.xwayland != self.startup_xwayland {
            report.refused.push(refused(
                field::XWAYLAND,
                "takes effect on restart: the X server starts once at startup (and needs an `xwayland` build)",
            ));
        }
        // The globals are advertised once at startup (or never), so
        // flipping the knob later cannot do what it says: advertising
        // mid-session would hand every connected client a new global
        // (and un-advertising would strand bound devices). Compared
        // against the request snapshot, like `xwayland` above.
        if fresh.virtual_input != self.startup_virtual_input {
            report.refused.push(refused(
                field::VIRTUAL_INPUT,
                "takes effect on restart: the virtual-pointer and virtual-keyboard globals are advertised once at startup",
            ));
        }
        // The bind gate is read on every virtual key, so it could flip
        // live -- but it refuses with the section instead: `binds` widens
        // the same trust boundary `enabled` opens (a bound keyboard can
        // spawn through binds), so the section stays one restart-only
        // contract, and a mid-hold flip can never strand a suppressed
        // virtual release (see `virtual_input.rs`). Compared against the
        // live value, which no reload ever writes.
        if fresh.virtual_input_binds != self.virtual_input_binds {
            report.refused.push(refused(
                field::VIRTUAL_INPUT_BINDS,
                "takes effect on restart: the virtual-keyboard bind gate is fixed at startup with the virtual-input globals",
            ));
        }
    }

    /// `[autostart] commands`: run-only-new-`Spawn`-entries, through the
    /// same `act` path startup drains (so the lock backstop, the spawn
    /// environment and the per-entry ordering apply unchanged), then advance
    /// the snapshot past the decided entries.
    ///
    /// Never a full re-drain -- entries the snapshot already holds stay
    /// silent -- and never a non-spawn action: a reloaded `quit` is refused
    /// by name rather than handed to `act`, which would end the session.
    /// A spawn the OS refuses stays pending rather than decided: it is
    /// refused by name (the cause is in the log), the snapshot does not
    /// advance past it, and the next reload retries it. Under lock nothing
    /// runs and the snapshot freezes (see the module doc): the delta defers
    /// to the first unlocked reload.
    fn apply_autostart_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        let delta = autostart_delta(&fresh.autostart, &self.startup_autostart);
        if self.session_lock.is_locked() {
            if !delta.run.is_empty() || !delta.refuse.is_empty() {
                report.refused.push(refused(
                    field::AUTOSTART,
                    "skipped while locked: pending entries are decided on the first unlocked reload",
                ));
            }
            return;
        }
        let mut accepted = Vec::new();
        let mut failed = Vec::new();
        for action in &delta.run {
            tracing::info!(?action, "running a new autostart entry from config reload");
            if self.act(action.clone()) {
                accepted.push(action.clone());
            } else {
                failed.push(action.clone());
            }
        }
        if !accepted.is_empty() {
            report.applied.push(field::AUTOSTART.to_owned());
        }
        for action in &failed {
            report.refused.push(refused(
                field::AUTOSTART,
                &format!("{action:?} failed to start; still pending, retried on the next reload"),
            ));
        }
        for action in &delta.refuse {
            report.refused.push(refused(
                field::AUTOSTART,
                &format!("{action:?} is not a spawn entry; only new spawn entries run on reload"),
            ));
        }
        // The snapshot becomes the fresh list minus the still-failing
        // spawns. Removals shrink it -- nothing is remembered past the file,
        // so a removed-then-re-added entry runs again -- decided entries
        // never re-report, and a failed spawn stays out so the next reload
        // sees it as unseen and retries it. Each failed occurrence removes
        // exactly one fresh occurrence, so duplicates keep the
        // per-occurrence accounting `autostart_delta` computes; a failed
        // entry always names a fresh occurrence (it came out of this
        // reload's own delta), so the search below always lands.
        let mut snapshot = fresh.autostart.clone();
        for action in &failed {
            if let Some(index) = snapshot.iter().position(|entry| entry == action) {
                snapshot.remove(index);
            }
        }
        self.startup_autostart = snapshot;
    }

    /// `[floating] auto` and `[[window_rule]]`: swapped in whole, and read
    /// by every window's first commit from then on (see `floating.rs`).
    /// Windows already mapped keep the layout they have -- rules decide at
    /// map time, never later -- so nothing is re-arranged. Each rule that
    /// was skipped as unusable is refused by name, on every reload that
    /// finds it: unlike a field that agrees with the session, a broken rule
    /// is never in effect, so saying so again is the only way the user
    /// learns it is still broken.
    fn apply_floating_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        let live = &self.floating_rules;
        let auto = fresh.floating.auto != live.auto;
        let rules = fresh.floating.rules != live.rules;
        if auto || rules {
            self.floating_rules = fresh.floating.clone();
            if auto {
                report.applied.push(field::FLOATING_AUTO.to_owned());
            }
            if rules {
                report.applied.push(field::WINDOW_RULES.to_owned());
            }
        }
        for skipped in &fresh.skipped_rules {
            report
                .refused
                .push(format!("{skipped}: skipped as unusable"));
        }
        // Read at each Mod+press, so the next one uses it; a drag already
        // under way is not ended by it. A value that names no modifier is
        // refused and the session keeps its own, like any refusal here.
        if let Some(name) = &fresh.invalid_floating_modifier {
            report.refused.push(format!(
                "{} (`{name}` is not one of super, alt, ctrl or shift; kept {})",
                field::FLOATING_MODIFIER,
                self.floating_modifier.name()
            ));
        } else if fresh.floating_modifier != self.floating_modifier {
            self.floating_modifier = fresh.floating_modifier;
            report.applied.push(field::FLOATING_MODIFIER.to_owned());
        }
    }

    /// `[wallpaper]`: handed to `scootbg apply-config` again on every reload
    /// while the section exists (an unchanged one is a no-op there, and
    /// brings back a daemon that crashed), `{}` on the reload that removes
    /// it (see `wallpaper.rs`). Reported applied when the section's values
    /// (`wallpaper`) or its `command` (`wallpaper.command`) differ from what
    /// the session last handed over -- "applied" meaning handed over: the
    /// run is spawned, never waited on, and its outcome is in the log. A
    /// section with a problem (an unknown key, a value that is not a
    /// string, a path that cannot be resolved) is refused by name on every
    /// reload that finds it, and the running one kept and re-run (see
    /// `State::reload_wallpaper`). Runs under lock too: the wallpaper is
    /// drawn on the background layer, which a locked frame does not show.
    fn apply_wallpaper_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        match self.reload_wallpaper(&fresh.wallpaper) {
            Reloaded::Unchanged => {}
            Reloaded::Applied { values, command } => {
                if values {
                    report.applied.push(field::WALLPAPER.to_owned());
                }
                if command {
                    report.applied.push(field::WALLPAPER_COMMAND.to_owned());
                }
            }
            Reloaded::Refused(problem) => {
                report.refused.push(refused(
                    field::WALLPAPER,
                    &format!("{problem}; kept the running section"),
                ));
            }
        }
    }

    /// `[binds]`: rebuilt from defaults plus the file (see
    /// [`keybindings_for`](super::config::keybindings_for)), swapped in
    /// whole. A key held across the swap neither wedges nor drops -- see
    /// the module doc.
    fn apply_binds_reload(&mut self, fresh: &LoadedConfig, report: &mut Report) {
        // The skipped list rides with the table: it describes the file this
        // table was built from, so a reload that only fixes a broken bind
        // (no effective row changes) still reports `binds` as applied -- the
        // `binds` reply changed, and saying otherwise would be silent.
        // Both lists are order-stable (`apply_binds` sorts the skipped, and
        // `same_bindings_as` compares order-insensitively), so an identical
        // reload still reports nothing.
        if !fresh.keybindings.same_bindings_as(&self.keybindings)
            || fresh.skipped_binds != self.skipped_binds
        {
            self.keybindings = fresh.keybindings.clone();
            self.skipped_binds = fresh.skipped_binds.clone();
            // The in-flight repeat's action and flags came from the old
            // table: it ends here rather than re-firing something the file
            // no longer binds. (The held key itself neither wedges nor
            // drops -- release routing never consults the table mid-hold,
            // see the module doc.)
            self.cancel_bind_repeat();
            report.applied.push(field::BINDS.to_owned());
        }
    }
}

/// A reload's applied-vs-refused lists, in reply order.
#[derive(Debug, Default)]
struct Report {
    applied: Vec<String>,
    refused: Vec<String>,
    /// Whether an output's scale actually moved, so the arrangement must be
    /// re-derived: not the same question as "a scale field applied", which
    /// can store a value no connected output runs at.
    rescaled: bool,
}

/// What a reload of `[output] scale` and `[[outputs]]` refuses, in reply
/// order: under `--nested` every difference (the host owns the one window's
/// size and scale); elsewhere only each changed entry `mode` (a reload does
/// not modeset -- see `output_config.rs`). Pure, so both halves pin without a
/// host connection, which no test harness can fake.
fn output_refusals(default: &ScaleReload, entries: &EntriesDiff, nested: bool) -> Vec<String> {
    let mut refusals = Vec::new();
    if nested {
        if *default == ScaleReload::RefuseNested {
            refusals.push(refused(
                field::SCALE,
                "refused under --nested: the host compositor owns the window's scale",
            ));
        }
        for (name, key) in entries
            .scales
            .iter()
            .map(|name| (name, "scale"))
            .chain(entries.modes.iter().map(|name| (name, "mode")))
        {
            refusals.push(refused(
                &output_field(name, key),
                "refused under --nested: the host compositor owns the window's size and scale",
            ));
        }
    } else {
        for name in &entries.modes {
            refusals.push(refused(
                &output_field(name, "mode"),
                "takes effect on restart: a reload does not modeset a running output; \
                 kept the mode the session started with",
            ));
        }
    }
    refusals
}

/// The reply name for one `[[outputs]]` entry's `key`: `outputs.DP-1.scale`.
fn output_field(name: &str, key: &str) -> String {
    format!("{}.{name}.{key}", field::OUTPUTS)
}

/// What a reloaded `[output] scale` (the session default) does: applies
/// live, agrees silently, or refuses under `--nested`.
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

/// What a reloaded `[autostart] commands` decides: the unseen entries split
/// by variant. Unseen means "absent from the snapshot", by value, counted
/// per occurrence -- see `autostart_delta`.
#[derive(Debug, Default, PartialEq, Eq)]
struct AutostartDelta {
    /// Unseen `Spawn` entries, in file order: the caller runs each through
    /// `act`, reports the field applied for what started, and refuses by
    /// name what the OS would not start (which stays pending for the next
    /// reload).
    run: Vec<Action>,
    /// Unseen non-`Spawn` entries, in file order: the caller refuses each by
    /// name. A reloaded `quit` lands here, never in `run` -- handing it to
    /// `act` would end the session.
    refuse: Vec<Action>,
}

/// Diffs a reloaded autostart list against the entries the session has
/// already decided (ran at startup, or ran/refused by an earlier unlocked
/// reload). Pure so the matrix pins without spawning: a multiset difference
/// by value, in file order -- an entry edited in place is a new entry
/// (there is no identity subtler than the action itself to key on), a
/// removed-then-re-added entry is new again (no memory past the last
/// snapshot), and duplicates count per occurrence (two identical entries run
/// twice at startup, so one seen plus two fresh is one run, not zero).
fn autostart_delta(fresh: &[Action], seen: &[Action]) -> AutostartDelta {
    let mut consumed = vec![false; seen.len()];
    let mut delta = AutostartDelta::default();
    for action in fresh {
        let prior = seen
            .iter()
            .enumerate()
            .find(|(index, seen)| !consumed[*index] && *seen == action);
        match prior {
            Some((index, _)) => consumed[index] = true,
            None if matches!(action, Action::Spawn(_)) => delta.run.push(action.clone()),
            None => delta.refuse.push(action.clone()),
        }
    }
    delta
}

fn refused(field: &str, reason: &str) -> String {
    format!("{field} ({reason})")
}
