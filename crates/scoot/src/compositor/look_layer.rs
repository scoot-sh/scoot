//! The look layer's state: where the active rendered look lives, and the
//! identity that tells a reload whether the layer changed.
//!
//! The merge itself (raw keys, before derivation, by the three rules in the
//! design's section 4.3) lives in [`crate::compositor::config`], beside the
//! `AppearanceConfig` and `WallpaperConfig` types it merges: parsing either
//! file into a `toml::Table` first would reintroduce the stack-overflow trap
//! `config.rs`'s module doc measures (a deeply nested file deserialized into
//! a passthrough table costs 32 MiB of stack in a debug build), so both
//! files parse into their bounded structs and the merge runs on the
//! pre-derivation `Option` values instead. This module owns the state dir,
//! the fragment bytes, and the identity.
//!
//! `<state>` resolution is an explicit dir (the future `--state` flag),
//! then `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`, then
//! `~/.local/state/scoot`. Only a private session exports
//! `SCOOT_LOOK_STATE` to its children; `--tty` and `--nested` use the user
//! path so the choice persists, and the per-display private states arrive
//! with `look-session-state`. Until then this module reads the user path.
//!
//! A missing link is "no look layer" without a warning (it is the default
//! state for every install that never picked a look); a dangling link or an
//! unreadable fragment is "no look layer" plus the warning the caller logs
//! at startup, and refuses a reload whole (everything kept). Nothing here
//! needs Nix: a plain binary install reads the same plain files.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The fragment filename inside the active rendered look.
pub const FRAGMENT: &str = "scoot.toml";
/// The identity filename inside the active rendered look.
pub const RENDER_JSON: &str = "render.json";

/// The active rendered look's fragment bytes plus its identity.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The raw `scoot.toml` bytes. `config` parses and strictly validates
    /// these; a refusal names the key and value.
    pub text: String,
    /// Who this render is: the name and hash from its `render.json`.
    pub identity: LookIdentity,
}

/// The layer's identity: what a reload compares to decide the layer
/// changed, and what `EventKind::Look` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookIdentity {
    /// The rendered look's name (`None` when `render.json` is missing or
    /// does not name one).
    pub name: Option<String>,
    /// The rendered look's hash from its `render.json` (empty when there is
    /// no `render.json`, in which case the fragment's own length stands in
    /// so two different fragments without metadata still compare different).
    pub source_hash: String,
}

/// How the fragment load went.
#[derive(Debug)]
pub enum FragmentLoad {
    /// The fragment bytes to parse and strictly validate.
    Present(Fragment),
    /// No layer: an absent link (silent), or a problem (warned about).
    Absent(FragmentAbsent),
}

/// Why there is no fragment to merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentAbsent {
    /// No link at all: the default state, not worth a warning.
    Missing,
    /// A dangling link or an unreadable file: warned about, with
    /// [`LoadOutcome::warn`]'s text.
    Broken,
}

/// The outcome of loading the fragment from a state dir, with the log line
/// the caller emits for it.
#[derive(Debug)]
pub struct LoadOutcome {
    pub load: FragmentLoad,
    /// Set when the caller should log a warning (a broken link or an
    /// unreadable fragment, naming the reason).
    pub warn: Option<String>,
}

/// Resolves the look state dir: an explicit dir (the future `--state` flag),
/// then `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`, then
/// `~/.local/state/scoot`. Pure over its env vars like
/// [`crate::compositor::config::default_path`], for the same testability
/// reason. `None` only when no home resolves at all.
pub fn state_dir(
    explicit: Option<&Path>,
    scoot_look_state: Option<OsString>,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(dir.to_owned());
    }
    let non_empty = |v: &OsString| !v.is_empty();
    if let Some(dir) = scoot_look_state.filter(non_empty) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = xdg_state_home.filter(non_empty) {
        return Some(PathBuf::from(dir).join("scoot"));
    }
    home.filter(non_empty)
        .map(|dir| PathBuf::from(dir).join(".local/state/scoot"))
}

/// Loads the active rendered look's fragment from `state_dir`.
///
/// Returns the fragment bytes plus the identity from its `render.json`, or
/// no layer with the warning the caller logs (startup) or refuses with
/// (reload). Never panics on malformed input: every I/O failure maps to
/// [`FragmentAbsent::Broken`]. Parsing and strict validation happen in
/// `config`, which owns the target types.
pub fn load(state_dir: &Path) -> LoadOutcome {
    let link = state_dir.join("look");
    if std::fs::symlink_metadata(&link).is_err() {
        return LoadOutcome {
            load: FragmentLoad::Absent(FragmentAbsent::Missing),
            warn: None,
        };
    }
    let fragment_path = link.join(FRAGMENT);
    let text = match std::fs::read_to_string(&fragment_path) {
        Ok(text) => text,
        Err(error) => {
            return LoadOutcome {
                load: FragmentLoad::Absent(FragmentAbsent::Broken),
                warn: Some(format!(
                    "the look layer at {} cannot be read ({error}); continuing without it",
                    fragment_path.display(),
                )),
            };
        }
    };
    LoadOutcome {
        load: FragmentLoad::Present(Fragment {
            identity: read_identity(&link.join(RENDER_JSON), &text),
            text,
        }),
        warn: None,
    }
}

/// Reads the layer's identity: the name and hash from its `render.json`,
/// falling back to the fragment's own length when there is none.
fn read_identity(render_json: &Path, fragment_text: &str) -> LookIdentity {
    #[derive(serde::Deserialize, Default)]
    struct RenderJson {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        source_hash: String,
    }
    match std::fs::read_to_string(render_json) {
        Ok(text) => match serde_json::from_str::<RenderJson>(&text) {
            Ok(parsed) => LookIdentity {
                name: parsed.name,
                source_hash: parsed.source_hash,
            },
            Err(_) => LookIdentity {
                name: None,
                source_hash: fragment_fallback(fragment_text),
            },
        },
        Err(_) => LookIdentity {
            name: None,
            source_hash: fragment_fallback(fragment_text),
        },
    }
}

/// The identity stand-in when there is no `render.json`: two different
/// fragments still compare different by length in every non-adversarial
/// case, and equal-length collisions only skip an event, never corrupt
/// state.
fn fragment_fallback(fragment_text: &str) -> String {
    format!("fragment:{}", fragment_text.len())
}

#[cfg(test)]
mod tests;
