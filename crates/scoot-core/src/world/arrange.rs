//! Projecting the tree into frames.

use super::World;
use super::tree::{Column, Output, Workspace};
use crate::geometry::Rect;
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
    /// covered: stacked in the same column as a fullscreen window.
    pub visible: bool,
    /// The window is fullscreen: `rect` is its output's whole area in size,
    /// and exactly that area while its column is in focus (see
    /// [`World::fullscreen_on`] for when that is). A shell tells the window
    /// it is fullscreen from this, and draws no decoration around it.
    pub fullscreen: bool,
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
        let gap = self.config.gap;
        // The output minus whatever the platform reserved (a bar's exclusive
        // zone), then minus the layout gap -- never `output.area`, which is
        // the whole screen and includes the reserved strip. See
        // `tree::Output::usable`. The one exception is a fullscreen column,
        // below, which covers `area` on purpose.
        let usable = output.usable.inset(gap);
        let spans = self.column_spans(ws, usable.w, output.area.w);
        let (starts, _) = layout::starts(spans.iter().map(|span| span.width), gap);
        for ((column, start), span) in ws.columns.iter().zip(starts).zip(spans) {
            if let Some(fullscreen) = span.fullscreen {
                place_fullscreen_column(output, ws, column, fullscreen, start, active, placements);
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
        let view = if ws.is_empty() {
            0
        } else {
            // A focused fullscreen column is `area.w` wide, never narrower
            // than `available` (`usable` is a sub-rectangle of `area`, and
            // the gap only shrinks it further), so this lands `view_x` exactly
            // on the column's start -- which is what puts its window on the
            // output's left edge in `place_fullscreen_column`.
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

/// Places a column whose focused window is fullscreen.
///
/// The fullscreen window is the output's whole area in size. Its `x` is the
/// column's place in the strip measured from the output's own left edge
/// rather than from the gap-inset usable area: when the column is in focus,
/// `fix_view` has put `view_x` exactly on `start`, so that is `area.x`
/// exactly and the window covers the output edge to edge, bars and gaps
/// included. Scrolled away, it sits beside the focused column at the same
/// size. Its stacked siblings get the same frame, invisible -- they are
/// behind it.
fn place_fullscreen_column(
    output: &Output,
    ws: &Workspace,
    column: &Column,
    fullscreen: WindowId,
    start: i32,
    active: bool,
    placements: &mut Vec<Placement>,
) {
    let area = output.area;
    let x = area.x.saturating_add(start).saturating_sub(ws.view_x);
    let rect = Rect::new(x, area.y, area.w.max(1), area.h.max(1));
    let on_screen = x < area.right() && x.saturating_add(rect.w) > area.x;
    for &id in &column.windows {
        let is_fullscreen = id == fullscreen;
        placements.push(Placement {
            id,
            output: output.id,
            rect,
            visible: is_fullscreen && active && on_screen,
            fullscreen: is_fullscreen,
        });
    }
}
