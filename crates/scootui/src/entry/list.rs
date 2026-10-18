//! The virtualized list (unbounded rows, only the visible ones drawn),
//! the inline selector, and activation carrying an owned id.
//!
//! The list owns no row text: rows live in the caller's data (the
//! launcher's ranked index, the greeter's user list) and are read
//! through callbacks at draw time. What the list owns is three scalars
//! (`count`, `selected`, `first`) plus one reused highlight scratch, so
//! scrolling is scalar arithmetic and a frame touches only its visible
//! rows: the per-frame cost does not move with the row count.

use super::line::{
    KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_PAGE_DOWN, KEY_PAGE_UP, KEY_RIGHT,
    KEY_UP,
};
use core::ops::Range;

/// Match-highlight spans stored per row at most: the matcher (the
/// launcher's fuzzy scorer) reports a few spans per visible row, never
/// hundreds. Bounding the array keeps [`Highlight`] `Copy` and the
/// scratch reuse exact.
pub const MAX_SPANS_PER_ROW: usize = 8;

/// Highlight rows remembered at most: the caller sets highlights for
/// the visible window each frame (tens of rows), not for the whole
/// index. Past this, further rows are dropped, never an error: a stale
/// highlight is a cosmetic miss, and an unbounded scratch is a memory
/// miss.
pub const MAX_HIGHLIGHT_ROWS: usize = 128;

/// One row's match-highlight spans: `(start, end)` byte offsets into
/// the row's text, drawn in the match color. Spans past the row's
/// length draw nothing (clipped at paint), so a matcher racing a
/// re-rank cannot panic the draw. Plain pairs (not `Range`: it is not
/// `Copy`, and the scratch reuses these by value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    /// Which row.
    pub row: usize,
    /// How many of `spans` are live.
    pub span_count: usize,
    /// The spans, `span_count` first live.
    pub spans: [(u32, u32); MAX_SPANS_PER_ROW],
}

impl Highlight {
    /// Empty: no spans. What rows without a match read as.
    pub const EMPTY: Self = Self {
        row: 0,
        span_count: 0,
        spans: [(0, 0); MAX_SPANS_PER_ROW],
    };
}

/// What picking a row asks of the app: the row's `index` plus its `id`,
/// owned. The id names what the caller's data owns (a desktop file id,
/// a username, a session id): it is taken at activation time into a
/// `String`, so the caller can re-rank or drop its index afterwards
/// without the activation dangling. Activations are rare (one per
/// launch or login), so the one allocation is on no hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activate {
    pub id: String,
    pub index: usize,
}

/// A virtualized list: `count` rows the caller owns, a selection, and
/// a visible window the draw reads.
///
/// Selection wraps at the ends (Up from the first row lands on the
/// last): a launcher or greeter list with one row still moves, and a
/// held key never sticks. An empty list selects nothing and ignores
/// every move.
#[derive(Debug, Clone)]
pub struct List {
    count: usize,
    selected: usize,
    first: usize,
    visible: usize,
    highlights: Vec<Highlight>,
}

impl List {
    /// Empty, drawing `visible` rows per frame. The highlight scratch
    /// sizes itself on the first [`List::set_highlights`].
    pub fn new(visible: usize) -> Self {
        Self {
            count: 0,
            selected: 0,
            first: 0,
            visible: visible.max(1),
            highlights: Vec::new(),
        }
    }

