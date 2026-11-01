//! One frame of a panel with a shadow or a motion: the part of the draw
//! that runs on the stage instead of straight into the buffer.
//!
//! A free function over plain values and a made-up clock, so a test draws
//! the entrance, the fade and the sliding highlight at chosen instants and
//! reads the pixels back, without a compositor.

use std::time::Instant;

use scootui::color::Color;
use scootui::density::Scale;
use scootui::output::Size;

use super::{PanelView, paint_panel};
use crate::icons;
use crate::layout::scale_len;
use crate::motion::{Slide, slide};
use crate::stage::{Effects, Scene, fit_room};
use scootui::text::Text;

/// What the stage needs to know about a panel, apart from how it is drawn.
pub(super) struct Shape {
    /// The panel's logical size.
    pub size: (u32, u32),
    /// Its corner radius, logical.
    pub radius: u32,
    /// What a glow is tinted with.
    pub tint: Color,
    pub scale: Scale,
}

/// A list whose selection highlight slides: where it is and whether the
/// rows are the ones the last frame had.
pub(super) struct Sliding {
    /// The selected row's slot among the shown rows, if it is on screen.
    pub selected: Option<usize>,
    /// How many rows are shown.
    pub slots: usize,
    /// A row's height, in device pixels.
    pub row_h: u32,
    /// The rows' generation ([`crate::ui::Ui::generation`]).
    pub generation: u64,
}

/// Draws the apps or run panel into `out` (`dims` pixels) at `now`: the
/// shadow room as the compositor left it, the panel repainted when `dirty`
/// (its content changed) or the highlight is somewhere else than it was
/// drawn, then the stage presented through the motion's pose. Returns the
/// panel's origin in the surface, in logical pixels, for the hit tests and
/// the input region.
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
    let shape = Shape {
        size: (view.width, view.height),
        radius: view.look.radius,
        tint: view.look.focus,
        scale: view.scale,
    };
    let visible = view.ui.list().visible_range();
    let sliding = Sliding {
        selected: view
            .ui
            .list()
            .selected()
            .filter(|row| visible.contains(row))
            .map(|row| row - visible.start),
        slots: visible.len(),
        row_h: super::device_layout(view.look, view.scale, view.height, view.rows).row_h,
        generation: view.ui.generation(),
    };
    stage_frame(
        fx,
        &shape,
        Some(sliding),
        out,
        dims,
        frame_size,
        dirty,
        now,
        |pixels, panel_dims, slide| paint_panel(pixels, panel_dims, view, text, icons, slide),
    )
}

/// The stage's frame for any panel: `paint` draws the panel (into a slice
/// sized to it, with the highlight's place when `sliding` slides one), and
/// the rest is [`paint_staged`]'s. A panel with no list to slide passes
/// `None` and gets `None` back in `paint`.
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_frame(
    fx: &mut Effects,
    shape: &Shape,
    sliding: Option<Sliding>,
    out: &mut [u8],
    dims: (u32, u32),
    frame_size: Size,
    dirty: bool,
    now: Instant,
    mut paint: impl FnMut(&mut [u8], (u32, u32), Option<Slide>),
) -> (u32, u32) {
    let Some(stage) = fx.stage.as_mut() else {
        paint(out, dims, None);
        return (0, 0);
    };
    let factor = shape.scale.factor().max(super::MIN_FACTOR);
    // The shadow's room, as the compositor left it: the panel stays where
    // it was and the shadow gives way.
    let room = fit_room(fx.room, (frame_size.width, frame_size.height), shape.size);
    let len = |logical| scale_len(logical, shape.scale);
    let panel = (
        len(shape.size.0).min(dims.0).max(1),
        len(shape.size.1).min(dims.1).max(1),
    );
    let origin = (
        len(room.0).min(dims.0.saturating_sub(panel.0)),
        len(room.1).min(dims.1.saturating_sub(panel.1)),
    );
    fx.motion.start(now);
    let slide = sliding.and_then(|sliding| {
        let placed = slide(
            &mut fx.motion,
            sliding.selected,
            sliding.slots,
            sliding.row_h,
            sliding.generation == fx.generation,
            now,
        );
        fx.generation = sliding.generation;
        placed
    });
    let scene = Scene {
        surface: dims,
        panel,
        origin,
        factor,
        radius: len(shape.radius),
        tint: shape.tint,
    };
    if dirty || fx.slide != slide || fx.composed != Some(scene) {
        let painted = stage.compose(&scene, |pixels, panel_dims| {
            paint(pixels, panel_dims, slide);
        });
        fx.composed = painted.then_some(scene);
        fx.slide = slide;
    }
    if fx.composed.is_none() {
        paint(out, dims, None);
        return (0, 0);
    }
    stage.present(out, dims, fx.motion.pose(now));
    room
}
