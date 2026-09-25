---
title: "Many connections each under their grace can still hold the fd table at pressure and shed scootctl"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Connection-count residual in fd pressure

Filed 2026-09-24 from the PR #239 re-review. Serves **computer use** first:
`scootctl` is the agent's control channel, and while the table sits at
pressure, it is shed along with every other newcomer.

With per-client fd bounds in place (#236, #239), no *single* connection can
reach the 896-fd pressure line, but connections multiply: six at the 128-fd
grace hold 6 x 162 + 43 = 1015 with nobody past grace, so nobody is refused
or killed, and newcomers (including `scootctl`) are shed for as long as they
stay. Measured with `~/evidence/cfb/pressure.sh` on a 700-fd table: 17
fillers held, a newcomer shed at 584.

**Cheaper since the wayland-backend queue bound** (2026-09-24,
[its ticket](../resolved/wayland-backend-fd-queue-done.md)). That fix made
one connection unable to fill the table through the received-fd queue, but
each connection may now park up to 128 fds there with no object at all,
which neither the fd ledger nor the grace sees. Measured on the dev VM
(headless pixman, 1024-fd table, the fork build `8b01249`,
`~/evidence/fdq/runs/many-connections-fork-8b01249.out`, each connection
`fdstuff2 8 12 16`: eight `wl_display.sync` requests carrying 16 fds
each):

- 6 connections: 792 fds, newcomers and `scootctl` served.
- 7 connections: 921 fds, a newcomer `wayland-info` got 0 globals,
  `scootctl` was refused under fd pressure.
- 8 connections: 1024 (the table full), `scootctl` got a connection reset.
- None of them was disconnected at any count.

Candidate 2 below cannot see these fds (scoot has no count of them);
candidate 1 still helps `scootctl` up to the point the table is full.

Ruled out: a per-uid budget (every client is the same user), a per-pid one
(`SO_PEERCRED` is defeated by fork).

Candidates, cheapest first:
1. **A separate, lower pressure line for IPC accepts** (the `scootctl`
   socket), so the agent's channel stays reachable while Wayland newcomers
   are shed. Likely a few lines in `fd_pressure.rs` and the IPC listener.
2. Under pressure, refuse or disconnect the *heaviest* holder by ledger
   weight (`ClientFds::held_by`) instead of relying on a fixed grace.

Evidence: a pressure run with many light connections where `scootctl`
stays served (and with 2, where the heaviest holder is shed and honest
clients survive).
