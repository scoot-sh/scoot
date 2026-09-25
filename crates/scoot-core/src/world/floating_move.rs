//! Moving and resizing floating windows: where the user puts one is kept.
//!
//! The rules are on [`Action::MoveFloating`](crate::Action::MoveFloating) and
//! [`Action::ResizeFloating`](crate::Action::ResizeFloating). This file is
//! how they are kept:
//!
//! - **One placement function.** [`floating_rect`] is where a floating
//!   window goes -- `arrange` places every floating window with it, and
//!   [`World::floating_geometry`] answers with it for one window -- so a
//!   shell that moves a window between arrangements (a pointer drag, once
//!   per motion) puts it exactly where the next arrangement will.
//! - **The anchor, not the rect, is the state.** A move or a resize writes
//!   the window's [`Anchor`]: the point it is held by and which point of the
//!   window that is. The rect is derived from it and the size the window
//!   last drew, so the size a client actually settles on (a terminal
//!   rounding to whole cells, a window refusing to go below its minimum)
//!   never moves the edge the user did not drag.
//! - **Nothing here allocates**, short of a window crossing onto another
//!   output: a pointer drag calls these once per motion event, at the
//!   device's rate.

use super::tree::{Align, Anchor, Output, Slot, WindowState};
use super::{Location, World};
use crate::geometry::{Point, Rect, Size};
use crate::messages::Edges;
use crate::types::{OutputId, WindowId};

/// Where one floating window is, as [`World::floating_geometry`] reports it:
/// the same numbers [`World::arrange`] places it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatingGeometry {
    /// The output it is on.
    pub output: OutputId,
    /// Where it is placed, in global logical coordinates.
    pub rect: Rect,
    /// The size to ask it to take -- `None` while it chooses its own (see
    /// [`Placement::requested`](crate::Placement::requested)).
    pub requested: Option<Size>,
}

/// A floating window's placement, before the workspace decides whether it
/// shows.
pub(super) struct Placed {
    pub(super) rect: Rect,
    pub(super) requested: Option<Size>,
    /// It has a size to be placed at: it has drawn, or it was asked for one.
    pub(super) sized: bool,
    /// Its output has a usable area to place it in.
    pub(super) fits: bool,
}

impl World {
    /// Where a floating window is placed and what it is asked to be, read
    /// without arranging everything else: what a shell moving a window once
    /// per pointer motion reads back, so the window lands exactly where the
    /// next [`World::arrange`] will put it.
    ///
    /// `None` for an unknown or tiled window, one waiting for an output, and
    /// a fullscreen one (which is placed over its whole output, not here).
    /// Says nothing about whether the window shows; `arrange` decides that.
    /// Allocation-free: one walk of the tree to find the window, and the
    /// placement arithmetic.
    pub fn floating_geometry(&self, id: WindowId) -> Option<FloatingGeometry> {
        let loc = self.floating_location(id)?;
        let window = self.windows.get(&id)?;
        let output = self.outputs.get(loc.output)?;
        let placed = floating_rect(window, output);
        Some(FloatingGeometry {
            output: output.id,
            rect: placed.rect,
            requested: placed.requested,
        })
    }

    /// Where a floating, non-fullscreen window is: the windows a move or a
    /// resize applies to.
    fn floating_location(&self, id: WindowId) -> Option<Location> {
        let window = self.windows.get(&id)?;
        if window.floating.is_none() || window.fullscreen.is_some() {
            return None;
        }
        let loc = self.locate(id)?;
        matches!(loc.slot, Slot::Floating { .. }).then_some(loc)
    }

