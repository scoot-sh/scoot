//! The dmenu state machine's tests: keys, the stdin contract, and
//! the zero-allocation pin.

use super::*;
use crate::rank::frecency::Frecency;

fn three() -> DmenuUi {
    DmenuUi::open(
        vec![
            "firefox".to_string(),
            "files".to_string(),
            "terminal".to_string(),
        ],
        10,
        false,
        false,
    )
}

#[test]
fn empty_query_shows_stdin_order() {
    // No frecency without `--history`: the writer's order stands.
    let ui = DmenuUi::open(
        vec!["zzz".to_string(), "aaa".to_string(), "mmm".to_string()],
        10,
        false,
        false,
    );
    assert_eq!(ui.shown(), vec!["zzz", "aaa", "mmm"]);
    assert_eq!(ui.selected_line(), Some(0));
}

#[test]
fn typing_narrows_and_return_prints_the_line() {
    let mut ui = three();
    assert_eq!(ui.key(0, false, "term"), Outcome::Changed);
    assert_eq!(ui.query(), "term");
    assert_eq!(ui.shown(), vec!["terminal"]);
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Line(2))
    );
}

#[test]
fn return_with_no_match_prints_the_typed_text() {
    let mut ui = three();
    ui.key(0, false, "zzz-no-such-row");
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Text("zzz-no-such-row".to_string()))
    );
}

#[test]
fn only_match_refuses_a_custom_entry() {
    let mut ui = DmenuUi::open(
        vec!["alpha".to_string(), "beta".to_string()],
        10,
        true,
        false,
    );
    ui.key(0, false, "zzz-no-such-row");
    assert_eq!(ui.key(KEY_RETURN, false, ""), Outcome::Ignored);
    // Empty query with an empty list is nothing either, not an empty print.
    let mut empty = DmenuUi::open(Vec::new(), 10, true, false);
    assert_eq!(empty.key(KEY_RETURN, false, ""), Outcome::Ignored);
    // Without the flag the typed text still prints.
    let mut loose = DmenuUi::open(Vec::new(), 10, false, false);
    loose.key(0, false, "custom");
    assert_eq!(
        loose.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Text("custom".to_string()))
    );
}

#[test]
fn ctrl_enter_prints_the_typed_text_even_with_only_match() {
    // The execute-input exception: Ctrl+Enter always prints, even with
    // `--only-match`.
    let mut ui = DmenuUi::open(
        vec!["alpha".to_string(), "beta".to_string()],
        10,
        true,
        false,
    );
    ui.key(0, false, "custom words");
    assert_eq!(
        ui.key(KEY_RETURN, true, ""),
        Outcome::Print(Print::Text("custom words".to_string()))
    );
    let mut empty = DmenuUi::open(Vec::new(), 10, true, false);
    assert_eq!(empty.key(KEY_RETURN, true, ""), Outcome::Ignored);
}

#[test]
fn duplicates_print_their_own_occurrence() {
    let mut ui = DmenuUi::open(
        vec!["same".to_string(), "same".to_string(), "other".to_string()],
        10,
        false,
        false,
    );
    // An empty query shows both occurrences, in order.
    assert_eq!(ui.shown(), vec!["same", "same", "other"]);
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Line(0))
    );
    ui.key(scootui::entry::KEY_DOWN, false, "");
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Line(1))
    );
}

#[test]
fn prompt_only_shows_no_list_and_ignores_clicks() {
    let mut ui = DmenuUi::open(vec!["a".to_string(), "b".to_string()], 10, false, true);
    assert_eq!(ui.list().count(), 0);
    assert_eq!(ui.selected_line(), None);
    assert_eq!(ui.click(0), Outcome::Ignored);
    // Typing still edits; Enter prints the typed text.
    ui.key(0, false, "s3cret");
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Text("s3cret".to_string()))
    );
    assert_eq!(ui.key(KEY_ESCAPE, false, ""), Outcome::Cancelled);
}

#[test]
fn arrows_tab_and_ctrl_hands_move() {
    let mut ui = three();
    ui.key(scootui::entry::KEY_DOWN, false, "");
    assert_eq!(ui.selected_line(), Some(1));
    ui.key(scootui::entry::KEY_UP, false, "");
    assert_eq!(ui.selected_line(), Some(0));
    ui.key(scootui::entry::KEY_TAB, false, "");
    assert_eq!(ui.selected_line(), Some(1));
    ui.key(KEY_SHIFT_TAB, false, "");
    assert_eq!(ui.selected_line(), Some(0));
    ui.key(u32::from(b'n'), true, "");
    assert_eq!(ui.selected_line(), Some(1));
    ui.key(u32::from(b'p'), true, "");
    assert_eq!(ui.selected_line(), Some(0));
}

#[test]
fn plain_home_and_end_move_the_caret_and_leave_the_selection() {
    let mut ui = three();
    for ch in "fi".chars() {
        let mut buf = [0u8; 4];
        ui.key(u32::from(ch), false, ch.encode_utf8(&mut buf));
    }
    let selected = ui.selected_line();
    ui.key(scootui::entry::KEY_HOME, false, "");
    assert_eq!(ui.selected_line(), selected);
    ui.key(scootui::entry::KEY_END, false, "");
    assert_eq!(ui.selected_line(), selected);
}

