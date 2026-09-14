---
title: "Foreign-toplevel management (window enumeration for external tools)."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Foreign-toplevel management (window enumeration for external tools).

Foreign-toplevel management (window enumeration for external tools).
User request, 2026-09-13. `ext-workspace-v1` above covers workspaces;
nothing today gives an external client (a taskbar, an alt-tab switcher
applet) the equivalent view of *windows* — only flexwm's own IPC
(`flexwm msg windows`) has that. Check which protocol is actually
current before implementing: `wlr-foreign-toplevel-management-unstable-v1`
is the older, widely-supported one; there may be a newer `ext-` successor
by the time this is picked up, and per `CLAUDE.md`'s standing preference
for the compositor-agnostic successor where one exists, that should win
if it does. No design work done.
