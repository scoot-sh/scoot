//! The volume module's built-in level icons: a speaker, its waves and its
//! cross, as SVG path data in the default `0 0 24 24` viewbox. Four levels
//! (muted, low, medium, high), drawn through the bar's own path rasterizer
//! like any `icon-path`, tinted from the view's class.
//!
//! Only fills: the rasterizer fills paths, so the waves are closed
//! crescents and the cross a closed polygon, never strokes. Only `M`, `L`,
//! `C` and `z`: no arcs, whose sweep flag is one silent mirror-image bug
//! waiting to happen.

/// Muted: the speaker and its cross.
pub const MUTED: &str = "M4 9v6h4l5 4V5L8 9H4zM16.3 9.3L17.9 9.3 19.6 11 21.3 9.3 22.9 9.3 21.2 12 22.9 14.7 21.3 14.7 19.6 13 17.9 14.7 16.3 14.7 18 12z";
/// Low: the speaker alone.
pub const LOW: &str = "M4 9v6h4l5 4V5L8 9H4z";
/// Medium: the speaker and the first wave.
pub const MEDIUM: &str = "M4 9v6h4l5 4V5L8 9H4zM16 8.5C18.5 10 18.5 14 16 15.5L14.2 13.9C15.5 13 15.5 11 14.2 10.1z";
/// High: the speaker and both waves.
pub const HIGH: &str = "M4 9v6h4l5 4V5L8 9H4zM16 8.5C18.5 10 18.5 14 16 15.5L14.2 13.9C15.5 13 15.5 11 14.2 10.1zM18.5 6C21.5 9 21.5 15 18.5 18L16.7 16.2C18.5 14.5 18.5 9.5 16.7 7.8z";

/// The four levels in slot order: muted, low, medium, high.
pub const PATHS: [&str; 4] = [MUTED, LOW, MEDIUM, HIGH];

/// The level's slot in [`PATHS`]: muted, low, medium or high.
pub fn index(muted: bool, percent: u32) -> usize {
    if muted {
        0
    } else if percent < 34 {
        1
    } else if percent < 67 {
        2
    } else {
        3
    }
}
