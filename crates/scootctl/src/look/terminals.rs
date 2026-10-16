//! The terminal static-leaf step of a swap: `SIGUSR1` to the user's own
//! `hx` and `SIGUSR2` to the user's own `btop`, each opt-out, plus the
//! one-time-setup notes (design section 4.7, ticket
//! `look-leaves-terminal`).
//!
//! The fragments these signals activate live in the rendered look
//! (`apps/helix.toml`, `apps/btop.theme`, `apps/starship.toml`): static
//! leaves copied from the look, trusted looks only. Helix 25.07.1
//! re-reads its config and theme from disk on `SIGUSR1`, btop 1.4.7 its
//! config and theme on `SIGUSR2` (both verified against the pinned
//! sources and live against the real binaries); starship is a new
//! process per prompt, so the next prompt picks the leaf up with no
//! signal at all.
//!
//! Every step here is best-effort and reported, like the daemon leaves:
//! a missing binary, no running process, or a failed signal is a note,
//! never a refusal. A private look state (a headless or `--private-look`
//! session) runs none of the signals: like the daemon signals they always
//! reach the ambient session, so running them from a test would restyle
//! the user's real editors.

use std::path::Path;

use super::leaves::{LeafReport, Leaves};

/// Which static leaves the rendered look carries. A look without an
/// `apps/` file renders no leaf for it, and the step stays silent about
/// that app: the leaf was never the look's to give.
#[derive(Debug, Default, PartialEq)]
pub struct TerminalPresence {
    pub helix: bool,
    pub btop: bool,
    pub starship: bool,
}

/// Reads the presence out of a rendered look directory.
pub fn presence_in(rendered: &Path) -> TerminalPresence {
    TerminalPresence {
        helix: rendered.join("apps/helix.toml").is_file(),
        btop: rendered.join("apps/btop.theme").is_file(),
        starship: rendered.join("apps/starship.toml").is_file(),
    }
}

/// Runs the terminal steps for a swap over the rendered `rendered` look.
/// Returns who reloaded and the notes. Never fails the swap.
pub fn apply_terminals(
    state_dir: &Path,
    rendered: &Path,
    leaves: &dyn Leaves,
    no_helix: bool,
    no_btop: bool,
) -> LeafReport {
    let mut report = LeafReport::default();
    if super::toolkit::is_private_state(state_dir) {
        report.notes.push(
            "private look state (headless or --private-look): terminal leaves left alone"
                .to_owned(),
        );
        return report;
    }
    let presence = presence_in(rendered);
    if presence.helix {
        if no_helix {
            report
                .notes
                .push("helix left alone for this swap (--no-helix)".to_owned());
        } else {
            match leaves.helix_signal() {
                Ok(0) => report
                    .notes
                    .push("helix not reloaded: no hx running".to_owned()),
                Ok(_) => report.reloaded.push("helix".to_owned()),
                Err(error) => report.notes.push(format!("helix not reloaded: {error}")),
            }
        }
        report.notes.push(helix_setup_note(state_dir));
    }
    if presence.btop {
        if no_btop {
            report
                .notes
                .push("btop left alone for this swap (--no-btop)".to_owned());
        } else {
            match leaves.btop_signal() {
                Ok(0) => report
                    .notes
                    .push("btop not reloaded: no btop running".to_owned()),
                Ok(_) => report.reloaded.push("btop".to_owned()),
                Err(error) => report.notes.push(format!("btop not reloaded: {error}")),
            }
        }
        report.notes.push(btop_setup_note(state_dir));
    }
    if presence.starship {
        report.notes.push(starship_setup_note(state_dir));
    }
    report
}

/// The one-time Helix wiring: a fixed theme name (`scoot-look`, the way
/// the fcitx5 leaf takes a fixed name) whose file follows the state link,
/// so a swap rewrites no file per look. Running `hx` re-reads it on the
/// signal above; new ones read it at start.
fn helix_setup_note(state_dir: &Path) -> String {
    format!(
        "helix: new hx windows follow the look once ~/.config/helix/themes/scoot-look.toml points at {} with theme = \"scoot-look\" in config.toml",
        state_dir.join("look/apps/helix.toml").display()
    )
}

/// The one-time btop wiring: the same fixed-name theme beside the user's
/// own `btop.conf` (which keeps naming it in `color_theme`).
fn btop_setup_note(state_dir: &Path) -> String {
    format!(
        "btop: new btop runs follow the look once ~/.config/btop/themes/scoot-look.theme points at {} with color_theme = \"scoot-look\" in btop.conf",
        state_dir.join("look/apps/btop.theme").display()
    )
}

/// The one-time starship wiring: starship reads one config file per
/// process, so the file itself follows the state link and every next
/// prompt draws the look with no signal.
fn starship_setup_note(state_dir: &Path) -> String {
    format!(
        "starship: the next prompt reads the look once STARSHIP_CONFIG or ~/.config/starship.toml points at {}",
        state_dir.join("look/apps/starship.toml").display()
    )
}

#[cfg(test)]
mod tests;
