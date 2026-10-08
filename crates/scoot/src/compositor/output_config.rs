//! `[[outputs]]`: a scale, a mode and a position per output, by output name.
//!
//! `[output] scale` (and `--mode`, and `--width`/`--height`) stay the session
//! default. An `[[outputs]]` entry names one output and overrides that
//! default for it alone, so every output without an entry -- and every output
//! of a config file with no entries -- is exactly what it was before this
//! existed.
//!
//! # Decisions, and why
//!
//! - **An entry matches by output name only**: the `wl_output.name` a client
//!   sees and `scoot msg outputs` lists (`eDP-1`, `DP-1`, `HDMI-A-1` under
//!   `--tty`; `headless`, `headless-2`, ... under `--headless`). There is no
//!   positional fallback. Output ids are never reused, so a monitor that is
//!   unplugged and plugged back in comes back under a new id and a new
//!   position in creation order, while its connector name does not move --
//!   measured on hardware (`Asahi.md` Tests 11-13: `DP-1` pulled and
//!   returned three ways, each time as `DP-1`, under a fresh output id). The
//!   position is exactly the key that is not stable across a reseat.
//! - **An entry beats the session-wide knob for its output**, including the
//!   `--mode`/`--width`/`--height` flags. Elsewhere in scoot a flag beats the
//!   file (`--gpu` over `[tty] gpu`, `--renderer` over `[renderer]
//!   backend`), but those name the same single thing twice; here the flag is
//!   the session default and the entry names one output specifically, and
//!   the specific wins -- otherwise a `--mode` meant for a projector would
//!   silently undo the entry for the laptop panel.
//! - **One `mode = "WxH"` key for both backends** rather than
//!   `width`/`height` plus `mode`: under `--tty` it picks the connector mode
//!   of that size (the same choice `--mode` makes, with the same warn-and-
//!   fall-back-to-preferred when the connector does not offer it); under
//!   `--headless` it is the virtual output's size. One spelling, parsed by
//!   the same function `--mode` uses, and nothing to disagree with itself.
//! - **A `position = [x, y]` in logical pixels per output** (either axis
//!   may be negative, to put a monitor left of or above the origin). An
//!   output without one packs left to right in creation order from the
//!   origin, stepping over placed outputs, so a placed output never shoves
//!   an earlier output aside and the primary stays at the origin (see
//!   `headless.rs`'s layout). Overlaps are allowed -- each output still
//!   renders its own strip -- and gaps are allowed too: the pointer clamp
//!   covers the bounding box of every output, so it can rest in a gap,
//!   exactly as it already can over uneven heights. A replugged monitor
//!   comes back at its entry's position while its neighbours stay where
//!   they are; the next full layout (a rescale, a removal, a mode change,
//!   or a reload that moves a position) restores the packed order behind
//!   it.
//! - **Mode and position take effect at startup, when a monitor is plugged
//!   in, and on reload.** A changed `mode` is a live resize: under
//!   `--headless` the output's render target is rebuilt at the new size
//!   through the same `resize_output_of` a host resize uses; under `--tty`
//!   the stored requests are swapped and the hotplug path's `NewMode` arm
//!   (`Head::retarget`) modesets the driven head, exactly as a re-probe
//!   after a replug would. A mode no connector offers keeps the running
//!   one with a warning, and the reload reply says so rather than reporting
//!   it applied. Scale and position reload live too.
//! - **`--nested` ignores every entry**, with a warning at startup and a
//!   refusal on reload: the host compositor owns the one window's size and
//!   scale there (the output is named `headless` too, so an entry for it
//!   would otherwise quietly apply).
//!
//! Parsed leniently, the way the rest of the file is (see `config.rs`'s
//! module doc): an entry with an unusable value costs that value only, with
//! a warning naming the entry. An unknown key inside an entry is an unknown
//! field like any other, and fails the whole file.

use serde::Deserialize;

use super::output_scale::{MAX_SCALE, MIN_SCALE, clamp_scale, clamp_scale_range};
use crate::cli::parse_mode;

#[cfg(test)]
mod tests;

/// One `[[outputs]]` table as the file spells it. `name` is required: an
/// entry with nothing to match is not an entry.
///
/// `position` is a `toml::Value`, not a fixed pair, so one entry's
/// mistyped position (a string, a single number, a triple) costs that
/// value only -- a warning naming the entry -- rather than failing the
/// whole file's parse the way a mistyped scalar would.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct OutputEntryConfig {
    pub(super) name: String,
    #[serde(default)]
    pub(super) scale: Option<f64>,
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) position: Option<toml::Value>,
}

