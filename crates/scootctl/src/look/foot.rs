//! The foot leaf after a swap: notes only, never a signal or a refusal.
//!
//! Split out of `apply.rs` so that file shrinks instead of growing:
//! the one-time-setup note and the include scan live here.

use std::path::Path;

use super::catalog::config_home;
use scoot_look::state::read_capped_follow;

/// The foot leaf after a swap: new terminals read the rendered `foot.ini`
/// through the state link, so the reply names the one-time setup until the
/// user's `foot.ini` includes it, and says running terminals keep their
/// palette either way.
///
/// No signal is ever sent: foot never re-reads its config, and `SIGUSR1` /
/// `SIGUSR2` only switch between the two palettes loaded at foot start —
/// after a swap those are the old colors, so signaling would flip running
/// terminals stale instead of applying the new look. A live OSC push into
/// running ptys stays research-only: it can interleave with a program's own
/// output. Best-effort notes only, never a refusal.
pub(crate) fn foot_notes(
    merged: &scoot_look::ResolvedLook,
    state_dir: &Path,
    config: Option<&Path>,
) -> Vec<String> {
    if merged.terminal.is_none() {
        return Vec::new();
    }
    if foot_include_present(config) {
        return vec![
            "foot: new terminals read the look's colors; running terminals keep their palette until restart".to_owned(),
        ];
    }
    vec![format!(
        "foot: new terminals follow the look once foot.ini includes `{}`; running terminals keep their palette until restart",
        state_dir.join("look/foot.ini").display()
    )]
}

/// Whether the user's `foot.ini` already includes a look fragment: any
/// uncommented `include` line naming `look/foot.ini`. A missing or
/// unreadable file counts as absent: the setup hint covers creating it.
fn foot_include_present(config: Option<&Path>) -> bool {
    let owned;
    let base = match config {
        Some(dir) => Some(dir),
        None => {
            let xdg = std::env::var_os("XDG_CONFIG_HOME");
            let home = std::env::var_os("HOME");
            owned = config_home(xdg.as_deref(), home.as_deref());
            owned.as_deref()
        }
    };
    let Some(path) = base.map(|dir| dir.join("foot/foot.ini")) else {
        return false;
    };
    // The user's own file: home-manager links it into the store, so
    // symlinks are followed here, like the shadowed-keys read.
    let text = match read_capped_follow(&path) {
        Ok(text) => text,
        Err(_) => return false,
    };
    text.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            return false;
        }
        let Some(rest) = line.strip_prefix("include") else {
            return false;
        };
        // `include`, `include =`, `include=` all spell the directive; a
        // longer word (`included`) is not it.
        if !rest.is_empty() && !rest.starts_with(['=', ' ', '\t']) {
            return false;
        }
        rest.contains("look/foot.ini")
    })
}
