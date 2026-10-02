---
title: "Window title module: the focused window's title, click to focus"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-01"
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
  [icons](resolved/icons-and-fonts-done.md) decides a path.

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

## What landed (PR #374, 2026-10-01)

The `window-title` module (`crates/scootbar/src/modules/window_title/`,
Cargo feature `window-title`, on by default), reference in
[cli.md](../../cli.md#window-title). Each output shows the title of the
window activated on it (empty or `placeholder` when none is focused);
`show-app-id` appends the app id, `max-width` (default 480 logical pixels)
caps the span with a pixel-measured ellipsis, a left click activates, and
a middle click / the `close` action close only with `allow-close = true`
(off by default; a `close` binding with it off is refused at read).
`query` carries the text and `{"title", "app_id", "fullscreen"}`; `invoke`
takes `activate` and `close` through the shared interaction keys.

Only the wlr manager is bound (while placed): the ext list has no
`activated` state, no output events and no requests, and the two protocols
share no client-visible key, so their handles cannot be correlated.

Evidence (dev VM, `SCOOTBAR_REQUIRE_SCOOT=1`): `cargo nextest run -p
scootbar` 747 passed (19 harness tests, 6 headless-scoot integration
tests: focus across outputs, click routing, close gating incl. reload,
control/CJK, flood bound, max-width cut); `cargo test -p scootbar` all ok;
clippy `-D warnings` clean on every feature combination; per-feature
`--bin` counts all pass. Flood probe (release, ~365 titles/s, 20 s):
10 jiffies (0.5% CPU), 9.9 events/s, RSS flat, against 27 jiffies and 59.8
events/s uncapped and 0 jiffies idle. Not run: `nix build` (no new
dependencies), sway/tty hardware, the Asahi ratchet.

The module API is extended, not frozen (`Module::max_width`,
`CustomDraw.hovered`), per the umbrella: volume and network have not
exercised it yet. App icons stay out of scope (`xdg-toplevel-icon`).
