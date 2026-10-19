//! The dmenu mode: lines on stdin, the pick on stdout.
//!
//! Stdin is read to EOF before the first frame (dmenu semantics), with
//! a 5 s bound on a writer that never closes, after which what arrived
//! is shown. The bounds live in [`lines`]: 1 MiB, 65,536 lines, 4 KiB
//! per line (cut at a boundary), lossy UTF-8, NUL as text. A dmenu
//! pick prints and launches nothing.
//!
//! [`DmenuUi`] is the panel's state machine for those lines: the query
//! entry plus the ranked list, with no Wayland and no I/O, so every
//! key, scroll and allocation pin is a unit test. Ranking reuses the
//! matcher's [`Ranker`](crate::rank::Ranker) over one-field rows with
//! an empty history (an empty query shows stdin order, not name
//! order: the writer chose that order, and there is no frecency to
//! break it with). A warmed-up keystroke allocates nothing (pinned by
//! the test below).
//!
//! Keys follow fuzzel's dmenu meaning: typing filters; Up/Down,
//! Ctrl+P/Ctrl+N and Tab/Shift+Tab move; PageUp/PageDown page;
//! Ctrl+Home/Ctrl+End jump; Enter prints the selection (or, without
//! `--only-match`, the typed text when nothing matches); Ctrl+Enter
//! prints the typed text even with `--only-match` (the execute-input
//! exception); Escape dismisses. Plain Home/End stay in the entry.

pub mod lines;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use scootui::entry::{Entry, KEY_TAB, List, PasswordEntry};

use crate::rank::{self, Ranker, Row, frecency::Frecency};

/// How long the stdin read waits for a writer that never closes:
/// past it what arrived is shown.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// The query entry's byte cap: the longest matchable line. Longer
/// typing is cut at the last boundary that fits, never allocated past.
pub const QUERY_CAP: usize = lines::MAX_LINE_BYTES;
/// The password entry's bytes: 1 KiB, as the widget's docs assume.
pub const SECRET_CAP: usize = 1024;
/// Shift+Tab's keysym (`ISO_Left_Tab`): Tab with Shift held.
pub const KEY_SHIFT_TAB: u32 = 0xfe20;
/// Return's keysym, for the Enter rows below.
pub const KEY_RETURN: u32 = 0xff0d;
/// Escape's keysym: the way out.
pub const KEY_ESCAPE: u32 = 0xff1b;
/// The list's highlight frame never holds more rows than the config
/// allows visible (at most 64): a stack array, never a `Vec`.
pub const MAX_FRAME_ROWS: usize = 64;

/// Reads stdin to EOF before the first frame, bounded: at most
/// [`lines::MAX_BYTES`], at most [`READ_TIMEOUT`] for a writer that
/// never closes, after which what arrived is split and shown. A
/// closed, missing or erroring stdin is empty input, never an error
/// and never a panic.
pub fn read_stdin() -> Vec<String> {
    lines::split_input(&read_fd(std::io::stdin(), READ_TIMEOUT, lines::MAX_BYTES))
}

/// Reads `file` (stdin, or a test pipe) to EOF or the cap, polling
/// with `timeout` left on the clock. The poll bounds a writer that
/// never closes; a read error ends the input with what arrived.
fn read_fd<Fd: std::os::unix::io::AsFd>(file: Fd, timeout: Duration, cap: usize) -> Vec<u8> {
    let fd = file.as_fd();
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if out.len() >= cap {
            break;
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        let mut polls = [rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN,
        )];
        let timespec = rustix::time::Timespec {
            tv_sec: remaining.as_secs() as _,
            tv_nsec: remaining.subsec_nanos() as _,
        };
        let _ = rustix::event::poll(&mut polls, Some(&timespec));
        if polls[0].revents().is_empty() {
            // Nothing readable: past the deadline the writer never
            // closes, so what arrived is the input. (A signal can end
            // the poll early with no events; the next turn re-polls
            // with what's left.)
            if Instant::now() >= deadline {
                break;
            }
            continue;
        }
        match rustix::io::read(fd, &mut chunk) {
            Ok(0) => break,
            Ok(got) => {
                let take = (cap - out.len()).min(got);
                out.extend_from_slice(&chunk[..take]);
                if take < got {
                    break;
                }
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => break,
        }
    }
    out
}

