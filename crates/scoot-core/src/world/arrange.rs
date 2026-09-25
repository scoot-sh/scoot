//! Projecting the tree into frames.

use super::World;
use super::floating::centred_within;
use super::tree::{Column, Output, Workspace};
use crate::geometry::{Point, Rect, Size};
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
    /// [`World::fullscreen_on`]). A floating window is also invisible before
    /// it has drawn (there is no size to place it at yet), and while it is
    /// fullscreen without focus.
    pub visible: bool,
    /// The window is fullscreen: `rect` is its output's whole area in size,
    /// and exactly that area while its column is in focus (see
    /// [`World::fullscreen_on`] for when that is). A shell tells the window
    /// it is fullscreen from this, and draws no decoration around it.
    pub fullscreen: bool,
    /// The window floats above its workspace's strip (see
    /// [`Action::ToggleFloating`](crate::Action::ToggleFloating)). Floating
    /// placements come after their workspace's tiled ones, bottom of the
    /// stack first, so a shell drawing and stacking windows in arrangement
    /// order puts them above the strip in the right order. A shell tells a
    /// floating window it is *not* tiled.
    pub floating: bool,
    /// The size to ask the window to take. `rect`'s size for every window
    /// the layout sizes -- tiled, fullscreen -- and `None` for a floating
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
struct Span {
    width: i32,
    /// The column's fullscreen window, when it has one.
    fullscreen: Option<WindowId>,
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
        // an active workspace. It hides everything else on the workspace:
        // the floating layer under a covering column, and the strip and the
        // rest of the floating layer under a covering floating window.
        // Only an active workspace shows anything, so only its answer is
        // read: an inactive one is placed invisible whatever covers it, and
        // skipping it saves a map lookup per workspace per arrangement.
        let covering = active
            .then(|| ws.focused_window())
            .flatten()
            .filter(|id| self.windows.get(id).is_some_and(|w| w.fullscreen.is_some()));
        let floating_covers = covering.is_some() && ws.floating_has_focus();
        self.place_strip(output, ws, active && !floating_covers, placements);
        self.place_floating(output, ws, active, covering, placements);
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
    /// be -- see `World::settle_fullscreen`), which it also names.
    ///
    /// The one source of both [`World::arrange`]'s placement and
    /// [`World::fix_view`]'s scroll, so the two can never disagree about
    /// where a fullscreen column sits. The fullscreen check rides on the
    /// window lookups the minimum width already makes, so a tiled
    /// arrangement pays no extra map lookup for it.
    fn column_spans(&self, ws: &Workspace, available: i32, full_width: i32) -> Vec<Span> {
        ws.columns
            .iter()
            .map(|column| {
                let mut min = 0;
                for (index, id) in column.windows.iter().enumerate() {
                    let Some(window) = self.windows.get(id) else {
                        continue;
                    };
                    if index == column.focused && window.fullscreen.is_some() {
                        return Span {
                            width: full_width.max(1),
                            fullscreen: Some(*id),
                        };
                    }
                    min = min.max(window.min().w);
                }
                let proportion = self.config.column_widths[column.preset];
                Span {
                    width: layout::column_width(available, self.config.gap, proportion, min),
                    fullscreen: None,
                }
            })
            .collect()
    }

    /// Places a workspace's floating layer, bottom of the stack first (see
    /// [`Action::ToggleFloating`](crate::Action::ToggleFloating) for the
    /// rules). Reads only the window map and the output: no parent lookup
    /// and no allocation beyond the placements themselves -- this runs on
    /// every frame.
    ///
    /// `covering` is the fullscreen window covering the output, when the
    /// workspace's focused window is one: a floating fullscreen window shows
    /// only while it is that window, and every other floating window hides
    /// while anything covers.
    fn place_floating(
        &self,
        output: &Output,
        ws: &Workspace,
        active: bool,
        covering: Option<WindowId>,
        placements: &mut Vec<Placement>,
    ) {
        let usable = output.usable;
        let area = output.area;
        let fits = usable.w > 0 && usable.h > 0;
        // A size squeezed into the usable area, never below 1: the floor is
        // what keeps an empty usable area (everything reserved) from
        // producing a zero-sized rect, and such a window is placed invisible.
        let clamp =
            |size: Size| Size::new(size.w.min(usable.w).max(1), size.h.min(usable.h).max(1));
        for &id in &ws.floating {
            let Some(window) = self.windows.get(&id) else {
                continue;
            };
            if window.fullscreen.is_some() {
                let (w, h) = (area.w.max(1), area.h.max(1));
                placements.push(Placement {
                    id,
                    output: output.id,
                    rect: Rect::new(area.x, area.y, w, h),
                    visible: active && covering == Some(id),
                    fullscreen: true,
                    floating: true,
                    requested: Some(Size::new(w, h)),
                });
                continue;
            }
            let (centre, request) = window
                .floating
                .map_or((None, None), |floating| (floating.centre, floating.request));
            let requested = request.filter(|_| fits).map(clamp);
            let drawn = window.drawn;
            let natural = if drawn.w > 0 && drawn.h > 0 {
                Some(drawn)
            } else {
                requested
            };
            let size = natural.map_or(Size::new(1, 1), clamp);
            let centre = match centre {
                Some(offset) => Point::new(
                    area.x.saturating_add(offset.x),
                    area.y.saturating_add(offset.y),
                ),
                None => Point::new(
                    usable.x.saturating_add(usable.w / 2),
                    usable.y.saturating_add(usable.h / 2),
                ),
            };
            placements.push(Placement {
                id,
                output: output.id,
                rect: centred_within(centre, size, usable),
                visible: active && covering.is_none() && natural.is_some() && fits,
                fullscreen: false,
                floating: true,
                requested,
            });
        }
    }

    fn column_heights(&self, column: &Column, available: i32) -> Vec<i32> {
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
        let Some(output) = self.outputs.get(o) else {
            return;
        };
        let available = output.usable.inset(self.config.gap).w;
        let ws = output.active_workspace();
        let view = if ws.columns.is_empty() {
            0
        } else {
            // A focused fullscreen column is `area.w` wide, never narrower
            // than `available` (`usable` is a sub-rectangle of `area`, and
            // the gap only shrinks it further), so this lands `view_x` exactly
            // on the column's start -- which is what scrolls every other
            // column out of view while `place_fullscreen_column` puts the
            // covering window on the output's whole area.
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
        self.outputs[o].active_workspace_mut().view_x = view;
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
            floating: false,
            requested: Some(rect.size()),
        });
    }
}
