//! Giving a reconnected output its workspaces back.
//!
//! When an output goes away its workspaces are adopted by the focused output
//! ([`World::remove_output`](super::World::remove_output), in `events.rs`),
//! and they stay there. When the same monitor comes back, the shell hands
//! those workspaces back with [`World::restore_output`]: the same workspaces,
//! active index and column order -- but only for the windows that are still
//! where the adoption put them. A window the user has since moved elsewhere
//! by hand stays where it is, and a closed window drops out.
//!
//! The shell (which owns connector identity -- a Wayland concept this core
//! must not know) records the [`EvictedOutput`] [`World::evict_output`]
//! answers keyed by that identity, and hands it back when an output with a
//! matching identity is added. The core's half is purely positional: a window
//! counts as unmoved when it is still at exactly the adopted path the
//! snapshot recorded, and anything else -- a hand move, a close, or an
//! unrelated structural change on the adopting output that shifted it --
//! keeps it where it is. That bias is deliberate: failing to restore a window
//! leaves it one move away, while yanking a window the user arranged would
//! undo their work.

use super::World;
use super::tree::{Column, Slot, Workspace};
use crate::types::{OutputId, WindowId};

/// One column as it was when its output went away: its windows in order, the
/// width preset they shared, and which window had focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnSnapshot {
    pub windows: Vec<WindowId>,
    pub preset: usize,
    pub focused: usize,
}

/// One workspace as it was when its output went away: its columns in order
/// with the focused column, and its floating layer bottom-first with whether
/// it had focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub columns: Vec<ColumnSnapshot>,
    pub focused: usize,
    pub floating: Vec<WindowId>,
    pub floating_focused: bool,
}

/// An output's workspaces as they were when it went away: the non-empty ones
/// in order (the adoption drops empties, so this is what actually moved), and
/// the output's active index. The index counts within the full list the
/// output had -- trailing empty workspace included -- so restoring clamps it
/// rather than trusting it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSnapshot {
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub active: usize,
}

/// Everything [`World::evict_output`] reports about one removed output: what
/// left, who adopted it, and where the adopted block starts in the adopter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvictedOutput {
    pub snapshot: OutputSnapshot,
    pub adopted_by: Option<OutputId>,
    /// The workspace index in the adopter the adopted block starts at. `0`
    /// with no adopter (the windows wait in `unplaced` instead). Recorded
    /// after the removal's focus switch and its normalize, which is the list
    /// the restore verifies each window against.
    pub adopted_at: usize,
    /// The opaque id the adopted workspaces were tagged with, for the shell
    /// to map back to a connector name. `None` when nothing was adopted.
    pub origin: Option<u64>,
    /// Where the adopter was looking before the adoption, for the restore to
    /// return to. `None` when the removal switched nothing (focus was
    /// elsewhere, so the adopter never moved) or when the view workspace was
    /// emptied and the switch dropped it -- in both cases the restore leaves
    /// the adopter's active workspace alone.
    pub adopter_active: Option<AdopterView>,
}

/// Where the adopter was looking before an adoption, measured against the
/// post-switch list -- the same coordinate frame as
/// [`EvictedOutput::adopted_at`].
///
/// Relative to the adopted block rather than a raw index, deliberately: the
/// block is what the restore verifies positions against, so a view expressed
/// against it refuses itself (rather than landing somewhere unasked) when an
/// intervening change moved the block over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdopterView {
    /// That many workspaces before the adopted block: the restore activates
    /// `adopted_at - n`, and only when that still lands before the block.
    BeforeBlock(usize),
    /// The trailing empty workspace: the restore activates whatever the
    /// trailing empty workspace is then.
    Trailing,
}

/// What [`World::remove_output`](super::World::remove_output) did with the
/// removed output's workspaces. The event path drops it; `evict_output`
/// keeps it.
pub(super) struct Removal {
    pub(super) adopted_by: Option<OutputId>,
    pub(super) adopted_at: usize,
    pub(super) origin: Option<u64>,
    pub(super) adopter_active: Option<AdopterView>,
}

/// One window still where the adoption put it, on its way back: its snapshot
/// workspace, and its position on the adopter *now* for the take (a column
/// and an index down it, or a floating-stack index).
struct Carry {
    wid: WindowId,
    ws: usize,
    column: Option<usize>,
    index: usize,
}

/// One snapshot workspace with only the windows still in place: rebuilt
/// columns (windows kept, preset, the snapshot's focused window and its
/// index), the snapshot's focused column, and the kept floating windows.
struct Rebuilt {
    columns: Vec<RebuiltColumn>,
    focused: usize,
    focused_wid: Option<WindowId>,
    floating: Vec<WindowId>,
    floating_focused: bool,
}