/// One resolved `[[outputs]]` entry: its scale already clamped and resolved
/// to 120ths exactly as `[output] scale` is (see `output_scale::clamp_scale`),
/// its mode already parsed, its position already validated. `None` in any
/// means "the session default" (the default scale, the default mode, packed
/// after the placed outputs).
#[derive(Clone, Debug, PartialEq)]
pub struct OutputEntry {
    pub name: String,
    pub scale: Option<f64>,
    pub mode: Option<(u16, u16)>,
    pub position: Option<(i32, i32)>,
}

/// Every usable `[[outputs]]` entry, in file order, at most one per name.
/// Empty for a file with none -- the case that must behave exactly like
/// the session before this existed.
///
/// Looked up linearly by name. Every lookup is on a cold path (creating an
/// output, a reload, a hotplug re-probe), never per frame or per event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputEntries(Vec<OutputEntry>);

impl OutputEntries {
    /// Resolves the file's entries, warning about (and dropping) what cannot
    /// be used: an empty `name`, a second entry for a name already seen (the
    /// first wins, as the first of two identical binds would), and an entry
    /// left with nothing to set. A `scale` that is not a finite number is
    /// dropped from its entry -- the output keeps the session default -- and
    /// one out of range is clamped, both with a warning; a `mode` that is not
    /// `WxH` is dropped from its entry with a warning; a `position` that is
    /// not two integers is dropped from its entry with a warning.
    pub(super) fn resolve(raw: Vec<OutputEntryConfig>) -> Self {
        let mut entries: Vec<OutputEntry> = Vec::with_capacity(raw.len());
        for (index, entry) in raw.into_iter().enumerate() {
            // 1-based, the way a person counts entries in a file.
            let position = index + 1;
            if entry.name.is_empty() {
                tracing::warn!(
                    entry = position,
                    "skipping an [[outputs]] entry with an empty name"
                );
                continue;
            }
            if entries.iter().any(|seen| seen.name == entry.name) {
                tracing::warn!(
                    entry = position,
                    output = %entry.name,
                    "skipping a second [[outputs]] entry for the same output; the first one wins"
                );
                continue;
            }
            let scale = entry
                .scale
                .and_then(|scale| resolve_scale(&entry.name, scale));
            let mode = entry.mode.as_deref().and_then(|raw| {
                let parsed = parse_mode(raw);
                if parsed.is_none() {
                    tracing::warn!(
                        output = %entry.name,
                        value = %raw,
                        "an [[outputs]] mode is not WxH (like 1920x1080); ignoring it"
                    );
                }
                parsed
            });
            let placed = entry
                .position
                .as_ref()
                .and_then(|value| resolve_position(&entry.name, value));
            if scale.is_none() && mode.is_none() && placed.is_none() {
                tracing::warn!(
                    entry = position,
                    output = %entry.name,
                    "an [[outputs]] entry sets neither scale nor mode nor position; ignoring it"
                );
                continue;
            }
            entries.push(OutputEntry {
                name: entry.name,
                scale,
                mode,
                position: placed,
            });
        }
        Self(entries)
    }

    /// The entries a config file holding just `text` (`[[outputs]]` tables)
    /// resolves to -- for suites that need a session with entries in force
    /// without writing a whole file.
    #[cfg(test)]
    pub(super) fn from_toml(text: &str) -> Self {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            outputs: Vec<OutputEntryConfig>,
        }
        let file: File = toml::from_str(text).expect("valid [[outputs]] toml");
        Self::resolve(file.outputs)
    }

    /// The entries, in file order.
    pub fn iter(&self) -> impl Iterator<Item = &OutputEntry> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn get(&self, name: &str) -> Option<&OutputEntry> {
        self.0.iter().find(|entry| entry.name == name)
    }

    /// The scale output `name` runs at: its entry's, else `default` (the
    /// resolved `[output] scale`).
    pub fn scale_for(&self, name: &str, default: f64) -> f64 {
        self.get(name)
            .and_then(|entry| entry.scale)
            .unwrap_or(default)
    }

    /// The mode size output `name` asks for: its entry's, else `None` (the
    /// session default -- `--mode` under `--tty`, `--width`/`--height` under
    /// `--headless`).
    pub fn mode_for(&self, name: &str) -> Option<(u16, u16)> {
        self.get(name).and_then(|entry| entry.mode)
    }

    /// Where output `name` is placed: its entry's `position`, else `None`
    /// (packed after the placed outputs -- see `headless.rs`'s layout).
    pub fn position_for(&self, name: &str) -> Option<(i32, i32)> {
        self.get(name).and_then(|entry| entry.position)
    }

    /// What a reload of `fresh` over these (the live) entries changes, per
    /// output name, in the order the names first appear (live entries
    /// first, then fresh ones): the names whose entry `scale` differs, those
    /// whose entry `mode` differs, and those whose entry `position` differs.
    /// An entry added or removed counts as a difference in each field it
    /// sets. Pure, and on the reload path only (it allocates the small
    /// lists).
    pub(super) fn diff(&self, fresh: &Self) -> EntriesDiff {
        let mut diff = EntriesDiff::default();
        let mut seen: Vec<&str> = Vec::with_capacity(self.0.len() + fresh.0.len());
        for entry in self.0.iter().chain(fresh.0.iter()) {
            let name = entry.name.as_str();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            let live = self.get(name);
            let new = fresh.get(name);
            if live.and_then(|entry| entry.scale) != new.and_then(|entry| entry.scale) {
                diff.scales.push(name.to_owned());
            }
            if live.and_then(|entry| entry.mode) != new.and_then(|entry| entry.mode) {
                diff.modes.push(name.to_owned());
            }
            if live.and_then(|entry| entry.position) != new.and_then(|entry| entry.position) {
                diff.positions.push(name.to_owned());
            }
        }
        diff
    }
}