/// What one fed key did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key typed nothing and moved nothing.
    Ignored,
    /// The query or the selection moved; redraw.
    Changed,
    /// Enter or Ctrl+Enter: print this on the way out.
    Print(Print),
    /// Escape: dismiss.
    Cancelled,
}

/// What a pick prints: the selected stdin line (by index, so
/// duplicates print their own occurrence), or the typed text (Enter
/// with no match and no `--only-match`, Ctrl+Enter always).
#[derive(Debug, PartialEq, Eq)]
pub enum Print {
    Line(usize),
    Text(String),
}

/// The query entry, plain or masked. The masked entry borrows a
/// caller-owned buffer that is wiped on drop: the buffer is leaked on
/// purpose ([`DmenuUi::open_secret`]), so the borrow is `'static` and
/// the state machine holds no lifetime the Wayland state would have
/// to carry (its handles require `'static`). One 1 KiB box per open;
/// the process opens once and exits, and the drop wipes the bytes
/// first. A self-referential struct (or a new crate for one) would buy
/// nothing here.
enum EntryKind {
    Plain(Entry),
    Secret(PasswordEntry<'static>),
}

impl EntryKind {
    fn text(&self) -> &str {
        match self {
            EntryKind::Plain(entry) => entry.text(),
            EntryKind::Secret(secret) => core::str::from_utf8(secret.secret_bytes()).unwrap_or(""),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            EntryKind::Plain(entry) => entry.is_empty(),
            EntryKind::Secret(secret) => secret.is_empty(),
        }
    }

    fn apply_key(&mut self, keysym: u32, ctrl: bool, text: &str) -> bool {
        match self {
            EntryKind::Plain(entry) => entry.apply_key(keysym, ctrl, text),
            EntryKind::Secret(secret) => secret.apply_key(keysym, ctrl, text),
        }
    }
}

/// The dmenu panel's state: the query entry, the stdin lines plus
/// the ranking scratch, all reused across keystrokes so a warmed-up
/// keystroke allocates nothing.
///
/// In password mode the entry is a masked widget over a
/// caller-owned buffer (an [`Entry`] sized once to [`SECRET_CAP`] in
/// plain mode, so it never reallocates and leaves no copy in freed
/// heap): it draws `*` per character, refuses copy, paste and IME,
/// never logs the secret (its `Debug` is redacted), and is wiped on
/// drop, so an early return or an unwinding panic leaves no secret
/// behind. There is no `Debug` on this type either.
pub struct DmenuUi {
    entry: EntryKind,
    list: List,
    ranker: Ranker,
    rows: Vec<Row>,
    lines: Vec<String>,
    displayed: Vec<usize>,
    query_codes: Vec<u32>,
    fold_scratch: String,
    span_scratch: Vec<(u32, u32)>,
    /// One highlight span buffer per visible slot, reused every rerank.
    highlight_slots: Vec<Vec<(u32, u32)>>,
    only_match: bool,
    prompt_only: bool,
}

impl DmenuUi {
    /// Opens on `lines` (stdin order): an empty query shows them in
    /// order and selects the top row. `visible` is the list's drawn
    /// rows. Allocates (the open, not the keystroke).
    pub fn open(lines: Vec<String>, visible: usize, only_match: bool, prompt_only: bool) -> Self {
        Self::with_entry(
            EntryKind::Plain(Entry::new(QUERY_CAP)),
            lines,
            visible,
            only_match,
            prompt_only,
        )
    }

