//! The panel's state machine: the query entry plus the ranked list,
//! with no Wayland and no I/O, so every key, scroll and allocation pin
//! is a unit test.
//!
//! The query types into an [`Entry`](scootui::entry::Entry) (512 bytes,
//! the design's cap); each change re-ranks the index rows through the
//! matcher's [`Ranker`](crate::rank::Ranker) and collapses the row order
//! to entries (each entry once, at its best row's position; hidden
//! entries stay out). The visible rows carry greedy match spans mapped
//! back to byte offsets of the shown name, fed to
//! [`List`](scootui::entry::List) highlights for
//! [`paint_list`](scootui::entry::paint_list).
//!
//! Keys follow the design's table: typing filters; Up/Down, Ctrl+P/Ctrl+N
//! and Tab/Shift+Tab move; PageUp/PageDown page; Ctrl+Home/Ctrl+End jump;
//! Enter launches the selection; Ctrl+Enter runs the typed text;
//! Escape closes. Plain Home/End and the other editing keys stay in the
//! entry (caret moves, word kills, select-all).
//!
//! Launching is the next ticket (`launch-spawn`): selecting an entry
//! reports its id, and Ctrl+Enter reports the typed text, for the caller
//! to print. Nothing here spawns.

use scootui::entry::{Entry, KEY_TAB, List};

use crate::index;
use crate::rank::{self, Ranker, Row, frecency::Frecency};

/// The query entry's byte cap: the design's 512 B.
pub const QUERY_CAP: usize = 512;
/// Shift+Tab's keysym (`ISO_Left_Tab`): Tab with Shift held.
pub const KEY_SHIFT_TAB: u32 = 0xfe20;
/// Return's keysym, for the Enter rows below.
pub const KEY_RETURN: u32 = 0xff0d;
/// Escape's keysym: the way out.
pub const KEY_ESCAPE: u32 = 0xff1b;
/// The list's highlight frame never holds more rows than the config
/// allows visible (at most 64): a stack array, never a `Vec`.
pub const MAX_FRAME_ROWS: usize = 64;

/// What one fed key did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key typed nothing and moved nothing.
    Ignored,
    /// The query or the selection moved; redraw.
    Changed,
    /// Enter on a selection: launch this entry id (`launch-spawn`).
    Selected(String),
    /// Ctrl+Enter on a non-empty query: run this text (`launch-spawn`).
    Run(String),
    /// Escape, or the toggle socket's `close`: dismiss.
    Cancelled,
}

/// The panel's state: the widgets plus the ranking scratch, all reused
/// across keystrokes so a warmed-up keystroke allocates nothing.
pub struct Ui {
    entry: Entry,
    list: List,
    ranker: Ranker,
    entries: Vec<index::Entry>,
    rows: Vec<Row>,
    /// The entry row's index in `rows`, per entry: action rows are never
    /// shown, so highlights always come from the entry's own name field.
    entry_row_of: Vec<usize>,
    history: Frecency,
    now: u64,
    query_codes: Vec<u32>,
    displayed: Vec<usize>,
    seen: Vec<bool>,
    fold_scratch: String,
    span_scratch: Vec<(u32, u32)>,
    /// One highlight span buffer per visible slot, reused every rerank.
    highlight_slots: Vec<Vec<(u32, u32)>>,
}

impl Ui {
    /// Opens on `entries`: builds the rows once, ranks the empty query
    /// (frecency, then name), selects the top row. `visible` is the
    /// list's drawn rows. Allocates (the open, not the keystroke).
    pub fn open(entries: Vec<index::Entry>, history: Frecency, now: u64, visible: usize) -> Self {
        let rows = rank::build_rows(&entries);
        let mut entry_row_of = vec![usize::MAX; entries.len()];
        for (row, _) in rows.iter().enumerate() {
            if rows[row].action.is_none() {
                entry_row_of[rows[row].entry] = row;
            }
        }
        let slots = visible.clamp(1, MAX_FRAME_ROWS);
        let mut ui = Self {
            entry: Entry::new(QUERY_CAP),
            list: List::new(visible),
            ranker: Ranker::new(),
            entries,
            rows,
            entry_row_of,
            history,
            now,
            query_codes: Vec::new(),
            displayed: Vec::new(),
            seen: Vec::new(),
            fold_scratch: String::new(),
            span_scratch: Vec::new(),
            highlight_slots: vec![Vec::new(); slots],
        };
        ui.rerank();
        ui
    }

