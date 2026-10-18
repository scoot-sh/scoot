//! The entry, list, selector and focus chain: edits, bounds, hostile
//! input, and the allocation pins.

use scootbg_mem::count_allocations;

use super::focus::FocusChain;
use super::line::{Entry, InsertOutcome, MAX_ENTRY_BYTES, PasswordEntry};
use super::list::{Activate, List, MAX_HIGHLIGHT_ROWS, MAX_SPANS_PER_ROW, Selector};

fn typed(cap: usize, s: &str) -> Entry {
    let mut entry = Entry::new(cap);
    assert_eq!(entry.insert_text(s), InsertOutcome::Inserted);
    entry
}

#[test]
fn typing_lands_at_the_caret_and_moves_it() {
    let mut entry = Entry::new(64);
    assert_eq!(entry.insert_text("ab"), InsertOutcome::Inserted);
    entry.move_left();
    assert_eq!(entry.insert_text("X"), InsertOutcome::Inserted);
    assert_eq!(entry.text(), "aXb");
    assert_eq!(entry.caret(), 2);
}

#[test]
fn caret_moves_by_character_not_by_byte() {
    let mut entry = typed(64, "a\u{e9}b");
    entry.move_home();
    entry.move_right();
    assert_eq!(entry.caret(), 1);
    entry.move_right();
    assert_eq!(entry.caret(), 3);
    entry.move_right();
    assert_eq!(entry.caret(), 4);
    entry.move_right();
    assert_eq!(entry.caret(), 4);
    entry.move_left();
    assert_eq!(entry.caret(), 3);
}

#[test]
fn home_end_backspace_and_delete() {
    let mut entry = typed(64, "abcd");
    entry.move_left();
    entry.move_left();
    assert_eq!(entry.caret(), 2);
    entry.backspace();
    assert_eq!(entry.text(), "acd");
    entry.delete();
    assert_eq!(entry.text(), "ad");
    entry.move_home();
    assert_eq!(entry.caret(), 0);
    entry.backspace();
    assert_eq!(entry.text(), "ad");
    entry.move_end();
    assert_eq!(entry.caret(), 2);
    entry.delete();
    assert_eq!(entry.text(), "ad");
}

#[test]
fn word_moves_skip_separators_then_words() {
    let mut entry = typed(64, "foo bar_baz qux");
    entry.move_end();
    entry.move_word_left();
    assert_eq!(entry.caret(), 12);
    assert_eq!(entry.text().get(entry.caret()..), Some("qux"));
    entry.move_word_left();
    assert_eq!(entry.text().get(entry.caret()..), Some("bar_baz qux"));
    entry.move_word_right();
    assert_eq!(entry.text().get(entry.caret()..), Some("qux"));
    entry.move_home();
    entry.move_word_right();
    assert_eq!(entry.caret(), 4);
    assert_eq!(entry.text().get(entry.caret()..), Some("bar_baz qux"));
}

#[test]
fn ctrl_w_kills_word_and_space_ctrl_u_to_start_ctrl_a_selects() {
    // Ctrl+W kills back over the word and the whitespace before it.
    let mut entry = typed(64, "foo/bar baz");
    entry.delete_word_back();
    assert_eq!(entry.text(), "foo/bar");
    assert_eq!(entry.caret(), 7);
    let mut entry = typed(64, "foo bar");
    entry.move_word_left();
    assert_eq!(entry.caret(), 4);
    entry.delete_word_back();
    assert_eq!(entry.text(), "foobar");
    assert_eq!(entry.caret(), 3);
    let mut entry = typed(64, "hello");
    entry.delete_to_start();
    assert_eq!(entry.text(), "");
    // Ctrl+A then typing replaces the whole line.
    let mut entry = typed(64, "hello");
    entry.select_all();
    assert!(entry.selected_all());
    assert_eq!(entry.insert_text("bye"), InsertOutcome::Inserted);
    assert_eq!(entry.text(), "bye");
    assert!(!entry.selected_all());
    // Ctrl+A then BackSpace clears the whole line.
    let mut entry = typed(64, "hello");
    entry.select_all();
    entry.backspace();
    assert_eq!(entry.text(), "");
    // Select-all on an empty line selects nothing.
    let mut entry = Entry::new(64);
    entry.select_all();
    assert!(!entry.selected_all());
}

