//! The one-key toolkit write: `org.gnome.desktop.interface color-scheme`
//! from the look's polarity (design D11, ticket R2/N3).
//!
//! Without it a live swap across polarity leaves dark/light stale until the
//! next activation, because the full toolkit set lands later (in
//! `look-toolkit-live`). The rules, both layers of "never overwrite a value
//! someone else set":
//!
//! - Skipped entirely with `apply --no-toolkit`, or persistently after
//!   `scoot look toolkit off` (kept in `look.json` as `toolkit = false`).
//! - Before writing, the tool reads the live value. Unset, or equal to what
//!   the tool last wrote (kept in `look.json` as `color-scheme`), means the
//!   tool owns the key: write the look's value and record it.
//! - Anything else means someone else changed it (a settings app, a hand
//!   `gsettings set`): leave it and name it in the reply.
//! - The one exception is the first write with no record yet: the tool
//!   adopts the key, whatever the live value is. On a non-Nix box there is
//!   no Nix key-name list, so the first `apply` overwrites a `color-scheme`
//!   the user chose in a settings app — the opt-outs above ship with it,
//!   and the docs say so where the first apply is described.
//!
//! A box without `dconf` gets a note, not a refusal. Every step here is
//! best-effort and reported; a toolkit failure never refuses a swap.

use std::path::Path;

use scoot_look::state::Record;

/// The dconf key the tool owns one value of.
pub const COLOR_SCHEME_KEY: &str = "/org/gnome/desktop/interface/color-scheme";

/// The value for a dark look, as `dconf write` takes it.
pub const PREFER_DARK: &str = "'prefer-dark'";
/// The value for a light look, as `dconf write` takes it.
pub const PREFER_LIGHT: &str = "'prefer-light'";

/// Whether the record says the toolkit writes are on. Absent means on:
/// only an explicit `toolkit off` skips every dconf write.
pub fn toolkit_on(record: &Record) -> bool {
    record.toolkit.unwrap_or(true)
}

/// The `color-scheme` value for a polarity: dark looks read dark.
pub fn want_for_polarity(dark: bool) -> &'static str {
    if dark { PREFER_DARK } else { PREFER_LIGHT }
}

/// One dconf read and write, so tests script the settings backend.
pub trait Dconf {
    /// The live value, trimmed, or `None` when unset or unreadable.
    /// Unreadable reads as unset: the write below either succeeds (and the
    /// record starts) or fails loudly itself.
    fn read(&self, key: &str) -> Option<String>;
    /// Writes `value`, exactly as given.
    fn write(&self, key: &str, value: &str) -> Result<(), String>;
    /// Resets `key` to unset (what it was before the tool's first write).
    fn reset(&self, key: &str) -> Result<(), String>;
    /// Whether the backend exists at all. A missing `dconf` is a note.
    fn available(&self) -> bool;
}

/// The real backend: the `dconf` CLI on `PATH`.
pub struct CliDconf {
    program: Option<std::path::PathBuf>,
}

impl CliDconf {
    pub fn new() -> Self {
        Self {
            program: find_on_path("dconf"),
        }
    }
}

impl Default for CliDconf {
    fn default() -> Self {
        Self::new()
    }
}

impl Dconf for CliDconf {
    fn available(&self) -> bool {
        self.program.is_some()
    }

    fn read(&self, key: &str) -> Option<String> {
        let program = self.program.as_ref()?;
        let out = super::apply::run_with_timeout(program, &["read", key]).ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        let trimmed = text.trim().to_owned();
        // `dconf read` prints nothing for an unset key.
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| "no `dconf` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["write", key, value])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("`dconf write {key}` failed"))
        }
    }

    fn reset(&self, key: &str) -> Result<(), String> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| "no `dconf` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["reset", key])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("`dconf reset {key}` failed"))
        }
    }
}