    /// See [`Action::MoveFloating`](crate::Action::MoveFloating).
    pub(super) fn move_floating(&mut self, id: WindowId, x: i32, y: i32) {
        let Some(loc) = self.floating_location(id) else {
            return;
        };
        let Some(size) = self
            .windows
            .get(&id)
            .zip(self.outputs.get(loc.output))
            .filter(|(_, output)| has_room(output))
            .map(|(window, output)| floating_rect(window, output).rect.size())
        else {
            return;
        };
        // The output under the middle of where it was asked to go, as long
        // as it has room for a window; otherwise the one it is on.
        let middle = Point::new(x.saturating_add(size.w / 2), y.saturating_add(size.h / 2));
        let target = self
            .outputs
            .iter()
            .position(|output| output.area.contains(middle) && has_room(output))
            .unwrap_or(loc.output);
        if target != loc.output {
            self.transfer_floating(id, loc, target);
        }
        let Some((window, output)) = self.windows.get(&id).zip(self.outputs.get(target)) else {
            return;
        };
        // Measured again: the target's usable area may clamp it smaller.
        let size = floating_rect(window, output).rect.size();
        let usable = output.usable;
        let x = x.min(usable.right().saturating_sub(size.w)).max(usable.x);
        let y = y.min(usable.bottom().saturating_sub(size.h)).max(usable.y);
        let (align_x, align_y) = window
            .floating
            .and_then(|floating| floating.anchor)
            .map_or((Align::Middle, Align::Middle), |anchor| {
                (anchor.x, anchor.y)
            });
        let area = output.area;
        let point = Point::new(
            x.saturating_add(align_x.offset(size.w))
                .saturating_sub(area.x),
            y.saturating_add(align_y.offset(size.h))
                .saturating_sub(area.y),
        );
        if let Some(floating) = self.windows.get_mut(&id).and_then(|w| w.floating.as_mut()) {
            floating.anchor = Some(Anchor {
                point,
                x: align_x,
                y: align_y,
            });
        }
    }

    /// See [`Action::ResizeFloating`](crate::Action::ResizeFloating).
    pub(super) fn resize_floating(&mut self, id: WindowId, size: Size, edges: Edges) {
        let Some(loc) = self.floating_location(id) else {
            return;
        };
        let Some((window, output)) = self.windows.get(&id).zip(self.outputs.get(loc.output)) else {
            return;
        };
        if !has_room(output) {
            return;
        }
        let (usable, area) = (output.usable, output.area);
        let current = floating_rect(window, output).rect;
        let anchor = global_anchor(window, output);
        let hints = window.info.hints;
        let (x, align_x, w) = resize_axis(
            Span {
                start: current.x,
                extent: current.w,
                pin: anchor.point.x,
                align: anchor.x,
                room_start: usable.x,
                room_end: usable.right(),
            },
            (edges.left, edges.right),
            size.w,
            (hints.min.w, hints.max.w),
        );
        let (y, align_y, h) = resize_axis(
            Span {
                start: current.y,
                extent: current.h,
                pin: anchor.point.y,
                align: anchor.y,
                room_start: usable.y,
                room_end: usable.bottom(),
            },
            (edges.top, edges.bottom),
            size.h,
            (hints.min.h, hints.max.h),
        );
        if let Some(floating) = self.windows.get_mut(&id).and_then(|w| w.floating.as_mut()) {
            floating.anchor = Some(Anchor {
                point: Point::new(x.saturating_sub(area.x), y.saturating_sub(area.y)),
                x: align_x,
                y: align_y,
            });
            floating.request = Some(Size::new(w, h));
        }
    }

    /// Carries a floating window to output `t`'s active workspace: on top
    /// of its floating layer, and focused there when it was the focused
    /// window (focus follows it to the output). The caller places it.
    ///
    /// The one step of a move that allocates (`normalize` rebuilds each
    /// output's workspace list, and `fix_view` measures the strips), which
    /// is fine: it happens once per output crossed, not per motion.
    fn transfer_floating(&mut self, id: WindowId, loc: Location, t: usize) {
        let Slot::Floating { index } = loc.slot else {
            return;
        };
        let Some(source) = self.outputs.get_mut(loc.output) else {
            return;
        };
        let follows = self.focused_output == loc.output
            && source.active == loc.workspace
            && source.active_workspace().focused_window() == Some(id);
        source.workspaces[loc.workspace].take_floating(index);
        source.normalize();
        let Some(target) = self.outputs.get_mut(t) else {
            return;
        };
        target.active_workspace_mut().push_floating(id, follows);
        target.normalize();
        if follows {
            self.focused_output = t;
        }
        self.fix_view(loc.output);
        self.fix_view(t);
    }
}

/// Whether an output has a usable area a floating window can be placed in.
fn has_room(output: &Output) -> bool {
    output.usable.w > 0 && output.usable.h > 0
}

