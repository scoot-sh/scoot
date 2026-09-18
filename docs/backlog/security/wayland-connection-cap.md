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
count (128), the live-buffer count (512). Wayland connections themselves
are unbounded -- Smithay's listening socket accepts without limit, unlike
the IPC control socket's 64 slots
(`docs/backlog/resolved/ipc-connection-cap-resolved.md`). So N connections
from one abuser hold N times any per-connection allowance: 2 connections at
both shm caps hold ~1280 fds (512 retained buffers + 128 live pools each),
past the whole soft `RLIMIT_NOFILE` (1024) on the dev VM, denying pools
(and sockets, and mmaps) to every client, not just the hoarder. (Before the
buffer cap the same paragraph said 8 connections: the pool count alone
never bounded retained fds -- see
[the live-pool cap correction](../resolved/shm-pool-cap-misses-retained-fds-done.md)
-- so the per-connection retained-fd bound is the buffer cap's 512, not the
pool cap's 128.)

Fix direction: a connection cap on the Wayland socket in the shape of the
IPC one (bounded table, refused-with-reason past it), or a global pool/fd
ceiling across connections -- needs deciding which, and what a legitimate
client past it observes (there is no protocol channel for refusing a
Wayland connection gracefully; the IPC cap's refused-with-reason has no
equivalent here). The blast radius standard is harsher than the IPC cap's:
a Wayland connection cap denies *shells* -- bars and panels bind at
startup, so a miscount kills the user's taskbar, not an attacker's agent
socket. Per-connection accounting stays as is either way; this
is the multiplier on top of it, not a replacement for it.
