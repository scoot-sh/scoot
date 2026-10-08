---
title: "Tray: themed icon names"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-07"
---

# Tray: themed icon names

Filed 2026-10-03, split out of [tray](resolved/tray-done.md) when its menus landed.
Serves **daily-drive**: most GTK and Ayatana apps send only `IconName`.

## The gap

An item that sends only `IconName` (no `IconPixmap`) is tracked and
clickable by index but takes no room: there is no icon-theme lookup and
no image decoder in this build
(`crates/scootbar/src/modules/tray/item.rs`: `shown()` needs
`!icons.is_empty()`). Attention and overlay icons, tooltip icons and
`IconThemePath` are likewise read for shape and dropped. So on a
real-world desktop a large fraction of tray icons are invisible.

## What to do

- Measure first, per the [tray](resolved/tray-done.md) "what to decide" rule: an
  icon-theme lookup (hicolor search across `$XDG_DATA_DIRS`) plus an
  image decoder (PNG at least, SVG would pull a renderer) against the
  [resource ratchet](lightest.md), before enabling by default. Pixmaps
  came first precisely because this cost was unmeasured.
- Draw themed names through the shared icon cache at the output's
  device pixels, like pixmaps; keep the hostile-item bounds (name
  length, lookup cost per icon version, not per frame).
- Attention/overlay icons and `IconThemePath` with it, or say why not.

## Not in this ticket

Menus (landed with [tray](resolved/tray-done.md)); tooltip icons.

## Status (2026-10-07): themed names draw through the shared icon cache

`IconName` (and `AttentionIconName` while `NeedsAttention`, preferring
it over the main name for items without a pixmap) resolves through a `hicolor` lookup across
`IconThemePath` first, then `$XDG_DATA_DIRS`/`$XDG_DATA_HOME` and the
legacy `/usr/share/pixmaps`, and decodes PNG only, with the `png`
decoder the `icon-image` feature already carries (same 0.18.1 lock
entry, MIT OR Apache-2.0: `cargo deny check` stays green, no new
package in the tree). The decoded icon joins the item's `icons` once
per `GetAll` answer (per icon version, on the bus turn, never per
frame) and draws at the output's device pixels like a pixmap; a
missing, SVG-only, or hostile name stays tracked-but-hidden as before.
Hostile bounds: names at most 128 bytes with no `/` and no leading
dot (no traversal, no absolute path), theme paths absolute with no
`..`, every candidate canonicalized back inside its base, files at
most 8 MiB under a 16 MiB decode budget with the header's size checked
first (512 a side, 1 M pixels). Overlay icons are read and not drawn
(a badge would be a second scaled draw a frame for something real
items rarely send); full theme inheritance past `hicolor` is future
work (the bar knows no theme setting yet). Theme changes arrive
lazily, on the item's next update or restart: nothing polls the theme
directories.

Measurement note (corrected 2026-10-08: the paragraph below first said
no Linux numbers were captured; they were, in the PR body, the proof
round and the reviews, and the maintainer waived the size row on
2026-10-07): release builds (`lto = "fat"`, stripped, aarch64) against
`main` at `ae10e90bc` measure the themed-icon decoder at release file
+131,072 B, `.text` +65,760 B, `.rodata` +24,568 B in the default build;
idle wakeups, fds and threads do not move, and the idle RSS reading
(+about 150 kB over base, inside the run-to-run noise) stays a
measurement, not an accepted cost. The gated alternative (themed icons
behind `icon-image`, default build +2,272 B `.text`) was measured and not
chosen. See the [resource ratchet](lightest.md) for the table and the
2026-10-07 waiver, which covers this row only. The pasystray case is
covered by a fixture-theme test (`IconName`-only answers draw once
resolved, attention prefers the alarm name for items without a pixmap,
missing names stay hidden), and the traversal/bomb cases by unit tests.
`cargo deny check` passes; `scripts/backlog check` shows only its 3
pre-existing problems.
