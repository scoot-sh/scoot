---
title: "No upper bound on *total* shm reservation per client (LOW/MEDIUM)."
status: "open"
area: "security"
priority: "low"
blocked: null
---

# No upper bound on *total* shm reservation per client (LOW/MEDIUM).

No upper bound on *total* shm reservation per client (LOW/MEDIUM).
Found by `flexwm-reviewer` while confirming item 12(d)'s per-pool cap
actually closed the original finding — it doesn't, fully. `dispatch.rs`'s
`MAX_SHM_POOL_BYTES` (512 MiB) bounds one pool, but nothing bounds how many
pools one client opens: 40 pools each at exactly the cap reserve ~20 GiB
from a single connection, more address space than the pre-fix 8-pool/16.1
GiB finding item 12(d) was written to close, just needing more requests to
get there. Not the separate IPC-connection-cap Backlog entry's territory
(that one is about the control socket's own connection count, unrelated to
wayland client accounting). Fix direction: per-client cumulative tracking
in the same `dispatch.rs` interception point, with its own cap — needs
deciding what identifies "one client" for accounting purposes (the
`ClientId` `dispatch.rs` already has access to) and where to hang the
running total (`ClientState`, most likely, alongside the existing
per-client data already tracked there).