/// What [`OutputEntries::diff`] found: the output names whose entry scale
/// changed, those whose entry mode changed, and those whose entry position
/// changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct EntriesDiff {
    pub(super) scales: Vec<String>,
    pub(super) modes: Vec<String>,
    pub(super) positions: Vec<String>,
}

/// What `--tty` asks each connector for: `--mode` as the default, and every
/// `[[outputs]]` entry's `mode` for its own connector. The default is fixed
/// for the session (it comes from a flag, which no reload re-reads); the
/// per-output half follows the live entries -- a reload swaps in the fresh
/// list and re-probes, so a hotplug, a VT switch back or an unrelated uevent
/// re-runs exactly the choice the last reload made instead of quietly
/// re-modesetting a connector back to `--mode` or its preferred mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeRequests {
    default: Option<(u16, u16)>,
    per_output: Vec<(String, (u16, u16))>,
}

impl ModeRequests {
    pub fn new(default: Option<(u16, u16)>, entries: &OutputEntries) -> Self {
        Self {
            default,
            per_output: entries
                .iter()
                .filter_map(|entry| entry.mode.map(|mode| (entry.name.clone(), mode)))
                .collect(),
        }
    }

    /// Swaps the per-output half for `entries`' (a reload's fresh list),
    /// keeping the session default: what `--tty` re-probes with after a
    /// reload changed a mode.
    pub fn update(&mut self, entries: &OutputEntries) {
        self.per_output = entries
            .iter()
            .filter_map(|entry| entry.mode.map(|mode| (entry.name.clone(), mode)))
            .collect();
    }

    /// The size connector `name` should be driven at, if one was asked for.
    pub fn for_output(&self, name: &str) -> Option<(u16, u16)> {
        self.per_output
            .iter()
            .find(|(output, _)| output == name)
            .map(|(_, mode)| *mode)
            .or(self.default)
    }
}

/// An entry's `position`, validated: exactly two integers, each fitting
/// an `i32` (TOML integers are `i64`; anything wider cannot be a logical
/// coordinate). Either axis may be negative, to place a monitor left of or
/// above the origin. Anything else -- a string, a float, a lone number, a
/// triple -- is dropped (`None`: the output packs after the placed ones)
/// with a warning naming the entry, costing that value only.
fn resolve_position(name: &str, value: &toml::Value) -> Option<(i32, i32)> {
    let pair = match value {
        toml::Value::Array(pair) if pair.len() == 2 => pair,
        _ => {
            tracing::warn!(
                output = %name,
                value = %value,
                "an [[outputs]] position is not [x, y] (two integers, like [-1920, 0]); this output packs after the placed ones"
            );
            return None;
        }
    };
    let mut axes = [0i32; 2];
    for (index, value) in pair.iter().enumerate() {
        let Some(axis) = value.as_integer().and_then(|axis| i32::try_from(axis).ok()) else {
            tracing::warn!(
                output = %name,
                value = %value,
                "an [[outputs]] position is not [x, y] (two integers, like [-1920, 0]); this output packs after the placed ones"
            );
            return None;
        };
        axes[index] = axis;
    }
    Some((axes[0], axes[1]))
}

/// An entry's `scale`, resolved the way `[output] scale` is -- clamped into
/// range with a warning, then to 120ths silently -- except that a value that
/// is not a finite number is dropped (`None`: the output keeps the session
/// default) rather than turned into 1.0, since an entry has a default to
/// fall back to that means more than 1.0 does.
fn resolve_scale(name: &str, scale: f64) -> Option<f64> {
    if !scale.is_finite() {
        tracing::warn!(
            output = %name,
            configured = scale,
            "an [[outputs]] scale is not a finite number; this output keeps the [output] scale"
        );
        return None;
    }
    let resolved = clamp_scale(scale);
    if clamp_scale_range(scale) != scale {
        tracing::warn!(
            output = %name,
            configured = scale,
            resolved,
            min = MIN_SCALE,
            max = MAX_SCALE,
            "an [[outputs]] scale is out of range; clamping"
        );
    }
    Some(resolved)
}
