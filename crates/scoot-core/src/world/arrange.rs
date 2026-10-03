//! Projecting the tree into frames.

use super::World;
use super::floating_order::FloatCover;
use super::tree::{Column, Output, Workspace};
use crate::geometry::{Rect, Size};
use crate::layout;
use crate::types::{OutputId, WindowId};

/// Where one window should be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: WindowId,
    pub output: OutputId,
    /// Target frame in global logical coordinates. Windows that aren't visible
    /// still get the frame they *would* have, so a shell can animate from it.
    pub rect: Rect,
    /// False when scrolled out of view or on an inactive workspace -- or
    /// covered: stacked in the same column as a fullscreen window, or behind
    /// a fullscreen window covering the output (see
    /// [`World::fullscreen_on`]). A maximized window follows the same rule
    /// against [`World::maximized_on`]. A floating window is also invisible before
    /// it has drawn (there is no size to place it at yet), and while it is
    /// fullscreen without focus.
    pub visible: bool,
    /// The window is fullscreen: `rect` is its output's whole area in size,
    /// and exactly that area while its column is in focus (see
    /// [`World::fullscreen_on`] for when that is). A shell tells the window
    /// it is fullscreen from this, and draws no decoration around it.
    pub fullscreen: bool,
    /// The window is maximized: `rect` is its output's usable area in size
    /// (the whole area minus the platform's reservations, minus the layout
    /// gap), and exactly that area while its column is in focus and no
    /// fullscreen window covers the output (see [`World::maximized_on`]).
    /// A shell tells the window it is maximized from this. Unlike
    /// fullscreen it keeps its gaps and its ring: the bar stays visible.
    /// Never set together with `fullscreen` on one placement -- fullscreen
    /// wins while both hold, so the placement reports that one.
    pub maximized: bool,
    /// The window floats above its workspace's strip (see
    /// [`Action::ToggleFloating`](crate::Action::ToggleFloating)). Floating
    /// placements come after their workspace's tiled ones, bottom of the
    /// stack first, so a shell drawing and stacking windows in arrangement
    /// order puts them above the strip in the right order. A shell tells a
    /// floating window it is *not* tiled.
    pub floating: bool,
    /// The size to ask the window to take. `rect`'s size for every window
    /// the layout sizes -- tiled, fullscreen, maximized -- and `None` for a floating
    /// window the core lets choose its own size (the protocol's 0x0), whose
    /// `rect` is then the size it last drew.
    pub requested: Option<Size>,
}

/// A complete, declarative picture of where everything should be. Shells diff
/// consecutive arrangements and apply the difference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    pub placements: Vec<Placement>,
    pub focused: Option<WindowId>,
    pub focused_output: Option<OutputId>,
}

/// One column's place in the scrolling strip, as `World::column_spans`
/// measures it.
pub(super) struct Span {
    pub(super) width: i32,
    /// The column's fullscreen window, when it has one. Wins over
    /// `maximized`: a window holding both is placed fullscreen.
    pub(super) fullscreen: Option<WindowId>,
    /// The column's maximized window, when it has one and no fullscreen
    /// window outranks it.
    pub(super) maximized: Option<WindowId>,
}

impl Arrangement {
    pub fn get(&self, id: WindowId) -> Option<&Placement> {
        self.placements.iter().find(|p| p.id == id)
    }
}

impl World {
    /// Where every window should be right now. Pure: two calls with nothing
    /// handled in between return the same arrangement.
    pub fn arrange(&self) -> Arrangement {
        let mut placements = Vec::new();
        for output in &self.outputs {
            for (index, ws) in output.workspaces.iter().enumerate() {
                self.place_workspace(output, ws, index == output.active, &mut placements);
            }
        }
        Arrangement {
            placements,
            focused: self.focused_window(),
            focused_output: self.focused_output(),
        }
    }