    /// The query text (tests: what the last keys typed).
    #[cfg(test)]
    pub fn query(&self) -> &str {
        self.entry.text()
    }

    /// The shown entries, best first (tests: the ranked order).
    #[cfg(test)]
    pub fn shown(&self) -> Vec<&index::Entry> {
        self.displayed
            .iter()
            .map(|index| &self.entries[*index])
            .collect()
    }

    /// The selected entry, if the list is non-empty.
    pub fn selected(&self) -> Option<&index::Entry> {
        self.list
            .selected()
            .map(|row| &self.entries[self.displayed[row]])
    }

    /// This frame's list state, for the draw.
    pub fn list(&self) -> &List {
        &self.list
    }

    /// This frame's entry state, for the draw.
    pub fn entry(&self) -> &Entry {
        &self.entry
    }

    /// The shown row's label: the entry name, or nothing past the end.
    /// No allocation: the draw reads through this, never a `Vec`.
    pub fn row_text(&self, row: usize) -> &str {
        self.displayed
            .get(row)
            .map(|entry| self.entries[*entry].name.as_str())
            .unwrap_or("")
    }

    /// Feeds one decoded key: `keysym` from the seat, `ctrl` whether
    /// Control is held (the app's own tracking off the modifiers mask),
    /// `text` what the seat decoded (possibly empty, possibly a control
    /// character for a Ctrl+letter without the flag).
    pub fn key(&mut self, keysym: u32, ctrl: bool, text: &str) -> Outcome {
        if keysym == KEY_ESCAPE {
            return Outcome::Cancelled;
        }
        if keysym == KEY_RETURN {
            if ctrl {
                if self.entry.is_empty() {
                    return Outcome::Ignored;
                }
                return Outcome::Run(self.entry.text().to_owned());
            }
            return match self.selected() {
                Some(entry) => Outcome::Selected(entry.id.clone()),
                None => Outcome::Ignored,
            };
        }
        if keysym == KEY_TAB {
            self.list.select_next();
            return Outcome::Changed;
        }
        if keysym == KEY_SHIFT_TAB {
            self.list.select_prev();
            return Outcome::Changed;
        }
        if self.list.apply_key(keysym, ctrl) {
            return Outcome::Changed;
        }
        if self.entry.apply_key(keysym, ctrl, text) {
            self.rerank();
            return Outcome::Changed;
        }
        Outcome::Ignored
    }

    /// Scrolls the window without moving the selection: what a pointer
    /// wheel does. `up` moves toward the first row.
    pub fn scroll(&mut self, up: bool) {
        let first = self.list.first_visible();
        if up {
            self.list.scroll_to(first.saturating_sub(1));
        } else {
            self.list.scroll_to(first.saturating_add(1));
        }
    }

    /// Clicks list row `row` (a position in the shown order, not an entry
    /// index): selects it and launches, as a mouse pick does. Past the
    /// end is ignored: [`List::select`] would clamp into range, and a
    /// click past the last row must not pick it.
    pub fn click(&mut self, row: usize) -> Outcome {
        if row >= self.list.count() {
            return Outcome::Ignored;
        }
        if self.list.select(row).is_none() {
            return Outcome::Ignored;
        }
        match self.selected() {
            Some(entry) => Outcome::Selected(entry.id.clone()),
            None => Outcome::Ignored,
        }
    }

