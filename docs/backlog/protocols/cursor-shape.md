---
title: "`cursor-shape-v1`."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# `cursor-shape-v1`.

`cursor-shape-v1`. User request, 2026-09-13. Lets a client (modern
GTK4/Qt6 toolkits increasingly prefer this) request a named cursor
shape (e.g. "text", "grab", "not-allowed") without needing to load an
xcursor theme client-side. Worth noting alongside the still-open
custom-cursor-theme-name backlog entry above: this protocol is a
parallel path to that problem, not a duplicate of it — a client using
`cursor-shape-v1` doesn't need flexwm to have loaded a real xcursor
theme at all, since the compositor can map the requested shape name to
its own procedurally-drawn cursor (as items 5/13 already do) rather
than needing theme assets. Might reduce how much the theme-name gap
above actually matters in practice, for clients that adopt this
protocol. No design work done.
