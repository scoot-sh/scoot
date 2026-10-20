//! The in-memory history ring: closed cards and held ones, newest
//! last, capped by `history_max` (64 by default, at most 256).
//!
//! An entry holds app, summary, body (capped at parse), urgency,
//! category, time and an icon *name or path* — never `image-data`
//! pixels. An entry enters when its card closes for any reason, or
//! when it skips the screen (DND, a rule's `hide`); `transient`
//! notifications never enter. The unread count is the entries not yet
//! seen on screen. `restore` re-shows the newest entry as a new card
//! with a new id and marks that entry seen.

use crate::bus::hints::Urgency;
use crate::model::store::{Icon, Notification};

/// What one closed or held card leaves behind: everything `msg
/// history` shows, nothing that draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub app_name: String,
    pub summary: String,
    /// Markup stripped, as the JSON view shows it.
    pub body: String,
    pub urgency: Urgency,
    pub category: String,
    pub desktop_entry: String,
    /// A name or path, never pixels: `Icon::Data` reads as none.
    pub icon: Option<IconName>,
    /// CLOCK_REALTIME ms at arrival.
    pub created_ms: u64,
    /// Whether the card was ever on screen.
    pub seen: bool,
}

/// The icon a history entry keeps: a theme name or a file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconName {
    Name(String),
    Path(String),
}

impl HistoryEntry {
    /// Builds the entry for `note`: `seen` says whether the card was
    /// ever presented. Pixels never cross into history.
    pub fn of(note: &Notification, seen: bool) -> Self {
        let icon = match &note.icon {
            Icon::None | Icon::Data(_) => None,
            Icon::Name(name) => Some(IconName::Name(name.clone())),
            Icon::Path(path) => Some(IconName::Path(path.clone())),
        };
        Self {
            app_name: note.app_name.clone(),
            summary: note.summary.clone(),
            body: note.plain_body.clone(),
            urgency: note.urgency,
            category: note.category.clone(),
            desktop_entry: note.desktop_entry.clone(),
            icon,
            created_ms: note.created_ms,
            seen,
        }
    }
}

/// The ring: entries oldest first, capped at the config's
/// `history_max` (a 0 cap keeps none). Pure state, no I/O.
#[derive(Debug, Default)]
pub struct History {
    entries: std::collections::VecDeque<HistoryEntry>,
}

impl History {
    /// Pushes `entry`, evicting the oldest past `cap`. A 0 cap keeps
    /// nothing (the push is a no-op).
    pub fn push(&mut self, entry: HistoryEntry, cap: u32) {
        if cap == 0 {
            return;
        }
        while self.entries.len() >= cap as usize {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Truncates to `cap`, keeping the newest (a reload's smaller cap
    /// never reorders, only drops the oldest).
    pub fn truncate(&mut self, cap: u32) {
        while self.entries.len() > cap as usize {
            self.entries.pop_front();
        }
    }

    /// The newest entry, if any: what `restore` re-shows.
    pub fn newest(&self) -> Option<&HistoryEntry> {
        self.entries.back()
    }

    /// Marks the newest entry seen: it is on screen again.
    pub fn mark_newest_seen(&mut self) {
        if let Some(newest) = self.entries.back_mut() {
            newest.seen = true;
        }
    }

    /// The entries not yet seen on screen.
    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.seen).count()
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries oldest first: the JSON view reads this.
    pub fn iter(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests;
