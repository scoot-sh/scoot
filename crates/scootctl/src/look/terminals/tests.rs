//! The terminal step against scripted signals: presence, opt-outs, the
//! private-state skip, and the setup notes.

use std::path::PathBuf;

use super::super::leaves::ScriptedLeaves;
use super::{apply_terminals, presence_in};

fn render_dir(tag: &str, leaves: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("scoot-terminals-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch render dir");
    for leaf in leaves {
        let path = dir.path().join(leaf);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("leaf parents");
        std::fs::write(&path, b"leaf").expect("a leaf");
    }
    let path = dir.path().to_owned();
    (dir, path)
}

#[test]
fn presence_reads_the_render() {
    let (_guard, dir) = render_dir("presence", &["apps/helix.toml", "apps/starship.toml"]);
    assert_eq!(
        presence_in(&dir),
        super::TerminalPresence {
            helix: true,
            btop: false,
            starship: true,
        }
    );
}

#[test]
fn signals_run_and_setup_notes_follow() {
    let (_guard, dir) = render_dir("signals", &["apps/helix.toml", "apps/btop.theme"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false);
    assert_eq!(leaves.signaled(), vec!["helix".to_owned(), "btop".to_owned()]);
    assert!(report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(report.reloaded.contains(&"btop".to_owned()), "{report:?}");
    assert!(
        report.notes.iter().any(|note| note.contains("scoot-look.toml")),
        "{report:?}"
    );
    assert!(
        report.notes.iter().any(|note| note.contains("scoot-look.theme")),
        "{report:?}"
    );
}

#[test]
fn no_process_is_a_note_not_a_reload() {
    let (_guard, dir) = render_dir("empty", &["apps/helix.toml"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    leaves.script_signals(vec![Ok(0)], vec![Ok(1)]);
    let report = apply_terminals(&state, &dir, &leaves, false, false);
    assert!(!report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(
        report.notes.iter().any(|note| note.contains("no hx running")),
        "{report:?}"
    );
}

#[test]
fn opt_outs_skip_the_signal_but_keep_the_note() {
    let (_guard, dir) = render_dir("optout", &["apps/helix.toml", "apps/btop.theme"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, true, true);
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(report.reloaded.is_empty(), "{report:?}");
    assert!(
        report.notes.iter().any(|note| note.contains("--no-helix")),
        "{report:?}"
    );
    assert!(
        report.notes.iter().any(|note| note.contains("--no-btop")),
        "{report:?}"
    );
    // The setup hint still prints: the leaf applied, only the signal
    // was skipped.
    assert!(
        report.notes.iter().any(|note| note.contains("scoot-look.toml")),
        "{report:?}"
    );
}

#[test]
fn absent_leaves_stay_silent() {
    let (_guard, dir) = render_dir("absent", &[]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false);
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(report.reloaded.is_empty(), "{report:?}");
    assert!(report.notes.is_empty(), "{report:?}");
}

#[test]
fn private_state_runs_no_signal() {
    let (_guard, dir) = render_dir("private", &["apps/helix.toml", "apps/btop.theme"]);
    let leaves = ScriptedLeaves::ok();
    // A headless session's private state dir: the leaf starts with the
    // private prefix, so the step stands down like the daemon one.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let private = runtime.join("scoot-look-test-display");
    let report = apply_terminals(&private, &dir, &leaves, false, false);
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(
        report.notes.iter().any(|note| note.contains("private look state")),
        "{report:?}"
    );
}
