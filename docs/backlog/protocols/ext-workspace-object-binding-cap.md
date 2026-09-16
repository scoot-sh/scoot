---
title: "Nothing bounds how many `ext_workspace_manager_v1` objects one client may bind."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Nothing bounds how many `ext_workspace_manager_v1` objects one client may bind.

Nothing bounds how many `ext_workspace_manager_v1` objects one client
may bind. Each costs a registry entry and a handle per workspace, and
every workspace change walks them all. Not specific to this protocol —
the same is true of layer surfaces, `wl_shm` pools (item 7's cap is
per-pool, and its own entry already asks for per-client accounting) and
IPC connections (see the screenshot/connection-cap entry below) — but
worth naming while the memory of writing it is fresh: the fix belongs
with those, as per-client accounting, not as a one-off limit here.

Update 2026-09-16: `ext_foreign_toplevel_list_v1` (see
`resolved/foreign-toplevel-list-done.md`) has exactly the same shape and
is deliberately left to this entry rather than capped on its own — each
bind costs one handle object per *window*, and every window change walks
every bound list. Whatever per-client accounting closes this should cover
both globals.