/// Finds `name` on `PATH`: each directory joined with the name, first
/// regular file wins. Symlinks resolve at exec time, which is what `PATH`
/// lookup means everywhere else.
pub fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| {
                std::fs::symlink_metadata(candidate)
                    .map(|meta| meta.is_file() || meta.is_symlink())
                    .unwrap_or(false)
            })
    })
}

/// Whether `state_dir` is a private look state (a headless or
/// `--private-look` session's): its leaf is `scoot-look-<display>`, or it
/// is the `$SCOOT_LOOK_STATE` dir. Toolkit writes never touch the user's
/// real dconf from one unless explicitly asked (an explicit `--state`
/// pointing at the user state).
pub fn is_private_state(state_dir: &Path) -> bool {
    use std::path::Path as P;
    let leaf = state_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if leaf.starts_with(scoot_look::state::PRIVATE_PREFIX) {
        return true;
    }
    if let Some(session) = std::env::var_os(scoot_look::state::STATE_ENV)
        && !session.is_empty()
        && P::new(&session) == state_dir
    {
        return true;
    }
    false
}

/// Applies the toolkit step for a swap: writes the look's `color-scheme`
/// under the D11 rules, recording what was written in `record`. Returns the
/// notes for the reply. Never fails the swap: a missing backend or a failed
/// write is a note.
/// The original live value is saved once (before the first write) into
/// `record.color_scheme_orig` (`Some("")` means it was unset), so
/// `toolkit off` can restore it. A private session never writes: its swaps
/// must not move the user's real desktop.
pub fn apply_toolkit(
    state_dir: &Path,
    record: &mut Record,
    dark: bool,
    dconf: &dyn Dconf,
) -> Vec<String> {
    let mut notes = Vec::new();
    if !toolkit_on(record) {
        notes.push(
            "toolkit writes off (`scoot look toolkit off`): color-scheme left alone".to_owned(),
        );
        return notes;
    }
    if is_private_state(state_dir) {
        notes.push(
            "private look state (headless or --private-look): color-scheme left alone"
                .to_owned(),
        );
        return notes;
    }
    if !dconf.available() {
        notes.push("no `dconf` on PATH: color-scheme left alone".to_owned());
        return notes;
    }
    let want = want_for_polarity(dark);
    // Save the original once, before the first write, so `toolkit off`
    // can restore it. `Some("")` means it was unset.
    if record.color_scheme_orig.is_none() && record.color_scheme.is_none() {
        let orig = dconf.read(COLOR_SCHEME_KEY).unwrap_or_default();
        record.color_scheme_orig = Some(orig);
    }
    match dconf.read(COLOR_SCHEME_KEY) {
        // Unset, or what the tool last wrote: the tool owns the key.
        None => write_and_record(record, want, dconf, &mut notes),
        Some(live) if Some(live.as_str()) == record.color_scheme.as_deref() => {
            write_and_record(record, want, dconf, &mut notes)
        }
        // No record yet: adopt the key (the documented first-apply
        // exception), whatever the live value is. The original is saved
        // above, so `toolkit off` restores it.
        Some(_) if record.color_scheme.is_none() => {
            notes.push(
                "color-scheme had no scoot record: adopting it (use `apply --no-toolkit` to opt out)".to_owned(),
            );
            write_and_record(record, want, dconf, &mut notes);
        }
        Some(live) => {
            notes.push(format!(
                "color-scheme is {live} (set outside `scoot look`): left alone"
            ));
        }
    }
    notes
}

/// Writes `want` and records it; a failed write is a note naming the key.
fn write_and_record(record: &mut Record, want: &str, dconf: &dyn Dconf, notes: &mut Vec<String>) {
    match dconf.write(COLOR_SCHEME_KEY, want) {
        Ok(()) => {
            record.color_scheme = Some(want.to_owned());
            notes.push(format!("color-scheme set to {want}"));
        }
        Err(error) => {
            notes.push(format!("color-scheme not written: {error}"));
        }
    }
}

#[cfg(test)]
mod tests;