#[test]
fn hostile_queries_never_panic() {
    let mut ui = DmenuUi::open(
        vec!["a\0b".to_string(), "ßẞſ".to_string()],
        10,
        false,
        false,
    );
    for query in [
        "",
        "\0",
        "ßẞſ",
        "e\u{301}",
        &"f".repeat(600),
        "a/b:c;d",
        "\r\n",
    ] {
        ui.key(0, false, query);
        let _ = ui.shown();
        let _ = ui.list().highlights();
    }
    assert_eq!(ui.key(KEY_ESCAPE, false, ""), Outcome::Cancelled);
}

#[test]
fn secret_mode_types_and_prints_while_hiding_the_type() {
    // The masked widget holds the secret; the draw reads through
    // `secret_entry` (never the bytes), and the widget's own `Debug`
    // is redacted (pinned in scootui).
    let mut ui = DmenuUi::open_secret(
        vec!["alpha".to_string(), "beta".to_string()],
        10,
        false,
        true,
    );
    assert!(ui.secret_entry().is_some());
    assert!(ui.plain_entry().is_none());
    let secret = ui.secret_entry().expect("masked in secret mode");
    assert!(secret.secret_bytes().is_empty());
    ui.key(0, false, "s3cret!");
    // The widget holds the typed bytes; the UI reads them back.
    let secret = ui.secret_entry().expect("masked in secret mode");
    assert_eq!(secret.secret_bytes(), b"s3cret!");
    assert_eq!(ui.query(), "s3cret!");
    // The list still filters on the masked query; nothing matches.
    assert_eq!(
        ui.key(KEY_RETURN, false, ""),
        Outcome::Print(Print::Text("s3cret!".to_string()))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn keystrokes_allocate_nothing_after_warm_up() {
    let lines: Vec<String> = (0..3000).map(|index| format!("line-{index:05}")).collect();
    let mut ui = DmenuUi::open(lines, 10, false, false);
    // Warm up every buffer: long queries, reranks, scrolls, highlights.
    for query in [
        "line-01234 line-99999",
        "e",
        "zzzzzz",
        "",
        "a",
        "li",
        "line-1",
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
    for char in "line-42".chars() {
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
fn a_pipe_to_eof_reads_all_of_it() {
    let (reader, writer) = rustix::pipe::pipe().expect("a pipe makes");
    let data = b"one\ntwo\nthree";
    let mut written = 0;
    while written < data.len() {
        match rustix::io::write(&writer, &data[written..]) {
            Ok(0) => break,
            Ok(got) => written += got,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => panic!("a pipe writes: {error}"),
        }
    }
    drop(writer);
    let bytes = read_fd(&reader, Duration::from_secs(5), lines::MAX_BYTES);
    assert_eq!(bytes, data);
}

#[test]
fn a_writer_that_never_ends_bounds_the_read() {
    // A pipe with the write end open but nothing in it: the read ends
    // at the timeout with what arrived (nothing), instead of wedging.
    let (reader, writer) = rustix::pipe::pipe().expect("a pipe makes");
    let start = Instant::now();
    let bytes = read_fd(&reader, Duration::from_millis(100), lines::MAX_BYTES);
    assert!(bytes.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the bound holds: {:?}",
        start.elapsed()
    );
    drop(writer);
}

#[test]
fn a_slow_writer_keeps_what_arrived() {
    // One line now, silence after: the read keeps the line.
    let (reader, writer) = rustix::pipe::pipe().expect("a pipe makes");
    rustix::io::write(&writer, b"partial\n").expect("a pipe writes");
    let bytes = read_fd(&reader, Duration::from_millis(100), lines::MAX_BYTES);
    assert_eq!(bytes, b"partial\n");
    drop(writer);
}

#[test]
fn the_read_stops_at_the_byte_cap() {
    // A writer thread feeds past the cap while this side reads: the
    // read ends at the cap (the pipe never wedges either side), and
    // the write end stays open, so no EOF ends it early either.
    let (reader, writer) = rustix::pipe::pipe().expect("a pipe makes");
    let big = vec![b'z'; 70_000];
    let writer_thread = std::thread::spawn(move || {
        let mut written = 0;
        while written < big.len() {
            match rustix::io::write(&writer, &big[written..]) {
                Ok(0) => break,
                Ok(got) => written += got,
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => break,
            }
        }
    });
    let bytes = read_fd(&reader, Duration::from_secs(5), 65_536);
    assert_eq!(bytes.len(), 65_536);
    writer_thread.join().expect("the writer finishes");
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn bounds_hold() {
    // The contract's numbers, re-derived where the ranker relies on
    // them: a drift that breaks them fails here, not on a host.
    assert!(Frecency::empty().weight("", 0) == 0);
    assert_eq!(lines::MAX_BYTES, 1024 * 1024);
    assert_eq!(lines::MAX_LINES, 65_536);
    assert_eq!(lines::MAX_LINE_BYTES, 4096);
    assert_eq!(QUERY_CAP, 4096);
    assert_eq!(SECRET_CAP, 1024);
}
