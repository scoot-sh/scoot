---
title: "Screen capture for clients (`wlr-screencopy` / `ext-image-copy-capture`) for shell thumbnails and previews."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Screen capture for clients (`wlr-screencopy` / `ext-image-copy-capture`) for shell thumbnails and previews.

Both shell probes hit this (DMS gap 6, Noctalia gap 6, 2026-09-14):
launcher window thumbnails and workspace-overview live previews
(`ScreencopyView`) have nothing to consume — neither global is
advertised. Previously bundled in
`docs/backlog/protocols/protocol-gaps-niche.md`.

This is purely a shell-client gap, not an agent gap: flexwm's own
screenshots for computer-use automation go over the privileged IPC
socket (`flexwm msg screenshot`, owner-only, rate-limited), which stays
regardless. The standard-protocol path is for third-party tools
(`grim`, `wf-recorder`, shell thumbnails, conferencing screen-share)
that will never speak flexwm IPC.

Per the standing rule, prefer the standard protocol: evaluate
`ext-image-copy-capture-v1` (with `ext-image-capture-source-v1`)
first, `wlr-screencopy-unstable-v1` for legacy clients second. Note
the privacy interaction with session lock: captures taken while
`ext-session-lock-v1` holds the session must see only the lock
framebuffer, never the windows behind it — the same guarantee
`flexwm msg screenshot` already gives. Rough size: M.
