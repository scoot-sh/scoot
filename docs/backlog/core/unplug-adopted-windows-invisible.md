---
title: "An unplugged monitor's windows seem to disappear"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# An unplugged monitor's windows seem to disappear

Filed 2026-09-26 from `Asahi.md` Test 12 (a real DP-1 unplug on the Asahi
M2 Air, GPU tier, `main` `87e1935`). Serves **daily-drive**: a docked laptop
losing its monitor is routine (cable, dock, screen saver, KVM).

## What happens

With a window focused on DP-1, pulling the cable removes the output within
about 30–60 ms (57 ms and 30 ms in the two pulls measured). scoot-core's
`remove_output` (`world/events.rs`) then adopts the monitor's workspaces
onto the remaining output after its own, "without moving focus". Here that
meant eDP-1's workspaces became [1: window 1] [2: window 2] [3: empty]. When
focus was on the removed output, the adopter is chosen by index, not by
focus: the output that slides into the removed one's index, or the last
output. That only matters with three or more outputs. The adopted
workspaces are inactive. Nothing on the panel changes except the focus
ring: the window reports `output: 1, visible: false` and focus falls to
whatever was already on the panel. There was no bar in that session, so
nothing on screen said where the window went. From the person at the
machine: "hasn't shown back on the original screen yet", then "it feels
like they disappear".

The window is not lost. `Super+2` or `Super+Ctrl+j` reaches the adopted
workspace, and a replug restores it (PR #249, proven live in Tests 11 and
12). But a user who
does not know the design sees their work vanish, which is the failure that
matters for a daily driver.

The current behaviour was chosen for a reason, and any fix must keep it
(`docs/backlog/resolved/output-reconnect-restore-done.md`): windows do not
pile onto the panel after every screen-saver cycle, and restore moves back
exactly the adopted block, in order, with the snapshot's active index.

## Requirement

From the user (2026-09-26): "It should be clear to the user where things
went. And that means the now off monitor's workspace(s) and the on one that
may have swapped." After an unplug, two things must be legible, not just
recoverable:

- **Where the removed monitor's workspaces went:** which workspaces on
  which remaining screen now hold them. There may be several, not just the
  one that had focus.
- **Where the remaining screen's own view went, if it swapped:** with the
  switch below, the panel stops showing what it showed a moment ago. That
  workspace did not move, but it is out of view, and the user must be able
  to tell that and get back to it.

"Findable with `Super+2` if you already know the design" does not meet this.