#[test]
fn any_caret_move_drops_the_selection() {
    let mut entry = typed(64, "hello");
    entry.select_all();
    entry.move_left();
    assert!(!entry.selected_all());
    assert_eq!(entry.text(), "hello");
}

#[test]
fn the_cap_cuts_at_a_character_boundary() {
    let mut entry = Entry::new(5);
    // "a\u{e9}" is 3 bytes; only 2 fit past "abc": the partial
    // character is refused whole, never split.
    assert_eq!(entry.insert_text("abc"), InsertOutcome::Inserted);
    assert_eq!(entry.insert_text("a\u{e9}"), InsertOutcome::Partial);
    assert_eq!(entry.text(), "abca");
    assert_eq!(entry.insert_text("zz"), InsertOutcome::Partial);
    assert_eq!(entry.text(), "abcaz");
    assert_eq!(entry.insert_text("z"), InsertOutcome::Full);
    assert!(entry.text().is_char_boundary(entry.text().len()));
}

#[test]
fn a_huge_paste_fills_to_the_cap_and_stays_valid() {
    let mut entry = Entry::new(512);
    let big: String = "x".repeat(100_000);
    assert_eq!(entry.insert_text(&big), InsertOutcome::Partial);
    assert_eq!(entry.len(), 512);
    assert_eq!(entry.caret(), 512);
    // Multibyte past the cap: cut whole, still valid.
    let mut entry = Entry::new(7);
    let emoji = "\u{1f600}".repeat(10);
    assert_eq!(entry.insert_text(&emoji), InsertOutcome::Partial);
    assert_eq!(entry.text(), "\u{1f600}");
    assert!(str::from_utf8(entry.text().as_bytes()).is_ok());
}

#[test]
fn caps_are_bounded_and_zero_is_always_full() {
    let entry = Entry::new(usize::MAX);
    assert_eq!(entry.cap(), MAX_ENTRY_BYTES);
    let mut entry = Entry::new(0);
    assert_eq!(entry.insert_text("a"), InsertOutcome::Full);
    assert_eq!(entry.text(), "");
    assert_eq!(entry.paste("a"), InsertOutcome::Full);
}

#[test]
fn copy_needs_select_all_and_ime_is_accepted() {
    let mut entry = typed(64, "hello");
    assert_eq!(entry.copy_text(), None);
    entry.select_all();
    assert_eq!(entry.copy_text(), Some("hello"));
    assert!(entry.accepts_ime());
}

#[test]
fn clear_keeps_the_buffer_for_the_next_fill() {
    let mut entry = typed(64, "hello world");
    entry.clear();
    assert_eq!(entry.text(), "");
    assert_eq!(entry.caret(), 0);
    // Refilling a cleared entry touches owned memory only.
    let (_, allocs) = count_allocations(|| {
        entry.insert_text("hi");
    });
    assert_eq!(allocs, 0);
    assert_eq!(entry.text(), "hi");
}

#[test]
fn key_table_moves_edits_and_passes_the_rest_through() {
    use super::line::{
        KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_RIGHT, KEY_UP,
    };
    let mut entry = typed(64, "abcd");
    entry.move_home();
    assert!(entry.apply_key(KEY_RIGHT, false, ""));
    assert_eq!(entry.caret(), 1);
    assert!(entry.apply_key(KEY_LEFT, false, ""));
    assert_eq!(entry.caret(), 0);
    assert!(entry.apply_key(KEY_END, false, ""));
    assert_eq!(entry.caret(), 4);
    assert!(entry.apply_key(KEY_HOME, false, ""));
    assert_eq!(entry.caret(), 0);
    assert!(entry.apply_key(KEY_RIGHT, true, ""));
    assert_eq!(entry.caret(), 4);
    assert!(entry.apply_key(KEY_LEFT, true, ""));
    assert_eq!(entry.caret(), 0);
    assert!(!entry.apply_key(KEY_UP, false, ""));
    assert!(!entry.apply_key(KEY_DOWN, false, ""));
    assert!(!entry.apply_key(0xff09, false, ""));
    // Text lands; control characters do not.
    assert!(entry.apply_key(u32::from(b'x'), false, "x"));
    assert_eq!(entry.text(), "xabcd");
    assert!(!entry.apply_key(0x17, false, "\u{17}"));
    assert_eq!(entry.text(), "xabcd");
    // Ctrl+letter by keysym or by control code.
    assert!(entry.apply_key(u32::from(b'a'), true, ""));
    assert!(entry.selected_all());
    assert!(entry.apply_key(0x15, true, ""));
    assert_eq!(entry.text(), "");
    let mut entry = typed(64, "foo bar");
    assert!(entry.apply_key(u32::from(b'W'), true, ""));
    assert_eq!(entry.text(), "foo");
    // Plain BackSpace/Delete.
    let mut entry = typed(64, "ab");
    assert!(entry.apply_key(KEY_BACKSPACE, false, ""));
    assert_eq!(entry.text(), "a");
    assert!(entry.apply_key(KEY_DELETE, false, ""));
    assert_eq!(entry.text(), "a");
    entry.move_home();
    assert!(entry.apply_key(KEY_DELETE, false, ""));
    assert_eq!(entry.text(), "");
}

