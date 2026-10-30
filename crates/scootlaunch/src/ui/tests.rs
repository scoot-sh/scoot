//! The panel state machine's tests: keys, collapse, highlights, and
//! the zero-allocation pin.

use super::*;
use crate::rank::frecency::Frecency;

fn entry(id: &str, name: &str) -> index::Entry {
    entry_with_actions(id, name, &[])
}

fn entry_with_actions(id: &str, name: &str, actions: &[(&str, &str)]) -> index::Entry {
    index::Entry {
        id: id.to_owned(),
        name: name.to_owned(),
        generic_name: None,
        comment: None,
        keywords: Vec::new(),
        icon: None,
        exec: vec![id.to_owned()],
        terminal: false,
        dbus_activatable: false,
        actions: actions
            .iter()
            .map(|(aid, aname)| index::Action {
                id: aid.to_string(),
                name: aname.to_string(),
                exec: vec![aid.to_string()],
            })
            .collect(),
        path: std::path::PathBuf::from("/usr/share/applications/x.desktop"),
        workdir: None,
        hidden: None,
    }
}

fn three() -> Ui {
    Ui::open(
        vec![
            entry("firefox.desktop", "Firefox"),
            entry("files.desktop", "Files"),
            entry("terminal.desktop", "Terminal"),
        ],
        Frecency::empty(),
        0,
        10,
    )
}

#[test]
fn open_selects_the_top_of_frecency_then_name() {
    let ui = three();
    assert_eq!(
        ui.selected().map(|entry| entry.id.as_str()),
        Some("files.desktop")
    );
}

#[test]
fn typing_narrows_and_enter_selects() {
    let mut ui = three();
    assert_eq!(ui.key(0, false, "term"), Outcome::Changed);
    assert_eq!(ui.query(), "term");
    let shown: Vec<&str> = ui.shown().iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(shown, vec!["terminal.desktop"]);
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Selected("terminal.desktop".to_owned())
    );
}

#[test]
fn arrows_tab_and_ctrl_hands_move() {
    let mut ui = three();
    ui.key(scootui::entry::KEY_DOWN, false, "");
    assert_eq!(
        ui.selected().map(|e| e.id.as_str()),
        Some("firefox.desktop")
    );
    ui.key(scootui::entry::KEY_UP, false, "");
    assert_eq!(ui.selected().map(|e| e.id.as_str()), Some("files.desktop"));
    ui.key(scootui::entry::KEY_TAB, false, "");
    assert_eq!(
        ui.selected().map(|e| e.id.as_str()),
        Some("firefox.desktop")
    );
    ui.key(KEY_SHIFT_TAB, false, "");
    assert_eq!(ui.selected().map(|e| e.id.as_str()), Some("files.desktop"));
    ui.key(u32::from(b'n'), true, "");
    assert_eq!(
        ui.selected().map(|e| e.id.as_str()),
        Some("firefox.desktop")
    );
    ui.key(u32::from(b'p'), true, "");
    assert_eq!(ui.selected().map(|e| e.id.as_str()), Some("files.desktop"));
    ui.key(scootui::entry::KEY_END, true, "");
    assert_eq!(
        ui.selected().map(|e| e.id.as_str()),
        Some("terminal.desktop")
    );
    ui.key(scootui::entry::KEY_HOME, true, "");
    assert_eq!(ui.selected().map(|e| e.id.as_str()), Some("files.desktop"));
}

#[test]
fn plain_home_and_end_move_the_caret_and_leave_the_selection() {
    let mut ui = three();
    for ch in "fi".chars() {
        let mut buf = [0u8; 4];
        ui.key(u32::from(ch), false, ch.encode_utf8(&mut buf));
    }
    let selected = ui.selected().map(|e| e.id.clone());
    assert_eq!(ui.entry().caret(), 2);
    ui.key(scootui::entry::KEY_HOME, false, "");
    assert_eq!(ui.entry().caret(), 0, "Home goes to the line start");
    assert_eq!(ui.selected().map(|e| e.id.clone()), selected);
    ui.key(scootui::entry::KEY_END, false, "");
    assert_eq!(ui.entry().caret(), 2, "End goes to the line end");
    assert_eq!(ui.selected().map(|e| e.id.clone()), selected);
}

#[test]
fn escape_cancels_and_ctrl_enter_runs_typed_text() {
    let mut ui = three();
    assert_eq!(ui.key(KEY_ESCAPE, false, ""), Outcome::Cancelled);
    assert_eq!(ui.key(KEY_RETURN, true, ""), Outcome::Ignored);
    ui.key(0, false, "htop");
    assert_eq!(
        ui.key(KEY_RETURN, true, ""),
        Outcome::Run("htop".to_owned())
    );
}

#[test]
fn enter_with_no_match_does_nothing() {
    let mut ui = three();
    ui.key(0, false, "zzzzzz");
    assert!(ui.shown().is_empty());
    assert_eq!(ui.key(KEY_RETURN, false, ""), Outcome::Ignored);
}

