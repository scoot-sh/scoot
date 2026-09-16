---
title: "The IPC connection cap turns one client's leak into everyone's refusal (LOW, accepted tradeoff)"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# The IPC connection cap turns one client's leak into everyone's refusal (LOW, accepted tradeoff)

Filed 2026-09-16 alongside
[the connection cap](../resolved/ipc-connection-cap-resolved.md), which
made this true. Recorded as an accepted tradeoff rather than a defect: it
is named here so that a future report of "the bar cannot connect" has
somewhere to land, not because the cap was the wrong call.

Before the cap, a same-user client that leaked IPC connections cost fds and
nothing else; every other client carried on. With it, the 64 slots are a
shared resource, so one misbehaving client of the same user can hold all of
them and every other client — the bar, the notifier, a `flexwm msg`
keybind — is refused until it lets go. The compositor survives either way,
which is the point of the bound; what changed is who notices.

Two things already keep the window finite, and are why this is LOW:

- Every way a connection can end releases its slot — closed, evicted for a
  peer that stopped reading (`WRITE_STALL_TIMEOUT`, ten seconds), or a
  `wait-idle` waiter finishing (`MAX_IDLE_WAIT`, a minute). Nothing holds a
  slot indefinitely any more.
- The refusal names the limit, so the failure reads as "flexwm serves at
  most 64 ipc connections at once, and every slot is in use" rather than as
  an unexplained disconnect.

What would close it properly is per-client accounting rather than a single
global table: `SO_PEERCRED` already gives the peer's uid at accept time
(`listener::peer_uid`), and the same call reports a pid, so a per-pid or
per-uid sub-cap is *possible*. It was deliberately not built for the
original entry — the peer's pid is not a stable identity (it is zero for a
peer in an invisible PID namespace, and pids are reused), every client here
is the same uid by construction, and a sub-cap invented for one hypothetical
client is the kind of speculative structure this project keeps out. Revisit
if a real workload ever hits it.
