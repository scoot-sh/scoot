//! Identifiers and window metadata. Each platform assigns its own ids; the core
//! never interprets them.

use crate::geometry::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(pub u64);

/// Size limits a window asks for. A zero on either axis of either limit
/// means "none" on that axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeHints {
    pub min: Size,
    /// The largest size the window says it can take. Read only when a
    /// floating window is resized ([`Action::ResizeFloating`](crate::Action::ResizeFloating)):
    /// the strip sizes columns from the output and the minimum alone. Where
    /// it is below `min` on an axis, `min` wins.
    pub max: Size,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    /// A Wayland app_id, or a macOS bundle identifier.
    pub app_id: String,
    pub title: String,
    pub hints: SizeHints,
    /// The window this one is transient for, if the platform says so: on
    /// Wayland the `xdg_toplevel.set_parent` target, on X11 (later)
    /// `WM_TRANSIENT_FOR`. Read when the window starts floating -- a
    /// floating window is centred on its parent when the parent is on the
    /// same output ([`Action::ToggleFloating`](crate::Action::ToggleFloating)
    /// has the rules) -- and when a focused floating window closes, to hand
    /// focus back to it. An id the core does not know is ignored.
    pub parent: Option<WindowId>,
}
