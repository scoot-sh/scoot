---
title: "`exec`, `push` and `button` modules: extend the bar without Rust"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "pointer-and-interactions"
---

# `exec`, `push` and `button` modules

Filed 2026-09-29. Serves **daily-drive** (launcher and power buttons) and
**computer use** (anything can put text on the bar).

The escape hatches that keep the built-in module set small.

- **`button`**: an icon or text and the interaction keys. A launcher button is
  `on-click = { exec = ["scootlaunch"] }` (fuzzel or wofi until ours exists);
  log out is `{ scoot = "quit" }` or `exec = ["scoot","msg","action","quit"]`.
- **`exec`**: run a command and treat each stdout line as an update, plain text
  or one JSON object (`text`, `class`, `tooltip`). The child's stdout is a
  polled fd; it restarts with backoff if it exits, and a chatty child cannot
  grow memory (line cap, drop the excess with a warning).
- **`push`**: `scootbar msg set ID JSON` updates a module from anywhere. No fd
  of its own beyond the control socket, so it costs nothing until used.

scootbar's own JSON shape, deliberately not Waybar's; document it and version
it. Both get the standard interaction keys. A module with neither output nor
push yet shows nothing (or a configured placeholder), not an error.

## Limits and safety

Command lines are never passed through a shell unless the user writes `sh -c`
explicitly. The control socket is `0600`, same-user, and `set` only changes
display text; it cannot run anything. Cap line length, module count and
update rate (coalesce to the next frame).

## Done when

A shell one-liner becomes a module with a click action, and `scootbar msg set`
updates it with no polling on either side.
