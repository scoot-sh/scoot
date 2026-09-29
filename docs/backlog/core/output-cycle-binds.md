---
title: "Super+comma / Super+period cycle every monitor left and right, wrapping, not just screens 1 and 2"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Cycle outputs left and right, in a loop

Filed 2026-09-29. Serves **daily-drive**: with three or more monitors the
default binds reach only the first two.

## Where things stand

`docs/configuration.md#moving-across-outputs` documents four default binds:

```toml
"super+comma"       = "focus-output-index 0"
"super+period"      = "focus-output-index 1"
"super+shift+comma" = "move-window-to-output-index 0"
"super+shift+period"= "move-window-to-output-index 1"
```

So `Super+,` always jumps to the first screen and `Super+.` to the second: they
are **absolute positions, not a direction**. A third monitor needs hand-written
binds, and neither key steps or wraps. Positions are creation order, "left to
right as the outputs are packed", so left and right already have a meaning.

## What to build

- **Relative actions**: focus the output to the left / right of the focused one,
  and carry the focused window there, **wrapping**: left of the leftmost is the
  rightmost, and the reverse. With one output, do nothing; with two, either key
  goes to the other.
- **Defaults**: `Super+comma` = left, `Super+period` = right,
  `Super+Shift+comma` / `Super+Shift+period` = move the window left / right (the
  same bare-focuses, Shift-carries split the workspace digits keep). The
  `-index N` actions stay for anyone who wants a fixed screen.
- **Order**: by geometry, x then y, which today equals creation order because the
  outputs are packed left to right. Say so, so it stays right when outputs become
  placeable (see [per-output scale/mode](per-output-scale-mode.md)).
- **Naming** (settle it): `focus-output left|right` matches the workspace pair
  (`focus-workspace up|down` beside `focus-workspace-index N`) but overloads
  `focus-output ID`; a distinct verb avoids the overload. Whatever is chosen
  parses unambiguously and is additive on the wire like `focus_output_index`,
  so `PROTOCOL_VERSION` need not change.
- **Semantics unchanged from `focus-output`**: it focuses the target's active
  workspace, or nothing if that holds no windows; refused while the session is
  locked; the pointer behaves as it does for `focus-output-index` today (check
  and pin it, do not guess).
- **Core**: the wrap and ordering are pure functions of the ordered output list,
  so they belong in `scoot-core` with unit tests, keeping it platform-independent.

## Behavior change to state plainly

On two monitors, `Super+,` used to mean "go to screen 0" and now means "step
left, wrapping". From the left monitor that lands on the right one. Note it in
the docs and the changelog; the old behavior is one bind away.

## Edge cases to pin

Zero and one output; the focused output removed while cycling; an output added
mid-cycle; an output whose workspaces were adopted by another after an unplug
(it is gone from the ring, and returns to its place when it comes back);
identical x positions; a window carried to an output with a fullscreen window.

## Docs

`docs/configuration.md` (the default binds and the section above), `docs/ipc.md`
(the action list and the wire form), the README keybinding table, and
`scoot --print-default-config`, in the same PR.

## Not in this ticket

Up/down navigation and directional-by-geometry across a staggered layout.
