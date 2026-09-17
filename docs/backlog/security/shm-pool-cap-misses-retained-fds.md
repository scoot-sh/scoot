---
title: "The live-`wl_shm_pool` cap does not bound fds or mappings, which is what its own docs and README say it bounds"
status: "open"
area: "security"
priority: "high"
blocked: null
---

# The live-`wl_shm_pool` cap does not bound fds or mappings, which is what its own docs and README say it bounds

Found by a retrospective audit of PRs #73–#89 on 2026-09-17, traced against
`673c9ea`. PR #75 added the cap and
`docs/backlog/resolved/shm-pool-count-cap-done.md` closed the ticket on it.

The cap counts *live `wl_shm_pool` protocol objects* per client
(`shm_pools.rs`, decremented by `forget_destroyed_shm_pool`,
`dispatch.rs:669`). What it is documented as bounding is fds and mappings:
`shm_pools.rs:25` states "What 128 bounds per connection: 128 fds (an
eighth of a 1024-fd `RLIMIT_NOFILE`), 128 mappings/objects", and the module
header motivates the whole cap with "on the order of a thousand pools from a
single connection exhausts the compositor's fds for every client, not just
the hoarder". `README.md` repeats the framing.

Those are not the same quantity, because `wl_shm_pool.destroy` does not
destroy the mapping. The protocol mandates the opposite: the backing memory
is released only once every `wl_buffer` created from the pool is also gone.
At the pinned Smithay rev that retention is structural — a buffer's user
data holds an `Arc<Pool>`, and the inner pool owns both the mapping and its
`OwnedFd` — so a client that keeps one buffer alive per pool keeps the fd
and the mapping after destroying the pool object.

That makes the documented protection bypassable by construction:

```
loop { create_pool(fd, 4096); create_buffer(); wl_shm_pool.destroy(); }
```

Each iteration returns the live-pool count to zero, so the cap never trips,
while the compositor's fd and mapping count grows without bound — attacker-
paced, with no memory pressure to notice, from one connection, and it
exhausts fds for every client. That is precisely the harm the entry claims
to have closed.

The sharpest part is that the resolved doc's own test list exercises the
bypass shape and reads it as correct bookkeeping: "create-128 / destroy-128 /
create-1 — the 129th lifetime pool succeeds" is the same sequence minus the
`create_buffer` that makes it bite.

What the cap *does* bound, and still usefully: a client naively hoarding
undestroyed pool objects, and the address-space reservation of 128 × the
per-pool 512 MiB limit. The bound is narrower than documented, not absent —
nothing here says the cap should be removed.

Two separable pieces of work, in order:

1. **Correct the claims** (cheap, and should not wait): `shm_pools.rs:25`'s
   "128 fds" sentence, the module header's `RLIMIT_NOFILE` motivation,
   README's framing, and the resolved entry's conclusion. Whatever the fix
   below turns out to be, the docs currently assert a protection the code
   does not provide, and a future reader will trust them.
2. **Decide whether to bound the real resource.** The quantity that matters
   is live *mappings/fds* per client, which means counting `Arc<Pool>`
   lifetimes rather than protocol-object lifetimes. At the pinned rev the
   compositor does not get a hook on the pool's actual drop (it happens on a
   worker thread when the last buffer goes), so this likely needs either an
   upstream affordance or flexwm tracking buffers-per-client itself — the
   same "needs upstream" wall PR #75 already hit for the byte total. Filing
   the constraint rather than a design.
