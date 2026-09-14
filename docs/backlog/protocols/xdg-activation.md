---
title: "`xdg-activation-v1`."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# `xdg-activation-v1`.

`xdg-activation-v1`. User request, 2026-09-13. Lets one client
politely request that another be raised/focused — e.g. clicking a
notification should focus the app it's from, or a taskbar's "flash to
focus" behavior. Without it, the only way to change focus is flexwm's
own keybindings/IPC; a client has no standard way to ask. Real
daily-drivability gap, no design work done, presumably a small,
well-scoped protocol relative to layer-shell/workspace/session-lock.
