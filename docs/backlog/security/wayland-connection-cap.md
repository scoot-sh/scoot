---
title: "No cap on Wayland connection count (per-connection bounds multiply)."
status: "open"
area: "security"
priority: "low"
blocked: null
---

# No cap on Wayland connection count (per-connection bounds multiply).

Every per-client bound in the compositor is per *connection*, keyed by
`ClientId`: the capture-frame cap (16), the bind budget (8), the live-pool
count (128). Wayland connections themselves are unbounded -- Smithay's
listening socket accepts without limit, unlike the IPC control socket's 64
slots (`docs/backlog/resolved/ipc-connection-cap-resolved.md`). So N
connections from one abuser hold N times any per-connection allowance: 8
connections at the pool cap hold ~1000 fds, which is the whole soft
`RLIMIT_NOFILE` (1024) on the dev VM, denying pools (and sockets, and
mmaps) to every client, not just the hoarder.

Fix direction: a connection cap on the Wayland socket in the shape of the
IPC one (bounded table, refused-with-reason past it), or a global pool/fd
ceiling across connections -- needs deciding which, and what a legitimate
client past it observes (there is no protocol channel for refusing a
Wayland connection gracefully; the IPC cap's refused-with-reason has no
equivalent here). Per-connection accounting stays as is either way; this
is the multiplier on top of it, not a replacement for it.
