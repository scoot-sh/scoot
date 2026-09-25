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

**Updated 2026-09-25 (PR #241): the raised fd limit moved the numbers a
long way, but did not make this moot.** scoot now raises its soft
`RLIMIT_NOFILE` to the hard limit capped at 65536 (`nofile.rs`), and the
wayland-backend fork lets each connection park up to 1024 received fds that
no request claims (128 on a 1024-fd table). Parked fds need no object at
all, so neither the fd ledger nor the grace sees them. Measured on the dev
VM (headless pixman, idle at 18), each connection `fdstuff2 64 20 16`
(sixty-four `wl_display.sync` requests carrying 16 fds each, 1024 parked):

| Table | Connections | scoot fds | Newcomers and `scootctl` |
| --- | --- | --- | --- |
| raised (65536) | 8 | 8218 | served |
| raised (65536) | 63 | 64593 | served |
| raised (65536) | 64 | 65536 (full) | newcomers dropped, `scootctl` reset |
| 1024 (fixed-128 build) | 7 | 921 | newcomer 0 globals, `scootctl` refused |
| 1024 (fixed-128 build) | 8 | 1024 (full) | `scootctl` reset |

Nobody was disconnected at any count
(`~/evidence/fdq/runs/many-connections-raise-2c18a93.out`,
`many-connections-fork-8b01249.out`). So it now takes 64 idle connections
instead of 7 wherever the hard limit allows the raise, and the old 7 where
it does not (containers at 1024). **Why this stays open:** 64 local
connections is still cheap for a hostile client (no objects, a few ms of
sends each), and at 64 the table fills outright, so the IPC-lower-line
candidate below still matters for keeping `scootctl` reachable. Candidate 2
cannot see parked fds (scoot has no count of them). (Observing a full
raised table is one ~7 ms readdir, cached for ~140 ms, so filling the table
no longer makes accepting connections expensive: PR #241 round 3.)
Priority unchanged (medium).

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