    /// Opens with a masked entry: the query types into a widget that
    /// draws `*` per character (never the secret) and is wiped on
    /// drop, while the list still filters the stdin lines. The widget
    /// borrows a caller-owned 1 KiB buffer, leaked for the session
    /// (see [`EntryKind`]): one box per open, wiped before the process
    /// exits.
    pub fn open_secret(
        lines: Vec<String>,
        visible: usize,
        only_match: bool,
        prompt_only: bool,
    ) -> Self {
        let buf: &'static mut [u8; SECRET_CAP] = Box::leak(Box::new([0u8; SECRET_CAP]));
        let secret = PasswordEntry::new(&mut buf[..]);
        Self::with_entry(
            EntryKind::Secret(secret),
            lines,
            visible,
            only_match,
            prompt_only,
        )
    }

    fn with_entry(
        entry: EntryKind,
        lines: Vec<String>,
        visible: usize,
        only_match: bool,
        prompt_only: bool,
    ) -> Self {
        let rows = build_rows(&lines);
        let slots = visible.clamp(1, MAX_FRAME_ROWS);
        let mut ui = Self {
            entry,
            list: List::new(if prompt_only { 0 } else { visible }),
            ranker: Ranker::new(),
            rows,
            lines,
            displayed: Vec::new(),
            query_codes: Vec::new(),
            fold_scratch: String::new(),
            span_scratch: Vec::new(),
            highlight_slots: vec![Vec::new(); slots],
            only_match,
            prompt_only,
        };
        ui.rerank();
        ui
    }

    /// The query text (tests: what the last keys typed).
    #[cfg(test)]
    pub fn query(&self) -> &str {
        self.entry.text()
    }

    /// The shown lines, best first (tests: the ranked order).
    #[cfg(test)]
    pub fn shown(&self) -> Vec<&str> {
        self.displayed
            .iter()
            .filter_map(|line| self.lines.get(*line).map(String::as_str))
            .collect()
    }

    /// The selected stdin index, if the list is non-empty.
    pub fn selected_line(&self) -> Option<usize> {
        self.list
            .selected()
            .and_then(|row| self.displayed.get(row))
            .copied()
    }

    /// This frame's list state, for the draw.
    pub fn list(&self) -> &List {
        &self.list
    }

    /// The plain entry, for the draw's [`paint_entry`](scootui::entry::paint_entry).
    /// `None` in password mode.
    pub fn plain_entry(&self) -> Option<&Entry> {
        match &self.entry {
            EntryKind::Plain(entry) => Some(entry),
            EntryKind::Secret(_) => None,
        }
    }

    /// The masked entry, for the draw's [`paint_password`](scootui::entry::paint_password).
    /// `None` unless `--password` masked it.
    pub fn secret_entry(&self) -> Option<&PasswordEntry<'static>> {
        match &self.entry {
            EntryKind::Plain(_) => None,
            EntryKind::Secret(secret) => Some(secret),
        }
    }