    fn place_workspace(
        &self,
        output: &Output,
        ws: &Workspace,
        active: bool,
        placements: &mut Vec<Placement>,
    ) {
        // The fullscreen window covering the output, if the workspace's
        // focused window is one -- the same answer `fullscreen_on` gives for
        // an active workspace. It hides everything else on the workspace
        // except its own floating dialogs (see `place_floating`): the rest
        // of the floating layer under a covering column, and the strip and
        // the rest of the floating layer under a covering floating window.
        // Only an active workspace shows anything, so only its answer is
        // read: an inactive one is placed invisible whatever covers it, and
        // skipping it saves a map lookup per workspace per arrangement.
        let covering = active
            .then(|| ws.focused_window())
            .flatten()
            .filter(|id| self.is_fullscreen(*id));
        // The maximized window filling the usable area, when the workspace's
        // focused window is one and no fullscreen window outranks it -- the
        // same answer `maximized_on` gives for an active workspace. A tiled
        // one hides nothing by itself: neighbours scroll off the view (the
        // column is exactly the view's width), stacked siblings hide behind
        // it, and the floating layer stays above it. A floating one in
        // focus covers like a fullscreen window (see `max_float_covers`).
        let max_covering = (active && covering.is_none())
            .then(|| ws.focused_window())
            .flatten()
            .filter(|id| self.is_maximized(*id));
        // A floating fullscreen window that is not the focused one still
        // shows, full size, while a floating window above it has focus --
        // its dialog, typically -- the way a fullscreen column stays in
        // place under a focused floating window. The topmost such window;
        // it hides the strip and the floating windows below it.
        let shown_fullscreen = (active && covering.is_none() && ws.floating_has_focus())
            .then(|| ws.floating.iter().rposition(|id| self.is_fullscreen(*id)))
            .flatten();
        // A floating maximized window (holding no fullscreen, which the
        // line above already claims) shown the same way: full usable size
        // under the floating window above it, hiding the strip and the
        // floating windows below it except its own dialogs.
        let shown_maximized = (active && covering.is_none() && ws.floating_has_focus())
            .then(|| {
                ws.floating
                    .iter()
                    .rposition(|id| self.is_maximized(*id) && !self.is_fullscreen(*id))
            })
            .flatten();
        let floating_covers =
            (covering.is_some() && ws.floating_has_focus()) || shown_fullscreen.is_some();
        // A floating maximized window in focus covers the usable area the
        // way a covering fullscreen window covers the output (hiding the
        // strip and the floating windows below it, except its own dialogs);
        // a tiled maximized cover stays under the whole floating layer and
        // hides nothing itself. `shown_maximized` is the below-focus twin
        // of `shown_fullscreen`.
        let max_float_covers =
            max_covering.is_some_and(|id| self.is_floating(id)) || shown_maximized.is_some();
        self.place_strip(
            output,
            ws,
            active && !floating_covers && !max_float_covers,
            placements,
        );
        self.place_floating(
            output,
            ws,
            active,
            &FloatCover {
                covering,
                shown_fullscreen,
                max_covering,
                shown_maximized,
            },
            placements,
        );
    }

    fn place_strip(
        &self,
        output: &Output,
        ws: &Workspace,
        active: bool,
        placements: &mut Vec<Placement>,
    ) {
        let gap = self.config.gap;
        // The output minus whatever the platform reserved (a bar's exclusive
        // zone), then minus the layout gap -- never `output.area`, which is
        // the whole screen and includes the reserved strip. See
        // `tree::Output::usable`. The one exception is a fullscreen column,
        // below, which covers `area` on purpose.
        let usable = output.usable.inset(gap);
        let spans = self.column_spans(ws, usable.w, output.area.w);
        let (starts, _) = layout::starts(spans.iter().map(|span| span.width), gap);
        for (index, ((column, start), span)) in ws.columns.iter().zip(starts).zip(spans).enumerate()
        {
            if let Some(fullscreen) = span.fullscreen {
                let strip_x = usable.x.saturating_add(start).saturating_sub(ws.view_x);
                let slot = FullscreenSlot {
                    // The column in focus is the one covering the output
                    // (see `World::fullscreen_on`).
                    covering: index == ws.focused,
                    strip_x,
                    usable,
                };
                place_fullscreen_column(output, column, fullscreen, slot, active, placements);
                continue;
            }
            if let Some(maximized) = span.maximized {
                let strip_x = usable.x.saturating_add(start).saturating_sub(ws.view_x);
                let slot = MaximizedSlot {
                    // The column in focus is the one filling the usable
                    // area (see `World::maximized_on`).
                    covering: index == ws.focused,
                    strip_x,
                    usable,
                };
                place_maximized_column(column, maximized, slot, active, output.id, placements);
                continue;
            }
            let width = span.width;
            // Saturating: a saturated strip (`layout::starts`) plus a
            // nonzero `usable.x` can put this past `i32::MAX` -- reachable
            // today only through an absurd configured proportion (which
            // saturates `column_width`'s float cast first), but a panic
            // there would take the session down all the same.
            let x = usable.x.saturating_add(start).saturating_sub(ws.view_x);
            let on_screen = x < usable.right() && x.saturating_add(width) > usable.x;
            let mut y = usable.y;
            for (&id, height) in column
                .windows
                .iter()
                .zip(self.column_heights(column, usable.h))
            {
                placements.push(Placement {
                    id,
                    output: output.id,
                    rect: Rect::new(x, y, width, height),
                    visible: active && on_screen,
                    fullscreen: false,
                    maximized: false,
                    floating: false,
                    requested: Some(Size::new(width, height)),
                });
                y += height + gap;
            }
        }
    }

