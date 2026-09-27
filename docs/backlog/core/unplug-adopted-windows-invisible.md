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
nothing on screen said where the window went. From the person at the machine: "hasn't shown back on
the original screen yet", then "it feels like they disappear".

The window is not lost. `Super+2` or `Super+Ctrl+j` reaches the adopted workspace, and a
replug restores it (PR #249, proven live in Tests 11 and 12). But a user who
does not know the design sees their work vanish, which is the failure that
matters for a daily driver.

The current behaviour was chosen for a reason, and any fix must keep it
(`docs/backlog/resolved/output-reconnect-restore-done.md`): windows do not
pile onto the panel after every screen-saver cycle, and restore moves back
exactly the adopted block, in order, with the snapshot's active index.

## Candidates (pick one with evidence, don't stack them)

1. **Show the adopted work when focus was on the removed screen.** If the
   focused window was on the removed output, activate the adopted
   workspace that held it on the adopter and keep focus on that window. If
   focus was elsewhere, change nothing, as today. This is closest to "my
   window moved to the other screen". Restore has to keep working. It
   already restores the removed output's own snapshot and "focus stays where
   it was". The adopter would then be left looking at the now-empty slot, so
   restore should also return the adopter to its pre-adopt active workspace
   when the adopted one is still active. That makes it a change to
   `scoot-core` (`remove_output` + `restore_output`), so it needs the fuzz
   targets and tests that guard that crate. Two things to settle first:
   - **Where focus lands on restore.** Today restore never steals focus
     (`restoring_moves_the_still_open_windows_back_in_order` pins
     `focused_output` staying on the adopter). With candidate 1 the
     focused window may be the one carried back, so pick a rule: focus
     follows it to the returned monitor, or stays on the adopter's
     restored view.
   - **Recording the adopter's previous active index.** Returning the
     adopter to its pre-adopt view means storing that index in
     `EvictedOutput`. `normalize` can shift it if the user's workspaces
     change in between, so store it relative to the adopted block (or
     verify it) rather than as a raw index.
2. **A visible cue without changing the layout.** The cheapest form needs
   no scoot change: a bar that speaks `ext-workspace-v1` already shows
   workspace 2 appear on the panel at the moment of the unplug. Test 12
   ran without a bar, which is part of why it read as a disappearance. A
   stronger form is an OSD or notification ("DP-1 disconnected: 1 window
   moved to workspace 2"), which needs a path scoot does not have yet.
   Either way the window stays a keystroke away, so this is a cue, not a
   fix.
3. **Merge into the active workspace.** Rejected unless 1 and 2 fail: it
   brings back the pile-up the current design exists to prevent, and it
   makes "restore exactly what was adopted" much harder.

Recommendation: 1. Measure it against the screen-saver case: a monitor that
drops and returns while the user is on the panel must not leave the panel
on a different workspace afterwards.

## Done when

- Unplugging a monitor while working on it leaves that work visible and
  focused on the remaining screen.
- A replug restores both the returned monitor and the adopter's previous
  view.
- The no-focus-on-removed-screen case is unchanged.
- Core tests pin all three, and a live re-run (virtual-pull rig, or hands)
  shows it.
- README's multi-monitor text says what happens on unplug.