/// A floating window's anchor in global coordinates: the one it has, or the
/// middle of its output's usable area for a window that has none (where
/// [`floating_rect`] places it).
fn global_anchor(window: &WindowState, output: &Output) -> Anchor {
    let (usable, area) = (output.usable, output.area);
    match window.floating.and_then(|floating| floating.anchor) {
        Some(anchor) => Anchor {
            point: Point::new(
                area.x.saturating_add(anchor.point.x),
                area.y.saturating_add(anchor.point.y),
            ),
            ..anchor
        },
        None => Anchor::centred(Point::new(
            usable.x.saturating_add(usable.w / 2),
            usable.y.saturating_add(usable.h / 2),
        )),
    }
}

/// Where a (non-fullscreen) floating window goes on `output`: its size is
/// what it last drew (or, before it has drawn, the size it was asked for;
/// or 1x1, invisible, with neither), squeezed into the usable area and
/// never below 1; its position is its anchor's, shifted just far enough to
/// lie inside the usable area.
///
/// The only placement arithmetic for floating windows: `arrange` and
/// [`World::floating_geometry`] both come here. Saturating throughout --
/// every operand is bounded by the output or by a clamp to it, but a panic
/// in the layout would take the session down.
pub(super) fn floating_rect(window: &WindowState, output: &Output) -> Placed {
    let usable = output.usable;
    let fits = has_room(output);
    // A size squeezed into the usable area, never below 1: the floor is
    // what keeps an empty usable area (everything reserved) from producing
    // a zero-sized rect, and such a window is placed invisible.
    let clamp = |size: Size| Size::new(size.w.min(usable.w).max(1), size.h.min(usable.h).max(1));
    let request = window.floating.and_then(|floating| floating.request);
    let requested = request.filter(|_| fits).map(clamp);
    let drawn = window.drawn;
    let natural = if drawn.w > 0 && drawn.h > 0 {
        Some(drawn)
    } else {
        requested
    };
    let size = natural.map_or(Size::new(1, 1), clamp);
    let anchor = global_anchor(window, output);
    let x = anchor
        .point
        .x
        .saturating_sub(anchor.x.offset(size.w))
        .min(usable.right().saturating_sub(size.w))
        .max(usable.x);
    let y = anchor
        .point
        .y
        .saturating_sub(anchor.y.offset(size.h))
        .min(usable.bottom().saturating_sub(size.h))
        .max(usable.y);
    Placed {
        rect: Rect::new(x, y, size.w, size.h),
        requested,
        sized: natural.is_some(),
        fits,
    }
}

/// One axis of a floating window, as [`resize_axis`] needs it. Global
/// logical coordinates throughout.
struct Span {
    /// Where the window starts along the axis, as placed now.
    start: i32,
    /// How long it is along the axis, as placed now.
    extent: i32,
    /// Its anchor's point along the axis, and which point of it that is.
    pin: i32,
    align: Align,
    /// The usable area along the axis.
    room_start: i32,
    room_end: i32,
}

/// One axis of a resize: the anchor point and alignment the window is held
/// by afterwards, and the length it is asked to take. `(near, far)` is
/// whether the left/top and right/bottom edges move; `(min, max)` is the
/// window's own limits along the axis (zero for none).
///
/// An axis with no moving edge keeps its anchor and the length it is placed
/// at. Otherwise the edge that does not move is held: by the anchor the
/// window already has, when it already holds that edge from inside the
/// usable area -- so the resizes a pointer drag sends, one per motion, all
/// hold the same point, whatever size the client has drawn in between --
/// else by where that edge is placed now.
fn resize_axis(
    span: Span,
    (near, far): (bool, bool),
    asked: i32,
    (min, max): (i32, i32),
) -> (i32, Align, i32) {
    let align = if far {
        Align::Start
    } else if near {
        Align::End
    } else {
        return (span.pin, span.align, span.extent);
    };
    let inside = span.pin >= span.room_start && span.pin <= span.room_end;
    let pin = if span.align == align && inside {
        span.pin
    } else if align == Align::Start {
        span.start
    } else {
        span.start.saturating_add(span.extent)
    };
    let mut extent = asked;
    if max > 0 {
        extent = extent.min(max);
    }
    if min > 0 {
        extent = extent.max(min);
    }
    // The room between the edge that stays and the far side of the usable
    // area: a resize grows the window up to it and never shifts the fixed
    // edge to make more.
    let room = if align == Align::Start {
        span.room_end.saturating_sub(pin)
    } else {
        pin.saturating_sub(span.room_start)
    };
    (pin, align, extent.min(room).max(1))
}