    /// Every column's span in the strip, in order: its preset's share of
    /// `available`, or the output's whole `full_width` for a column whose
    /// focused window is fullscreen (the only place a fullscreen window can
    /// be -- see `World::settle_fullscreen`), which it also names -- or
    /// `available` itself for a column whose focused window is maximized and
    /// no fullscreen window outranks it (see `World::settle_maximize`).
    ///
    /// The one source of both [`World::arrange`]'s placement and
    /// [`World::fix_view`]'s scroll, so the two can never disagree about
    /// where a fullscreen or maximized column sits. Both checks ride on the
    /// window lookups the minimum width already makes, so a tiled
    /// arrangement pays no extra map lookup for them.
    pub(super) fn column_spans(
        &self,
        ws: &Workspace,
        available: i32,
        full_width: i32,
    ) -> Vec<Span> {
        ws.columns
            .iter()
            .map(|column| {
                let mut min = 0;
                for (index, id) in column.windows.iter().enumerate() {
                    let Some(window) = self.windows.get(id) else {
                        continue;
                    };
                    if index == column.focused {
                        if window.fullscreen.is_some() {
                            return Span {
                                width: full_width.max(1),
                                fullscreen: Some(*id),
                                maximized: None,
                            };
                        }
                        if window.maximized.is_some() {
                            return Span {
                                width: available.max(1),
                                fullscreen: None,
                                maximized: Some(*id),
                            };
                        }
                    }
                    min = min.max(window.min().w);
                }
                let proportion = self.config.column_widths[column.preset];
                Span {
                    width: layout::column_width(available, self.config.gap, proportion, min),
                    fullscreen: None,
                    maximized: None,
                }
            })
            .collect()
    }

    pub(super) fn column_heights(&self, column: &Column, available: i32) -> Vec<i32> {
        let mins: Vec<i32> = column
            .windows
            .iter()
            .map(|id| self.windows.get(id).map_or(0, |w| w.min().h))
            .collect();
        let gaps = self.config.gap * (column.windows.len() as i32 - 1);
        layout::distribute(available - gaps, &mins)
    }

    /// Scrolls the active workspace of output `o` so its focused column is in
    /// view.
    pub(super) fn fix_view(&mut self, o: usize) {
        if let Some(active) = self.outputs.get(o).map(|output| output.active) {
            self.fix_workspace_view(o, active);
        }
    }

    /// [`World::fix_view`] for workspace `w` of output `o`, active or not.
    /// An inactive workspace is scrolled when it is switched to anyway; this
    /// is for the rare callers that need its layout right before then (an
    /// output that changed size, or adopted workspaces from another one,
    /// before floating windows are re-centred on parents there).
    pub(super) fn fix_workspace_view(&mut self, o: usize, w: usize) {
        let Some(output) = self.outputs.get(o) else {
            return;
        };
        let Some(ws) = output.workspaces.get(w) else {
            return;
        };
        let available = output.usable.inset(self.config.gap).w;
        let view = if ws.columns.is_empty() {
            0
        } else {
            // A focused fullscreen column is `area.w` wide, never narrower
            // than `available` (`usable` is a sub-rectangle of `area`, and
            // the gap only shrinks it further), so this lands `view_x` exactly
            // on the column's start -- which is what scrolls every other
            // column out of view while `place_fullscreen_column` puts the
            // covering window on the output's whole area. A focused
            // maximized column is exactly `available` wide, so the same
            // holds for it while `place_maximized_column` fills the usable
            // area.
            let spans = self.column_spans(ws, available, output.area.w);
            let (starts, strip) =
                layout::starts(spans.iter().map(|span| span.width), self.config.gap);
            layout::scroll_into_view(
                ws.view_x,
                available,
                starts[ws.focused],
                spans[ws.focused].width,
                strip,
            )
        };
        self.outputs[o].workspaces[w].view_x = view;
    }

    pub(super) fn fix_all_views(&mut self) {
        for o in 0..self.outputs.len() {
            self.fix_view(o);
        }
    }
}

