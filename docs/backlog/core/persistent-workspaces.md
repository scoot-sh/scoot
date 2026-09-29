---
title: "Decision: should scoot offer persistent (fixed-count) workspaces alongside the dynamic set?"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Persistent workspaces: a decision

Filed 2026-09-29 (scootbar research). Serves **daily-drive**, and is a
deliberate design question, not a bug.

## The evidence

Persistent workspaces (always show, and be able to switch to, slots 1..N even
when empty) are the most-requested workspace feature of the leading bar: Waybar
#1629 has 44 comments and 47 hearts, and says hiding empty workspaces "hurts
ricing and usability on anything other than sway". So users will ask scoot's
bar for it.

## Where scoot stands

The workspace set is **dynamic on purpose**: empty workspaces are dropped, and
targeting one that does not exist does nothing
(`docs/configuration.md`, "Targeting a workspace that doesn't exist yet";
`docs/ipc.md`). `Super+9` with three workspaces open is a no-op by design. A bar
cannot honestly draw slots scoot cannot switch to, so
[the bar's workspaces module](../../scootbar/backlog/resolved/workspaces-module-done.md)
shows what scoot reports and does not synthesize slots.

## What to decide

- Keep the dynamic model as the only one (close this ticket as a deliberate
  refusal, and document it in the bar's docs so the first user who asks gets an
  answer), **or**
- add an opt-in `[workspaces] persistent = N` (name to taste): the first N
  workspaces of each output always exist, empty or not; `focus-workspace-index`
  and `move-window-to-workspace-index` reach them; the `ext-workspace-v1` list
  includes them, so every bar works with no special case.

## If it proceeds

The core keeps N slots per output through window churn, adoption from an
unplugged monitor, and restore (`output-reconnect-restore-done.md`); the
trailing empty workspace still exists after them; index numbering and the
`windows` snapshot stay consistent; the default (unset) is byte-identical to today.
Design against what niri and others do, but do not copy their code (GPL,
`CLAUDE.md`). Fail-first tests per transport, and the docs said in the same PR.
