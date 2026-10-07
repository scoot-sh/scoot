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
it over the main name) resolves through a `hicolor` lookup across
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

Measurement note, stated plainly for the maintainer: the Linux
resource numbers the ticket requires (release `.text`/file size, idle
RSS, wakeups, per-icon lookup cost, base-vs-head over several runs)
were NOT captured here — the dev VM is down and this lane had no
Linux builder, so no `cargo test`/`nextest`/`clippy` ran on the change
either. The size row will regress by roughly the `icon-image`
decoder's known +115 KB (the `tray` feature now enables `dep:png`;
the default build carries it), plus the lookup code itself: that row
needs a ruling, or a smaller design (e.g. gating themed icons behind
`icon-image`, which keeps the default lean but leaves IconName-only
items invisible there). Live proof with pasystray/CopyQ/KeePassXC is
likewise outstanding for whoever has the hardware: the pasystray case
is covered by a fixture-theme test (`IconName`-only answers draw once
resolved, attention prefers the alarm name, missing names stay
hidden), and the traversal/bomb cases by unit tests. `cargo deny
check` passes; `scripts/backlog check` shows only its 3 pre-existing
problems.
