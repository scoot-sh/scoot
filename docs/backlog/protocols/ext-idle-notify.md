---
title: "`ext-idle-notify-v1` and `idle-inhibit-unstable-v1` \u2014 pairs naturally with session-lock."
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# `ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally with session-lock.

`ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally
with session-lock. User request, 2026-09-13. `ext-idle-notify-v1` is
what lets an external tool (a `swayidle`-style daemon) learn "the user
has been idle N seconds" so it can dim the screen, lock it, or suspend
the machine — without it, `ext-session-lock-v1` above has no automatic
trigger, only a manual one. `idle-inhibit-unstable-v1` is the reverse:
lets a client (a video player, a presentation app) tell the compositor
not to consider the session idle while it's active. Neither has design
work done; natural to scope alongside session-lock since they're the
same feature area (idle/lock lifecycle), not before it. Now the obvious
next pick in that area: item 18 landed the lock itself, and without an
idle notification nothing can trigger it automatically — locking is
whatever the user runs by hand.