struct RebuiltColumn {
    windows: Vec<WindowId>,
    preset: usize,
    focused: usize,
    focused_wid: Option<WindowId>,
}

impl World {
    /// Removes output `id` like [`Event::OutputRemoved`](crate::Event), and
    /// answers what left and where it went -- the record the shell keeps
    /// keyed by connector identity until the monitor comes back. `None` for
    /// an unknown output, which changes nothing.
    pub fn evict_output(&mut self, id: OutputId) -> Option<EvictedOutput> {
        let o = self.output_index(id)?;
        let snapshot = self.snapshot_output_index(o);
        let removal = self.remove_output(id);
        Some(EvictedOutput {
            snapshot,
            adopted_by: removal.adopted_by,
            adopted_at: removal.adopted_at,
            origin: removal.origin,
            adopter_active: removal.adopter_active,
        })
    }

    /// Moves an evicted output's workspaces onto output `id` -- the output
    /// added for the returning monitor -- for every window still where the
    /// adoption put it.
    ///
    /// Each snapshot window is restored only when it is still on its adopted
    /// workspace, in the same layer (tiled or floating) the adoption put it
    /// in -- or still waiting in `unplaced` when nothing adopted it. A window
    /// the user moved elsewhere by hand (another output, another workspace),
    /// a window they floated or un-floated, a closed window, and a window an
    /// unrelated change shifted onto a different workspace index all stay
    /// where they are; a workspace left with nothing kept is dropped, and an
    /// empty snapshot restores nothing at all. The restored workspaces keep
    /// their column order, presets and floating order; the output's active
    /// index is the snapshot's, clamped.
    ///
    /// Focus follows a carried window home: when the focused window is among
    /// the windows moved back, it stays focused on the returned monitor.
    /// Otherwise focus stays where it was -- a replugged monitor must not
    /// steal it, and a standby cycle while the user works on the panel leaves
    /// the panel exactly as it was.
    ///
    /// The adopter goes back to its pre-adopt view while it is still showing
    /// the adopted block: [`EvictedOutput::adopter_active`], verified against
    /// the block rather than trusted blindly. When the user has since switched
    /// the adopter elsewhere, its active workspace is left alone.
    ///
    /// The adoption's origin tag comes off the adopter's remaining workspaces
    /// either way: a restore ends the association, so a workspace the user
    /// kept there stops claiming a monitor that is back.
    ///
    /// Unknown outputs (new or adopter) are ignored, as is restoring onto
    /// the adopter itself: there is nothing to pull back to itself.
    /// Hands the record back: how many still-open windows actually moved.
    pub fn restore_output(&mut self, id: OutputId, evicted: EvictedOutput) -> usize {
        let EvictedOutput {
            snapshot,
            adopted_by,
            adopted_at,
            origin,
            adopter_active,
        } = evicted;
        if snapshot.workspaces.is_empty() {
            return 0;
        }
        let Some(n) = self.output_index(id) else {
            return 0;
        };
        if adopted_by == Some(id) {
            return 0;
        }
        let a = adopted_by.and_then(|by| self.output_index(by));
        // A restore against a live adopter ends the association, whatever
        // moves: the adopter's leftovers stop naming the returned monitor.
        if let (Some(ax), Some(origin)) = (a, origin) {
            self.outputs[ax].clear_origin(origin);
            self.cleared_origin();
        }
        // Decided before the takes, which empty the block and let normalize
        // move the active workspace: focus follows a carried window home,
        // and the adopter returns to its pre-adopt view while it is still
        // showing the adopted block.
        let focused = self.focused_window();
        let return_adopter = a.is_some_and(|ax| {
            adopter_active.is_some()
                && (adopted_at..adopted_at.saturating_add(snapshot.workspaces.len()))
                    .contains(&self.outputs[ax].active)
        });

        // Phase 1, read-only: which snapshot windows are still on their
        // adopted workspace. Nothing here mutates, so every position it
        // verifies still holds in phase 2.
        //
        // Workspace-level, deliberately, not exact paths: taking a hand-moved
        // window out of its column collapses the column and shifts its old
        // neighbours, and an exact check would strand those neighbours too.
        // A window counts as unmoved while it is still on the adopted
        // workspace it landed on, in the same layer (a window the user
        // floated or un-floated by hand keeps the state they chose).
        // Anything on another workspace or output, anything closed, and
        // anything an unrelated change shifted onto a different workspace
        // index stays where it is.
        let mut carries: Vec<Carry> = Vec::new();
        let mut rebuilt: Vec<Rebuilt> = Vec::with_capacity(snapshot.workspaces.len());
        for ws in snapshot.workspaces.iter() {
            let adopted_here = adopted_at + rebuilt.len();
            let mut columns = Vec::with_capacity(ws.columns.len());
            for col in ws.columns.iter() {
                let mut kept = Vec::with_capacity(col.windows.len());
                for wid in col.windows.iter() {
                    // One `locate` per window: closed windows (no location)
                    // and windows anywhere but their adopted workspace fall
                    // through to the ignore arm. The take position for an
                    // `unplaced` window is never read (phase 2 removes those
                    // by id), so it carries a dummy.
                    match (a, self.locate(*wid)) {
                        (Some(ax), Some(loc))
                            if loc.output == ax && loc.workspace == adopted_here =>
                        {
                            if let Slot::Tiled { column, index } = loc.slot {
                                carries.push(Carry {
                                    wid: *wid,
                                    ws: rebuilt.len(),
                                    column: Some(column),
                                    index,
                                });
                                kept.push(*wid);
                            }
                        }
                        (None, _) if self.unplaced.contains(wid) => {
                            carries.push(Carry {
                                wid: *wid,
                                ws: rebuilt.len(),
                                column: Some(0),
                                index: 0,
                            });
                            kept.push(*wid);
                        }
                        _ => {}
                    }
                }
                if !kept.is_empty() {
                    columns.push(RebuiltColumn {
                        windows: kept,
                        preset: col.preset,
                        focused: col.focused,
                        focused_wid: col.windows.get(col.focused).copied(),
                    });
                }
            }
            let mut floating = Vec::with_capacity(ws.floating.len());
            for wid in ws.floating.iter() {
                match (a, self.locate(*wid)) {
                    (Some(ax), Some(loc)) if loc.output == ax && loc.workspace == adopted_here => {
                        if let Slot::Floating { index } = loc.slot {
                            carries.push(Carry {
                                wid: *wid,
                                ws: rebuilt.len(),
                                column: None,
                                index,
                            });
                            floating.push(*wid);
                        }
                    }
                    (None, _) if self.unplaced.contains(wid) => {
                        carries.push(Carry {
                            wid: *wid,
                            ws: rebuilt.len(),
                            column: None,
                            index: 0,
                        });
                        floating.push(*wid);
                    }
                    _ => {}
                }
            }
            rebuilt.push(Rebuilt {
                columns,
                focused: ws.focused,
                focused_wid: ws
                    .columns
                    .get(ws.focused)
                    .and_then(|col| col.windows.get(col.focused).copied()),
                floating,
                floating_focused: ws.floating_focused,
            });
        }
        if carries.is_empty() {
            return 0;
        }

        // Phase 2: take them off the adopter or out of `unplaced`.
        //
        // Descending indices, so each take names a position the earlier ones
        // did not shift: floating lists and columns are disjoint, and takes
        // address one workspace each, so sorting the whole list at once is
        // enough (workspace, column-or-layer, index -- tiled takes sort
        // before floating ones of the same workspace, which only reads better).
        if let Some(ax) = a {
            let mut takes: Vec<(usize, Option<usize>, usize)> = carries
                .iter()
                .map(|carry| (carry.ws, carry.column, carry.index))
                .collect();
            takes.sort_by(|x, y| y.cmp(x));
            for (ws, column, index) in takes {
                let sws = &mut self.outputs[ax].workspaces[adopted_at + ws];
                match column {
                    Some(c) => {
                        sws.take(c, index);
                    }
                    None => {
                        sws.take_floating(index);
                    }
                }
            }
        } else {
            let kept: Vec<WindowId> = carries.iter().map(|carry| carry.wid).collect();
            self.unplaced.retain(|wid| !kept.contains(wid));
        }

        // Phase 3: rebuild the workspaces in snapshot order. Focus follows
        // the focused window when it survived, and clamps to what did
        // otherwise; a workspace left with nothing kept is dropped. When
        // the session's focused window is carried home, its rebuilt
        // workspace focuses it rather than the snapshot's window -- the
        // session follows the window, so the window must hold the focus.
        let mut moved_floating: Vec<WindowId> = Vec::new();
        let mut restored: Vec<Workspace> = Vec::with_capacity(rebuilt.len());
        for ws in rebuilt {
            if ws.columns.is_empty() && ws.floating.is_empty() {
                continue;
            }
            let followed = focused.filter(|wid| {
                ws.columns.iter().any(|column| column.windows.contains(wid))
                    || ws.floating.contains(wid)
            });
            let mut columns = Vec::with_capacity(ws.columns.len());
            let mut focused = 0;
            for (c, column) in ws.columns.into_iter().enumerate() {
                let j = followed
                    .and_then(|wid| column.windows.iter().position(|w| *w == wid))
                    .or_else(|| {
                        column
                            .focused_wid
                            .and_then(|wid| column.windows.iter().position(|w| *w == wid))
                    })
                    .unwrap_or_else(|| column.focused.min(column.windows.len() - 1));
                if followed
                    .or(ws.focused_wid)
                    .is_some_and(|wid| column.windows.contains(&wid))
                {
                    focused = c;
                }
                columns.push(Column {
                    windows: column.windows,
                    focused: j,
                    preset: column.preset,
                });
            }
            if !columns.is_empty() {
                focused = followed
                    .or(ws.focused_wid)
                    .and_then(|wid| columns.iter().position(|col| col.windows.contains(&wid)))
                    .unwrap_or_else(|| ws.focused.min(columns.len() - 1));
            }
            moved_floating.extend(ws.floating.iter().copied());
            let floating_focused = (followed.is_some_and(|wid| ws.floating.contains(&wid))
                || ws.floating_focused)
                && !ws.floating.is_empty();
            restored.push(Workspace {
                columns,
                focused,
                view_x: 0,
                floating: ws.floating,
                floating_focused,
                // Home again: a restored workspace names no monitor.
                origin: None,
            });
        }
        // Structural, not defensive: `carries` is non-empty (returned above
        // otherwise), and every carry lands in exactly one rebuilt workspace
        // -- so at least one restored workspace survives the empty filter.
        debug_assert!(!restored.is_empty());
        let output = &mut self.outputs[n];
        let at = output.workspaces.len() - 1;
        output.workspaces.splice(at..at, restored);
        output.active = snapshot.active.min(output.workspaces.len() - 1);
        output.normalize();
        self.fix_view(n);
        if let Some(ax) = a {
            self.outputs[ax].normalize();
            if return_adopter {
                // Verified, not trusted: a before-block view only applies
                // while it still lands before the block, so an intervening
                // change that moved the block over it refuses itself rather
                // than switching somewhere unasked. A trailing sitter goes
                // back to whatever the trailing empty workspace is now.
                let adopter = &mut self.outputs[ax];
                let index = match adopter_active {
                    Some(AdopterView::BeforeBlock(distance)) => adopted_at.checked_sub(distance),
                    Some(AdopterView::Trailing) => Some(adopter.workspaces.len() - 1),
                    None => None,
                };
                let verified = index.is_some_and(|index| {
                    index < adopter.workspaces.len()
                        && (matches!(adopter_active, Some(AdopterView::Trailing))
                            || index < adopted_at)
                });
                if let (Some(index), true) = (index, verified) {
                    adopter.active = index;
                    // The adopted workspace just left may have survived the
                    // normalize above only because it was active: leaving it
                    // behind strands a stray empty workspace, so normalize
                    // again around the returned-to view.
                    adopter.normalize();
                }
            }
            self.fix_view(ax);
        }
        // A centre measured against the old output's area means nothing on a
        // screen of another size -- the same re-centring the removal does on
        // the way out.
        self.recentre_floating_on_output(n, &moved_floating);
        // Focus follows a carried window home (see the doc above): the
        // rebuild already focuses it within its workspace, and the output's
        // active index is the snapshot's, which held focus when the session
        // was looking at the removed output.
        if focused.is_some_and(|wid| carries.iter().any(|carry| carry.wid == wid)) {
            self.focused_output = n;
        }
        carries.len()
    }

    /// One output's non-empty workspaces in order, with its active index.
    fn snapshot_output_index(&self, o: usize) -> OutputSnapshot {
        let output = &self.outputs[o];
        let workspaces = output
            .workspaces
            .iter()
            .filter(|ws| !ws.is_empty())
            .map(|ws| WorkspaceSnapshot {
                columns: ws
                    .columns
                    .iter()
                    .map(|column| ColumnSnapshot {
                        windows: column.windows.clone(),
                        preset: column.preset,
                        focused: column.focused,
                    })
                    .collect(),
                focused: ws.focused,
                floating: ws.floating.clone(),
                floating_focused: ws.floating_focused,
            })
            .collect();
        OutputSnapshot {
            workspaces,
            active: output.active,
        }
    }
}
