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
            nvim: false,
        }
    );
    let (_guard, nvim_dir) = render_dir("presence-nvim", &["apps/nvim.lua"]);
    assert!(
        presence_in(&nvim_dir).nvim,
        "a render carrying apps/nvim.lua reports it"
    );
}

#[test]
fn signals_run_and_setup_notes_follow() {
    let (_guard, dir) = render_dir("signals", &["apps/helix.toml", "apps/btop.theme"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false, None);
    assert_eq!(
        leaves.signaled(),
        vec!["helix".to_owned(), "btop".to_owned()]
    );
    assert!(report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(report.reloaded.contains(&"btop".to_owned()), "{report:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.toml")),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.theme")),
        "{report:?}"
    );
}

#[test]
fn nvim_carries_a_setup_note_and_no_signal() {
    let (_guard, dir) = render_dir("nvim", &["apps/nvim.lua"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false, None);
    // No signal exists for nvim: the colorscheme watches itself, so
    // nothing is signaled and nothing reloaded.
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(report.reloaded.is_empty(), "{report:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.lua")),
        "{report:?}"
    );
}

#[test]
fn no_process_is_a_note_not_a_reload() {
    let (_guard, dir) = render_dir("empty", &["apps/helix.toml"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    leaves.script_signals(vec![Ok(0)], vec![Ok(1)]);
    let report = apply_terminals(&state, &dir, &leaves, false, false, None);
    assert!(!report.reloaded.contains(&"helix".to_owned()), "{report:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("no hx running")),
        "{report:?}"
    );
}

#[test]
fn opt_outs_skip_the_signal_but_keep_the_note() {
    let (_guard, dir) = render_dir("optout", &["apps/helix.toml", "apps/btop.theme"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, true, true, None);
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
        report
            .notes
            .iter()
            .any(|note| note.contains("scoot-look.toml")),
        "{report:?}"
    );
}

#[test]
fn absent_leaves_stay_silent() {
    let (_guard, dir) = render_dir("absent", &[]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false, None);
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
    let report = apply_terminals(&private, &dir, &leaves, false, false, None);
    assert!(leaves.signaled().is_empty(), "no signal attempted");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("private look state")),
        "{report:?}"
    );
}

#[test]
fn a_pinned_config_home_prints_real_paths() {
    let (_guard, dir) = render_dir("pinned", &["apps/helix.toml", "apps/starship.toml"]);
    let state = PathBuf::from("/tmp/scoot-terminals-state");
    let config = PathBuf::from("/tmp/scoot-terminals-config");
    let leaves = ScriptedLeaves::ok();
    let report = apply_terminals(&state, &dir, &leaves, false, false, Some(&config));
    let helix = report
        .notes
        .iter()
        .find(|note| note.contains("scoot-look.toml"))
        .expect("a helix note");
    assert!(
        helix.contains("/tmp/scoot-terminals-config/helix/themes/scoot-look.toml"),
        "{helix}"
    );
    assert!(!helix.contains('~'), "no tilde in a pinned note: {helix}");
    let starship = report
        .notes
        .iter()
        .find(|note| note.contains("starship.toml"))
        .expect("a starship note");
    assert!(
        starship.contains("/tmp/scoot-terminals-config/starship.toml"),
        "{starship}"
    );
}