    /// How many rows the caller holds. Clamps the selection and the
    /// window into range; an empty list selects nothing.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        if count == 0 {
            self.selected = 0;
            self.first = 0;
        } else {
            self.selected = self.selected.min(count - 1);
            self.first = self.first.min(count - 1);
            self.clamp_window();
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// Rows drawn per frame (the surface height in rows). At least 1.
    pub fn set_visible(&mut self, visible: usize) {
        self.visible = visible.max(1);
        self.clamp_window();
    }

    pub fn visible(&self) -> usize {
        self.visible
    }

    /// The selected row, or `None` when empty.
    pub fn selected(&self) -> Option<usize> {
        (self.count > 0).then_some(self.selected)
    }

    /// The first drawn row.
    pub fn first_visible(&self) -> usize {
        self.first
    }

    /// The drawn window: `first .. min(first + visible, count)`, empty
    /// when the list is. Computing it touches the three scalars only,
    /// so its cost is independent of the row count; the draw then reads
    /// exactly these rows through the caller's callback and no others.
    pub fn visible_range(&self) -> Range<usize> {
        let end = self.first.saturating_add(self.visible).min(self.count);
        self.first.min(end)..end
    }

    /// Select `index` (clamped into range; ignored when empty) and
    /// scroll it into view. Returns the selection.
    pub fn select(&mut self, index: usize) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.selected = index.min(self.count - 1);
        self.ensure_visible();
        Some(self.selected)
    }

    /// One row down, wrapping to the first.
    pub fn select_next(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.selected = if self.selected + 1 >= self.count {
            0
        } else {
            self.selected + 1
        };
        self.ensure_visible();
        Some(self.selected)
    }

    /// One row up, wrapping to the last.
    pub fn select_prev(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.selected = self.selected.checked_sub(1).unwrap_or(self.count - 1);
        self.ensure_visible();
        Some(self.selected)
    }

    /// One window down (the selection moves with it).
    pub fn page_down(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.selected = self
            .selected
            .saturating_add(self.visible)
            .min(self.count - 1);
        self.ensure_visible();
        Some(self.selected)
    }

    /// One window up.
    pub fn page_up(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.selected = self.selected.saturating_sub(self.visible);
        self.ensure_visible();
        Some(self.selected)
    }

    /// First row.
    pub fn home(&mut self) -> Option<usize> {
        self.select(0)
    }

    /// Last row.
    pub fn end(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.select(self.count - 1)
    }

    /// Scroll the window so `index` shows (clamped), without moving the
    /// selection: what a pointer wheel does.
    pub fn scroll_to(&mut self, index: usize) {
        if self.count == 0 {
            self.first = 0;
            return;
        }
        self.first = index.min(self.count - 1);
        self.clamp_window();
    }

    /// Feed one decoded key: arrows move, PageUp/PageDown page,
    /// Home/End jump (Ctrl+Home/End too), Ctrl+P/Ctrl+N move like
    /// Up/Down for the launcher's Emacs hands. Left/Right do nothing
    /// here (the entry keeps them). Enter, Escape and Tab travel on to
    /// the app. Returns whether the key moved anything.
    pub fn apply_key(&mut self, keysym: u32, ctrl: bool) -> bool {
        if ctrl {
            let lower = ascii_lower(keysym);
            if lower == u32::from(b'n') || keysym == 0x0e {
                return self.select_next().is_some();
            }
            if lower == u32::from(b'p') || keysym == 0x10 {
                return self.select_prev().is_some();
            }
            if keysym == KEY_HOME || keysym == KEY_END {
                // Ctrl+Home/Ctrl+End handled below with the plain ones.
            } else {
                return false;
            }
        }
        match keysym {
            KEY_DOWN => self.select_next().is_some(),
            KEY_UP => self.select_prev().is_some(),
            KEY_PAGE_DOWN => self.page_down().is_some(),
            KEY_PAGE_UP => self.page_up().is_some(),
            KEY_HOME => self.home().is_some(),
            KEY_END => self.end().is_some(),
            KEY_LEFT | KEY_RIGHT | KEY_DELETE => false,
            _ => false,
        }
    }

    /// Remember match-highlight spans for the visible window. The
    /// caller passes this frame's rows; the scratch is cleared and
    /// refilled, keeping its memory, so a warmed-up frame allocates
    /// nothing. Rows past [`MAX_HIGHLIGHT_ROWS`] are dropped. Each
    /// row keeps at most [`MAX_SPANS_PER_ROW`] spans.
    pub fn set_highlights(&mut self, rows: &[(usize, &[(u32, u32)])]) {
        self.highlights.clear();
        for (row, spans) in rows.iter().take(MAX_HIGHLIGHT_ROWS) {
            let mut kept = [(0, 0); MAX_SPANS_PER_ROW];
            let mut count = 0;
            for (lo, hi) in spans.iter().take(MAX_SPANS_PER_ROW) {
                if let Some(slot) = kept.get_mut(count) {
                    *slot = (*lo, *hi);
                    count += 1;
                }
            }
            // `clear` kept the capacity and the row bound caps the
            // push, so a warmed-up call only writes into owned memory:
            // the first fills size the scratch to the working set.
            self.highlights.push(Highlight {
                row: *row,
                span_count: count,
                spans: kept,
            });
        }
    }

    /// This frame's highlights, in the order they were set.
    pub fn highlights(&self) -> &[Highlight] {
        &self.highlights
    }

    /// The spans for `row`, or nothing when the row has no match. A
    /// linear scan over at most the visible window's rows.
    pub fn highlights_for(&self, row: usize) -> &[(u32, u32)] {
        for highlight in &self.highlights {
            if highlight.row == row {
                if let Some(live) = highlight.spans.get(..highlight.span_count) {
                    return live;
                }
                break;
            }
        }
        &[]
    }

    /// Move the window the minimum that shows the selection.
    fn ensure_visible(&mut self) {
        if self.count == 0 {
            self.first = 0;
            return;
        }
        if self.selected < self.first {
            self.first = self.selected;
        } else {
            let end = self.first.saturating_add(self.visible);
            if self.selected >= end {
                self.first = self.selected.saturating_sub(self.visible) + 1;
            }
        }
    }

    /// Keep the window inside the list: `first + visible` past the end
    /// pulls `first` back (never past 0), so a shrinking filter never
    /// strands the draw below the last row.
    fn clamp_window(&mut self) {
        if self.count == 0 {
            self.first = 0;
            return;
        }
        let max_first = self.count.saturating_sub(self.visible.min(self.count));
        self.first = self.first.min(max_first);
    }
}

/// ASCII case-fold, as in the entry: Ctrl+N and Ctrl+n are the same key.
fn ascii_lower(keysym: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&keysym) {
        keysym + (u32::from(b'a') - u32::from(b'A'))
    } else {
        keysym
    }
}

/// An inline selector: one line showing the current choice, expanding
/// to a [`List`] in place (no `xdg_popup`: the greeter draws user and
/// session in the same card surface). Collapsed it behaves as one
/// focus stop; expanded, Up/Down (and the list's other keys) move
/// through the rows and Enter activates the choice with its owned id.
#[derive(Debug, Clone)]
pub struct Selector {
    list: List,
    expanded: bool,
}

impl Selector {
    pub fn new(visible: usize) -> Self {
        Self {
            list: List::new(visible),
            expanded: false,
        }
    }

    /// The rows, selected and scrolled exactly like a [`List`].
    pub fn list(&self) -> &List {
        &self.list
    }

    pub fn list_mut(&mut self) -> &mut List {
        &mut self.list
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn expand(&mut self) {
        self.expanded = true;
    }

    pub fn collapse(&mut self) {
        self.expanded = false;
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    /// The choice: the selected row, or `None` when empty.
    pub fn selected(&self) -> Option<usize> {
        self.list.selected()
    }

    /// Activate the choice with its caller-owned `id`: the username,
    /// the session id, whatever names the row in the caller's data.
    /// `None` when nothing is selected.
    pub fn activate(&self, id: impl Into<String>) -> Option<Activate> {
        self.list.selected().map(|index| Activate {
            id: id.into(),
            index,
        })
    }
}