**Accepted residual (the user's decision, 2026-09-26): a session with no
bar.** There, the switch makes the removed monitor's focused work visible,
but nothing on screen says the panel's own previous workspace went out of
view (it stays one keystroke away: `Super+N` for its number, `Super+1` in
Test 12). A daily-drive setup runs a
bar, and the bar shows the swap. A cue that scoot draws itself (a switch
animation or a text label) is out of scope here.

## Design

### 1. Show the adopted work when focus was on the removed screen

If the focused window was on the removed output, activate the adopted
workspace that held it on the adopter and keep focus on that window. If
focus was elsewhere, change nothing, as today. The panel's previous
workspace keeps its index (workspace 1 in Test 12), one keystroke away
(`Super+1`, `Super+Ctrl+k`).

Restore has to keep working. It already restores the removed output's own
snapshot and "focus stays where it was". The adopter would then be left
looking at the now-empty slot, so restore should also return the adopter
to its pre-adopt active workspace when the adopted one is still active.
That makes it a change to `scoot-core` (`remove_output` +
`restore_output`). There are no cargo-fuzz targets. Extend the randomized
invariant test (`world/tests/invariants.rs`), which today sends
`OutputRemoved` but never evicts or restores, to cover evict → switch →
restore. Update `evicting_reports_the_adopter_and_adopts_without_moving_focus`,
whose premise (focus on the removed output, from `two_workspace_world`)
changes meaning under this design. Three things to settle first:

- **Where focus lands on restore.** Today restore never steals focus
  (`restoring_moves_the_still_open_windows_back_in_order` pins
  `focused_output` staying on the adopter). With this change the focused
  window may be the one carried back, so pick a rule: focus follows it to
  the returned monitor, or stays on the adopter's restored view.
- **Recording the adopter's previous active index.** Returning the adopter
  to its pre-adopt view means storing that index in `EvictedOutput`.
  `normalize` can shift it if the user's workspaces change in between, so
  store it relative to the adopted block (or verify it) rather than as a
  raw index.
- **The switch itself can shift the adopted block and silently break
  restore (reproduced in review of PR #271).** `normalize` keeps an empty
  workspace while it is active. Suppose the adopter is sitting on an
  empty, non-trailing workspace (the user closed its last window and has
  not left it). That workspace survives `adopt`. Switching away from it
  lets `normalize` drop it, and everything after it shifts down one,
  including the adopted block. `restore_output` carries a window back only
  when it sits at exactly `adopted_at + i`, so it restores nothing. The
  review's scratch run: adopter `{count: 4, active: 1}` (ws1 empty and
  active), `adopted_at = 3`, after the switch `{count: 4, active: 2}`, then
  `restored 0`. The "previous view" to return to no longer exists either.
  This happens with any switch that leaves an empty active workspace, not
  just one implementation. Either record `adopted_at` and the previous view
  *after* the switch's normalize, or define what "previous view" means when
  it was an empty workspace that the switch dropped.

The screen-saver case is the regression to measure against. A monitor that
drops its connection in standby (Test 12 saw this) and comes back, while
the user works on the panel, must leave the panel exactly as it was.

### 2. Make both movements legible through surfaces that already exist

The switch alone answers "where is my window" only for the focused one. To
make the rest legible, in order of cost:

- **Bars, through `ext-workspace-v1` (standard protocol, no new UI in
  scoot).** Today a bar sees the panel's list grow "1 2" → "1 2 3" and,
  with the switch, the active marker move 1 → 2. That shows the swap. But
  scoot names each handle only by its 1-based position (`describe` in
  `compositor/ext_workspace.rs`, kept equal to `coordinates` on purpose),
  and the protocol carries no occupancy, so nothing says which workspaces
  came from DP-1. Candidate: name adopted workspaces by origin (e.g.
  "2 DP-1") until restore or until the user empties them. The protocol
  allows it: `name` is sent "whenever the name of the workspace changes",
  and names need not be unique. The cost:
  - It deliberately breaks the name == coordinates rule `describe`
    documents.
  - Handles are positional, so any renumbering in front of an adopted
    workspace means re-sending `name` on every handle that shifted, not only
    on adopt and restore. `diff::changes` over `Workspaces {count, active}`
    cannot express a rename, so this needs a new snapshot shape and a rename
    `Change`, followed by the manager's `done`.
  - The origin tag must stay connector-agnostic in `scoot-core`. The core
    holds an opaque origin id, and the shell maps it to "DP-1", as it does
    for the `EvictedOutput` key.
  - Define when a tag clears in two cases. After a partial restore,
    windows that stayed on the adopter (moved by hand, or shifted out of
    place by other changes) keep their workspace, which must lose the
    "DP-1" tag. After a chained unplug (three outputs: DP-2 adopts
    DP-1's workspaces, then DP-2 goes too), decide whether the tag says
    DP-1 or DP-2.

  Evaluate against what the common bars display before choosing, and keep
  the default names unchanged for workspaces that were never adopted.
- **Agents and scripts, through IPC.** `windows` has no workspace field
  today, so an agent cannot tell where a window went either (Test 12 had
  to derive "workspace 2" from the code). Add `workspace` (and whether it
  was adopted, from which connector) to `windows`. Make it **0-based**, to
  match `focus-workspace-index`: the `ext_workspace.rs` module doc warns
  about exactly this 1-based/0-based mismatch. It is cheap, and it serves
  computer use directly.
- **An IPC event for "output removed / restored"**: the adopter, the
  adopted workspace range, and the adopter's previous and new active
  workspace. With it, a user who wants a desktop notification wires
  `notify-send` to it, and an agent learns without polling. scoot-ipc has
  no event subscription yet, so this is its own, larger item. File it
  separately rather than blocking this ticket on it. It fires on every
  monitor standby too. That is fine for an event a consumer can filter or
  debounce, and it is the difference from a notification pushed at the
  user unconditionally.

### Not chosen

- **scoot sending or drawing notifications itself.** It would fire on every
  monitor standby (a routine unplug to scoot), for docks and for KVM
  switches. scoot has no D-Bus client, and on a bare tty or in webtop
  there is no notification daemon to show it. An OSD drawn by scoot means
  text rendering in the compositor. Either would tell the user where things
  went, and the user would still have to act. The IPC event above lets
  anyone who wants a notification have one.
- **Merge into the active workspace.** It brings back the pile-up the
  current design exists to prevent, and it makes "restore exactly what was
  adopted" much harder.

## Done when

- Unplugging a monitor while working on it leaves that work visible and
  focused on the remaining screen. The no-focus-on-removed-screen case is
  unchanged.
- A replug restores both the returned monitor and the adopter's previous
  view.
- A bar speaking `ext-workspace-v1` can show which of the panel's
  workspaces came from the removed monitor (by origin naming, or by the
  alternative chosen after evaluating bars), and that the panel's view
  switched.
- IPC `windows` reports each window's 0-based workspace.
- Core tests pin the switch and restore cases, including the
  empty-active-adopter state from the third settle-first item, and the
  invariant test covers evict → switch → restore. Protocol tests pin the
  workspace names across adopt, renumbering and restore. A live re-run
  (virtual-pull rig, or hands) shows it, including a standby
  drop-and-return that leaves the panel untouched.
- README's multi-monitor text says what happens on unplug and how to get
  back, `docs/configuration.md`'s unplug/restore paragraph is updated, and
  `docs/ipc.md` documents the new field.