#[test]
fn highlights_cover_the_matched_name_bytes() {
    let mut ui = three();
    ui.key(0, false, "fi");
    // Both prefix-match; the tie breaks by row order.
    let shown: Vec<&str> = ui.shown().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(shown, vec!["Firefox", "Files"]);
    let list = ui.list();
    assert_eq!(list.highlights_for(0), &[(0u32, 2u32)]);
    assert_eq!(list.highlights_for(1), &[(0u32, 2u32)]);
}

#[test]
fn clicks_select_and_launch_and_wheels_scroll() {
    let mut ui = Ui::open(
        vec![
            entry("a.desktop", "Alpha"),
            entry("b.desktop", "Beta"),
            entry("c.desktop", "Gamma"),
        ],
        Frecency::empty(),
        0,
        2,
    );
    assert_eq!(ui.click(2), Outcome::Selected("c.desktop".to_owned()));
    assert_eq!(ui.click(99), Outcome::Ignored);
    ui.click(0);
    ui.scroll(false);
    assert_eq!(ui.list().first_visible(), 1);
    ui.scroll(true);
    assert_eq!(ui.list().first_visible(), 0);
}

#[test]
fn hidden_entries_never_show() {
    let mut hidden = entry("secret.desktop", "Secret");
    hidden.hidden = Some(crate::index::filter::HiddenReason::NoDisplay);
    let ui = Ui::open(
        vec![hidden, entry("files.desktop", "Files")],
        Frecency::empty(),
        0,
        10,
    );
    assert_eq!(ui.shown().len(), 1);
}

#[test]
fn hostile_queries_never_panic() {
    let mut ui = three();
    for query in ["", "\0", "ßẞſ", "e\u{301}", &"f".repeat(600), "a/b:c;d"] {
        ui.key(0, false, query);
        let _ = ui.shown();
        let _ = ui.list().highlights();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn keystrokes_allocate_nothing_after_warm_up() {
    let mut ui = three();
    // Warm up every buffer: long queries, reranks, scrolls, highlights.
    for query in [
        "firefox terminal files",
        "e",
        "zzzzzz",
        "",
        "a",
        "fi",
        "term",
    ] {
        for char in query.chars() {
            let mut piece = [0u8; 4];
            ui.key(0, false, char.encode_utf8(&mut piece));
        }
        ui.key(scootui::entry::KEY_DOWN, false, "");
        ui.key(scootui::entry::KEY_UP, false, "");
        ui.scroll(false);
        ui.scroll(true);
        ui.key(scootui::entry::KEY_BACKSPACE, false, "");
        while !ui.query().is_empty() {
            ui.key(scootui::entry::KEY_BACKSPACE, false, "");
        }
    }
    for char in "terminal".chars() {
        let mut piece = [0u8; 4];
        let text = char.encode_utf8(&mut piece).to_owned();
        let (_, allocs) = scootbg_mem::count_allocations(|| {
            ui.key(0, false, &text);
        });
        assert_eq!(allocs, 0, "typing {text:?} allocates");
    }
    for keysym in [
        scootui::entry::KEY_DOWN,
        scootui::entry::KEY_UP,
        scootui::entry::KEY_PAGE_DOWN,
        scootui::entry::KEY_PAGE_UP,
        scootui::entry::KEY_HOME,
        scootui::entry::KEY_END,
        scootui::entry::KEY_TAB,
        KEY_SHIFT_TAB,
    ] {
        let ctrl = keysym == scootui::entry::KEY_HOME || keysym == scootui::entry::KEY_END;
        let (_, allocs) = scootbg_mem::count_allocations(|| {
            ui.key(keysym, ctrl, "");
        });
        assert_eq!(allocs, 0, "keysym {keysym:#x} allocates");
    }
    let (_, allocs) = scootbg_mem::count_allocations(|| {
        ui.scroll(false);
        ui.scroll(true);
    });
    assert_eq!(allocs, 0, "scrolling allocates");
}

#[test]
fn an_action_row_win_reports_its_action() {
    let mut ui = Ui::open(
        vec![entry_with_actions(
            "firefox.desktop",
            "Firefox",
            &[("new-window", "New Window")],
        )],
        Frecency::empty(),
        0,
        10,
    );
    // An empty query shows entry rows only: no action yet.
    assert!(ui.selected_action().is_none());
    // Typing the action's name ranks its row first, and the selection
    // carries the action for `ActivateAction`.
    ui.key(0, false, "new window");
    assert_eq!(
        ui.selected().map(|entry| entry.id.as_str()),
        Some("firefox.desktop")
    );
    assert_eq!(
        ui.selected_action().map(|action| action.id.as_str()),
        Some("new-window")
    );
}

#[test]
fn the_generation_moves_with_the_rows_and_not_with_the_selection() {
    let mut ui = three();
    let opened = ui.generation();
    // A selection move and a scroll leave the rows as they were.
    ui.key(scootui::entry::KEY_DOWN, false, "");
    ui.key(scootui::entry::KEY_TAB, false, "");
    ui.scroll(false);
    assert_eq!(ui.generation(), opened);
    // A typed character rebuilds them.
    ui.key(0, false, "t");
    assert_ne!(ui.generation(), opened);
}
