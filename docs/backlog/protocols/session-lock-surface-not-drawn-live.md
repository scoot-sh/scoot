---
title: "A lock surface mapped after the confirming frame never appears on live `--tty`"
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# A lock surface mapped after the confirming frame never appears on live `--tty`

Locking on dev-VM `--tty` and mapping a lock surface afterwards blanks
the screen, but the surface's own pixels never appear: locked IPC
screenshots stay backdrop-black (plus two stray magenta pixels at the
left edge, mechanism not isolated), while the session is genuinely
locked (`locked: true` on IPC replies, `locked` duly arriving over the
wire).

Proven pre-existing, not a regression from the vblank-confirm change:
the identical black screenshot (same two stray pixels) reproduces on the
pre-change binary, and the harness covers this path green — a surface
mapped after the lock appears through timer-driven renders alone when
checked there.

The harness/live delta points at damage under real buffer ages: with age
0 every render redraws everything, while under `--tty` a surface
committed after the confirming frame appears to get no damage redraw —
cursor sweeps across the whole screen do not reveal it either. Prime
suspect, not yet isolated: nothing feeds a lock surface commit's damage
to the tracker the way `Space`/`LayerMap` do for windows and layer
surfaces.

Why this is high priority next to the guarantee just closed: a lock
screen that never shows the locker leaves the user staring at black
with no visible password prompt. Keyboard focus still lands on the
surface (liveness-based), so blind entry may work, but that is not a
lock screen anyone can daily-drive.
