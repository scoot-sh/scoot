---
title: "An action that sets a column's width directly, not just cycle-column-width"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# An action that sets a column's width directly, not just cycle-column-width

Filed as gh issue #204 (2026-09-21). `cycle-column-width` is the only way
to change a column's width, so "make this window as wide as the output" is
N keypresses away with N depending on where the column already is in the
cycle. There is no way to *land* on a width. The reporter's case: a single
key widening the focused column to the whole output (the closest thing to
a "fullscreen" scoot deliberately has no concept of) — today that means
appending `1.0` to `[layout] column_widths` and binding a second key to the
same cycle, which still steps past every other entry.

Proposed shape (offered, not assumed):

```
set-column-width INDEX     # index into [layout] column_widths, like focus-workspace-index
```

## Design notes from the issue (kept, not decided)

- **Index, not a fraction.** `focus-workspace-index N` establishes indexing
  into a configured list as the shape; an index keeps the config the single
  place widths are defined. A fraction argument would put layout values in
  two places; if preferred, clamp to the same `0 < w <= 1` rule
  `column_widths` uses.
- **Toggle is a separate ask.** Widen-to-full-then-restore needs per-column
  remembered state — a bigger change. Out of scope here; a plain set plus
  two binds covers most of it. Reporter is not asking for it.
- Reporter offered a PR if this shape is the one wanted.

## Why it matters beyond convenience

`column_widths` is startup-only (a reload refuses it), so the current
workaround costs a session restart to add the full-width entry — while
binds re-apply live, so the action itself would be live-bindable on its
own.

## Precedent

`resolved/workspace-index-keybindings-done.md` (2026-09-20): new
`MoveWindowToWorkspaceIndex` core/IPC/config action mirroring
`FocusWorkspaceIndex`'s ignore-out-of-range rule, with numbered binds.
Follow the same path: `scoot-core` `Action` (`world/actions.rs`, stepped
`cycle_preset` at `world/tree.rs:152-154` stays), `scoot-ipc` action
grammar (`ipc/action.rs`, `CycleColumnWidth` at `:43`), config
emit/parse (`compositor/config.rs:751-760`), docs (`configuration.md`
binds + README Keys), harness tests (`world/tests/columns.rs`,
`workspaces.rs` index suites as template). Check the
`PROTOCOL_VERSION` bump rule for a new action variant before landing
(reload's new variant bumped 2 → 3).

## What done looks like

- `set-column-width N` lands on entry N (out-of-range rule stated like the
  workspace-index one), one keypress to full width, no cycle stepping.
- No default bind required (workspace-index shipped its own; decide
  explicitly like milestone 19 phase F did for output binds).
- Toggle/memory explicitly still out.
