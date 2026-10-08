---
title: "A wallpaper per workspace (milestone 3)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# A wallpaper per workspace (milestone 3)

Through `ext-workspace-v1`, the standard protocol scoot already offers, so
this works on any compositor that has it, not only scoot.

- Map workspace (by name or index) to an image per output; switch with a
  transition when the active workspace changes.
- Decide how much to preload: switching must be instant, but holding every
  workspace's 4K buffer in memory is not free. Measure.

## Resolution (2026-10-08, PR TBD)

Keyed by workspace **name** (the string the compositor announces; on
scoot the 1-based position, so mappings follow positions across
renumbering), per output or for every output, newest-wins against the
base choice on one timeline. `scootbg set --workspace NAME` /
`clear --workspace NAME` are new control-protocol types (an old daemon
refuses them loudly instead of misreading one as a global `set`);
`query` reports the active workspace per output and the live mappings.
The daemon binds the workspace manager only while a mapping exists
(zero cost otherwise: no extra wakeups idle), follows `done` batches
with no polling, ignores groups that cannot attribute an output (zero
or several), and switches through the mapping's transition. Every
mapped image is decoded in the background at set time (and re-rendered
on output resize) and its buffers kept: about 33 MB per 4K image per
size, shared across outputs; past 64 mappings a set is refused. State
file format v3 (`workspace` / `workspace-output` lines with an optional
transition trailer; older readers refuse the file whole and never write
it away). `[wallpaper]` section keys are the deferred half, split to
`../config-workspace-wallpapers.md`: `apply-config` leaves mappings
alone. E2E on headless scoot (switches by real pixels, restore across
restart); unit tests for the tracker, choices arbitration, protocol and
format, each proven by revert-run-restore.
