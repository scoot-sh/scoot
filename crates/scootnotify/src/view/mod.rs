//! The cards and the stack: geometry ([`layout`]), pixels ([`render`]),
//! the logical-to-device mapping the pixels are drawn through ([`dev`])
//! and the motion between layouts ([`motion`]).
//!
//! Layout runs on content change; the repaint replays it. Both are pure
//! over their inputs (no Wayland, no filesystem, no clock), so the
//! geometry is a unit test and the repaint a pixel test.

pub mod dev;
pub mod layout;
pub mod motion;
pub mod render;
