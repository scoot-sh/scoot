---
title: "scootlaunch: the panel, keys, the virtualized list, the look layer"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M7"
---

# scootlaunch: the panel

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#keyboard-first-ux); split in the #579 fix
round. Serves **daily-drive**. `look-format` is the live-looks `scoot-look` crate (#578, merged).

## What to do

- The `overlay` exclusive-keyboard panel with the entry and the virtualized list.
- Keys and mouse per the design.
- The single-instance toggle over a socket file in `$XDG_RUNTIME_DIR` (`0700`), with the peer's uid checked through rustix's `socket_peercred`. There is no pid file and no abstract name another user could bind first.
- A null output: the core ticket `layer-shell-default-output` decides which output that is.
- The look layer, read at every open: `<state>/look/launch.toml`, the allowlist and the grammar.

## Acceptance

Headless tests:

- open, type through `scoot msg type`, and select;
- the toggle closes a running instance;
- goldens per shipped look against today's fuzzel colors (the design's table);
- zero allocation per keystroke.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Launching (`launch-spawn`), icons (`launch-icons`).