    /// Re-ranks for the current query, rebuilds the shown order and the
    /// visible highlights, and selects the top row. Reuses every buffer:
    /// a warmed-up call allocates nothing (pinned by the test below).
    fn rerank(&mut self) {
        crate::rank::fuzzy::fold_into(self.entry.text(), &mut self.fold_scratch);
        self.query_codes.clear();
        self.query_codes
            .extend(self.fold_scratch.chars().map(|char| char as u32));
        if self.seen.len() != self.entries.len() {
            self.seen.resize(self.entries.len(), false);
        }
        self.seen.fill(false);
        self.displayed.clear();
        // `order` borrows the ranker; the collapse writes the other
        // fields, which the borrow checker sees as disjoint.
        {
            let order = self
                .ranker
                .rank(&self.fold_scratch, &self.rows, &self.history, self.now);
            self.displayed.reserve(order.len());
            for row in order.iter().copied() {
                let entry = self.rows[row as usize].entry;
                if self.entries[entry].hidden.is_some() {
                    continue;
                }
                if !self.seen[entry] {
                    self.seen[entry] = true;
                    self.displayed.push(entry);
                }
            }
        };
        self.list.set_count(self.displayed.len());
        self.list.select(0);
        self.rehighlight();
    }

    /// Recomputes the visible window's highlights into the reused slots.
    /// Only rows whose name field matches carry spans; a row matched
    /// through another field (or an action name) shows unhighlighted.
    /// The highlight frame is a stack array: no allocation.
    fn rehighlight(&mut self) {
        if self.query_codes.is_empty() {
            self.list.set_highlights(&[]);
            return;
        }
        // Fill the slot buffers first (mutable), then frame them
        // (shared): two phases, so the borrows never overlap.
        for (slot, row) in self.list.visible_range().enumerate() {
            if slot >= self.highlight_slots.len() {
                break;
            }
            let spans = &mut self.highlight_slots[slot];
            spans.clear();
            let Some(entry) = self.displayed.get(row) else {
                continue;
            };
            let row_index = self.entry_row_of[*entry];
            if row_index == usize::MAX {
                continue;
            }
            let Some(row) = self.rows.get(row_index) else {
                continue;
            };
            self.span_scratch.clear();
            if !crate::rank::fuzzy::greedy_spans(
                &row.fields[0].codes,
                &self.query_codes,
                &mut self.span_scratch,
            ) {
                continue;
            }
            let name = &self.entries[*entry].name;
            let folded = &row.name_folded;
            for (start, end) in self.span_scratch.iter().copied() {
                if let Some(byte_span) = byte_span(name, folded, &mut self.fold_scratch, start, end)
                {
                    spans.push(byte_span);
                }
            }
        }
        let mut pairs: [(usize, &[(u32, u32)]); MAX_FRAME_ROWS] = [(0, &[]); MAX_FRAME_ROWS];
        let mut count = 0;
        for (slot, row) in self.list.visible_range().enumerate() {
            if slot >= self.highlight_slots.len() || count >= MAX_FRAME_ROWS {
                break;
            }
            if self.highlight_slots[slot].is_empty() {
                continue;
            }
            pairs[count] = (row, &self.highlight_slots[slot]);
            count += 1;
        }
        self.list.set_highlights(&pairs[..count]);
    }
}

/// Maps a folded-char span to byte offsets of the shown `name`: walks the
/// name's characters with their byte offsets, advancing the folded index
/// by each character's fold length (whose concatenation is `folded` by
/// construction), recording where the span starts and ends. `None` when
/// the walk disagrees (never for spans the matcher just produced): the
/// caller draws no span rather than panicking the frame.
fn byte_span(
    name: &str,
    folded: &str,
    scratch: &mut String,
    start: u32,
    end: u32,
) -> Option<(u32, u32)> {
    let mut folded_index: u32 = 0;
    let mut byte_start: Option<u32> = None;
    let mut piece = [0u8; 4];
    for (byte, char) in name.char_indices() {
        if folded_index == start {
            byte_start = Some(byte as u32);
        }
        if folded_index == end {
            return byte_start.map(|lo| (lo, byte as u32));
        }
        // One character's fold, off the stack: no allocation.
        let piece = char.encode_utf8(&mut piece);
        scratch.clear();
        crate::rank::fuzzy::fold_into(piece, scratch);
        folded_index = folded_index.saturating_add(scratch.chars().count() as u32);
    }
    let total = folded_index;
    debug_assert_eq!(total, folded.chars().count() as u32);
    if folded_index == end {
        return byte_start.map(|lo| (lo, name.len() as u32));
    }
    None
}

#[cfg(test)]
mod tests;
