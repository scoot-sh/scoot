//! Identifiers and window metadata. Each platform assigns its own ids; the core
//! never interprets them.

use crate::geometry::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(pub u64);

/// Size limits a window asks for. A zero minimum means "none".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeHints {
    pub min: Size,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    /// A Wayland app_id, or a macOS bundle identifier.
    pub app_id: String,
    pub title: String,
    pub hints: SizeHints,
}