#[test]
fn keystrokes_allocate_nothing_after_warm_up() {
    let mut entry = Entry::new(512);
    // Warm-up: size the buffer to the cap and run every path once.
    assert_eq!(
        entry.insert_text("warming up the buffer"),
        InsertOutcome::Inserted
    );
    entry.select_all();
    entry.backspace();
    assert_eq!(entry.insert_text("warm"), InsertOutcome::Inserted);
    entry.move_word_left();
    entry.delete_word_back();
    entry.delete_to_start();
    entry.clear();
    let (_, allocs) = count_allocations(|| {
        let mut total = 0;
        for word in ["alpha", "beta", "\u{e9}clair", "delta words here"] {
            match entry.insert_text(word) {
                InsertOutcome::Inserted | InsertOutcome::Partial => total += 1,
                InsertOutcome::Full => {}
            }
            entry.move_word_left();
            entry.move_word_right();
            entry.move_left();
            entry.move_right();
            entry.backspace();
            entry.delete();
            entry.delete_word_back();
            entry.delete_to_start();
            entry.select_all();
            entry.paste("p");
            entry.copy_text();
            entry.clear();
        }
        total
    });
    assert_eq!(allocs, 0, "a warmed-up keystroke must not allocate");
}

#[test]
fn password_writes_into_the_caller_slice_and_never_past_it() {
    let mut backing = [0u8; 16];
    let mut password = PasswordEntry::new(&mut backing);
    assert_eq!(password.cap(), 16);
    assert_eq!(password.insert_text("s3cret"), InsertOutcome::Inserted);
    assert_eq!(password.len(), 6);
    assert_eq!(password.secret_bytes(), b"s3cret");
    assert_eq!(password.char_len(), 6);
    // Past the slice: a prefix lands, cut whole.
    let big: String = "y".repeat(100);
    assert_eq!(password.insert_text(&big), InsertOutcome::Partial);
    assert_eq!(password.len(), 16);
    assert_eq!(password.insert_text("z"), InsertOutcome::Full);
    // The slice itself was never resized: its length is the caller's.
    assert_eq!(password.cap(), 16);
    // Caret edits inside the slice.
    password.move_home();
    password.move_right();
    password.backspace();
    assert_eq!(password.secret_bytes(), &b"3cretyyyyyyyyyy"[..]);
}

#[test]
fn password_clears_to_zeros_and_drops_to_zeros() {
    let mut backing = [0xA5u8; 64];
    {
        let mut password = PasswordEntry::new(&mut backing);
        assert_eq!(
            password.insert_text("hunter2 hunter2"),
            InsertOutcome::Inserted
        );
        password.move_word_left();
        password.backspace();
        password.clear();
        // The whole slice reads zeros after clear, not just the used
        // prefix: no length oracle in the freed tail.
        assert!(password.backing_bytes().iter().all(|b| *b == 0));
        assert_eq!(password.insert_text("again"), InsertOutcome::Inserted);
    }
    // Dropped without clearing (an early return past the login
    // screen): the slice still reads zeros.
    assert!(backing.iter().all(|b| *b == 0));
}

