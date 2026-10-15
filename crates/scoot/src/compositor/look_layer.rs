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
//! `~/.local/state/scoot` (the one rule in `scoot-look`'s `state` module,
//! shared with scootbar; scootbg takes no look state). Only a private
//! session exports
//! `SCOOT_LOOK_STATE` to its children; `--tty` and `--nested` use the user
//! path so the choice persists, and `--headless` (or any backend with
//! `--private-look`) seeds a per-display private state at startup.
//!
//! A missing link is "no look layer" without a warning (it is the default
//! state for every install that never picked a look); a dangling link or an
//! unreadable fragment is "no look layer" plus the warning the caller logs
//! at startup, and refuses a reload whole (everything kept). Nothing here
//! needs Nix: a plain binary install reads the same plain files.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The fragment filename inside the active rendered look.
pub const FRAGMENT: &str = "scoot.toml";
/// The identity filename inside the active rendered look.
pub const RENDER_JSON: &str = "render.json";

/// The private state dir for a display: `$XDG_RUNTIME_DIR/scoot-look-<display>/`.
/// Pure over its inputs, like [`state_dir`]. Refuses a missing runtime dir
/// or a display outside the socket-name grammar (naming which), so the
/// caller falls back to the user state loudly rather than sharing it
/// silently or writing outside the runtime dir.
pub fn private_state_dir(runtime_dir: Option<&OsStr>, display: &OsStr) -> Result<PathBuf, String> {
    let Some(dir) = runtime_dir.filter(|dir| !dir.is_empty()) else {
        return Err(
            "$XDG_RUNTIME_DIR is not set: no private look state without a runtime dir".to_owned(),
        );
    };
    let name = scoot_look::state::private_dir_name(display)?;
    Ok(PathBuf::from(dir).join(name))
}

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
/// `~/.local/state/scoot`. The rule lives once in `scoot-look`'s
/// [`scoot_look::state`] (shared with scootbar; scootbg takes no look
/// state); this is the compositor's call site, not a second copy. Pure over its env vars like
/// [`crate::compositor::config::default_path`], for the same testability
/// reason. `None` only when no home resolves at all.
pub fn state_dir(
    explicit: Option<&Path>,
    scoot_look_state: Option<OsString>,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    scoot_look::state::resolve_state_dir(
        explicit,
        scoot_look_state.as_deref(),
        xdg_state_home.as_deref(),
        home.as_deref(),
    )
}

/// The largest look-layer file this reads: a rendered `scoot.toml` fragment
/// and its `render.json` are both machine-rendered and small (a few KiB), so
/// anything past `scoot-look`'s cap is not a look layer. The gated read both
/// files share is owned by `scoot-look`'s [`scoot_look::state`] (the same
/// read the session record goes through); a symlink is followed and judged
/// by its target -- which is how the `look` link itself resolves, the one
/// symlink this intentionally follows; a loop in one is the kernel's `ELOOP`.
fn read_capped(path: &Path) -> Result<String, String> {
    scoot_look::state::read_capped(path).map_err(|error| error.to_string())
}

/// Loads the active rendered look's fragment from `state_dir`.
///
/// Returns the fragment bytes plus the identity from its `render.json`, or
/// no layer with the warning the caller logs (startup) or refuses with
/// (reload). Never panics on malformed input: every I/O failure maps to
/// [`FragmentAbsent::Broken`]. Parsing and strict validation happen in
/// `config`, which owns the target types.
///
/// Both files go through [`read_capped`]: a FIFO at the fragment path
/// refuses instead of hanging the compositor, and an oversize file is
/// refused without reading it whole -- on the reload path as well, so a
/// hostile layer can never wedge the loop thread.
pub fn load(state_dir: &Path) -> LoadOutcome {
    let link = state_dir.join("look");
    if std::fs::symlink_metadata(&link).is_err() {
        return LoadOutcome {
            load: FragmentLoad::Absent(FragmentAbsent::Missing),
            warn: None,
        };
    }
    let fragment_path = link.join(FRAGMENT);
    let text = match read_capped(&fragment_path) {
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
/// falling back to the fragment's content hash when there is none (or it
/// cannot be read: a FIFO, a directory, an oversize file, or bad JSON all
/// fall back rather than hanging or refusing the layer).
fn read_identity(render_json: &Path, fragment_text: &str) -> LookIdentity {
    #[derive(serde::Deserialize, Default)]
    struct RenderJson {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        source_hash: String,
    }
    match read_capped(render_json) {
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

/// The identity stand-in when there is no `render.json`: a cheap FNV-1a
/// hash of the fragment bytes, so two different fragments without metadata
/// still compare different. Length alone collides for any two equal-length
/// fragments (which only skips a `Look` event, never corrupts state), but
/// the hash costs one pass and removes even that.
fn fragment_fallback(fragment_text: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in fragment_text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("fragment:{hash:016x}")
}

#[cfg(test)]
mod tests;
