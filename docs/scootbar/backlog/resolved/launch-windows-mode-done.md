---
title: "scootlaunch: a windows mode over wlr-foreign-toplevel-management"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-11-05"
---

# scootlaunch: windows mode

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#windows-mode). Serves **daily-drive**. Low,
and deliberately after the profile switch: it is not needed to replace fuzzel.

## What to do

`--windows` lists **and** activates through `zwlr_foreign_toplevel_handle_v1`, which carries
title, app_id and state, with `activate(seat)`. `ext-foreign-toplevel-list-v1` is not used: its
handles share nothing with wlr's, so it could not be activated from. Icons come from `app_id`
matched to a desktop id or `StartupWMClass`.

## Acceptance

Headless tests on scoot and on sway: three windows listed, and the chosen one focused.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/panel.md` with the windows mode, its key and what it lists. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Window actions beyond activate.

## Resolution

Landed 2026-11-05 on `feat/scootlaunch-windows-mode` (PR number in the
PR itself; `feat(scootlaunch): windows mode`).

What shipped: `scootlaunch --windows` (`src/windows.rs`, the protocol
session; `src/windows/dispatch.rs`, its events; `src/windows/entries.rs`,
the rows and the icon match), the `Mode::Windows` arms in `src/panel.rs`,
the CLI flag, `StartupWMClass` read into the index (index cache format
version 3, a one-time rescan), `tests/windows.rs` with its toy `xdg_toplevel` client
and sway virtual keyboard, a sway fetch in the scootlaunch CI job, and the
`panel.md` / `cli.md` / `index.md` entries under `site/`.

Decisions the design did not settle:

- **Snapshot at open.** The list is read once; a title change or a new
  window is not shown, and a window that closes stays in the list but is
  refused at pick time (`that window closed before it could be
  activated`, exit 1), because a request on a closed handle is a protocol
  error.
- **The pick is activated after the panel is gone** (the layer surface's
  exclusive keyboard is released first), on a second event queue of the
  same connection that holds the handles.
- **Order** is by title, then announce order (row ids are zero-padded so
  the ranker's text tie-break keeps it): windows have no frecency, so
  none is read or written.
- **Shares the single-instance toggle** with apps and run (dmenu never
  did).
- **With a motion set, activation follows the fade-out**; the keyboard
  is released at its first frame.
- **Icon match order:** desktop id, `StartupWMClass`, then each ignoring
  ASCII case; no match tries the `app_id` as an icon name.
- **No default key.** The docs show a `[binds]` example.
- **Ctrl+Enter does nothing** (there is no typed command to run).