#[test]
fn password_deletes_zero_what_they_free() {
    let mut backing = [0u8; 32];
    let mut password = PasswordEntry::new(&mut backing);
    assert_eq!(password.insert_text("abcdefgh"), InsertOutcome::Inserted);
    password.move_home();
    password.move_right();
    password.move_right();
    password.delete();
    assert_eq!(password.secret_bytes(), b"abdefgh");
    // The freed tail byte is zeroed, not left holding 'h'.
    assert_eq!(password.backing_bytes()[7], 0);
    // Replacing the selection zeroes the old secret at once.
    password.select_all();
    assert_eq!(password.insert_text("xy"), InsertOutcome::Inserted);
    assert_eq!(password.secret_bytes(), b"xy");
    assert!(password.backing_bytes()[2..].iter().all(|b| *b == 0));
}

#[test]
fn password_refuses_copy_paste_and_ime() {
    let mut backing = [0u8; 32];
    let mut password = PasswordEntry::new(&mut backing);
    assert_eq!(password.insert_text("secret"), InsertOutcome::Inserted);
    password.select_all();
    // Even selected, nothing copies out.
    assert_eq!(password.copy_text(), None);
    assert!(!password.accepts_ime());
    // And there is no `paste` method on the type at all: a password
    // paste cannot compile. (This test pins the refusals that exist
    // at runtime; the missing method is structural.)
}

#[test]
fn password_edits_multibyte_and_keys_like_the_entry() {
    let mut backing = [0u8; 64];
    let mut password = PasswordEntry::new(&mut backing);
    assert_eq!(
        password.insert_text("caf\u{e9} ok"),
        InsertOutcome::Inserted
    );
    password.move_home();
    password.move_word_right();
    assert_eq!(password.caret(), 6);
    password.delete_word_back();
    assert_eq!(password.secret_bytes(), "caf\u{e9}ok".as_bytes());
    assert!(password.apply_key(u32::from(b'a'), true, ""));
    assert!(password.selected_all());
    // A zero-length slice is valid and always full.
    let mut empty: [u8; 0] = [];
    let mut password = PasswordEntry::new(&mut empty);
    assert_eq!(password.insert_text("x"), InsertOutcome::Full);
}

#[test]
fn password_keystrokes_allocate_nothing() {
    let mut backing = [0u8; 256];
    let mut password = PasswordEntry::new(&mut backing);
    assert_eq!(password.insert_text("warm"), InsertOutcome::Inserted);
    password.clear();
    let (_, allocs) = count_allocations(|| {
        for word in ["one", "two", "three"] {
            password.insert_text(word);
            password.move_word_left();
            password.backspace();
            password.delete_to_start();
            password.select_all();
            password.clear();
        }
    });
    assert_eq!(allocs, 0, "the password path owns no heap at all");
}

#[test]
fn an_empty_list_selects_nothing_and_ignores_moves() {
    let mut list = List::new(10);
    assert_eq!(list.selected(), None);
    assert_eq!(list.visible_range(), 0..0);
    assert_eq!(list.select_next(), None);
    assert_eq!(list.select_prev(), None);
    assert_eq!(list.page_down(), None);
    assert_eq!(list.page_up(), None);
    assert_eq!(list.home(), None);
    assert_eq!(list.end(), None);
    assert_eq!(list.select(3), None);
    list.scroll_to(99);
    assert_eq!(list.first_visible(), 0);
}

#[test]
fn selection_wraps_and_drags_the_window() {
    let mut list = List::new(4);
    list.set_count(10);
    assert_eq!(list.selected(), Some(0));
    assert_eq!(list.select_prev(), Some(9));
    assert_eq!(list.first_visible(), 6);
    assert_eq!(list.select_next(), Some(0));
    assert_eq!(list.first_visible(), 0);
    assert_eq!(list.select(7), Some(7));
    assert_eq!(list.first_visible(), 4);
    assert_eq!(list.select(99), Some(9));
    assert_eq!(list.page_up(), Some(5));
    assert_eq!(list.page_down(), Some(9));
    assert_eq!(list.first_visible(), 6);
    assert_eq!(list.home(), Some(0));
    assert_eq!(list.first_visible(), 0);
    assert_eq!(list.end(), Some(9));
    // A shrinking filter pulls the window back over the last row.
    list.set_count(3);
    assert_eq!(list.selected(), Some(2));
    assert_eq!(list.first_visible(), 0);
    assert_eq!(list.visible_range(), 0..3);
}

