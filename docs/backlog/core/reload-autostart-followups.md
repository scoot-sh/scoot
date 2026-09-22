---
title: "Config reload autostart follow-ups (review findings from PR #214, filed not fixed)"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Config reload autostart follow-ups

Filed from the `scoot-reviewer` report on PR #214 (Phase 4 + reword,
merge `56f98a1`), which landed with a clean gate. None strands users,
kills clients, or misleads agents badly enough to have blocked merge —
but none should evaporate either. All three live in the autostart side of
`apply_reload` (`crates/scoot/src/compositor/reload.rs`,
`apply_autostart_reload`).

## 1. A spawn that fails still advances the snapshot and reports `applied`

`apply_autostart_reload` runs `self.act(...)` in a loop, then
unconditionally pushes `applied` and clones `fresh.autostart` into
`startup_autostart`. But `State::spawn` maps `Command::spawn` failure to
a `warn!` log and returns — so `commands = ["spawn
/nonexistent-prog-xyz"]` logs `could not spawn`, the IPC reply says
`applied: ["autostart.commands"]` though nothing started, and the entry
is now "seen" so a second reload never retries it even after the binary
appears. Matches startup's fail-open/no-supervision doctrine and fails
loudly in the log, so the shape is honest everywhere except the reply —
which, unlike startup (where the user watches their program not
appear), is the reload's only signal. Fix shape: advance the snapshot
per-entry past only what `spawn` accepted, or at minimum a doc line
that a failed spawn is decided, not retried. Unpinned either way today
(no test covers the failed-spawn path).

## 2. The locked-skip message over-promises for non-spawn deltas

Under lock with a non-empty delta the reply is `"skipped while locked:
new entries run on the first unlocked reload"`. While locked, the file
gaining only `"quit"` gets that promise — but on unlock the entry lands
in `refuse` (refused by name, never runs). Harmless and accurate for
the spawn case it was written for; strictly false for the quit-only
case. One-word-class fix (`"pending entries are decided on …"`) or
leave.

## 3. Removal-while-locked cancellation is traced sound but unpinned

The delta is recomputed from the live file on every reload and the
snapshot freezes under lock, so add-while-locked → skip →
remove-while-locked → silent → unlock → silent means the entry correctly
never runs (nothing is ever queued). No test pins that three-step
sequence — the exact disclosure/interference shape the lock-skip exists
to prevent. Cheap insurance alongside a pin for finding 1's
failed-spawn path.

## What done looks like

Finding 1 decided (per-entry snapshot or documented decided-not-retried,
pinned), finding 2 reworded or explicitly accepted, finding 3 pinned.
One small PR, no protocol change, docs only where behavior is
documented.
