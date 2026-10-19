//! Do-not-disturb: a mode, not a filter list.
//!
//! While on, new notifications skip the screen and go straight to
//! history and the unread count — except critical ones, which still
//! show when the config allows (`dnd-allows-critical`, the default),
//! and cards a rule lets through (`dnd-bypass`). Expiry for cards that
//! skipped the screen never starts, so they stay unread until seen.
//! Turning DND off shows nothing retroactively; `restore` does.
//!
//! The mode survives a reload (a config re-read never touches it) but
//! never a restart: it starts off, every time.

/// Whether new cards show or hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dnd {
    /// New cards show (critical or not).
    #[default]
    Off,
    /// New cards hold in history, except criticals and bypasses.
    On,
}

impl Dnd {
    /// The `msg dnd` word for this mode.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }

    /// Whether the mode holds `critical`: the opposite of showing it.
    pub fn holds(self) -> bool {
        matches!(self, Self::On)
    }
}

#[cfg(test)]
mod tests;