#[test]
fn scrolling_moves_the_window_but_not_the_selection() {
    let mut list = List::new(4);
    list.set_count(10);
    list.select(2);
    list.scroll_to(8);
    assert_eq!(list.first_visible(), 6);
    assert_eq!(list.selected(), Some(2));
    assert_eq!(list.visible_range(), 6..10);
    list.scroll_to(usize::MAX);
    assert_eq!(list.first_visible(), 6);
}

#[test]
fn highlights_round_trip_and_cap_out() {
    let mut list = List::new(10);
    list.set_count(100);
    list.set_highlights(&[
        (3, &[(0u32, 2u32), (5u32, 9u32)][..]),
        (7, &[(1u32, 1u32)][..]),
    ]);
    assert_eq!(list.highlights_for(3), &[(0, 2), (5, 9)][..]);
    assert_eq!(list.highlights_for(7), &[(1, 1)][..]);
    assert_eq!(list.highlights_for(4), &[] as &[_]);
    // Span overflow keeps the first eight; row overflow keeps 128.
    let wide: Vec<(u32, u32)> = (0..20).map(|i| (i, i + 1)).collect();
    list.set_highlights(&[(1, &wide[..])]);
    assert_eq!(list.highlights_for(1).len(), MAX_SPANS_PER_ROW);
    let many: Vec<(usize, &[(u32, u32)])> = (0..1000).map(|i| (i, &[] as &[(u32, u32)])).collect();
    list.set_highlights(&many);
    assert_eq!(list.highlights().len(), MAX_HIGHLIGHT_ROWS);
    // Refilling replaces, never accumulates.
    list.set_highlights(&[]);
    assert!(list.highlights().is_empty());
}

#[test]
fn list_key_table_moves_and_passes_the_rest_through() {
    use super::line::{KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_PAGE_DOWN};
    let mut list = List::new(4);
    list.set_count(10);
    assert!(list.apply_key(KEY_DOWN, false));
    assert_eq!(list.selected(), Some(1));
    assert!(list.apply_key(KEY_HOME, false));
    assert!(list.apply_key(KEY_PAGE_DOWN, false));
    assert_eq!(list.selected(), Some(4));
    assert!(list.apply_key(u32::from(b'n'), true));
    assert_eq!(list.selected(), Some(5));
    assert!(list.apply_key(u32::from(b'p'), true));
    assert_eq!(list.selected(), Some(4));
    assert!(list.apply_key(KEY_END, true));
    assert_eq!(list.selected(), Some(9));
    assert!(!list.apply_key(KEY_LEFT, false));
    assert!(!list.apply_key(KEY_DELETE, false));
    assert!(!list.apply_key(u32::from(b'x'), false));
    assert!(!list.apply_key(u32::from(b'n'), false));
    assert_eq!(list.selected(), Some(9));
}

#[test]
fn scroll_ops_allocate_nothing_after_warm_up() {
    let mut list = List::new(10);
    list.set_count(65_536);
    let spans: [(u32, u32); 2] = [(0, 2), (4, 5)];
    let warm: [(usize, &[(u32, u32)]); 10] = core::array::from_fn(|i| (i, &spans[..]));
    list.set_highlights(&warm);
    list.select_next();
    let (_, allocs) = count_allocations(|| {
        for i in 0..500usize {
            list.select_next();
            list.select_prev();
            list.page_down();
            list.page_up();
            list.scroll_to(i * 331 % 65_536);
            list.select(i * 7919 % 65_536);
            // Reused stack-side frame description: the refill below is
            // the only heap touch, into already-owned memory.
            let frame: [(usize, &[(u32, u32)]); 10] =
                core::array::from_fn(|r| (list.first_visible() + r, &spans[..]));
            list.set_highlights(&frame);
            list.highlights_for(list.selected().unwrap_or(0));
        }
    });
    assert_eq!(allocs, 0, "a warmed-up scroll must not allocate");
}

