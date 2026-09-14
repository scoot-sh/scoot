---
title: "No upper bound on *per-pool* shm size (LOW) \u2014 DONE as item 12(d)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No upper bound on *per-pool* shm size (LOW) — DONE as item 12(d)

~~No upper bound on *per-pool* shm size (LOW)~~ — DONE as item 12(d),
512 MiB at both `create_pool` and `resize`, refused rather than clamped.
The entry's "a cap on pool size at creation/resize time would close it" is
exactly what shipped for one pool; what it did not anticipate is that
refusing `create_pool` leaves an uninitialized object whose `request` is a
`panic!` (safe, because `post_error` kills the client synchronously — see
item 12(d) for the full argument, including a second, independent panic
site `flexwm-reviewer` found and confirmed is covered by the same fact).
This closes the per-pool case only — the *total* across many pools from
one client is still unbounded, see the new entry directly below, found
by `flexwm-reviewer` while confirming this one.
Original diagnosis, left as written: found while verifying item 7's
fix, not by the original audit. `wl_shm_pool.resize(i32::MAX)` (or
`wl_shm.create_pool(fd, i32::MAX)` directly — reaches the same `mmap`,
not specific to `resize`) is accepted, reserving a ~2 GiB mapping per
pool, repeatable per pool and per connection, with no cap anywhere.
Verified live: no error, no crash — Smithay's own SIGBUS handler covers
reads/writes past the backing fd's real size, so this isn't the same
memory-safety class as item 7's bug, just unbounded address-space/fd
reservation. Same family as the IPC line-length and screenshot-throttling
findings above (resource exhaustion, not memory corruption) — a cap on
pool size at creation/resize time would close it.