    /// The stdin line at `index`, if one arrived (a pick prints its
    /// own occurrence, so this reads by line index, not shown row).
    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    /// The shown row's line: the draw reads through this, never a
    /// `Vec`. Past the end is `""`.
    pub fn row_text(&self, row: usize) -> &str {
        self.displayed
            .get(row)
            .and_then(|line| self.lines.get(*line))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Feeds one decoded key: `keysym` from the seat, `ctrl` whether
    /// Control is held (the app's own tracking off the modifiers mask),
    /// `text` what the seat decoded (possibly empty).
    pub fn key(&mut self, keysym: u32, ctrl: bool, text: &str) -> Outcome {
        if keysym == KEY_ESCAPE {
            return Outcome::Cancelled;
        }
        if keysym == KEY_RETURN {
            if ctrl {
                // The execute-input exception: Ctrl+Enter prints the
                // typed text even with `--only-match`.
                if self.entry.is_empty() {
                    return Outcome::Ignored;
                }
                return Outcome::Print(Print::Text(self.entry.text().to_owned()));
            }
            return match self.selected_line() {
                Some(line) => Outcome::Print(Print::Line(line)),
                None if self.only_match => Outcome::Ignored,
                None if self.entry.is_empty() => Outcome::Ignored,
                None => Outcome::Print(Print::Text(self.entry.text().to_owned())),
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
        // Plain Home/End are the entry's caret keys; the list takes them
        // only with Ctrl (the design's key table). `List::apply_key`
        // would swallow both, so they skip it.
        let for_the_entry =
            !ctrl && (keysym == scootui::entry::KEY_HOME || keysym == scootui::entry::KEY_END);
        if !for_the_entry && self.list.apply_key(keysym, ctrl) {
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

    /// Clicks list row `row` (a position in the shown order, not a line
    /// index): prints that occurrence. Past the end is ignored:
    /// [`List::select`] would clamp into range, and a click past the
    /// last row must not print it. A prompt-only panel shows no list,
    /// so every click is ignored there.
    pub fn click(&mut self, row: usize) -> Outcome {
        if self.prompt_only || row >= self.list.count() {
            return Outcome::Ignored;
        }
        if self.list.select(row).is_none() {
            return Outcome::Ignored;
        }
        match self.selected_line() {
            Some(line) => Outcome::Print(Print::Line(line)),
            None => Outcome::Ignored,
        }
    }

    /// Re-ranks for the current query and rebuilds the shown order and
    /// the visible highlights. An empty query shows stdin order (the
    /// writer's order, with no frecency to break it); a typed query
    /// ranks through the matcher's order (ties keep stdin order).
    /// Reuses every buffer: a warmed-up call allocates nothing.
    fn rerank(&mut self) {
        crate::rank::fuzzy::fold_into(self.entry.text(), &mut self.fold_scratch);
        self.query_codes.clear();
        self.query_codes
            .extend(self.fold_scratch.chars().map(|char| char as u32));
        self.displayed.clear();
        // A prompt-only panel shows no list: nothing is ever shown or
        // selected, whatever the query.
        if self.prompt_only {
            // Nothing to show.
        } else if self.fold_scratch.is_empty() {
            self.displayed.extend(0..self.lines.len());
        } else {
            // `order` borrows the ranker; the extend reads the other
            // fields, which the borrow checker sees as disjoint.
            let order = self
                .ranker
                .rank(&self.fold_scratch, &self.rows, &Frecency::empty(), 0);
            self.displayed.reserve(order.len());
            self.displayed.extend(order.iter().map(|row| *row as usize));
        }
        self.list.set_count(self.displayed.len());
        self.list.select(0);
        self.rehighlight();
    }

    /// Recomputes the visible window's highlights into the reused slots.
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
            let Some(line) = self.displayed.get(row).copied() else {
                continue;
            };
            let Some(row) = self.rows.get(line) else {
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
            let Some(line) = self.lines.get(line) else {
                continue;
            };
            for (start, end) in self.span_scratch.iter().copied() {
                if let Some(byte_span) =
                    crate::ui::byte_span(line, &row.name_folded, &mut self.fold_scratch, start, end)
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

/// Builds the searchable rows for `lines`: one single-field row per
/// stdin line. Duplicates stay separate rows (each occurrence prints
/// its own index); the id stays empty (no frecency without
/// `--history`, and nothing else reads it).
fn build_rows(lines: &[String]) -> Vec<Row> {
    let mut rows = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let folded = {
            let mut folded = String::new();
            crate::rank::fuzzy::fold_into(line, &mut folded);
            folded
        };
        let codes: Vec<u32> = folded.chars().map(|char| char as u32).collect();
        let mask = crate::rank::fuzzy::mask_of_codes(&codes);
        rows.push(Row {
            entry: index,
            action: None,
            name_folded: folded,
            id: String::new(),
            fields: [
                rank::Field {
                    codes,
                    mask,
                    weight: rank::fuzzy::WEIGHT_NAME,
                },
                rank::Field {
                    codes: Vec::new(),
                    mask: 0,
                    weight: rank::fuzzy::WEIGHT_GENERIC,
                },
                rank::Field {
                    codes: Vec::new(),
                    mask: 0,
                    weight: rank::fuzzy::WEIGHT_KEYWORDS,
                },
                rank::Field {
                    codes: Vec::new(),
                    mask: 0,
                    weight: rank::fuzzy::WEIGHT_EXEC,
                },
            ],
        });
    }
    rows
}
