---
title: "Config reload autostart follow-ups (review findings from PR #214, filed not fixed) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Config reload autostart follow-ups — RESOLVED

RESOLVED 2026-09-22 (PR #TBD, coordinator-filed ticket, no gh issue):
all three findings decided in one small PR, no protocol change
(`Response::Reloaded` keeps its `{applied, refused}` string shape, so no
`PROTOCOL_VERSION` bump).

1. **Failed spawn vs snapshot/reply — per-entry snapshot.** `State::spawn`
   now reports acceptance (`bool`: `true` when the child started, or when
   there was nothing to start; `false` on `Command::spawn` failure, with
   the OS error in the existing `warn!`), and `State::act` folds it across
   the action's effects (`&=` so a bad entry mid-list cannot stop the
   entries after it; vacuously `true` for spawn-free actions, `false` when
   locked). `apply_autostart_reload` advances `startup_autostart` past
   decided entries only (accepted spawns, refused non-spawns): a failed
   spawn is refused by name (`still pending, retried on the next reload`),
   stays out of the snapshot, and retries on the next reload -- running
   exactly once if its program has appeared by then. Threading the `bool`
   through `act` rather than documenting decided-not-retried because the
   change is three lines at each of two sites (all 30+ existing `act`
   callers use statement position, so the return type change is
   source-compatible) and it makes the reply honest instead of merely
   documented: `applied` means the entry started. Pinned fail-first, plus
   bug-bash pins (mid-list failure, duplicate failures, command-less
   entries as load-time skips).
2. **Locked-skip message — reworded.** `"skipped while locked: pending
   entries are decided on the first unlocked reload"`: no more promised run
   for the quit-only case (which decides as a refusal by name on unlock).
   Pinned end to end with a quit-only locked→unlocked test.
3. **Removal-while-locked cancellation — pinned.** Add-while-locked →
   skip (snapshot frozen) → remove-while-locked → silent → unlock →
   silent, entry never runs, with the real lock-client pattern PR #214
   established (extended with an `unlock_and_destroy` step; the helper
   waits for the `locked` event first, since Smithay only routes the
   unlock once confirmation has been sent). Sensitivity proven by
   neutering (advancing the snapshot under lock turns the pin red).

Docs only where behavior is documented: the reply strings and lock
paragraph in `docs/configuration.md` (`Reloading the config`, `[autostart]`
reload rule), the `reloaded` paragraph in `docs/ipc.md`, and the reload
bullet in `README.md`. Every claim cross-checked against the pins. No
benchmark: config reload is a cold path (stated, not measured).
Original entry below, kept verbatim.

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
