//! One frame of a panel with a shadow or a motion: the part of the draw
//! that runs on the stage instead of straight into the buffer.
//!
//! A free function over plain values and a made-up clock, so a test draws
//! the entrance, the fade and the sliding highlight at chosen instants and
//! reads the pixels back, without a compositor.

use std::time::Instant;

use scootui::output::Size;

use super::{PanelView, paint_panel, scale_len};
use crate::icons;
use crate::motion::slide;
use crate::stage::{Effects, Scene, fit_room};
use scootui::text::Text;

/// Draws the panel into `out` (`dims` pixels) at `now`: the shadow room as
/// the compositor left it, the panel repainted when `dirty` (its content
/// changed) or the highlight is somewhere else than it was drawn, then the stage presented through
/// the motion's pose. Returns the panel's origin in the surface, in
/// logical pixels, for the hit tests and the input region.
///
/// `frame_size` is the surface's logical size. Needs `fx.stage`; a layer
/// that cannot be made (a size past any output) draws the plain panel at
/// the corner and answers `(0, 0)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_staged(
    fx: &mut Effects,
    view: &PanelView<'_>,
    text: &mut Text,
    icons: &mut icons::Cache,
    out: &mut [u8],
    dims: (u32, u32),
    frame_size: Size,
    dirty: bool,
    now: Instant,
) -> (u32, u32) {
    let Some(stage) = fx.stage.as_mut() else {
        paint_panel(out, dims, view, text, icons, None);
        return (0, 0);
    };
    let layout = super::device_layout(view.look, view.scale);
    let factor = view.scale.factor().max(super::MIN_FACTOR);
    // The shadow's room, as the compositor left it: the panel stays where
    // it was and the shadow gives way.
    let room = fit_room(
        fx.room,
        (frame_size.width, frame_size.height),
        (view.width, view.height),
    );
    let len = |logical| scale_len(logical, factor);
    let panel = (
        len(view.width).min(dims.0).max(1),
        len(view.height).min(dims.1).max(1),
    );
    let origin = (
        len(room.0).min(dims.0.saturating_sub(panel.0)),
        len(room.1).min(dims.1.saturating_sub(panel.1)),
    );
    fx.motion.start(now);
    let visible = view.ui.list().visible_range();
    let slide = slide(
        &mut fx.motion,
        view.ui
            .list()
            .selected()
            .filter(|row| visible.contains(row))
            .map(|row| row - visible.start),
        visible.len(),
        layout.row_h,
        view.ui.generation() == fx.generation,
        now,
    );
    fx.generation = view.ui.generation();
    let scene = Scene {
        surface: dims,
        panel,
        origin,
        factor,
        radius: len(view.look.radius),
        tint: view.look.focus,
    };
    if dirty || fx.slide != slide || fx.composed != Some(scene) {
        let painted = stage.compose(&scene, |pixels, panel_dims| {
            paint_panel(pixels, panel_dims, view, text, icons, slide);
        });
        fx.composed = painted.then_some(scene);
        fx.slide = slide;
    }
    if fx.composed.is_none() {
        paint_panel(out, dims, view, text, icons, None);
        return (0, 0);
    }
    stage.present(out, dims, fx.motion.pose(now));
    room
}
