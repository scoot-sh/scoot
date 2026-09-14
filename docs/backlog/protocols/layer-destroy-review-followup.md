---
title: "Layer-destroy review follow-up from PR #34 (`mapped_layers` can retain a dead entry on implicit disconnect)."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Layer-destroy review follow-up from PR #34 (`mapped_layers` can retain a dead entry on implicit disconnect).

One non-blocking finding from the independent review of PR #34
(`fix/layer-surface-post-destroy-commit-kill`, merged 2026-09-14),
recorded so it doesn't get lost:

- On implicit client disconnect with the unlucky callback order
  (`layer_shell.rs`, `wl_surface` dead first), `layer_destroyed` never
  runs, and `CompositorHandler::destroyed` (`handlers.rs`) clears the
  cursor and lock surfaces but not `mapped_layers` — so a dead entry can
  be retained. Verified memory-only, never a wrong neutralize:
  server-side `ObjectId` includes `client_id` plus a generation serial,
  so a stale entry can never equal a live surface. Suggested fix: a
  defensive `retain(alive)` alongside `forget_dead_clicked_layer`. Can
  land any time, e.g. bundled with the next touch of `layer_shell.rs`.
