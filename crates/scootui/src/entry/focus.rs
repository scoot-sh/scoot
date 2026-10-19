//! The focus chain: Tab/Shift+Tab across a panel's focus stops, with a
//! visible focus ring on the focused widget.
//!
//! The chain is positions only: position 0 is the entry, 1 the list,
//! and so on, as the app wires them. Wrapping both ways (Tab from the
//! last stop lands on the first), so every stop is reachable from every
//! other with one hand. No stops (`count` 0) focuses nothing.

/// Tab/Shift+Tab across `count` focus stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusChain {
    count: usize,
    current: usize,
}

impl FocusChain {
    /// Focused on stop 0 (or nothing, when `count` is 0).
    pub fn new(count: usize) -> Self {
        Self { count, current: 0 }
    }

    /// Rewire to `count` stops (a panel whose rows appear or go away),
    /// keeping the position inside range.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        if count == 0 {
            self.current = 0;
        } else {
            self.current = self.current.min(count - 1);
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// The focused stop, or `None` when there are no stops.
    pub fn focused(&self) -> Option<usize> {
        (self.count > 0).then_some(self.current)
    }

    /// Tab: one stop forward, wrapping to the first. Named for the
    /// key rather than `next` (which clippy reads as the iterator
    /// method): the chain is not an iterator.
    pub fn tab_next(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.current = if self.current + 1 >= self.count {
            0
        } else {
            self.current + 1
        };
        Some(self.current)
    }

    /// Shift+Tab: one stop back, wrapping to the last.
    pub fn shift_tab_prev(&mut self) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        self.current = self.current.checked_sub(1).unwrap_or(self.count - 1);
        Some(self.current)
    }

    /// Jump to `index`. False (and no move) past the last stop.
    pub fn set(&mut self, index: usize) -> bool {
        if index < self.count {
            self.current = index;
            true
        } else {
            false
        }
    }
}