#[test]
fn a_65536_row_frame_costs_what_a_small_one_costs() {
    use std::time::Instant;
    // One frame: move, resolve the window, and read exactly the
    // visible rows plus their spans, as the paint does.
    fn frame(list: &mut List, rows: &[String]) -> usize {
        list.select_next();
        let mut bytes = 0;
        for row in list.visible_range() {
            if let Some(label) = rows.get(row % rows.len()) {
                bytes += label.len();
            }
            bytes += list.highlights_for(row).len() * 8;
        }
        bytes
    }
    let rows: Vec<String> = (0..32).map(|i| format!("application number {i}")).collect();
    let mut small = List::new(10);
    small.set_count(64);
    let mut big = List::new(10);
    big.set_count(65_536);
    let spans: [(u32, u32); 1] = [(0, 3)];
    // The same ten-highlight working set on both lists, so the only
    // difference the measurement can see is the row count: the scan
    // below walks highlights, never rows.
    let hi_small: [(usize, &[(u32, u32)]); 10] = core::array::from_fn(|i| (i * 6 % 64, &spans[..]));
    small.set_highlights(&hi_small);
    let hi: [(usize, &[(u32, u32)]); 10] = core::array::from_fn(|i| (i * 6000, &spans[..]));
    big.set_highlights(&hi);
    // Warm-up (caches, branch predictors, the allocator).
    for _ in 0..500 {
        frame(&mut small, &rows);
        frame(&mut big, &rows);
    }
    // Alternating rounds: at hundreds of nanoseconds a frame,
    // scheduling noise outruns any signal, so each round measures
    // both back to back and the report reads all six.
    let turns = 5_000;
    let mut sink = 0;
    let (mut small_ns, mut big_ns) = (0.0, 0.0);
    for _ in 0..3 {
        let start = Instant::now();
        for _ in 0..turns {
            sink += frame(&mut small, &rows);
        }
        let round_small = start.elapsed().as_nanos() as f64 / turns as f64;
        let start = Instant::now();
        for _ in 0..turns {
            sink += frame(&mut big, &rows);
        }
        let round_big = start.elapsed().as_nanos() as f64 / turns as f64;
        println!("round: 64 rows {round_small:.1} ns, 65536 rows {round_big:.1} ns");
        small_ns += round_small;
        big_ns += round_big;
    }
    std::hint::black_box(sink);
    let (small_ns, big_ns) = (small_ns / 3.0, big_ns / 3.0);
    println!("frame cost: 64 rows {small_ns:.1} ns, 65536 rows {big_ns:.1} ns");
    // Generous bounds (real costs are hundreds of nanoseconds): the
    // frame must be fast, and must not move with the row count.
    assert!(
        big_ns < 1_000_000.0,
        "a 65536-row frame took {big_ns:.1} ns, past the 1 ms budget"
    );
    assert!(
        big_ns < small_ns * 5.0 + 100.0,
        "frame cost moves with row count: {small_ns:.1} ns at 64 rows, {big_ns:.1} ns at 65536"
    );
}

#[test]
fn focus_tabs_forward_and_back_and_wraps() {
    let mut chain = FocusChain::new(3);
    assert_eq!(chain.focused(), Some(0));
    assert_eq!(chain.tab_next(), Some(1));
    assert_eq!(chain.tab_next(), Some(2));
    assert_eq!(chain.tab_next(), Some(0));
    assert_eq!(chain.shift_tab_prev(), Some(2));
    assert!(chain.set(1));
    assert_eq!(chain.focused(), Some(1));
    assert!(!chain.set(3));
    assert_eq!(chain.focused(), Some(1));
    chain.set_count(1);
    assert_eq!(chain.focused(), Some(0));
    let mut chain = FocusChain::new(0);
    assert_eq!(chain.focused(), None);
    assert_eq!(chain.tab_next(), None);
    assert_eq!(chain.shift_tab_prev(), None);
}

#[test]
fn activation_carries_an_owned_id() {
    let mut selector = Selector::new(5);
    assert_eq!(selector.selected(), None);
    assert!(!selector.is_expanded());
    selector.list_mut().set_count(3);
    assert_eq!(selector.selected(), Some(0));
    selector.list_mut().select_next();
    // The id is taken (owned) at activation: dropping or mutating the
    // caller's row data afterwards cannot touch it.
    let mut source = String::from("org.mozilla.firefox.desktop");
    let activated: Activate = selector.activate(source.clone()).unwrap();
    source.clear();
    assert_eq!(activated.id, "org.mozilla.firefox.desktop");
    assert_eq!(activated.index, 1);
    selector.toggle();
    assert!(selector.is_expanded());
    selector.collapse();
    assert!(!selector.is_expanded());
    selector.expand();
    // Empty selects nothing to activate.
    let empty = Selector::new(5);
    assert!(empty.activate("x").is_none());
}
