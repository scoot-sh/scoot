//! An output's identity for the tooltip and popup state machines: one id per
//! bound output, never reused for the daemon's life.
//!
//! Extracted from `scootbar`'s `outputs.rs` so the popup's pure half
//! (`popup::hover`) has no Wayland scaffolding to reach for. The per-output
//! model itself moves in `scootui-extract-wayland`; this id is the only
//! part the drawing half needs.

/// Identifies one bound output for the daemon's whole life; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputId(pub u64);

/// A width and height, in whatever unit the context says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}