/// Where a fullscreen column sits, as `place_workspace` measured it.
struct FullscreenSlot {
    /// Whether it is its workspace's focused column -- the one covering the
    /// output.
    covering: bool,
    /// Its left edge in the strip, measured the way every tiled column's is:
    /// from the gap-inset usable area, minus the scroll.
    strip_x: i32,
    /// The gap-inset usable area the strip is laid out in.
    usable: Rect,
}

/// Places a column whose focused window is fullscreen.
///
/// The fullscreen window is always the output's whole area in size, so the
/// client is never reconfigured just because focus moved. Where it goes
/// depends on whether it covers:
///
/// - **Covering** (its column is the focused one): exactly `area`, edge to
///   edge, bars and gaps included. Every other column on the workspace is
///   scrolled off the view (the column is `area.w` wide in the strip, never
///   narrower than the view, and `fix_view` lines the view up with its
///   start), so nothing is placed beside it.
/// - **Not covering**: exactly where a tiled column of that width would sit
///   -- `strip_x`, measured from the gap-inset usable area like every other
///   column -- so the ordinary gap separates it from its neighbours on both
///   sides and it never overlaps the focused window. Measuring it from
///   `area.x` instead (as a first cut did) put it `usable.x - area.x` (the
///   gap, plus any left exclusive zone) to the left of its slot: a zero or
///   negative gap to a left neighbour, a doubled one to a right neighbour,
///   and a window lying over -- and taking clicks meant for -- the focused
///   one.
///
/// Its stacked siblings get the same frame, invisible: they are behind it.
fn place_fullscreen_column(
    output: &Output,
    column: &Column,
    fullscreen: WindowId,
    slot: FullscreenSlot,
    active: bool,
    placements: &mut Vec<Placement>,
) {
    let area = output.area;
    let (w, h) = (area.w.max(1), area.h.max(1));
    let (rect, on_screen) = if slot.covering {
        (Rect::new(area.x, area.y, w, h), true)
    } else {
        let x = slot.strip_x;
        let on_screen = x < slot.usable.right() && x.saturating_add(w) > slot.usable.x;
        (Rect::new(x, area.y, w, h), on_screen)
    };
    for &id in &column.windows {
        let is_fullscreen = id == fullscreen;
        placements.push(Placement {
            id,
            output: output.id,
            rect,
            visible: is_fullscreen && active && on_screen,
            fullscreen: is_fullscreen,
            maximized: false,
            floating: false,
            requested: Some(rect.size()),
        });
    }
}

/// Where a maximized column sits, as `place_workspace` measured it.
struct MaximizedSlot {
    /// Whether it is its workspace's focused column -- the one filling the
    /// usable area.
    covering: bool,
    /// Its left edge in the strip, measured the way every tiled column's is:
    /// from the gap-inset usable area, minus the scroll.
    strip_x: i32,
    /// The gap-inset usable area the strip is laid out in -- also the
    /// maximized window's frame while it covers.
    usable: Rect,
}

/// Places a column whose focused window is maximized (and no fullscreen
/// window outranks it -- see `World::column_spans`).
///
/// The maximized window is always its output's usable area in size, so the
/// client is never reconfigured just because focus moved. Where it goes
/// depends on whether it covers:
///
/// - **Covering** (its column is the focused one): exactly `usable`, gaps
///   and ring kept, the bar's zone excluded. Every other column on the
///   workspace is scrolled off the view (the column is exactly the view's
///   width, and `fix_view` lines the view up with its start), so nothing
///   is placed beside it.
/// - **Not covering**: exactly where a tiled column of that width would sit
///   -- `strip_x`, measured from the gap-inset usable area like every other
///   column -- so the ordinary gap separates it from its neighbours on both
///   sides and it never overlaps the focused window.
///
/// Its stacked siblings get the same frame, invisible: they are behind it.
fn place_maximized_column(
    column: &Column,
    maximized: WindowId,
    slot: MaximizedSlot,
    active: bool,
    output: OutputId,
    placements: &mut Vec<Placement>,
) {
    let usable = slot.usable;
    let (w, h) = (usable.w.max(1), usable.h.max(1));
    let (rect, on_screen) = if slot.covering {
        (Rect::new(usable.x, usable.y, w, h), true)
    } else {
        let x = slot.strip_x;
        let on_screen = x < usable.right() && x.saturating_add(w) > usable.x;
        (Rect::new(x, usable.y, w, h), on_screen)
    };
    for &id in &column.windows {
        let is_maximized = id == maximized;
        placements.push(Placement {
            id,
            output,
            rect,
            visible: is_maximized && active && on_screen,
            fullscreen: false,
            maximized: is_maximized,
            floating: false,
            requested: Some(rect.size()),
        });
    }
}
