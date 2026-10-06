//! Rendering for `scoot msg binds`: the live keymap as a grouped, aligned
//! table for humans (`--json` keeps the raw reply for agents).
//!
//! A cold path (one process per invocation): the allocations here cost
//! nothing at runtime. Grouping mirrors the keybindings reference
//! (`site/src/content/docs/scoot/keybindings.md`) -- Move around, Move
//! windows, Shape windows, Launch and leave -- so the terminal output and
//! the docs teach the same map; anything neither page groups (session VT
//! switches, unbound defaults, skipped config binds) gets its own trailing
//! section.

use scoot_ipc::{BindRow, SkippedBind};

/// Renders a `binds` reply as the human table: live rows grouped by intent
/// with aligned columns, then the defaults a config unbind removed, then
/// the config binds that were skipped with their reasons.
pub fn format_binds(bindings: &[BindRow], skipped: &[SkippedBind]) -> String {
    let (live, unbound): (Vec<&BindRow>, Vec<&BindRow>) = bindings
        .iter()
        .partition(|row| !row.source.starts_with("config (unbinds default:"));
    let mut text = String::new();
    let width = live
        .iter()
        .map(|row| row.combo.len())
        .max()
        .unwrap_or(0)
        .min(28);
    for group in [
        "Move around",
        "Move windows",
        "Shape windows",
        "Launch and leave",
    ] {
        let mut rows: Vec<&BindRow> = live
            .iter()
            .filter(|row| group_of(&row.action) == group)
            .copied()
            .collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_by(|a, b| a.combo.cmp(&b.combo));
        text.push_str(group);
        text.push('\n');
        for row in rows {
            text.push_str(&format_row(row, width));
        }
        text.push('\n');
    }
    // Anything the page's groups do not cover (session VT switches, and any
    // action added without a group here): still listed, under one roof,
    // rather than silently dropped from the human view.
    let mut other: Vec<&BindRow> = live
        .iter()
        .filter(|row| group_of(&row.action) == "Session")
        .copied()
        .collect();
    other.sort_by(|a, b| a.combo.cmp(&b.combo));
    if !other.is_empty() {
        text.push_str("Session\n");
        for row in other {
            text.push_str(&format_row(row, width));
        }
        text.push('\n');
    }
    if !unbound.is_empty() {
        let mut removed: Vec<&BindRow> = unbound;
        removed.sort_by(|a, b| a.combo.cmp(&b.combo));
        text.push_str("Unbound defaults\n");
        for row in removed {
            text.push_str(&format_row(row, width));
        }
        text.push('\n');
    }
    if !skipped.is_empty() {
        let mut missed: Vec<&SkippedBind> = skipped.iter().collect();
        missed.sort_by(|a, b| a.bind.cmp(&b.bind));
        text.push_str("Skipped config binds\n");
        for entry in missed {
            text.push_str(&format!(
                "  \"{}\" = {} -- {}\n",
                entry.bind, entry.value, entry.reason
            ));
        }
    }
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

/// One live or unbound row: the combo padded to the table width, the action,
/// the source, and any flags. The combo and action columns are what align;
/// a long `replaces default:` source may run past 80 columns rather than
/// wrap the meaningful half onto a second line.
fn format_row(row: &BindRow, width: usize) -> String {
    let mut line = format!(
        "  {:<width$}  {}",
        row.combo,
        row.action,
        width = width.max(row.combo.len())
    );
    line.push_str(&format!("  {}", row.source));
    if row.repeat {
        line.push_str("  [repeat]");
    }
    if row.allow_when_locked {
        line.push_str("  [allow-when-locked]");
    }
    line.push('\n');
    line
}

/// The keybindings-page group one action string belongs to, by its verb --
/// the first word, so arguments (`focus-workspace-index 2 --output 3`) and
/// flags never affect it. `change-vt` is the session's own spelling (see
/// `BindRow::action`); anything unrecognized lands in `Session` rather than
/// vanishing from the human view.
fn group_of(action: &str) -> &'static str {
    let verb = action.split_whitespace().next().unwrap_or("");
    match verb {
        "focus-column"
        | "focus-window"
        | "focus-workspace"
        | "focus-workspace-index"
        | "focus-window-id"
        | "focus-output"
        | "focus-output-index"
        | "focus-output-left"
        | "focus-output-right"
        | "toggle-floating-focus" => "Move around",
        "move-column"
        | "move-window"
        | "move-window-to-workspace"
        | "move-window-to-workspace-index"
        | "move-window-to-output"
        | "move-window-to-output-index"
        | "move-window-to-output-left"
        | "move-window-to-output-right"
        | "consume-or-expel" => "Move windows",
        "cycle-column-width" | "set-column-width" | "toggle-fullscreen" | "set-fullscreen"
        | "toggle-maximize" | "set-maximized" | "toggle-floating" | "set-floating"
        | "move-floating" | "resize-floating" => "Shape windows",
        "close" | "spawn" | "quit" | "show-keymap" => "Launch and leave",
        _ => "Session",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(combo: &str, action: &str, source: &str) -> BindRow {
        BindRow {
            combo: combo.to_owned(),
            action: action.to_owned(),
            source: source.to_owned(),
            repeat: false,
            allow_when_locked: false,
        }
    }

    #[test]
    fn rows_group_the_way_the_keybindings_page_groups_them() {
        let bindings = vec![
            row("super+q", "close", "default"),
            row("super+h", "focus-column left", "default"),
            row("super+m", "toggle-maximize", "default"),
            row("super+shift+h", "move-column left", "default"),
        ];
        let text = format_binds(&bindings, &[]);
        let (mut around, mut windows, mut shape, mut launch) = (0, 0, 0, 0);
        let mut group = "";
        for line in text.lines() {
            match line {
                "Move around" => group = "around",
                "Move windows" => group = "windows",
                "Shape windows" => group = "shape",
                "Launch and leave" => group = "launch",
                _ => match group {
                    "around" if line.contains("super+h") => around += 1,
                    "windows" if line.contains("super+shift+h") => windows += 1,
                    "shape" if line.contains("super+m") => shape += 1,
                    "launch" if line.contains("super+q") => launch += 1,
                    _ => {}
                },
            }
        }
        assert_eq!((around, windows, shape, launch), (1, 1, 1, 1));
    }

    #[test]
    fn unbinds_and_skipped_get_their_own_sections() {
        let bindings = vec![
            row("super+h", "focus-column left", "default"),
            row("super+q", "close", "config (unbinds default: close)"),
        ];
        let skipped = vec![SkippedBind {
            bind: "Super+H".to_owned(),
            value: "\"close\"".to_owned(),
            reason: "collides with \"super+h\": none of the group applies".to_owned(),
        }];
        let text = format_binds(&bindings, &skipped);
        assert!(text.contains("Unbound defaults\n  super+q"), "{text}");
        assert!(
            text.contains("Skipped config binds\n  \"Super+H\" = \"close\" -- collides"),
            "{text}"
        );
        // The unbind is not listed among the live rows.
        assert!(!text.contains("Move around\n  super+q"));
    }

    #[test]
    fn flags_read_on_the_row() {
        let bindings = vec![BindRow {
            combo: "XF86AudioRaiseVolume".to_owned(),
            action: "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+".to_owned(),
            source: "config".to_owned(),
            repeat: true,
            allow_when_locked: true,
        }];
        let text = format_binds(&bindings, &[]);
        assert!(text.contains("[repeat]"), "{text}");
        assert!(text.contains("[allow-when-locked]"), "{text}");
    }

    #[test]
    fn combos_align_and_session_rows_are_kept() {
        let bindings = vec![
            row("super+h", "focus-column left", "default"),
            row("ctrl+alt+F3", "change-vt 3", "session (VT switch)"),
        ];
        let text = format_binds(&bindings, &[]);
        assert!(text.contains("Session\n  ctrl+alt+F3"), "{text}");
        let focus = text.lines().find(|line| line.contains("super+h")).unwrap();
        let vt = text.lines().find(|line| line.contains("ctrl+alt")).unwrap();
        assert_eq!(
            focus.find("focus-column"),
            vt.find("change-vt"),
            "the action column aligns:\n{text}"
        );
    }
}
