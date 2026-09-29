---
title: "Window title module: the focused window's title, click to focus"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "pointer-and-interactions"
milestone: "M5"
---

# Window title module

Filed 2026-09-29. Serves **daily-drive**; also the cheapest way for an agent
to read what is focused through `query`.

## Source

`wlr-foreign-toplevel-management-v1` (title, app id, `output_enter`, the
`activated` state bit, `activate` and `close` requests) and
`ext-foreign-toplevel-list-v1`, both implemented in scoot
(`docs/protocols.md#window-lists-two-protocols`). Event-driven, no polling. The
foreign-toplevel identifier carries the `scoot msg windows` id, the bridge for
an agent.

## What to build

- Show the activated window's title (and app id, if configured) for the bar's
  own output; empty or a placeholder when none is focused.
- Truncate by measured pixel width with an ellipsis, not by character count;
  the module gets a maximum width from config and yields the rest to others.
- Click focuses/`activate`s; middle-click `close`s (behind a config key, off by
  default: closing a window by an accidental click loses work).
- Later, the app's icon from `xdg-toplevel-icon` (name only in scoot today) once
  [icons](icons-and-fonts.md) decides a path.

## The edge that matters

A terminal or browser can retitle at hundreds of events per second (a progress
readout). Coalesce to one redraw per frame, damage only this module's rect,
and cap the update rate; measure a title-flood against idle CPU. Titles are
untrusted text: bound the length, and draw control characters as nothing
rather than passing them to the glyph cache.

## Done when

Title follows focus across outputs on headless scoot, a title flood costs a
bounded redraw rate, and a title with control characters and CJK renders
safely.
